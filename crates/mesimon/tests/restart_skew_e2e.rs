//! Build-skew restart e2e: a client newer than the running daemon shuts that
//! daemon down and brings a fresh one up on the new binary, before the board
//! draws anything.
//!
//! This is the one test that drives the REAL client path
//! (`mesimon_tui::client::Client::connect`). Every other e2e hand-rolls a
//! `TestClient` over a raw socket, so none of them touch `open`, the
//! handshake, or the restart. Both processes here are the real built binary:
//! the daemon via `CARGO_BIN_EXE_mesimon`, the respawn via
//! `MESIMON_DAEMON_BIN` (the test binary has no `daemon` subcommand).
//!
//! No tmux needed: the daemon only shells out to tmux when a session spawns,
//! and this test never spawns one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use mesimon_core::command::{Command as Cmd, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;

mod common;

/// One Hello over the socket, without the TUI client — so the assertions never
/// depend on the machinery under test.
fn hello(sock: &Path) -> Option<(u32, String)> {
    let stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut w = stream.try_clone().ok()?;
    let env = Envelope {
        principal: Principal::Local,
        command: Cmd::Hello { version: PROTOCOL_VERSION, client: "skew-e2e".into() },
    };
    writeln!(w, "{}", serde_json::to_string(&env).unwrap()).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    match serde_json::from_str::<Response>(&line).ok()? {
        Response::Hello { daemon_pid, build, .. } => Some((daemon_pid, build)),
        _ => None,
    }
}

fn wait_for(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

fn spawn_daemon_claiming(fixture: &common::TestFixture, dir: &Path, build: &str) -> common::support::TestProcess {
    fixture.set_env("MESIMON_FAKE_BUILD", build);
    fixture.set_env("MESIMON_DETACHED", "1");
    let child = fixture.daemon(dir);
    fixture.remove_env("MESIMON_FAKE_BUILD");
    child
}

/// Stop whatever is listening and take the scratch tree down.
fn teardown(sock: &Path) {
    if let Ok(mut s) = UnixStream::connect(sock) {
        let env = Envelope { principal: Principal::Local, command: Cmd::Shutdown };
        let _ = writeln!(s, "{}", serde_json::to_string(&env).unwrap());
    }
    wait_for(Duration::from_secs(5), || !sock.exists());
}

/// Exercise the real TUI client in a supervised process, so its replacement
/// daemon inherits the fixture's fake shell/agent settings, not the user's rc.
fn connect_client(fixture: &common::TestFixture, dir: &Path) {
    fixture.set_env("MESIMON_DAEMON_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_TEST_REPO", dir);
    fixture.spawn(vec![std::env::current_exe().unwrap().to_str().unwrap().into(),
        "--exact".into(), "client_connect_fixture_helper".into(), "--ignored".into()])
        .join().expect("isolated TUI client");
}

#[test]
#[ignore = "subprocess helper; exercised by both restart-skew tests"]
fn client_connect_fixture_helper() {
    let dir = std::env::var("MESIMON_TEST_REPO").expect("fixture-only helper");
    let _client = mesimon_tui::client::Client::connect(std::path::Path::new(&dir)).expect("connect");
}

#[test]
fn newer_client_restarts_a_stale_daemon() {
    let fixture = common::TestFixture::new("skew");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let paths = fixture.paths(&dir);
    let sock = paths.orch_sock();
    let _ = std::fs::remove_file(&sock);

    let old = spawn_daemon_claiming(&fixture, &dir, "0.0.1");
    assert!(wait_for(Duration::from_secs(10), || sock.exists()), "daemon socket never appeared");

    let (old_pid, old_build) = hello(&sock).expect("hello from the stale daemon");
    assert_eq!(old_build, "0.0.1", "the stale daemon should report its fake build");

    // The whole point: constructing the client is what restarts the daemon.
    connect_client(&fixture, &dir);

    let (new_pid, new_build) = hello(&sock).expect("hello from the replacement daemon");
    assert_ne!(new_pid, old_pid, "a different process must be serving now");
    assert_eq!(
        new_build,
        env!("CARGO_PKG_VERSION"),
        "the replacement must run this build, not the stale one"
    );

    // The old daemon really exited, rather than being left behind holding the
    // flock — the failure that would make the next start silently no-op.
    assert!(
        wait_for(Duration::from_secs(5), || old.try_wait().ok().flatten().is_some()),
        "the stale daemon did not exit"
    );

    teardown(&sock);
}

/// The rule that stops a restart war: two TUIs of different builds on one repo
/// would otherwise take turns restarting the daemon to their own version,
/// forever. Only a strictly newer client acts.
#[test]
fn older_client_leaves_a_newer_daemon_alone() {
    let fixture = common::TestFixture::new("skew-new");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let paths = fixture.paths(&dir);
    let sock = paths.orch_sock();
    let _ = std::fs::remove_file(&sock);

    let newer = spawn_daemon_claiming(&fixture, &dir, "99.0.0");
    assert!(wait_for(Duration::from_secs(10), || sock.exists()), "daemon socket never appeared");
    let (pid_before, build) = hello(&sock).expect("hello from the newer daemon");
    assert_eq!(build, "99.0.0");

    connect_client(&fixture, &dir);

    let (pid_after, build_after) = hello(&sock).expect("hello after connect");
    assert_eq!(pid_after, pid_before, "an older client must not restart a newer daemon");
    assert_eq!(build_after, "99.0.0", "the newer daemon must still be the one serving");
    assert!(newer.try_wait().ok().flatten().is_none(), "it must still be running");

    teardown(&sock);
    let _ = newer.wait();
}
