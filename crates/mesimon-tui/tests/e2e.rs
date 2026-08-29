//! End-to-end: in-process daemon + real private tmux server + the wire protocol.
//! Covers the M1 acceptance except the interactive handover (manual check).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
use mesimon_daemon::Paths;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let stream = UnixStream::connect(sock).expect("connect");
        let write = stream.try_clone().expect("clone");
        TestClient { write, read: BufReader::new(stream) }
    }

    fn request(&mut self, command: Command) -> Response {
        let env = Envelope { principal: Principal::Local, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap()).unwrap();
        let mut line = String::new();
        loop {
            line.clear();
            self.read.read_line(&mut line).unwrap();
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
    if std::process::Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("tmux missing; skipping");
        return;
    }

    // Per-test dir (pid + name): two e2e binaries or tests must never share a
    // repo — the daemon flock would silently no-op the second one.
    let dir = PathBuf::from(format!("/tmp/msmn-e2e-m1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths = Paths::for_repo(&dir).unwrap();
    let sock = paths.orch_sock();
    let _ = std::fs::remove_file(&sock);

    let repo = dir.clone();
    let daemon = std::thread::spawn(move || {
        mesimon_daemon::run_foreground(&repo).expect("daemon run");
    });

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

    // Fresh board: 4 default columns (D33i).
    let (board, _) = board_of(c.request(Command::Snapshot));
    let names: Vec<String> = board.sorted_columns().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["TODO", "IN PROGRESS", "REVIEW", "DONE"]);

    // Create → rename → move.
    c.request(Command::CreateTicket { column: "TODO".into(), title: "first ticket".into() });
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
    let r = c.request(Command::SpawnSession { ticket: t.id, kind: SessionKind::Bash });
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

    // Delete → grace band with the detached session; restore resurrects both.
    c.request(Command::DeleteTicket { id: t.id });
    let (board, grace) = board_of(c.request(Command::Snapshot));
    assert!(board.tickets.is_empty());
    assert_eq!(grace.len(), 1);
    assert_eq!(grace[0].live_sessions, 1);

    c.request(Command::RestoreTicket { id: t.id });
    let (board, grace) = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.len(), 1);
    assert!(grace.is_empty());
    assert_eq!(board.sessions.len(), 1);

    // Daemon-restart persistence + reconcile: kill daemon, restart, session still linked.
    c.request(Command::KillSession { id: s.id });
    c.request(Command::Shutdown);
    daemon.join().expect("daemon thread");

    let repo = dir.clone();
    let daemon2 = std::thread::spawn(move || {
        mesimon_daemon::run_foreground(&repo).expect("daemon rerun");
    });
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

    // Cleanup: private tmux server + dirs.
    let _ = std::process::Command::new("tmux")
        .args(["-S", paths.tmux_sock().to_str().unwrap(), "kill-server"])
        .output();
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&paths.state_dir).ok();
    std::fs::remove_dir_all(&paths.rt_dir).ok();
}
