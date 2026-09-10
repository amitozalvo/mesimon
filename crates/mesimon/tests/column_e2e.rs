//! The column lifecycle (T-117) end to end against a real daemon: add,
//! reorder, rename — carrying every ticket, archived ones included, and
//! every rule naming the column — delete refused while tickets are in it,
//! a self-referencing rule refused, a one-shot sort that is not a move, the
//! settings inside each `[[columns]]` table on disk at schema 4, and all of
//! it surviving a daemon restart.
//!
//! No tmux and no agent — columns are pure board state, so this runs
//! everywhere.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::board::{Board, ColumnSettings, SortBy, TrainReach};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

fn names(b: &Board) -> Vec<String> {
    b.sorted_columns().iter().map(|c| c.name.clone()).collect()
}

fn settings(b: &Board, name: &str) -> ColumnSettings {
    b.column(name).unwrap_or_else(|| panic!("column {name}")).settings.clone()
}

fn create(c: &mut TestClient, column: &str, title: &str) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: column.into(),
        title: title.into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    }
}

#[test]
fn columns_are_added_renamed_sorted_deleted_and_survive_a_restart() {
    let fixture = common::TestFixture::new("column");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let cols_path = repo.join(".mesimon/board/columns.toml");

    let daemon = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "column".into()
        }),
        Response::Hello { .. }
    ));

    // ---- a fresh board carries the template's rules, on the columns -------
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["TODO", "IN PROGRESS", "REVIEW", "DONE"]);
    assert_eq!(settings(&board, "TODO").on_working.as_deref(), Some("IN PROGRESS"));
    assert_eq!(settings(&board, "IN PROGRESS").on_done.as_deref(), Some("REVIEW"));
    assert_eq!(settings(&board, "REVIEW").train, TrainReach::Merge);
    assert!(settings(&board, "DONE").requires_merge && settings(&board, "DONE").reclaim);

    // ---- add + reorder ------------------------------------------------------
    assert!(matches!(
        c.request(Command::AddColumn { name: " QA ".into(), after: Some("REVIEW".into()) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::AddColumn { name: "LATER".into(), after: None }),
        Response::Ok
    ));
    err_containing(c.request(Command::AddColumn { name: "qa".into(), after: None }), "already");
    err_containing(c.request(Command::AddColumn { name: "  ".into(), after: None }), "name");
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["TODO", "IN PROGRESS", "REVIEW", "QA", "DONE", "LATER"]);
    assert_eq!(settings(&board, "QA"), ColumnSettings::default(), "a new column carries no rule");
    assert!(matches!(
        c.request(Command::ReorderColumn { name: "LATER".into(), before: Some("TODO".into()) }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["LATER", "TODO", "IN PROGRESS", "REVIEW", "QA", "DONE"]);

    // ---- settings: the whole struct, validated together ----------------------
    let mut s = settings(&board, "QA");
    s.on_working = Some("QA".into());
    err_containing(
        c.request(Command::SetColumnSettings { name: "QA".into(), settings: s }),
        "itself",
    );
    let mut s = settings(&board, "QA");
    s.on_done = Some("NOPE".into());
    err_containing(
        c.request(Command::SetColumnSettings { name: "QA".into(), settings: s }),
        "no such column",
    );
    let mut s = settings(&board, "QA");
    s.on_done = Some("DONE".into());
    s.auto_run = true;
    s.offers = Some(mesimon_core::board::ColumnOffers::Archive);
    s.claude_mode = mesimon_core::board::ClaudeMode::Plan;
    s.agent_tools = mesimon_core::board::AgentTools::Read;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "QA".into(), settings: s.clone() }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(settings(&board, "QA"), s);

    // ---- rename carries the tickets, the archive and the rules ---------------
    let t1 = create(&mut c, "TODO", "one");
    let t2 = create(&mut c, "TODO", "two");
    let t3 = create(&mut c, "REVIEW", "three");
    let parked = create(&mut c, "TODO", "parked");
    assert!(matches!(c.request(Command::ArchiveTicket { id: parked }), Response::Ok));
    // REVIEW's rule points at IN PROGRESS; rename that and watch it follow.
    assert!(matches!(
        c.request(Command::RenameColumn { name: "IN PROGRESS".into(), to: "DOING".into() }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::RenameColumn { name: "TODO".into(), to: "INBOX".into() }),
        Response::Ok
    ));
    err_containing(
        c.request(Command::RenameColumn { name: "INBOX".into(), to: "review".into() }),
        "already",
    );
    err_containing(
        c.request(Command::RenameColumn { name: "GONE".into(), to: "X".into() }),
        "no such column",
    );
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["LATER", "INBOX", "DOING", "REVIEW", "QA", "DONE"]);
    for id in [t1, t2, parked] {
        assert_eq!(board.ticket(id).unwrap().column, "INBOX", "{id} followed the rename");
    }
    assert!(board.ticket(parked).unwrap().is_archived(), "still archived");
    assert_eq!(board.ticket(t3).unwrap().column, "REVIEW");
    assert_eq!(settings(&board, "INBOX").on_working.as_deref(), Some("DOING"));
    assert_eq!(settings(&board, "REVIEW").on_working.as_deref(), Some("DOING"));
    assert_eq!(settings(&board, "DOING").on_done.as_deref(), Some("REVIEW"));
    // On disk too: the ticket files were rewritten.
    for id in [t1, parked] {
        let key = &board.ticket(id).unwrap().short_key;
        let text = std::fs::read_to_string(
            repo.join(".mesimon/board/tickets").join(key).join("ticket.toml"),
        )
        .unwrap();
        assert!(text.contains("column = \"INBOX\""), "{text}");
    }

    // ---- delete: refused while tickets are in it, then clears the refs ------
    err_containing(c.request(Command::DeleteColumn { name: "INBOX".into() }), "move its 2 tickets");
    for id in [t1, t2] {
        assert!(matches!(
            c.request(Command::MoveTicket { id, column: "LATER".into(), before: None }),
            Response::Ok
        ));
    }
    // The archived one stays where it is: it keeps its string and the
    // restore falls back to the first column.
    assert!(matches!(c.request(Command::DeleteColumn { name: "INBOX".into() }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["LATER", "DOING", "REVIEW", "QA", "DONE"]);
    assert_eq!(board.ticket(parked).unwrap().column, "INBOX");
    assert!(matches!(c.request(Command::UnarchiveTicket { id: parked }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.ticket(parked).unwrap().column, "LATER", "restored into the first column");
    // DOING pointed at REVIEW and REVIEW at DOING; delete REVIEW after
    // emptying it and DOING's rule is cleared rather than left dangling.
    assert!(matches!(
        c.request(Command::MoveTicket { id: t3, column: "QA".into(), before: None }),
        Response::Ok
    ));
    assert!(matches!(c.request(Command::DeleteColumn { name: "REVIEW".into() }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(settings(&board, "DOING").on_done, None);

    // ---- a one-shot sort is not a move ----------------------------------------
    // Put T-1 at the bottom of LATER by hand (a same-column move is a
    // reorder), so a sort by key has something to do.
    assert!(matches!(
        c.request(Command::MoveTicket { id: t1, column: "LATER".into(), before: None }),
        Response::Ok
    ));
    let before = board_of(c.request(Command::Snapshot));
    let later: Vec<ulid::Ulid> = before.column_tickets("LATER").iter().map(|t| t.id).collect();
    assert_eq!(later.last(), Some(&t1));
    assert_ne!(later, vec![t1, t2, parked]);
    let entered: Vec<Option<String>> =
        later.iter().map(|id| before.ticket(*id).unwrap().entered_at.clone()).collect();
    assert!(matches!(
        c.request(Command::SortColumn { column: "LATER".into(), by: SortBy::Key }),
        Response::Ok
    ));
    let after = board_of(c.request(Command::Snapshot));
    let sorted: Vec<ulid::Ulid> = after.column_tickets("LATER").iter().map(|t| t.id).collect();
    assert_eq!(sorted, vec![t1, t2, parked], "T-1, T-2, T-4");
    for (id, was) in later.iter().zip(entered) {
        assert_eq!(after.ticket(*id).unwrap().entered_at, was, "a sort does not restamp the age");
    }
    err_containing(
        c.request(Command::SortColumn { column: "GONE".into(), by: SortBy::Key }),
        "no such column",
    );

    // ---- the file ----------------------------------------------------------------
    let cols = std::fs::read_to_string(&cols_path).unwrap();
    assert!(
        cols.contains(&format!("schema_version = {}", mesimon_daemon::store::COLUMNS_SCHEMA)),
        "{cols}"
    );
    let qa = cols.find("name = \"QA\"").unwrap();
    let rule = cols.find("on_done = \"DONE\"").unwrap();
    let mode = cols.find("claude_mode = \"plan\"").unwrap();
    let next_table = cols[qa..].find("[[columns]]").map(|i| i + qa).unwrap_or(cols.len());
    assert!(qa < rule && rule < next_table, "the rule sits inside QA's table:\n{cols}");
    assert!(qa < mode && mode < next_table, "{cols}");
    let first_table = cols.find("[[columns]]").unwrap();
    for scalar in ["schema_version =", "next_key =", "mcp_tools ="] {
        assert!(cols.find(scalar).unwrap() < first_table, "{scalar} must precede [[columns]]");
    }
    assert!(!cols.contains("collapsed"), "a default is never written:\n{cols}");

    // ---- survives a daemon restart ---------------------------------------------
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let daemon = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never reappeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    let _ = c.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "column".into(),
    });
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["LATER", "DOING", "QA", "DONE"]);
    assert_eq!(settings(&board, "QA"), s);
    assert_eq!(settings(&board, "DOING").on_done, None);
    assert!(settings(&board, "DONE").requires_merge);
    let sorted: Vec<ulid::Ulid> = board.column_tickets("LATER").iter().map(|t| t.id).collect();
    assert_eq!(sorted, vec![t1, t2, parked]);

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
