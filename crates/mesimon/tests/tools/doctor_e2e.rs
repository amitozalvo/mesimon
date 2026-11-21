//! `mesimon doctor`, `mesimon doctor --mcp` and `mesimon state compatibility agy`
//! (T-1238): the real binary over a scratch HOME and checkout, proving agy's
//! installation/version rows (`MESIMON_AGY_BIN`), the launch-file disclosure
//! (`<state>/agy/<session>/.agents/hooks.json`, `<state>/agy/<session>.json`),
//! the compatibility manifest lookup, and Promise 2 (doctor changes nothing).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::init_repo;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("msmn-doctor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tree_entries(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

fn run_mesimon(cwd: &Path, home: &Path, env: &[(&str, &Path)], flags: &[(&str, &str)], args: &[&str]) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mesimon"));
    cmd.current_dir(cwd)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("SHELL", "/bin/sh")
        .env("TERM", "xterm-256color");
    for (k, v) in env {
        cmd.env(k, v);
    }
    for (k, v) in flags {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run mesimon");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn doctor_reports_agy_binary_version_and_launch_files_without_mutating_anything() {
    let root = scratch("agy");
    let home = root.join("home");
    let repo = root.join("repo");
    std::fs::create_dir_all(&home).unwrap();
    init_repo(&repo, "README.md", "doctor fixture\n");

    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let fake_agy = bin_dir.join("fake-agy");
    std::fs::write(&fake_agy, "#!/bin/sh\nprintf '1.5.13\\n'\n").unwrap();
    std::fs::set_permissions(&fake_agy, std::fs::Permissions::from_mode(0o700)).unwrap();

    let agy_home = home.join(".gemini/antigravity-cli");
    let before_home = tree_entries(&home);
    let before_repo = tree_entries(&repo);

    // Without MESIMON_AGY or an agy tier, doctor stays quiet about agy.
    let (code, quiet) = run_mesimon(&repo, &home, &[], &[], &["doctor", "agents"]);
    assert!(!quiet.contains("agy version"), "{quiet}");
    let _ = code;

    // With MESIMON_AGY=1 and MESIMON_AGY_BIN, doctor reports both rows.
    let (code, shown) = run_mesimon(
        &repo,
        &home,
        &[("MESIMON_AGY_BIN", &fake_agy), ("MESIMON_AGY_HOME", &agy_home)],
        &[("MESIMON_AGY", "1")],
        &["doctor", "agents", "--verbose"],
    );
    // Claude/Codex are absent from `/usr/bin:/bin`: Claude is default so exit is 1,
    // while agy's own rows are `ok`.
    assert_eq!(code, 1, "{shown}");
    assert!(shown.contains(&format!("ok    agy              {}", fake_agy.display())), "{shown}");
    assert!(shown.contains("ok    agy version      1.5.13"), "{shown}");

    // When the board selects agy and MESIMON_AGY_BIN is missing, doctor fails on agy.
    let board_paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    std::fs::create_dir_all(board_paths.board_dir.join("board")).unwrap();
    let mut board = mesimon_core::board::Board::with_default_columns();
    board.agent_provider = mesimon_core::board::AgentProvider::Agy;
    mesimon_daemon::store::save_columns(&board_paths, &board).unwrap();

    let missing = bin_dir.join("no-such-agy");
    let (code, failed) = run_mesimon(
        &repo,
        &home,
        &[("MESIMON_AGY_BIN", &missing)],
        &[("MESIMON_AGY", "1")],
        &["doctor", "agents", "--verbose"],
    );
    assert_eq!(code, 1, "{failed}");
    assert!(failed.contains("FAIL  agy"), "{failed}");
    assert!(failed.contains("selected for new sessions"), "{failed}");
    assert!(failed.contains("agy uses its native AGENTS.md and GEMINI.md loading"), "{failed}");

    // With a working MESIMON_AGY_BIN on an agy-selected board, `doctor agents` exits 0!
    let (code, green) = run_mesimon(
        &repo,
        &home,
        &[("MESIMON_AGY_BIN", &fake_agy)],
        &[("MESIMON_AGY", "1")],
        &["doctor", "agents"],
    );
    assert_eq!(code, 0, "{green}");
    assert!(green.contains("ok    agy"), "{green}");
    assert!(green.contains("ok    agy version      1.5.13"), "{green}");

    // `doctor --mcp` names `<state>/agy/<session>/.agents/hooks.json`,
    // `<state>/agy/<session>.json`, `~/.gemini/`, and the agy write gate.
    let before_mcp_home = tree_entries(&home);
    let before_mcp_repo = tree_entries(&repo);
    let (code, mcp) = run_mesimon(&repo, &home, &[], &[], &["doctor", "--mcp"]);
    assert_eq!(code, 0, "{mcp}");
    assert!(mcp.is_ascii(), "{mcp}");
    for needle in [
        "/agy/<session>/.agents/hooks.json",
        "/agy/<session>.json",
        "/agy/<session>/.agents/mcp_config.json",
        "~/.gemini/",
        "write_to_file, write_file",
    ] {
        assert!(mcp.contains(needle), "doctor --mcp missing {needle:?}:\n{mcp}");
    }
    assert_eq!(tree_entries(&home), before_mcp_home, "doctor --mcp wrote under HOME");
    assert_eq!(tree_entries(&repo), before_mcp_repo, "doctor --mcp wrote under repo");
    assert!(!agy_home.exists(), "doctor touched ~/.gemini");
    assert_eq!(tree_entries(&home), before_home, "doctor wrote under HOME");
    let _ = before_repo;

    // `mesimon state compatibility agy <version>` reads the compiled manifest offline.
    let (code, compat) =
        run_mesimon(&repo, &home, &[], &[], &["state", "compatibility", "agy", "1.5.13"]);
    assert_eq!(code, 0, "{compat}");
    let parsed: serde_json::Value = serde_json::from_str(&compat).unwrap();
    assert_eq!(parsed["provider"], "agy");
    assert_eq!(parsed["status"], "partially_observed");
    assert_eq!(parsed["auto_updates_at_launch"], true);

    let (code, unknown) =
        run_mesimon(&repo, &home, &[], &[], &["state", "compatibility", "agy", "9.9.9"]);
    assert_eq!(code, 0, "{unknown}");
    let parsed: serde_json::Value = serde_json::from_str(&unknown).unwrap();
    assert_eq!(parsed["status"], "untested");

    let _ = std::fs::remove_dir_all(&root);
}
