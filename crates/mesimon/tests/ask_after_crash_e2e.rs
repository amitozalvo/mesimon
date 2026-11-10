//! Shift+Enter on a ticket whose claude crashed resumes THAT conversation
//! (T-1119). The corpse holds no seat, so the ask used to read the ticket as
//! empty and start a second claude beside it — silently, with the crashed
//! conversation left on the rail unreachable from the card. A crash is an
//! exit the person did not choose; the conversation it leaves is the one
//! they expect to go on, exactly as a parked one does. Real tmux, real
//! `pane-died`, in-process daemon, a stub that dies with status 1.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use mesimon_core::board::{ExitReason, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

#[test]
fn asking_a_crashed_claude_resumes_its_conversation_and_starts_nothing() {
    // A claude that dies a second in, nonzero: the pane-died road files it
    // as a crash, which the park-on-exit gate deliberately leaves a corpse.
    let Some(h) = Harness::boot_bare("askcrash", Some("#!/bin/sh\nsleep 1\nexit 1\n"), &[]) else {
        return;
    };
    let mut c = h.client("askcrash");
    // Claude's own store is what `--resume` reads: a transcript under the
    // census root makes the record resumable rather than fresh.
    let projects = h.dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&projects).unwrap();

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "crashed".into(),
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
    std::fs::write(
        projects.join(format!("{sid}.jsonl")),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .unwrap();

    let dead = c.await_state(sid, "exited", |s| matches!(s, SessionState::Exited { .. }));
    assert_eq!(dead, SessionState::Exited { reason: ExitReason::Crashed }, "a crash is a corpse");

    // The board's Shift+Enter with words: the receipt is the SAME record,
    // resumed — not a fresh conversation, not a second session.
    match c.request(Command::PromptSession {
        ticket,
        text: "carry on".into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    }) {
        Response::Spawned { id, fresh } => {
            assert_eq!(id, sid, "the crashed record is the one resumed");
            assert!(!fresh, "its conversation was there to resume");
        }
        other => panic!("the ask did not resume: {other:?}"),
    }
    let board = c.board();
    let agents: Vec<_> =
        board.sessions.iter().filter(|s| s.ticket == ticket && s.kind.is_agent()).collect();
    assert_eq!(agents.len(), 1, "no second claude beside the corpse: {agents:?}");
    assert!(agents[0].state.is_live(), "the record is live again: {:?}", agents[0].state);
}

/// A claude that stays up until `crash-<its ticket's key>` sits beside the
/// stub, then spends the file and dies nonzero: a crash a test places after
/// the agent's turn, as a reboot ends an idle worker. A probe (`--version`
/// and the like, no conversation flag) answers nothing and leaves.
const CRASH_ON_FILE_STUB: &str =
    "#!/bin/sh\ncase \" $* \" in *\" --session-id \"*|*\" --resume \"*) ;; \
     *) exit 0 ;; esac\nf=\"$(dirname \"$0\")/crash-${MESIMON_TICKET:-none}\"\n\
     stty -icanon 2>/dev/null\nwhile [ ! -f \"$f\" ]; do sleep 0.2; done\nrm -f \"$f\"\nexit 1\n";

/// `ticket`'s agent through one turn to idle, then crashed: the record a
/// reboot leaves, `Exited{Crashed}` with nothing owed it.
fn crash_idle(h: &Harness, c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    wait_until(std::time::Duration::from_secs(20), "the start to land", || {
        c.board().live_agent(ticket).is_some()
    });
    let sid = c.board().live_agent(ticket).unwrap().id;
    h.one_turn(c, sid);
    let key = c.key_of(ticket);
    std::fs::write(h.dir.join(format!("crash-{key}")), "").unwrap();
    let dead = c.await_state(sid, "exited", |s| matches!(s, SessionState::Exited { .. }));
    assert_eq!(dead, SessionState::Exited { reason: ExitReason::Crashed }, "a crash is a corpse");
    sid
}

fn agents_of(c: &mut TestClient, ticket: ulid::Ulid) -> Vec<mesimon_core::board::SessionRecord> {
    c.board().sessions.iter().filter(|s| s.ticket == ticket && s.kind.is_agent()).cloned().collect()
}

/// T-1190. A reboot filed two of simbly's crown-started workers crashed; the
/// crown read no state on their tickets, called `start_agent` on each, and
/// each got a SECOND claude on the brief beside the conversation it had, the
/// one a person's Enter on the row would have resumed. The crown's start on
/// such a ticket is now that resume, its ask is sent and not held as "a
/// person's" (nor dropped as a seat that went), the ticket reads `exited`,
/// a person's crashed agent is refused in words, and a corpse with no
/// conversation is still a fresh start.
#[test]
fn the_crown_s_start_resumes_the_crashed_worker_it_started() {
    let Some(h) = Harness::boot_with_env("crownresume", Some(CRASH_ON_FILE_STUB), CROWN_ENV) else {
        return;
    };
    let mut c = h.client("crownresume");
    init_repo(&h.repo, "a.txt", "hello\n");
    let projects = h.dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&projects).unwrap();
    let transcript = |sid: uuid::Uuid| {
        std::fs::write(
            projects.join(format!("{sid}.jsonl")),
            "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
        )
        .unwrap();
    };
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 4 }), Response::Ok));

    let a = c.create_ticket("coordinate");
    let sa = h.crown_idle(&mut c, a);
    let (w, asked, person, bare) = (
        c.create_ticket("a worker that crashes"),
        c.create_ticket("a worker asked after its crash"),
        c.create_ticket("a person's agent that crashes"),
        c.create_ticket("a worker that never wrote a conversation"),
    );
    let (kw, kasked, kperson, kbare) =
        (c.key_of(w), c.key_of(asked), c.key_of(person), c.key_of(bare));

    // ---- start_agent on the crown's own crashed worker: that record, resumed
    assert!(matches!(c.crown_start(sa, &kw, "worktree"), Response::AgentStarted { .. }));
    let sw = crash_idle(&h, &mut c, w);
    transcript(sw);
    // The ticket says so: `exited` and how, where it said nothing at all.
    let state = c.agent_read(sa, &kw).unwrap().state.expect("an exited agent is read");
    assert_eq!((state.state.as_str(), state.exited.as_deref()), ("exited", Some("crashed")));
    match c.send(mesimon_core::Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => {
            let row = board.tickets.iter().find(|t| t.key == kw).expect("the row");
            assert_eq!(row.state.as_deref(), Some("exited"), "list_board says it too");
        }
        other => panic!("list_board: {other:?}"),
    }
    match c.crown_start(sa, &kw, "worktree") {
        Response::AgentStarted { woken, session_started, .. } => {
            assert!(woken, "the crashed worker is woken, not replaced");
            assert!(session_started);
        }
        other => panic!("start_agent on a crashed worker: {other:?}"),
    }
    let agents = agents_of(&mut c, w);
    assert_eq!(agents.len(), 1, "no second claude beside the corpse: {agents:?}");
    assert_eq!(agents[0].id, sw, "the crashed record is the one resumed");
    assert!(agents[0].state.is_live(), "{:?}", agents[0].state);
    assert!(agents[0].argv.iter().any(|a| a == "--resume"), "{:?}", agents[0].argv);
    assert!(agents[0].started_by.is_some(), "and still the crown's");

    // ---- ask_agent on one: sent, where it was held as "a person's" --------
    assert!(matches!(c.crown_start(sa, &kasked, "worktree"), Response::AgentStarted { .. }));
    let sasked = crash_idle(&h, &mut c, asked);
    transcript(sasked);
    match c.crown_ask(sa, &kasked, "carry on", mesimon_core::command::Deliver::Idle, None) {
        Response::AgentAsked { held_for_person, held_because, .. } => {
            assert!(!held_for_person, "the crown's own worker: {held_because:?}");
        }
        other => panic!("ask_agent on a crashed worker: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(20), "the ask to resume it", || {
        c.board().live_agent(asked).is_some_and(|s| s.id == sasked)
    });
    assert_eq!(agents_of(&mut c, asked).len(), 1);

    // ---- a person's crashed agent stays the person's ----------------------
    let sp = c.spawn_claude(person);
    assert_eq!(crash_idle(&h, &mut c, person), sp);
    transcript(sp);
    match c.crown_start(sa, &kperson, "shared_checkout") {
        Response::Err { message } => {
            assert!(message.contains("was started by a person and exited"), "{message}");
            assert!(message.contains("a person resumes it"), "{message}");
        }
        other => panic!("start_agent on a person's crashed agent: {other:?}"),
    }
    assert_eq!(agents_of(&mut c, person).len(), 1, "a refused start starts nothing");

    // ---- no conversation to keep: the fresh start it always was -----------
    assert!(matches!(c.crown_start(sa, &kbare, "worktree"), Response::AgentStarted { .. }));
    let sbare = crash_idle(&h, &mut c, bare);
    match c.crown_start(sa, &kbare, "worktree") {
        Response::AgentStarted { woken, .. } => assert!(!woken, "nothing to resume"),
        other => panic!("start_agent on a corpse with no conversation: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(20), "the fresh agent", || {
        c.board().live_agent(bare).is_some_and(|s| s.id != sbare)
    });
}
