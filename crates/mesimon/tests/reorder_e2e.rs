//! Reorder inside a column: the MOVE ghost's drop is the one gesture that
//! means "same column, new slot", and it arrives as `MoveTicket` with the
//! column it is already in. In-process daemon over the real wire.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use mesimon_core::board::Board;
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::Principal;

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let stream = UnixStream::connect(sock).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let read = BufReader::new(stream.try_clone().unwrap());
        Self { write: stream, read }
    }

    fn request(&mut self, command: Command) -> Response {
        let env = Envelope { principal: Principal::Local, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap()).unwrap();
        loop {
            let mut buf = String::new();
            self.read.read_line(&mut buf).expect("read");
            if let Ok(resp) = serde_json::from_str::<Response>(&buf) {
                return resp;
            }
        }
    }

    fn board(&mut self) -> Board {
        match self.request(Command::Snapshot) {
            Response::Board { board, .. } => board,
            other => panic!("expected board, got {other:?}"),
        }
    }
}

fn titles(board: &Board, column: &str) -> Vec<String> {
    board.column_tickets(column).iter().map(|t| t.title.clone()).collect()
}

#[test]
fn a_card_reorders_inside_its_own_column() {
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-reorder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();

    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "reorder".into() }),
        Response::Hello { .. }
    ));

    for title in ["a", "b", "c"] {
        assert!(matches!(
            c.request(Command::CreateTicket { column: "TODO".into(), title: title.into() }),
            Response::Created { .. }
        ));
    }
    let board = c.board();
    assert_eq!(titles(&board, "TODO"), ["a", "b", "c"], "created in order");
    let id = |b: &Board, t: &str| b.tickets.iter().find(|x| x.title == t).unwrap().id;
    let (a, c_id) = (id(&board, "a"), id(&board, "c"));

    // The ghost dropped on the top row: before `a`, in the column it is in.
    assert!(matches!(
        c.request(Command::MoveTicket { id: c_id, column: "TODO".into(), before: Some(a) }),
        Response::Ok
    ));
    assert_eq!(titles(&c.board(), "TODO"), ["c", "a", "b"], "reorder to the top");

    // Dropped past the last row: `before: None` is the bottom, not a no-op.
    assert!(matches!(
        c.request(Command::MoveTicket { id: c_id, column: "TODO".into(), before: None }),
        Response::Ok
    ));
    assert_eq!(titles(&c.board(), "TODO"), ["a", "b", "c"], "reorder to the bottom");

    // It survives the disk round-trip: order is a ticket file field.
    let key = c.board().ticket(c_id).unwrap().short_key.clone();
    let toml =
        std::fs::read_to_string(repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml"))
            .unwrap();
    assert!(toml.contains("order ="), "ticket.toml carries the order: {toml}");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let _ = std::fs::remove_dir_all(&dir);
}
