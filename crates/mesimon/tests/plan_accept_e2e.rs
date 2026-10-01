//! The board accepts an agent's plan (T-420), end to end: the stub paints
//! Claude Code's plan dialog at its default row, the hook lands the `≡`,
//! and a `PromptSession` with `accept_plan` — blank, or with words — makes
//! the daemon press ONE Enter into the pane. `read` returns on a newline,
//! so a `got:` line (or `got:planned `, the spawn's title prefill submitted
//! by the first Enter) proves the Enter and nothing else went. The words go
//! in the moment the harness confirms the press; a pane at `Plan` with no dialog
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
fn the_board_accepts_a_plan_with_one_enter_and_the_words_ride_the_approval() {
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
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
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
            plan: false,
            tier: None,
            resend: false,
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
    // until the plan is ready; then the Enter, and the words the moment
    // the harness confirms it — at the head of the approved turn, not on
    // the idle after.
    start(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-420 then do this".into(),
            queued: true,
            accept_plan: true,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| p.accept_plan && !p.in_flight), "{p:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(), 1, "no plan yet, no Enter");
    plan_ready(&mut c);
    wait_until(Duration::from_secs(10), "the second accept's Enter", || presses() == 2);
    assert!(!text().contains("mesimon-probe-420"), "the words wait for the approval: {:?}", text());
    assert!(
        pending_of(&mut c, Some(ticket)).iter().any(|p| p.accept_plan),
        "the row says accepting until the harness confirms"
    );
    approved(&mut c);
    wait_until(Duration::from_secs(10), "the words at the head of the approved turn", || {
        text().contains("got:mesimon-probe-420 then do this")
    });
    assert_eq!(presses(), 2, "the paste's own Enter rides the paste: {:?}", text());
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"queued_ask_sent_after_plan""#)
    });
    // The paste is acked by the next prompt hook, then the turn ends.
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
            plan: false,
            tier: None,
            resend: false,
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
            plan: false,
            tier: None,
            resend: false,
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

/// The dialog again, each line echoed with the pane's ticket key in front
/// (`MESIMON_TICKET`, which every spawn gets), so two panes writing one
/// file can be told apart.
const STUB_KEYED: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
printf ' Would you like to proceed?\\n\\n ❯ 1. Yes, and use auto mode\\n   2. Yes, manually approve edits\\n   3. No, keep planning\\n   4. Tell Claude what to change\\n'\n\
while IFS= read -r line; do \
  printf 'got:%s:%s\\n' \"$MESIMON_TICKET\" \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// T-429: accepts obey the checkout. Two shared-checkout tickets, both on
/// their plan dialogs, both accepted from the board: ONE Enter lands — the
/// first in board order — and the second's row names the first
/// (`accepts plan ∙ after T-1`). The second press goes only when the first
/// agent's approved turn has ended, never while it implements. Then the
/// column's own accept: `PromptColumn` with the flag parks both, and the
/// presses go the same way, the first now and the other as the checkout
/// goes quiet. Worktree tickets are unaffected — their checkout is their
/// own — and that is `checkout_holders`' cwd clause, not tested again here.
#[test]
fn accepts_go_one_per_quiet_checkout_and_a_column_accepts_every_plan() {
    let Some(h) = Harness::boot_with_env(
        "planq",
        Some(STUB_KEYED),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let mut c = h.client("planq");
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();

    let mut make = |title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let (ta, tb) = (make("planA"), make("planB"));
    let board = c.board();
    let key = |t: ulid::Ulid| board.ticket(t).unwrap().short_key.clone();
    let (ka, kb) = (key(ta), key(tb));
    // An Enter of the daemon's submits whatever the box holds: the title
    // prefill the first time, nothing after. Counted per pane.
    let presses = |k: &str| {
        let own = format!("got:{k}:");
        text()
            .lines()
            .filter(|l| {
                l.strip_prefix(own.as_str()).is_some_and(|rest| {
                    matches!(rest, "" | "planA" | "planA " | "planB" | "planB ")
                })
            })
            .count()
    };
    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let (sa, sb) = (spawn(&mut c, ta), spawn(&mut c, tb));
    let plan = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##;
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let plan_ready = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "PreToolUse", plan);
        c.await_state(sid, "plan ready", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Plan }
        });
    };
    let approved = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "PostToolUse", plan);
        c.await_state(sid, "running on the plan", |s| *s == SessionState::Running);
    };
    let accept = |c: &mut TestClient, ticket| {
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket,
                text: String::new(),
                queued: false,
                accept_plan: true,
                plan: false,
                tier: None,
                resend: false,
            }),
            Response::Queued { .. }
        ));
    };
    // Let both stubs paint, then walk each through a turn so the checkout
    // starts quiet (a stub emits no `SessionStart`, so a fresh record sits
    // at `Spawning`, which is WORKING).
    std::thread::sleep(Duration::from_millis(800));
    for sid in [sa, sb] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    // One column, A above B: the presses go in board order.
    let column = c.board().ticket(ta).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: tb, column: column.clone(), before: None }),
        Response::Ok
    ));

    // (1) Both dialogs up, both accepted from the board. A's Enter goes;
    // B's waits on A and its row says so.
    plan_ready(&mut c, sa);
    plan_ready(&mut c, sb);
    accept(&mut c, ta);
    accept(&mut c, tb);
    wait_until(Duration::from_secs(10), "A's Enter", || presses(&ka) == 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 0, "B waits on A's press in flight: {:?}", text());
    let pb = pending_of(&mut c, Some(tb));
    assert!(pb.iter().any(|p| p.accept_plan && p.waits_on == vec![ka.clone()]), "{pb:?}");
    // A's approval is confirmed and A implements: still B's checkout is
    // held, by a working agent now.
    approved(&mut c, sa);
    wait_until(Duration::from_secs(5), "A's entry to leave the queue", || {
        pending_of(&mut c, Some(ta)).is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 0, "B waits while A implements: {:?}", text());
    let pb = pending_of(&mut c, Some(tb));
    assert!(pb.iter().any(|p| p.accept_plan && p.waits_on == vec![ka.clone()]), "{pb:?}");
    // A's turn ends: B's press goes.
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "B's Enter after A's turn", || presses(&kb) == 1);
    assert_eq!(presses(&ka), 1, "A was pressed once: {:?}", text());
    approved(&mut c, sb);
    wait_until(Duration::from_secs(5), "B's entry to leave the queue", || {
        pending_of(&mut c, Some(tb)).is_empty()
    });
    stop(&mut c, sb);

    // (2) The column accepts every plan (T-429): both on their dialogs
    // again, one `PromptColumn` with the flag and no words parks both —
    // the receipt counts them — and the presses go as before: A's now, B's
    // when A's approved turn has ended.
    plan_ready(&mut c, sa);
    plan_ready(&mut c, sb);
    // Automove carried both cards on as their turns ended; put B under A
    // again so the order is the test's, not the automove's.
    let column = c.board().ticket(ta).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: tb, column: column.clone(), before: None }),
        Response::Ok
    ));
    match c.request(Command::PromptColumn {
        column: column.clone(),
        text: String::new(),
        queued: true,
        accept_plan: true,
    }) {
        Response::Asked { accepts, sent, woke, started, skipped, failed, .. } => {
            assert_eq!((accepts, sent, woke, started, skipped, failed), (2, 0, 0, 0, 0, 0));
        }
        other => panic!("{other:?}"),
    }
    wait_until(Duration::from_secs(10), "A's second Enter", || presses(&ka) == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 1, "B waits on A again: {:?}", text());
    approved(&mut c, sa);
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "B's second Enter", || presses(&kb) == 2);
    approved(&mut c, sb);
    wait_until(Duration::from_secs(5), "the queue to empty", || {
        pending_of(&mut c, None).is_empty()
    });
    // No Enter went anywhere it was not owed.
    assert_eq!((presses(&ka), presses(&kb)), (2, 2), "{:?}", text());

    let _ = c.request(Command::Shutdown);
}

/// T-447 (2026-09-23): "No, keep planning" fires no hook of its own — no
/// `PostToolUse`, no `PostToolUseFailure`, no `PermissionDenied` — so the
/// agent's next tool call is the first frame that says the dialog is gone.
/// The card wore "plan" through four tool completions and cleared only
/// when the agent asked its next question. A subagent's call (`agent_id`)
/// is not the lead's answer and leaves the dialog held.
#[test]
fn a_refused_plan_clears_on_the_agents_next_tool_call() {
    let Some(h) = Harness::boot_with_env(
        "planno",
        Some("#!/bin/sh\nexec sleep 120\n"),
        &[("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let mut c = h.client("planno");
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "refused".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();
    hook_send(&hooks, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    for (dialog, reason) in [
        (r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##, Reason::Plan),
        (r#"{"tool_name":"AskUserQuestion"}"#, Reason::Question),
    ] {
        hook_send(&hooks, &session, "PreToolUse", dialog);
        c.await_state(sid, "dialog up", |s| *s == SessionState::RequiresAction { reason });
        // A subagent reading while the dialog is open is not the answer.
        hook_send(
            &hooks,
            &session,
            "PreToolUse",
            r#"{"tool_name":"Read","tool_input":{"file_path":"/x"},"agent_id":"sub-1"}"#,
        );
        hook_send(
            &hooks,
            &session,
            "PostToolUse",
            r#"{"tool_name":"Read","tool_input":{"file_path":"/x"},"agent_id":"sub-1"}"#,
        );
        std::thread::sleep(Duration::from_millis(2000));
        let held = board_of(c.request(Command::Snapshot))
            .sessions
            .iter()
            .any(|s| s.id == sid && s.state == SessionState::RequiresAction { reason });
        assert!(held, "a subagent's call left {reason:?} held");
        // The refusal: no frame for it — the lead's own next call clears.
        hook_send(
            &hooks,
            &session,
            "PreToolUse",
            r#"{"tool_name":"Bash","tool_input":{"command":"grep -n x"}}"#,
        );
        c.await_state(sid, "working after the refusal", |s| *s == SessionState::Running);
    }
}
