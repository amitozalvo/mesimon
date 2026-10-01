//! Agent tiers (T-443) through the real daemon: a tier's model and effort
//! ride the launch argv from either layer (the machine's `tiers.toml`, a
//! board's override); a ticket's pick on a running seat is owed and happens
//! at its next idle — the seat parked and woken on the new tier, the same
//! conversation resumed — never mid-turn, never while a person is inside
//! the pane; an ask sent with a pick rides the relaunch; and a seat keeps
//! its provider.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use mesimon_core::board::{AgentProvider, SessionKind, SessionRecord, SessionState, StopReason};
use mesimon_core::command::{Command, Response};
use mesimon_core::tier::{Effort, Tier, TierScope};

mod common;
use common::*;

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";
const DONE: &str = r#"{"stop_hook_active":false,"background_tasks":[]}"#;

fn tier(id: &str, name: &str, provider: AgentProvider, model: &str, effort: Effort) -> Tier {
    Tier { id: id.into(), name: name.into(), provider, model: model.into(), effort }
}

fn create(c: &mut TestClient, title: &str, pick: Option<&str>) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: pick.map(str::to_string),
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    }
}

fn record(c: &mut TestClient, id: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == id).expect("session")
}

fn flag(rec: &SessionRecord, name: &str) -> Option<String> {
    let at = rec.argv.iter().position(|a| a == name)?;
    rec.argv.get(at + 1).cloned()
}

/// Spawn a claude on the ticket and give it a conversation on disk, so a
/// relaunch resumes it (a switch that could only start fresh never runs).
fn start(h: &Harness, c: &mut TestClient, ticket: ulid::Ulid) -> (uuid::Uuid, uuid::Uuid) {
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let conversation = uuid::Uuid::new_v4();
    let path = h.dir.join(format!("{conversation}.jsonl"));
    std::fs::write(&path, "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n").unwrap();
    hook_send(
        &h.paths.hook_sock(),
        &id.to_string(),
        "SessionStart",
        &serde_json::json!({"session_id": conversation, "transcript_path": path}).to_string(),
    );
    c.await_state(id, "started", |s| matches!(s, SessionState::Idle { .. }));
    (id, conversation)
}

fn run(h: &Harness, c: &mut TestClient, id: uuid::Uuid) {
    hook_send(&h.paths.hook_sock(), &id.to_string(), "UserPromptSubmit", "{}");
    c.await_state(id, "running", |s| *s == SessionState::Running);
}

fn stop(h: &Harness, c: &mut TestClient, id: uuid::Uuid) {
    hook_send(&h.paths.hook_sock(), &id.to_string(), "Stop", DONE);
    c.await_state(id, "finished", |s| {
        matches!(
            s,
            SessionState::Idle { stop_reason: StopReason::EndTurn } | SessionState::Spawning
        )
    });
}

/// The relaunch landed: a new pane on the same record, on `tier`.
fn await_relaunch(c: &mut TestClient, id: uuid::Uuid, pane: &Option<String>, tier: &str) {
    wait_until(Duration::from_secs(15), "the tier relaunch", || {
        let rec = record(c, id);
        rec.pane_key != *pane && rec.tier == tier && !rec.tier_owed
    });
}

#[test]
fn a_tier_rides_the_launch_and_a_pick_switches_a_running_seat_at_its_idle() {
    let Some(h) = Harness::boot_with_env(
        "tiers",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let mut c = h.client("tiers");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();

    // ---- the two layers ------------------------------------------------------
    // A machine tier lands in the machine's file (the test HOME's), and a
    // board entry with its id overrides it in place.
    let coder = tier("01CODER", "coder", AgentProvider::ClaudeCode, "opus", Effort::Xhigh);
    let quick = tier("01QUICK", "quick", AgentProvider::ClaudeCode, "sonnet", Effort::High);
    for t in [&coder, &quick] {
        let saved = c.request(Command::SaveTier { scope: TierScope::Machine, tier: t.clone() });
        assert!(matches!(saved, Response::Ok), "{saved:?}");
    }
    let machine_file = h.dir.join("home/.local/state/mesimon/tiers.toml");
    let text = std::fs::read_to_string(&machine_file).expect("the machine's tiers file");
    assert!(text.contains("name = \"coder\"") && text.contains("schema_version = 1"), "{text}");
    let over = Tier { effort: Effort::Max, ..coder.clone() };
    assert!(matches!(
        c.request(Command::SaveTier { scope: TierScope::Board, tier: over }),
        Response::Ok
    ));
    // A name another tier holds is refused, whatever its case.
    let dup = tier("01DUP", "Coder", AgentProvider::ClaudeCode, "", Effort::Default);
    err_containing(c.request(Command::SaveTier { scope: TierScope::Board, tier: dup }), "already");

    // ---- the order (T-562) ----------------------------------------------------
    // The machine's order is its file's; a board's version of a machine tier
    // is not the board's to move. Moved there and back, so the rings below
    // read as they were.
    let order = || {
        let text = std::fs::read_to_string(&machine_file).unwrap_or_default();
        (text.find("name = \"quick\""), text.find("name = \"coder\""))
    };
    let mv = |id: &str, to_index| Command::MoveTier {
        scope: TierScope::Machine,
        id: id.into(),
        to_index,
    };
    assert!(matches!(c.request(mv("01QUICK", 0)), Response::Ok));
    let (q, k) = order();
    assert!(q.zip(k).is_some_and(|(q, k)| q < k), "quick moved above coder on disk");
    assert!(matches!(c.request(mv("01QUICK", 1)), Response::Ok));
    let (q, k) = order();
    assert!(q.zip(k).is_some_and(|(q, k)| k < q), "and back");
    err_containing(
        c.request(Command::MoveTier { scope: TierScope::Board, id: "01CODER".into(), to_index: 0 }),
        "the machine orders",
    );

    // ---- a start on the ticket's pick ----------------------------------------
    let t1 = create(&mut c, "picked at the composer", Some("01CODER"));
    assert_eq!(c.board().ticket(t1).unwrap().tier.as_deref(), Some("01CODER"));
    let (s1, conversation) = start(&h, &mut c, t1);
    let rec = record(&mut c, s1);
    assert_eq!(flag(&rec, "--model").as_deref(), Some("opus"));
    assert_eq!(flag(&rec, "--effort").as_deref(), Some("max"), "the board's override wins");
    assert_eq!(rec.tier, "01CODER");

    // ---- an idle seat switches after the settle, on the same conversation ----
    let pane = rec.pane_key.clone();
    assert!(matches!(
        c.request(Command::SetTicketTier { id: t1, tier: Some("01QUICK".into()) }),
        Response::Ok
    ));
    assert!(record(&mut c, s1).tier_owed, "owed the moment it is picked");
    await_relaunch(&mut c, s1, &pane, "01QUICK");
    let rec = record(&mut c, s1);
    assert_eq!(flag(&rec, "--model").as_deref(), Some("sonnet"));
    assert_eq!(flag(&rec, "--effort").as_deref(), Some("high"));
    assert_eq!(flag(&rec, "--resume"), Some(conversation.to_string()), "the same conversation");
    assert_eq!(rec.argv.iter().filter(|a| *a == "--model").count(), 1, "never two");
    assert_eq!(c.board().sessions.iter().filter(|s| s.ticket == t1).count(), 1, "one seat");
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"tier_switch""#)
    });

    // ---- a working seat waits for its idle ------------------------------------
    hook_send(
        &h.paths.hook_sock(),
        &s1.to_string(),
        "SessionStart",
        &serde_json::json!({"session_id": conversation, "source": "resume",
            "transcript_path": h.dir.join(format!("{conversation}.jsonl"))})
        .to_string(),
    );
    c.await_state(s1, "resumed", |s| matches!(s, SessionState::Idle { .. }));
    run(&h, &mut c, s1);
    let pane = record(&mut c, s1).pane_key;
    // Back to the default, which stores as inherit.
    assert!(matches!(
        c.request(Command::SetTicketTier { id: t1, tier: Some("claude".into()) }),
        Response::Ok
    ));
    assert_eq!(c.board().ticket(t1).unwrap().tier, None, "the default is stored as inherit");
    std::thread::sleep(Duration::from_millis(3500));
    let rec = record(&mut c, s1);
    assert_eq!(rec.pane_key, pane, "a turn is never cut");
    assert!(rec.tier_owed);
    stop(&h, &mut c, s1);
    await_relaunch(&mut c, s1, &pane, "claude");
    let rec = record(&mut c, s1);
    assert!(flag(&rec, "--model").is_none() && flag(&rec, "--effort").is_none(), "{:?}", rec.argv);

    // ---- an ask with a pick, sent now at a working pane: it queues, then the
    //      switch carries it -------------------------------------------------
    hook_send(
        &h.paths.hook_sock(),
        &s1.to_string(),
        "SessionStart",
        &serde_json::json!({"session_id": conversation, "source": "resume",
            "transcript_path": h.dir.join(format!("{conversation}.jsonl"))})
        .to_string(),
    );
    c.await_state(s1, "resumed again", |s| matches!(s, SessionState::Idle { .. }));
    run(&h, &mut c, s1);
    let pane = record(&mut c, s1).pane_key;
    match c.request(Command::PromptSession {
        ticket: t1,
        text: "mesimon-probe-443 after the switch".into(),
        queued: false,
        accept_plan: false,
        plan: false,
        tier: Some("01CODER".into()),
        resend: false,
    }) {
        Response::Queued { .. } => {}
        other => panic!("a working pane owed a switch must queue the words: {other:?}"),
    }
    assert_eq!(c.board().ticket(t1).unwrap().tier.as_deref(), Some("01CODER"));
    stop(&h, &mut c, s1);
    await_relaunch(&mut c, s1, &pane, "01CODER");
    let rec = record(&mut c, s1);
    assert!(rec.pending_submit, "the words are parked for the relaunched pane");
    assert!(pending_of(&mut c, Some(t1)).is_empty(), "the ask left the queue with the switch");

    // ---- a seat keeps its provider; an empty seat may take any --------------
    let reviewer = tier("01REVIEW", "reviewer", AgentProvider::Codex, "gpt-6-astra", Effort::High);
    assert!(matches!(
        c.request(Command::SaveTier { scope: TierScope::Board, tier: reviewer }),
        Response::Ok
    ));
    err_containing(
        c.request(Command::SetTicketTier { id: t1, tier: Some("01REVIEW".into()) }),
        "fresh",
    );
    let t2 = create(&mut c, "empty seat", None);
    assert!(matches!(
        c.request(Command::SetTicketTier { id: t2, tier: Some("01REVIEW".into()) }),
        Response::Ok
    ));
    assert_eq!(c.board().ticket(t2).unwrap().tier.as_deref(), Some("01REVIEW"));
    err_containing(
        c.request(Command::SetTicketTier { id: t2, tier: Some("nope".into()) }),
        "no tier",
    );

    // ---- a person inside the pane is never thrown out --------------------------
    let t3 = create(&mut c, "focused", None);
    let (s3, _) = start(&h, &mut c, t3);
    let pane = record(&mut c, s3).pane_key;
    let mut holder = h.client("focus");
    let focused = holder.request(Command::FocusStart { session: s3 });
    assert!(!matches!(focused, Response::Err { .. }), "{focused:?}");
    assert!(matches!(
        c.request(Command::SetTicketTier { id: t3, tier: Some("01QUICK".into()) }),
        Response::Ok
    ));
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(record(&mut c, s3).pane_key, pane, "the focused pane is left alone");
    assert!(matches!(holder.request(Command::FocusEnd { session: s3 }), Response::Ok));
    await_relaunch(&mut c, s3, &pane, "01QUICK");

    // ---- deleting a tier returns this board's tickets to the default ---------
    assert!(matches!(
        c.request(Command::DeleteTier { scope: TierScope::Machine, id: "01QUICK".into() }),
        Response::Ok
    ));
    assert_eq!(c.board().ticket(t3).unwrap().tier, None);
    assert!(!record(&mut c, s3).tier_owed, "nobody picked what it now resolves to");
}
