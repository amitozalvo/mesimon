//! The mod mesimon lays for Claude Code, and which road a launch takes
//! (T-574; T-573 measured the road).
//!
//! The mod's sources are compiled in and laid under the state dir at
//! `<state>/mod/<version>-<digest8>/`: Claude Code writes `.claude-plugin/
//! types/` and a `tsconfig.json` into the folder at every load, so it is never
//! a checkout (README promise 1 allows the state dir), and keying the folder
//! by the sources' digest means a rebuild never rewrites one a live session
//! loaded. `--plugin-dir <folder>` is the whole installation.
//!
//! The road is `auto`, always (T-588; T-574 made it a setting and T-587
//! took the setting away): the mod only once a probe of the Claude Code on
//! PATH found it new enough and `claude plugin validate` passed on the laid
//! folder, the hook set otherwise, and nobody is asked. The probe is cached
//! per binary (path, mtime, length) and per mod digest, so a Claude Code that
//! updated itself is probed again before it is trusted. The seam
//! `MESIMON_CLAUDE_ROAD` (`hooks|mod|auto`) is for the tests: `TestFixture`
//! sets `hooks` so no stub is ever probed as Claude Code.
//!
//! Validating is not loading (T-598): Claude Code 2.1.288 turned mods off by
//! a remote flag while `claude plugin validate` still passed and the laid mod
//! still rode `--plugin-dir`, so the probe's third step runs `claude plugin
//! test` on a one-test folder laid beside the mod ([`LOAD_PROBE`]), which
//! Claude Code refuses in the flag's words while mods are off. A launch whose
//! mod never reports is relaunched on the hook set and writes the same
//! verdict ([`Verdict::ModsOff`]); that verdict expires
//! ([`Probe::recheck_due`]), so a Claude Code that turns mods back on gets
//! the mod again with nobody asked.
//!
//! Loading is not hearing (T-650): on a Team or Enterprise account, or a
//! machine with managed settings, Claude Code 2.1.289 seats its own
//! `cc-plugin-sec-default` outermost, whose `classic.*` hook hands every
//! classic hook event past a person's plugins (`next.to(e, "append")`). The
//! mod then loads, serves the tools and takes a `submit`, and never hears
//! `SessionStart`, `UserPromptSubmit` or `Stop`. The probe's fourth step
//! reads what seats it where it can: `claude auth status --json`'s
//! `subscriptionType` (`team`, `enterprise`) and the managed-settings file
//! on this machine; either is [`Verdict::ClassicOff`] before any launch. What
//! the probe cannot see (managed settings served remotely) a launch finds:
//! one whose bridge polled and whose `SessionStart` never came is relaunched
//! and writes the same verdict, which expires as the mods-off one does.
//! Under it a launch is native (T-658): the mod gets `MESIMON_MOD_NATIVE=1`
//! and reports from Claude Code's own events (T-651), and a one-entry hook
//! set rides beside it for the permission dialog alone.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mesimon_core::road::{has_mods, parse_claude_version, Road, RoadPref};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::paths::Paths;

/// The mod, as laid: (path in the folder, contents).
pub const FILES: [(&str, &str); 5] = [
    (".claude-plugin/plugin.json", include_str!("../mod/.claude-plugin/plugin.json")),
    ("hooks/hooks.json", include_str!("../mod/hooks/hooks.json")),
    ("hooks/register.ts", include_str!("../mod/hooks/register.ts")),
    ("hooks/register.test.ts", include_str!("../mod/hooks/register.test.ts")),
    (".gitignore", include_str!("../mod/.gitignore")),
];

/// The sources' digest, hex: what names the folder and keys the probe.
pub fn digest() -> String {
    let mut h = Sha256::new();
    for (path, text) in FILES {
        h.update(path.as_bytes());
        h.update([0]);
        h.update(text.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `<state>/mod/<version>-<digest8>`.
pub fn folder(paths: &Paths) -> PathBuf {
    paths.mod_root().join(format!("{}-{}", env!("CARGO_PKG_VERSION"), &digest()[..8]))
}

/// The load probe (T-598): a mod whose module registers nothing and whose
/// one test passes. `claude plugin test` on it answers in a third of a
/// second with no model turn (measured on 2.1.288), and refuses before any
/// test runs while Claude Code has mods off. Its own folder, so the probe
/// never runs the real mod's tests, whose clocks a loaded machine can miss.
pub const LOAD_PROBE: [(&str, &str); 4] = [
    (
        ".claude-plugin/plugin.json",
        "{\n  \"name\": \"mesimon-load-probe\",\n  \"version\": \"1.0.0\",\n  \"description\": \"mesimon's check that this Claude Code loads mods. Registers nothing; never passed to a session.\"\n}\n",
    ),
    ("hooks/hooks.json", "{ \"modules\": [\"./load.ts\"] }\n"),
    (
        "hooks/load.ts",
        "// Registers nothing: mesimon's check that this Claude Code loads mods.\nexport const register = (_on: any) => {}\n",
    ),
    (
        "hooks/load.test.ts",
        "import { expect, test } from 'claude-code/testing'\n\ntest('a mod loads here', () => {\n  expect(true).toBe(true)\n})\n",
    ),
];

/// `<state>/mod/load-probe`.
pub fn load_probe_folder(paths: &Paths) -> PathBuf {
    paths.mod_root().join("load-probe")
}

/// The words `claude plugin test` refuses in while mods are off: by the
/// remote flag ("… in this process: …") or by a setting or a policy
/// ("… here (disableAllHooks, …)"), measured on 2.1.288.
const MODS_OFF_WORDS: &str = "hooks modules are turned off";

/// How long a mods-off verdict stands before the probe asks again (T-598).
/// A daemon start asks at once, and so does a Claude Code that changed.
pub const MODS_OFF_TTL_MS: u64 = 6 * 60 * 60 * 1000;

/// Lay the mod (dirs 0700, files 0600), writing only what is missing or
/// differs: the startup lay and the check before each mod launch are the
/// same call, and the second also repairs a folder something rewrote.
pub fn lay(paths: &Paths) -> anyhow::Result<PathBuf> {
    lay_files(paths, &folder(paths), &FILES)
}

/// Lay the load probe, the same way.
pub fn lay_load_probe(paths: &Paths) -> anyhow::Result<PathBuf> {
    lay_files(paths, &load_probe_folder(paths), &LOAD_PROBE)
}

fn lay_files(paths: &Paths, root: &Path, files: &[(&str, &str)]) -> anyhow::Result<PathBuf> {
    let root = root.to_path_buf();
    crate::paths::own_private_dir(&paths.mod_root())?;
    crate::paths::own_private_dir(&root)?;
    for &(rel, text) in files {
        let path = root.join(rel);
        if let Some(dir) = path.parent() {
            if dir != root {
                crate::paths::own_private_dir(dir)?;
            }
        }
        if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
            crate::store::write_atomic(&path, text, crate::store::PRIVATE)?;
        }
    }
    Ok(root)
}

/// Where the road a launch asked for came from, for the verdict file and
/// `doctor`: the test seam, or nothing (`auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Seam,
    Default,
}

/// The setting as one launch reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub pref: RoadPref,
    pub source: Source,
}

/// The seam's word, else `auto`. A word this build does not know is no
/// word: the road is `auto`. No file is read — a `claude_integration` key
/// an alpha.36 board wrote stays in its `prefs.json`, unread.
pub fn resolve(seam: Option<&str>) -> Setting {
    match seam.and_then(RoadPref::from_word) {
        Some(pref) => Setting { pref, source: Source::Seam },
        None => Setting { pref: RoadPref::Auto, source: Source::Default },
    }
}

/// The road this process's launches ask for.
pub fn read_setting() -> Setting {
    resolve(std::env::var("MESIMON_CLAUDE_ROAD").ok().as_deref())
}

/// The Claude Code a launch would run: `MESIMON_CLAUDE_BIN` (a name or a
/// path), else `claude`, found on the captured `PATH`.
pub fn claude_binary(path_var: Option<&str>) -> Option<PathBuf> {
    let name = std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    if name.contains('/') {
        return Some(PathBuf::from(name)).filter(|p| p.is_file());
    }
    std::env::split_paths(path_var?).map(|dir| dir.join(&name)).find(|p| p.is_file())
}

/// What a probe is cached by: the binary as it stands, and the mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeKey {
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub len: u64,
    pub digest: String,
}

/// The key for `bin` now: its canonical path (the symlink an installer
/// moves is followed) and that file's stamp.
pub fn probe_key(bin: &Path) -> Option<ProbeKey> {
    let path = std::fs::canonicalize(bin).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let mtime_ms = mesimon_core::clock::epoch_ms(meta.modified().ok()?)?;
    Some(ProbeKey { path, mtime_ms, len: meta.len(), digest: digest() })
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// New enough, and the laid mod validated: `auto` takes the mod.
    Passed { version: String },
    /// Older than 2.1.287: no mods.
    TooOld { version: String },
    /// `claude --version` said nothing a version could be read from.
    Unreadable { output: String },
    /// New enough, and `claude plugin validate` refused the mod — a Claude
    /// Code update changed the surface under it.
    ValidateFailed { version: String, error: String },
    /// The mod validates and this Claude Code does not load mods (T-598):
    /// `claude plugin test` refused in the flag's words, or a launch's mod
    /// never reported and was relaunched on the hook set. `seen_at` (epoch
    /// ms) is when; the verdict is asked again after [`MODS_OFF_TTL_MS`].
    ModsOff { version: String, seen_at: u64 },
    /// The mod loads and this Claude Code keeps the hook events from it
    /// (T-650): every launch is native (T-658), the mod reporting from its
    /// own events with the permission hooks beside it. A Team or
    /// Enterprise account, or managed settings, seat `cc-plugin-sec-default`,
    /// which hands every `classic.*` event past a person's plugins. `found`
    /// says how the probe knew (the account, the managed-settings file, or a
    /// launch whose mod heard no `SessionStart`); `seen_at` (epoch ms) is
    /// when; the verdict is asked again after [`MODS_OFF_TTL_MS`].
    ClassicOff {
        version: String,
        seen_at: u64,
        #[serde(default)]
        found: String,
    },
    /// The mod validates and `claude plugin test` failed on the load probe
    /// in other words: the mod is not proven to load.
    LoadFailed { version: String, error: String },
    /// The binary could not be run, or ran out of time.
    Failed { error: String },
}

impl Verdict {
    pub fn passed(&self) -> bool {
        matches!(self, Verdict::Passed { .. })
    }

    /// Whether `auto` takes the mod: it is proven to load, heard (`Passed`)
    /// or not (`ClassicOff`, T-650).
    pub fn loads(&self) -> bool {
        matches!(self, Verdict::Passed { .. } | Verdict::ClassicOff { .. })
    }

    /// Whether the mod launches native (T-650, T-658): it loads and this
    /// Claude Code keeps the hook events from it.
    pub fn deaf(&self) -> bool {
        matches!(self, Verdict::ClassicOff { .. })
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            Verdict::Passed { version }
            | Verdict::TooOld { version }
            | Verdict::ValidateFailed { version, .. }
            | Verdict::ModsOff { version, .. }
            | Verdict::ClassicOff { version, .. }
            | Verdict::LoadFailed { version, .. } => Some(version),
            _ => None,
        }
    }

    /// One line for the feed and `doctor`.
    pub fn line(&self) -> String {
        match self {
            Verdict::Passed { version } => format!("claude {version}, the mod validated"),
            Verdict::TooOld { version } => format!("claude {version} is older than 2.1.287"),
            Verdict::Unreadable { output } => format!("claude --version said {output:?}"),
            Verdict::ValidateFailed { version, error } => {
                format!("claude plugin validate failed on {version}: {error}")
            }
            Verdict::ModsOff { version, seen_at } => format!(
                "claude {version}: mods are off in this Claude Code (seen {}); the hook set is used",
                clock_of(*seen_at)
            ),
            Verdict::ClassicOff { version, seen_at, found } => format!(
                "claude {version}: hook events do not reach the mod in this Claude Code (seen {}; {}); the plugin reports from its own events, with one hook for permissions",
                clock_of(*seen_at),
                if found.is_empty() { FOUND_UNSAID } else { found }
            ),
            Verdict::LoadFailed { version, error } => {
                format!("claude plugin test failed on {version}: {error}")
            }
            Verdict::Failed { error } => error.clone(),
        }
    }
}

/// A classic-off verdict from a `probe.json` written before `found` was.
const FOUND_UNSAID: &str = "a Team or Enterprise account, or managed settings";
/// How a launch found it (`Silence::Deaf`'s verdict).
pub const FOUND_BY_LAUNCH: &str = "a launch's mod heard no SessionStart";
/// How the probe found it on this machine's managed-settings file.
pub const FOUND_BY_MANAGED: &str = "managed settings on this machine";

/// `HH:MM` local, for a verdict's line (`--:--` where libc cannot say).
fn clock_of(ms: u64) -> String {
    // `t`'s type is inferred from `localtime_r`'s parameter and never spelled
    // (the libc crate deprecates the `time_t` alias on musl).
    let Ok(t) = (ms / 1000).try_into() else { return "--:--".into() };
    // SAFETY: `localtime_r` writes only into the zeroed `tm` it is handed,
    // which lives for the call; nothing is read on a null return.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm).is_null() } {
        return "--:--".into();
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// One cached probe: `<state>/mod/probe.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    pub key: ProbeKey,
    #[serde(flatten)]
    pub verdict: Verdict,
}

impl Probe {
    /// Whether this verdict is to be asked again (T-598): a mods-off one, or
    /// a classic-off one (T-650), [`MODS_OFF_TTL_MS`] after it was seen. A
    /// verdict about a binary that changed since is asked by the key, and
    /// every other verdict stands until then: it was read off the binary and
    /// the mod, which a key names.
    pub fn recheck_due(&self, now_ms: u64) -> bool {
        match &self.verdict {
            Verdict::ModsOff { seen_at, .. } | Verdict::ClassicOff { seen_at, .. } => {
                now_ms.saturating_sub(*seen_at) >= MODS_OFF_TTL_MS
            }
            _ => false,
        }
    }

    /// Claude Code's own fallback, which asks nothing of the person: it has
    /// mods off (T-598), or it keeps the hook events from them (T-650).
    /// Either is a verdict with a clock, asked again when it is due.
    pub fn claude_off(&self) -> bool {
        matches!(self.verdict, Verdict::ModsOff { .. } | Verdict::ClassicOff { .. })
    }
}

pub fn load_probe(paths: &Paths) -> Option<Probe> {
    serde_json::from_str(&std::fs::read_to_string(paths.mod_root().join("probe.json")).ok()?).ok()
}

pub fn save_probe(paths: &Paths, probe: &Probe) {
    if let Ok(text) = serde_json::to_string_pretty(probe) {
        let _ = crate::paths::own_private_dir(&paths.mod_root());
        let _ = crate::store::write_atomic(
            &paths.mod_root().join("probe.json"),
            &text,
            crate::store::PRIVATE,
        );
    }
}

const VERSION_TIMEOUT: Duration = Duration::from_secs(20);
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(20);

/// The managed-settings file Claude Code reads on this machine, by its
/// documented path: present, Claude Code seats its security default
/// outermost ("this machine has managed settings", T-650). Managed settings
/// served remotely, by MDM profile or the registry are not seen here; a
/// launch finds those.
pub fn managed_settings_present() -> bool {
    let path = if cfg!(target_os = "macos") {
        "/Library/Application Support/ClaudeCode/managed-settings.json"
    } else {
        "/etc/claude-code/managed-settings.json"
    };
    Path::new(path).is_file()
}

/// What `claude auth status --json` says that seats the security default
/// (T-650): a `subscriptionType` of `team` or `enterprise`. `None` for any
/// other account, and for an answer that is not that JSON.
pub fn deaf_account(status: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(status.trim()).ok()?;
    let kind = value.get("subscriptionType")?.as_str()?.trim().to_ascii_lowercase();
    match kind.as_str() {
        "team" => Some("a team account".into()),
        "enterprise" => Some("an enterprise account".into()),
        _ => None,
    }
}

/// Run the probe. `version`, `validate`, `load` (`claude plugin test` on
/// the load probe) and `account` (`claude auth status --json`) are whole
/// command lines, the launcher included (the pane's environment, as a
/// launch gets it: an environment that turns Claude Code's flags service
/// off loads mods, and the probe must see what the pane will). `managed`
/// is [`managed_settings_present`]. Runs on a worker: a cold Claude Code
/// takes seconds to answer. `now_ms` stamps a mods-off or classic-off
/// verdict. An account the command cannot say is not held against the mod:
/// a launch finds a deaf one (T-650).
pub fn probe(
    version: &[String],
    validate: &[String],
    load: &[String],
    account: &[String],
    managed: bool,
    cwd: &Path,
    now_ms: u64,
) -> Verdict {
    let out = match run_bounded(version, cwd, VERSION_TIMEOUT) {
        Ok(out) => out,
        Err(error) => return Verdict::Failed { error },
    };
    let Some(parsed) = parse_claude_version(&out.stdout) else {
        let output: String = out.stdout.trim().chars().take(80).collect();
        return Verdict::Unreadable { output };
    };
    let version = format!("{}.{}.{}", parsed.0, parsed.1, parsed.2);
    if !has_mods(parsed) {
        return Verdict::TooOld { version };
    }
    match run_bounded(validate, cwd, VALIDATE_TIMEOUT) {
        Ok(out) if out.code == Some(0) => {}
        Ok(out) => return Verdict::ValidateFailed { version, error: first_error(&out) },
        Err(error) => return Verdict::ValidateFailed { version, error },
    }
    match run_bounded(load, cwd, LOAD_TIMEOUT) {
        Ok(out) if out.code == Some(0) => {}
        Ok(out) if format!("{}\n{}", out.stdout, out.stderr).contains(MODS_OFF_WORDS) => {
            return Verdict::ModsOff { version, seen_at: now_ms };
        }
        Ok(out) => return Verdict::LoadFailed { version, error: first_error(&out) },
        Err(error) => return Verdict::LoadFailed { version, error },
    }
    if managed {
        return Verdict::ClassicOff { version, seen_at: now_ms, found: FOUND_BY_MANAGED.into() };
    }
    let found = match run_bounded(account, cwd, ACCOUNT_TIMEOUT) {
        Ok(out) if out.code == Some(0) => deaf_account(&out.stdout),
        _ => None,
    };
    match found {
        Some(found) => Verdict::ClassicOff { version, seen_at: now_ms, found },
        None => Verdict::Passed { version },
    }
}

/// The line `claude plugin validate` gave its refusal in: the first `❯`
/// line after a `✘`, else the first line that says anything.
fn first_error(out: &Output) -> String {
    let text = format!("{}\n{}", out.stdout, out.stderr);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let after_cross = lines.iter().position(|l| l.starts_with('✘')).and_then(|i| {
        lines[i..].iter().find(|l| l.starts_with('❯')).map(|l| l.trim_start_matches('❯').trim())
    });
    let line = after_cross
        .or_else(|| lines.first().copied())
        .unwrap_or("exited without a word")
        .to_string();
    line.chars().take(200).collect()
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// A child in a process group of its own, read whole, killed whole at the
/// deadline.
fn run_bounded(argv: &[String], cwd: &Path, timeout: Duration) -> Result<Output, String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let (bin, args) = argv.split_first().ok_or("no command")?;
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("could not start {bin}: {e}"))?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_string(&mut text);
            }
            text
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) | Err(_) => {
                // SAFETY: a signal to the group this function made.
                unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
                let _ = child.wait();
                return Err(format!("{bin} did not answer in {} s", timeout.as_secs()));
            }
        }
    };
    Ok(Output {
        code,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// What the daemon decided, for `doctor` (the daemon's seam is not in
/// doctor's environment): `<state>/mod/road.json`, rewritten when it changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoadVerdict {
    pub road: Road,
    pub setting: String,
    pub source: Source,
    /// The probe's line, when the setting is `auto` and a probe ran.
    #[serde(default)]
    pub probe: Option<String>,
    /// The mod could not be laid: the launch took `hooks`.
    #[serde(default)]
    pub lay_error: Option<String>,
    /// `auto` fell back after a passing probe: say so loudly.
    #[serde(default)]
    pub fallback: bool,
    /// The fallback is Claude Code's own, and nothing is asked of the
    /// person: it turned mods off (T-598, the hook set alone), or it keeps
    /// the hook events from them (T-650, a Team or Enterprise account: the
    /// native mod, T-658). The probe's line says which. The key keeps
    /// its first name for the `doctor` of an older build.
    #[serde(default)]
    pub mods_off: bool,
}

pub fn read_verdict(paths: &Paths) -> Option<RoadVerdict> {
    serde_json::from_str(&std::fs::read_to_string(paths.mod_root().join("road.json")).ok()?).ok()
}

pub fn write_verdict(paths: &Paths, verdict: &RoadVerdict) {
    if let Ok(text) = serde_json::to_string_pretty(verdict) {
        let _ = crate::paths::own_private_dir(&paths.mod_root());
        let _ = crate::store::write_atomic(
            &paths.mod_root().join("road.json"),
            &text,
            crate::store::PRIVATE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn paths(dir: &Path) -> Paths {
        let mut p = Paths::for_repo(dir).unwrap();
        p.state_dir = dir.join("state");
        p
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_mod_is_laid_private_under_a_digest_folder_and_repaired_when_touched() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let root = lay(&p).unwrap();
        assert_eq!(root, folder(&p));
        let name = root.file_name().unwrap().to_str().unwrap().to_string();
        assert_eq!(name, format!("{}-{}", env!("CARGO_PKG_VERSION"), &digest()[..8]));
        assert!(root.starts_with(&p.state_dir), "never a checkout");
        assert_eq!(mode(&p.mod_root()), 0o700);
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("hooks")), 0o700);
        for (rel, text) in FILES {
            assert_eq!(std::fs::read_to_string(root.join(rel)).unwrap(), text);
            assert_eq!(mode(&root.join(rel)), 0o600, "{rel}");
        }
        // Claude Code's own files beside it are left alone; a rewritten
        // source is put back.
        std::fs::write(root.join("tsconfig.json"), "{}").unwrap();
        std::fs::write(root.join("hooks/register.ts"), "export const register = () => {}").unwrap();
        lay(&p).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("hooks/register.ts")).unwrap(), FILES[2].1);
        assert_eq!(std::fs::read_to_string(root.join("tsconfig.json")).unwrap(), "{}");
    }

    #[test]
    fn the_plugin_is_named_mesimon() {
        let manifest: serde_json::Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(manifest["name"], "mesimon");
    }

    #[test]
    fn the_road_is_the_seam_else_auto() {
        let auto = Setting { pref: RoadPref::Auto, source: Source::Default };
        assert_eq!(resolve(None), auto);
        assert_eq!(resolve(Some("socket")), auto, "a word this build does not know");
        for (word, pref) in
            [("hooks", RoadPref::Hooks), ("mod", RoadPref::Mod), ("auto", RoadPref::Auto)]
        {
            assert_eq!(resolve(Some(word)), Setting { pref, source: Source::Seam });
        }
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.display().to_string()
    }

    #[test]
    fn the_probe_reads_the_floor_then_the_validator() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let new = script(d, "new", "echo '2.1.287 (Claude Code)'");
        let old = script(d, "old", "echo '2.1.286 (Claude Code)'");
        let odd = script(d, "odd", "echo 'Claude Code'");
        let pass = script(d, "pass", "echo '✔ Validation passed'");
        let fail = script(
            d,
            "fail",
            "echo 'Validating hooks'; echo '✘ Found 1 error:'; echo '  ❯ modules./x.ts: $ is assigned'; exit 1",
        );
        let v = |a: &str, b: &str| {
            probe(&[a.to_string()], &[b.to_string()], std::slice::from_ref(&pass), &[], false, d, 7)
        };
        assert_eq!(v(&new, &pass), Verdict::Passed { version: "2.1.287".into() });
        assert_eq!(v(&old, &pass), Verdict::TooOld { version: "2.1.286".into() });
        assert_eq!(v(&odd, &pass), Verdict::Unreadable { output: "Claude Code".into() });
        assert_eq!(
            v(&new, &fail),
            Verdict::ValidateFailed {
                version: "2.1.287".into(),
                error: "modules./x.ts: $ is assigned".into()
            }
        );
        assert!(matches!(
            probe(
                &[d.join("missing").display().to_string()],
                std::slice::from_ref(&pass),
                std::slice::from_ref(&pass),
                &[],
                false,
                d,
                7
            ),
            Verdict::Failed { .. }
        ));
    }

    /// T-598: validating is not loading. The load probe's refusal, in the
    /// words 2.1.288 gives while its remote flag has mods off, is a mods-off
    /// verdict stamped with the probe's clock; any other failure of it is
    /// not proof the mod loads either.
    #[test]
    fn the_probe_proves_the_mod_loads_and_reads_mods_off_from_the_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let new = script(d, "new", "echo '2.1.288 (Claude Code)'");
        let pass = script(d, "pass", "echo '✔ Validation passed'");
        let off = script(
            d,
            "off",
            "echo 'claude plugin test: hooks modules are turned off in this process: the rollout switch was saved off by an earlier session and is not refreshed yet.' >&2; exit 1",
        );
        let policy = script(
            d,
            "policy",
            "echo 'claude plugin test: hooks modules are turned off here (disableAllHooks, allowManagedHooksOnly or a policy)' >&2; exit 1",
        );
        let broken = script(d, "broken", "echo ' 0 pass'; echo ' 1 fail'; exit 1");
        let v = |load: &str| {
            probe(
                std::slice::from_ref(&new),
                std::slice::from_ref(&pass),
                &[load.to_string()],
                &[],
                false,
                d,
                42,
            )
        };
        assert_eq!(v(&pass), Verdict::Passed { version: "2.1.288".into() });
        let mods_off = Verdict::ModsOff { version: "2.1.288".into(), seen_at: 42 };
        assert_eq!(v(&off), mods_off);
        assert_eq!(v(&policy), mods_off);
        assert_eq!(
            v(&broken),
            Verdict::LoadFailed { version: "2.1.288".into(), error: "0 pass".into() }
        );
        assert!(!v(&broken).passed());
        let line = mods_off.line();
        assert!(
            line.starts_with("claude 2.1.288: mods are off in this Claude Code (seen "),
            "{line}"
        );
        assert!(line.ends_with("); the hook set is used"), "{line}");
    }

    /// T-650: loading is not hearing. The probe's fourth step reads the
    /// account: a `team` or `enterprise` one, or a managed-settings file on
    /// this machine, is a classic-off verdict before any launch, with how it
    /// was found; any other account, or an answer that says nothing, is
    /// passed (a launch finds a deaf mod the probe could not see).
    #[test]
    fn the_probe_reads_the_account_that_seats_the_security_default() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let new = script(d, "new", "echo '2.1.289 (Claude Code)'");
        let pass = script(d, "pass", "echo '✔ Validation passed'");
        let account = |name: &str, json: &str| script(d, name, &format!("echo '{json}'"));
        let max = account("max", r#"{"loggedIn":true,"subscriptionType":"max"}"#);
        let team = account("team", r#"{"loggedIn":true,"subscriptionType":"team"}"#);
        let ent = account("ent", r#"{"loggedIn":true,"subscriptionType":"Enterprise"}"#);
        let mute = account("mute", "");
        let gone = script(d, "gone", "echo 'not logged in' >&2; exit 1");
        let v = |account: &str, managed: bool| {
            probe(
                std::slice::from_ref(&new),
                std::slice::from_ref(&pass),
                std::slice::from_ref(&pass),
                &[account.to_string()],
                managed,
                d,
                9,
            )
        };
        let passed = Verdict::Passed { version: "2.1.289".into() };
        assert_eq!(v(&max, false), passed);
        assert_eq!(v(&mute, false), passed);
        assert_eq!(v(&gone, false), passed);
        let deaf = |found: &str| Verdict::ClassicOff {
            version: "2.1.289".into(),
            seen_at: 9,
            found: found.into(),
        };
        assert_eq!(v(&team, false), deaf("a team account"));
        assert_eq!(v(&ent, false), deaf("an enterprise account"));
        assert_eq!(v(&max, true), deaf(FOUND_BY_MANAGED));
        let line = v(&team, false).line();
        assert!(line.contains("(seen "), "{line}");
        assert!(
            line.contains(
                "; a team account); the plugin reports from its own events, with one hook for permissions"
            ),
            "{line}"
        );
        assert_eq!(deaf_account("garbage"), None);
        assert_eq!(deaf_account(r#"{"subscriptionType":"pro"}"#), None);
        // A `probe.json` from before `found` reads with the general words.
        let old: Verdict =
            serde_json::from_str(r#"{"verdict":"classic_off","version":"2.1.289","seen_at":9}"#)
                .unwrap();
        assert!(old.line().contains(FOUND_UNSAID), "{}", old.line());
    }

    /// T-598: a mods-off verdict is asked again six hours after it was seen,
    /// and so is a classic-off one (T-650); a passing one, or one the
    /// binary's version settles, is not.
    #[test]
    fn a_mods_off_verdict_expires_and_no_other_does() {
        let key = ProbeKey { path: "/c".into(), mtime_ms: 1, len: 1, digest: digest() };
        let at = |verdict| Probe { key: key.clone(), verdict };
        let seen = 1_000_000;
        let off = at(Verdict::ModsOff { version: "2.1.288".into(), seen_at: seen });
        let deaf = at(Verdict::ClassicOff {
            version: "2.1.289".into(),
            seen_at: seen,
            found: FOUND_BY_LAUNCH.into(),
        });
        for p in [&off, &deaf] {
            assert!(p.claude_off());
            assert!(!p.recheck_due(seen));
            assert!(!p.recheck_due(seen + MODS_OFF_TTL_MS - 1));
            assert!(p.recheck_due(seen + MODS_OFF_TTL_MS));
            assert!(!p.recheck_due(0), "a clock behind the stamp is not past it");
        }
        for verdict in [
            Verdict::Passed { version: "2.1.288".into() },
            Verdict::TooOld { version: "2.1.286".into() },
            Verdict::LoadFailed { version: "2.1.288".into(), error: "x".into() },
        ] {
            let p = at(verdict);
            assert!(!p.claude_off());
            assert!(!p.recheck_due(seen + 10 * MODS_OFF_TTL_MS));
        }
        // T-650: the classic-off line names what was seen and the account
        // that does it, and ends on the road taken.
        let line = deaf.verdict.line();
        assert!(
            line.starts_with(
                "claude 2.1.289: hook events do not reach the mod in this Claude Code (seen "
            ),
            "{line}"
        );
        assert!(line.contains(FOUND_BY_LAUNCH), "{line}");
        assert!(
            line.ends_with(
                "); the plugin reports from its own events, with one hook for permissions"
            ),
            "{line}"
        );
        assert_eq!(deaf.verdict.version(), Some("2.1.289"));
        // The mod is taken either way; native only where it is deaf.
        assert!(deaf.verdict.loads() && deaf.verdict.deaf());
        assert!(!off.verdict.loads() && !off.verdict.deaf());
        let passed = Verdict::Passed { version: "2.1.289".into() };
        assert!(passed.loads() && !passed.deaf());
        // Both round-trip through the cache file.
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        save_probe(&p, &off);
        assert_eq!(load_probe(&p), Some(off));
        save_probe(&p, &deaf);
        assert_eq!(load_probe(&p), Some(deaf));
    }

    #[test]
    fn the_load_probe_is_laid_beside_the_mod_and_never_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let root = lay_load_probe(&p).unwrap();
        assert_eq!(root, load_probe_folder(&p));
        assert!(!root.starts_with(folder(&p)) && !folder(&p).starts_with(&root));
        assert_eq!(mode(&root), 0o700);
        for (rel, text) in LOAD_PROBE {
            assert_eq!(std::fs::read_to_string(root.join(rel)).unwrap(), text);
            assert_eq!(mode(&root.join(rel)), 0o600, "{rel}");
        }
        let manifest: serde_json::Value = serde_json::from_str(LOAD_PROBE[0].1).unwrap();
        assert_eq!(manifest["name"], "mesimon-load-probe");
        let hooks: serde_json::Value = serde_json::from_str(LOAD_PROBE[1].1).unwrap();
        assert_eq!(hooks["modules"][0], "./load.ts");
    }

    #[test]
    fn a_probe_that_hangs_is_killed_at_its_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let hang = script(dir.path(), "hang", "sleep 30");
        let t = Instant::now();
        let r = run_bounded(&[hang], dir.path(), Duration::from_millis(300));
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_probe_key_moves_with_the_binary_and_the_cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "a").unwrap();
        let first = probe_key(&bin).unwrap();
        assert_eq!(first.digest, digest());
        std::fs::write(&bin, "a longer binary").unwrap();
        assert_ne!(probe_key(&bin).unwrap(), first, "an update changes the key");
        let p = paths(dir.path());
        let probe = Probe { key: first, verdict: Verdict::Passed { version: "2.1.287".into() } };
        save_probe(&p, &probe);
        assert_eq!(load_probe(&p), Some(probe));
        assert_eq!(mode(&p.mod_root().join("probe.json")), 0o600);
    }

    #[test]
    fn the_binary_is_found_on_the_captured_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("claude"), "").unwrap();
        let path = format!("{}:{}", a.display(), b.display());
        if std::env::var_os("MESIMON_CLAUDE_BIN").is_none() {
            assert_eq!(claude_binary(Some(&path)), Some(b.join("claude")));
            assert_eq!(claude_binary(None), None);
        }
    }
}
