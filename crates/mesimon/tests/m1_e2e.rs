//! End-to-end: supervised daemon + real private tmux server + the wire protocol.
//! Covers the M1 acceptance except the interactive handover (manual check).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
mod common;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let stream = UnixStream::connect(sock).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let write = stream.try_clone().expect("clone");
        TestClient { write, read: BufReader::new(stream) }
    }

    fn request(&mut self, command: Command) -> Response {
        let env = Envelope { principal: Principal::Local, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap()).unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(self.read.read_line(&mut line).unwrap(), 0, "daemon disconnected");
            // Skip broadcast events (we never Subscribe here, but be tolerant).
            if let Ok(resp) = serde_json::from_str::<Response>(&line) {
                return resp;
            }
        }
    }
}

fn board_of(resp: Response) -> (mesimon_core::board::Board, Vec<mesimon_core::command::GraceItem>) {
    match resp {
        Response::Board { board, grace, .. } => (board, grace),
        other => panic!("expected board, got {other:?}"),
    }
}

#[test]
fn m1_acceptance_headless() {
    // Skip locally, FAIL in CI: a machine without tmux would otherwise run
    // almost nothing and still report a green suite.
    if !common::require_tmux() {
        return;
    }

    // Per-test dir (pid + name): two e2e binaries or tests must never share a
    // repo — the daemon flock would silently no-op the second one.
    let fixture = common::TestFixture::new("m1");
    let dir = fixture.dir.clone();
    let paths = fixture.paths(&dir);
    let sock = paths.orch_sock();
    let _ = std::fs::remove_file(&sock);

    let repo = dir.clone();
    let daemon = fixture.daemon(&repo);

    // Wait for the socket.
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sock.exists(), "daemon socket never appeared");

    let mut c = TestClient::connect(&sock);

    // Hello + protocol.
    let hello = c.request(Command::Hello { version: PROTOCOL_VERSION, client: "e2e".into() });
    assert!(matches!(hello, Response::Hello { .. }), "{hello:?}");

    // Debug builds expose Mesophon without an environment opt-in, while a
    // fresh board still requires explicit local enablement and pairing.
    let control =
        c.request(Command::Mesophon { action: mesimon_core::mesophon::LocalAction::Status });
    if cfg!(debug_assertions) {
        assert!(
            matches!(control, Response::Mesophon { info } if !info.enabled && !info.connected && info.devices.is_empty())
        );
    } else {
        assert!(matches!(control, Response::Err { .. }));
    }

    // Fresh board: 4 default columns (D33i).
    let (board, _) = board_of(c.request(Command::Snapshot));
    let names: Vec<String> = board.sorted_columns().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["TODO", "IN PROGRESS", "REVIEW", "DONE"]);

    // Create → rename → move.
    c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "first ticket".into(),
        workspace: None,
    });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let t = board.tickets.first().expect("ticket created").clone();
    assert_eq!(t.short_key, "T-1");

    c.request(Command::RenameTicket { id: t.id, title: "renamed".into() });
    c.request(Command::MoveTicket { id: t.id, column: "IN PROGRESS".into(), before: None });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let t2 = board.ticket(t.id).unwrap();
    assert_eq!(t2.title, "renamed");
    assert_eq!(t2.column, "IN PROGRESS");

    // Spawn a bash session → live pane in the private tmux server.
    let r = c.request(Command::SpawnSession {
        ticket: t.id,
        kind: SessionKind::Bash,
        submit_prompt: false,
    });
    assert!(matches!(r, Response::Spawned { .. }), "{r:?}");
    let (board, _) = board_of(c.request(Command::Snapshot));
    let s = board.sessions.first().expect("session record").clone();

    // Focus grant returns attach argv; a second focus on another session would deny.
    let r = c.request(Command::FocusStart { session: s.id });
    let Response::Attach { argv } = r else { panic!("expected attach argv, got {r:?}") };
    assert!(argv.iter().any(|a| a.contains("tmux")));
    c.request(Command::FocusEnd { session: s.id });

    // Gate: not passed initially, gate session attachable; GatePassed persists.
    let r = c.request(Command::GateStatus);
    assert!(matches!(r, Response::Gate { passed: false, attach_argv: Some(_) }), "{r:?}");
    c.request(Command::GatePassed);
    let r = c.request(Command::GateStatus);
    assert!(matches!(r, Response::Gate { passed: true, .. }), "{r:?}");

    // A note rides the ticket through delete + undo: the body is a file
    // under the ticket directory, which delete removes eagerly.
    let r =
        c.request(Command::WriteNote { ticket: t.id, note: None, text: "kept across undo".into() });
    let Response::NoteWritten { note: Some(note) } = r else {
        panic!("expected NoteWritten, got {r:?}")
    };

    // Delete → grace band with the detached session; restore resurrects both.
    c.request(Command::DeleteTicket { id: t.id, discard_worktree: false });
    let (board, grace) = board_of(c.request(Command::Snapshot));
    assert!(board.tickets.is_empty());
    assert_eq!(grace.len(), 1);
    assert_eq!(grace[0].live_sessions, 1);

    c.request(Command::RestoreTicket { id: t.id });
    let (board, grace) = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.len(), 1);
    assert!(grace.is_empty());
    assert_eq!(board.sessions.len(), 1);
    let r = c.request(Command::ReadNote { ticket: t.id, note });
    let Response::Note { text, .. } = r else { panic!("note lost across undo: {r:?}") };
    assert_eq!(text, "kept across undo");

    // Daemon-restart persistence + reconcile: kill daemon, restart, session still linked.
    c.request(Command::KillSession { id: s.id });
    c.request(Command::Shutdown);
    daemon.join().expect("daemon thread");

    let repo = dir.clone();
    let daemon2 = fixture.daemon(&repo);
    for _ in 0..100 {
        if UnixStream::connect(&sock).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut c = TestClient::connect(&sock);
    c.request(Command::Hello { version: PROTOCOL_VERSION, client: "e2e2".into() });
    let (board, _) = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.len(), 1, "board survived restart");
    c.request(Command::Shutdown);
    daemon2.join().expect("daemon2 thread");

    // Fixture owner checks cleanup, including panic and killed-runner paths.
}
