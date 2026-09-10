//! Reorder inside a column: the MOVE ghost's drop is the one gesture that
//! means "same column, new slot", and it arrives as `MoveTicket` with the
//! column it is already in. In-process daemon over the real wire.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::Board;
use mesimon_core::command::{Command, Response};

fn titles(board: &Board, column: &str) -> Vec<String> {
    board.column_tickets(column).iter().map(|t| t.title.clone()).collect()
}

#[test]
fn a_card_reorders_inside_its_own_column() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("reorder");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "reorder".into()
        }),
        Response::Hello { .. }
    ));

    for title in ["a", "b", "c"] {
        assert!(matches!(
            c.request(Command::CreateTicket {
                column: "TODO".into(),
                title: title.into(),
                workspace: None
            }),
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
    let ticket_toml = repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");
    let toml = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(toml.contains("order ="), "ticket.toml carries the order: {toml}");

    // The card's age is time in COLUMN: `entered_at` is stamped at mint, a
    // reorder leaves it alone, and only a column change restarts it.
    let stamp = |b: &Board| b.ticket(c_id).unwrap().entered_at.clone();
    let minted = stamp(&board).expect("minted with an entered_at stamp");
    assert_eq!(stamp(&c.board()).as_deref(), Some(minted.as_str()), "reorders do not restamp");
    let secs = |s: &str| s.strip_prefix('@').unwrap().parse::<u64>().unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // the stamp is whole seconds
    assert!(matches!(
        c.request(Command::MoveTicket { id: c_id, column: "IN PROGRESS".into(), before: None }),
        Response::Ok
    ));
    let moved = stamp(&c.board()).unwrap();
    assert!(secs(&moved) > secs(&minted), "a column move restamps: {minted} -> {moved}");
    let toml = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(toml.contains("entered_at ="), "ticket.toml carries the stamp: {toml}");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
