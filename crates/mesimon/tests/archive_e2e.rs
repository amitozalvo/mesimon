//! Archive (T-46): the gate refuses awake sessions, the header suggestion
//! prices an all-asleep DONE ticket past the (shrunk) hour, archive survives
//! the disk round-trip, the refusal guards hold, restore lands in the same
//! column. Real tmux, in-process daemon, stub agent.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{ColumnOffers, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn archive_gates_suggests_and_restores() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("archive");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    fixture.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    fixture.set_env("MESIMON_ARCHIVE_SUGGEST_MS", "1");

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "archive".into() }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket {
        column: "DONE".into(),
        title: "cold".into(),
        workspace: None,
    });
    // A session-less DONE ticket: suggested once created_at ages past the
    // threshold, archivable any time.
    let _ = c.request(Command::CreateTicket {
        column: "DONE".into(),
        title: "empty".into(),
        workspace: None,
    });
    let (board, _) = snapshot_of(c.request(Command::Snapshot));
    let cold = board.tickets.iter().find(|t| t.title == "cold").unwrap().id;
    let key = board.ticket(cold).unwrap().short_key.clone();
    let ticket_toml = repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");

    // One claude session, driven idle by real hooks.
    let sid = match c.request(Command::SpawnSession {
        ticket: cold,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let transcript = dir.join("cold.jsonl");
    std::fs::write(
        &transcript,
        "{\"uuid\":\"u0\",\"type\":\"user\",\"message\":{}}\n\
         {\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
    )
    .unwrap();
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        &format!(r#"{{"session_id":"x","transcript_path":"{}"}}"#, transcript.display()),
    );
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    hook_send(
        &hook_sock,
        &sid.to_string(),
        "Stop",
        r#"{"stop_hook_active":false,"background_tasks":[]}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let (board, _) = snapshot_of(c.request(Command::Snapshot));
        if board.sessions.iter().any(|s| matches!(s.state, SessionState::Idle { .. })) {
            break;
        }
        assert!(Instant::now() < deadline, "session never settled idle");
        std::thread::sleep(Duration::from_millis(200));
    }

    // 1. Awake (idle still holds a pane): archive refuses.
    err_containing(c.request(Command::ArchiveTicket { id: cold }), "awake");

    // Independent column offers govern both the prices and the bulk actions.
    let set_offer = |c: &mut TestClient, offers| {
        let (board, _) = snapshot_of(c.request(Command::Snapshot));
        let mut settings = board.column("DONE").unwrap().settings.clone();
        settings.offers = Some(offers);
        assert!(matches!(
            c.request(Command::SetColumnSettings { name: "DONE".into(), settings }),
            Response::Ok
        ));
    };
    let wait_offer = |c: &mut TestClient, sleep, archive| {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let (_, resources) = snapshot_of(c.request(Command::Snapshot));
            if (resources.reclaim_sessions, resources.archive_tickets) == (sleep, archive) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "offers did not become sleep={sleep}, archive={archive}: {resources:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    set_offer(&mut c, ColumnOffers::Archive);
    wait_offer(&mut c, 0, 1);
    assert!(matches!(c.request(Command::ReclaimAll), Response::Reclaimed { slept: 0, .. }));
    set_offer(&mut c, ColumnOffers::Sleep);
    wait_offer(&mut c, 1, 0);
    assert!(matches!(c.request(Command::ArchiveAll), Response::Archived { archived: 0, .. }));
    set_offer(&mut c, ColumnOffers::Both);

    // 2. Sleep it; the suggestion prices both tickets within the 1 s bucket
    // ("cold" all-asleep + the session-less "empty" past its created_at age).
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let (_, resources) = snapshot_of(c.request(Command::Snapshot));
        if resources.archive_tickets == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "archive suggestion never priced");
        std::thread::sleep(Duration::from_millis(200));
    }

    // 3. Archive: field on disk, off the board's columns, offer re-priced.
    assert!(matches!(c.request(Command::ArchiveTicket { id: cold }), Response::Ok));
    let (board, resources) = snapshot_of(c.request(Command::Snapshot));
    assert!(board.ticket(cold).unwrap().is_archived());
    let done: Vec<_> = board.column_tickets("DONE").iter().map(|t| t.title.clone()).collect();
    assert_eq!(done, vec!["empty"], "only the session-less ticket stays on the board");
    assert_eq!(resources.archive_tickets, 1, "cold re-priced away; empty still offered");
    let toml = std::fs::read_to_string(&ticket_toml).expect("ticket.toml");
    assert!(toml.contains("[archived]"), "field must persist: {toml}");

    // 4. The refusal guards.
    err_containing(
        c.request(Command::SpawnSession {
            ticket: cold,
            kind: SessionKind::Bash,
            submit_prompt: false,
        }),
        "archived",
    );
    err_containing(c.request(Command::WakeSession { id: sid }), "archived");
    err_containing(
        c.request(Command::MoveTicket { id: cold, column: "TODO".into(), before: None }),
        "archived",
    );
    err_containing(c.request(Command::ArchiveTicket { id: cold }), "already archived");

    // 5. Restore: same column, field gone from disk.
    assert!(matches!(c.request(Command::UnarchiveTicket { id: cold }), Response::Ok));
    let (board, _) = snapshot_of(c.request(Command::Snapshot));
    let t = board.ticket(cold).unwrap();
    assert!(!t.is_archived());
    assert_eq!(t.column, "DONE", "restore must land in the archived-from column");
    assert_eq!(board.column_tickets("DONE").len(), 2, "cold back beside empty");
    let toml = std::fs::read_to_string(&ticket_toml).expect("ticket.toml");
    assert!(!toml.contains("[archived]"), "field must clear: {toml}");

    // 6. X takes the whole offer: both tickets re-price, ArchiveAll takes
    // exactly that set, the offer reads zero after.
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let (_, resources) = snapshot_of(c.request(Command::Snapshot));
        if resources.archive_tickets == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "offer never re-priced after restore");
        std::thread::sleep(Duration::from_millis(200));
    }
    set_offer(&mut c, ColumnOffers::Archive);
    match c.request(Command::ArchiveAll) {
        Response::Archived { archived, skipped } => assert_eq!((archived, skipped), (2, 0)),
        other => panic!("archive_all failed: {other:?}"),
    }
    let (board, resources) = snapshot_of(c.request(Command::Snapshot));
    assert!(board.column_tickets("DONE").is_empty());
    assert_eq!(board.archived_tickets().len(), 2);
    assert_eq!(resources.archive_tickets, 0);

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
