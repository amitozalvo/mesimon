//! The mod mesimon lays, against the real Claude Code (T-574):
//! `claude plugin validate` (what `auto` gates the mod road on) and `claude
//! plugin test` (the mod's own `register.test.ts`) on the folder exactly as
//! the daemon lays it. Needs `claude` on PATH, and no sign-in: neither
//! command talks to a model. Without it the test says SKIPPED, and
//! `MESIMON_REQUIRE_CLAUDE=1` (the release gate) turns that into a failure.
//!
//! While Claude Code has mods turned off (2.1.288's remote flag, T-598),
//! `claude plugin test` refuses in the flag's words before any test runs.
//! The validate half still runs, and the test half says SKIPPED with the
//! refusal, under `MESIMON_REQUIRE_CLAUDE=1` too: what the gate requires is
//! that `claude` is here, and a refusal is Claude Code's call, which `auto`
//! answers by taking the hook set. The load probe the daemon asks the same
//! question with is run here as well.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

fn claude() -> Option<String> {
    let found = Command::new("claude").arg("--version").output().is_ok_and(|o| o.status.success());
    if found {
        return Some("claude".into());
    }
    assert!(
        std::env::var_os("MESIMON_REQUIRE_CLAUDE").is_none(),
        "claude is required (MESIMON_REQUIRE_CLAUDE=1) but is not on PATH"
    );
    eprintln!("SKIPPED: claude is not on PATH, so the laid mod was not validated");
    None
}

fn paths(dir: &Path) -> mesimon_daemon::Paths {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let mut paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    paths.state_dir = dir.join("state");
    paths
}

/// The words `claude plugin test` refuses in while mods are off.
const MODS_OFF: &str = "hooks modules are turned off";

fn run(bin: &str, args: &[&str], folder: &Path) -> (bool, String) {
    let out = Command::new(bin).args(args).arg(folder).output().expect("run claude");
    let text =
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn the_laid_mod_validates_and_its_tests_pass_on_this_claude_code() {
    let Some(bin) = claude() else { return };
    let dir = std::env::temp_dir().join(format!("msmn-modplugin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let paths = paths(&dir);
    let folder = mesimon_daemon::modroad::lay(&paths).unwrap();
    let (ok, text) = run(&bin, &["plugin", "validate"], &folder);
    assert!(ok, "claude plugin validate refused the mod:\n{text}");
    for env in [
        "MESIMON_MOD_BIN",
        "MESIMON_MOD_HOOK_SOCK",
        "MESIMON_MOD_ORCH_SOCK",
        "MESIMON_MOD_SESSION",
        "MESIMON_MOD_GATE_BOARD",
        "MESIMON_MOD_GATE_STATE",
        "MESIMON_MOD_GATE_ALLOW",
    ] {
        assert!(text.contains(env), "validate lists the variables the mod reads: {env}\n{text}");
    }
    // The load probe answers what the mod's own tests do: both run, or
    // both are refused in the same words.
    let probe = mesimon_daemon::modroad::lay_load_probe(&paths).unwrap();
    let (probe_ok, probe_text) = run(&bin, &["plugin", "test"], &probe);
    let (ok, text) = run(&bin, &["plugin", "test"], &folder);
    if !ok && text.contains(MODS_OFF) {
        assert!(!probe_ok && probe_text.contains(MODS_OFF), "the probe disagrees:\n{probe_text}");
        let words = text.lines().find(|l| l.contains(MODS_OFF)).unwrap_or(MODS_OFF).trim();
        eprintln!("SKIPPED: the mod validated; its tests did not run: {words}");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    assert!(ok, "claude plugin test failed:\n{text}");
    assert!(text.contains(" 0 fail"), "{text}");
    assert!(probe_ok, "claude plugin test refused the load probe:\n{probe_text}");
    assert!(probe_text.contains(" 1 pass") && probe_text.contains(" 0 fail"), "{probe_text}");
    let _ = std::fs::remove_dir_all(&dir);
}
