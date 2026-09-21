//! The crown (T-411): one ticket per board whose agent may edit the others
//! through the keyed forms of its tools. A person grants it (`CrownTicket`),
//! an agent never can; an uncrowned agent reaching for another ticket reads
//! how a person grants one; every keyed write is judged against the ticket
//! as it was READ (`seen`); the touched card rides the snapshot for the
//! board to light; and the crown leaves with its ticket.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::SessionKind;
use mesimon_core::command::{AgentTicketView, Command, CrownTouch, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

fn create(c: &mut TestClient, title: &str) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    }
}

fn spawn(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    }
}

fn key_of(c: &mut TestClient, id: ulid::Ulid) -> String {
    c.board().ticket(id).unwrap().short_key.clone()
}

fn read(c: &mut TestClient, session: uuid::Uuid, key: &str) -> Result<AgentTicketView, String> {
    match c.send(Principal::Agent { session }, Command::AgentReadTicket { key: key.into() }) {
        Response::AgentTicket { ticket } => Ok(ticket),
        Response::Err { message } => Err(message),
        other => panic!("read {key}: {other:?}"),
    }
}

fn touches(c: &mut TestClient) -> Vec<CrownTouch> {
    match c.request(Command::Snapshot) {
        Response::Board { crown_touches, .. } => crown_touches,
        other => panic!("snapshot: {other:?}"),
    }
}

#[test]
fn the_crown_lets_one_agent_edit_the_others() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";
    // No starter tags (the registry is built by hand), and the archive offer
    // prices a ticket the moment it is untouched rather than after an hour,
    // so the offer's road can be driven at the end.
    let Some(h) = Harness::boot_with_env(
        "crown",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_ARCHIVE_SUGGEST_MS", "0")],
    ) else {
        return;
    };
    let sock = h.paths.orch_sock();
    let mut c = h.client("crown");

    let a = create(&mut c, "triage the board");
    let b = create(&mut c, "fix the thing");
    let d = create(&mut c, "later");
    let (ka, kb, kd) = (key_of(&mut c, a), key_of(&mut c, b), key_of(&mut c, d));
    let sa = spawn(&mut c, a);
    let _sb = spawn(&mut c, b);

    // ---- uncrowned: another ticket is refused, and the refusal teaches ----
    let refusal = read(&mut c, sa, &kb).expect_err("no crown yet");
    for word in ["crown", "^o", "raise_hand", &ka] {
        assert!(refusal.contains(word), "the refusal names {word}: {refusal}");
    }
    assert!(refusal.contains("no ticket wears the crown"), "{refusal}");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentMoveTicket {
            to_column: "IN PROGRESS".into(),
            idempotency_key: None,
            key: Some(kb.clone()),
            before: None,
            seen: None,
        },
    ) {
        Response::Err { message } => assert!(message.contains("crown"), "{message}"),
        other => panic!("an uncrowned move of another ticket: {other:?}"),
    }
    // A session may always address its OWN ticket by key, crown or not.
    let own = read(&mut c, sa, &ka).expect("own key");
    assert!(!own.crowned);
    assert!(own.seen.is_some(), "every read carries a stamp");
    // An unknown key is an answer, not a crown question.
    let unknown = read(&mut c, sa, "T-999").expect_err("no such ticket");
    assert!(unknown.contains("no such ticket"), "{unknown}");

    // ---- the crown is the person's to give ---------------------------------
    match c.send(Principal::Agent { session: sa }, Command::CrownTicket { id: a }) {
        Response::Err { message } => assert!(message.contains("not available"), "{message}"),
        other => panic!("an agent crowning itself: {other:?}"),
    }
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(c.board().crown, Some(a));
    assert!(c.board().is_crowned(a));
    let own = read(&mut c, sa, &ka).unwrap();
    assert!(own.crowned, "get_ticket says the caller wears it");
    match c.send(Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => {
            assert_eq!(board.crown.as_deref(), Some(ka.as_str()));
            let row = board.tickets.iter().find(|t| t.key == kb).unwrap();
            assert_eq!(row.by.as_deref(), Some("person"));
            assert!(row.state.is_some(), "a ticket with an agent has a state word");
            let later = board.tickets.iter().find(|t| t.key == kd).unwrap();
            assert!(later.state.is_none(), "no agent, no word");
        }
        other => panic!("list_board: {other:?}"),
    }

    // ---- read another ticket: the card's words and a stamp ------------------
    let view = read(&mut c, sa, &kb).expect("crowned");
    assert!(!view.crowned);
    let seen = view.seen.clone().expect("a stamp");
    let state = view.state.expect("B has an agent");
    assert!(!state.state.is_empty());

    // ---- a keyed move needs the stamp, and the stamp must be fresh -----------
    let mv = |seen: Option<&str>, before: Option<&str>| Command::AgentMoveTicket {
        to_column: "IN PROGRESS".into(),
        idempotency_key: None,
        key: Some(kb.clone()),
        before: before.map(str::to_string),
        seen: seen.map(str::to_string),
    };
    match c.send(Principal::Agent { session: sa }, mv(None, None)) {
        Response::Err { message } => assert!(message.contains("seen is required"), "{message}"),
        other => panic!("no stamp: {other:?}"),
    }
    match c.send(Principal::Agent { session: sa }, mv(Some("stale"), None)) {
        Response::Err { message } => {
            assert!(message.contains("changed since it was read"), "{message}");
            assert!(message.contains(&kb), "{message}");
        }
        other => panic!("a stale stamp: {other:?}"),
    }
    let fresh = match c.send(Principal::Agent { session: sa }, mv(Some(&seen), None)) {
        Response::AgentMoved { column, seen, replayed, .. } => {
            assert_eq!(column, "IN PROGRESS");
            assert!(!replayed);
            seen.expect("a keyed move hands back the fresh stamp")
        }
        other => panic!("the move: {other:?}"),
    };
    assert_ne!(fresh, seen, "the ticket changed, so the stamp did");
    assert_eq!(c.board().ticket(b).unwrap().column, "IN PROGRESS");
    let t = touches(&mut c);
    assert_eq!(t.len(), 1, "the board is told what the crown touched: {t:?}");
    assert_eq!(t[0].ticket, b);
    assert_eq!(t[0].action, "moved");

    // ---- position is priority: `before` lands above a named ticket ----------
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentMoveTicket {
            to_column: "IN PROGRESS".into(),
            idempotency_key: None,
            key: Some(kd.clone()),
            before: Some(kb.clone()),
            seen: dv.seen,
        },
    ) {
        Response::AgentMoved { column, .. } => assert_eq!(column, "IN PROGRESS"),
        other => panic!("the priority move: {other:?}"),
    }
    let order: Vec<String> =
        c.board().column_tickets("IN PROGRESS").iter().map(|t| t.short_key.clone()).collect();
    assert_eq!(order, [kd.clone(), kb.clone()], "D above B");

    // ---- rename, with the receipt's stamp; the old stamp is now stale --------
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket {
            key: kb.clone(),
            title: "  fix it properly ".into(),
            seen: Some(fresh.clone()),
        },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.title, "fix it properly"),
        other => panic!("rename: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket {
            key: kb.clone(),
            title: "again".into(),
            seen: Some(fresh.clone()),
        },
    ) {
        Response::Err { message } => assert!(message.contains("changed since"), "{message}"),
        other => panic!("a reused stamp: {other:?}"),
    }
    assert_eq!(c.board().ticket(b).unwrap().title, "fix it properly");
    assert_eq!(touches(&mut c).iter().find(|t| t.ticket == b).unwrap().action, "renamed");

    // ---- a note and a tag on another ticket ---------------------------------
    let note = match c.send(
        Principal::Agent { session: sa },
        Command::AgentWriteNote {
            note: None,
            text: "from the crown".into(),
            key: Some(kb.clone()),
        },
    ) {
        Response::NoteWritten { note } => note.expect("created"),
        other => panic!("keyed note: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentReadNote { note, key: Some(kb.clone()) },
    ) {
        Response::Note { text, meta } => {
            assert_eq!(text.trim(), "from the crown");
            assert_eq!(meta.created_by, format!("agent:{sa}"));
        }
        other => panic!("keyed read_note: {other:?}"),
    }
    assert!(matches!(
        c.request(Command::RegisterTag { group: 1, name: "BUG".into() }),
        Response::Ok
    ));
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentTagTicket {
            name: "bug".into(),
            group: None,
            remove: false,
            key: Some(kb.clone()),
        },
    ) {
        Response::AgentTagged { tags, seen, .. } => {
            assert_eq!(tags[0].name, "BUG");
            assert!(seen.is_some(), "a keyed tag hands back the stamp");
        }
        other => panic!("keyed tag: {other:?}"),
    }
    assert_eq!(c.board().ticket(b).unwrap().tags[0].name, "BUG");

    // ---- workspace: a pending ticket only ------------------------------------
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace { key: kd.clone(), workspace: "worktree".into(), seen: dv.seen },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.workspace, "worktree"),
        other => panic!("set_workspace: {other:?}"),
    }
    let bv = read(&mut c, sa, &kb).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace { key: kb.clone(), workspace: "worktree".into(), seen: bv.seen },
    ) {
        Response::Err { message } => assert!(message.contains("locked"), "{message}"),
        other => panic!("a ticket with a pane keeps its workspace: {other:?}"),
    }
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace {
            key: kd.clone(),
            workspace: "elsewhere".into(),
            seen: dv.seen,
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("worktree or shared_checkout"), "{message}")
        }
        other => panic!("an unknown workspace word: {other:?}"),
    }

    // ---- archive is the reversible spelling of delete -------------------------
    let bv = read(&mut c, sa, &kb).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kb.clone(), restore: false, seen: bv.seen },
    ) {
        Response::Err { message } => assert!(message.contains("awake"), "{message}"),
        other => panic!("an awake ticket cannot be archived: {other:?}"),
    }
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kd.clone(), restore: false, seen: dv.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("archive: {other:?}"),
    }
    assert!(c.board().ticket(d).unwrap().is_archived());
    // An archived ticket still reads, and refuses every edit but the restore.
    let dv = read(&mut c, sa, &kd).expect("an archived ticket still reads");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket { key: kd.clone(), title: "x".into(), seen: dv.seen.clone() },
    ) {
        Response::Err { message } => assert!(message.contains("archived"), "{message}"),
        other => panic!("renaming an archived ticket: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kd.clone(), restore: true, seen: dv.seen },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.column, "IN PROGRESS"),
        other => panic!("restore: {other:?}"),
    }
    assert!(!c.board().ticket(d).unwrap().is_archived());

    // ---- the shim: twelve tools, and `get_ticket` with a key ------------------
    let mut shim = Shim::start(&sock, sa);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    let listed = shim.rpc("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 12);
    let other = shim.call_ok("get_ticket", json!({ "key": kb }));
    assert_eq!(other["key"], json!(kb));
    assert!(other["seen"].is_string(), "{other}");
    assert_eq!(other["crowned"], json!(false));
    let mine = shim.call_ok("get_ticket", json!({}));
    assert_eq!(mine["crowned"], json!(true));
    let r = shim.call("rename_ticket", json!({ "key": kb, "title": "x" }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");

    // ---- uncrown, and the crown leaves with its ticket ------------------------
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(c.board().crown.is_none());
    assert!(read(&mut c, sa, &kb).is_err(), "uncrowned again");
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // Persisted with the board's scalars.
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains(&format!("crown = \"{a}\"")), "{file}");
    // A second crown displaces the first.
    assert!(matches!(c.request(Command::CrownTicket { id: d }), Response::Ok));
    assert_eq!(c.board().crown, Some(d));
    assert!(read(&mut c, sa, &kb).is_err(), "A no longer wears it");
    // Deleting the holder takes the crown with it.
    assert!(matches!(
        c.request(Command::DeleteTicket { id: d, discard_worktree: false }),
        Response::Ok
    ));
    assert!(c.board().crown.is_none(), "the crown left with its ticket");
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(!file.contains("crown ="), "{file}");

    // ---- the header's archive offer drops it too, like `a a` ------------------
    // A sessionless ticket in the template's DONE column (reclaim on) is what
    // the offer prices; taking the offer archives it and the crown goes with
    // it — the one road that had skipped the drop.
    let e = create(&mut c, "shipped");
    assert!(matches!(c.request(Command::CrownTicket { id: e }), Response::Ok));
    assert!(matches!(
        c.request(Command::MoveTicket { id: e, column: "DONE".into(), before: None }),
        Response::Ok
    ));
    assert!(c.board().is_crowned(e));
    match c.request(Command::ArchiveAll) {
        Response::Archived { archived, .. } => assert!(archived >= 1, "the offer took E"),
        other => panic!("archive all: {other:?}"),
    }
    assert!(c.board().ticket(e).unwrap().is_archived());
    assert!(c.board().crown.is_none(), "the offer's archive drops the crown");
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(!file.contains("crown ="), "{file}");
}
