//! `mesimon exec` against the real binary, no tmux and no daemon: the file is
//! applied, `--set` wins over it, a bare command resolves on the FILE's PATH,
//! and a missing file warns rather than killing the pane.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use std::process::Command;

fn launcher() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mesimon"));
    // A deliberately bare parent: whatever the pane sees came from the file.
    c.env_clear().env("PATH", "/usr/bin:/bin").arg("exec");
    c
}

#[test]
fn the_launcher_applies_the_file_then_the_sets_then_execs() {
    let fixture = common::TestFixture::new("exec");
    let dir = fixture.dir.clone();
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let probe = bin.join("msmn-probe");
    std::fs::write(&probe, "#!/bin/sh\nprintf '%s|%s|%s' \"$MSMN_A\" \"$MSMN_B\" \"$PATH\"\n")
        .unwrap();
    std::fs::set_permissions(&probe, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let file = dir.join("shellenv.env");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    std::fs::write(&file, format!("MSMN_A=file\0MSMN_B=file\0PATH={path}\0")).unwrap();

    // `msmn-probe` is a bare name that exists only on the file's PATH.
    let out = launcher()
        .args(["--env", &file.display().to_string(), "--set", "MSMN_B=set", "--", "msmn-probe"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("file|set|{path}"));

    // No file: the command still runs, with a warning where the pane would show it.
    let out = launcher()
        .args(["--env", &dir.join("missing").display().to_string(), "--set", "MSMN_B=set"])
        .args(["--", "/bin/sh", "-c", "printf '%s' \"$MSMN_B\""])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "set");
    assert!(String::from_utf8_lossy(&out.stderr).contains("keeps the tmux server's environment"));

    // No command is a usage error, not a pane running nothing.
    let out = launcher().args(["--env", &file.display().to_string()]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
