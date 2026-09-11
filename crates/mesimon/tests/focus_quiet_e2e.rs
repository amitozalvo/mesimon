//! Who is inside the pane the board handed its terminal to (T-299).
//!
//! While a handover is running the board can see nothing: focus reporting is
//! off for the duration and every keystroke reaches tmux instead of the TUI.
//! `Command::FocusQuiet` is the one way back to that fact, and it is a fork
//! against a real tmux server — so it is worth an end-to-end test rather than
//! a mocked one. Real tmux, in-process daemon, a real pane, a real client
//! attached to it.
//!
//! The client here is a CONTROL-MODE one (`tmux -C attach`), because that is
//! the only kind that attaches without a pty. tmux lists it and stamps its
//! `client_activity` like any other, which is what this proves. That the
//! stamp then MOVES when somebody types is tmux's own contract, verified by
//! hand against a pty client on tmux 3.6a; the arithmetic on top of it is
//! unit-tested in `core::notify`.
//!
//! And the token itself: it is exclusive, and a board that dies inside the
//! pane it took the token for still gives it back (the second test).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::process::Stdio;
use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};

fn quiet(c: &mut TestClient) -> Option<u64> {
    match c.request(Command::FocusQuiet) {
        Response::FocusQuiet { quiet_ms } => quiet_ms,
        other => panic!("focus quiet failed: {other:?}"),
    }
}

#[test]
fn the_daemon_says_how_long_the_attached_pane_has_been_quiet() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("focusquiet");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let tmux_sock = paths.tmux_sock();

    fixture.set_env("SHELL", "/bin/sh");
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));

    let daemon = fixture.daemon(&repo.clone());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "focusquiet".into()
        }),
        Response::Hello { .. }
    ));

    // Nothing focused: the daemon holds no token, so there is no pane to ask
    // about. `None` rather than a refusal — the caller reads every way of
    // not knowing as "nobody is there".
    assert_eq!(quiet(&mut c), None, "nothing focused answers None");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "focus quiet".into(),
        workspace: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid16 = sid.simple().to_string()[..16].to_string();

    // The gate stands between a board and its first attach (D20), and this
    // test is not about the ceremony.
    let _ = c.request(Command::GatePassed);
    assert!(
        matches!(c.request(Command::FocusStart { session: sid }), Response::Attach { .. }),
        "the focus token is what names the pane"
    );

    // Focused, but nobody has attached a client to it: the TUI has been
    // granted the argv and has not exec'd tmux yet. An empty listing is
    // nobody there, which is not an error.
    assert_eq!(quiet(&mut c), None, "a granted attach with no client is nobody");

    // Now somebody is in there.
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &sid16])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");

    let deadline = Instant::now() + Duration::from_secs(10);
    let ms = loop {
        if let Some(ms) = quiet(&mut c) {
            break ms;
        }
        assert!(Instant::now() < deadline, "the attached client was never seen");
        std::thread::sleep(Duration::from_millis(200));
    };
    // Seconds is tmux's resolution, so a client that just attached reads as
    // zero or one — what matters is that it is nowhere near the window the
    // presence rule judges it against.
    assert!(ms < 10_000, "a client that just attached is not quiet: {ms}");

    // And when they leave, the answer goes back to nobody — which is what
    // lets the ticket start speaking again.
    client.kill().unwrap();
    let _ = client.wait();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if quiet(&mut c).is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "a detached client kept answering");
        std::thread::sleep(Duration::from_millis(200));
    }

    // Releasing the token is releasing the question with it.
    let _ = c.request(Command::FocusEnd { session: sid });
    assert_eq!(quiet(&mut c), None);

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}

#[test]
fn shutdown_reply_survives_process_exit_with_concurrent_snapshot_clients() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("shutdownreply");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);
    for iteration in 0..8 {
        let daemon = fixture.daemon(&repo);
        let mut c = TestClient::connect(&paths.orch_sock());
        assert!(matches!(
            c.request(Command::Hello {
                version: mesimon_core::command::PROTOCOL_VERSION,
                client: format!("shutdown reply {iteration}"),
            }),
            Response::Hello { .. }
        ));
        let ready = std::sync::Arc::new(std::sync::Barrier::new(5));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let ready = ready.clone();
                let socket = paths.orch_sock();
                std::thread::spawn(move || {
                    let mut reader = TestClient::connect(&socket);
                    let _ = board_of(reader.request(Command::Snapshot));
                    ready.wait();
                    for _ in 0..8 {
                        if reader
                            .try_send(mesimon_core::Principal::Local, Command::Snapshot)
                            .is_none()
                        {
                            break;
                        }
                    }
                })
            })
            .collect();
        ready.wait();
        // The daemon's main thread exits as soon as shutdown completes. Its
        // requesting client must receive the complete response before then,
        // even when other client threads are runnable at the same time.
        assert!(matches!(c.request(Command::Shutdown), Response::Ok));
        daemon.join().unwrap();
        for reader in readers {
            reader.join().unwrap();
        }
    }
}

/// A board killed while it is inside the pane — cmd+W on the terminal window,
/// a crash, a `kill` — never sends the `FocusEnd` that comes after a handover
/// it will not return from. The token used to strand there for the life of the
/// daemon: every later attach, from that board or the next one, answered
/// "another session is focused". The connection dying is the release.
#[test]
fn a_board_that_dies_inside_the_pane_gives_the_focus_token_back() {
    let Some(h) = Harness::boot_with_env("focusdrop", None, &[("SHELL", "/bin/sh")]) else {
        return;
    };
    let mut setup = h.client("setup");
    let mut sids = Vec::new();
    for title in ["one", "two"] {
        let ticket = match setup.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
        }) {
            Response::Created { id, .. } => id,
            other => panic!("create: {other:?}"),
        };
        match setup.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Bash,
            submit_prompt: false,
        }) {
            Response::Spawned { id, .. } => sids.push(id),
            other => panic!("spawn: {other:?}"),
        }
    }

    // One board takes the token, and while it lives the token is its own.
    let mut first = h.client("board-one");
    assert!(matches!(
        first.request(Command::FocusStart { session: sids[0] }),
        Response::Attach { .. }
    ));
    let mut second = h.client("board-two");
    match second.request(Command::FocusStart { session: sids[1] }) {
        Response::Err { message } => assert!(message.contains("focused"), "{message}"),
        other => panic!("the token is exclusive while its board lives: {other:?}"),
    }

    // The board goes without a word. The next attach works — the daemon never
    // restarted, and nothing but this dropped connection said so.
    drop(first);
    wait_until(Duration::from_secs(10), "the dead board's token to come back", || {
        matches!(second.request(Command::FocusStart { session: sids[1] }), Response::Attach { .. })
    });
    assert!(matches!(second.request(Command::FocusEnd { session: sids[1] }), Response::Ok));
}
