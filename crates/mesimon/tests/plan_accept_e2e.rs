//! The board accepts an agent's plan (T-420), end to end: the stub paints
//! Claude Code's plan dialog at its default row, the hook lands the `≡`,
//! and a `PromptSession` with `accept_plan` — blank, or with words — makes
//! the daemon press ONE Enter into the pane. `read` returns on a newline,
//! so a `got:` line (or `got:planned `, the spawn's title prefill submitted
//! by the first Enter) proves the Enter and nothing else went. The words wait
//! for the idle after the accepted turn; a pane at `Plan` with no dialog
//! the daemon recognises gives the flag up after its tries and says so in
//! the feed, and the words keep waiting as an unflagged ask does.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{Reason, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

/// Claude Code's dialog as the pane shows it, then every line read echoed
/// as `got:<line>` — an Enter alone is `got:`. A line reading `clear`
/// wipes the screen, so the same pane can then show NO dialog.
const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
printf ' Would you like to proceed?\\n\\n ❯ 1. Yes, and use auto mode\\n   2. Yes, manually approve edits\\n   3. No, keep planning\\n   4. Tell Claude what to change\\n'\n\
while IFS= read -r line; do \
  case \"$line\" in clear) printf '\\033[2J\\033[H';; esac; \
  printf 'got:%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

#[test]
fn the_board_accepts_a_plan_with_one_enter_and_the_words_wait_for_the_turn_after() {
    let Some(h) = Harness::boot_with_env(
        "planok",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let mut c = h.client("planok");
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    // An Enter of the daemon's submits whatever sits in the box: the title
    // the spawn typed as a prefill (T-224) the first time, nothing after.
    // A paste brings its own Enter and its own words, so those lines are
    // not presses.
    let presses =
        || text().lines().filter(|l| matches!(*l, "got:" | "got:planned " | "got:planned")).count();

    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "planned".into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();
    let plan = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##;
    let start = |c: &mut TestClient| {
        hook_send(&hooks, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient| {
        hook_send(&hooks, &session, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let plan_ready = |c: &mut TestClient| {
        hook_send(&hooks, &session, "PreToolUse", plan);
        c.await_state(sid, "plan ready", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Plan }
        });
    };
    let approved = |c: &mut TestClient| {
        hook_send(&hooks, &session, "PostToolUse", plan);
        c.await_state(sid, "running on the plan", |s| *s == SessionState::Running);
    };
    // Let the stub paint before anything reads the screen.
    std::thread::sleep(Duration::from_millis(800));
    start(&mut c);
    stop(&mut c);

    // (1) The `≡` is up: a blank accept is one Enter, and the entry leaves
    // the queue with it — nothing was asked. The harness confirms it
    // through its own hook, and the feed names the press.
    plan_ready(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: String::new(),
            queued: false,
            accept_plan: true,
        }),
        Response::Queued { .. }
    ));
    wait_until(Duration::from_secs(10), "the accept's Enter", || presses() == 1);
    wait_until(Duration::from_secs(5), "the queue to empty", || {
        pending_of(&mut c, Some(ticket)).is_empty()
    });
    assert_eq!(text(), "got:planned \n", "one Enter and nothing else went: {:?}", text());
    approved(&mut c);
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"plan_accepted""#)
    });
    stop(&mut c);

    // (2) Queued while the agent still works, with the flag: nothing goes
    // until the plan is ready; then the Enter, and the words only on the
    // idle after the accepted turn.
    start(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-420 then do this".into(),
            queued: true,
            accept_plan: true,
        }),
        Response::Queued { .. }
    ));
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| p.accept_plan && !p.in_flight), "{p:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(), 1, "no plan yet, no Enter");
    plan_ready(&mut c);
    wait_until(Duration::from_secs(10), "the second accept's Enter", || presses() == 2);
    wait_until(Duration::from_secs(5), "the flag to be spent", || {
        pending_of(&mut c, Some(ticket)).iter().any(|p| !p.accept_plan && !p.in_flight)
    });
    assert!(!text().contains("mesimon-probe-420"), "the words wait for the turn: {:?}", text());
    approved(&mut c);
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!text().contains("mesimon-probe-420"), "still working: {:?}", text());
    stop(&mut c);
    wait_until(Duration::from_secs(10), "the words after the turn", || {
        text().contains("got:mesimon-probe-420 then do this")
    });
    assert_eq!(presses(), 2, "the paste's own Enter rides the paste: {:?}", text());
    // The paste is acked by the next prompt hook.
    start(&mut c);
    wait_until(Duration::from_secs(5), "the ack", || pending_of(&mut c, Some(ticket)).is_empty());
    stop(&mut c);

    // (3) A pane at `Plan` showing no dialog the daemon recognises: the
    // flag is given up after its tries, the feed says so, and the words
    // stay queued the way an unflagged ask does — behind the `≡`.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "clear".into(),
            queued: false,
            accept_plan: false,
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(5), "the screen to clear", || text().contains("got:clear"));
    start(&mut c);
    stop(&mut c);
    plan_ready(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-421 unrecognised".into(),
            queued: true,
            accept_plan: true,
        }),
        Response::Queued { .. }
    ));
    wait_until(Duration::from_secs(25), "the flag to be given up", || {
        feed().contains(r#""cmd":"plan_accept_unrecognised""#)
    });
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| !p.accept_plan && !p.in_flight), "{p:?}");
    assert_eq!(presses(), 2, "no blind Enter: {:?}", text());
    assert!(!text().contains("mesimon-probe-421"));

    let _ = c.request(Command::Shutdown);
}
