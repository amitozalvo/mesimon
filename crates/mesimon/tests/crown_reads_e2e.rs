//! What the crown reads of a worker beyond its card (T-969): its agent's
//! context fill, off the transcript the session points at, and the ticket's
//! cost as its card shows it, each on the crown's `get_ticket` behind a
//! board switch (Settings → Agents → Crown reads context, on by default;
//! Crown reads cost, off). And a `/` text the crown's `ask_agent` sends is
//! typed into the agent's box and entered, never a mod's `submit`: a
//! Claude worker on either road, and a Codex worker on the synthetic
//! runtime.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AgentTicketView, Command, Deliver, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

const RECORDING_STUB: &str =
    "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; \
                              printf '%s\\n' \"$line\" \
                              >> \"$(dirname \"$0\")/got-${MESIMON_TICKET:-none}.txt\"; done\n";

fn key_of(c: &mut TestClient, id: ulid::Ulid) -> String {
    c.board().ticket(id).unwrap().short_key.clone()
}

fn read(c: &mut TestClient, session: uuid::Uuid, key: &str) -> AgentTicketView {
    match c.send(
        Principal::Agent { session },
        Command::AgentReadTicket { key: key.into(), full: true, plan: false },
    ) {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("read {key}: {other:?}"),
    }
}

/// One Claude reply whose input is `fill` tokens, split as Claude Code
/// writes it.
fn reply(id: &str, fill: u64) -> String {
    json!({
        "type": "assistant", "isSidechain": false, "requestId": format!("r-{id}"),
        "timestamp": "2026-10-25T00:00:00.000Z",
        "message": { "id": id, "model": "claude-opus-5-5", "usage": {
            "input_tokens": 2, "cache_read_input_tokens": fill - 1_002,
            "cache_creation_input_tokens": 1_000,
            "cache_creation": { "ephemeral_1h_input_tokens": 1_000 }, "output_tokens": 40 } }
    })
    .to_string()
}

fn append(path: &std::path::Path, line: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(f, "{line}").unwrap();
}

/// A turn's end, which owes the cost ledger a pass; the pass has landed once
/// `costs.json` says it read the whole of `transcript`.
fn turn_read(h: &Harness, sid: uuid::Uuid, transcript: &std::path::Path) {
    hook_send(&h.paths.hook_sock(), &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    let len = std::fs::metadata(transcript).unwrap().len();
    let file = h.paths.costs_file();
    wait_until(Duration::from_secs(10), "the transcript read", || {
        std::fs::read_to_string(&file).is_ok_and(|t| t.contains(&format!("\"offset\":{len}")))
    });
}

#[test]
fn the_crown_reads_a_worker_s_context_and_cost_behind_their_switches() {
    let Some(h) = Harness::boot_with_env(
        "crown_reads",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_reads");
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("a long worker");
    let n = c.create_ticket("a worker with no transcript");
    let (kw, kn) = (key_of(&mut c, w), key_of(&mut c, n));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    let sw = c.spawn_claude(w);
    let _sn = c.spawn_claude(n);
    let transcript = h.dir.join("worker.jsonl");
    hook_send_with(
        &hook_sock,
        &sw.to_string(),
        "SessionStart",
        Some("startup"),
        &json!({
            "session_id": "c0ffee00-0000-0000-0000-000000000969",
            "transcript_path": transcript,
            "cwd": h.repo,
        })
        .to_string(),
    );
    let board = c.board();
    assert!(board.crown_context && !board.crown_cost, "the defaults");

    // Before any line is read: no reading, and no line about one.
    let v = read(&mut c, sa, &kw);
    assert!(v.context.is_none() && v.context_about.is_none(), "{:?}", v.context);
    assert!(v.cost.is_none() && v.cost_about.is_none(), "cost is off");

    // Two replies: the last one's input is the reading.
    append(&transcript, &reply("m1", 300_000));
    append(&transcript, &reply("m2", 812_345));
    turn_read(&h, sw, &transcript);
    let v = read(&mut c, sa, &kw);
    let ctx = v.context.expect("the worker's context");
    assert_eq!(
        (ctx.tokens, ctx.window, ctx.pct, ctx.compacted),
        (Some(812_345), 1_000_000, Some(81), false)
    );
    let about = v.context_about.expect("its line");
    assert!(about.contains("not the engine's own counter"), "{about}");
    assert!(v.cost.is_none(), "cost is still off");
    // A worker with no transcript read has no reading.
    assert!(read(&mut c, sa, &kn).context.is_none());

    // A compaction: the reading waits for the next reply.
    append(
        &transcript,
        &json!({"isSidechain": false, "type": "system", "subtype": "compact_boundary",
                "compactMetadata": {"trigger": "manual", "preTokens": 812_400, "postTokens": 16_800}})
        .to_string(),
    );
    turn_read(&h, sw, &transcript);
    let ctx = read(&mut c, sa, &kw).context.expect("a compacted reading");
    assert_eq!((ctx.tokens, ctx.pct, ctx.compacted), (None, None, true));
    append(&transcript, &reply("m3", 92_957));
    turn_read(&h, sw, &transcript);
    let ctx = read(&mut c, sa, &kw).context.unwrap();
    assert_eq!((ctx.tokens, ctx.pct, ctx.compacted), (Some(92_957), Some(9), false));

    // Cost on: the card's figure, with the tokens by kind.
    assert!(matches!(c.request(Command::SetCrownCost { on: true }), Response::Ok));
    let v = read(&mut c, sa, &kw);
    let cost = v.cost.expect("the ticket's cost");
    assert_eq!((cost.input, cost.cache_write, cost.output), (6, 3_000, 120), "{cost:?}");
    assert_eq!(cost.tokens, cost.input + cost.output + cost.cache_write + cost.cache_read);
    // The card's own figure: the snapshot's account of the same ticket.
    let card = match c.request(Command::Snapshot) {
        Response::Board { costs, .. } => {
            costs.into_iter().find(|x| x.ticket == w).expect("W's cost")
        }
        other => panic!("snapshot: {other:?}"),
    };
    assert_eq!(cost.usd, mesimon_core::cost::usd_word(card.usd));
    assert_eq!((cost.tokens, cost.unpriced), (card.tokens, card.unpriced));
    assert!(v.cost_about.unwrap().contains("not a bill"));

    // Round 1: the reads are the crown's. Over the real MCP shim, with
    // both switches on and W's reading and cost on hand, W's own reads,
    // unkeyed and by its own key, carry none of the four fields; the
    // crown's of its own ticket, unkeyed and keyed, carry its own.
    let four = ["context", "context_about", "cost", "cost_about"];
    let crown_transcript = h.dir.join("crown.jsonl");
    hook_send_with(
        &hook_sock,
        &sa.to_string(),
        "SessionStart",
        Some("startup"),
        &json!({
            "session_id": "c0ffee00-0000-0000-0000-000000000970",
            "transcript_path": crown_transcript,
            "cwd": h.repo,
        })
        .to_string(),
    );
    append(&crown_transcript, &reply("k1", 400_000));
    turn_read(&h, sa, &crown_transcript);
    let ka = key_of(&mut c, a);
    let mut worker = Shim::start(&h.paths.orch_sock(), sw);
    worker.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    worker.notify("notifications/initialized");
    for args in [json!({}), json!({ "key": kw })] {
        let answer = worker.call_ok("get_ticket", args.clone());
        assert_eq!(answer["key"], json!(kw), "{args}");
        for f in four {
            assert!(answer.get(f).is_none(), "a worker's {args} read carries {f}: {answer}");
        }
    }
    let mut crown = Shim::start(&h.paths.orch_sock(), sa);
    crown.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    crown.notify("notifications/initialized");
    let compact = crown.call_ok("get_ticket", json!({"key": ka}));
    assert_eq!(compact["context"]["tokens"], json!(400_000));
    assert!(compact.get("cost").is_some());
    assert!(compact.get("context_about").is_none());
    assert!(compact.get("cost_about").is_none());
    for args in [json!({}), json!({ "key": ka, "full": true })] {
        let answer = crown.call_ok("get_ticket", args.clone());
        for f in four {
            assert!(answer.get(f).is_some(), "the crown's {args} read lacks {f}: {answer}");
        }
        assert_eq!(answer["context"]["tokens"], json!(400_000), "{args}");
    }
    let answer = crown.call_ok("get_ticket", json!({ "key": kw }));
    assert_eq!(answer["context"]["tokens"], json!(92_957));
    assert_eq!(answer["cost"]["usd"], json!(cost.usd));
    assert!(read(&mut c, sa, &kn).cost.is_none(), "nothing counted, nothing said");

    // Context off over the wire: gone with its line; cost stays.
    assert!(matches!(c.request(Command::SetCrownContext { on: false }), Response::Ok));
    assert!(!c.board().crown_context);
    let v = read(&mut c, sa, &kw);
    assert!(v.context.is_none() && v.context_about.is_none());
    assert!(v.cost.is_some());
    let file = std::fs::read_to_string(h.repo.join(".mesimon/board/columns.toml")).unwrap();
    assert!(file.contains("crown_context = false") && file.contains("crown_cost = true"), "{file}");
    // And cost off: neither.
    assert!(matches!(c.request(Command::SetCrownCost { on: false }), Response::Ok));
    let v = read(&mut c, sa, &kw);
    assert!(v.cost.is_none() && v.cost_about.is_none());
    // An agent cannot turn either switch.
    for cmd in [Command::SetCrownContext { on: false }, Command::SetCrownCost { on: true }] {
        assert!(matches!(c.send(Principal::Agent { session: sa }, cmd), Response::Err { .. }));
    }
}

/// `/compact` through the crown's `ask_agent` to a Claude worker the crown
/// started: typed into its box and entered, as a person types it. On the
/// mod road the fake mod sees no `submit` for it (`$.prompt.submit`
/// refuses a leading `/`).
#[test]
fn the_crown_s_slash_command_is_typed_into_a_claude_worker_s_box() {
    let Some(h) = Harness::boot_with_env(
        "crown_slash",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_slash");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-969 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw);
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(Duration::from_secs(20), "W's seat", || c.board().live_agent(w).is_some());
    let ws = c.board().live_agent(w).unwrap().id;
    assert_eq!(c.board().ticket(w).unwrap().workspace_strategy(), WorkspaceStrategy::Worktree);
    std::thread::sleep(Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    start(&mut c, sa);
    stop(&mut c, sa);
    if test_road() == "mod" {
        let pid = h.dir.join(format!("mod-bridge-{ws}.pid"));
        wait_until(Duration::from_secs(10), "W's mod bridge", || pid.exists());
        std::thread::sleep(Duration::from_millis(300));
    }
    let got = h.dir.join(format!("got-{kw}.txt"));
    let typed = || std::fs::read_to_string(&got).unwrap_or_default();
    let v = read(&mut c, sa, &kw);
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "/compact keep the case matrix".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { held_for_person, .. } => assert!(!held_for_person),
        other => panic!("ask_agent: {other:?}"),
    }
    // The stub reads its tty a line at a time: a line is the paste and its
    // Enter, and nothing else of the crown's went into W's box.
    wait_until(Duration::from_secs(10), "the command typed and entered in W's pane", || {
        typed().lines().any(|l| l.trim_end().ends_with("/compact keep the case matrix"))
    });
    wait_until(Duration::from_secs(5), "the command's line in the feed", || {
        feed_count(&h, "command_sent", w) == 1
    });
    // Words after it take the road's own way: the mod's `submit` on the
    // mod road, which the command never was.
    start(&mut c, ws);
    stop(&mut c, ws);
    let v = read(&mut c, sa, &kw);
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-969 words".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { held_for_person, .. } => assert!(!held_for_person),
        other => panic!("ask_agent words: {other:?}"),
    }
    wait_until(Duration::from_secs(10), "the words in W's pane", || {
        typed().contains("mesimon-probe-969 words")
    });
    if test_road() == "mod" {
        let frames =
            std::fs::read_to_string(h.dir.join(format!("mod-{ws}.ndjson"))).unwrap_or_default();
        assert!(
            frames.lines().any(|l| l.contains("\"submit\"") && l.contains("mesimon-probe-969")),
            "the words are the mod's submit: {frames}"
        );
        assert!(
            !frames.lines().any(|l| l.contains("/compact")),
            "a slash command is never the mod's: {frames}"
        );
    }
}

/// `/compact` through the crown's `ask_agent` to a Codex worker, on the
/// synthetic runtime: a bracketed paste and an Enter of its own after it,
/// the road Codex 0.164.0 was measured to run the command from.
#[test]
fn the_crown_s_slash_command_is_pasted_and_entered_in_a_codex_worker_s_box() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_slash_codex",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_slash_codex");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-969 codex worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
    let v = read(&mut c, sa, &kw);
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(Duration::from_secs(20), "W's Codex seat", || {
        c.board().live_agent(w).is_some_and(|s| s.kind == SessionKind::Codex)
    });
    let ws = c.board().live_agent(w).unwrap().id;
    c.await_state(ws, "the synthetic Codex at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    wait_until(Duration::from_secs(10), "W's prefill", || {
        c.board().sessions.iter().any(|s| s.id == ws && !s.pending_prefill)
    });
    // The title's turn goes in and ends: the ask waits for W's idle.
    let submits = h.dir.join(format!("{ws}.submits"));
    wait_until(Duration::from_secs(10), "W's first turn", || submits.exists());
    let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, ws);
    let control = std::path::PathBuf::from(format!("{}.control", path.display()));
    mesimon_daemon::agents::codex::write_json(
        &control,
        &json!({"state": {"state": "idle", "stop_reason": "end_turn"}, "turn_id": "synthetic-turn-1"}),
    )
    .unwrap();
    c.await_state(ws, "W's first turn ended", |s| matches!(s, SessionState::Idle { .. }));
    let input = h.dir.join(format!("{ws}.input"));
    let text = || String::from_utf8_lossy(&std::fs::read(&input).unwrap_or_default()).to_string();
    let before = text().len();
    let v = read(&mut c, sa, &kw);
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "/compact".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { held_for_person, .. } => assert!(!held_for_person),
        other => panic!("ask_agent: {other:?}"),
    }
    wait_until(Duration::from_secs(10), "the command pasted, then an Enter of its own", || {
        let after = text()[before..].to_string();
        after
            .split_once("\x1b[200~/compact\x1b[201~")
            .is_some_and(|(_, rest)| rest.contains('\r') || rest.contains('\n'))
    });
}
