//! A raised hand's reason outlives its telling (T-922). The board wakes the
//! crown for a hand a worker it started raises, with a sentence that names
//! the hand and never its words (T-414), and lowers the hand once the
//! sentence is in (T-861). The crown that sentence sends to `get_ticket`
//! reads the reason there as `told_hand`: through a restart, replaced by the
//! worker's next hand, gone at the worker's next turn, and only for the
//! crown — another agent's keyed read is refused, the worker's own read
//! carries none, and a hand a person lowered before the telling keeps
//! nothing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::SessionState;
use mesimon_core::command::{AgentTicketView, Command, PendingAction, Response};
use mesimon_core::Principal;

mod common;
use common::*;

/// Each line the stub reads is appended to one file beside it, so the
/// crown's sentences and the workers' words are all there.
const RECORDING_STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

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

fn told(c: &mut TestClient, crown: uuid::Uuid, key: &str) -> Option<String> {
    read(c, crown, key).unwrap().told_hand.map(|h| h.reason)
}

fn raise(c: &mut TestClient, session: uuid::Uuid, reason: &str) {
    match c.send(Principal::Agent { session }, Command::AgentRaiseHand { reason: reason.into() }) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
}

#[test]
fn the_crown_reads_the_reason_of_a_hand_it_was_told_of() {
    let Some(h) = Harness::boot_with_env(
        "told_hand",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("told_hand");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> usize {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .count()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let wakes = |c: &mut TestClient, t: ulid::Ulid| -> usize {
        pending_of(c, Some(t)).into_iter().filter(|p| p.action == PendingAction::CrownWake).count()
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-81 worker");
    let b = c.create_ticket("mesimon-probe-82 a person's");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("the worker holds a seat").id;
    // Another agent on the board, a person's: not the crown.
    let sb = c.spawn_claude(b);
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The worker raises its hand mid-turn and goes on working, as on T-917.
    start(&mut c, ws);
    // The clause, not the whole sentence: a restart's look may fold a
    // word of its own into a later one.
    let raised = format!("{kw} \"mesimon-probe-81 worker\" raised its hand");

    // ---- 1. told: the hand is down, the reason is the crown's to read ----
    let first = "mesimon-secret-81 which sandbox key, staging or prod?";
    raise(&mut c, ws, first);
    wait_until(std::time::Duration::from_secs(10), "the hand's wake to land", || {
        lines_with(&raised) == 1
    });
    assert_eq!(lines_with("mesimon-secret-81"), 0, "the reason never reaches the crown's pane");
    wait_until(std::time::Duration::from_secs(5), "the told hand to come down", || {
        !c.board().ticket(w).unwrap().hand_raised()
    });
    let view = read(&mut c, sa, &kw).unwrap();
    assert_eq!(view.told_hand.as_ref().map(|h| h.reason.as_str()), Some(first), "{view:?}");
    assert!(view.told_hand.as_ref().unwrap().told_secs.is_some());
    let state = view.state.expect("the worker's state");
    assert_eq!(state.state, "working", "a hand is raised mid-turn");
    assert_eq!(state.raised, None, "no hand is up: the told reason is not a hand");
    assert!(c.board().raised_tickets().is_empty(), "nothing counts a told hand");

    // ---- 2. only the crown reads it ---------------------------------------
    let refused = read(&mut c, sb, &kw).expect_err("another agent's keyed read is refused");
    assert!(!refused.contains("mesimon-secret-81"), "{refused}");
    match c.send(Principal::Agent { session: ws }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.told_hand, None, "the worker's own read carries none")
        }
        other => panic!("the worker's own read: {other:?}"),
    }
    let ka = key_of(&mut c, a);
    let own = read(&mut c, sa, &ka).unwrap();
    assert_eq!(own.told_hand, None, "the crown's own ticket keeps none");

    // ---- 3. a restart between the wake and the read keeps it --------------
    drop(c);
    h.restart();
    assert!(h.paths.state_dir.join("crown.json").is_file(), "the ledger was kept");
    let mut c = h.client("told_hand_2");
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(first), "read back from crown.json");
    assert!(!c.board().ticket(w).unwrap().hand_raised(), "and the hand stays down");
    // The restart left both agents' state to their next hooks.
    start(&mut c, sa);
    stop(&mut c, sa);
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(first), "the crown's own turn keeps it");

    // ---- 4. a new hand overtakes it, and its telling replaces it ----------
    start(&mut c, sa);
    let second = "mesimon-secret-82 prod, then: may I rotate the key?";
    raise(&mut c, ws, second);
    let view = read(&mut c, sa, &kw).unwrap();
    assert_eq!(view.state.and_then(|s| s.raised).as_deref(), Some(second), "the hand is up");
    assert_eq!(view.told_hand, None, "the told reason gave way to the new hand");
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the second hand's wake to land", || {
        lines_with(&raised) == 2
    });
    wait_until(std::time::Duration::from_secs(5), "the second told hand", || {
        told(&mut c, sa, &kw).as_deref() == Some(second)
    });
    assert!(!c.board().ticket(w).unwrap().hand_raised());

    // ---- 5. the worker's next turn takes it: it was given something -------
    stop(&mut c, ws);
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(second), "a turn's end is not an answer");
    start(&mut c, ws);
    wait_until(std::time::Duration::from_secs(5), "the next turn to take the reason", || {
        told(&mut c, sa, &kw).is_none()
    });

    // ---- 6. a hand a person lowered before the telling keeps nothing ------
    start(&mut c, sa);
    raise(&mut c, ws, "mesimon-secret-83 one more");
    assert_eq!(wakes(&mut c, a), 1, "the hand's wake is owed to the working crown");
    assert!(matches!(c.request(Command::LowerHand { id: w }), Response::Ok));
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the owed wake to go", || wakes(&mut c, a) == 0);
    assert_eq!(told(&mut c, sa, &kw), None, "the person dealt with it");

    // ---- 7. a new crown has heard nothing ---------------------------------
    start(&mut c, sa);
    stop(&mut c, sa);
    raise(&mut c, ws, "mesimon-secret-84 last one");
    wait_until(std::time::Duration::from_secs(10), "the fourth hand's telling", || {
        told(&mut c, sa, &kw).is_some()
    });
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(told(&mut c, sa, &kw), None, "the reason went with the crown it was told");
}
