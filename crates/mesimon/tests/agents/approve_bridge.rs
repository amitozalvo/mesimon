//! `mesimon approve`'s hold, against a socket standing in for the daemon
//! (T-632): no tmux, no daemon. A round that runs out under `--renew` says
//! so by its exit; a wait the daemon closed, or a hook set's run, ends with
//! nothing; a decision is printed whole.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};

use mesimon_core::mesophon::PERMISSION_RENEW_EXIT;

const BODY: &str = r#"{"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash","tool_input":{"command":"ls"}}"#;

/// Runs `mesimon approve` with `extra` against a listener that hands the
/// accepted stream and its header to `daemon`; returns the exit code and
/// stdout.
fn approve(
    name: &str,
    extra: &[&str],
    daemon: impl FnOnce(std::os::unix::net::UnixStream, serde_json::Value) + Send + 'static,
) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("msmn-approve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("hook.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // The whole frame, header and payload, as the daemon reads it.
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let (mut header, mut payload) = (String::new(), String::new());
        reader.read_line(&mut header).unwrap();
        reader.read_line(&mut payload).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&payload).unwrap()["tool_name"],
            "Bash"
        );
        daemon(stream, serde_json::from_str(&header).unwrap());
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["approve", "--sock", sock.to_str().unwrap(), "--session", "s"])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(BODY.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (out.status.code().unwrap(), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn a_renewing_round_that_runs_out_says_so_and_prints_nothing() {
    let (code, out) = approve("round", &["--hold", "1", "--renew"], |stream, header| {
        assert_eq!(header["event"], "RemotePermission");
        assert_eq!(header["reason"], "renew");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        drop(stream);
    });
    assert_eq!((code, out.as_str()), (PERMISSION_RENEW_EXIT, ""));
}

#[test]
fn a_hook_set_s_run_that_runs_out_ends_with_nothing() {
    let (code, out) = approve("hookset", &["--hold", "1"], |stream, header| {
        assert!(header.get("reason").is_none());
        std::thread::sleep(std::time::Duration::from_millis(1500));
        drop(stream);
    });
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn a_wait_the_daemon_closed_ends_the_hold() {
    let (code, out) = approve("closed", &["--hold", "5", "--renew"], |stream, _| drop(stream));
    assert_eq!((code, out.as_str()), (0, ""));
}

#[test]
fn a_phone_s_answer_is_printed_as_the_dialog_s_decision() {
    let (code, out) = approve("answer", &["--hold", "5", "--renew"], |mut stream, _| {
        stream.write_all(b"\"allow\"").unwrap();
    });
    assert_eq!(code, 0);
    let out: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "allow");
}
