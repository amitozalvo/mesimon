//! A compaction in flight is painted and moves nothing (T-894). The hook
//! frames of a compaction — `PreCompact`, `SessionStart{compact}`,
//! `PostCompact` — reach the daemon through the real hook binary on both
//! roads, and the snapshot's record carries the window (`compacting`) from
//! the first to the last, with the state and the column it began in kept
//! throughout: a person's `/compact` at a finished turn, a quiet one with a
//! daemon restart inside it, an automatic one inside a turn, one a person
//! interrupts with Esc (no hook: the screen closes it), and a vetoed one
//! that fires `PreCompact` alone. A queued ask waits out the window. And a
//! `/compact` sent from the board is acked off the screen, since it fires
//! no `UserPromptSubmit`.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionRecord, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

/// A stand-in for Claude Code's pane (T-894): it reads its input into
/// `got.txt`, and while `compacting` exists beside it paints the spinner
/// line Claude Code draws for the whole of a compaction —
/// `✻ Compacting conversation… (1s)` — above the composer the harness
/// paints. An Esc out of a real compaction fires no hook and takes that
/// line away, which is what removing the file does here.
const STUB: &str = "#!/bin/sh\nd=\"$(dirname \"$0\")\"\n\
    ( while :; do if [ -e \"$d/compacting\" ]; then \
    printf '\\0337\\033[3;1H✻ Compacting conversation… (1s)\\033[K\\0338'; \
    else printf '\\0337\\033[3;1H\\033[K\\0338'; fi; sleep 0.2; done ) &\n\
    stty -icanon 2>/dev/null\n\
    cat | while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$d/got.txt\"; done\n";

fn hook_sock(h: &Harness) -> std::path::PathBuf {
    h.paths.hook_sock()
}

/// A compaction begins: its `PreCompact`, and the pane's line painted.
fn pre(h: &Harness, trigger: &str) {
    std::fs::write(h.dir.join("compacting"), "").unwrap();
    let body = format!(r#"{{"trigger":"{trigger}"}}"#);
    hook_send(&hook_sock(h), &session_of(h), "PreCompact", &body);
}

/// A compaction completes: its `PostCompact`, and the line gone.
fn post(h: &Harness, trigger: &str) {
    let body = format!(r#"{{"trigger":"{trigger}"}}"#);
    hook_send(&hook_sock(h), &session_of(h), "PostCompact", &body);
    let _ = std::fs::remove_file(h.dir.join("compacting"));
}

/// The one session the test spawned, as the hook frames name it.
fn session_of(h: &Harness) -> String {
    std::fs::read_to_string(h.dir.join("session")).unwrap()
}

fn feed(h: &Harness) -> String {
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap_or_default()
}

fn got(h: &Harness) -> String {
    std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default()
}

/// Board a ticket, start its claude and bring it up at its prompt.
fn seat(h: &Harness, c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    std::fs::write(h.dir.join("session"), sid.to_string()).unwrap();
    wait_composer(h, sid);
    (ticket, sid)
}

fn ask(c: &mut TestClient, ticket: ulid::Ulid, text: &str, queued: bool) -> Response {
    c.request(Command::PromptSession {
        ticket,
        text: text.into(),
        queued,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    })
}

/// The automoves the feed recorded for `ticket` as made, each as its row.
/// The feed is flushed on the daemon's tick, a beat after the snapshot
/// shows the move, so a count is waited for (`moves_reach`) and a "no move"
/// is read after a tick has passed (`settled_moves`).
fn moves(h: &Harness, ticket: ulid::Ulid) -> Vec<String> {
    let id = ticket.to_string();
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.contains("\"kind\":\"movement_decision\"")
                && l.contains("\"outcome\":\"moved\"")
                && l.contains(&id)
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn a_compaction_is_painted_and_moves_nothing() {
    let Some(h) = Harness::boot("compact", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("compact");
    let (ticket, sid) = seat(&h, &mut c, "squeeze");
    let session = sid.to_string();
    let rec = |c: &mut TestClient| -> SessionRecord {
        c.board().sessions.iter().find(|s| s.id == sid).cloned().expect("record")
    };
    let column = |c: &mut TestClient| c.board().ticket(ticket).unwrap().column.clone();
    let done = SessionState::Idle { stop_reason: StopReason::EndTurn };

    // A turn, taken and finished: the template's rules carry the ticket to
    // IN PROGRESS and back to REVIEW, which is where a compaction finds it.
    hook_send_with(
        &hook_sock,
        &session,
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-compact.jsonl","cwd":"/tmp"}"#,
    );
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(5), "the turn moves the ticket to IN PROGRESS", || {
        column(&mut c) == "IN PROGRESS"
    });
    hook_send(&hook_sock, &session, "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(5), "the turn's end moves the ticket to REVIEW", || {
        column(&mut c) == "REVIEW"
    });
    assert_eq!(rec(&mut c).state, done);
    let moves_reach = |n: usize| {
        wait_until(Duration::from_secs(5), "the feed records the moves", || {
            moves(&h, ticket).len() == n
        });
        moves(&h, ticket)
    };
    let settled_moves = || {
        std::thread::sleep(Duration::from_millis(600));
        moves(&h, ticket)
    };
    let moved = moves_reach(2);

    // A person's `/compact` at a finished turn: the window opens on the
    // record with the state and the column untouched — before T-894 the
    // seat went `Running`, the ticket to IN PROGRESS and back, and the
    // crown heard a turn end.
    pre(&h, "manual");
    wait_until(Duration::from_secs(5), "the compaction window opens", || rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, done, "a compaction moves no state");
    assert_eq!(column(&mut c), "REVIEW", "a compaction moves no ticket");
    // Its duplicate and its own SessionStart keep the window open and move
    // nothing either.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"manual"}"#);
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    std::thread::sleep(Duration::from_millis(400));
    let r = rec(&mut c);
    assert!(r.compacting, "still compacting after its SessionStart");
    assert_eq!(r.state, done);
    assert_eq!(column(&mut c), "REVIEW");
    post(&h, "manual");
    wait_until(Duration::from_secs(5), "the compaction window closes", || !rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, done, "the finished turn is where it was");
    assert_eq!(column(&mut c), "REVIEW");
    assert_eq!(settled_moves(), moved, "a manual compaction made no move");

    // A daemon restarted inside a quiet compaction (the `U` chip) comes
    // back with no window and a record it cannot read: the compaction's
    // own `SessionStart` reopens the window, its two helper `SubagentStop`s
    // hold inside it (before the fix they read the restart's Low record as
    // a turn and moved the ticket), and its end closes it. No turn, no move.
    pre(&h, "auto");
    wait_until(Duration::from_secs(5), "the quiet window opens", || rec(&mut c).compacting);
    drop(c);
    h.restart();
    let mut c = h.client("compact-after");
    assert!(!rec(&mut c).compacting, "a restart's machine starts with no window");
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    wait_until(Duration::from_secs(5), "the compaction's start reopens the window", || {
        rec(&mut c).compacting
    });
    hook_send(&hook_sock, &session, "SubagentStop", r#"{"agent_type":""}"#);
    hook_send(&hook_sock, &session, "SubagentStop", r#"{"agent_type":""}"#);
    std::thread::sleep(Duration::from_millis(400));
    let r = rec(&mut c);
    assert_ne!(r.state, SessionState::Running, "a helper's stop is no turn");
    assert!(r.compacting, "still compacting through the helper's stops");
    assert_eq!(column(&mut c), "REVIEW");
    post(&h, "auto");
    wait_until(Duration::from_secs(5), "the window closes after the restart", || {
        !rec(&mut c).compacting
    });
    assert_ne!(rec(&mut c).state, SessionState::Running, "a compaction's end is no turn");
    assert_eq!(column(&mut c), "REVIEW");
    assert_eq!(settled_moves(), moved, "a compaction across a restart made no move");

    // An automatic compaction inside a turn: the window opens over
    // `Running`, which the turn keeps, and the turn ends as any turn does.
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"more"}"#);
    wait_until(Duration::from_secs(5), "the next turn moves the ticket to IN PROGRESS", || {
        column(&mut c) == "IN PROGRESS"
    });
    let moved = moves_reach(3);
    pre(&h, "auto");
    wait_until(Duration::from_secs(5), "the window opens mid-turn", || rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running, "the turn goes on");
    hook_send_with(&hook_sock, &session, "SessionStart", Some("compact"), r#"{"session_id":"x"}"#);
    post(&h, "auto");
    wait_until(Duration::from_secs(5), "the window closes mid-turn", || !rec(&mut c).compacting);
    assert_eq!(rec(&mut c).state, SessionState::Running);
    assert_eq!(column(&mut c), "IN PROGRESS");
    assert_eq!(settled_moves(), moved, "an automatic compaction made no move");
    hook_send(&hook_sock, &session, "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(5), "the turn's end moves the ticket to REVIEW", || {
        column(&mut c) == "REVIEW"
    });
    let moved = moves_reach(4);

    // A person's Esc out of a compaction (measured on 2.1.308): no hook, no
    // transcript line, and the pane's `Compacting conversation…` line gone
    // with the composer back. Before the fix the card kept "compacting
    // context" until the next prompt; the screen closes the window now,
    // and moves nothing.
    pre(&h, "manual");
    wait_until(Duration::from_secs(5), "the window opens", || rec(&mut c).compacting);
    std::thread::sleep(Duration::from_millis(2_500));
    assert!(rec(&mut c).compacting, "the painted line keeps the window open");
    std::fs::remove_file(h.dir.join("compacting")).unwrap();
    wait_until(Duration::from_secs(8), "the screen closes an interrupted window", || {
        !rec(&mut c).compacting
    });
    wait_until(Duration::from_secs(5), "the feed says why", || {
        feed_count(&h, "compaction_ended_on_screen", ticket) == 1
    });
    assert_eq!(rec(&mut c).state, done, "an interrupted compaction moves no state");
    assert_eq!(column(&mut c), "REVIEW");
    assert_eq!(settled_moves(), moved, "an interrupted compaction made no move");

    // A queued ask waits out a compaction (T-894: the crown's `ask_agent`
    // went down mid-compaction and was refused). It goes when the window
    // closes, whole.
    pre(&h, "manual");
    wait_until(Duration::from_secs(5), "the window opens", || rec(&mut c).compacting);
    let words = "mesimon-894 after the squeeze";
    assert!(
        matches!(ask(&mut c, ticket, words, true), Response::Queued { .. }),
        "an ask at a compacting seat queues"
    );
    std::thread::sleep(Duration::from_millis(1_200));
    assert!(!got(&h).contains(words), "nothing goes into a compaction");
    assert_eq!(pending_of(&mut c, Some(ticket)).len(), 1, "the ask waits");
    post(&h, "manual");
    wait_until(Duration::from_secs(8), "the ask goes once the window closes", || {
        got(&h).contains(words)
    });

    // A vetoed compaction fires `PreCompact` alone (nothing to compact):
    // the next prompt closes the window, and the turn it begins is a turn.
    hook_send(&hook_sock, &session, "PreCompact", r#"{"trigger":"manual"}"#);
    wait_until(Duration::from_secs(5), "the window opens on a veto", || rec(&mut c).compacting);
    hook_send(&hook_sock, &session, "UserPromptSubmit", r#"{"prompt":"on"}"#);
    wait_until(Duration::from_secs(5), "a prompt closes the window", || {
        let r = rec(&mut c);
        !r.compacting && r.state == SessionState::Running
    });
    assert_eq!(column(&mut c), "IN PROGRESS");
}

/// A `/compact` sent from the board's ask field (T-894: the author's, on
/// this ticket's own seat) goes by the paste road and fires no
/// `UserPromptSubmit`, so its paste was never acked: the entry held for
/// its eleven seconds and ended `paste_unacked`. It is acked as a `!` line
/// is, off the screen: the box reads empty after the Enter. Never kept
/// unsent, and never a mod submit.
#[test]
fn a_slash_command_from_the_board_is_acked_off_the_screen() {
    let Some(h) = Harness::boot("compact-cmd", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("compact-cmd");
    let (ticket, sid) = seat(&h, &mut c, "squeeze by hand");
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), "{}");
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    if test_road() == "mod" {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(Duration::from_secs(10), "the mod's bridge", || pid.exists());
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(matches!(ask(&mut c, ticket, "/compact", false), Response::Ok));
    // Its `PreCompact` opens the window, and a follow-up is queued behind
    // it before the screen acks the command (T-894 review: the ack's queue
    // sweep deleted the follow-up and told its asker the person dropped it).
    pre(&h, "manual");
    let words = "mesimon-894 after the command";
    assert!(matches!(ask(&mut c, ticket, words, true), Response::Queued { .. }));
    wait_until(Duration::from_secs(8), "the command typed and entered", || {
        // The stub's line holds the plain start's prefilled title too: a
        // stub draws no box, so the composer the daemon reads stays empty.
        got(&h).lines().any(|l| l.trim_end().ends_with("/compact"))
    });
    wait_until(Duration::from_secs(5), "the command's line in the feed", || {
        feed_count(&h, "command_sent", ticket) == 1
    });
    wait_until(Duration::from_secs(5), "acked off the screen", || {
        feed_count(&h, "prompt_submitted", ticket) == 1
    });
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).expect("record");
    assert!(rec.unsent.is_none(), "{:?}", rec.unsent);
    assert!(!rec.pending_submit);
    assert_eq!(feed_count(&h, "paste_unacked", ticket), 0);
    // The ack took the command and nothing else: the follow-up waits out
    // the window, and goes when it closes.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(feed_count(&h, "queued_ask_dropped_by_hand", ticket), 0, "{}", feed(&h));
    assert_eq!(pending_of(&mut c, Some(ticket)).len(), 1, "the follow-up waits");
    assert!(!got(&h).contains(words));
    post(&h, "manual");
    wait_until(Duration::from_secs(8), "the follow-up goes once the window closes", || {
        got(&h).contains(words)
    });
}
