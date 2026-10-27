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
use mesimon_core::command::{Command, PendingAction, Response};
use mesimon_core::Principal;

mod common;
use common::*;

fn told(c: &mut TestClient, crown: uuid::Uuid, key: &str) -> Option<String> {
    c.agent_read(crown, key).unwrap().told_hand.map(|h| h.reason)
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
    let mut c = h.client("told_hand");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> usize {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .count()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| h.prompt_turn(c, sid);
    let stop = |c: &mut TestClient, sid: uuid::Uuid| h.end_turn(c, sid);
    let wakes = |c: &mut TestClient, t: ulid::Ulid| -> usize {
        pending_of(c, Some(t)).into_iter().filter(|p| p.action == PendingAction::CrownWake).count()
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-81 worker");
    let b = c.create_ticket("mesimon-probe-82 a person's");
    let kw = c.key_of(w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.one_turn(&mut c, sa);
    match c.crown_start(sa, &kw, "shared_checkout") {
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
    let view = c.agent_read(sa, &kw).unwrap();
    assert_eq!(view.told_hand.as_ref().map(|h| h.reason.as_str()), Some(first), "{view:?}");
    assert!(view.told_hand.as_ref().unwrap().told_secs.is_some());
    let state = view.state.expect("the worker's state");
    assert_eq!(state.state, "working", "a hand is raised mid-turn");
    assert_eq!(state.raised, None, "no hand is up: the told reason is not a hand");
    assert!(c.board().raised_tickets().is_empty(), "nothing counts a told hand");

    // ---- 2. only the crown reads it ---------------------------------------
    let refused = c.agent_read(sb, &kw).expect_err("another agent's keyed read is refused");
    assert!(!refused.contains("mesimon-secret-81"), "{refused}");
    match c.send(Principal::Agent { session: ws }, Command::AgentGetTicket { full: false }) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.told_hand, None, "the worker's own read carries none")
        }
        other => panic!("the worker's own read: {other:?}"),
    }
    let ka = c.key_of(a);
    let own = c.agent_read(sa, &ka).unwrap();
    assert_eq!(own.told_hand, None, "the crown's own ticket keeps none");

    // ---- 3. a restart between the wake and the read keeps it --------------
    drop(c);
    h.restart();
    assert!(h.paths.state_dir.join("crown.json").is_file(), "the ledger was kept");
    let mut c = h.client("told_hand_2");
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(first), "read back from crown.json");
    assert!(!c.board().ticket(w).unwrap().hand_raised(), "and the hand stays down");
    // The restart left both agents' state to their next hooks.
    h.one_turn(&mut c, sa);
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(first), "the crown's own turn keeps it");

    // ---- 4. a new hand overtakes it, and its telling replaces it ----------
    start(&mut c, sa);
    let second = "mesimon-secret-82 prod, then: may I rotate the key?";
    raise(&mut c, ws, second);
    let view = c.agent_read(sa, &kw).unwrap();
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
    h.one_turn(&mut c, sa);
    raise(&mut c, ws, "mesimon-secret-84 last one");
    wait_until(std::time::Duration::from_secs(10), "the fourth hand's telling", || {
        told(&mut c, sa, &kw).is_some()
    });
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(told(&mut c, sa, &kw), None, "the reason went with the crown it was told");
}

/// What `crown.json` keeps of `worker`'s told hand (T-922), as written.
fn kept(h: &Harness, worker: ulid::Ulid) -> Option<String> {
    let text = std::fs::read_to_string(h.paths.state_dir.join("crown.json")).ok()?;
    let ledger: serde_json::Value = serde_json::from_str(&text).ok()?;
    ledger["hands"][worker.to_string()]["reason"].as_str().map(str::to_string)
}

/// The stamp a keyed edit is judged by covers the told hand (T-922): two
/// hands told inside one turn of the worker leave its column, notes, state
/// and its hand-that-is-up as they were, so a stamp that left the told hand
/// out let an answer read against the first pass for the second. And the
/// told hand goes with its ticket: an archive and a delete forget it.
#[test]
fn an_answer_read_against_one_told_hand_is_refused_for_the_next() {
    let Some(h) = Harness::boot_with_env(
        "told_stamp",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let mut c = h.client("told_stamp");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> usize {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .count()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| h.prompt_turn(c, sid);
    let stop = |c: &mut TestClient, sid: uuid::Uuid| h.end_turn(c, sid);
    let start_agent = |c: &mut TestClient, crown: uuid::Uuid, key: &str, workspace: &str| match c
        .crown_start(crown, key, workspace)
    {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent {key}: {other:?}"),
    };
    let ask = |c: &mut TestClient, crown: uuid::Uuid, key: &str, seen: Option<String>| {
        c.send(
            Principal::Agent { session: crown },
            Command::AgentAskTicket {
                key: key.into(),
                text: "mesimon-probe-86 staging".into(),
                seen,
                plan: false,
                deliver: mesimon_core::command::Deliver::Idle,
                expect_secs: None,
            },
        )
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-85 worker");
    let w2 = c.create_ticket("mesimon-probe-87 worker");
    let (kw, kw2) = (c.key_of(w), c.key_of(w2));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.one_turn(&mut c, sa);
    start_agent(&mut c, sa, &kw, "shared_checkout");
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    start_agent(&mut c, sa, &kw2, "worktree");
    wait_until(std::time::Duration::from_secs(15), "W2's worktree start to land", || {
        c.board().live_agent(w2).is_some()
    });
    let ws2 = c.board().live_agent(w2).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let told_of = |c: &mut TestClient, n: usize| {
        wait_until(std::time::Duration::from_secs(10), "the hand's wake to land", || {
            lines_with(&format!("{kw} \"mesimon-probe-85 worker\" raised its hand")) == n
        });
        wait_until(std::time::Duration::from_secs(5), "the told hand to come down", || {
            !c.board().ticket(w).unwrap().hand_raised()
        });
        // The stub crown fires no hook of its own: its turn on the wake is
        // walked here, so the next wake finds it idle.
        start(c, sa);
        stop(c, sa);
    };

    // ---- 1. two hands told inside one turn of the worker ------------------
    start(&mut c, ws);
    let first = "mesimon-secret-85 staging or prod?";
    raise(&mut c, ws, first);
    told_of(&mut c, 1);
    let read_a = c.agent_read(sa, &kw).unwrap();
    assert_eq!(read_a.told_hand.as_ref().map(|t| t.reason.as_str()), Some(first));
    let second = "mesimon-secret-86 and which region?";
    raise(&mut c, ws, second);
    told_of(&mut c, 2);
    assert_eq!(told(&mut c, sa, &kw).as_deref(), Some(second));
    assert_eq!(
        c.board().live_agent(w).unwrap().state,
        SessionState::Running,
        "the same turn: nothing else on the ticket moved"
    );

    // ---- 2. an answer to the first is refused; a fresh read goes ----------
    match ask(&mut c, sa, &kw, read_a.seen.clone()) {
        Response::Err { message } => {
            assert!(message.contains("changed since it was read"), "{message}")
        }
        other => panic!("an ask on the first hand's stamp: {other:?}"),
    }
    let read_b = c.agent_read(sa, &kw).unwrap();
    assert_ne!(read_b.seen, read_a.seen, "the second told hand is another stamp");
    match ask(&mut c, sa, &kw, read_b.seen.clone()) {
        Response::AgentAsked { .. } => {}
        other => panic!("an ask on a fresh read: {other:?}"),
    }
    // The stamp holds still while nothing changes: elapsed time is not in it.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(c.agent_read(sa, &kw).unwrap().seen, read_b.seen);

    // ---- 3. the same words told again are another hand --------------------
    raise(&mut c, ws, second);
    told_of(&mut c, 3);
    let again = c.agent_read(sa, &kw).unwrap();
    assert_eq!(again.told_hand.map(|t| t.reason).as_deref(), Some(second));
    assert_ne!(again.seen, read_b.seen, "a new telling of the same words is a new stamp");

    // ---- 4. an archive forgets it; nothing comes back with the ticket -----
    start(&mut c, ws2);
    raise(&mut c, ws2, "mesimon-secret-87 which branch?");
    wait_until(std::time::Duration::from_secs(10), "W2's told hand", || {
        told(&mut c, sa, &kw2).is_some()
    });
    stop(&mut c, ws2);
    assert!(matches!(c.request(Command::SleepSession { id: ws2 }), Response::Ok));
    assert_eq!(
        told(&mut c, sa, &kw2).as_deref(),
        Some("mesimon-secret-87 which branch?"),
        "a park gave the agent nothing: the crown still holds the question"
    );
    wait_until(std::time::Duration::from_secs(10), "the ledger to keep W2's hand", || {
        kept(&h, w2).is_some()
    });
    assert!(matches!(c.request(Command::ArchiveTicket { id: w2 }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the archive to forget W2's hand", || {
        kept(&h, w2).is_none()
    });
    assert!(matches!(c.request(Command::UnarchiveTicket { id: w2 }), Response::Ok));
    assert_eq!(told(&mut c, sa, &kw2), None, "an archive's return brings no told hand");

    // ---- 5. a delete forgets it -------------------------------------------
    wait_until(std::time::Duration::from_secs(10), "the ledger to keep W's hand", || {
        kept(&h, w).as_deref() == Some(second)
    });
    assert!(matches!(
        c.request(Command::DeleteTicket { id: w, discard_worktree: false }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(10), "the delete to forget W's hand", || {
        kept(&h, w).is_none()
    });
}

/// A turn begun while no daemon listened answered the told hand too (T-922):
/// its `UserPromptSubmit` is lost, and the recovered turn comes back at Low,
/// which takes no hand down. The restart reads the worker's transcript for
/// a prompt dated after the telling. A restart over the hand's own turn
/// going on (a tool's result, a reply) keeps the reason; one over the next
/// prompt forgets it.
#[test]
fn a_prompt_while_the_daemon_was_down_takes_the_told_hand() {
    let Some(h) = Harness::boot_with_env(
        "told_gap",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("told_gap");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> usize {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .count()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| h.prompt_turn(c, sid);

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-88 worker");
    let kw = c.key_of(w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.one_turn(&mut c, sa);
    match c.crown_start(sa, &kw, "shared_checkout") {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("the worker holds a seat").id;
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let line = |kind: &str, at: u64, content: &str| {
        format!(
            r#"{{"uuid":"u{at}{kind}","type":"user","timestamp":"{}","message":{{"content":{content}}}}}"#,
            iso_ms(at)
        ) + "\n"
    };
    std::fs::write(&transcript, line("p", now_ms(), r#""the brief""#)).unwrap();
    let begun = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &begun);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    start(&mut c, ws);

    // ---- 1. told, then a restart over the same turn going on ---------------
    let reason = "mesimon-secret-88 which sandbox key?";
    raise(&mut c, ws, reason);
    wait_until(std::time::Duration::from_secs(10), "the hand's wake to land", || {
        lines_with(&format!("{kw} \"mesimon-probe-88 worker\" raised its hand")) == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the told hand", || {
        told(&mut c, sa, &kw).as_deref() == Some(reason)
    });
    drop(c);
    let append = |text: String| {
        let mut f = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
        std::io::Write::write_all(&mut f, text.as_bytes()).unwrap();
    };
    h.restart_after(|| append(line("r", now_ms(), r#"[{"type":"tool_result"}]"#)));
    let mut c = h.client("told_gap_2");
    assert_eq!(
        told(&mut c, sa, &kw).as_deref(),
        Some(reason),
        "a tool's result is the hand's own turn going on"
    );

    // ---- 2. a restart over the next prompt forgets it ----------------------
    drop(c);
    h.restart_after(|| append(line("p", now_ms(), r#""staging""#)));
    let mut c = h.client("told_gap_3");
    assert_eq!(told(&mut c, sa, &kw), None, "the prompt in the gap answered it");
    wait_until(std::time::Duration::from_secs(10), "the ledger to forget it", || {
        kept(&h, w).is_none()
    });
}
