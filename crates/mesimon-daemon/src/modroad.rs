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
//! The road is the "Claude integration" setting: the seam
//! `MESIMON_CLAUDE_ROAD`, else this board's `prefs.json`, else the machine's,
//! else `hooks`. The daemon reads the files itself, at every launch, because
//! `hooks` is the kill switch and must hold with no board open (a queued
//! wake, the crown's start, a phone). `auto` takes the mod only once a probe
//! of the Claude Code on PATH found it new enough and `claude plugin
//! validate` passed on the laid folder; the probe is cached per binary (path,
//! mtime, length) and per mod digest, so a Claude Code that updated itself is
//! probed again before it is trusted.

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

/// Lay the mod (dirs 0700, files 0600), writing only what is missing or
/// differs: the startup lay and the check before each mod launch are the
/// same call, and the second also repairs a folder something rewrote.
pub fn lay(paths: &Paths) -> anyhow::Result<PathBuf> {
    let root = folder(paths);
    crate::paths::own_private_dir(&paths.mod_root())?;
    crate::paths::own_private_dir(&root)?;
    for (rel, text) in FILES {
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

/// Where the setting came from, for the verdict file and `doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Seam,
    Board,
    Machine,
    Default,
}

/// The setting as one launch reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub pref: RoadPref,
    pub source: Source,
}

/// The seam's word, then the board's, then the machine's, then `hooks`. A
/// word this build does not know falls through to the next layer: a newer
/// build's road is no reason to turn the mod on.
pub fn resolve(
    seam: Option<&str>,
    board: Option<&serde_json::Value>,
    machine: Option<&serde_json::Value>,
) -> Setting {
    let key = mesimon_core::prefs::PrefKey::ClaudeRoad.name();
    if let Some(pref) = seam.and_then(RoadPref::from_word) {
        return Setting { pref, source: Source::Seam };
    }
    for (doc, source) in [(board, Source::Board), (machine, Source::Machine)] {
        if let Some(pref) =
            doc.and_then(|d| d.get(key)).and_then(|v| v.as_str()).and_then(RoadPref::from_word)
        {
            return Setting { pref, source };
        }
    }
    Setting { pref: RoadPref::default(), source: Source::Default }
}

/// The setting as the files say it now. Two small reads per Claude launch.
pub fn read_setting(paths: &Paths) -> Setting {
    let read = |path: Option<PathBuf>| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(path?).ok()?).ok()
    };
    let seam = std::env::var("MESIMON_CLAUDE_ROAD").ok();
    let board = read(Some(paths.prefs_file()));
    let machine = read(crate::paths::state_root().ok().map(|r| r.join("prefs.json")));
    resolve(seam.as_deref(), board.as_ref(), machine.as_ref())
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
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)?;
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
    /// The binary could not be run, or ran out of time.
    Failed { error: String },
}

impl Verdict {
    pub fn passed(&self) -> bool {
        matches!(self, Verdict::Passed { .. })
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            Verdict::Passed { version }
            | Verdict::TooOld { version }
            | Verdict::ValidateFailed { version, .. } => Some(version),
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
            Verdict::Failed { error } => error.clone(),
        }
    }
}

/// One cached probe: `<state>/mod/probe.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Probe {
    pub key: ProbeKey,
    #[serde(flatten)]
    pub verdict: Verdict,
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

/// Run the probe. `version` and `validate` are whole command lines, the
/// launcher included (the pane's environment, as a launch gets it). Runs on a
/// worker: a cold Claude Code takes seconds to answer.
pub fn probe(version: &[String], validate: &[String], cwd: &Path) -> Verdict {
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
        Ok(out) if out.code == Some(0) => Verdict::Passed { version },
        Ok(out) => Verdict::ValidateFailed { version, error: first_error(&out) },
        Err(error) => Verdict::ValidateFailed { version, error },
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
    use serde_json::json;
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
    fn the_seam_beats_the_board_which_beats_the_machine_which_beats_hooks() {
        let board = json!({ "claude_integration": "auto" });
        let machine = json!({ "claude_integration": "mod" });
        let s = |seam, b, m| resolve(seam, b, m);
        assert_eq!(s(None, None, None), Setting { pref: RoadPref::Hooks, source: Source::Default });
        assert_eq!(
            s(None, None, Some(&machine)),
            Setting { pref: RoadPref::Mod, source: Source::Machine }
        );
        assert_eq!(
            s(None, Some(&board), Some(&machine)),
            Setting { pref: RoadPref::Auto, source: Source::Board }
        );
        assert_eq!(
            s(Some("hooks"), Some(&board), Some(&machine)),
            Setting { pref: RoadPref::Hooks, source: Source::Seam }
        );
        // A word this build does not know falls through.
        let foreign = json!({ "claude_integration": "socket" });
        assert_eq!(
            s(Some("x"), Some(&foreign), Some(&machine)),
            Setting { pref: RoadPref::Mod, source: Source::Machine }
        );
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
        let v = |a: &str, b: &str| probe(&[a.to_string()], &[b.to_string()], d);
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
            probe(&[d.join("missing").display().to_string()], &[pass], d),
            Verdict::Failed { .. }
        ));
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
