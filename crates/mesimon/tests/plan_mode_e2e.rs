//! Plan mode from the composer and the ask field (T-434): the `plan` flag
//! on `SpawnSession` and `PromptSession` puts `--permission-mode plan` on
//! the ONE launch the words end in — a start, a wake, or a live IDLE pane
//! parked and woken with it — whatever the column says, and the next wake
//! reads the column again. A working pane refuses the send-now form; a
//! queued one waits for idle and relaunches then. The crown's `start_agent`
//! and `ask_agent` carry the same flag.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";

#[test]
fn plan_mode_rides_the_start_the_wake_and_an_idle_panes_relaunch() {
    let Some(h) = Harness::boot_with_env(
        "planmode",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let mut c = h.client("planmode");
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let mode_of = |c: &mut TestClient, sid| -> Option<String> {
        let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
        let at = rec.argv.iter().position(|a| a == "--permission-mode")?;
        rec.argv.get(at + 1).cloned()
    };
    // The stub speaks no hook; the test speaks them in Claude Code's own
    // order — `SessionStart` first, which is where a spawn's owed Enter is
    // delivered, then the prompt's ack, then the stop.
    let idle = |c: &mut TestClient, sid: uuid::Uuid| {
        let session = sid.to_string();
        hook_send(&hooks, &session, "SessionStart", r#"{"source":"startup"}"#);
        hook_send(&hooks, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
        hook_send(&hooks, &session, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    // ---- the composer's ^p: a start in plan mode, column at inherit -------
    let t1 = create(&mut c, "composed");
    let s1 = match c.request(Command::SpawnSession {
        ticket: t1,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: true,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert_eq!(mode_of(&mut c, s1).as_deref(), Some("plan"), "the composer's start");
    let pane_before = c.board().sessions.iter().find(|s| s.id == s1).unwrap().pane_key.clone();

    // ---- an idle pane: the send-now ask parks and wakes it with the flag ----
    // First a turn, so the record is Idle (the stub answers no hook of its
    // own; the test speaks them).
    idle(&mut c, s1);
    // Mid-turn, the send-now form is refused and nothing moves.
    hook_send(&hooks, &s1.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(s1, "running", |s| *s == SessionState::Running);
    match c.request(Command::PromptSession {
        ticket: t1,
        text: "mesimon-probe-434 now".into(),
        queued: false,
        accept_plan: false,
        plan: true,
    }) {
        Response::Err { message } => assert!(message.contains("mid-turn"), "{message}"),
        other => panic!("a working pane took a plan ask: {other:?}"),
    }
    assert_eq!(
        c.board().sessions.iter().find(|s| s.id == s1).unwrap().pane_key,
        pane_before,
        "refused means untouched"
    );
    // Queued, it waits for idle and then relaunches: a new pane, same
    // record, plan in the argv, the words parked for the first tick.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: t1,
            text: "mesimon-probe-434 queued".into(),
            queued: true,
            accept_plan: false,
            plan: true,
        }),
        Response::Queued { .. }
    ));
    let p = pending_of(&mut c, Some(t1));
    assert!(p.iter().any(|p| p.plan && !p.in_flight), "the row carries the flag: {p:?}");
    hook_send(&hooks, &s1.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(10), "the relaunch", || {
        let rec = c.board().sessions.into_iter().find(|s| s.id == s1).unwrap();
        rec.pane_key != pane_before && rec.state == SessionState::Spawning
    });
    let rec = c.board().sessions.into_iter().find(|s| s.id == s1).unwrap();
    assert_eq!(mode_of(&mut c, s1).as_deref(), Some("plan"), "{:?}", rec.argv);
    assert!(rec.pending_submit, "the words are parked for the pane");
    assert_eq!(
        c.board().sessions.iter().filter(|s| s.ticket == t1).count(),
        1,
        "one seat: the relaunch re-enters the record"
    );
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"plan_relaunch""#)
    });
    assert!(pending_of(&mut c, Some(t1)).is_empty(), "the entry left the queue with the wake");

    // ---- a parked claude: the wake carries the flag, the next does not ------
    let t2 = create(&mut c, "parked");
    let s2 = match c.request(Command::SpawnSession {
        ticket: t2,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    assert_ne!(mode_of(&mut c, s2).as_deref(), Some("plan"));
    idle(&mut c, s2);
    assert!(matches!(c.request(Command::SleepSession { id: s2 }), Response::Ok));
    c.await_state(s2, "sleeping", |s| *s == SessionState::Sleeping);
    match c.request(Command::PromptSession {
        ticket: t2,
        text: "mesimon-probe-434 wake".into(),
        queued: false,
        accept_plan: false,
        plan: true,
    }) {
        Response::Spawned { id, .. } => assert_eq!(id, s2),
        other => panic!("wake: {other:?}"),
    }
    assert_eq!(mode_of(&mut c, s2).as_deref(), Some("plan"), "the wake took the flag");
    assert_eq!(
        c.board()
            .sessions
            .iter()
            .find(|s| s.id == s2)
            .unwrap()
            .argv
            .iter()
            .filter(|a| *a == "--permission-mode")
            .count(),
        1,
        "one pair, never stacked"
    );
    // A later wake without the flag reads the column again.
    idle(&mut c, s2);
    assert!(matches!(c.request(Command::SleepSession { id: s2 }), Response::Ok));
    c.await_state(s2, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(matches!(c.request(Command::WakeSession { id: s2 }), Response::Spawned { .. }));
    assert_ne!(mode_of(&mut c, s2).as_deref(), Some("plan"), "the flag is per launch");

    // ---- a queued start survives the file with its flag ---------------------
    // A start on an empty shared seat while t1's claude works: it queues,
    // and queue.json carries `plan` (schema 2) for a restart to honour.
    hook_send(&hooks, &s1.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    hook_send(&hooks, &s1.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(s1, "running again", |s| *s == SessionState::Running);
    let t3 = create(&mut c, "queued start");
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: t3,
            text: String::new(),
            queued: true,
            accept_plan: false,
            plan: true,
        }),
        Response::Queued { .. }
    ));
    let queue = std::fs::read_to_string(h.paths.queue_file()).unwrap_or_default();
    assert!(queue.contains("\"plan\": true"), "{queue}");
    assert!(queue.contains("\"schema_version\": 2"), "{queue}");
}
