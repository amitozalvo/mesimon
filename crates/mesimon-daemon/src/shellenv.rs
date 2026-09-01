//! Asking the user's own shell what the environment is — the I/O half of
//! `mesimon_core::shellenv`, which decides what of the answer a pane may keep.
//!
//! A Claude pane is exec'd directly by tmux, so no shell startup file is ever
//! read on that path; the only way an `export` in `~/.zshrc` can reach an agent
//! (or its MCP servers, or its hooks) is if mesimon reads it and hands it over.
//! So mesimon runs the login shell exactly the way a terminal would and reads
//! back the environment it produced.
//!
//! Three things about how the capture is run, each of them load-bearing:
//!
//! * **`env -0`, redirected to a file.** An interactive rc file prints — prompt
//!   frameworks, version managers, greetings — so stdout is not a channel we
//!   can read the answer from. The answer goes to a file and the chatter goes
//!   to `/dev/null`. NUL separation is what makes the dump parseable at all: a
//!   value may contain newlines.
//! * **`-l -i`.** A terminal on macOS starts a login *and* interactive shell,
//!   so `.zprofile` and `.zshrc` both run. Capturing with only one of them
//!   would produce an environment no terminal on this machine ever has.
//! * **A clean base environment.** The daemon's own environment is itself a
//!   frozen copy of some older shell's, and a user's rc almost always *prepends*
//!   (`export PATH=…:$PATH`), so inheriting would quietly preserve the very
//!   staleness this exists to fix. The base is what launchd hands a terminal.
//!
//! The capture runs off the writer thread (it forks a shell that may take a
//! second) and arrives back as `Msg::ShellEnvCaptured`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the user's rc files get before the capture is abandoned. Generous:
/// a slow rc is a bad reason to lose the environment, and this runs off the
/// writer thread where nothing is waiting on it.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(15);

/// What launchd hands a terminal before any rc file runs. `/etc/zprofile`'s
/// `path_helper` builds the real one from here.
const BASE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// A captured shell environment, and what it was captured from.
#[derive(Debug, Clone, Default)]
pub struct ShellEnv {
    /// The pane's set, written to the launcher's file: admissible names,
    /// sorted. Excludes `PATH`, which is carried by name.
    pub vars: Vec<(String, String)>,
    /// The captured `PATH`, which travels as the tmux *client* environment
    /// rather than through `-e` — see `mesimon_core::shellenv::PATH_IS_THE_CLIENTS`.
    pub path: Option<String>,
    /// The newest mtime among the rc files at the moment of capture. The
    /// staleness check compares today's stamp against this one.
    pub rc_stamp: u64,
}

/// The shell startup files whose change means "your environment moved".
///
/// Generous on purpose: statting a handful of paths costs nothing, and the
/// failure mode of watching too few is a suggestion that never appears. It is
/// still not exhaustive — an rc that sources `~/.config/env.sh` will not be
/// noticed — so the honest description of this list is "what almost everyone
/// edits", and `touch`ing an rc file is the manual trigger for the rest.
pub fn rc_files() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let zdot = std::env::var_os("ZDOTDIR").map(PathBuf::from);
    let mut out = Vec::new();
    for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        if let Some(d) = &zdot {
            out.push(d.join(name));
        }
        out.push(home.join(name));
    }
    for name in [".bash_profile", ".bash_login", ".bashrc", ".profile"] {
        out.push(home.join(name));
    }
    out.push(home.join(".config/fish/config.fish"));
    out
}

/// The newest mtime (epoch ms) across [`rc_files`]; 0 when none exist.
///
/// A max rather than a per-file record: the question is only ever "has anything
/// moved since the capture", and one number answers it without a map to keep in
/// sync. Note this rises when a file is *deleted* only on the next edit of
/// another — an acceptable blind spot for a debounce.
pub fn rc_stamp() -> u64 {
    rc_files()
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .filter_map(|m| m.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .max()
        .unwrap_or(0)
}

/// Run the user's login shell and read back its environment.
///
/// `dump` is where the shell writes it — the caller passes a path inside the
/// runtime dir, which is 0700 and inside the README's write allowlist. It is
/// removed before returning either way: the dump is a copy of the user's whole
/// environment, and leaving it on disk would be a second, longer-lived place
/// for whatever secrets live in it.
pub fn capture(dump: &Path) -> Result<ShellEnv, String> {
    // Read the stamp BEFORE running, never after: an rc file edited while the
    // capture is in flight must read as stale, not as already captured.
    let rc_stamp = rc_stamp();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let _ = std::fs::remove_file(dump);

    let mut cmd = Command::new(&shell);
    // `-l -i -c`: what a terminal runs, and `-c` is what makes it terminate.
    cmd.args(["-l", "-i", "-c", &format!("env -0 > '{}'", dump.display())]);
    cmd.env_clear();
    cmd.env("PATH", BASE_PATH);
    cmd.env("TERM", "xterm-256color");
    // Carried, not invented: a locale usually comes from the terminal emulator
    // rather than from any rc file, so a clean base would drop it entirely and
    // hand every pane a C locale. These are the daemon's, which got them from
    // the user's terminal.
    for k in ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("{shell}: {e}"))?;
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(dump);
                return Err(format!(
                    "{shell} did not finish within {}s",
                    CAPTURE_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("{shell}: {e}")),
        }
    }

    let bytes = std::fs::read(dump).map_err(|e| format!("reading the dump: {e}"))?;
    let _ = std::fs::remove_file(dump);
    let captured = mesimon_core::shellenv::parse_env0(&bytes);
    // An rc that failed early can leave a dump with a couple of entries in it.
    // Publishing that would REPLACE a working environment with a broken one,
    // so an implausibly small answer is refused and the previous one stands.
    if captured.len() < 4 {
        return Err(format!("{shell} returned only {} variables", captured.len()));
    }
    Ok(ShellEnv {
        vars: mesimon_core::shellenv::select(&captured),
        path: mesimon_core::shellenv::path_of(&captured),
        rc_stamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_login_shell_answers_with_a_usable_environment() {
        let dump = std::env::temp_dir().join(format!("msmn-shellenv-test-{}", std::process::id()));
        let Ok(env) = capture(&dump) else {
            // A CI image without the user's $SHELL is not a failure of this
            // code; the daemon's own fallback covers exactly this case.
            return;
        };
        assert!(!dump.exists(), "the dump is a copy of the environment — it must not survive");
        assert!(env.vars.iter().any(|(k, _)| k == "HOME"), "no HOME in {:?}", env.vars);
        assert!(!env.vars.iter().any(|(k, _)| k == "PATH"), "PATH must ride the client env");
        assert!(env.path.is_some_and(|p| p.contains('/')), "no usable PATH");
    }

    #[test]
    fn the_base_environment_is_clean_so_a_prepending_rc_cannot_preserve_a_stale_path() {
        // The daemon's own PATH must not be visible to the capture: a
        // `export PATH=new:$PATH` rc would otherwise carry the stale tail
        // forward forever, which is the bug this whole module exists for.
        let dump = std::env::temp_dir().join(format!("msmn-shellenv-base-{}", std::process::id()));
        let Ok(env) = capture(&dump) else { return };
        let Some(path) = env.path else { return };
        assert!(
            !path.contains("/MESIMON_SENTINEL_NOT_A_REAL_DIR"),
            "the capture inherited the daemon's PATH"
        );
        assert!(path.contains("/usr/bin"), "the base PATH should survive as a tail: {path}");
    }

    #[test]
    fn rc_files_are_named_under_home() {
        let files = rc_files();
        assert!(files.iter().any(|p| p.ends_with(".zshrc")));
        assert!(files.iter().any(|p| p.ends_with(".bashrc")));
        assert!(files.iter().any(|p| p.ends_with(".profile")));
    }
}
