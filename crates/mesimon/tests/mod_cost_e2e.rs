//! A ticket's cost and the machine's quota on both Claude roads (T-581).
//!
//! On the mod road the mod reports each turn's end as `ModUsage`: the
//! engine's own count of the turn, and the account's rate-limit windows. The
//! first main turn fences the conversation, and from then on the ticket is
//! counted from the reports while its transcript is read beside them as a
//! check. On the hook set's road no such frame is taken and the transcript
//! is the count, as ever. The two tests send the same frames and the same
//! transcript; only the road differs, and the turn the mod reports counts 300
//! output tokens where its transcript says 100, so the snapshot says which
//! source counted it.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::road::Road;
use mesimon_core::usage::{Usage, WindowKind};

const STUB: &str = "#!/bin/sh\nexec sleep 300\n";

/// One assistant message: `out` output tokens, stamped `at`.
fn line(id: &str, out: u64, at: &str) -> String {
    format!(
        r#"{{"type":"assistant","requestId":"r-{id}","timestamp":"{at}","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":{out}}}}}}}"#
    )
}

fn usage_frame(turn: &str, out: u64) -> String {
    serde_json::json!({
        "turnId": turn,
        "reason": "answer",
        "durationMs": 1200,
        "usage": {
            "input_tokens": 1, "output_tokens": out,
            "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0,
            "model": "claude-opus-5-5"
        },
        "rateLimits": [
            {"kind": "five_hour", "percentUsed": 9, "resetsAt": "2099-01-01T00:00:00Z"},
            {"kind": "seven_day", "percentUsed": 74, "resetsAt": "2099-01-03T00:00:00Z"}
        ]
    })
    .to_string()
}

fn snapshot(c: &mut TestClient) -> (u64, Usage) {
    match c.request(Command::Snapshot) {
        Response::Board { costs, usage, .. } => (costs.iter().map(|t| t.tokens).sum(), usage),
        other => panic!("not a board: {other:?}"),
    }
}

fn append(path: &Path, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(f, "{text}").unwrap();
}

/// The same turns on one road: what the ticket's tokens come to, and the
/// quota the snapshot carries.
fn two_turns(name: &str, road: &str) -> Option<(Harness, u64, Usage, Road)> {
    let h = Harness::boot_with_env(name, Some(STUB), &[("MESIMON_CLAUDE_ROAD", road)])?;
    let mut c = h.client(name);
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "cost".into(),
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
    let sid_s = sid.to_string();
    let transcript = h.dir.join("transcript.jsonl");
    let hook = h.paths.hook_sock();
    hook_send_with(
        &hook,
        &sid_s,
        "SessionStart",
        Some("startup"),
        &serde_json::json!({
            "session_id": "c0ffee00-0000-0000-0000-000000000001",
            "transcript_path": transcript,
            "cwd": h.dir,
        })
        .to_string(),
    );
    let road_of = c.board().sessions.iter().find(|s| s.id == sid).unwrap().road;

    // Turn one, before any report: the transcript's.
    append(&transcript, &line("m1", 100, "2026-01-01T00:00:00.000Z"));
    hook_send(&hook, &sid_s, "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(10), "turn one counted", || snapshot(&mut c).0 == 101);
    // Its end as the mod reports it: on the mod road, the fence.
    hook_send_road(
        &hook,
        &sid_s,
        "ModUsage",
        Some("answer"),
        None,
        &usage_frame("t1", 100),
        Some("mod"),
    );

    // Turn two: the transcript says 100 output tokens, the mod 300.
    append(&transcript, &line("m2", 100, "2099-01-01T00:00:00.000Z"));
    hook_send_road(
        &hook,
        &sid_s,
        "ModUsage",
        Some("answer"),
        None,
        &usage_frame("t2", 300),
        Some("mod"),
    );
    hook_send(&hook, &sid_s, "Stop", r#"{"stop_hook_active":false}"#);
    // The pass that read turn two has landed when the ledger's cursor is
    // past it.
    let len = std::fs::metadata(&transcript).unwrap().len();
    let file = h.paths.costs_file();
    wait_until(Duration::from_secs(10), "turn two read", || {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains(&format!("\"offset\":{len}")))
    });
    let (tokens, usage) = snapshot(&mut c);
    Some((h, tokens, usage, road_of))
}

#[test]
fn on_the_mod_road_a_ticket_is_counted_from_the_mods_reports_and_the_quota_line_fills() {
    let Some((h, tokens, usage, road)) = two_turns("modcost", "mod") else { return };
    assert_eq!(road, Road::Mod);
    assert_eq!(tokens, 101 + 301, "turn one the transcript's, turn two the mod's");
    // The transcript's count of turn two is kept beside it, as the check.
    let ledger = std::fs::read_to_string(h.paths.costs_file()).unwrap();
    assert!(ledger.contains(r#""check":{"mod":301,"tail":101}"#), "{ledger}");
    let reading = usage.claude.reading.expect("the mod's windows on the snapshot");
    let five = reading.windows.iter().find(|w| w.kind == WindowKind::Session).unwrap();
    assert_eq!((five.percent, five.label.as_str()), (9.0, "5h"));
    let week = reading.windows.iter().find(|w| w.kind == WindowKind::Weekly).unwrap();
    assert_eq!(week.percent, 74.0);
    // And the machine's shared file holds it for every other board.
    let shared = std::fs::read_to_string(h.dir.join("home/.local/state/mesimon/usage.json"))
        .expect("the shared quota file");
    assert!(shared.contains("\"percent\":74.0"), "{shared}");
}

#[test]
fn on_the_hook_sets_road_the_transcript_is_the_count_and_no_report_is_taken() {
    let Some((h, tokens, usage, road)) = two_turns("hookcost", "hooks") else { return };
    assert_eq!(road, Road::Hooks);
    assert_eq!(tokens, 101 + 101, "both turns the transcript's");
    assert!(usage.claude.reading.is_none(), "a report off the mod road is not taken");
    let ledger = std::fs::read_to_string(h.paths.costs_file()).unwrap();
    assert!(!ledger.contains("\"check\""), "{ledger}");
}
