//! A column's `agent_tools` (T-117): the tier the shim LISTS at spawn
//! (`--tools` on the blob's argv) and the tier the daemon ADMITS at every
//! call, against the ticket's column as it stands then — so narrowing a
//! column narrows a live session, widening it widens one without a wake,
//! `get_ticket` advertises no move below `full`, and the board's own switch
//! still turns everything off.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::{AgentTools, SessionKind};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

fn tier_of(c: &mut TestClient, column: &str, tier: AgentTools) {
    let mut s = c.board().column(column).unwrap().settings.clone();
    s.agent_tools = tier;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: column.into(), settings: s }),
        Response::Ok
    ));
}

fn listed(shim: &mut Shim) -> Vec<String> {
    let tools = shim.rpc("tools/list", json!({}));
    tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_columns_tier_is_listed_at_spawn_and_enforced_at_every_call() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot("agtools", Some(STUB)) else { return };
    let sock = h.paths.orch_sock();
    let mut c = h.client("agtools");

    let create =
        |c: &mut TestClient, column: &str, title: &str| match c.request(Command::CreateTicket {
            column: column.into(),
            title: title.into(),
            workspace: None,
            tier: None,
        }) {
            Response::Created { id, .. } => id,
            other => panic!("create: {other:?}"),
        };
    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let blob_tools = |c: &mut TestClient, sid| -> String {
        let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
        let at = rec.argv.iter().position(|a| a == "--mcp-config").expect("the blob");
        let blob: serde_json::Value = serde_json::from_str(&rec.argv[at + 1]).unwrap();
        let args = blob["mcpServers"]["mesimon"]["args"].as_array().unwrap();
        let i = args.iter().position(|a| a == "--tools").expect("--tools on the shim's argv");
        args[i + 1].as_str().unwrap().to_string()
    };

    // ---- full: today's behaviour, the whole surface -------------------------
    let t_full = create(&mut c, "TODO", "full");
    let s_full = spawn(&mut c, t_full);
    assert_eq!(blob_tools(&mut c, s_full), "full");
    // T-362: the read tools are pre-approved on argv, the writers are not.
    let rec = c.board().sessions.into_iter().find(|s| s.id == s_full).unwrap();
    let at = rec.argv.iter().position(|a| a == "--allowedTools").expect("--allowedTools");
    assert_eq!(
        rec.argv[at + 1],
        "mcp__mesimon__get_ticket,mcp__mesimon__list_board,mcp__mesimon__read_note,mcp__mesimon__read_attachment"
    );
    let mut shim = Shim::start(&sock, s_full);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    assert_eq!(listed(&mut shim).len(), mesimon_core::mcp::tools().len());

    // ---- read: four listed, the rest refused by the daemon -----------------
    tier_of(&mut c, "TODO", AgentTools::Read);
    let t_read = create(&mut c, "TODO", "read only");
    let s_read = spawn(&mut c, t_read);
    assert_eq!(blob_tools(&mut c, s_read), "read");
    // The shim as the blob starts it: with the column's tier on its argv.
    let mut shim_r = Shim::start_with(&sock, s_read, &["--tools", "read"]);
    shim_r.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim_r.notify("notifications/initialized");
    assert_eq!(listed(&mut shim_r), ["get_ticket", "list_board", "read_note", "read_attachment"]);
    let t = shim_r.call_ok("get_ticket", json!({}));
    assert_eq!(t["allowed_columns"], json!([]), "no move is offered below full");
    // Straight at the daemon, past the shim: the refusal names the tier.
    match c.send(
        Principal::Agent { session: s_read },
        Command::AgentMoveTicket {
            to_column: "REVIEW".into(),
            idempotency_key: None,
            key: None,
            before: None,
            seen: None,
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("read") && message.contains("TODO"), "{message}");
        }
        other => panic!("a read column admits no move: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: s_read },
        Command::AgentWriteNote { note: None, text: "x".into(), key: None },
    ) {
        Response::Err { message } => assert!(message.contains("read"), "{message}"),
        other => panic!("a read column admits no note: {other:?}"),
    }
    // A raised hand writes on the caller's own ticket, so it sits on the
    // same rung as a note (T-107) and a read column refuses it too.
    match c.send(
        Principal::Agent { session: s_read },
        Command::AgentRaiseHand { reason: "which one?".into() },
    ) {
        Response::Err { message } => assert!(message.contains("read"), "{message}"),
        other => panic!("a read column admits no raised hand: {other:?}"),
    }
    // The live `full` session in the same column is narrowed too — the tier
    // is the column's NOW, not the spawn's.
    match c.send(
        Principal::Agent { session: s_full },
        Command::AgentWriteNote { note: None, text: "x".into(), key: None },
    ) {
        Response::Err { message } => assert!(message.contains("read"), "{message}"),
        other => panic!("narrowed at call time: {other:?}"),
    }

    // ---- widen to annotate without a wake: notes yes, moves no ---------------
    tier_of(&mut c, "TODO", AgentTools::Annotate);
    assert!(matches!(
        c.send(
            Principal::Agent { session: s_read },
            Command::AgentWriteNote { note: None, text: "now allowed".into(), key: None },
        ),
        Response::NoteWritten { .. }
    ));
    assert!(matches!(
        c.send(
            Principal::Agent { session: s_read },
            Command::AgentRaiseHand { reason: "now allowed".into() },
        ),
        Response::AgentRaised { .. }
    ));
    match c.send(
        Principal::Agent { session: s_read },
        Command::AgentMoveTicket {
            to_column: "REVIEW".into(),
            idempotency_key: None,
            key: None,
            before: None,
            seen: None,
        },
    ) {
        Response::Err { message } => assert!(message.contains("annotate"), "{message}"),
        other => panic!("annotate admits no move: {other:?}"),
    }
    assert_eq!(listed(&mut shim_r).len(), 4, "the shim lists what it was born with");

    // ---- a hand move to a full column widens the session --------------------
    assert!(matches!(
        c.request(Command::MoveTicket { id: t_read, column: "REVIEW".into(), before: None }),
        Response::Ok
    ));
    // (Not back to TODO: the move gate refuses an agent undoing the hand's
    // move inside a minute, and that is the gate's business, not the tier's.)
    match c.send(
        Principal::Agent { session: s_read },
        Command::AgentMoveTicket {
            to_column: "IN PROGRESS".into(),
            idempotency_key: None,
            key: None,
            before: None,
            seen: None,
        },
    ) {
        Response::AgentMoved { .. } => {}
        other => panic!("REVIEW is full, so the move is admitted: {other:?}"),
    }

    // ---- off: no blob at all, like the board switch -----------------------------
    tier_of(&mut c, "DONE", AgentTools::Off);
    let t_off = create(&mut c, "DONE", "no tools");
    let s_off = spawn(&mut c, t_off);
    let rec = c.board().sessions.into_iter().find(|s| s.id == s_off).unwrap();
    assert!(!rec.argv.iter().any(|a| a == "--mcp-config"), "{:?}", rec.argv);
    assert!(!rec.argv.iter().any(|a| a == "--allowedTools"), "{:?}", rec.argv);
    match c.send(Principal::Agent { session: s_off }, Command::AgentGetTicket) {
        Response::Err { message } => assert!(message.contains("off"), "{message}"),
        other => panic!("off admits nothing: {other:?}"),
    }

    // ---- the board switch still ANDs ---------------------------------------------
    assert!(matches!(c.request(Command::SetMcpTools { on: false }), Response::Ok));
    match c.send(Principal::Agent { session: s_full }, Command::AgentGetTicket) {
        Response::Err { message } => assert!(message.contains("off"), "{message}"),
        other => panic!("the board switch off refuses everything: {other:?}"),
    }
}
