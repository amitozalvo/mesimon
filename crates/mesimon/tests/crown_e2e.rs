//! The crown (T-411): one ticket per board whose agent may edit the others
//! through the keyed forms of its tools. A person grants it (`CrownTicket`),
//! an agent never can; an uncrowned agent reaching for another ticket reads
//! how a person grants one; every keyed write is judged against the ticket
//! as it was READ (`seen`); the touched card rides the snapshot for the
//! board to light; and the crown leaves with its ticket. The crown's one
//! start (T-412) sits behind the board's spawn budget: a seat it started
//! is counted while held, the cap names its holders, and a crown-started
//! ticket can never be crowned. The crown's ask (T-413) is words HELD on
//! another ticket's card: nothing reaches the pane until a person's send.
//! The crown's sleep (T-539) parks an idle agent it started and nobody
//! else's, so the archive that was refused over the awake seat goes through,
//! and the park alone frees that agent's budget seat (T-541).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AgentTicketView, Command, CrownTouch, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

fn create(c: &mut TestClient, title: &str) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    }
}

fn spawn(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    }
}

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

fn touches(c: &mut TestClient) -> Vec<CrownTouch> {
    match c.request(Command::Snapshot) {
        Response::Board { crown_touches, .. } => crown_touches,
        other => panic!("snapshot: {other:?}"),
    }
}

#[test]
fn the_crown_lets_one_agent_edit_the_others() {
    // The stub records what reaches its stdin, so the held ask can be
    // shown to land only on the person's send.
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    // No starter tags (the registry is built by hand), and the archive offer
    // prices a ticket the moment it is untouched rather than after an hour,
    // so the offer's road can be driven at the end.
    let Some(h) = Harness::boot_with_env(
        "crown",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_ARCHIVE_SUGGEST_MS", "0")],
    ) else {
        return;
    };
    let sock = h.paths.orch_sock();
    let mut c = h.client("crown");

    let a = create(&mut c, "triage the board");
    let b = create(&mut c, "fix the thing");
    let d = create(&mut c, "later");
    let (ka, kb, kd) = (key_of(&mut c, a), key_of(&mut c, b), key_of(&mut c, d));
    let sa = spawn(&mut c, a);
    let sb = spawn(&mut c, b);

    // ---- uncrowned: another ticket is refused, and the refusal teaches ----
    let refusal = read(&mut c, sa, &kb).expect_err("no crown yet");
    for word in ["crown", "^o", "raise_hand", &ka] {
        assert!(refusal.contains(word), "the refusal names {word}: {refusal}");
    }
    assert!(refusal.contains("no ticket wears the crown"), "{refusal}");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentMoveTicket {
            to_column: "IN PROGRESS".into(),
            idempotency_key: None,
            key: Some(kb.clone()),
            before: None,
            seen: None,
        },
    ) {
        Response::Err { message } => assert!(message.contains("crown"), "{message}"),
        other => panic!("an uncrowned move of another ticket: {other:?}"),
    }
    // A session may always address its OWN ticket by key, crown or not.
    let own = read(&mut c, sa, &ka).expect("own key");
    assert!(!own.crowned);
    assert!(own.crown.is_none(), "an uncrowned ticket carries no wake words");
    assert!(own.seen.is_some(), "every read carries a stamp");
    // An unknown key is an answer, not a crown question.
    let unknown = read(&mut c, sa, "T-999").expect_err("no such ticket");
    assert!(unknown.contains("no such ticket"), "{unknown}");

    // ---- the crown is the person's to give ---------------------------------
    match c.send(Principal::Agent { session: sa }, Command::CrownTicket { id: a }) {
        Response::Err { message } => assert!(message.contains("not available"), "{message}"),
        other => panic!("an agent crowning itself: {other:?}"),
    }
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(c.board().crown, Some(a));
    assert!(c.board().is_crowned(a));
    let own = read(&mut c, sa, &ka).unwrap();
    assert!(own.crowned, "get_ticket says the caller wears it");
    // T-537: and how the board wakes it, so it arms no monitor of its own.
    assert_eq!(own.crown.as_deref(), Some(mesimon_core::mcp::CROWN_WAKES));
    match c.send(Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => {
            assert_eq!(board.crown.as_deref(), Some(ka.as_str()));
            let row = board.tickets.iter().find(|t| t.key == kb).unwrap();
            assert_eq!(row.by.as_deref(), Some("person"));
            assert!(row.state.is_some(), "a ticket with an agent has a state word");
            let later = board.tickets.iter().find(|t| t.key == kd).unwrap();
            assert!(later.state.is_none(), "no agent, no word");
        }
        other => panic!("list_board: {other:?}"),
    }

    // ---- read another ticket: the card's words and a stamp ------------------
    let view = read(&mut c, sa, &kb).expect("crowned");
    assert!(!view.crowned);
    assert!(view.crown.is_none(), "the wake's words ride the crowned ticket alone");
    let seen = view.seen.clone().expect("a stamp");
    let state = view.state.expect("B has an agent");
    assert!(!state.state.is_empty());

    // ---- a keyed move needs the stamp, and the stamp must be fresh -----------
    let mv = |seen: Option<&str>, before: Option<&str>| Command::AgentMoveTicket {
        to_column: "IN PROGRESS".into(),
        idempotency_key: None,
        key: Some(kb.clone()),
        before: before.map(str::to_string),
        seen: seen.map(str::to_string),
    };
    match c.send(Principal::Agent { session: sa }, mv(None, None)) {
        Response::Err { message } => assert!(message.contains("seen is required"), "{message}"),
        other => panic!("no stamp: {other:?}"),
    }
    match c.send(Principal::Agent { session: sa }, mv(Some("stale"), None)) {
        Response::Err { message } => {
            assert!(message.contains("changed since it was read"), "{message}");
            assert!(message.contains(&kb), "{message}");
        }
        other => panic!("a stale stamp: {other:?}"),
    }
    let fresh = match c.send(Principal::Agent { session: sa }, mv(Some(&seen), None)) {
        Response::AgentMoved { column, seen, replayed, .. } => {
            assert_eq!(column, "IN PROGRESS");
            assert!(!replayed);
            seen.expect("a keyed move hands back the fresh stamp")
        }
        other => panic!("the move: {other:?}"),
    };
    assert_ne!(fresh, seen, "the ticket changed, so the stamp did");
    assert_eq!(c.board().ticket(b).unwrap().column, "IN PROGRESS");
    let t = touches(&mut c);
    assert_eq!(t.len(), 1, "the board is told what the crown touched: {t:?}");
    assert_eq!(t[0].ticket, b);
    assert_eq!(t[0].action, "moved");

    // ---- position is priority: `before` lands above a named ticket ----------
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentMoveTicket {
            to_column: "IN PROGRESS".into(),
            idempotency_key: None,
            key: Some(kd.clone()),
            before: Some(kb.clone()),
            seen: dv.seen,
        },
    ) {
        Response::AgentMoved { column, .. } => assert_eq!(column, "IN PROGRESS"),
        other => panic!("the priority move: {other:?}"),
    }
    let order: Vec<String> =
        c.board().column_tickets("IN PROGRESS").iter().map(|t| t.short_key.clone()).collect();
    assert_eq!(order, [kd.clone(), kb.clone()], "D above B");

    // ---- rename, with the receipt's stamp; the old stamp is now stale --------
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket {
            key: kb.clone(),
            title: "  fix it properly ".into(),
            seen: Some(fresh.clone()),
        },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.title, "fix it properly"),
        other => panic!("rename: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket {
            key: kb.clone(),
            title: "again".into(),
            seen: Some(fresh.clone()),
        },
    ) {
        Response::Err { message } => assert!(message.contains("changed since"), "{message}"),
        other => panic!("a reused stamp: {other:?}"),
    }
    assert_eq!(c.board().ticket(b).unwrap().title, "fix it properly");
    assert_eq!(touches(&mut c).iter().find(|t| t.ticket == b).unwrap().action, "renamed");

    // ---- a note and a tag on another ticket ---------------------------------
    let note = match c.send(
        Principal::Agent { session: sa },
        Command::AgentWriteNote {
            note: None,
            text: "from the crown".into(),
            key: Some(kb.clone()),
        },
    ) {
        Response::NoteWritten { note } => note.expect("created"),
        other => panic!("keyed note: {other:?}"),
    };
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentReadNote { note, key: Some(kb.clone()) },
    ) {
        Response::Note { text, meta } => {
            assert_eq!(text.trim(), "from the crown");
            assert_eq!(meta.created_by, format!("agent:{sa}"));
        }
        other => panic!("keyed read_note: {other:?}"),
    }
    assert!(matches!(
        c.request(Command::RegisterTag { group: 1, name: "BUG".into() }),
        Response::Ok
    ));
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentTagTicket {
            name: "bug".into(),
            group: None,
            remove: false,
            key: Some(kb.clone()),
        },
    ) {
        Response::AgentTagged { tags, seen, .. } => {
            assert_eq!(tags[0].name, "BUG");
            assert!(seen.is_some(), "a keyed tag hands back the stamp");
        }
        other => panic!("keyed tag: {other:?}"),
    }
    assert_eq!(c.board().ticket(b).unwrap().tags[0].name, "BUG");

    // ---- workspace: a pending ticket only ------------------------------------
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace { key: kd.clone(), workspace: "worktree".into(), seen: dv.seen },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.workspace, "worktree"),
        other => panic!("set_workspace: {other:?}"),
    }
    let bv = read(&mut c, sa, &kb).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace { key: kb.clone(), workspace: "worktree".into(), seen: bv.seen },
    ) {
        Response::Err { message } => assert!(message.contains("locked"), "{message}"),
        other => panic!("a ticket with a pane keeps its workspace: {other:?}"),
    }
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSetWorkspace {
            key: kd.clone(),
            workspace: "elsewhere".into(),
            seen: dv.seen,
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("worktree or shared_checkout"), "{message}")
        }
        other => panic!("an unknown workspace word: {other:?}"),
    }

    // ---- archive is the reversible spelling of delete -------------------------
    let bv = read(&mut c, sa, &kb).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kb.clone(), restore: false, seen: bv.seen },
    ) {
        Response::Err { message } => {
            // The refusal says whose road it is (T-539): B's agent is the
            // person's, so no tool of the crown's parks it.
            assert!(message.contains("awake"), "{message}");
            assert!(message.contains("started by a person"), "{message}");
            assert!(!message.contains("sleep_agent parks it"), "{message}");
        }
        other => panic!("an awake ticket cannot be archived: {other:?}"),
    }
    // And `sleep_agent` itself refuses a person's agent by provenance,
    // whatever its state.
    let bv = read(&mut c, sa, &kb).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kb.clone(), seen: bv.seen },
    ) {
        Response::Err { message } => {
            assert!(message.contains("started by a person"), "{message}");
            assert!(message.contains("x on its card"), "{message}");
        }
        other => panic!("sleeping a person's agent: {other:?}"),
    }
    assert!(c.board().live_agent(b).unwrap().state.has_pane(), "B's agent is untouched");
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kd.clone(), restore: false, seen: dv.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("archive: {other:?}"),
    }
    assert!(c.board().ticket(d).unwrap().is_archived());
    // An archived ticket still reads, and refuses every edit but the restore.
    let dv = read(&mut c, sa, &kd).expect("an archived ticket still reads");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentRenameTicket { key: kd.clone(), title: "x".into(), seen: dv.seen.clone() },
    ) {
        Response::Err { message } => assert!(message.contains("archived"), "{message}"),
        other => panic!("renaming an archived ticket: {other:?}"),
    }
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kd.clone(), restore: true, seen: dv.seen },
    ) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.column, "IN PROGRESS"),
        other => panic!("restore: {other:?}"),
    }
    assert!(!c.board().ticket(d).unwrap().is_archived());

    // ---- start_agent: the crown starts work, behind the spawn budget --------
    // The budget is a board scalar a person sets; two here so the cap is
    // reached on the second start.
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    assert_eq!(c.board().crown_budget, 2);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_budget = 2"), "{file}");
    let e1 = create(&mut c, "first start");
    let e2 = create(&mut c, "second start");
    let e3 = create(&mut c, "third start");
    let (k1, k2, k3) = (key_of(&mut c, e1), key_of(&mut c, e2), key_of(&mut c, e3));
    let start = |c: &mut TestClient, key: &str, seen: Option<String>| {
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket { key: key.into(), seen, plan: false },
        )
    };
    // Refused: a ticket that already holds a seat, the crown's own ticket,
    // and a start without the stamp.
    let bv = read(&mut c, sa, &kb).unwrap();
    match start(&mut c, &kb, bv.seen) {
        Response::Err { message } => assert!(message.contains("already has an agent"), "{message}"),
        other => panic!("a start on a seated ticket: {other:?}"),
    }
    match start(&mut c, &ka, None) {
        Response::Err { message } => assert!(message.contains("own ticket"), "{message}"),
        other => panic!("a start on the crown's own ticket: {other:?}"),
    }
    match start(&mut c, &k1, None) {
        Response::Err { message } => assert!(message.contains("seen is required"), "{message}"),
        other => panic!("a start without the stamp: {other:?}"),
    }
    // The start: a launch on the card, the record carries the crown's id,
    // the touch says so, and the receipt says what is left.
    let v1 = read(&mut c, sa, &k1).unwrap();
    match start(&mut c, &k1, v1.seen) {
        Response::AgentStarted { key, session_started, budget_left } => {
            assert_eq!(key, k1);
            assert!(session_started);
            assert_eq!(budget_left, 1);
        }
        other => panic!("the first start: {other:?}"),
    }
    let started = c.board().live_agent(e1).cloned().expect("E1 holds a seat now");
    assert_eq!(started.started_by, Some(a), "the record names the crown's ticket");
    assert_eq!(started.kind, SessionKind::Claude);
    assert!(
        matches!(started.state, SessionState::Spawning | SessionState::Running),
        "{:?}",
        started.state
    );
    assert!(started.pending_submit, "Shift+Enter's road: the title is submitted, not typed");
    assert_eq!(touches(&mut c).iter().find(|t| t.ticket == e1).unwrap().action, "started");
    let sessions = std::fs::read_to_string(h.paths.state_dir.join("sessions.json")).unwrap();
    assert!(sessions.contains("\"started_by\""), "persisted, so a restart keeps the count");
    // A second start on the same ticket is refused by the seat.
    let v1 = read(&mut c, sa, &k1).unwrap();
    match start(&mut c, &k1, v1.seen) {
        Response::Err { message } => assert!(message.contains("already has an agent"), "{message}"),
        other => panic!("a second start on E1: {other:?}"),
    }
    // A crown-started ticket cannot be crowned: the graph is one level deep.
    match c.request(Command::CrownTicket { id: e1 }) {
        Response::Err { message } => assert!(message.contains("crown-started"), "{message}"),
        other => panic!("crowning a crown-started ticket: {other:?}"),
    }
    assert_eq!(c.board().crown, Some(a), "A still wears it");
    // The cap: the third start is refused with the number and the holders.
    let v2 = read(&mut c, sa, &k2).unwrap();
    match start(&mut c, &k2, v2.seen) {
        Response::AgentStarted { budget_left, .. } => assert_eq!(budget_left, 0),
        other => panic!("the second start: {other:?}"),
    }
    let v3 = read(&mut c, sa, &k3).unwrap();
    match start(&mut c, &k3, v3.seen) {
        Response::Err { message } => {
            for word in ["2 of 2", &k1, &k2] {
                assert!(message.contains(word), "the refusal names {word}: {message}");
            }
        }
        other => panic!("the (N+1)th start: {other:?}"),
    }
    // A seat frees when its agent exits (or sleeps: the wake test, T-541).
    let s2 = c.board().live_agent(e2).unwrap().id;
    let _ = c.request(Command::KillSession { id: s2 });
    assert!(c.board().live_agent(e2).is_none(), "E2's seat is free");
    let v3 = read(&mut c, sa, &k3).unwrap();
    match start(&mut c, &k3, v3.seen) {
        Response::AgentStarted { budget_left, .. } => assert_eq!(budget_left, 0),
        other => panic!("the start after a seat freed: {other:?}"),
    }
    // A start parked behind a worktree cut (T-466) is accepted, not refused:
    // the receipt says it is not running yet, the parked start already holds
    // its seat, and the spawn replays as the crown's once the cut is ready.
    init_repo(&h.repo, "a.txt", "hello\n");
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 3 }), Response::Ok));
    let e5 = create(&mut c, "parked start");
    let k5 = key_of(&mut c, e5);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: e5, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let v5 = read(&mut c, sa, &k5).unwrap();
    match start(&mut c, &k5, v5.seen) {
        Response::AgentStarted { session_started, budget_left, .. } => {
            assert!(!session_started, "parked on the cut, not running yet");
            assert_eq!(budget_left, 0, "the parked start holds its seat");
        }
        other => panic!("a start on an uncut worktree ticket: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(e5).is_some()
    });
    assert_eq!(c.board().live_agent(e5).unwrap().started_by, Some(a), "replayed as the crown's");
    // The feed says the agent started it (buffered; flushed on a later tick).
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    wait_until(std::time::Duration::from_secs(5), "the feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| l.contains("\"start_agent\"") && l.contains("\"actor\":\"agent\""))
        })
    });

    // A budget of zero turns the road off, in words.
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 0 }), Response::Ok));
    let e4 = create(&mut c, "never started");
    let k4 = key_of(&mut c, e4);
    let v4 = read(&mut c, sa, &k4).unwrap();
    match start(&mut c, &k4, v4.seen) {
        Response::Err { message } => assert!(message.contains("budget is 0"), "{message}"),
        other => panic!("a start at budget 0: {other:?}"),
    }
    assert!(c.board().live_agent(e4).is_none());

    // ---- ask_agent: words held on the card until a person sends them ----
    let got = h.dir.join("got.txt");
    let landed = |probe: &str| std::fs::read_to_string(&got).unwrap_or_default().contains(probe);
    let ask =
        |c: &mut TestClient, from: uuid::Uuid, key: &str, text: &str, seen: Option<String>| {
            c.send(
                Principal::Agent { session: from },
                Command::AgentAskTicket { key: key.into(), text: text.into(), seen, plan: false },
            )
        };
    // Refused: an uncrowned session, the crown's own ticket, no stamp, a
    // ticket with no agent to receive the words, and blank words.
    let bv = read(&mut c, sa, &kb).unwrap();
    match ask(&mut c, sb, &ka, "mesimon-probe-61 never", None) {
        Response::Err { message } => assert!(message.contains("crown"), "{message}"),
        other => panic!("an uncrowned ask: {other:?}"),
    }
    match ask(&mut c, sa, &ka, "mesimon-probe-61 never", None) {
        Response::Err { message } => assert!(message.contains("own ticket"), "{message}"),
        other => panic!("an ask at the crown's own ticket: {other:?}"),
    }
    match ask(&mut c, sa, &kb, "mesimon-probe-61 never", None) {
        Response::Err { message } => assert!(message.contains("seen is required"), "{message}"),
        other => panic!("an ask without the stamp: {other:?}"),
    }
    let dv = read(&mut c, sa, &kd).unwrap();
    match ask(&mut c, sa, &kd, "mesimon-probe-61 never", dv.seen) {
        Response::Err { message } => assert!(message.contains("start_agent"), "{message}"),
        other => panic!("an ask at an empty seat: {other:?}"),
    }
    match ask(&mut c, sa, &kb, "   \n ", bv.seen.clone()) {
        Response::Err { message } => assert!(message.contains("nothing to send"), "{message}"),
        other => panic!("a blank ask: {other:?}"),
    }
    // The ask: held on B's card, authored by A, delivered to nobody.
    let seen_after = match ask(&mut c, sa, &kb, "mesimon-probe-62 commit it", bv.seen) {
        Response::AgentAsked { key, replaced, seen } => {
            assert_eq!(key, kb);
            assert!(!replaced);
            seen.expect("a fresh stamp rides back")
        }
        other => panic!("the ask: {other:?}"),
    };
    let p = pending_of(&mut c, Some(b));
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(p[0].action, mesimon_core::command::PendingAction::Ask);
    assert_eq!(p[0].by.as_deref(), Some(ka.as_str()), "the card names the author");
    assert_eq!(p[0].text.as_deref(), Some("mesimon-probe-62 commit it"));
    assert!(p[0].waits_on.is_empty(), "it waits on a person, not the checkout: {p:?}");
    assert_eq!(touches(&mut c).iter().find(|t| t.ticket == b).unwrap().action, "asked");
    // The drain runs on the tick and at every settle; a held ask is not its.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!landed("mesimon-probe-62"), "held words must not reach the pane on their own");
    // A second ask replaces the first, and says so.
    match ask(&mut c, sa, &kb, "mesimon-probe-63 then push", Some(seen_after)) {
        Response::AgentAsked { replaced, .. } => assert!(replaced),
        other => panic!("the second ask: {other:?}"),
    }
    // Take-back hands the crown's words to the person, like any queued ask.
    match c.request(Command::TakeQueuedAsk { ticket: b }) {
        Response::PromptTakenBack { text } => assert_eq!(text, "mesimon-probe-63 then push"),
        other => panic!("take-back: {other:?}"),
    }
    assert!(pending_of(&mut c, Some(b)).is_empty());
    // A person's own queued ask is never overwritten by the crown's.
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-64 the person's".into(),
        queued: true,
        accept_plan: false,
        plan: false,
        tier: None,
    }) {
        Response::Queued { .. } | Response::Ok => {}
        other => panic!("the person's queued ask: {other:?}"),
    }
    if pending_of(&mut c, Some(b)).iter().any(|p| p.by.is_none() && !p.in_flight) {
        let bv = read(&mut c, sa, &kb).unwrap();
        match ask(&mut c, sa, &kb, "mesimon-probe-65 over it", bv.seen) {
            Response::Err { message } => assert!(message.contains("person's ask"), "{message}"),
            other => panic!("the crown over a person's ask: {other:?}"),
        }
        assert!(matches!(c.request(Command::DropQueuedAsk { ticket: b }), Response::Ok));
    }
    wait_until(std::time::Duration::from_secs(5), "B's queue to clear", || {
        pending_of(&mut c, Some(b)).is_empty()
    });
    // The person's send is the one road to the pane.
    let bv = read(&mut c, sa, &kb).unwrap();
    match ask(&mut c, sa, &kb, "mesimon-probe-66 now go", bv.seen) {
        Response::AgentAsked { replaced, .. } => assert!(!replaced),
        other => panic!("the third ask: {other:?}"),
    }
    assert!(!landed("mesimon-probe-66"));
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: b }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the sent words to land", || {
        landed("mesimon-probe-66 now go")
    });
    // The by-hand send is a paste of mesimon's own and owes its ack like
    // every other (T-244): the card says `sending` until Claude takes it.
    let p = pending_of(&mut c, Some(b));
    assert!(p.iter().all(|p| p.in_flight), "only the paste's own mark remains: {p:?}");
    hook_send(&h.paths.hook_sock(), &sb.to_string(), "UserPromptSubmit", "{}");
    wait_until(std::time::Duration::from_secs(5), "the paste's ack to clear it", || {
        pending_of(&mut c, Some(b)).is_empty()
    });
    // The feed names the tool and the actor, never the words; the words are
    // in no state file either.
    wait_until(std::time::Duration::from_secs(5), "the ask_agent feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| l.contains("\"ask_agent\"") && l.contains("\"actor\":\"agent\""))
        })
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-6"), "the feed never carries the words");
    let sessions = std::fs::read_to_string(h.paths.state_dir.join("sessions.json")).unwrap();
    assert!(!sessions.contains("mesimon-probe-6"), "sessions.json never carries the words");
    let queue = std::fs::read_to_string(h.paths.queue_file()).unwrap_or_default();
    assert!(!queue.contains("mesimon-probe-6"), "a held ask is never persisted");

    // ---- the shim: fifteen tools, and `get_ticket` with a key -----------------
    let mut shim = Shim::start(&sock, sa);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    let listed = shim.rpc("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 15);
    let r = shim.call("sleep_agent", json!({ "key": kb }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("ask_agent", json!({ "key": kb, "text": "x" }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("start_agent", json!({ "key": k4 }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");

    let other = shim.call_ok("get_ticket", json!({ "key": kb }));
    assert_eq!(other["key"], json!(kb));
    assert!(other["seen"].is_string(), "{other}");
    assert_eq!(other["crowned"], json!(false));
    let mine = shim.call_ok("get_ticket", json!({}));
    assert_eq!(mine["crowned"], json!(true));
    let r = shim.call("rename_ticket", json!({ "key": kb, "title": "x" }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");

    // ---- uncrown, and the crown leaves with its ticket ------------------------
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(c.board().crown.is_none());
    assert!(read(&mut c, sa, &kb).is_err(), "uncrowned again");
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // Persisted with the board's scalars.
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains(&format!("crown = \"{a}\"")), "{file}");
    // A second crown displaces the first.
    assert!(matches!(c.request(Command::CrownTicket { id: d }), Response::Ok));
    assert_eq!(c.board().crown, Some(d));
    assert!(read(&mut c, sa, &kb).is_err(), "A no longer wears it");
    // Deleting the holder takes the crown with it.
    assert!(matches!(
        c.request(Command::DeleteTicket { id: d, discard_worktree: false }),
        Response::Ok
    ));
    assert!(c.board().crown.is_none(), "the crown left with its ticket");
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(!file.contains("crown ="), "{file}");

    // ---- the header's archive offer drops it too, like `a a` ------------------
    // A sessionless ticket in the template's DONE column (reclaim on) is what
    // the offer prices; taking the offer archives it and the crown goes with
    // it — the one road that had skipped the drop.
    let e = create(&mut c, "shipped");
    assert!(matches!(c.request(Command::CrownTicket { id: e }), Response::Ok));
    assert!(matches!(
        c.request(Command::MoveTicket { id: e, column: "DONE".into(), before: None }),
        Response::Ok
    ));
    assert!(c.board().is_crowned(e));
    match c.request(Command::ArchiveAll) {
        Response::Archived { archived, .. } => assert!(archived >= 1, "the offer took E"),
        other => panic!("archive all: {other:?}"),
    }
    assert!(c.board().ticket(e).unwrap().is_archived());
    assert!(c.board().crown.is_none(), "the offer's archive drops the crown");
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(!file.contains("crown ="), "{file}");
}

/// The stub every wake test runs: each line it reads is appended to one file
/// beside it, so the crown's sentences and the workers' words are all there.
const RECORDING_STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// One commit in `repo` touching `file`; the new HEAD, short.
fn commit(repo: &std::path::Path, file: &str) -> String {
    std::fs::write(repo.join(file), format!("{file}\n")).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-qm", file]);
    git(repo, &["rev-parse", "--short=7", "HEAD"]).trim().to_string()
}

/// The board wakes the crown (T-414) on what happened to a ticket, not on
/// how its agent breathes (T-469): a worker it started DELIVERS — a turn
/// ends with a commit the crown has not heard of — or raises its hand, and
/// the daemon puts ONE sentence of its own, with what changed, in front of
/// the crown when the crown itself is idle. A turn that leaves nothing new
/// is silent. Deliveries that pile up while the crown works coalesce into
/// one sentence; a person's queued ask on the crown goes first; the hand's
/// reason never rides the sentence; uncrowning drops what was owed.
#[test]
fn the_board_wakes_the_crown_when_a_started_worker_delivers() {
    // The quiet probe would otherwise flip a stub between working and idle
    // on its own clock; the hooks are the only voice here.
    let Some(h) = Harness::boot_with_env(
        "crown_wake",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    // A shared checkout: a delivery is a HEAD the crown has not heard of.
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_wake");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
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
    let wake_rows = |c: &mut TestClient, t: ulid::Ulid| -> Vec<mesimon_core::command::Pending> {
        pending_of(c, Some(t))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };
    // The probe of a turn's end runs off the writer; give it time to land
    // before asserting that nothing did.
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));

    let a = create(&mut c, "coordinate");
    let w1 = create(&mut c, "mesimon-probe-71 worker");
    let (ka, kw1) = (key_of(&mut c, a), key_of(&mut c, w1));
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // The stub emits no `SessionStart`, so the crown sits at `Spawning` —
    // WORKING — until a turn is walked through it.
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);

    let start_agent = |c: &mut TestClient, key: &str| {
        let v = read(c, sa, key).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket { key: key.into(), seen: v.seen, plan: false },
        ) {
            Response::AgentStarted { .. } => {}
            other => panic!("start_agent {key}: {other:?}"),
        }
    };
    start_agent(&mut c, &kw1);
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));

    // A turn that leaves nothing new wakes nobody.
    start(&mut c, ws1);
    stop(&mut c, ws1);
    settle();
    assert_eq!(lines_with(&kw1), 0, "an empty turn is not news");
    assert!(wake_rows(&mut c, a).is_empty());

    // A commit, then the turn ends: one sentence, mesimon's template over
    // the worker's key and title and what changed, in the crown's pane.
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered1 = format!("{kw1} \"mesimon-probe-71 worker\" delivered");
    wait_until(std::time::Duration::from_secs(10), "the wake to land on the crown", || {
        lines_with(&delivered1) == 1
    });
    let column = c.board().ticket(w1).unwrap().column.clone();
    let sentence = format!(
        "{delivered1} (commit {h1}, column {column}) ∙ get_ticket key={kw1} for state and notes"
    );
    assert_eq!(lines_with(&sentence), 1, "{}", std::fs::read_to_string(&got).unwrap());
    assert!(wake_rows(&mut c, a).is_empty(), "delivered, so nothing is owed");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == a).map(|t| t.action.as_str()),
        Some("woke"),
        "the crown's card lights"
    );
    // The feed names both tickets and the cause, never the sentence.
    wait_until(std::time::Duration::from_secs(5), "the crown_wake feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"crown\":\"{a}\""))
                    && l.contains(&format!("\"worker\":\"{w1}\""))
                    && l.contains("\"cause\":\"delivered\"")
            })
        })
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-71"), "the feed never carries the words:\n{feed}");
    assert!(feed.contains("\"crown_wake_sent\""), "{feed}");
    // The crown takes its turn on it (the paste's ack).
    start(&mut c, sa);
    stop(&mut c, sa);

    // A second `Stop` on an idle worker is no edge, and a whole second turn
    // at the same HEAD is nothing new: once per delivery.
    hook_send(&hook_sock, &ws1.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    start(&mut c, ws1);
    stop(&mut c, ws1);
    settle();
    assert!(wake_rows(&mut c, a).is_empty());
    assert_eq!(lines_with(&delivered1), 1, "a second idle with nothing new is silent");

    // ---- two deliveries while the crown works: one row, one sentence ------
    start(&mut c, sa);
    let w2 = create(&mut c, "mesimon-probe-72 worker");
    let kw2 = key_of(&mut c, w2);
    start_agent(&mut c, &kw2);
    let ws2 = c.board().live_agent(w2).expect("W2 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let h2 = commit(&h.repo, "w1b.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(5), "W1's delivery to be owed", || {
        wake_rows(&mut c, a).len() == 1
    });
    let h3 = commit(&h.repo, "w2.txt");
    start(&mut c, ws2);
    stop(&mut c, ws2);
    let column2 = c.board().ticket(w2).unwrap().column.clone();
    // W1's column is what the crown last heard, so only the commit is news;
    // W2 has told the crown nothing yet.
    let both = format!(
        "{kw1} \"mesimon-probe-71 worker\" delivered (commit {h2}); {kw2} \"mesimon-probe-72 \
         worker\" delivered (commit {h3}, column {column2}) ∙ get_ticket key={kw1}, {kw2} for \
         state and notes"
    );
    wait_until(std::time::Duration::from_secs(5), "both deliveries in one row", || {
        wake_rows(&mut c, a).first().and_then(|r| r.text.clone()).as_deref() == Some(&both)
    });
    let rows = wake_rows(&mut c, a);
    assert_eq!(rows.len(), 1, "one row for both deliveries: {rows:?}");
    assert_eq!(rows[0].waits_on, vec![ka.clone()], "it waits on the crown's own turn");
    assert!(rows[0].by.is_none());
    settle();
    assert_eq!(lines_with("mesimon-probe-72"), 0, "a working crown is not interrupted");
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the coalesced wake to land", || {
        lines_with(&both) == 1
    });
    assert!(wake_rows(&mut c, a).is_empty());

    // ---- a hand: the sentence says a hand went up, never why ----------------
    start(&mut c, sa);
    match c.send(
        Principal::Agent { session: ws1 },
        Command::AgentRaiseHand { reason: "mesimon-secret-73 need the sandbox key".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    let rows = wake_rows(&mut c, a);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let raised = format!(
        "{kw1} \"mesimon-probe-71 worker\" raised its hand ∙ get_ticket key={kw1} for state and \
         notes"
    );
    assert_eq!(rows[0].text.as_deref(), Some(raised.as_str()));
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the hand's wake to land", || {
        lines_with(&raised) == 1
    });
    assert_eq!(lines_with("mesimon-secret-73"), 0, "the reason never reaches the crown's pane");
    assert!(matches!(c.request(Command::LowerHand { id: w1 }), Response::Ok));

    // ---- a person's ask on the crown goes first; the wake follows ----------
    start(&mut c, sa);
    match c.request(Command::PromptSession {
        ticket: a,
        text: "mesimon-probe-74 person".into(),
        queued: true,
        accept_plan: false,
        plan: false,
        tier: None,
    }) {
        Response::Queued { .. } => {}
        other => panic!("queue a person's ask: {other:?}"),
    }
    commit(&h.repo, "w1c.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(5), "W1's delivery to be owed", || {
        wake_rows(&mut c, a).len() == 1
    });
    let before = lines_with(&delivered1);
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the person's words to land", || {
        lines_with("mesimon-probe-74 person") == 1
    });
    settle();
    assert_eq!(lines_with(&delivered1), before, "the wake waits for the crown's next turn");
    let rows = wake_rows(&mut c, a);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].waits_on, vec![ka.clone()]);
    // The crown takes the person's turn (the ack) and ends it: now the wake.
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the wake after the person's ask", || {
        lines_with(&delivered1) == before + 1
    });

    // ---- sleep_agent: the crown parks what it started, then archives -------
    // (T-539) Its own ticket is refused; a working worker reads the words a
    // person's `z` would; the archive over that awake seat names the road;
    // an idle crown-started worker parks — the record Sleeping, `started_by`
    // kept, the card lit `♛ parked`, the feed line with the agent as actor
    // — and the park alone frees its budget seat (T-541): the start the
    // full budget refused goes through. The archive then goes through too.
    let sleep = |c: &mut TestClient, key: &str, seen: Option<String>| {
        c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: key.into(), seen },
        )
    };
    let va = read(&mut c, sa, &ka).unwrap();
    match sleep(&mut c, &ka, va.seen) {
        Response::Err { message } => assert!(message.contains("own ticket"), "{message}"),
        other => panic!("sleeping the crown's own ticket: {other:?}"),
    }
    start(&mut c, ws2);
    let v2 = read(&mut c, sa, &kw2).unwrap();
    match sleep(&mut c, &kw2, v2.seen) {
        Response::Err { message } => {
            assert!(message.contains("still awake"), "{message}");
            assert!(message.contains("only idle sessions sleep"), "{message}");
        }
        other => panic!("sleeping a working worker: {other:?}"),
    }
    let v2 = read(&mut c, sa, &kw2).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kw2.clone(), restore: false, seen: v2.seen },
    ) {
        Response::Err { message } => {
            assert!(message.contains("sleep_agent parks it, then archive_ticket"), "{message}");
            assert!(message.contains("working"), "{message}");
        }
        other => panic!("archiving over an awake crown-started seat: {other:?}"),
    }
    assert_eq!(c.board().live_agent(w2).unwrap().state, SessionState::Running, "untouched");
    stop(&mut c, ws2);
    // A budget of two is spent by W1 and W2, both awake.
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    let w3 = create(&mut c, "mesimon-probe-73 worker");
    let kw3 = key_of(&mut c, w3);
    let start_w3 = |c: &mut TestClient| {
        let v = read(c, sa, &kw3).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket { key: kw3.clone(), seen: v.seen, plan: false },
        )
    };
    match start_w3(&mut c) {
        Response::Err { message } => {
            for word in ["2 of 2", &kw1, &kw2, "sleep_agent"] {
                assert!(message.contains(word), "the refusal names {word}: {message}");
            }
        }
        other => panic!("a start past the budget: {other:?}"),
    }
    let v2 = read(&mut c, sa, &kw2).unwrap();
    match sleep(&mut c, &kw2, v2.seen) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.key, kw2);
            assert_eq!(ticket.state.as_ref().map(|s| s.state.as_str()), Some("sleeping"));
        }
        other => panic!("sleep_agent: {other:?}"),
    }
    let rec = c.board().sessions.iter().find(|s| s.id == ws2).cloned().expect("W2's record");
    assert_eq!(rec.state, SessionState::Sleeping, "parked, not killed");
    assert_eq!(rec.started_by, Some(a), "provenance survives the park");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w2).map(|t| t.action.as_str()),
        Some("parked"),
        "the card lights"
    );
    wait_until(std::time::Duration::from_secs(5), "the sleep_agent feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains("\"cmd\":\"sleep_agent\"")
                    && l.contains("\"actor\":\"agent\"")
                    && l.contains(&format!("\"ticket\":\"{w2}\""))
            })
        })
    });
    let sessions = std::fs::read_to_string(h.paths.state_dir.join("sessions.json")).unwrap();
    assert!(sessions.contains("\"sleeping\""), "the park is persisted: {sessions}");
    let v2 = read(&mut c, sa, &kw2).unwrap();
    match sleep(&mut c, &kw2, v2.seen) {
        Response::Err { message } => assert!(message.contains("already asleep"), "{message}"),
        other => panic!("sleeping a parked worker: {other:?}"),
    }
    // Parked is freed: the seat W2 held takes W3's start, with no archive.
    assert!(!c.board().crown_started().iter().any(|s| s.ticket == w2), "a parked seat is free");
    match start_w3(&mut c) {
        Response::AgentStarted { budget_left, .. } => assert_eq!(budget_left, 0),
        other => panic!("the start after the park: {other:?}"),
    }
    assert_eq!(c.board().live_agent(w3).unwrap().started_by, Some(a));
    let v2 = read(&mut c, sa, &kw2).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kw2.clone(), restore: false, seen: v2.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("archive after the park: {other:?}"),
    }
    assert!(c.board().ticket(w2).unwrap().is_archived());
    assert!(!c.board().crown_started().iter().any(|s| s.ticket == w2), "the seat stays free");
    assert!(
        c.board().sessions.iter().any(|s| s.id == ws2 && s.state == SessionState::Sleeping),
        "archive keeps the sleeping record"
    );

    // ---- uncrown drops what was owed; an uncrowned board owes nothing -----
    start(&mut c, sa);
    commit(&h.repo, "w1d.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(5), "W1's delivery to be owed", || {
        wake_rows(&mut c, a).len() == 1
    });
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(wake_rows(&mut c, a).is_empty(), "uncrown cancels the pending wake");
    wait_until(std::time::Duration::from_secs(5), "the crown_wake_dropped feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|f| f.contains("\"crown_wake_dropped\""))
    });
    let before = lines_with(&delivered1);
    stop(&mut c, sa);
    commit(&h.repo, "w1e.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    settle();
    assert!(wake_rows(&mut c, a).is_empty(), "an uncrowned board queues nothing");
    assert_eq!(lines_with(&delivered1), before, "nothing lands on an uncrowned board");
}

/// One ticket landing is three wakes, not four (T-469, seen on T-461; T-527):
/// the delivery, the rebase the crown itself asked for coming back as its
/// answer, and the merge — each with what changed. The merged notice's turn
/// is the person's; the crown is not woken for it.
#[test]
fn one_landing_wakes_the_crown_for_the_delivery_and_its_own_ask() {
    let Some(h) = Harness::boot_with_env(
        "crown_landing",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_landing");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-81 landing");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);

    // The crown starts the worker; its worktree is cut first.
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket { key: kw.clone(), seen: v.seen, plan: false },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let path = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kw} \"mesimon-probe-81 landing\"");

    // ---- 1. the delivery: work on the branch, and main moved past it -------
    commit(&path, "work.txt");
    commit(&h.repo, "elsewhere.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    let column = c.board().ticket(w).unwrap().column.clone();
    // The crown's box held its title unsent, so its first line leads with it.
    let delivered = lines_with(&worker).pop().unwrap();
    assert!(
        delivered.ends_with(&format!(
            "{worker} delivered (merge_state needs_rebase, column {column}) ∙ get_ticket \
             key={kw} for state and notes"
        )),
        "{delivered}"
    );
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 2. the crown asks for the rebase; a person sends it ----------------
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-82 rebase onto main".into(),
            seen: v.seen,
            plan: false,
        },
    ) {
        Response::AgentAsked { .. } => {}
        other => panic!("ask_agent: {other:?}"),
    }
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: w }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the ask to reach the worker", || {
        lines_with("mesimon-probe-82").len() == 1
    });
    start(&mut c, ws);
    git(&path, &["rebase", "-q", "main"]);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the answer's wake", || {
        lines_with(&worker).len() == 2
    });
    let answer = lines_with(&worker).pop().unwrap();
    assert!(
        answer
            .starts_with(&format!("{worker} answered your ask (merge_state needs_rebase → ahead")),
        "{answer}"
    );
    assert!(answer.ends_with(&format!("∙ get_ticket key={kw} for state and notes")), "{answer}");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 3. the person merges: the crown hears it once (T-527); the notice
    //         that tells the agent, and its turn, are the person's ----------
    match c.request(Command::MergeTicket { id: w }) {
        Response::Merge { outcome: mesimon_core::command::MergeOutcome::Merged, .. } => {}
        other => panic!("merge: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 3
    });
    let merged = lines_with(&worker).pop().unwrap();
    assert_eq!(
        merged,
        format!(
            "{worker} merged (merge_state ahead → merged) ∙ get_ticket key={kw} for state and \
             notes"
        )
    );
    start(&mut c, sa);
    stop(&mut c, sa);
    assert!(matches!(
        c.request(Command::MergeToAgent {
            id: w,
            request: mesimon_core::command::MergeRequest::MergedNotice
        }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(10), "the merged notice to reach the worker", || {
        lines_with("has been merged").len() == 1
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    // And one more idle turn after it, with nothing new.
    start(&mut c, ws);
    stop(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(
        lines_with(&worker).len(),
        3,
        "the crown heard the delivery, its answer and the merge, nothing else:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
    let owed: Vec<_> = pending_of(&mut c, Some(a))
        .into_iter()
        .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
        .collect();
    assert!(owed.is_empty(), "{owed:?}");

    // ---- 4. new work is a new delivery; a person's rebase ask is a step ----
    commit(&path, "more.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the second delivery's wake", || {
        lines_with(&worker).len() == 4
    });
    assert!(lines_with(&worker)[3].starts_with(&format!("{worker} delivered")));
    start(&mut c, sa);
    stop(&mut c, sa);
    commit(&h.repo, "elsewhere-again.txt");
    assert!(matches!(
        c.request(Command::MergeToAgent {
            id: w,
            request: mesimon_core::command::MergeRequest::Rebase
        }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(10), "the rebase ask to reach the worker", || {
        lines_with("Rebase your current branch").len() == 1
    });
    start(&mut c, ws);
    git(&path, &["rebase", "-q", "main"]);
    stop(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(
        lines_with(&worker).len(),
        4,
        "the rebase the person asked for is theirs, at a new tip or not:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
}

/// A worker's merge wakes the crown that started it (T-527), whoever made
/// it: here a `git merge --ff-only` in a terminal, which no mesimon road
/// sees — only the worktree flags' refresh reads the branch as merged. One
/// line, once; and a delivery merged before the crown heard of it is the
/// delivery's line with the merge in its delta, not two.
#[test]
fn a_hand_merge_wakes_the_crown_that_started_the_worker() {
    // The flags every second rather than every ten.
    let Some(h) = Harness::boot_with_env(
        "crown_merge",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_merge");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-91 merged by hand");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket { key: kw.clone(), seen: v.seen, plan: false },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let path = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    let branch = read(&mut c, sa, &kw).unwrap().branch.expect("a branch");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kw} \"mesimon-probe-91 merged by hand\"");
    let wait_ahead = |c: &mut TestClient| {
        wait_until(
            std::time::Duration::from_secs(10),
            "the flags to read the branch ahead",
            || read(c, sa, &kw).unwrap().merge_state.as_deref() == Some("ahead"),
        );
    };

    // ---- 1. the delivery, heard -------------------------------------------
    commit(&path, "work.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_ahead(&mut c);

    // ---- 2. merged in a terminal: one line, the merge ---------------------
    git(&h.repo, &["merge", "--ff-only", "-q", &branch]);
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 2
    });
    assert_eq!(
        lines_with(&worker)[1],
        format!(
            "{worker} merged (merge_state ahead → merged) ∙ get_ticket key={kw} for state and \
             notes"
        )
    );
    wait_until(std::time::Duration::from_secs(5), "the merged feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"merged\"")
            })
        })
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    // More refreshes at `merged`, and a turn that leaves it there: silent.
    start(&mut c, ws);
    stop(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert_eq!(
        lines_with(&worker).len(),
        2,
        "one wake per merge:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );

    // ---- 3. delivered and merged while the crown works: one line ----------
    start(&mut c, sa);
    commit(&path, "more.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    let owed = |c: &mut TestClient| -> Vec<Option<String>> {
        pending_of(c, Some(a))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .map(|p| p.text)
            .collect()
    };
    wait_until(std::time::Duration::from_secs(10), "the delivery to be owed", || {
        owed(&mut c).len() == 1
    });
    wait_ahead(&mut c);
    git(&h.repo, &["merge", "--ff-only", "-q", &branch]);
    wait_until(std::time::Duration::from_secs(10), "the merge to join the delivery", || {
        owed(&mut c)
            .first()
            .cloned()
            .flatten()
            .is_some_and(|t| t.starts_with(&format!("{worker} delivered (merge_state merged")))
    });
    assert_eq!(owed(&mut c).len(), 1, "one row, not two");
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the one line to land", || {
        lines_with(&worker).len() == 3
    });
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let lines = lines_with(&worker);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[2].starts_with(&format!("{worker} delivered (merge_state merged")), "{lines:?}");
}
