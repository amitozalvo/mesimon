//! A column's description (T-467): the user's words on what the column is
//! for, scrubbed on the way in, kept inside the column's `[[columns]]` table
//! on disk, and returned to an agent by `list_board` and `get_ticket` — the
//! real `mesimon mcp` shim, as Claude Code drives it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use serde_json::json;

mod common;
use common::*;

#[test]
fn an_agent_reads_what_each_column_is_for() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot("coldesc", Some(STUB)) else { return };
    let mut c = h.client("coldesc");

    let mut s = c.board().column("TODO").unwrap().settings.clone();
    s.description = Some("  planned\nfor this version \u{1b}[1m\u{202e} ".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings: s }),
        Response::Ok
    ));
    // Scrubbed to one line and trimmed: the bytes on disk are the bytes an
    // agent reads.
    let want = "plannedfor this version [1m";
    let board = c.board();
    assert_eq!(board.column("TODO").unwrap().settings.description.as_deref(), Some(want));
    let toml = std::fs::read_to_string(h.repo.join(".mesimon/board/columns.toml")).unwrap();
    assert!(toml.contains(&format!("description = \"{want}\"")), "{toml}");

    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "described".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let mut shim = Shim::start(&h.paths.orch_sock(), sid);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");

    // Only the described column is named; the order is `columns`'.
    let described = json!({ "TODO": want });
    let board = shim.call_ok("list_board", json!({}));
    assert_eq!(board["column_descriptions"], described, "{board}");
    assert_eq!(board["columns"][0], "TODO");
    let ticket = shim.call_ok("get_ticket", json!({}));
    assert_eq!(ticket["column_descriptions"], described, "{ticket}");

    // Cleared: blank words are no description, and the key leaves the answer.
    let mut s = c.board().column("TODO").unwrap().settings.clone();
    s.description = Some(" \t ".into());
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings: s }),
        Response::Ok
    ));
    assert_eq!(c.board().column("TODO").unwrap().settings.description, None);
    let board = shim.call_ok("list_board", json!({}));
    assert!(board.get("column_descriptions").is_none(), "{board}");
}
