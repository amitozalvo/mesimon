//! Opening a link outside the terminal (T-256): a URL in the browser, a
//! file that is not text in whatever the OS associates with it.
//!
//! The ladder is `MESIMON_OPEN` (a program name or path — the explicit
//! choice, and the seam a test uses so nothing real launches), then the
//! platform's own opener: `open` on macOS, `wslview` under WSL (the Windows
//! browser is the one the user has), `xdg-open` on Linux when it is on PATH.
//! Resolved once per process in `lib.rs` — never in `App::new`, so no test
//! app ever finds the developer's browser — and parked on `App::opener`.
//!
//! The launch is DETACHED: null stdio, no terminal handover, no wait. The
//! board says `opening …`, never `opened`: whether the browser took it is
//! not ours to know, the way `asked` never says `sent`. A spawn that fails
//! at once (no such program) is the one error that comes back.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The program that opens a URL or a file here, if there is one.
pub fn find() -> Option<String> {
    find_from(std::env::var("MESIMON_OPEN").ok().as_deref(), is_wsl(), which_on_path)
}

fn find_from(
    env: Option<&str>,
    wsl: bool,
    which: impl Fn(&str) -> Option<PathBuf>,
) -> Option<String> {
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v.to_string());
    }
    if cfg!(target_os = "macos") {
        return Some("open".into());
    }
    if wsl && which("wslview").is_some() {
        return Some("wslview".into());
    }
    which("xdg-open").map(|_| "xdg-open".into())
}

/// `/proc/version` names Microsoft under WSL — `doctor`'s rule. Shared with
/// `caffeine.rs`, which asks the same question for a different reason: in
/// there, no Linux call reaches the host that decides when to sleep.
pub(crate) fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/version")
            .is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"))
}

/// Shared with `caffeine.rs` rather than copied a third time.
pub(crate) fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| std::fs::metadata(cand).is_ok_and(|m| m.is_file()))
}

/// What `mesimon doctor` says: the program and where it came from.
pub fn doctor_line() -> String {
    let env = std::env::var("MESIMON_OPEN").ok().filter(|v| !v.trim().is_empty());
    match (env, find()) {
        (Some(v), _) => format!("{} ($MESIMON_OPEN) — ^k on a ticket", v.trim()),
        (None, Some(p)) => format!("{p} — ^k on a ticket opens a link with it"),
        (None, None) => {
            "none found — ^k can copy a link but not open one ∙ MESIMON_OPEN names a program".into()
        }
    }
}

/// Run `argv` detached: null stdio, the child reaped on a thread of its own
/// so it never sits as a zombie for the life of the board. `open`/`xdg-open`
/// return at once; a browser they started is not ours.
pub fn launch(argv: &[String], cwd: Option<&Path>) -> std::io::Result<()> {
    let (prog, rest) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "nothing to run"))?;
    let mut cmd = Command::new(prog);
    cmd.args(rest).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn()?;
    std::thread::Builder::new()
        .name("mesimon-open".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_wins_and_the_platform_answers_otherwise() {
        let none = |_: &str| None;
        let all = |n: &str| Some(PathBuf::from(format!("/usr/bin/{n}")));
        assert_eq!(find_from(Some(" firefox "), false, none), Some("firefox".into()));
        assert_eq!(find_from(Some("  "), false, none), find_from(None, false, none));
        if cfg!(target_os = "macos") {
            assert_eq!(find_from(None, false, none), Some("open".into()));
        } else {
            assert_eq!(find_from(None, false, none), None);
            assert_eq!(find_from(None, false, all), Some("xdg-open".into()));
            assert_eq!(find_from(None, true, all), Some("wslview".into()));
            // WSL without wslutilities falls through to xdg-open.
            let xdg_only = |n: &str| (n == "xdg-open").then(|| PathBuf::from("/usr/bin/xdg-open"));
            assert_eq!(find_from(None, true, xdg_only), Some("xdg-open".into()));
        }
    }

    #[test]
    fn a_missing_program_fails_at_spawn() {
        let err = launch(&["/nonexistent/mesimon-opener".into(), "x".into()], None).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(launch(&[], None).is_err());
    }
}
