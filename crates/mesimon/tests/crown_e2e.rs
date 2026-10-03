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
//! and the park alone frees that agent's budget seat (T-541). The crown
//! archives only on a board whose person turned Settings → Agents → Crown
//! archives tickets on (T-590); otherwise it closes a landed ticket by
//! moving it to DONE. The crown
//! names a workspace on every start and every filing (T-583): an unstarted
//! ticket takes it, a worktree or a parked agent keeps its own, the shared
//! checkout is refused while another ticket's agent holds it, and a start
//! on a worker the crown parked is that worker's wake.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::{SessionKind, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AgentTicketView, AskRoad, Command, CrownTouch, Deliver, Response};
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
    assert_eq!(t[0].from, Some(a), "the touch names whose agent did it: the bolt's start");

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

    // ---- the crown archives only where a person lets it (T-590) --------------
    // Off on a fresh board: archive and restore alike are refused in words
    // naming the row, before anything changes and with no feed line.
    assert!(!c.board().crown_archives, "off by default");
    let dv = read(&mut c, sa, &kd).unwrap();
    for (restore, says) in [
        (false, format!("move {kd} to DONE instead, or a person archives")),
        (true, format!("a person restores {kd}")),
    ] {
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentArchiveTicket { key: kd.clone(), restore, seen: dv.seen.clone() },
        ) {
            Response::Err { message } => assert_eq!(
                message,
                format!("Settings → Agents → Crown archives tickets is off; {says}")
            ),
            other => panic!("the crown's archive while the row is off: {other:?}"),
        }
    }
    assert!(!c.board().ticket(d).unwrap().is_archived());
    assert_eq!(read(&mut c, sa, &kd).unwrap().seen, dv.seen, "nothing changed");
    // A person turns the row on; the board and doctor read it on.
    assert!(matches!(c.request(Command::SetCrownArchives { on: true }), Response::Ok));
    assert!(c.board().crown_archives);
    assert!(mesimon_daemon::store::read_columns_scalars(&h.paths).crown_archives);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_archives = true"), "{file}");

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
    // The refusals wrote nothing to the feed: the archive and the restore
    // that landed are its only two lines of the crown's.
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let crown_lines = |cmd: &str| {
        std::fs::read_to_string(&feed_path).map_or(0, |feed| {
            feed.lines()
                .filter(|l| {
                    l.contains(&format!("\"cmd\":\"{cmd}\"")) && l.contains("\"actor\":\"agent\"")
                })
                .count()
        })
    };
    wait_until(std::time::Duration::from_secs(5), "the restore's feed line", || {
        crown_lines("unarchive_ticket") == 1
    });
    assert_eq!(crown_lines("archive_ticket"), 1, "the refused calls left no line");
    // Turned off mid-session, the next call is refused at once.
    assert!(matches!(c.request(Command::SetCrownArchives { on: false }), Response::Ok));
    let dv = read(&mut c, sa, &kd).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentArchiveTicket { key: kd.clone(), restore: false, seen: dv.seen },
    ) {
        Response::Err { message } => {
            assert!(message.contains("Crown archives tickets is off"), "{message}")
        }
        other => panic!("the crown's archive after the row went off: {other:?}"),
    }
    assert!(!c.board().ticket(d).unwrap().is_archived());

    // ---- start_agent: the crown starts work, behind the spawn budget --------
    // The budget is a board scalar a person sets; two here so the cap is
    // reached on the second start.
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    assert_eq!(c.board().crown_budget, 2);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_budget = 2"), "{file}");
    // A repository, so a start can cut a worktree (T-583: B, a person's
    // agent, already works on the checkout).
    init_repo(&h.repo, "a.txt", "hello\n");
    let e1 = create(&mut c, "first start");
    let e2 = create(&mut c, "second start");
    let e3 = create(&mut c, "third start");
    let (k1, k2, k3) = (key_of(&mut c, e1), key_of(&mut c, e2), key_of(&mut c, e3));
    let start_in = |c: &mut TestClient, key: &str, seen: Option<String>, workspace: &str| {
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: key.into(),
                seen,
                plan: false,
                tier: None,
                workspace: Some(workspace.into()),
            },
        )
    };
    let start =
        |c: &mut TestClient, key: &str, seen: Option<String>| start_in(c, key, seen, "worktree");
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
    // T-583: the workspace is the crown's to name, in one of two words, and
    // the shared checkout is refused while another ticket's agent works
    // there (B, a person's; the crown's own A coordinates and holds none).
    let v1 = read(&mut c, sa, &k1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: k1.clone(),
            seen: v1.seen.clone(),
            plan: false,
            tier: None,
            workspace: None,
        },
    ) {
        Response::Err { message } => assert!(message.contains("needs workspace"), "{message}"),
        other => panic!("a start with no workspace: {other:?}"),
    }
    match start_in(&mut c, &k1, v1.seen.clone(), "elsewhere") {
        Response::Err { message } => {
            assert!(message.contains("worktree or shared_checkout"), "{message}")
        }
        other => panic!("an unknown workspace word: {other:?}"),
    }
    match start_in(&mut c, &k1, v1.seen.clone(), "shared_checkout") {
        Response::Err { message } => {
            assert!(message.contains(&format!("{kb} (")), "names the holder: {message}");
            assert!(message.contains("works on this checkout; use worktree, or wait"), "{message}");
            assert!(!message.contains(&ka), "the crown is no holder: {message}");
        }
        other => panic!("a second agent on a held checkout: {other:?}"),
    }
    match c.send(Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => assert_eq!(board.checkout_held_by, vec![kb.clone()]),
        other => panic!("list_board: {other:?}"),
    }
    assert!(c.board().live_agent(e1).is_none(), "a refused start starts nothing");
    assert_eq!(c.board().ticket(e1).unwrap().workspace, None, "and sets nothing");
    // The start: the ticket takes the crown's workspace, a worktree is cut,
    // and the parked start replays as a launch on the card; the record
    // carries the crown's id, the touch says so, and the receipt says what
    // is left.
    match start(&mut c, &k1, v1.seen) {
        Response::AgentStarted { key, session_started, budget_left, woken, workspace, .. } => {
            assert_eq!(key, k1);
            assert!(!session_started, "waiting for its worktree first");
            assert!(!woken);
            assert_eq!(workspace, "worktree");
            assert_eq!(budget_left, 1);
        }
        other => panic!("the first start: {other:?}"),
    }
    assert_eq!(
        c.board().ticket(e1).unwrap().workspace,
        Some(WorkspaceStrategy::Worktree),
        "the crown's choice is the ticket's"
    );
    wait_until(std::time::Duration::from_secs(15), "E1's worktree start to land", || {
        c.board().live_agent(e1).is_some()
    });
    let started = c.board().live_agent(e1).cloned().expect("E1 holds a seat now");
    assert_eq!(started.started_by, Some(a), "the record names the crown's ticket");
    assert_eq!(started.kind, SessionKind::Claude);
    assert_ne!(started.cwd, h.repo.to_string_lossy(), "it runs in its worktree");
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
    assert_eq!(c.board().ticket(e3).unwrap().workspace, None, "a refused start sets nothing");
    // A seat frees when its agent exits (or sleeps: the wake test, T-541).
    wait_until(std::time::Duration::from_secs(15), "E2's worktree start to land", || {
        c.board().live_agent(e2).is_some()
    });
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
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 3 }), Response::Ok));
    wait_until(std::time::Duration::from_secs(15), "E3's worktree start to land", || {
        c.board().live_agent(e3).is_some()
    });
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
    // A ticket with a worktree starts there: the other word is refused in
    // words naming what it has, before the budget is asked. E2's agent
    // exited; its worktree stands.
    let v2 = read(&mut c, sa, &k2).unwrap();
    match start_in(&mut c, &k2, v2.seen, "shared_checkout") {
        Response::Err { message } => assert_eq!(
            message,
            format!("{k2} has a worktree; start it there (workspace worktree) or archive it")
        ),
        other => panic!("the checkout on a worktree ticket: {other:?}"),
    }
    // The feed says the agent started it (buffered; flushed on a later tick).
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
                Command::AgentAskTicket {
                    key: key.into(),
                    text: text.into(),
                    seen,
                    plan: false,
                    deliver: Deliver::Idle,
                },
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
        Response::AgentAsked { key, replaced, seen, held_for_person, held_because, road } => {
            assert_eq!(key, kb);
            assert!(!replaced);
            // T-550: off by default, so the board holds it and says so.
            assert!(held_for_person);
            assert_eq!(road, Some(mesimon_core::command::AskRoad::HeldForPerson));
            assert!(held_because.is_some_and(|w| w.contains("Crown sends its asks")));
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
    // T-568: the crown reads that its words were dropped, and by whom; the
    // worker reading its own ticket reads nothing of the crown's.
    let dropped = |c: &mut TestClient| {
        read(c, sa, &kb).unwrap().asked.map(|a| (a.status, a.by, a.since_secs.is_some()))
    };
    assert_eq!(dropped(&mut c), Some(("dropped".into(), "person".into(), true)));
    match c.send(Principal::Agent { session: sb }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => assert!(ticket.asked.is_none(), "{:?}", ticket.asked),
        other => panic!("the worker's own read: {other:?}"),
    }
    // The crown's next ask supersedes it; a person's words queued over that
    // ask drop it again, and say so.
    let bv = read(&mut c, sa, &kb).unwrap();
    match ask(&mut c, sa, &kb, "mesimon-probe-63b once more", bv.seen) {
        Response::AgentAsked { held_for_person, .. } => assert!(held_for_person),
        other => panic!("the re-ask: {other:?}"),
    }
    assert_eq!(dropped(&mut c), None, "a new ask clears the dropped one");
    // A person's own queued ask is never overwritten by the crown's.
    match c.request(Command::PromptSession {
        ticket: b,
        text: "mesimon-probe-64 the person's".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { .. } | Response::Ok => {}
        other => panic!("the person's queued ask: {other:?}"),
    }
    assert_eq!(dropped(&mut c), Some(("dropped".into(), "person".into(), true)));
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
    assert_eq!(dropped(&mut c), None, "the third ask clears the record");
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

    // ---- a crowned filing is a touch the board strikes (T-544) ----------------
    let file_in = |c: &mut TestClient, title: &str, workspace: Option<&str>| {
        c.send(
            Principal::Agent { session: sa },
            Command::AgentCreateTicket {
                title: title.into(),
                column: None,
                description: None,
                tags: Vec::new(),
                idempotency_key: None,
                tier: None,
                workspace: workspace.map(str::to_string),
            },
        )
    };
    let filed = |c: &mut TestClient, title: &str, workspace: Option<&str>| {
        match file_in(c, title, workspace) {
            Response::AgentCreated { workspace: echoed, .. } => {
                assert_eq!(echoed, workspace.unwrap_or("shared_checkout"), "the receipt echoes it")
            }
            other => panic!("create_ticket: {other:?}"),
        }
        c.board().tickets.iter().find(|t| t.title == title).expect("filed").id
    };
    // T-583: the crown files a ticket with its workspace decided.
    match file_in(&mut c, "filed without a workspace", None) {
        Response::Err { message } => {
            assert!(message.contains("the crown files a ticket with its workspace decided"))
        }
        other => panic!("a crowned filing with no workspace: {other:?}"),
    }
    assert!(!c.board().tickets.iter().any(|t| t.title == "filed without a workspace"));
    let crowned_filing = filed(&mut c, "filed by the crown", Some("worktree"));
    assert_eq!(
        c.board().ticket(crowned_filing).unwrap().workspace,
        Some(WorkspaceStrategy::Worktree)
    );
    let touch = touches(&mut c).into_iter().find(|t| t.ticket == crowned_filing);
    let touch = touch.expect("the crown's filing is a touch");
    assert_eq!((touch.action.as_str(), touch.from), ("created", Some(a)));

    // ---- the shim: seventeen tools, and `get_ticket` with a key --------------
    let mut shim = Shim::start(&sock, sa);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    let listed = shim.rpc("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 17);
    let r = shim.call("answer_agent", json!({ "key": kb, "seen": "x", "index": 0 }));
    assert_eq!(r["isError"], true, "request is required by the tool: {r}");
    let r = shim.call("accept_plan", json!({ "key": kb, "seen": "x" }));
    assert_eq!(r["isError"], true, "request is required by the tool: {r}");
    let r = shim.call("sleep_agent", json!({ "key": kb }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("ask_agent", json!({ "key": kb, "text": "x" }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("start_agent", json!({ "key": k4 }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("start_agent", json!({ "key": k4, "seen": "x" }));
    assert_eq!(r["isError"], true, "workspace is required by the tool: {r}");
    assert!(r.to_string().contains("workspace is required"), "{r}");

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
    // Any agent may file a ticket; only the crown's filing is a touch.
    let plain_filing = filed(&mut c, "filed uncrowned", None);
    assert!(!touches(&mut c).iter().any(|t| t.ticket == plain_filing), "not the crown's");
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

    let start_in = |c: &mut TestClient, key: &str, workspace: &str| {
        let v = read(c, sa, key).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: key.into(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some(workspace.into()),
            },
        )
    };
    match start_in(&mut c, &kw1, "shared_checkout") {
        Response::AgentStarted { session_started: true, workspace, .. } => {
            assert_eq!(workspace, "shared_checkout")
        }
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));

    let crown_wake_fed = |cause: &str| {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"crown\":\"{a}\""))
                    && l.contains(&format!("\"worker\":\"{w1}\""))
                    && l.contains(&format!("\"cause\":\"{cause}\""))
            })
        })
    };

    // A turn that leaves nothing new, with nothing pending on W1, finished
    // (T-591): W1 is done with what it was asked, and the crown hears it —
    // one sentence, mesimon's template over the worker's key and title and
    // what changed, in the crown's pane. A checkout's HEAD is not named:
    // the turn did not move it.
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let finished1 = format!("{kw1} \"mesimon-probe-71 worker\" finished its turn");
    wait_until(std::time::Duration::from_secs(10), "the finish to land on the crown", || {
        lines_with(&finished1) == 1
    });
    let column = c.board().ticket(w1).unwrap().column.clone();
    let sentence = format!(
        "{finished1} (nothing new to merge, column {column}) ∙ get_ticket key={kw1} for state \
         and notes"
    );
    assert_eq!(lines_with(&sentence), 1, "{}", std::fs::read_to_string(&got).unwrap());
    assert!(wake_rows(&mut c, a).is_empty(), "delivered, so nothing is owed");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == a).map(|t| t.action.as_str()),
        Some("woke"),
        "the crown's card lights"
    );
    wait_until(std::time::Duration::from_secs(5), "the finished feed line", || {
        crown_wake_fed("finished")
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // The same idle re-entered with no turn between is no second finish: a
    // `/clear`'s `SessionStart` in the living pane, then a `Stop`.
    hook_send(&hook_sock, &ws1.to_string(), "SessionStart", r#"{"source":"clear"}"#);
    c.await_state(ws1, "idle, unknown", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown }
    });
    stop(&mut c, ws1);
    settle();
    assert!(wake_rows(&mut c, a).is_empty());
    assert_eq!(lines_with(&finished1), 1, "one finish per turn");

    // A commit, then the turn ends: it delivered. The column is what the
    // finish already said, so only the commit is news.
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered1 = format!("{kw1} \"mesimon-probe-71 worker\" delivered");
    wait_until(std::time::Duration::from_secs(10), "the wake to land on the crown", || {
        lines_with(&delivered1) == 1
    });
    assert_eq!(c.board().ticket(w1).unwrap().column, column);
    let sentence = format!("{delivered1} (commit {h1}) ∙ get_ticket key={kw1} for state and notes");
    assert_eq!(lines_with(&sentence), 1, "{}", std::fs::read_to_string(&got).unwrap());
    assert!(wake_rows(&mut c, a).is_empty(), "delivered, so nothing is owed");
    // The feed names both tickets and the cause, never the sentence.
    wait_until(std::time::Duration::from_secs(5), "the crown_wake feed line", || {
        crown_wake_fed("delivered")
    });
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-71"), "the feed never carries the words:\n{feed}");
    assert!(feed.contains("\"crown_wake_sent\""), "{feed}");
    // The crown takes its turn on it (the paste's ack).
    start(&mut c, sa);
    stop(&mut c, sa);

    // A second `Stop` on an idle worker is no edge; a whole second turn at
    // the same HEAD delivers nothing — once per delivery — and finished.
    hook_send(&hook_sock, &ws1.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    settle();
    assert_eq!(lines_with(&finished1), 1, "a second Stop is no edge");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let again =
        format!("{finished1} (nothing new to merge) ∙ get_ticket key={kw1} for state and notes");
    wait_until(std::time::Duration::from_secs(10), "the second finish", || lines_with(&again) == 1);
    assert_eq!(lines_with(&delivered1), 1, "a second idle with nothing new delivers nothing");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- words queued on the worker: pending, so its turn end is silent ---
    // A person's ask queued at W1's pane while it works goes when the turn
    // ends; that end is not news, and the turn the words run is judged on
    // its own end.
    start(&mut c, ws1);
    match c.request(Command::PromptSession {
        ticket: w1,
        text: "mesimon-probe-75 person".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { .. } => {}
        other => panic!("queue a person's ask on W1: {other:?}"),
    }
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(10), "the person's words to reach W1", || {
        std::fs::read_to_string(&got).unwrap_or_default().contains("mesimon-probe-75 person")
    });
    settle();
    assert_eq!(lines_with(&finished1), 2, "a turn with words queued behind it is silent");
    assert!(wake_rows(&mut c, a).is_empty());
    start(&mut c, ws1);
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(10), "the queued turn's finish", || {
        lines_with(&finished1) == 3
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- two deliveries while the crown works: one row, one sentence ------
    start(&mut c, sa);
    let w2 = create(&mut c, "mesimon-probe-72 worker");
    let kw2 = key_of(&mut c, w2);
    // T-583: a second crown worker on the checkout W1 holds is refused, by
    // name; W2 works in a worktree of its own instead.
    match start_in(&mut c, &kw2, "shared_checkout") {
        Response::Err { message } => assert_eq!(
            message,
            format!("{kw1} (idle) works on this checkout; use worktree, or wait")
        ),
        other => panic!("a second worker on W1's checkout: {other:?}"),
    }
    match start_in(&mut c, &kw2, "worktree") {
        Response::AgentStarted { session_started: false, workspace, .. } => {
            assert_eq!(workspace, "worktree")
        }
        other => panic!("start_agent {kw2}: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "W2's worktree start to land", || {
        c.board().live_agent(w2).is_some()
    });
    let ws2 = c.board().live_agent(w2).expect("W2 holds a seat").id;
    let tree2 = std::path::PathBuf::from(c.board().live_agent(w2).unwrap().cwd.clone());
    assert_ne!(tree2, h.repo, "W2 runs in its worktree");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let h2 = commit(&h.repo, "w1b.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    wait_until(std::time::Duration::from_secs(5), "W1's delivery to be owed", || {
        wake_rows(&mut c, a).len() == 1
    });
    commit(&tree2, "w2.txt");
    start(&mut c, ws2);
    stop(&mut c, ws2);
    let column2 = c.board().ticket(w2).unwrap().column.clone();
    // W1's column is what the crown last heard, so only the commit is news;
    // W2 has told the crown nothing yet, and its branch says it in its merge
    // state (behind W1's commit on the base, whichever word that reads).
    let head = format!(
        "{kw1} \"mesimon-probe-71 worker\" delivered (commit {h2}); {kw2} \"mesimon-probe-72 \
         worker\" delivered (merge_state "
    );
    let tail = format!(", column {column2}) ∙ get_ticket key={kw1}, {kw2} for state and notes");
    let mut both = String::new();
    wait_until(std::time::Duration::from_secs(10), "both deliveries in one row", || {
        let text = wake_rows(&mut c, a).first().and_then(|r| r.text.clone()).unwrap_or_default();
        let whole = text.starts_with(&head) && text.ends_with(&tail);
        if whole {
            both = text;
        }
        whole
    });
    let rows = wake_rows(&mut c, a);
    assert_eq!(rows.len(), 1, "one row for both deliveries: {rows:?}");
    assert_eq!(rows[0].waits_on, vec![ka.clone()], "it waits on the crown's own turn");
    assert!(rows[0].by.is_none());
    settle();
    // The wake's words, not W2's own title: on the mod road W2's launch
    // submits its title at once (T-575), into the same `got.txt`.
    let w2_delivered = format!("{kw2} \"mesimon-probe-72 worker\" delivered");
    assert_eq!(lines_with(&w2_delivered), 0, "a working crown is not interrupted");
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
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
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
    // full budget refused goes through. The archive then goes through too,
    // on a board whose person let the crown archive (T-590).
    assert!(matches!(c.request(Command::SetCrownArchives { on: true }), Response::Ok));
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
    // T-583: the crown files W3 with its workspace decided, and starts it
    // there; a start in the other word would be applied, not refused, on a
    // ticket nobody has started.
    let kw3 = match c.send(
        Principal::Agent { session: sa },
        Command::AgentCreateTicket {
            title: "mesimon-probe-73 worker".into(),
            column: None,
            description: None,
            tags: Vec::new(),
            idempotency_key: None,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentCreated { key, workspace, .. } => {
            assert_eq!(workspace, "worktree");
            key
        }
        other => panic!("create_ticket: {other:?}"),
    };
    let w3 = c.board().ticket_by_key(&kw3).unwrap().id;
    let start_w3 = |c: &mut TestClient| start_in(c, &kw3, "worktree");
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
    // ---- the crown wakes what it parked, where it parked it (T-583) --------
    // A wake in the other workspace is refused in words naming its own; a
    // person's parked agent is no crown's to wake (the unit tests); in its
    // worktree, the start is W2's wake: the same record, the conversation
    // kept, the card lit `♛ woken`, the seat taken back.
    match start_in(&mut c, &kw2, "shared_checkout") {
        Response::Err { message } => assert_eq!(
            message,
            format!(
                "{kw2}'s parked agent works in its worktree, and a wake runs it there: \
                 workspace worktree"
            )
        ),
        other => panic!("a wake in the other workspace: {other:?}"),
    }
    match start_in(&mut c, &kw2, "worktree") {
        Response::AgentStarted { key, session_started, woken, workspace, budget_left, .. } => {
            assert_eq!(key, kw2);
            assert!(session_started && woken, "the receipt says woken");
            assert_eq!(workspace, "worktree");
            assert_eq!(budget_left, 0, "the woken seat counts again");
        }
        other => panic!("the crown's wake of its parked worker: {other:?}"),
    }
    let woke = c.board().live_agent(w2).cloned().expect("W2 holds its seat");
    assert_eq!(woke.id, ws2, "the same record, never a second agent");
    assert_eq!(woke.started_by, Some(a), "still the crown's");
    assert_eq!(std::path::PathBuf::from(&woke.cwd), tree2, "in its worktree");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w2).map(|t| t.action.as_str()),
        Some("woken"),
    );
    start(&mut c, ws2);
    stop(&mut c, ws2);
    let v2 = read(&mut c, sa, &kw2).unwrap();
    assert!(matches!(sleep(&mut c, &kw2, v2.seen), Response::AgentTicket { .. }), "parked again");
    match start_w3(&mut c) {
        Response::AgentStarted { session_started, budget_left, .. } => {
            assert!(!session_started, "filed for a worktree, so waiting for it first");
            assert_eq!(budget_left, 0)
        }
        other => panic!("the start after the park: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "W3's worktree start to land", || {
        c.board().live_agent(w3).is_some()
    });
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
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
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
            deliver: Deliver::Idle,
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
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(
        lines_with(&worker).len(),
        3,
        "the crown heard the delivery, its answer and the merge, nothing else:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
    // And one more turn after it, with nothing new and nothing pending: the
    // worker finished (T-591), and what to do with a merged worker is the
    // crown's to decide.
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the finish's wake", || {
        lines_with(&worker).len() == 4
    });
    assert_eq!(
        lines_with(&worker)[3],
        format!(
            "{worker} finished its turn (nothing new to merge) ∙ get_ticket key={kw} for state \
             and notes"
        )
    );
    let owed: Vec<_> = pending_of(&mut c, Some(a))
        .into_iter()
        .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
        .collect();
    assert!(owed.is_empty(), "{owed:?}");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 4. new work is a new delivery; a person's rebase ask is a step ----
    commit(&path, "more.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the second delivery's wake", || {
        lines_with(&worker).len() == 5
    });
    assert!(lines_with(&worker)[4].starts_with(&format!("{worker} delivered")));
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
        5,
        "the rebase the person asked for is theirs, at a new tip or not:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );

    // ---- 5. the crown closes a landed ticket in DONE (T-590) ----------------
    // The road its words name while the archive row is off: the keyed move,
    // refused by the DONE gate while the branch is unmerged and admitted
    // once a person merged it. The worktree stays — its reclaim is the
    // person's, by DONE's offer or an archive.
    assert!(!c.board().crown_archives);
    let to_done = |c: &mut TestClient| {
        let v = read(c, sa, &kw).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentMoveTicket {
                to_column: "DONE".into(),
                idempotency_key: None,
                key: Some(kw.clone()),
                before: None,
                seen: v.seen,
            },
        )
    };
    match to_done(&mut c) {
        Response::Err { message } => {
            assert_eq!(message, "worktree unmerged — merge before DONE")
        }
        other => panic!("DONE before the merge: {other:?}"),
    }
    match c.request(Command::MergeTicket { id: w }) {
        Response::Merge { outcome: mesimon_core::command::MergeOutcome::Merged, .. } => {}
        other => panic!("the second merge: {other:?}"),
    }
    match to_done(&mut c) {
        Response::AgentMoved { column, .. } => assert_eq!(column, "DONE"),
        other => panic!("DONE after the merge: {other:?}"),
    }
    let t = c.board().ticket(w).cloned().unwrap();
    assert_eq!(t.column, "DONE");
    assert!(!t.is_archived(), "on the board until a person archives it");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w).map(|t| t.action.as_str()),
        Some("moved")
    );
    assert!(path.exists(), "the crown reclaims nothing");
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
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
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
    // More refreshes at `merged`: silent.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert_eq!(
        lines_with(&worker).len(),
        2,
        "one wake per merge:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
    // A turn that leaves it there is no second merge; with nothing pending
    // it finished (T-591).
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the finish's wake", || {
        lines_with(&worker).len() == 3
    });
    assert!(lines_with(&worker)[2].starts_with(&format!("{worker} finished its turn")));
    start(&mut c, sa);
    stop(&mut c, sa);

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
        lines_with(&worker).len() == 4
    });
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let lines = lines_with(&worker);
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[3].starts_with(&format!("{worker} delivered (merge_state merged")), "{lines:?}");
}

/// A wake owed across a daemon restart is not lost (T-602): the crown
/// heard a worker's delivery, the daemon went down (`Shutdown`, as a `U`
/// handover sends it), the branch was merged in a terminal while no daemon
/// was looking, and the next daemon wakes the crown once, saying the merge
/// `after a restart`. A restart with nothing new to say is silent.
#[test]
fn a_merge_while_the_daemon_is_down_wakes_the_crown_after_the_restart() {
    let Some(h) = Harness::boot_with_env(
        "crown_restart",
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
    let mut c = h.client("crown_restart");
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
    let w = create(&mut c, "mesimon-probe-92 merged while down");
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
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
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
    let worker = format!("{kw} \"mesimon-probe-92 merged while down\"");

    // ---- 1. the delivery, heard -------------------------------------------
    commit(&path, "work.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the flags to read the branch ahead", || {
        read(&mut c, sa, &kw).unwrap().merge_state.as_deref() == Some("ahead")
    });
    drop(c);

    // ---- 2. merged while no daemon is up: one line, after the restart ----
    h.restart_after(|| {
        git(&h.repo, &["merge", "--ff-only", "-q", &branch]);
    });
    assert!(h.paths.state_dir.join("crown.json").is_file(), "the ledger was kept");
    let mut c = h.client("crown_restart_2");
    wait_until(std::time::Duration::from_secs(15), "the merge's wake", || {
        lines_with(&worker).len() == 2
    });
    assert_eq!(
        lines_with(&worker)[1],
        format!(
            "{worker} merged (merge_state ahead → merged, after a restart) ∙ get_ticket key={kw} \
             for state and notes"
        )
    );
    start(&mut c, sa);
    stop(&mut c, sa);
    // More refreshes at `merged`, the flags' first reading among them:
    // silent.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert_eq!(lines_with(&worker).len(), 2, "{}", std::fs::read_to_string(&got).unwrap());
    drop(c);

    // ---- 3. a restart with nothing new: silent ----------------------------
    h.restart();
    let mut c = h.client("crown_restart_3");
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert_eq!(
        lines_with(&worker).len(),
        2,
        "a wake already heard is silent after a restart:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
    // The worker is still on the board the crown can read.
    assert!(c.board().ticket(w).is_some());
}

/// The crown sends its own asks (T-550), where the person lets it: off by
/// default, so the words wait for `^y` as T-413 built them; on, an ask to an
/// agent the crown STARTED goes by the queue once that agent is idle — the
/// feed's actor is the agent, the card lights `♛ sent`, and the turn that
/// takes the words wakes the crown as a person's send would. An agent a
/// person started is held whatever the switch says, words that would wake a
/// parked agent need a free budget seat and hold one while they wait, and
/// switching off or uncrowning holds what had not gone yet.
#[test]
fn the_crown_sends_its_asks_to_the_agents_it_started() {
    let Some(h) = Harness::boot_with_env(
        "crown_sends",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_sends");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let landed = |probe: &str| std::fs::read_to_string(&got).unwrap_or_default().contains(probe);
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));
    let touch_on = |c: &mut TestClient, t: ulid::Ulid| {
        touches(c).into_iter().find(|x| x.ticket == t).map(|x| x.action)
    };

    let a = create(&mut c, "coordinate");
    let p = create(&mut c, "the person's own");
    let w = create(&mut c, "mesimon-probe-91 worker");
    let w2 = create(&mut c, "second worker");
    let w3 = create(&mut c, "third worker");
    let (kp, kw, kw2, kw3) =
        (key_of(&mut c, p), key_of(&mut c, w), key_of(&mut c, w2), key_of(&mut c, w3));
    let sa = spawn(&mut c, a);
    // The person's own agent works in a worktree, so the crown's W may take
    // the checkout beside the crown (T-583: a start there is refused while
    // another ticket's agent holds it).
    assert!(matches!(
        c.request(Command::SetWorkspace { id: p, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: p,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false,
        }),
        Response::Provisioning
    ));
    wait_until(std::time::Duration::from_secs(15), "P's worktree spawn to land", || {
        c.board().live_agent(p).is_some()
    });
    let sp = c.board().live_agent(p).unwrap().id;
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // The stub emits no `SessionStart`: a turn walked through each pane is
    // what makes it idle, and an idle checkout is what the queue waits for.
    std::thread::sleep(std::time::Duration::from_millis(500));
    for s in [sa, sp] {
        start(&mut c, s);
        stop(&mut c, s);
    }
    let start_agent = |c: &mut TestClient, key: &str, workspace: &str| {
        let v = read(c, sa, key).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: key.into(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some(workspace.into()),
            },
        )
    };
    assert!(matches!(start_agent(&mut c, &kw, "shared_checkout"), Response::AgentStarted { .. }));
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    // W's first turn left nothing new: it finished (T-591). The crown takes
    // that turn, so its pane no longer holds the checkout W shares.
    wait_until(std::time::Duration::from_secs(10), "W's finish on the crown", || {
        landed(&format!("{kw} \"mesimon-probe-91 worker\" finished its turn"))
    });
    assert!(!landed(&format!("{kp} \"the person's own\"")), "a person's agent wakes nobody");
    start(&mut c, sa);
    stop(&mut c, sa);
    let ask = |c: &mut TestClient, key: &str, text: &str| {
        let v = read(c, sa, key).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: key.into(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
            },
        ) {
            Response::AgentAsked { held_for_person, held_because, .. } => {
                (held_for_person, held_because.unwrap_or_default())
            }
            other => panic!("ask_agent {key}: {other:?}"),
        }
    };
    let row = |c: &mut TestClient, t: ulid::Ulid| {
        pending_of(c, Some(t)).into_iter().find(|p| p.is_queued_ask() && !p.in_flight)
    };

    // ---- off, the default: held for a person, even on the crown's own worker
    assert!(!c.board().crown_sends, "off by default");
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-92 held");
    assert!(held && why.contains("Crown sends its asks"), "{why}");
    let r = row(&mut c, w).expect("held on W's card");
    assert!(!r.sends && r.by.is_some() && r.waits_on.is_empty(), "{r:?}");
    settle();
    assert!(!landed("mesimon-probe-92"), "held words do not go on their own");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));

    // ---- on: refused to an agent, persisted, printed ------------------------
    match c.send(Principal::Agent { session: sa }, Command::SetCrownSends { on: true }) {
        Response::Err { .. } => {}
        other => panic!("an agent switching it on: {other:?}"),
    }
    assert!(!c.board().crown_sends);
    assert!(matches!(c.request(Command::SetCrownSends { on: true }), Response::Ok));
    assert!(c.board().crown_sends);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_sends = true"), "{file}");

    // A person's agent waits for its person whatever the switch says.
    let (held, why) = ask(&mut c, &kp, "mesimon-probe-93 for the person's");
    assert!(held && why.contains("a person started"), "{why}");
    settle();
    assert!(!landed("mesimon-probe-93"));
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: p }), Response::Ok));

    // ---- a working worker takes the crown's words when its turn ends --------
    start(&mut c, ws);
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-94 then push");
    assert!(!held && why.is_empty(), "{why}");
    let r = row(&mut c, w).expect("queued on W's card");
    assert!(r.sends && r.by.as_deref() == Some(key_of(&mut c, a).as_str()), "{r:?}");
    assert_eq!(r.waits_on, vec![kw.clone()], "it waits on W's own turn: {r:?}");
    assert_eq!(touch_on(&mut c, w).as_deref(), Some("asked"));
    settle();
    assert!(!landed("mesimon-probe-94"), "a working agent is not interrupted");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the crown's words to reach W", || {
        landed("mesimon-probe-94 then push")
    });
    assert_eq!(touch_on(&mut c, w).as_deref(), Some("sent"), "the card says it went");
    wait_until(std::time::Duration::from_secs(5), "the ask_agent_sent feed line", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines()
                .any(|l| l.contains("\"ask_agent_sent\"") && l.contains("\"actor\":\"agent\""))
        })
    });
    // The turn that takes them is the crown's answer, as after a `^y`.
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the answer's wake on the crown", || {
        landed(&format!("{kw} \"mesimon-probe-91 worker\" answered your ask"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- an idle worker takes them at once ----------------------------------
    let (held, _) = ask(&mut c, &kw, "mesimon-probe-95 at once");
    assert!(!held);
    wait_until(std::time::Duration::from_secs(10), "the words to reach idle W", || {
        landed("mesimon-probe-95 at once")
    });
    assert!(row(&mut c, w).is_none(), "nothing left waiting");
    start(&mut c, ws);
    stop(&mut c, ws);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a worker on its question: the crown's words wait behind it ---------
    // The friend's board: asked while W waits on a person's answer, the
    // words read `after its turn` for a turn only the answer could end.
    // Onto a question already standing the crown's ask is refused (T-566),
    // so it reads the question instead; words it queued before the question
    // are held the moment it comes (T-565), and a person sends them after.
    let question = r#"{"tool_name":"AskUserQuestion"}"#;
    start(&mut c, ws);
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", question);
    c.await_state(ws, "asking", SessionState::question_stop);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-90 over the question".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
        },
    ) {
        Response::Err { message } => assert!(message.contains("asking a question"), "{message}"),
        other => panic!("ask_agent over a standing question: {other:?}"),
    }
    assert!(row(&mut c, w).is_none(), "a refused ask queues nothing");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", question);
    c.await_state(ws, "answered", |s| *s == SessionState::Running);
    let (held, _) = ask(&mut c, &kw, "mesimon-probe-90 after the answer");
    assert!(!held, "the crown's words for its working worker go after the turn");
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", question);
    c.await_state(ws, "asking again", SessionState::question_stop);
    let r = row(&mut c, w).expect("held on W's card");
    assert!(r.is_held() && r.sends && r.held.as_deref() == Some("agent asked"), "{r:?}");
    assert_eq!(r.asking, vec![kw.clone()], "the card says the answer comes first: {r:?}");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", question);
    c.await_state(ws, "answered", |s| *s == SessionState::Running);
    stop(&mut c, ws);
    settle();
    assert!(!landed("mesimon-probe-90"), "held words do not go on the turn's end");
    let r = row(&mut c, w).expect("still held");
    assert!(r.is_held() && r.asking.is_empty(), "{r:?}");
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: w }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the held words, sent by hand", || {
        landed("mesimon-probe-90 after the answer")
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a parked worker: the wake is a budget seat -------------------------
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("sleep_agent: {other:?}"),
    }
    c.await_state(ws, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 1 }), Response::Ok));
    // W, parked on the checkout, still holds it (T-583): W2 takes a worktree.
    match start_agent(&mut c, &kw2, "shared_checkout") {
        Response::Err { message } => assert_eq!(
            message,
            format!("{kw} (sleeping) works on this checkout; use worktree, or wait")
        ),
        other => panic!("a start on the checkout a parked worker holds: {other:?}"),
    }
    assert!(matches!(start_agent(&mut c, &kw2, "worktree"), Response::AgentStarted { .. }));
    wait_until(std::time::Duration::from_secs(15), "W2's worktree start to land", || {
        c.board().live_agent(w2).is_some()
    });
    let ws2 = c.board().live_agent(w2).expect("W2 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws2);
    stop(&mut c, ws2);
    wait_until(std::time::Duration::from_secs(10), "W2's finish on the crown", || {
        landed(&format!("{kw2} \"second worker\" finished its turn"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-96 wake for this");
    assert!(held && why.contains("budget is spent") && why.contains(&kw2), "{why}");
    settle();
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Sleeping, "nobody woke it");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    // Words that would wake W onto a checkout another ticket's agent holds
    // wait for a person (T-583), as a start there would be refused.
    let q = create(&mut c, "the person's on the checkout");
    let kq = key_of(&mut c, q);
    let sq = spawn(&mut c, q);
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-96b wake onto a held checkout");
    assert!(held && why.contains("parked on the shared checkout"), "{why}");
    assert!(why.contains(&format!("{kq} (")) && why.contains("works on this checkout"), "{why}");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));
    let _ = c.request(Command::KillSession { id: sq });
    assert!(c.board().live_agent(q).is_none(), "Q's agent gone, the checkout is free");
    // With a seat free, the words go — and while they wait on the checkout
    // (the crown mid-turn holds it), the wake holds the seat they need.
    start(&mut c, sa);
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-97 wake for this");
    assert!(!held, "{why}");
    let r = row(&mut c, w).expect("the wake waits on the checkout");
    assert!(r.sends && r.action == mesimon_core::command::PendingAction::Wake, "{r:?}");
    match start_agent(&mut c, &kw3, "worktree") {
        Response::Err { message } => {
            assert!(message.contains("budget is spent") && message.contains(&kw), "{message}")
        }
        other => panic!("a start over the waking seat: {other:?}"),
    }
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the crown's ask to wake W", || {
        c.board().live_agent(w).is_some_and(|s| s.state != SessionState::Sleeping)
    });
    assert!(row(&mut c, w).is_none(), "{:?}", pending_of(&mut c, Some(w)));
    assert_eq!(touch_on(&mut c, w).as_deref(), Some("sent"));
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(15), "the parked words to land", || {
        landed("mesimon-probe-97 wake for this")
    });
    start(&mut c, ws);
    stop(&mut c, ws);

    // ---- off again, or the crown leaving, holds what had not gone -----------
    start(&mut c, ws);
    let (held, _) = ask(&mut c, &kw, "mesimon-probe-98 not now");
    assert!(!held);
    assert!(matches!(c.request(Command::SetCrownSends { on: false }), Response::Ok));
    let r = row(&mut c, w).expect("still on the card");
    assert!(!r.sends && r.waits_on.is_empty(), "held for the person now: {r:?}");
    stop(&mut c, ws);
    settle();
    assert!(!landed("mesimon-probe-98"), "switched off, nothing more goes");
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: w }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownSends { on: true }), Response::Ok));
    start(&mut c, ws);
    let (held, _) = ask(&mut c, &kw, "mesimon-probe-99 not now either");
    assert!(!held);
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(!row(&mut c, w).expect("still on the card").sends, "the crown left");
    stop(&mut c, ws);
    settle();
    assert!(!landed("mesimon-probe-99"), "uncrowned, nothing more goes");

    // The words are in no state file.
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-9"), "the feed never carries the words");
    let queue = std::fs::read_to_string(h.paths.queue_file()).unwrap_or_default();
    assert!(!queue.contains("mesimon-probe-9"), "a crown's ask is never persisted");
}

/// The crown's `now` (T-600): the board's own Shift+Enter "now", made on the
/// crown's say-so. With crown sends on, the words reach a WORKING agent the
/// crown started at once — mid-turn, by the paste on the hook set and by the
/// stand-in engine's `submit` on the mod pass — and the turn that takes them
/// wakes the crown at its end, also when their ack outlives the in-flight
/// window. Without `now` they wait for idle as before; `now` at a dialog is
/// refused, as the person's send is; and with sends off the words are held
/// for a person with the send preset to now, which `^y` delivers at once.
#[test]
fn the_crown_sends_now_into_a_working_turn() {
    let Some(h) = Harness::boot_with_env(
        "crown_now",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_now");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| read_got().contains(probe);
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));
    // The feed is flushed on the writer's clock, so a line is waited for.
    let feed_has = |cmd: &str, outcome: Option<&str>| {
        wait_until(std::time::Duration::from_secs(5), &format!("the {cmd} feed line"), || {
            std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
                feed.lines().any(|l| {
                    l.contains(&format!("\"cmd\":\"{cmd}\""))
                        && l.contains("\"actor\":\"agent\"")
                        && outcome.is_none_or(|o| l.contains(&format!("\"outcome\":\"{o}\"")))
                })
            })
        });
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-81 worker");
    let kw = key_of(&mut c, w);
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownSends { on: true }), Response::Ok));
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
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "W's finish on the crown", || {
        landed(&format!("{kw} \"mesimon-probe-81 worker\" finished its turn"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    let answered = format!("{kw} \"mesimon-probe-81 worker\" answered your ask");
    let answers = || read_got().matches(answered.as_str()).count();
    let ask = |c: &mut TestClient, text: &str, deliver: Deliver| {
        let v = read(c, sa, &kw).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver,
            },
        )
    };
    let road_of = |r: Response| match r {
        Response::AgentAsked { held_for_person, road, .. } => (held_for_person, road.unwrap()),
        other => panic!("ask_agent: {other:?}"),
    };
    let row = |c: &mut TestClient| {
        pending_of(c, Some(w)).into_iter().find(|p| p.is_queued_ask() && !p.in_flight)
    };
    // The answer's wake on the crown, then the crown's own turn on it, so
    // the next wake finds it idle.
    let answer_heard = |c: &mut TestClient, before: usize| {
        wait_until(std::time::Duration::from_secs(20), "the answer's wake on the crown", || {
            answers() > before
        });
        start(c, sa);
        stop(c, sa);
    };

    // ---- now, at a working worker: the words land mid-turn ------------------
    start(&mut c, ws);
    let before = answers();
    assert_eq!(
        road_of(ask(&mut c, "mesimon-probe-82 now", Deliver::Now)),
        (false, AskRoad::SentNow)
    );
    wait_until(std::time::Duration::from_secs(5), "the words in W's running turn", || {
        landed("mesimon-probe-82 now")
    });
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "mid-turn");
    assert!(row(&mut c).is_none(), "nothing parked: {:?}", pending_of(&mut c, Some(w)));
    feed_has("ask_agent", Some("sent_now"));
    feed_has("ask_agent_sent", None);
    assert_eq!(
        touches(&mut c).into_iter().find(|t| t.ticket == w).map(|t| t.action).as_deref(),
        Some("sent")
    );
    // The ack, then the turn's end: the crown's answer.
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    stop(&mut c, ws);
    answer_heard(&mut c, before);

    // ---- without now: the queue waits for idle, as before -------------------
    start(&mut c, ws);
    assert_eq!(
        road_of(ask(&mut c, "mesimon-probe-83 later", Deliver::Idle)),
        (false, AskRoad::Queued)
    );
    feed_has("ask_agent", Some("queued"));
    settle();
    assert!(!landed("mesimon-probe-83"), "a working agent is not interrupted");
    let before = answers();
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the queued words at W's idle", || {
        landed("mesimon-probe-83 later")
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    answer_heard(&mut c, before);

    // ---- now at a dialog: refused, nothing parked ---------------------------
    start(&mut c, ws);
    let permission = r#"{"tool_name":"Bash"}"#;
    hook_send(&hook_sock, &ws.to_string(), "PermissionRequest", permission);
    c.await_state(ws, "at the dialog", |s| matches!(s, SessionState::RequiresAction { .. }));
    match ask(&mut c, "mesimon-probe-84 into the dialog", Deliver::Now) {
        Response::Err { message } => {
            assert!(
                message.contains("at a dialog") && message.contains("deliver idle"),
                "{message}"
            )
        }
        other => panic!("now at a dialog: {other:?}"),
    }
    assert!(row(&mut c).is_none(), "a refused ask queues nothing");
    settle();
    assert!(!landed("mesimon-probe-84"));
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", permission);
    c.await_state(ws, "past the dialog", |s| *s == SessionState::Running);

    // ---- an ack that outlives the in-flight window keeps the mark -----------
    // A long turn takes the words at its next step (a paste) or after it
    // ends (the mod's submit): the window's give-up is not the turn's end.
    let before = answers();
    assert_eq!(
        road_of(ask(&mut c, "mesimon-probe-85 late", Deliver::Now)),
        (false, AskRoad::SentNow)
    );
    wait_until(std::time::Duration::from_secs(5), "the late words in W's turn", || {
        landed("mesimon-probe-85 late")
    });
    wait_until(std::time::Duration::from_secs(20), "the in-flight window to give up", || {
        std::fs::read_to_string(&feed_path)
            .is_ok_and(|f| f.contains("\"paste_unacked\"") || f.contains("\"submit_unacked\""))
    });
    stop(&mut c, ws);
    if test_road() == "mod" {
        // Held behind the turn, the submit runs as a turn of its own.
        start(&mut c, ws);
        stop(&mut c, ws);
    }
    answer_heard(&mut c, before);

    // ---- sends off: held for a person, the send preset to now ---------------
    assert!(matches!(c.request(Command::SetCrownSends { on: false }), Response::Ok));
    start(&mut c, ws);
    let (held, road) = road_of(ask(&mut c, "mesimon-probe-86 held now", Deliver::Now));
    assert!(held && road == AskRoad::HeldForPerson);
    feed_has("ask_agent", Some("held_for_person"));
    let r = row(&mut c).expect("held on W's card");
    assert!(r.is_held() && r.deliver == Deliver::Now && !r.sends, "{r:?}");
    settle();
    assert!(!landed("mesimon-probe-86"), "held words do not go on their own");
    // The person's `^y` sends them at once, into the running turn.
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: w }), Response::Ok));
    wait_until(std::time::Duration::from_secs(5), "the held words, sent by hand", || {
        landed("mesimon-probe-86 held now")
    });
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "mid-turn");
    stop(&mut c, ws);

    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-8"), "the feed never carries the words");
}

/// A stub that records every byte its pane is sent, raw: no line
/// discipline, so a key with no Enter after it (Claude Code's send-now,
/// Ctrl+X Ctrl+S, which the tty's flow control would otherwise eat as
/// XOFF) is on the record as it came.
const RAW_STUB: &str = "#!/bin/sh\nstty raw -echo -ixon 2>/dev/null\n\
                        exec cat >> \"$(dirname \"$0\")/got.txt\"\n";

/// The crown's `immediately` (T-601): Claude Code's own send-now over the
/// words, the third level above T-600's `now`. With crown sends on, the
/// words reach a WORKING agent the crown started and go in by the send-now
/// keys, with no Enter after them — pasted on the hook set, filled into
/// the empty composer by the stand-in engine's `fill` on the mod pass —
/// while the agent stays mid-turn; the turn that takes them wakes the
/// crown at its end. A person's field sends the same way. On the mod pass a
/// fill refused over a person's draft falls back to the plain `submit`,
/// and the feed says why.
#[test]
fn the_crown_sends_immediately_into_a_working_turn() {
    let Some(h) = Harness::boot_with_env(
        "crown_immediately",
        Some(RAW_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_immediately");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| read_got().contains(probe);
    // The send-now keys right after the words, and no Enter between.
    let sent_now = |probe: &str| read_got().contains(&format!("{probe}\u{18}\u{13}"));
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let feed_has = |actor: &str, cmd: &str, outcome: Option<&str>| {
        wait_until(std::time::Duration::from_secs(5), &format!("the {cmd} feed line"), || {
            std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
                feed.lines().any(|l| {
                    l.contains(&format!("\"cmd\":\"{cmd}\""))
                        && l.contains(&format!("\"actor\":\"{actor}\""))
                        && outcome.is_none_or(|o| l.contains(&format!("\"outcome\":\"{o}\"")))
                })
            })
        });
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-71 worker");
    let kw = key_of(&mut c, w);
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownSends { on: true }), Response::Ok));
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
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "W's finish on the crown", || {
        landed(&format!("{kw} \"mesimon-probe-71 worker\" finished its turn"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    let answered = format!("{kw} \"mesimon-probe-71 worker\" answered your ask");
    let answers = || read_got().matches(answered.as_str()).count();
    let ask = |c: &mut TestClient, text: &str, deliver: Deliver| {
        let v = read(c, sa, &kw).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver,
            },
        )
    };
    let road_of = |r: Response| match r {
        Response::AgentAsked { held_for_person, road, .. } => (held_for_person, road.unwrap()),
        other => panic!("ask_agent: {other:?}"),
    };

    // ---- immediately, at a working worker: the send-now, mid-turn -----------
    start(&mut c, ws);
    let before = answers();
    assert_eq!(
        road_of(ask(&mut c, "mesimon-probe-72 at once", Deliver::Immediately)),
        (false, AskRoad::SentImmediately)
    );
    wait_until(std::time::Duration::from_secs(5), "the words and the send-now", || {
        sent_now("mesimon-probe-72 at once")
    });
    // The running turn is not lost: no Stop, the agent still working.
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "mid-turn");
    feed_has("agent", "ask_agent", Some("sent_immediately"));
    feed_has("agent", "ask_agent_sent", Some("immediately"));
    feed_has("daemon", "prompt_sent_immediately", None);
    // The send's ack, then the turn's end: the crown's answer.
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(20), "the answer's wake on the crown", || {
        answers() > before
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- immediately at a dialog: refused, nothing sent ---------------------
    start(&mut c, ws);
    let permission = r#"{"tool_name":"Bash"}"#;
    hook_send(&hook_sock, &ws.to_string(), "PermissionRequest", permission);
    c.await_state(ws, "at the dialog", |s| matches!(s, SessionState::RequiresAction { .. }));
    match ask(&mut c, "mesimon-probe-73 into the dialog", Deliver::Immediately) {
        Response::Err { message } => assert!(
            message.contains("at a dialog") && message.contains("sent immediately"),
            "{message}"
        ),
        other => panic!("immediately at a dialog: {other:?}"),
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!landed("mesimon-probe-73"));
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", permission);
    c.await_state(ws, "past the dialog", |s| *s == SessionState::Running);

    // ---- a person's field at `immediately`: the same send -------------------
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: w,
            text: "mesimon-probe-74 by hand".into(),
            queued: false,
            immediately: true,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
        }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(5), "the person's send-now", || {
        sent_now("mesimon-probe-74 by hand")
    });
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "mid-turn");
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);

    // ---- the mod pass: a fill over a person's draft falls back --------------
    if test_road() == "mod" {
        std::fs::write(h.dir.join("mod-draft"), "").unwrap();
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket: w,
                text: "mesimon-probe-75 over a draft".into(),
                queued: false,
                immediately: true,
                accept_plan: false,
                plan: false,
                tier: None,
                resend: false,
            }),
            Response::Ok
        ));
        feed_has("daemon", "prompt_send_now_refused", Some("draft"));
        // The plain submit takes the words, and no send-now goes over them.
        wait_until(std::time::Duration::from_secs(5), "the words by the submit", || {
            landed("mesimon-probe-75 over a draft")
        });
        assert!(!sent_now("mesimon-probe-75 over a draft"), "the draft is not sent");
        std::fs::remove_file(h.dir.join("mod-draft")).unwrap();
        hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    }
    stop(&mut c, ws);

    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-7"), "the feed never carries the words");
}

/// A delivery the armed merge train will take does not wake the crown
/// (T-554): the crown hears it once, at the merge, as the delivery with
/// `merged` in its delta. And when the train will not land it after all —
/// the rebase it asked for left the branch behind, or the checkout refused
/// the merge — the held delivery wakes the crown then.
#[test]
fn a_delivery_the_train_will_take_wakes_the_crown_at_its_merge() {
    // The flags (and the train) every second rather than every ten.
    let Some(h) = Harness::boot_with_env(
        "crown_train",
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
    let mut c = h.client("crown_train");
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
    let feed_count = |needle: &str| -> usize {
        std::fs::read_to_string(&feed_path).unwrap_or_default().matches(needle).count()
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
    let w = create(&mut c, "mesimon-probe-92 on the train");
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
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
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
    // This connection arms the train and keeps it armed.
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let worker = format!("{kw} \"mesimon-probe-92 on the train\"");
    let merged = || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };

    // ---- 1. delivered and merged by the train: one line, at the merge -----
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the train to merge it", merged);
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let lines = lines_with(&worker);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(&format!("{worker} delivered (merge_state merged")),
        "the delivery was held for the merge: {lines:?}"
    );
    assert_eq!(feed_count("\"crown_wake_deferred\""), 1);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 2. the rebase the train asked for left it behind: the crown hears
    std::fs::write(h.repo.join("base1.txt"), "base\n").unwrap();
    git(&h.repo, &["add", "base1.txt"]);
    git(&h.repo, &["commit", "-qm", "base1"]);
    commit(&path, "two.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    let ask = format!("Rebase your current branch {branch} onto main");
    wait_until(std::time::Duration::from_secs(15), "the train's rebase ask", || {
        !lines_with(&ask).is_empty()
    });
    assert_eq!(lines_with(&worker).len(), 1, "a delivery the train takes is silent");
    assert_eq!(feed_count("\"crown_wake_deferred\""), 2);
    // The agent takes the ask and ends its turn still behind.
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the give-up's wake", || {
        lines_with(&worker).len() == 2
    });
    let lines = lines_with(&worker);
    assert!(
        lines[1].starts_with(&format!("{worker} delivered (merge_state merged → needs_rebase")),
        "{lines:?}"
    );
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 3. the checkout refuses the merge: the crown hears ---------------
    // In the way: the fast-forward would write this file, and git will not
    // overwrite one it does not know about.
    std::fs::write(h.repo.join("two.txt"), "not mine\n").unwrap();
    start(&mut c, ws);
    git(&path, &["rebase", "-q", "main"]);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the refusal", || {
        feed_count("merge_train_refused:merge") > 0
    });
    wait_until(std::time::Duration::from_secs(10), "the refusal's wake", || {
        lines_with(&worker).len() == 3
    });
    assert_eq!(feed_count("\"crown_wake_deferred\""), 3);
    let lines = lines_with(&worker);
    assert!(
        lines[2].starts_with(&format!("{worker} delivered (merge_state needs_rebase → ahead")),
        "{lines:?}"
    );
    assert!(!merged(), "refused");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 3, "once");
}

/// The crown hears a landing when its worker is finished entirely (T-596):
/// the train merges a crown-started worker's branch and pastes the merged
/// notice, and the wake waits while the notice is on its way and while its
/// turn runs. It arrives when that turn ends, saying so, and the crown's
/// close-out — `sleep_agent`, then DONE — goes through right after it. A
/// delivery held for the train (T-554) and landed with a notice waits the
/// same way.
#[test]
fn the_merged_wake_waits_for_the_notice_turn() {
    // The flags (and the train) every second rather than every ten.
    let Some(h) = Harness::boot_with_env(
        "crown_notice",
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
    let mut c = h.client("crown_notice");
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
    let w = create(&mut c, "mesimon-probe-96 notified");
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
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
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
    let worker = format!("{kw} \"mesimon-probe-96 notified\"");
    let merged = || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };
    let train = |c: &mut TestClient, on: bool| {
        assert!(matches!(
            c.request(Command::SetAutomation { merge_train: on, merge_notice: on }),
            Response::Ok
        ));
    };
    // The train merged and pasted its notice; nothing reaches the crown
    // while the notice waits for the worker, nor while its turn runs. Then
    // the turn ends and the one line comes.
    let land = |c: &mut TestClient, notices: usize, heard: usize| {
        wait_until(std::time::Duration::from_secs(15), "the train to merge it", merged);
        wait_until(std::time::Duration::from_secs(10), "the train's merged notice", || {
            lines_with("has been merged").len() == notices
        });
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert_eq!(lines_with(&worker).len(), heard, "held while the notice is on its way");
        start(c, ws);
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert_eq!(lines_with(&worker).len(), heard, "held while the notice turn runs");
        stop(c, ws);
        wait_until(std::time::Duration::from_secs(10), "the wake at the turn's end", || {
            lines_with(&worker).len() == heard + 1
        });
        lines_with(&worker).pop().unwrap()
    };

    // ---- 1. a delivery held for the train, landed with its notice ---------
    train(&mut c, true);
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    let line = land(&mut c, 1, 0);
    assert!(
        line.contains(&format!(
            "{worker} delivered and finished its turn (merge_state merged, column "
        )),
        "{line}"
    );
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert_eq!(feed.matches("\"crown_wake_deferred\"").count(), 1, "the train's hold");
    assert_eq!(feed.matches("\"crown_wake_deferred:merge_step\"").count(), 1, "the notice's");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 2. a delivery heard, then merged with its notice -----------------
    train(&mut c, false);
    commit(&path, "two.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 2
    });
    assert!(lines_with(&worker)[1].starts_with(&format!("{worker} delivered (")));
    start(&mut c, sa);
    stop(&mut c, sa);
    train(&mut c, true);
    let line = land(&mut c, 2, 2);
    assert_eq!(
        line,
        format!(
            "{worker} merged and finished its turn (merge_state ahead → merged) ∙ get_ticket \
             key={kw} for state and notes"
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

    // ---- 3. the crown's close-out, on the wake ----------------------------
    start(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    assert_eq!(v.state.as_ref().map(|s| s.state.as_str()), Some("idle"), "finished entirely");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
    ) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.state.as_ref().map(|s| s.state.as_str()), Some("sleeping"));
        }
        other => panic!("sleep_agent on the wake: {other:?}"),
    }
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentMoveTicket {
            to_column: "DONE".into(),
            idempotency_key: None,
            key: Some(kw.clone()),
            before: None,
            seen: v.seen,
        },
    ) {
        Response::AgentMoved { column, .. } => assert_eq!(column, "DONE"),
        other => panic!("DONE on the wake: {other:?}"),
    }
    stop(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(
        lines_with(&worker).len(),
        3,
        "one wake per landing:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );
}

/// T-566: a worker stopped on `AskUserQuestion` reads `needs-you` with the
/// stop's reason and the question on `get_ticket` — off the projection
/// Remote Control draws, on a board with no phone paired — and the crown's
/// `ask_agent` to it is refused in words naming the road, until the answer.
#[test]
fn the_crown_reads_a_worker_s_question_and_cannot_talk_over_it() {
    let Some(h) = Harness::boot_with_env(
        "crown_question",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_question");
    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "the worker");
    let kw = key_of(&mut c, w);
    let sa = spawn(&mut c, a);
    let sw = spawn(&mut c, w);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sw.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sw, "running", |s| *s == SessionState::Running);
    let ask = |c: &mut TestClient, text: &str| {
        let v = read(c, sa, &kw).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
            },
        )
    };
    let asked_touch =
        |c: &mut TestClient| touches(c).into_iter().any(|t| t.ticket == w && t.action == "asked");
    // A working agent is at no stop.
    assert!(read(&mut c, sa, &kw).unwrap().needs_you.is_none());

    // ---- one question: the reason, the words, the options, the request ----
    let one = json!({
        "tool_name": "AskUserQuestion",
        "tool_use_id": "toolu_q1",
        "tool_input": { "questions": [{
            "question": "Which auth provider?",
            "header": "Auth",
            "options": [
                { "label": "Okta", "description": "SSO" },
                { "label": "Auth0", "description": "hosted" }
            ],
            "multiSelect": false
        }]}
    });
    hook_send(&hook_sock, &sw.to_string(), "PreToolUse", &one.to_string());
    c.await_state(sw, "asking", |s| matches!(s, SessionState::RequiresAction { .. }));
    let v = read(&mut c, sa, &kw).unwrap();
    assert_eq!(v.state.as_ref().map(|s| s.state.as_str()), Some("needs-you"));
    let needs = v.needs_you.expect("a question stop says what it is");
    assert_eq!(needs.reason, "question");
    assert_eq!(needs.request.as_deref(), Some("toolu_q1"));
    assert_eq!(needs.questions.len(), 1);
    assert_eq!(needs.answerable, Some(true));
    let q = &needs.questions[0];
    assert_eq!(q.text, "Which auth provider?");
    assert_eq!(q.options, ["Okta", "Auth0"]);
    assert!(!q.multi_select);
    // `list_board` keeps its one word.
    match c.send(Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => {
            let row = board.tickets.iter().find(|t| t.key == kw).unwrap();
            assert_eq!(row.state.as_deref(), Some("needs-you"));
            assert!(!serde_json::to_string(&board).unwrap().contains("needs_you"));
        }
        other => panic!("list_board: {other:?}"),
    }

    // ---- ask_agent is refused while it stands, in words naming the road ----
    match ask(&mut c, "mesimon-probe-566 over the question") {
        Response::Err { message } => {
            let opening = format!("{kw}'s agent is asking a question");
            for words in [
                opening.as_str(),
                "a person answers it in the pane or from Remote Control",
                // T-569: the crown's own road, where the board lets it.
                "or answer_agent where the board lets the crown answer",
                "get_ticket (needs_you)",
                "would wait behind the answer",
            ] {
                assert!(message.contains(words), "the refusal says {words:?}: {message}");
            }
        }
        other => panic!("ask_agent over a question: {other:?}"),
    }
    assert!(!asked_touch(&mut c), "a refusal lights nothing");
    assert!(pending_of(&mut c, Some(w)).is_empty(), "and queues nothing");

    // ---- several questions: each listed, and answerable (T-571) -----------
    let two = json!({
        "tool_name": "AskUserQuestion",
        "tool_use_id": "toolu_q2",
        "tool_input": { "questions": [
            { "question": "Which provider?", "header": "Auth",
              "options": [{ "label": "Okta", "description": "" }], "multiSelect": false },
            { "question": "Which regions?", "header": "Region",
              "options": [{ "label": "eu", "description": "" }, { "label": "us", "description": "" }],
              "multiSelect": true }
        ]}
    });
    hook_send(&hook_sock, &sw.to_string(), "PermissionRequest", &two.to_string());
    wait_until(std::time::Duration::from_secs(5), "the second dialog", || {
        read(&mut c, sa, &kw).unwrap().needs_you.and_then(|n| n.request).as_deref()
            == Some("toolu_q2")
    });
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.unwrap();
    assert_eq!(needs.answerable, Some(true), "{needs:?}");
    let listed: Vec<_> = needs
        .questions
        .iter()
        .map(|q| (q.text.as_str(), q.options.len(), q.multi_select))
        .collect();
    assert_eq!(listed, [("Which provider?", 1, false), ("Which regions?", 2, true)]);

    // ---- answered: the field is gone and the crown's words go through -------
    hook_send(&hook_sock, &sw.to_string(), "PostToolUse", &two.to_string());
    c.await_state(sw, "running again", |s| *s == SessionState::Running);
    assert!(read(&mut c, sa, &kw).unwrap().needs_you.is_none());
    match ask(&mut c, "mesimon-probe-566 after the answer") {
        Response::AgentAsked { .. } => {}
        other => panic!("ask_agent after the answer: {other:?}"),
    }
    assert!(asked_touch(&mut c));
}

/// The crown answers a worker's question where the person lets it (T-569).
/// The stub paints Claude Code's one-question dialog on its screen, so the
/// answer walks Remote Control's own screen-verified road: on by default
/// since T-582, and off by a person's hand no wake and a refusal naming the
/// row; on, a person's agent is still the
/// person's; a question from the crown's own worker wakes the crown without
/// its words; a stale request and a plan (`accept_plan`'s) are refused, and
/// so is the one
/// question's answer on a two-question dialog, which takes one each (T-571);
/// the answer's receipt waits for the stub's `PostToolUse` and says
/// `answered`, the feed carries the label with actor `agent`, the card says
/// `♛ answered` and `answered by` until the next edge, and that turn's end
/// wakes the crown with `answered your ask`.
#[test]
fn the_crown_answers_a_question_where_the_person_lets_it() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        printf 'Which auth provider?\\n\\342\\235\\257 1. Okta\\n  2. Auth0\\n  \
                        3. Type something.\\nEnter to select \\302\\267 Esc to cancel\\n'\n\
                        while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    let Some(h) = Harness::boot_with_env(
        "crown_answer",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_answer");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let typed = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| typed().contains(probe);
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));
    let asking = |s: &SessionState| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    };
    let question = |request: &str| {
        json!({
            "tool_name": "AskUserQuestion",
            "tool_use_id": request,
            "tool_input": { "questions": [{
                "question": "Which auth provider?",
                "header": "Auth",
                "options": [
                    { "label": "Okta", "description": "" },
                    { "label": "Auth0", "description": "" }
                ],
                "multiSelect": false
            }]}
        })
        .to_string()
    };

    let a = create(&mut c, "coordinate");
    let p = create(&mut c, "the person's own");
    let w = create(&mut c, "mesimon-probe-569 worker");
    let (ka, kp, kw) = (key_of(&mut c, a), key_of(&mut c, p), key_of(&mut c, w));
    let sa = spawn(&mut c, a);
    let sp = spawn(&mut c, p);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    for s in [sa, sp] {
        start(&mut c, s);
        stop(&mut c, s);
    }
    let v = read(&mut c, sa, &kw).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
            },
        ),
        Response::AgentStarted { .. }
    ));
    // T-583: P, a person's agent, holds the checkout, so W works in a
    // worktree of its own.
    wait_until(std::time::Duration::from_secs(15), "W's worktree start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    let answer_cmd = |c: &mut TestClient, key: &str, request: &str, index, text: Option<&str>| {
        let v = read(c, sa, key).unwrap();
        Command::AgentAnswerTicket {
            key: key.into(),
            seen: v.seen,
            request: request.into(),
            index,
            text: text.map(str::to_string),
            answers: None,
        }
    };
    let refused = |c: &mut TestClient, key: &str, request: &str, index, text| {
        let cmd = answer_cmd(c, key, request, index, text);
        match c.send(Principal::Agent { session: sa }, cmd) {
            Response::Err { message } => message,
            other => panic!("answer_agent {key} {request}: {other:?}"),
        }
    };

    // ---- off (on by default since T-582): no wake, the refusal names the row -
    assert!(c.board().crown_answers, "on by default");
    assert!(matches!(c.request(Command::SetCrownAnswers { on: false }), Response::Ok));
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_a1"));
    c.await_state(ws, "asking", asking);
    settle();
    assert!(!landed("asks a question"), "off, the person is the one to wake");
    let why = refused(&mut c, &kw, "toolu_a1", Some(0), None);
    assert!(why.contains("Settings → Agents → Crown answers questions"), "{why}");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &question("toolu_a1"));
    c.await_state(ws, "running", |s| *s == SessionState::Running);

    // ---- on: an agent may not switch it, a person does, and it is kept -----
    match c.send(Principal::Agent { session: sa }, Command::SetCrownAnswers { on: true }) {
        Response::Err { .. } => {}
        other => panic!("an agent switching it on: {other:?}"),
    }
    assert!(!c.board().crown_answers);
    assert!(matches!(c.request(Command::SetCrownAnswers { on: true }), Response::Ok));
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_answers = true"), "{file}");
    assert!(!file.contains("crown_sends = true"), "a switch of its own: {file}");

    // ---- a person's agent: the one who started it answers it ---------------
    start(&mut c, sp);
    hook_send(&hook_sock, &sp.to_string(), "PreToolUse", &question("toolu_p1"));
    c.await_state(sp, "the person's asking", asking);
    let why = refused(&mut c, &kp, "toolu_p1", Some(0), None);
    assert!(why.contains("a person started") && why.contains("answers it"), "{why}");
    settle();
    assert!(!landed(&format!("{kp} \"the person's own\" asks")), "nor does it wake the crown");
    hook_send(&hook_sock, &sp.to_string(), "PostToolUse", &question("toolu_p1"));
    c.await_state(sp, "running", |s| *s == SessionState::Running);
    stop(&mut c, sp);

    // ---- the crown's worker asks: the crown wakes, without the words -------
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_a2"));
    c.await_state(ws, "asking", asking);
    // T-595: the question rode one parallel batch with a Bash call and a
    // subagent, which finish while its dialog stands.
    let bash = json!({ "tool_name": "Bash", "tool_use_id": "toolu_bash" }).to_string();
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &bash);
    hook_send(&hook_sock, &ws.to_string(), "PostToolUseFailure", &bash);
    let nested = json!({ "tool_name": "Read", "tool_use_id": "toolu_sub", "agent_id": "a1" });
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &nested.to_string());
    let wake = format!("{kw} \"mesimon-probe-569 worker\" asks a question");
    wait_until(std::time::Duration::from_secs(10), "the question's wake on the crown", || {
        landed(&wake)
    });
    assert!(!typed().contains("Which auth provider"), "the question's words never ride a wake");
    start(&mut c, sa);
    c.await_state(ws, "still asking past its siblings", asking);
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.expect("the question");
    assert_eq!(needs.request.as_deref(), Some("toolu_a2"), "the words outlive the batch");
    assert_eq!(needs.questions[0].text, "Which auth provider?");

    // ---- refusals in words -------------------------------------------------
    let why = refused(&mut c, &kw, "toolu_a1", Some(0), None);
    assert!(why.contains("dialog changed; read get_ticket again"), "{why}");
    let why = refused(&mut c, &kw, "toolu_a2", Some(5), None);
    assert!(why.contains("out of range") && why.contains("2 options"), "{why}");
    let why = refused(&mut c, &kw, "toolu_a2", None, Some("one\ntwo"));
    assert!(why.contains("one line"), "{why}");
    let why = refused(&mut c, &ka, "toolu_a2", Some(0), None);
    assert!(why.contains("own ticket"), "{why}");
    let mut stale = answer_cmd(&mut c, &kw, "toolu_a2", Some(0), None);
    if let Command::AgentAnswerTicket { seen, .. } = &mut stale {
        *seen = Some("0000000000000000".into());
    }
    match c.send(Principal::Agent { session: sa }, stale) {
        Response::Err { message } => assert!(message.contains("changed since it was read")),
        other => panic!("a stale stamp: {other:?}"),
    }
    assert!(
        !std::fs::read_to_string(&feed_path).unwrap().contains("\"answer_agent\""),
        "a refusal writes nothing"
    );

    // ---- answered: the receipt waits for the dialog's own hook edge --------
    let lines = typed().lines().count();
    let cmd = answer_cmd(&mut c, &kw, "toolu_a2", Some(0), None);
    let sock = h.paths.orch_sock();
    let call = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    if test_road() == "mod" {
        // T-576: the answer goes down W's mod as one frame, by the label, and
        // the mod's own report is the receipt: no key, no `PostToolUse`.
        let record = h.dir.join(format!("mod-{ws}.ndjson"));
        wait_until(std::time::Duration::from_secs(5), "the answer at W's mod", || {
            std::fs::read_to_string(&record).unwrap_or_default().lines().any(|l| {
                l.contains("\"kind\":\"answer\"")
                    && l.contains("\"tool_use_id\":\"toolu_a2\"")
                    && l.contains("\"Which auth provider?\":\"Okta\"")
            })
        });
    } else {
        wait_until(std::time::Duration::from_secs(5), "the answer's Enter in W's pane", || {
            typed().lines().count() > lines
        });
        assert!(!call.is_finished(), "the receipt waits for the hook edge");
        hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &question("toolu_a2"));
    }
    match call.join().unwrap() {
        Response::AgentAnswered { key, outcome, reason, answer, seen } => {
            assert_eq!((key.as_str(), outcome.as_str()), (kw.as_str(), "answered"));
            assert_eq!((reason, answer.as_str()), (None, "Okta"));
            assert!(seen.is_some());
        }
        other => panic!("answer_agent: {other:?}"),
    }
    assert!(
        touches(&mut c).iter().any(|t| t.ticket == w && t.action == "answered"),
        "♛ answered on the card"
    );
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(
        feed.lines().any(|l| l.contains("\"cmd\":\"answer_agent\"")
            && l.contains("\"actor\":\"agent\"")
            && l.contains("\"answer\":\"Okta\"")
            && l.contains("\"outcome\":\"answered\"")),
        "{feed}"
    );
    let line = format!("answered by {ka}: Okta");
    let detail = |c: &mut TestClient| {
        c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.detail.clone())
    };
    assert_eq!(detail(&mut c).as_deref(), Some(line.as_str()));
    c.await_state(ws, "running on the answer", |s| *s == SessionState::Running);
    assert_eq!(detail(&mut c).as_deref(), Some(line.as_str()), "kept through its own edge");

    // ---- the turn that took the answer wakes the crown when it ends --------
    stop(&mut c, sa);
    stop(&mut c, ws);
    assert_eq!(detail(&mut c), None, "gone at the next state edge");
    wait_until(std::time::Duration::from_secs(10), "the answer's turn waking the crown", || {
        landed(&format!("{kw} \"mesimon-probe-569 worker\" answered your ask"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a plan is a person's ---------------------------------------------
    start(&mut c, ws);
    let plan = json!({
        "tool_name": "ExitPlanMode",
        "tool_use_id": "toolu_plan",
        "tool_input": { "plan": "Ship it." }
    })
    .to_string();
    let plan = plan.as_str();
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", plan);
    c.await_state(ws, "on its plan", |s| matches!(s, SessionState::RequiresAction { .. }));
    let why = refused(&mut c, &kw, "toolu_plan", Some(0), None);
    assert!(why.contains("stopped on a plan") && why.contains("accept_plan"), "{why}");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", plan);
    c.await_state(ws, "running", |s| *s == SessionState::Running);

    // ---- several questions take one answer each (T-571) ----------------------
    let two = json!({
        "tool_name": "AskUserQuestion",
        "tool_use_id": "toolu_q2",
        "tool_input": { "questions": [
            { "question": "Which provider?", "header": "Auth",
              "options": [{ "label": "Okta", "description": "" }], "multiSelect": false },
            { "question": "Which region?", "header": "Region",
              "options": [{ "label": "eu", "description": "" }], "multiSelect": false }
        ]}
    })
    .to_string();
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &two);
    c.await_state(ws, "asking two", asking);
    let why = refused(&mut c, &kw, "toolu_q2", Some(0), None);
    assert!(why.contains("asks 2 questions at once") && why.contains("answers carries"), "{why}");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &two);
    c.await_state(ws, "running", |s| *s == SessionState::Running);

    // ---- off again: the crown's answer is refused once more ----------------
    assert!(matches!(c.request(Command::SetCrownAnswers { on: false }), Response::Ok));
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_a3"));
    c.await_state(ws, "asking", asking);
    let why = refused(&mut c, &kw, "toolu_a3", Some(0), None);
    assert!(why.contains("Crown answers questions is off"), "{why}");
}

/// The crown answers a batch (T-571). The worker's pane runs a stand-in for
/// Claude Code's dialog as measured on 2.1.287 (`fake_claude_dialog.py`):
/// `get_ticket` lists every question, an answer whose count or kind does not
/// fit is refused in words, and the answer walks the dialog tab by tab — a
/// choice, two ticks and words — to its review, whose `Submit answers` is
/// pressed once; the receipt reads `answered` only on the stub's
/// `PostToolUse`, and the feed and the card carry every answer.
#[test]
fn the_crown_answers_a_batch_one_answer_per_question() {
    use mesimon_core::mesophon::QuestionAnswer as Q;
    const STUB: &str = include_str!("common/fake_claude_dialog.py");
    let Some(h) = Harness::boot_bare(
        "crown_batch",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_batch");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-571 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownAnswers { on: true }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let v = read(&mut c, sa, &kw).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("shared_checkout".into()),
            },
        ),
        Response::AgentStarted { .. }
    ));
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);

    // ---- the batch, drawn in W's pane and seen by the hook --------------------
    let input = json!({ "questions": [
        { "question": "Which color?", "header": "Color", "multiSelect": false,
          "options": [{ "label": "Blue", "description": "Calm" },
                      { "label": "Green", "description": "Go" }] },
        { "question": "Which toppings?", "header": "Toppings", "multiSelect": true,
          "options": [{ "label": "Cheese", "description": "Melted" },
                      { "label": "Olives", "description": "" },
                      { "label": "Basil", "description": "Fresh" }] },
        { "question": "Which size?", "header": "Size", "multiSelect": false,
          "options": [{ "label": "Small", "description": "" },
                      { "label": "Large", "description": "" }] }
    ]});
    std::fs::write(h.dir.join(format!("dialog-{kw}.json")), input.to_string()).unwrap();
    let frame = json!({
        "tool_name": "AskUserQuestion", "tool_use_id": "toolu_b1", "tool_input": input
    })
    .to_string();
    let pane = |c: &mut TestClient| match c.request(Command::PaneTail { session: ws, lines: 60 }) {
        Response::PaneTail { lines, .. } => lines.join("\n"),
        _ => String::new(),
    };
    wait_until(std::time::Duration::from_secs(10), "the batch drawn", || {
        pane(&mut c).contains("✔ Submit")
    });
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &frame);
    c.await_state(ws, "asking", |s| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    });

    // ---- get_ticket lists every question -------------------------------------
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.expect("the batch");
    assert_eq!(needs.request.as_deref(), Some("toolu_b1"));
    assert_eq!(needs.answerable, Some(true));
    let listed: Vec<_> = needs
        .questions
        .iter()
        .map(|q| (q.text.as_str(), q.options.join("/"), q.multi_select))
        .collect();
    assert_eq!(
        listed,
        [
            ("Which color?", "Blue/Green".to_string(), false),
            ("Which toppings?", "Cheese/Olives/Basil".to_string(), true),
            ("Which size?", "Small/Large".to_string(), false),
        ]
    );

    // ---- an answer that does not fit is refused in words, no key typed -------
    let keys = h.dir.join(format!("keys-{kw}"));
    let typed = || std::fs::read_to_string(&keys).unwrap_or_default();
    let before = typed();
    let answer = |c: &mut TestClient, index: Option<usize>, answers: Option<Vec<Q>>| {
        let v = read(c, sa, &kw).unwrap();
        Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: v.seen,
            request: "toolu_b1".into(),
            index,
            text: None,
            answers,
        }
    };
    let refused = |c: &mut TestClient, index, answers| {
        let cmd = answer(c, index, answers);
        match c.send(Principal::Agent { session: sa }, cmd) {
            Response::Err { message } => message,
            other => panic!("answer_agent: {other:?}"),
        }
    };
    let why = refused(&mut c, Some(1), None);
    assert!(why.contains("asks 3 questions at once"), "{why}");
    let why =
        refused(&mut c, None, Some(vec![Q::Choice { index: 1 }, Q::Choices { indices: vec![0] }]));
    assert!(why.contains("answers carries 2 answers") && why.contains("asks 3 questions"), "{why}");
    let why = refused(
        &mut c,
        None,
        Some(vec![Q::Choice { index: 1 }, Q::Choice { index: 0 }, Q::Choice { index: 1 }]),
    );
    assert!(why.contains("question 2 takes several choices"), "{why}");
    assert_eq!(typed(), before, "a refusal types nothing");

    // ---- the answer walks the tabs; the receipt waits for the hook edge ------
    let cmd = answer(
        &mut c,
        None,
        Some(vec![
            Q::Choice { index: 1 },
            Q::Choices { indices: vec![0, 2] },
            Q::Text { text: "Large please".into() },
        ]),
    );
    let sock = h.paths.orch_sock();
    let call = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    let answered = h.dir.join(format!("answered-{kw}.json"));
    wait_until(std::time::Duration::from_secs(30), "the dialog submitted", || answered.exists());
    let took: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&answered).unwrap()).unwrap();
    assert_eq!(
        took,
        json!({
            "Which color?": "Green",
            "Which toppings?": "Cheese, Basil",
            "Which size?": "Large please"
        })
    );
    let walked = typed()[before.len()..].to_string();
    if test_road() == "mod" {
        // T-576: the whole answer went down W's mod in one frame, no key at
        // all, and the mod's own report (no `PostToolUse` fires for it) is
        // the receipt.
        assert_eq!(walked, "", "no key reaches the pane");
    } else {
        assert_eq!(walked.matches("\\r").count(), 4, "three tabs and one Submit: {walked}");
        assert!(!call.is_finished(), "the receipt waits for the hook edge");
        hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &frame);
    }
    match call.join().unwrap() {
        Response::AgentAnswered { key, outcome, reason, answer, .. } => {
            assert_eq!((key.as_str(), outcome.as_str(), reason), (kw.as_str(), "answered", None));
            assert_eq!(answer, "Green; Cheese, Basil; Large please");
        }
        other => panic!("answer_agent: {other:?}"),
    }
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(
        feed.lines().any(|l| l.contains("\"cmd\":\"answer_agent\"")
            && l.contains("\"answer\":\"Green; Cheese, Basil; Large please\"")
            && l.contains("\"outcome\":\"answered\"")),
        "{feed}"
    );
    let detail = c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.detail.clone());
    assert_eq!(
        detail.as_deref(),
        Some(format!("answered by {ka}: Green; Cheese, Basil; Large please").as_str())
    );
}

/// The crown accepts a plan (T-582), by default. The panes run the stand-in
/// for Claude Code's dialog (`fake_claude_dialog.py`) drawing the plan
/// dialog measured on 2.1.287: the switch is on on a fresh board; a plan on
/// an agent a person started is refused and wakes nobody; the crown's
/// worker's plan wakes the crown without its words, and `get_ticket` shows
/// the plan; refusals are in words and type nothing; the accept is ONE
/// Enter on the default row, `Yes, auto-accept edits`, and its receipt
/// waits for the stub's `PostToolUse` to say `accepted`, with `♛ accepted
/// plan`, the feed line and `plan accepted by` on the card, and that turn's
/// end wakes the crown; an Enter no edge confirms is `input_sent` and claims
/// nothing on the card; and a person's answer before the crown's press is
/// `unknown { a_person_answered }`, with no key of the crown's typed.
#[test]
fn the_crown_accepts_a_plan_by_default() {
    use mesimon_core::board::Reason;
    const STUB: &str = include_str!("common/fake_claude_dialog.py");
    const PLAN: &str = "1. Read the code\n2. Write the code";
    let Some(h) = Harness::boot_bare(
        "crown_plan",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_plan");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let typed = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| typed().contains(probe);
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let at_plan = |s: &SessionState| *s == SessionState::RequiresAction { reason: Reason::Plan };
    let frame = |request: &str| {
        json!({ "tool_name": "ExitPlanMode", "tool_use_id": request, "tool_input": { "plan": PLAN } })
            .to_string()
    };
    let pane = |c: &mut TestClient, sid: uuid::Uuid| match c
        .request(Command::PaneTail { session: sid, lines: 60 })
    {
        Response::PaneTail { lines, .. } => lines.join("\n"),
        _ => String::new(),
    };
    // The plan drawn in a pane, then the hook that stops its agent on it.
    let show_plan = |c: &mut TestClient, key: &str, sid: uuid::Uuid, request: &str| {
        let dialog = h.dir.join(format!("dialog-{key}.json"));
        std::fs::write(dialog, json!({ "plan": PLAN }).to_string()).unwrap();
        wait_until(std::time::Duration::from_secs(10), "the plan drawn", || {
            pane(c, sid).contains("Would you like to proceed?")
        });
        hook_send(&hook_sock, &sid.to_string(), "PreToolUse", &frame(request));
        c.await_state(sid, "on its plan", at_plan);
    };

    let a = create(&mut c, "coordinate");
    let p = create(&mut c, "the person's own");
    let w = create(&mut c, "mesimon-probe-582 worker");
    let (ka, kp, kw) = (key_of(&mut c, a), key_of(&mut c, p), key_of(&mut c, w));
    let sa = spawn(&mut c, a);
    let sp = spawn(&mut c, p);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    for s in [sa, sp] {
        start(&mut c, s);
        stop(&mut c, s);
    }

    // ---- on by default: no person switched anything ----------------------
    assert!(c.board().crown_answers, "a fresh board lets the crown answer and accept");
    let v = read(&mut c, sa, &kw).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: true,
                tier: None,
                workspace: Some("worktree".into()),
            },
        ),
        Response::AgentStarted { .. }
    ));
    // T-583: P, a person's agent, holds the checkout, so W works in a
    // worktree of its own.
    wait_until(std::time::Duration::from_secs(15), "W's worktree start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    let accept_cmd = |c: &mut TestClient, key: &str, request: &str| {
        let v = read(c, sa, key).unwrap();
        Command::AgentAcceptPlan { key: key.into(), seen: v.seen, request: request.into() }
    };
    let refused = |c: &mut TestClient, key: &str, request: &str| {
        let cmd = accept_cmd(c, key, request);
        match c.send(Principal::Agent { session: sa }, cmd) {
            Response::Err { message } => message,
            other => panic!("accept_plan {key} {request}: {other:?}"),
        }
    };
    let call = |c: &mut TestClient, key: &str, request: &str| {
        let cmd = accept_cmd(c, key, request);
        let sock = h.paths.orch_sock();
        std::thread::spawn(move || {
            TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
        })
    };

    // ---- a person's agent: the one who started it accepts its plan ---------
    start(&mut c, sp);
    show_plan(&mut c, &kp, sp, "toolu_p1");
    let why = refused(&mut c, &kp, "toolu_p1");
    assert!(why.contains("a person started") && why.contains("accepts its plan"), "{why}");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!landed(&format!("{kp} \"the person's own\" stops on a plan")), "nor a wake");
    std::fs::remove_file(h.dir.join(format!("dialog-{kp}.json"))).unwrap();
    hook_send(&hook_sock, &sp.to_string(), "PostToolUse", &frame("toolu_p1"));
    c.await_state(sp, "running", |s| *s == SessionState::Running);
    stop(&mut c, sp);

    // ---- the crown's worker stops on a plan: the crown wakes, not its words --
    show_plan(&mut c, &kw, ws, "toolu_w1");
    let wake = format!("{kw} \"mesimon-probe-582 worker\" stops on a plan");
    wait_until(std::time::Duration::from_secs(10), "the plan's wake on the crown", || {
        landed(&wake)
    });
    assert!(!typed().contains("Read the code"), "the plan's words never ride a wake");
    start(&mut c, sa);
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.expect("the plan");
    assert_eq!(needs.reason, "plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w1"));
    assert_eq!(needs.plan.as_deref(), Some(PLAN), "the markdown, lines kept");

    // ---- refusals in words, none typing a key or writing a feed line --------
    let keys = h.dir.join(format!("keys-{kw}"));
    let pressed = || std::fs::read_to_string(&keys).unwrap_or_default();
    let before = pressed();
    let why = refused(&mut c, &kw, "toolu_old");
    assert!(why.contains("plan changed; read get_ticket again"), "{why}");
    let why = refused(&mut c, &ka, "toolu_w1");
    assert!(why.contains("own ticket"), "{why}");
    let mut stale = accept_cmd(&mut c, &kw, "toolu_w1");
    if let Command::AgentAcceptPlan { seen, .. } = &mut stale {
        *seen = Some("0000000000000000".into());
    }
    match c.send(Principal::Agent { session: sa }, stale) {
        Response::Err { message } => assert!(message.contains("changed since it was read")),
        other => panic!("a stale stamp: {other:?}"),
    }
    let v = read(&mut c, sa, &kw).unwrap();
    let answer = Command::AgentAnswerTicket {
        key: kw.clone(),
        seen: v.seen.clone(),
        request: "toolu_w1".into(),
        index: Some(0),
        text: None,
        answers: None,
    };
    match c.send(Principal::Agent { session: sa }, answer) {
        Response::Err { message } => {
            assert!(message.contains("stopped on a plan") && message.contains("accept_plan"))
        }
        other => panic!("answer_agent on a plan: {other:?}"),
    }
    let ask = Command::AgentAskTicket {
        key: kw.clone(),
        text: "also".into(),
        seen: v.seen,
        plan: false,
        deliver: Deliver::Idle,
    };
    match c.send(Principal::Agent { session: sa }, ask) {
        Response::Err { message } => assert!(
            message.contains("stopped on a plan") && message.contains("accept_plan accepts it"),
            "{message}"
        ),
        other => panic!("ask_agent on a plan: {other:?}"),
    }
    assert!(matches!(c.request(Command::SetCrownAnswers { on: false }), Response::Ok));
    let why = refused(&mut c, &kw, "toolu_w1");
    assert!(why.contains("Crown answers questions is off") && why.contains("raise_hand"), "{why}");
    assert!(matches!(c.request(Command::SetCrownAnswers { on: true }), Response::Ok));
    assert_eq!(pressed(), before, "a refusal types nothing");
    assert!(!feed().contains("\"accept_plan\""), "a refusal writes nothing");

    // ---- accepted: one Enter on the default row, the receipt on the edge ----
    let accepted = h.dir.join(format!("accepted-{kw}"));
    let receipt = call(&mut c, &kw, "toolu_w1");
    wait_until(std::time::Duration::from_secs(10), "the plan accepted in W's pane", || {
        accepted.exists()
    });
    assert_eq!(std::fs::read_to_string(&accepted).unwrap(), "Yes, auto-accept edits");
    assert_eq!(pressed()[before.len()..].matches("\\r").count(), 1, "one Enter: {}", pressed());
    assert!(!receipt.is_finished(), "the receipt waits for the hook edge");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &frame("toolu_w1"));
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { key, outcome, reason, seen } => {
            assert_eq!((key.as_str(), outcome.as_str(), reason), (kw.as_str(), "accepted", None));
            assert!(seen.is_some());
        }
        other => panic!("accept_plan: {other:?}"),
    }
    assert!(
        touches(&mut c).iter().any(|t| t.ticket == w && t.action == "accepted plan"),
        "♛ accepted plan on the card"
    );
    assert!(
        feed().lines().any(|l| l.contains("\"cmd\":\"accept_plan\"")
            && l.contains("\"actor\":\"agent\"")
            && l.contains(&format!("\"ticket\":\"{w}\""))
            && l.contains("\"outcome\":\"accepted\"")),
        "{}",
        feed()
    );
    let line = format!("plan accepted by {ka}");
    let detail = |c: &mut TestClient| {
        c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.detail.clone())
    };
    c.await_state(ws, "running the plan", |s| *s == SessionState::Running);
    assert_eq!(detail(&mut c).as_deref(), Some(line.as_str()), "kept through its own edge");

    // ---- the accepted plan's turn wakes the crown when it ends --------------
    stop(&mut c, sa);
    stop(&mut c, ws);
    assert_eq!(detail(&mut c), None, "gone at the next state edge");
    wait_until(std::time::Duration::from_secs(10), "the plan's turn waking the crown", || {
        landed(&format!("{kw} \"mesimon-probe-582 worker\" answered your ask"))
    });
    start(&mut c, sa);

    // ---- an Enter no edge confirms: input_sent, nothing claimed -------------
    std::fs::remove_file(&accepted).unwrap();
    start(&mut c, ws);
    show_plan(&mut c, &kw, ws, "toolu_w2");
    let receipt = call(&mut c, &kw, "toolu_w2");
    wait_until(std::time::Duration::from_secs(10), "the Enter in W's pane", || accepted.exists());
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { outcome, reason, .. } => {
            assert_eq!((outcome.as_str(), reason), ("input_sent", None))
        }
        other => panic!("accept_plan unconfirmed: {other:?}"),
    }
    assert_ne!(detail(&mut c).as_deref(), Some(line.as_str()), "no claim without the edge");
    assert!(
        feed()
            .lines()
            .any(|l| l.contains("\"cmd\":\"accept_plan\"")
                && l.contains("\"outcome\":\"input_sent\""))
    );
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &frame("toolu_w2"));
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    stop(&mut c, ws);

    // ---- a person answers first: the crown typed nothing, and says so -------
    start(&mut c, ws);
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &frame("toolu_w3"));
    c.await_state(ws, "on its plan, not drawn yet", at_plan);
    let before = pressed();
    let receipt = call(&mut c, &kw, "toolu_w3");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!receipt.is_finished(), "no dialog on the screen: the press waits");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &frame("toolu_w3"));
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { outcome, reason, .. } => {
            assert_eq!(
                (outcome.as_str(), reason.as_deref()),
                ("unknown", Some("a_person_answered"))
            )
        }
        other => panic!("accept_plan after a person: {other:?}"),
    }
    assert_eq!(pressed(), before, "the crown pressed nothing");
}

/// T-584: the crown picks a tier by the person's words. `list_board` lists
/// the tiers a ticket may start on, each with the words the person wrote on
/// when to use it, to every agent; the crown files a ticket on one and its
/// start launches with that tier's flags, and a start that names a tier
/// switches the ticket to it first and says so in the receipt. A worker may
/// not pick, a tier that does not resolve is refused with the ids it could
/// have named, and a ticket a person started keeps the person's tier.
#[test]
fn the_crown_picks_a_tier_by_the_persons_words() {
    use mesimon_core::board::AgentProvider;
    use mesimon_core::tier::{Effort, Tier, TierScope};
    const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";
    let Some(h) = Harness::boot_with_env("crown_tier", Some(STUB), &[("MESIMON_NO_TAG_SEED", "1")])
    else {
        return;
    };
    // A repository, so the crown's starts can each take a worktree (T-583:
    // the person's P holds the checkout, and then the crown's own F).
    init_repo(&h.repo, "a.txt", "hello\n");
    let mut c = h.client("crown_tier");
    let tier = |id: &str, name: &str, model: &str, effort, words: &str| Tier {
        id: id.into(),
        name: name.into(),
        provider: AgentProvider::ClaudeCode,
        model: model.into(),
        effort,
        description: words.into(),
    };
    // The person's tiers, one on each layer, with their words on each.
    let quick = tier("01QUICK", "quick", "sonnet", Effort::Low, "docs, renames, one-file fixes");
    let deep = tier("01DEEP", "deep", "opus", Effort::Max, "cross-crate refactors");
    for (scope, t) in [(TierScope::Machine, quick), (TierScope::Board, deep)] {
        assert!(matches!(c.request(Command::SaveTier { scope, tier: t }), Response::Ok));
    }
    let flag = |c: &mut TestClient, ticket: ulid::Ulid, name: &str| -> Option<String> {
        let board = c.board();
        let argv = &board.live_agent(ticket).expect("a seat").argv;
        argv.iter().position(|a| a == name).and_then(|at| argv.get(at + 1).cloned())
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "split the writer");
    let p = create(&mut c, "a person's own");
    let (kw, kp) = (key_of(&mut c, w), key_of(&mut c, p));
    let sa = spawn(&mut c, a);
    let sp = spawn(&mut c, p);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));

    // ---- list_board: the tiers in the list's order, the person's words on each
    let listed = |c: &mut TestClient, session| match c
        .send(Principal::Agent { session }, Command::AgentListBoard)
    {
        Response::AgentBoard { board } => board.tiers,
        other => panic!("list_board: {other:?}"),
    };
    let tiers = listed(&mut c, sa);
    let rows: Vec<(&str, &str, bool)> =
        tiers.iter().map(|t| (t.name.as_str(), t.description.as_str(), t.is_default)).collect();
    assert_eq!(
        rows,
        [
            ("claude", "", true),
            ("quick", "docs, renames, one-file fixes", false),
            ("deep", "cross-crate refactors", false)
        ]
    );
    assert_eq!((tiers[2].model.as_str(), tiers[2].effort), ("opus", Effort::Max));
    assert_eq!(listed(&mut c, sp), tiers, "board data: a worker reads it too");

    let create_on = |c: &mut TestClient, session, title: &str, pick: &str| {
        c.send(
            Principal::Agent { session },
            Command::AgentCreateTicket {
                title: title.into(),
                column: None,
                description: None,
                tags: Vec::new(),
                idempotency_key: None,
                tier: Some(pick.into()),
                workspace: Some("worktree".into()),
            },
        )
    };
    // ---- a worker's ticket is a person's to pick up, tier and all -------------
    match create_on(&mut c, sp, "a worker's idea", "01QUICK") {
        Response::Err { message } => assert!(message.contains("crown's to pick"), "{message}"),
        other => panic!("a worker picking a tier: {other:?}"),
    }
    assert!(!c.board().tickets.iter().any(|t| t.title == "a worker's idea"), "nothing filed");
    // ---- a tier that does not resolve names the ones that do ------------------
    match create_on(&mut c, sa, "nowhere", "huge") {
        Response::Err { message } => {
            for word in ["no tier huge", "claude", "01QUICK (quick)", "01DEEP (deep)"] {
                assert!(message.contains(word), "{word}: {message}");
            }
        }
        other => panic!("an unknown tier: {other:?}"),
    }

    // ---- the crown files on a tier, and the start launches on it --------------
    let kf = match create_on(&mut c, sa, "rename the field", "01QUICK") {
        Response::AgentCreated { key, .. } => key,
        other => panic!("create_ticket with a tier: {other:?}"),
    };
    let f = c.board().ticket_by_key(&kf).unwrap().id;
    assert_eq!(c.board().ticket(f).unwrap().tier.as_deref(), Some("01QUICK"));
    let start = |c: &mut TestClient, key: &str, pick: Option<&str>| {
        let v = read(c, sa, key).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: key.into(),
                seen: v.seen,
                plan: false,
                tier: pick.map(str::to_string),
                workspace: Some("worktree".into()),
            },
        )
    };
    match start(&mut c, &kf, None) {
        Response::AgentStarted { session_started, tier, .. } => {
            assert!(!session_started, "waiting for its worktree first");
            assert_eq!(tier, "quick", "the receipt names the tier it launched on");
        }
        other => panic!("start_agent: {other:?}"),
    }
    let landed = |c: &mut TestClient, t: ulid::Ulid| {
        wait_until(std::time::Duration::from_secs(15), "the worktree start to land", || {
            c.board().live_agent(t).is_some()
        })
    };
    landed(&mut c, f);
    assert_eq!(flag(&mut c, f, "--model").as_deref(), Some("sonnet"));
    assert_eq!(flag(&mut c, f, "--effort").as_deref(), Some("low"));

    // ---- a start that names a tier picks it first, by name as well as id ------
    match start(&mut c, &kw, Some("Deep")) {
        Response::AgentStarted { tier, .. } => assert_eq!(tier, "deep"),
        other => panic!("start_agent with a tier: {other:?}"),
    }
    landed(&mut c, w);
    assert_eq!(c.board().ticket(w).unwrap().tier.as_deref(), Some("01DEEP"));
    assert_eq!(flag(&mut c, w, "--model").as_deref(), Some("opus"));
    assert_eq!(flag(&mut c, w, "--effort").as_deref(), Some("max"));
    assert_eq!(c.board().live_agent(w).unwrap().tier, "01DEEP");

    // ---- a ticket a person started keeps the person's tier --------------------
    let _ = c.request(Command::KillSession { id: sp });
    wait_until(std::time::Duration::from_secs(10), "the person's agent gone", || {
        c.board().live_agent(p).is_none()
    });
    match start(&mut c, &kp, Some("01DEEP")) {
        Response::Err { message } => {
            assert!(message.contains("a person started"), "{message}");
            assert!(message.contains("(claude)"), "names the person's tier: {message}");
        }
        other => panic!("the crown's tier over a person's start: {other:?}"),
    }
    assert_eq!(c.board().ticket(p).unwrap().tier, None, "the pick is untouched");
    assert!(c.board().live_agent(p).is_none(), "and nothing started");
    // The person's own tier is the crown's to start it on again.
    match start(&mut c, &kp, Some("claude")) {
        Response::AgentStarted { tier, .. } => assert_eq!(tier, "claude"),
        other => panic!("a start on the person's own tier: {other:?}"),
    }
    landed(&mut c, p);
    assert_eq!(flag(&mut c, p, "--model"), None, "the built-in passes no model");
}

/// A worker idle at its composer with background tasks still running
/// (T-599): words reach it, the crown is told once, and nothing is decided
/// by time. A person's queued prompt and the crown's ask land on it while
/// its tasks run; its answer, given with the tasks still running, wakes the
/// crown as any answer does; past the lingering grace the crown is woken
/// once (`has been idle with 3 background tasks`) and `get_ticket` says
/// `background`; no second wake without a foreground turn; and the board
/// never touches the seat — still `Idle{Background}`, still `Busy` to the
/// train. The worker's own `get_ticket` says who merges under a crown.
#[test]
fn a_worker_idle_with_background_tasks_takes_words_and_the_crown_is_told() {
    use mesimon_core::board::StopReason;
    let Some(h) = Harness::boot_with_env(
        "crown_linger",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_LINGER_MS", "6000"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_linger");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let send = |sid: uuid::Uuid, event: &str, body: &str| {
        hook_send(&hook_sock, &sid.to_string(), event, body);
    };
    let backgrounded = SessionState::Idle { stop_reason: StopReason::Background };
    // A turn that ends with three background commands still running.
    let leave_tasks = |c: &mut TestClient, sid: uuid::Uuid| {
        let mut rows = Vec::new();
        for n in 1..=3 {
            send(
                sid,
                "PostToolUse",
                &format!(
                    r#"{{"tool_name":"Bash","tool_response":{{"stdout":"","stderr":"","backgroundTaskId":"loop{n}"}}}}"#
                ),
            );
            rows.push(format!(r#"{{"id":"loop{n}","type":"shell","status":"running"}}"#));
        }
        send(
            sid,
            "Stop",
            &format!(r#"{{"stop_hook_active":false,"background_tasks":[{}]}}"#, rows.join(",")),
        );
        c.await_state(sid, "idle with its tasks", |s| {
            *s == SessionState::Idle { stop_reason: StopReason::Background }
        });
    };

    let a = create(&mut c, "coordinate");
    let w = create(&mut c, "mesimon-probe-94 waits in the background");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = spawn(&mut c, a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownSends { on: true }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    send(sa, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    send(sa, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    wait_attached(&mut c, w);
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The worker reads who merges under a crown; nobody else reads it.
    match c.send(Principal::Agent { session: ws }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.under_crown.as_deref(), Some(mesimon_core::mcp::WORKER_UNDER_CROWN))
        }
        other => panic!("the worker's get_ticket: {other:?}"),
    }
    assert!(read(&mut c, sa, &kw).unwrap().under_crown.is_none());

    // ---- 1. a person's queued prompt lands while the tasks run -------------
    send(ws, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    leave_tasks(&mut c, ws);
    let view = read(&mut c, sa, &kw).unwrap().background.expect("background on get_ticket");
    assert_eq!(view.tasks, 3);
    let rec = c.board().live_agent(w).cloned().unwrap();
    assert_eq!(rec.tasks_running, Some(3), "the card's count");
    match c.request(Command::PromptSession {
        ticket: w,
        text: "mesimon-probe-94 person".into(),
        queued: true,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
    }) {
        Response::Queued { .. } | Response::Ok => {}
        other => panic!("queue a person's ask: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the person's words to land", || {
        !lines_with("mesimon-probe-94 person").is_empty()
    });
    send(ws, "UserPromptSubmit", r#"{"prompt":"mesimon-probe-94 person"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    leave_tasks(&mut c, ws);

    // ---- 2. the crown's ask lands; its answer, tasks running, wakes it ------
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-94 crown: are you done".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
        },
    ) {
        Response::AgentAsked { held_for_person: false, .. } => {}
        other => panic!("ask_agent: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the crown's words to land", || {
        !lines_with("mesimon-probe-94 crown").is_empty()
    });
    send(ws, "UserPromptSubmit", r#"{"prompt":"are you done"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    leave_tasks(&mut c, ws);
    let worker = format!("{kw} \"mesimon-probe-94 waits in the background\"");
    wait_until(std::time::Duration::from_secs(10), "the answer's wake", || {
        !lines_with(&format!("{worker} answered your ask")).is_empty()
    });
    send(sa, "UserPromptSubmit", r#"{"prompt":"wake"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    send(sa, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // ---- 3. lingering: once after the grace, never decided ------------------
    let lingering = format!("{worker} has been idle with 3 background tasks for ");
    assert!(lines_with(&lingering).is_empty(), "silent inside the grace");
    wait_until(std::time::Duration::from_secs(20), "the lingering wake", || {
        lines_with(&lingering).len() == 1
    });
    send(sa, "UserPromptSubmit", r#"{"prompt":"wake"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    send(sa, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    std::thread::sleep(std::time::Duration::from_millis(7000));
    assert_eq!(lines_with(&lingering).len(), 1, "no second wake without a foreground turn");
    let rec = c.board().live_agent(w).cloned().unwrap();
    assert_eq!(rec.state, backgrounded, "nothing parked, killed or moved by time");
    let board = c.board();
    assert_eq!(
        mesimon_core::train::seat(&board, w),
        mesimon_core::train::Seat::Busy,
        "never merged under a worker that may be mid-work"
    );
    let view = read(&mut c, sa, &kw).unwrap().background.expect("still background");
    assert!(view.since_secs.is_some_and(|s| s >= 6), "{view:?}");
}
