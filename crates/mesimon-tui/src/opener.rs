//! Opening a link outside the terminal (T-256): a URL in the browser, a
//! file that is not text in whatever the OS associates with it.
//!
//! The ladder is `MESIMON_OPEN` (a program name or path — the explicit
//! choice, and the seam a test uses so nothing real launches), then the
//! platform's own opener: `open` on macOS; under WSL `wslview` (the Windows
//! browser is the one the user has) and then Windows' own `explorer.exe`
//! (T-1096: it hands the target to ShellExecute, so the machine's default
//! browser takes a URL — a fresh WSL has neither `wslu` nor `xdg-utils`);
//! `xdg-open` on Linux when it is on PATH. Resolved once per process in
//! `lib.rs` — never in `App::new`, so no test app ever finds the developer's
//! browser — and parked on `App::opener`.
//!
//! The launch is DETACHED: null stdio, no terminal handover, no wait. The
//! board says `opening …`, never `opened`: whether the browser took it is
//! not ours to know, the way `asked` never says `sent`. A spawn that fails
//! at once (no such program) is the one error that comes back.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where Windows keeps Explorer, seen from WSL's default drive mount: the
/// rung for a WSL whose PATH leaves the Windows directories out
/// (`appendWindowsPath=false`). `image_paste.rs` finds PowerShell the same way.
const WSL_EXPLORER: &str = "/mnt/c/Windows/explorer.exe";
/// Where WSL's PATH usually has PowerShell, for the case where interop is on
/// but the System32 directory is not on PATH.
pub(crate) const WSL_POWERSHELL: &str =
    "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";

/// Windows PowerShell as a WSL guest finds it: on PATH, or at its usual
/// place when that is a file.
pub(crate) fn powershell() -> Option<PathBuf> {
    which_on_path("powershell.exe").or_else(|| {
        let path = PathBuf::from(WSL_POWERSHELL);
        path.is_file().then_some(path)
    })
}

/// The program that opens a URL or a file here, if there is one.
pub fn find() -> Option<String> {
    find_from(
        std::env::var("MESIMON_OPEN").ok().as_deref(),
        is_wsl(),
        which_on_path,
        Path::new(WSL_EXPLORER).is_file(),
    )
}

fn find_from(
    env: Option<&str>,
    wsl: bool,
    which: impl Fn(&str) -> Option<PathBuf>,
    host_explorer: bool,
) -> Option<String> {
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v.to_string());
    }
    if cfg!(target_os = "macos") {
        return Some("open".into());
    }
    if wsl {
        if which("wslview").is_some() {
            return Some("wslview".into());
        }
        if which("explorer.exe").is_some() {
            return Some("explorer.exe".into());
        }
        if host_explorer {
            return Some(WSL_EXPLORER.into());
        }
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

/// The argv that opens `target` with `prog`. Explorer is a Windows program:
/// a URL it takes as is, a Linux path it cannot see, so that one is spelled
/// the way Windows reaches it (`wslpath -w`'s answer, without the process).
pub fn host_argv(prog: &str, target: &str) -> Vec<String> {
    let target = if is_explorer(prog) {
        windows_path(target, std::env::var("WSL_DISTRO_NAME").ok().as_deref())
            .unwrap_or_else(|| target.to_string())
    } else {
        target.to_string()
    };
    vec![prog.to_string(), target]
}

fn is_explorer(prog: &str) -> bool {
    Path::new(prog).file_name().is_some_and(|n| n.eq_ignore_ascii_case("explorer.exe"))
}

/// A Linux path as Windows spells it: a mounted drive's file is the drive's
/// (`/mnt/c/x` → `C:\x`), anything else is reached through the distro's
/// share (`\\wsl$\Ubuntu\home\…`). A URL, a relative path and a path with no
/// distro name to share it under come back `None`, untouched.
fn windows_path(target: &str, distro: Option<&str>) -> Option<String> {
    if target.contains("://") || !target.starts_with('/') {
        return None;
    }
    let rest = target.strip_prefix("/mnt/").and_then(|r| {
        let (drive, tail) = r.split_once('/').unwrap_or((r, ""));
        let mut chars = drive.chars();
        let letter = chars.next().filter(|c| c.is_ascii_alphabetic())?;
        chars.next().is_none().then(|| (letter.to_ascii_uppercase(), tail))
    });
    let backslashed = |s: &str| s.replace('/', "\\");
    Some(match rest {
        Some((letter, tail)) => format!("{letter}:\\{}", backslashed(tail)),
        None => format!("\\\\wsl$\\{}{}", distro?, backslashed(target)),
    })
}

/// What `mesimon doctor` says: the program and where it came from.
pub fn doctor_line() -> String {
    let env = std::env::var("MESIMON_OPEN").ok().filter(|v| !v.trim().is_empty());
    match (env, find()) {
        (Some(v), _) => format!("{} ($MESIMON_OPEN) — ^k on a ticket", v.trim()),
        (None, Some(p)) => format!("{p} — ^k on a ticket opens a link with it"),
        // Under WSL, Explorer is always there unless the Windows side is
        // walled off: say where to look.
        (None, None) if is_wsl() => {
            "none found — ^k can copy a link but not open one ∙ Windows interop is off (/etc/wsl.conf) or MESIMON_OPEN names a program".into()
        }
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
        assert_eq!(find_from(Some(" firefox "), false, none, false), Some("firefox".into()));
        assert_eq!(find_from(Some("  "), false, none, false), find_from(None, false, none, false));
        if cfg!(target_os = "macos") {
            assert_eq!(find_from(None, false, none, false), Some("open".into()));
        } else {
            assert_eq!(find_from(None, false, none, false), None);
            assert_eq!(find_from(None, false, all, false), Some("xdg-open".into()));
            assert_eq!(find_from(None, true, all, false), Some("wslview".into()));
            // WSL without wslutilities takes Windows' own Explorer, from
            // PATH or from the drive mount, before any Linux opener.
            let xdg_only = |n: &str| (n == "xdg-open").then(|| PathBuf::from("/usr/bin/xdg-open"));
            let explorer = |n: &str| {
                (n == "explorer.exe" || n == "xdg-open").then(|| PathBuf::from(format!("/x/{n}")))
            };
            assert_eq!(find_from(None, true, explorer, false), Some("explorer.exe".into()));
            assert_eq!(find_from(None, true, xdg_only, true), Some(WSL_EXPLORER.into()));
            assert_eq!(find_from(None, true, xdg_only, false), Some("xdg-open".into()));
            assert_eq!(find_from(None, true, none, false), None);
            // Not under WSL, the drive mount means nothing.
            assert_eq!(find_from(None, false, none, true), None);
        }
    }

    #[test]
    fn explorer_takes_a_url_as_is_and_a_file_by_its_windows_name() {
        assert_eq!(windows_path("https://a.test/x?a=1&b=2", Some("Ubuntu")), None);
        assert_eq!(windows_path("src/a.rs", Some("Ubuntu")), None);
        assert_eq!(
            windows_path("/mnt/c/Users/me/x.png", None),
            Some("C:\\Users\\me\\x.png".into())
        );
        assert_eq!(windows_path("/mnt/d", None), Some("D:\\".into()));
        assert_eq!(
            windows_path("/home/me/x.png", Some("Ubuntu")),
            Some("\\\\wsl$\\Ubuntu\\home\\me\\x.png".into())
        );
        // `/mnt/wsl` and `/mnt/data` are no drives: the share reaches them.
        assert_eq!(
            windows_path("/mnt/data/x", Some("Ubuntu")),
            Some("\\\\wsl$\\Ubuntu\\mnt\\data\\x".into())
        );
        assert_eq!(windows_path("/home/me/x.png", None), None);
        // Only Explorer's argv is respelled; every other opener sees the path.
        assert!(is_explorer(WSL_EXPLORER) && is_explorer("Explorer.exe"));
        assert_eq!(host_argv("open", "/mnt/c/x"), vec!["open".to_string(), "/mnt/c/x".to_string()]);
        assert_eq!(host_argv(WSL_EXPLORER, "/mnt/c/x")[1], "C:\\x");
    }

    #[test]
    fn a_missing_program_fails_at_spawn() {
        let err = launch(&["/nonexistent/mesimon-opener".into(), "x".into()], None).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(launch(&[], None).is_err());
    }
}
