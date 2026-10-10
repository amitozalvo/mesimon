//! The board's ask can WAIT for a quiet checkout (2026-09-04), end to end:
//! two claudes in the shared checkout, one mid-turn; an ask queued at the
//! other is parked, lands the moment the first one's turn settles, and is
//! dropped when the user talks to the agent ahead of it. The second test
//! queues two and sorts them by moving the cards (T-263): the queue is in
//! board order, top first, never first-come.
//!
//! The stub agent appends every line it reads to one file beside itself —
//! both panes share it, and every probe is unique, so what is asserted is
//! which words landed and when, never which pane. `read` only returns on a
//! newline, so a line proves delivery AND the separate Enter (T-5).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, PendingAction, Response};

#[test]
fn a_queued_ask_waits_for_the_checkout_and_is_dropped_when_the_user_talks_first() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    // The quiet probe must not free the checkout under the test, and the
    // sleep floor must not refuse the sleep case.
    let Some(h) = Harness::boot_with_env(
        "askq",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askq");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "holder".into(),
        workspace: None,
        tier: None,
    });
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "waiter".into(),
        workspace: None,
        tier: None,
    });
    let board = c.board();
    let a = board.tickets.iter().find(|t| t.title == "holder").expect("a").id;
    let b = board.tickets.iter().find(|t| t.title == "waiter").expect("b").id;
    let a_key = board.ticket(a).unwrap().short_key.clone();

    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sa = spawn(&mut c, a);
    let sb = spawn(&mut c, b);
    // Both stubs reading before anything is pasted.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let panes = tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        if panes >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "the stub agents never got their panes");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));

    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    // A stub emits no `SessionStart`, so both records sit at `Spawning` —
    // which is WORKING — until the hooks say otherwise. Walk each through a
    // turn first so the checkout starts quiet.
    for sid in [sa, sb] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }

    // (1) A holds the checkout; an ask at B waits, then lands on A's settle.
    start(&mut c, sa);
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-51 commit what you have".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { behind, .. } => assert_eq!(behind, vec![a_key.clone()]),
        other => panic!("expected the ask to be parked: {other:?}"),
    }
    let p = pending_of(&mut c, None);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].ticket, b);
    assert_eq!(p[0].action, PendingAction::Ask);
    assert_eq!(p[0].waits_on, vec![a_key.clone()]);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-51 commit what you have"));
    assert!(!p[0].in_flight);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!text().contains("mesimon-probe-51"), "parked words must not land: {:?}", text());
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "the queued ask to land", || {
        text().contains("mesimon-probe-51 commit what you have")
    });
    assert_eq!(text().lines().filter(|l| l.contains("mesimon-probe-51")).count(), 1);
    // In flight until B's agent acks it; the ack clears the entry.
    let p = pending_of(&mut c, None);
    assert!(p.iter().any(|p| p.ticket == b && p.in_flight), "{p:?}");
    start(&mut c, sb);
    wait_until(Duration::from_secs(5), "the ack to clear the entry", || {
        pending_of(&mut c, None).is_empty()
    });
    stop(&mut c, sb);

    // (2) The user talks to B ahead of the ask: the ask is dropped.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-52 never".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    assert_eq!(pending_of(&mut c, None).len(), 1);
    start(&mut c, sb);
    wait_until(Duration::from_secs(5), "the hand prompt to drop the ask", || {
        pending_of(&mut c, None).is_empty()
    });
    stop(&mut c, sa);
    stop(&mut c, sb);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-52"), "a dropped ask must never land: {:?}", text());

    // (3) A second queued ask on the same ticket replaces the first.
    start(&mut c, sa);
    for probe in ["mesimon-probe-53 first", "mesimon-probe-54 second"] {
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket: b,
                text: probe.into(),
                queued: true,
                immediately: false,
                accept_plan: false,
                plan: false,
                tier: None,
                resend: false,
            }),
            Response::Queued { .. }
        ));
    }
    let p = pending_of(&mut c, None);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-54 second"));
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "the replaced ask to land", || {
        text().contains("mesimon-probe-54")
    });
    assert!(!text().contains("mesimon-probe-53"), "{:?}", text());
    start(&mut c, sb);
    stop(&mut c, sb);

    // (4) A quiet checkout sends at once: `Ok`, not `Queued`.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-55 now".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(10), "the immediate ask to land", || {
        text().contains("mesimon-probe-55")
    });
    start(&mut c, sb);
    stop(&mut c, sb);

    // (5) The person drops it: once is `Ok`, twice is nothing to drop.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-56 dropped".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: b }), Response::Ok));
    err_containing(c.request(Command::DropQueuedAsk { ticket: b }), "nothing queued");
    assert!(pending_of(&mut c, None).is_empty());
    stop(&mut c, sa);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-56"), "{:?}", text());

    // (6) Sleeping the waiting agent drops its ask: a parked pane is not one
    // the words can be pasted into later without waking it against the
    // user's gesture.
    start(&mut c, sa);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-57 asleep".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    assert!(matches!(c.request(Command::SleepSession { id: sb }), Response::Ok));
    c.await_state(sb, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(pending_of(&mut c, None).is_empty());
    // A send-now at the parked seat is refused (T-685): the TUI offers the
    // `immediately` stop by `Board::send_now_seat` and the daemon refuses by
    // it, so a wake — which has no turn for the key to cut into — gets the
    // same answer from both. The refusal names `now`, which the field does
    // offer there.
    err_containing(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-57b at once".into(),
            queued: false,
            immediately: true,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        "no turn for a send-now",
    );
    assert!(pending_of(&mut c, None).is_empty(), "a refusal parks nothing");
    assert_eq!(c.board().live_agent(b).map(|s| s.state.clone()), Some(SessionState::Sleeping));
    // And a queued ask AT a sleeping claude parks as a wake (T-294): the
    // delivery is what wakes it, so the words wait with everything else
    // rather than starting a turn in a checkout somebody else is holding.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: b,
            text: "mesimon-probe-58 later".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    let p = pending_of(&mut c, Some(b));
    assert_eq!(p.len(), 1);
    assert_eq!(
        p[0].action,
        PendingAction::Wake,
        "the card says a session will wake, not that words wait"
    );
    // Take it back off: the wake's own road is the test below, and this one
    // is about to free the checkout.
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: b }), Response::Ok));
    stop(&mut c, sa);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!text().contains("mesimon-probe-57"), "{:?}", text());

    let _ = c.request(Command::Shutdown);
}

/// T-294: an EMPTY seat is a seat too. A queued ask on a ticket with no
/// claude parks a START — the loudest thing the board's Shift+Enter does,
/// and the one that most deserves to wait — and the checkout going quiet
/// spawns one on the ticket's own title with the words under it.
#[test]
fn a_queued_start_waits_for_the_checkout_and_then_spawns_a_claude() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) =
        Harness::boot_with_env("askstart", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let got = h.dir.join("got.txt");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askstart");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();

    for title in ["holder", "waiter"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
            tier: None,
        });
    }
    let board = c.board();
    let a = board.tickets.iter().find(|t| t.title == "holder").expect("a").id;
    let b = board.tickets.iter().find(|t| t.title == "waiter").expect("b").id;
    let a_key = board.ticket(a).unwrap().short_key.clone();

    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    // A stub emits no `SessionStart`, so the record sits at `Spawning` —
    // which is WORKING — until the hooks say otherwise.
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);

    // The ask parks: no session on B, and the card says one is coming.
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-71 read the ticket".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { behind, .. } => assert_eq!(behind, vec![a_key.clone()]),
        other => panic!("expected the start to be parked: {other:?}"),
    }
    let p = pending_of(&mut c, Some(b));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].action, PendingAction::Start);
    assert_eq!(p[0].waits_on, vec![a_key]);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-71 read the ticket"));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        c.board().sessions.iter().all(|s| s.ticket != b),
        "nothing may start while the checkout is held"
    );

    // A settles; the start goes.
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(10), "the queued start to spawn a claude", || {
        c.board().sessions.iter().any(|s| s.ticket == b && s.kind == SessionKind::Claude)
    });
    let sb = c
        .board()
        .sessions
        .iter()
        .find(|s| s.ticket == b)
        .map(|s| (s.id, s.pending_submit))
        .expect("the started session");
    assert!(sb.1, "the composed spawn owes its Enter");
    assert!(pending_of(&mut c, Some(b)).is_empty(), "the entry left with the delivery");

    // The words ride the spawn: typed title first, then the paste on the
    // first tick after `SessionStart`, exactly as a composed spawn does.
    hook_send(&hook_sock, &sb.0.to_string(), "SessionStart", r#"{"source":"startup"}"#);
    wait_until(Duration::from_secs(10), "the parked words to land", || {
        text().contains("mesimon-probe-71 read the ticket")
    });
    assert!(text().contains("waiter"), "the title was typed too: {:?}", text());

    let _ = c.request(Command::Shutdown);
}

/// Two asks wait on one checkout; the card higher in its column goes first,
/// and a move while they wait re-sorts them (T-263). `waits_on` names the
/// holder and then the asks ahead, so the row's `+N` falls as a card rises.
#[test]
fn queued_asks_go_in_board_order_and_a_move_resorts_them() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot_with_env(
        "askq-order",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let got = h.dir.join("got.txt");
    let tmux_sock = h.paths.tmux_sock();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askq-order");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();

    for title in ["holder", "upper", "lower"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
            tier: None,
        });
    }
    let board = c.board();
    let id = |title: &str| board.tickets.iter().find(|t| t.title == title).expect(title).id;
    let (a, x, y) = (id("holder"), id("upper"), id("lower"));
    let key = |t| board.ticket(t).unwrap().short_key.clone();
    let (a_key, x_key, y_key) = (key(a), key(x), key(y));

    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let (sa, sx, sy) = (spawn(&mut c, a), spawn(&mut c, x), spawn(&mut c, y));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let panes = tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
            .unwrap_or(0);
        if panes >= 3 {
            break;
        }
        assert!(Instant::now() < deadline, "the stub agents never got their panes");
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));

    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    for sid in [sa, sx, sy] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    // Both waiters idle in one column now (automove parked them); put
    // `upper` above `lower` by hand so the starting order is stated, not
    // inherited from who finished last.
    let col = c.board().ticket(y).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: x, column: col.clone(), before: Some(y) }),
        Response::Ok
    ));

    // The holder works; `lower` asks first, `upper` second — and `upper`
    // is still ahead, because the board says so.
    start(&mut c, sa);
    for (t, probe) in [(y, "mesimon-probe-61 lower"), (x, "mesimon-probe-62 upper")] {
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket: t,
                text: probe.into(),
                queued: true,
                immediately: false,
                accept_plan: false,
                plan: false,
                tier: None,
                resend: false,
            }),
            Response::Queued { .. }
        ));
    }
    let waits = |c: &mut TestClient, t| pending_of(c, Some(t)).remove(0).waits_on;
    assert_eq!(waits(&mut c, x), vec![a_key.clone()]);
    assert_eq!(waits(&mut c, y), vec![a_key.clone(), x_key.clone()]);
    let order: Vec<ulid::Ulid> = pending_of(&mut c, None).iter().map(|p| p.ticket).collect();
    assert_eq!(order, vec![x, y], "the snapshot lists them in board order");

    // The user sorts while they wait: `lower` moves above `upper`.
    assert!(matches!(
        c.request(Command::MoveTicket { id: y, column: col, before: Some(x) }),
        Response::Ok
    ));
    assert_eq!(waits(&mut c, y), vec![a_key.clone()]);
    assert_eq!(waits(&mut c, x), vec![a_key.clone(), y_key.clone()]);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!text().contains("mesimon-probe-6"), "parked words must not land: {:?}", text());

    // The holder settles: the TOP card's ask goes, the other keeps waiting
    // on it until its agent has acked and finished.
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "the top ask to land", || {
        text().contains("mesimon-probe-61 lower")
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !text().contains("mesimon-probe-62"),
        "the lower card's ask jumped the queue: {:?}",
        text()
    );
    assert_eq!(waits(&mut c, x), vec![y_key.clone()]);
    start(&mut c, sy);
    stop(&mut c, sy);
    wait_until(Duration::from_secs(10), "the second ask to land", || {
        text().contains("mesimon-probe-62 upper")
    });
    start(&mut c, sx);
    wait_until(Duration::from_secs(5), "the ack to clear the entry", || {
        pending_of(&mut c, None).is_empty()
    });
    stop(&mut c, sx);

    let _ = c.request(Command::Shutdown);
}

/// Follow-ups in an isolated worktree wait for this turn, including its
/// permission stop; a question stop HOLDS them for a person (T-420).
/// Explicit send-now bypasses the idle wait, never a dialog.
#[test]
fn worktree_follow_up_waits_for_idle_with_send_now_and_take_back() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) =
        Harness::boot_with_env("idleq", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    init_repo(&h.repo, "seed", "seed\n");
    let mut c = h.client("idleq");
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "isolated follow-ups".into(),
        workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning | Response::Spawned { .. }
    ));
    let mut session = None;
    wait_until(Duration::from_secs(20), "worktree agent", || {
        session = c.board().live_agent(ticket).map(|s| s.id);
        session.is_some()
    });
    let sid = session.unwrap();
    let hooks = h.paths.hook_sock();
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    hook_send(&hooks, &sid.to_string(), "UserPromptSubmit", "{}");
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    let queue = |c: &mut TestClient, words: &str| {
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket,
                text: words.into(),
                queued: true,
                immediately: false,
                accept_plan: false,
                plan: false,
                tier: None,
                resend: false,
            }),
            Response::Queued { .. }
        ));
    };
    queue(&mut c, "idle-queue-first");
    // A permission prompt holds the words through it, as it always did,
    // and a send-now into the dialog is refused (T-420): a paste there is
    // an answer, not an ask.
    let permission = r#"{"tool_name":"Bash"}"#;
    hook_send(&hooks, &sid.to_string(), "PermissionRequest", permission);
    c.await_state(sid, "waiting for person", |s| matches!(s, SessionState::RequiresAction { .. }));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!text().contains("idle-queue-first"));
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| !p.in_flight && p.held.is_none()), "{p:?}");
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket }), Response::Err { .. }));
    hook_send(&hooks, &sid.to_string(), "PostToolUse", permission);
    c.await_state(sid, "running again", |s| *s == SessionState::Running);
    // A QUESTION holds them for a person (T-420): the answer may change
    // what the follow-up should say, so the idle after it delivers nothing
    // and the row says why; `^y` sends.
    let question = r#"{"tool_name":"AskUserQuestion"}"#;
    hook_send(&hooks, &sid.to_string(), "PreToolUse", question);
    c.await_state(sid, "asking", |s| matches!(s, SessionState::RequiresAction { .. }));
    wait_until(Duration::from_secs(5), "the ask to be held", || {
        pending_of(&mut c, Some(ticket)).iter().any(|p| p.held.as_deref() == Some("agent asked"))
    });
    hook_send(&hooks, &sid.to_string(), "PostToolUse", question);
    c.await_state(sid, "running again", |s| *s == SessionState::Running);
    hook_send(&hooks, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!text().contains("idle-queue-first"), "a held ask never goes on its own");
    assert!(pending_of(&mut c, Some(ticket)).iter().any(|p| p.held.is_some()));
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket }), Response::Ok));
    wait_until(Duration::from_secs(10), "the held ask, sent by hand", || {
        text().contains("idle-queue-first")
    });
    assert_eq!(text().matches("idle-queue-first").count(), 1);
    hook_send(&hooks, &sid.to_string(), "UserPromptSubmit", "{}");
    c.await_state(sid, "running", |s| *s == SessionState::Running);
    queue(&mut c, "idle-queue-send-now");
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket }), Response::Ok));
    wait_until(Duration::from_secs(5), "send now during turn", || {
        text().contains("idle-queue-send-now")
    });
    // The by-hand send is a paste of mesimon's own and owes its ack like
    // every other (T-244): until Claude takes it the checkout is busy, so
    // the ack lands here as it would in a real pane.
    hook_send(&hooks, &sid.to_string(), "UserPromptSubmit", "{}");
    queue(&mut c, "idle-queue-taken-back");
    assert!(
        matches!(c.request(Command::TakeQueuedAsk { ticket }), Response::PromptTakenBack { text } if text == "idle-queue-taken-back")
    );
    hook_send(&hooks, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!text().contains("idle-queue-taken-back"));
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "idle-queue-already-idle".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(5), "immediate idle delivery", || {
        text().contains("idle-queue-already-idle")
    });
}

/// Words queued while an agent is ALREADY on its question (T-565): the
/// T-420 hold was an edge, so an ask parked after the stop read `queued ∙
/// after its turn` and waited on a turn only a person's answer could end.
/// Now the park itself holds it — nothing is pasted, `^y` is refused while
/// the question is up, and lands after the answer — and an ask in the same
/// checkout behind the questioned agent says whose answer it waits on.
#[test]
fn an_ask_queued_onto_an_open_question_is_held_at_once() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) =
        Harness::boot_with_env("askedq", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let got = h.dir.join("got.txt");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askedq");
    let text = || std::fs::read_to_string(&got).unwrap_or_default();
    for title in ["asker", "behind"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
            tier: None,
        });
    }
    let board = c.board();
    let a = board.tickets.iter().find(|t| t.title == "asker").expect("a").id;
    let b = board.tickets.iter().find(|t| t.title == "behind").expect("b").id;
    let a_key = board.ticket(a).unwrap().short_key.clone();
    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sa = spawn(&mut c, a);
    let sb = spawn(&mut c, b);
    std::thread::sleep(Duration::from_millis(800));
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    for sid in [sa, sb] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    let queue = |c: &mut TestClient, ticket, words: &str| {
        c.request(Command::PromptSession {
            ticket,
            text: words.into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        })
    };

    // A stops on its question, mid-turn.
    let question = r#"{"tool_name":"AskUserQuestion"}"#;
    start(&mut c, sa);
    hook_send(&hook_sock, &sa.to_string(), "PreToolUse", question);
    c.await_state(sa, "asking", SessionState::question_stop);

    // B, in the same checkout, waits on A's ANSWER and says so.
    match queue(&mut c, b, "mesimon-probe-71 behind the question") {
        Response::Queued { behind, asking, held } => {
            assert_eq!(behind, vec![a_key.clone()]);
            assert_eq!(asking, vec![a_key.clone()]);
            assert_eq!(held, None, "B's own agent asked nothing");
        }
        other => panic!("expected B parked: {other:?}"),
    }
    // A's own words, queued onto the open question, are held at once.
    match queue(&mut c, a, "mesimon-probe-72 after your answer") {
        Response::Queued { behind, asking, held } => {
            assert!(behind.is_empty(), "a held ask waits on nobody's turn: {behind:?}");
            assert_eq!(asking, vec![a_key.clone()]);
            assert_eq!(held.as_deref(), Some("agent asked"));
        }
        other => panic!("expected A held: {other:?}"),
    }
    let pa = pending_of(&mut c, Some(a));
    assert!(pa.iter().any(|p| p.is_held() && p.held.as_deref() == Some("agent asked")), "{pa:?}");
    let pb = pending_of(&mut c, Some(b));
    assert!(pb.iter().any(|p| p.asking == vec![a_key.clone()] && !p.is_held()), "{pb:?}");
    // `^y` into the open dialog is refused: a paste there is an answer.
    match c.request(Command::SendQueuedAsk { ticket: a }) {
        Response::Err { message } => assert!(message.contains("answer it"), "{message}"),
        other => panic!("a send into the question: {other:?}"),
    }
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!text().contains("mesimon-probe-7"), "nothing is pasted: {:?}", text());

    // The person answers; A's turn runs on, B now waits on its turn.
    hook_send(&hook_sock, &sa.to_string(), "PostToolUse", question);
    c.await_state(sa, "answered", |s| *s == SessionState::Running);
    let pb = pending_of(&mut c, Some(b));
    assert!(pb.iter().any(|p| p.asking.is_empty() && p.waits_on == vec![a_key.clone()]), "{pb:?}");
    // A's turn ends: B's words go, A's stay held for the person's send.
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "B's words after A's turn", || {
        text().contains("mesimon-probe-71 behind the question")
    });
    start(&mut c, sb);
    stop(&mut c, sb);
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!text().contains("mesimon-probe-72"), "a held ask never goes on its own");
    let pa = pending_of(&mut c, Some(a));
    assert!(pa.iter().any(|p| p.is_held() && p.asking.is_empty()), "{pa:?}");
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: a }), Response::Ok));
    wait_until(Duration::from_secs(10), "A's held words, sent by hand", || {
        text().contains("mesimon-probe-72 after your answer")
    });
    assert_eq!(text().matches("mesimon-probe-72").count(), 1);
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed.contains("queued_ask_held_question"), "the park's hold is in the feed");

    let _ = c.request(Command::Shutdown);
}

/// A queued START outlives the daemon (T-418): the column's Shift+Enter
/// parked twenty starts, the first one's agent rebuilt the daemon and
/// `pkill`ed it, and the other nineteen vanished with no line in the feed.
/// Now `queue.json` carries starts and wakes across a restart; the card
/// shows the same `start` mark afterwards and the start goes when the
/// holder settles. A queued PANE ask — words owed to a pane the new daemon
/// re-derives at Low confidence — is still dropped, as before.
#[test]
fn a_queued_start_survives_a_daemon_restart_and_a_queued_pane_ask_does_not() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) =
        Harness::boot_with_env("askrestart", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("askrestart");

    for title in ["holder", "waiter"] {
        let _ = c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: title.into(),
            workspace: None,
            tier: None,
        });
    }
    let board = c.board();
    let a = board.tickets.iter().find(|t| t.title == "holder").expect("a").id;
    let b = board.tickets.iter().find(|t| t.title == "waiter").expect("b").id;
    let a_key = board.ticket(a).unwrap().short_key.clone();

    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);

    // A start parks on the empty seat, and a pane ask parks on the holder.
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-418 read the ticket".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { behind, .. } => assert_eq!(behind, vec![a_key.clone()]),
        other => panic!("expected the start to be parked: {other:?}"),
    }
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: a,
            text: "mesimon-probe-418 follow-up for the holder".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Queued { .. }
    ));
    assert_eq!(pending_of(&mut c, None).len(), 2);
    assert!(h.paths.queue_file().is_file(), "the start is on disk the moment it parks");

    // The daemon goes and comes back; the holder's pane never noticed.
    h.restart();
    let mut c = h.client("askrestart-2");
    let p = pending_of(&mut c, None);
    assert_eq!(p.len(), 1, "the start came back and the pane ask did not: {p:?}");
    assert_eq!(p[0].ticket, b);
    assert_eq!(p[0].action, PendingAction::Start);
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-418 read the ticket"));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        c.board().sessions.iter().all(|s| s.ticket != b),
        "a restored start waits for the checkout like any other"
    );

    // The holder settles; the restored start goes, and the file goes with it.
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(10), "the restored start to spawn a claude", || {
        c.board().sessions.iter().any(|s| s.ticket == b && s.kind == SessionKind::Claude)
    });
    assert!(pending_of(&mut c, None).is_empty(), "the entry left with the delivery");
    wait_until(Duration::from_secs(5), "queue.json to be removed once empty", || {
        !h.paths.queue_file().exists()
    });

    let _ = c.request(Command::Shutdown);
}

/// Cooked for four seconds — long enough for a `SessionStart` and a daemon
/// restart before anything could be pasted — then raw, bracketed paste and
/// the composer, as Claude Code comes up; every byte it reads is kept.
const SLOW: &str = "#!/bin/sh
d=\"$(dirname \"$0\")\"
sleep 4
stty -icanon -echo 2>/dev/null
printf '\\033[?2004h\\033[999;1H\\033[3A────────────────────\\n❯ \\n────────────────────\\n  ? for shortcuts'
exec cat >> \"$d/got.bin\"
";

const BRIEF: &str = "## Brief\n\nmesimon-probe-722 the brief outlives the daemon";

/// A composed start on `SLOW`, its `SessionStart` sent with a transcript
/// path under the fixture, and the daemon restarted before the composer
/// painted — so before any paste and any ack (T-722). `took`: the
/// conversation took a prompt meanwhile (its transcript is written in the
/// restart's window). Returns the client on the new daemon, the ticket,
/// the session and its title.
fn restart_before_the_brief(
    h: &Harness,
    took: bool,
) -> (TestClient, ulid::Ulid, uuid::Uuid, &'static str) {
    let title = "mesimon-probe-722 resend me";
    let mut c = h.client("owed");
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.into_iter().find(|t| t.title == title).expect("ticket").id;
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: BRIEF.into(), rev: None }),
        Response::NoteWritten { .. }
    ));
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let transcript = h.dir.join(format!("{sid}.jsonl"));
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&h.paths.hook_sock(), &sid.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(
        c.board().sessions.iter().any(|s| s.id == sid && s.pending_submit),
        "the brief is owed"
    );
    drop(c);

    h.restart_after(|| {
        let text = std::fs::read_to_string(h.paths.sessions_file()).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let rec = file["sessions"]
            .as_array()
            .and_then(|s| s.iter().find(|r| r["id"] == sid.to_string()))
            .cloned()
            .expect("the record on disk");
        assert_eq!(
            rec["unsent"],
            serde_json::json!({ "brief": true }),
            "the brief is kept on the seat across the restart: {rec}"
        );
        if took {
            std::fs::write(&transcript, "{}\n").unwrap();
        }
    });
    (h.client("owed-2"), ticket, sid, title)
}

fn read_bin(h: &Harness) -> String {
    String::from_utf8_lossy(&std::fs::read(h.dir.join("got.bin")).unwrap_or_default()).into_owned()
}

/// The bracketed pastes the stub read, with tmux's CR read back as the
/// newline the paste carried.
fn pastes(text: &str) -> Vec<String> {
    text.split("\x1b[200~")
        .skip(1)
        .filter_map(|rest| rest.split_once("\x1b[201~").map(|(body, _)| body.replace('\r', "\n")))
        .collect()
}

/// A daemon restart between a launch and its first prompt (T-722): the
/// owed brief was memory and died with it, the feed said
/// `prompt_submit_abandoned`, and the worker sat at its composer with the
/// title typed. Now the shutdown keeps it on the seat as `unsent`, and the
/// new daemon — the seat at its composer, its conversation never prompted —
/// sends it again by itself once the composer paints: title and brief, one
/// paste, `prompt_resent` in the feed, and the agent's ack clears the mark.
#[test]
fn a_brief_owed_across_a_daemon_restart_is_resent_to_a_seat_that_took_no_prompt() {
    let env = [("MESIMON_CLAUDE_ROAD", "hooks"), ("MESIMON_PANE_QUIET_MS", "600000")];
    let Some(h) = Harness::boot_bare("owedrestart", Some(SLOW), &env) else { return };
    let (mut c, ticket, sid, title) = restart_before_the_brief(&h, false);

    let deadline = Instant::now() + Duration::from_secs(20);
    let text = loop {
        let text = read_bin(&h);
        if text.contains("\x1b[201~") {
            break text;
        }
        assert!(Instant::now() < deadline, "the brief was never resent: {text:?}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(pastes(&text), vec![format!("{title}\n\n{BRIEF}")], "title and brief: {text:?}");
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert!(rec.pending_submit, "the Enter is owed until the ack");
    assert!(rec.ticket_read, "the brief went in");
    assert_eq!(c.board().needs_you_count(), 0, "a resend under way is not a failed start");

    hook_send_with(&h.paths.hook_sock(), &sid.to_string(), "UserPromptSubmit", None, "{}");
    wait_until(Duration::from_secs(5), "the ack", || {
        c.board().sessions.iter().any(|s| s.id == sid && !s.pending_submit && s.unsent.is_none())
    });
    wait_until(Duration::from_secs(3), "the resend's feed line", || {
        feed_count(&h, "prompt_resent", ticket) == 1
            && feed_count(&h, "prompt_submitted", ticket) == 1
    });
    assert_eq!(feed_count(&h, "prompt_submit_abandoned", ticket), 0);
    let _ = c.request(Command::KillSession { id: sid });
}

/// The same restart at a seat whose conversation took a prompt in the
/// window (T-722): its transcript exists, so nothing is sent again — a
/// second brief into a conversation under way is the double prompt T-603
/// guards — and the seat keeps the card's `brief not sent` and its resend.
#[test]
fn a_brief_owed_across_a_restart_is_not_resent_to_a_seat_that_took_a_prompt() {
    let env = [("MESIMON_CLAUDE_ROAD", "hooks"), ("MESIMON_PANE_QUIET_MS", "600000")];
    let Some(h) = Harness::boot_bare("owedtook", Some(SLOW), &env) else { return };
    let (mut c, ticket, sid, _) = restart_before_the_brief(&h, true);

    // Past the stub's composer, and a cadence or two beyond.
    std::thread::sleep(Duration::from_millis(6000));
    assert!(pastes(&read_bin(&h)).is_empty(), "nothing is pasted: {:?}", read_bin(&h));
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert_eq!(
        rec.unsent,
        Some(mesimon_core::board::Unsent { text: String::new(), brief: true }),
        "the mark stays for a person"
    );
    assert!(!rec.pending_submit, "nothing is owed");
    assert_eq!(c.board().needs_you_count(), 1, "the card says so");
    assert_eq!(feed_count(&h, "prompt_resent", ticket), 0);
    assert_eq!(feed_count(&h, "prompt_submit_abandoned", ticket), 0);
    let _ = c.request(Command::KillSession { id: sid });
}
