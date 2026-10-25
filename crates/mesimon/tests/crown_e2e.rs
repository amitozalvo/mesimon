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
//! on a worker the crown parked is that worker's wake. The crown watches a
//! ticket a person started only where Settings → Agents → Crown watches
//! tickets is on (T-712): the board then wakes it for that ticket as for a
//! worker it started, and the watch ends at the merge.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::{
    Confidence, CrownMode, FailReason, MergeInto, SessionKind, SessionState, WorkspaceStrategy,
};
use mesimon_core::command::{
    AgentTicketView, AskRoad, AskWake, Command, CrownTouch, Deliver, PendingAction, Response,
    StartPrompt,
};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

/// The two lines a turn's end after a crown ask can wake the crown with
/// (T-938), for `head` (the worker's key and title): a receipt of the
/// agent's named the words (the mod's `entered`), or none did (a paste, a
/// fill) and the line says only that they were sent.
fn ask_ends(head: &str) -> [String; 2] {
    [
        format!("{head} ended its turn after your ask went in"),
        format!("{head} ended a turn after ask "),
    ]
}

fn key_of(c: &mut TestClient, id: ulid::Ulid) -> String {
    c.board().ticket(id).unwrap().short_key.clone()
}

fn read(c: &mut TestClient, session: uuid::Uuid, key: &str) -> Result<AgentTicketView, String> {
    match c.send(
        Principal::Agent { session },
        Command::AgentReadTicket { key: key.into(), plan: false },
    ) {
        Response::AgentTicket { ticket } => Ok(ticket),
        Response::Err { message } => Err(message),
        other => panic!("read {key}: {other:?}"),
    }
}

/// The words the board shows under `ticket`'s title from the crown (T-783).
fn said_on(c: &mut TestClient, ticket: ulid::Ulid) -> Option<String> {
    match c.request(Command::Snapshot) {
        Response::Board { crown_said, .. } => {
            crown_said.into_iter().find(|s| s.ticket == ticket).map(|s| s.words)
        }
        other => panic!("snapshot: {other:?}"),
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
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
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

    let a = c.create_ticket("triage the board");
    let b = c.create_ticket("fix the thing");
    let d = c.create_ticket("later");
    let (ka, kb, kd) = (key_of(&mut c, a), key_of(&mut c, b), key_of(&mut c, d));
    let sa = c.spawn_claude(a);
    let sb = c.spawn_claude(b);

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
    assert_eq!(own.crowned, Some(false), "its own read says it either way");
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
    // T-908: under a crown, every keyed road a worker reaches for says who
    // reads another ticket, that the crown copies what is needed here, and
    // that a raised hand is the road. B's agent is a person's, unwatched,
    // so its hand reaches the person alone.
    let reach = |key: &str| -> Vec<Command> {
        let key = Some(key.to_string());
        vec![
            Command::AgentReadTicket { key: key.clone().unwrap(), plan: false },
            Command::AgentReadNote { note: ulid::Ulid::new(), key: key.clone() },
            Command::AgentReadAttachment { attachment: ulid::Ulid::new(), key: key.clone() },
            Command::AgentReadShell { shell: uuid::Uuid::new_v4(), key: key.clone() },
            Command::AgentWriteNote { note: None, text: "x".into(), key: key.clone() },
            Command::AgentTagTicket { name: "x".into(), group: None, remove: false, key },
        ]
    };
    for command in reach(&kd) {
        let what = format!("{command:?}");
        match c.send(Principal::Agent { session: sb }, command) {
            Response::Err { message } => assert_eq!(
                message,
                format!(
                    "only the crown ({ka}) reads or changes another ticket ({kd}); the crown \
                     can copy what is needed onto this ticket, and raise_hand on this ticket, \
                     naming what is missing, reaches the person."
                ),
                "{what}"
            ),
            other => panic!("{what}: {other:?}"),
        }
    }
    let own = read(&mut c, sa, &ka).unwrap();
    assert_eq!(own.crowned, Some(true), "get_ticket says the caller wears it");
    // T-916: what the crown is for, then (T-537) how the board wakes it, so
    // it arms no monitor of its own.
    assert_eq!(own.crown, Some(mesimon_core::mcp::crown_view()));
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
    // T-916: another ticket's view says nothing of a crown, which a Codex
    // crown took for its own `false`.
    assert_eq!(view.crowned, None);
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
    assert!(!c.is_archived(d));
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
    assert!(c.is_archived(d));
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
    assert!(!c.is_archived(d));
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
    assert!(!c.is_archived(d));

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
    let e1 = c.create_ticket("first start");
    let e2 = c.create_ticket("second start");
    let e3 = c.create_ticket("third start");
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
                expect_secs: None,
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
            expect_secs: None,
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
        Response::AgentStarted {
            key,
            session_started,
            budget_left,
            woken,
            workspace,
            prompt,
            ..
        } => {
            assert_eq!(key, k1);
            assert!(!session_started, "waiting for its worktree first");
            assert!(!woken);
            // T-920: a start's first prompt is its brief, once the cut lands.
            assert_eq!(prompt, Some(StartPrompt::Brief));
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
    // T-908: the crown's own worker reaching for another ticket reads that
    // its raised hand wakes the crown as well as reaching the person.
    match c.send(
        Principal::Agent { session: started.id },
        Command::AgentReadNote { note: ulid::Ulid::new(), key: Some(kb.clone()) },
    ) {
        Response::Err { message } => {
            assert!(message.starts_with(&format!("only the crown ({ka})")), "{message}");
            assert!(message.ends_with("reaches the person and the crown."), "{message}");
        }
        other => panic!("a worker reading another ticket's note: {other:?}"),
    }
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
    let e5 = c.create_ticket("parked start");
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
    let e4 = c.create_ticket("never started");
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
                    expect_secs: None,
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
        Response::AgentAsked {
            key,
            replaced,
            seen,
            held_for_person,
            held_because,
            road,
            wake,
            ..
        } => {
            assert_eq!(key, kb);
            assert!(!replaced);
            assert_eq!(wake, None, "B's agent has a pane: the words wake nothing");
            // T-550: a person started B's agent, so the board holds it
            // for them whatever the crown's mode, and says so.
            assert!(held_for_person);
            assert_eq!(road, Some(mesimon_core::command::AskRoad::HeldForPerson));
            assert!(held_because.is_some_and(|w| w.contains("a person started")));
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
    // T-938: the record is the new ask's now, on the board.
    assert_eq!(dropped(&mut c).map(|d| d.0), Some("queued".into()), "a new ask replaces it");
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
        uploads: Vec::new(),
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
    assert_eq!(dropped(&mut c).map(|d| d.0), Some("queued".into()), "the third ask's record");
    assert!(!landed("mesimon-probe-66"));
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: b }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the sent words to land", || {
        landed("mesimon-probe-66 now go")
    });
    // The by-hand send is a paste of mesimon's own and owes its ack like
    // every other (T-244): the card says `sending` until Claude takes it.
    let p = pending_of(&mut c, Some(b));
    assert!(p.iter().all(|p| p.in_flight), "only the paste's own mark remains: {p:?}");
    // Pasted and not yet confirmed; the mod pass's stand-in engine reports
    // `entered` with the words, which hands them at once.
    let want = if test_road() == "mod" { "handed" } else { "sent" };
    assert_eq!(dropped(&mut c).map(|d| d.0), Some(want.into()));
    hook_send(&h.paths.hook_sock(), &sb.to_string(), "UserPromptSubmit", "{}");
    wait_until(std::time::Duration::from_secs(5), "the paste's ack to clear it", || {
        pending_of(&mut c, Some(b)).is_empty()
    });
    // A prompt hook names no words (T-938): on the hook set the paste stays
    // `sent`; the mod pass's `entered` handed it.
    let want = if test_road() == "mod" { "handed" } else { "sent" };
    assert_eq!(dropped(&mut c).map(|d| d.0), Some(want.into()));
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

    // ---- the shim: twenty-four tools, and `get_ticket` with a key ----------------
    let mut shim = Shim::start(&sock, sa);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    let listed = shim.rpc("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 24);
    let r = shim.call("watch_ticket", json!({ "key": kb }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
    let r = shim.call("merge_ticket", json!({ "key": kb }));
    assert_eq!(r["isError"], true, "seen is required by the tool: {r}");
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
    assert!(other.get("crowned").is_none(), "another ticket's view leaves it out: {other}");
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
    let e = c.create_ticket("shipped");
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
    assert!(c.is_archived(e));
    assert!(c.board().crown.is_none(), "the offer's archive drops the crown");
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(!file.contains("crown ="), "{file}");
}

/// The stub every wake test runs: each line it reads is appended to one file
/// beside it, so the crown's sentences and the workers' words are all there,
/// and to one per ticket (`got-<key>.txt`), so a count of what reached one
/// pane reads that pane alone (T-937).
const RECORDING_STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; \
                              printf '%s\\n' \"$line\" \
                              >> \"$(dirname \"$0\")/got-${MESIMON_TICKET:-none}.txt\"; done\n";

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
    // The words in the crown's pane are not the wake's receipt: on the mod
    // road the stand-in engine types them and only then relays its
    // `entered`, through a `mesimon hook` of its own, and the row is owed
    // until that lands (T-971).
    let received = |c: &mut TestClient, t: ulid::Ulid| {
        wait_until(std::time::Duration::from_secs(10), "the wake's receipt", || {
            wake_rows(c, t).is_empty()
        });
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-71 worker");
    let (ka, kw1) = (key_of(&mut c, a), key_of(&mut c, w1));
    let sa = c.spawn_claude(a);
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
                expect_secs: None,
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
    received(&mut c, a);
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
    received(&mut c, a);
    // The crown's card says the news, its titles left to the cards (T-783).
    assert_eq!(said_on(&mut c, a), Some(format!("{kw1} delivered (commit {h1})")));
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
        uploads: Vec::new(),
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
    let w2 = c.create_ticket("mesimon-probe-72 worker");
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
    received(&mut c, a);

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
    // Told, the crown holds the question and the worker's hand is down
    // (T-861): one card asks a person one thing, never two.
    received(&mut c, a);
    assert!(!c.board().ticket(w1).unwrap().hand_raised(), "the hand the crown was told of is down");
    // …and the reason is the crown's to read where the sentence sent it
    // (T-922): `told_hand` on its keyed `get_ticket`.
    let told = read(&mut c, sa, &kw1).unwrap().told_hand.map(|h| h.reason);
    assert_eq!(told.as_deref(), Some("mesimon-secret-73 need the sandbox key"));

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
        uploads: Vec::new(),
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
    assert!(c.is_archived(w2));
    assert!(!c.board().crown_started().iter().any(|s| s.ticket == w2), "the seat stays free");
    assert!(
        c.sessions_of(w2).iter().any(|s| s.id == ws2 && s.state == SessionState::Sleeping),
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

/// A recording stub whose composer keeps the cursor in its box, as Claude
/// Code's does: what a person types there is a draft the screen shows
/// (`composer::read` reads it `Holding`), and each line read is recorded,
/// then the box painted empty again.
const BOX_STUB: &str =
    "#!/bin/sh\nstty -icanon 2>/dev/null\npaint() { printf '\\033[2J\\033[999;1H\\033[3A\
                        ────────────────────\\n❯ \\n────────────────────\\n  ? for shortcuts\
                        \\033[999;1H\\033[2A\\033[3G'; }\npaint\ncat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; paint; done\n";

/// The crown's wake row while a client attached to its pane holds it, at
/// the default bound (T-945): ten minutes after tmux last recorded activity.
const WHEN_YOU_LEAVE: &str = "leave or 10m idle";

/// A person at the crown's box (T-918): the wake is not pasted into what
/// they may be typing. It waits while a tmux client has the crown's pane
/// open and active (`leave or 10m idle`),
/// then while the box holds the draft they left
/// (`after your draft`), and goes in on its own line once the box is
/// empty — the person's words sent as they wrote them, never joined. The
/// paste road's case, so the hook set's road under both passes: a mod that
/// speaks `submit` takes the wake past the box (T-931, below).
#[test]
fn the_crown_wake_waits_for_a_person_at_the_crown_s_box() {
    let Some(h) = Harness::boot_bare(
        "crown_attended",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_attended");
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
    let wake_rows = |c: &mut TestClient, t: ulid::Ulid| -> Vec<mesimon_core::command::Pending> {
        pending_of(c, Some(t))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };
    let held = |c: &mut TestClient, t: ulid::Ulid| -> Option<String> {
        wake_rows(c, t).first().and_then(|p| p.held.clone())
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-91 worker");
    let (ka, kw1) = (key_of(&mut c, a), key_of(&mut c, w1));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The spawn left the title in the crown's box, unsent: that is a draft
    // too, and it goes first, so the box is empty when the person comes.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the title out of the box", || {
        std::fs::read_to_string(&got).is_ok_and(|g| g.lines().any(|l| l.trim() == "coordinate"))
    });

    // A person comes into the crown's pane: a tmux client on its session.
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });

    // W1 delivers while the person is in there: the crown is idle and
    // would take the words, and the wake waits for the person instead.
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered = format!("{kw1} \"mesimon-probe-91 worker\" delivered (commit {h1}");
    wait_until(std::time::Duration::from_secs(10), "the wake row to wait on the person", || {
        wake_rows(&mut c, a).first().is_some_and(|p| {
            p.held.as_deref() == Some(WHEN_YOU_LEAVE) && p.waits_on == [ka.clone()]
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_with(&delivered), 0, "{}", std::fs::read_to_string(&got).unwrap_or_default());
    assert_eq!(held(&mut c, a).as_deref(), Some(WHEN_YOU_LEAVE), "still waiting");

    // They type half a prompt and leave it in the box: gone from the pane,
    // their words still are not sent, and the wake keeps waiting.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(10), "the wake row to wait on the draft", || {
        held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_with(&delivered), 0, "{}", std::fs::read_to_string(&got).unwrap_or_default());
    assert_eq!(lines_with("half a thought"), 0, "the draft is theirs to send");

    // They send it: their words go as they wrote them, and the wake follows
    // on a line of its own once the box is empty.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the wake to land after the draft", || {
        lines_with(&delivered) == 1
    });
    let lines = std::fs::read_to_string(&got).unwrap();
    assert!(lines.lines().any(|l| l == "half a thought"), "{lines}");
    assert_eq!(lines_with("half a thought"), 1, "never joined to the wake: {lines}");
    assert!(wake_rows(&mut c, a).is_empty(), "delivered, so nothing is owed");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == a).map(|t| t.action.as_str()),
        Some("woke"),
        "the crown's card lit when the wake was owed"
    );
    // The feed is written a bucket at a time.
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(std::time::Duration::from_secs(5), "crown_wake_sent in the feed", || {
        std::fs::read_to_string(&feed).is_ok_and(|f| f.contains("\"crown_wake_sent\""))
    });
}

/// The idle bound the T-945 tests run on, and the row's words under it.
const IDLE_MS: &str = "5000";
const WHEN_YOU_LEAVE_5S: &str = "leave or 5s idle";

/// A control-mode tmux client on `session`, as the board's focus attaches
/// one, and its name. It sends no input, so tmux records no activity from
/// it after its attach (measured on 3.6a, T-945).
fn attach_client(tmux_sock: &std::path::Path, session: &str) -> (std::process::Child, String) {
    let names = || -> Vec<String> {
        let out = tmux(tmux_sock)
            .args(["list-clients", "-t", session, "-F", "#{client_name}"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect()
    };
    let before = names();
    let child = tmux(tmux_sock)
        .args(["-C", "attach", "-t", session])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    let mut name = None;
    wait_until(std::time::Duration::from_secs(10), "the client on the session", || {
        name = names().into_iter().find(|n| !before.contains(n));
        name.is_some()
    });
    (child, name.unwrap())
}

/// `#{session_activity}`: the epoch second tmux last recorded input from a
/// client of `session`.
fn session_activity(tmux_sock: &std::path::Path, session: &str) -> u64 {
    let out = tmux(tmux_sock)
        .args(["display-message", "-p", "-t", session, "#{session_activity}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().expect("a session_activity")
}

/// A key from `client`, as its terminal would send one (`send-keys -K`), on
/// a root-table binding that does nothing: tmux records the client's
/// activity, and the pane sees no key. Waits until the stamp moves.
fn renew_activity(tmux_sock: &std::path::Path, session: &str, client: &str) -> u64 {
    assert!(tmux(tmux_sock)
        .args(["bind-key", "-n", "F12", "set-option", "-g", "@t945", "1"])
        .status()
        .unwrap()
        .success());
    let was = session_activity(tmux_sock, session);
    let mut now = was;
    wait_until(std::time::Duration::from_secs(5), "the client's activity renewed", || {
        assert!(tmux(tmux_sock)
            .args(["send-keys", "-K", "-c", client, "F12"])
            .status()
            .unwrap()
            .success());
        now = session_activity(tmux_sock, session);
        now > was
    });
    now
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// T-945: a client attached to the crown's pane holds its wake only while
/// tmux has recorded activity from it inside the bound (`5 s` here, ten
/// minutes by default). The row names the bound; a key from either of two
/// clients renews it, read off the stamp tmux reports, never a timer; past
/// it, both clients still attached, the wake goes in once. A second wake
/// meets a draft left with the clients idle: the draft still holds it
/// (`after your draft`), and it goes once the person sends their words.
#[test]
fn an_attached_client_holds_the_crown_wake_only_while_active() {
    let Some(h) = Harness::boot_bare(
        "crown_idle_client",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_PERSON_IDLE_MS", IDLE_MS),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_idle_client");
    let got = h.dir.join("got.txt");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let lines_with = |needle: &str| read_got().lines().filter(|l| l.contains(needle)).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let held = |c: &mut TestClient, t: ulid::Ulid| -> Option<String> {
        pending_of(c, Some(t))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let owed = |c: &mut TestClient, t: ulid::Ulid| {
        pending_of(c, Some(t))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
    };
    let bound: u64 = IDLE_MS.parse().unwrap();

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-945 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The spawn's title, left in the crown's box, goes first.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the title out of the box", || {
        read_got().lines().any(|l| l.trim() == "coordinate")
    });

    // Two clients on the crown's session; W1 delivers while they are fresh.
    let (mut one, _) = attach_client(&tmux_sock, &crown);
    let (mut two, two_name) = attach_client(&tmux_sock, &crown);
    let first = session_activity(&tmux_sock, &crown);
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered = format!("{kw1} \"mesimon-probe-945 worker\" delivered (commit {h1}");
    wait_until(std::time::Duration::from_secs(10), "the row names the bound", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE_5S)
    });
    // The second client's key renews the hold before the bound goes by.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let renewed = renew_activity(&tmux_sock, &crown, &two_name);
    assert!(renewed > first, "tmux recorded the key: {first} → {renewed}");
    assert_eq!(read_got().lines().filter(|l| l.contains("F12")).count(), 0, "no key in the box");
    // Past the first stamp's bound and short of the renewed one's: held.
    let first_expiry = (first + 1) * 1000 + bound;
    let renewed_expiry = (renewed + 1) * 1000 + bound;
    while epoch_ms() < first_expiry + 1000 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(epoch_ms() + 500 < renewed_expiry, "the window is still open");
    assert_eq!(lines_with(&delivered), 0, "held past the first bound: {}", read_got());
    assert_eq!(held(&mut c, a).as_deref(), Some(WHEN_YOU_LEAVE_5S), "renewed");

    // Past the renewed bound, both clients still in: the wake goes, once.
    wait_until(std::time::Duration::from_secs(15), "the wake past the bound", || {
        lines_with(&delivered) == 1
    });
    let at = epoch_ms();
    assert!(at + 1000 >= renewed_expiry, "not before the bound: {at} < {renewed_expiry}");
    assert_eq!(session_activity(&tmux_sock, &crown), renewed, "no input since the key");
    assert_eq!(
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .unwrap()
            .stdout
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .count(),
        2,
        "both clients still attached"
    );
    // The crown's turn on it.
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "nothing owed", || !owed(&mut c, a));

    // A draft left in the box (typed as the board types, which tmux records
    // as no client's activity), the clients idle: the draft holds the wake.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    let h2 = commit(&h.repo, "w2.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered2 = format!("{kw1} \"mesimon-probe-945 worker\" delivered (commit {h2}");
    wait_until(std::time::Duration::from_secs(10), "the draft holds the wake", || {
        held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_with(&delivered2), 0, "nothing pasted into their draft: {}", read_got());
    assert_eq!(lines_with("half a thought"), 0, "the draft is theirs to send");
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the wake after the draft", || {
        lines_with(&delivered2) == 1
    });
    assert_eq!(lines_with("half a thought"), 1, "never joined: {}", read_got());
    one.kill().unwrap();
    let _ = one.wait();
    two.kill().unwrap();
    let _ = two.wait();
}

/// `BOX_STUB`, with two things more for T-966: each line read is also
/// written to `got-at.txt` after an epoch ms taken once the line was read
/// and recorded (a receiver's stamp, later than the read and the paste it
/// dates, never earlier), and a line `@absent` paints a screen with no
/// composer on it (a dialog's shape, `Composer::Absent`) until the next
/// line, which paints the box again.
const BOX_AT_STUB: &str = r#"#!/bin/sh
stty -icanon 2>/dev/null
paint() { printf '\033[2J\033[999;1H\033[3A────────────────────\n❯ \n────────────────────\n  ? for shortcuts\033[999;1H\033[2A\033[3G'; }
gone() { printf '\033[2J\033[1;1HDo you want to proceed?\n  1. Yes\n  2. No\n'; }
paint
cat | while IFS= read -r line; do
  printf '%s\n' "$line" >> "$(dirname "$0")/got.txt"
  printf '%s %s\n' "$(perl -MTime::HiRes=time -e 'printf "%d", time*1000')" "$line" >> "$(dirname "$0")/got-at.txt"
  case "$line" in @absent) gone ;; *) paint ;; esac
done
"#;

/// T-966: a queued ask the hook road would type into a worker's pane waits
/// for a client attached there only while tmux has recorded activity from
/// it inside the bound (`5 s` here), as the crown's wake does (T-945), and
/// the row says why. Past the bound, the client still attached, the words
/// go in once, the drain never looping on its own refusal; the stub's stamp
/// of them is at or after the expiry, a receiver's stamp and no proof of
/// when the paste went, which `crownwake::client_hold`'s unit pins to the
/// millisecond. A second ask meets a draft left with the client idle: the
/// draft holds it (`after your draft`), and it goes on its own line once the
/// person sends their words. A third meets a box read as no composer (round
/// 1): it holds (`box unknown ∙ leave`), and goes once the box reads empty
/// again, once. The integration of the row's words; that a sample's own
/// change of them pushes is `queued_person_push_tests` in the daemon (round 2).
#[test]
fn a_queued_ask_goes_past_an_idle_client_only_into_an_empty_box() {
    let Some(h) = Harness::boot_bare(
        "crown_ask_idle_client",
        Some(BOX_AT_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_PERSON_IDLE_MS", IDLE_MS),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let mut c = h.client("crown_ask_idle_client");
    let got = h.dir.join("got.txt");
    let got_at = h.dir.join("got-at.txt");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let lines_with = |needle: &str| read_got().lines().filter(|l| l.contains(needle)).count();
    // Lines the pane read exactly as `line`: a line that joined two sends
    // matches neither.
    let lines_are = |line: &str| read_got().lines().filter(|l| l.trim() == line).count();
    // When the pane read `line`, by the stub's own clock.
    let read_at = |line: &str| -> Option<u64> {
        std::fs::read_to_string(&got_at).unwrap_or_default().lines().find_map(|l| {
            let (ms, rest) = l.split_once(' ')?;
            (rest.trim() == line).then(|| ms.parse().ok()).flatten()
        })
    };
    let feed_count = |cmd: &str| {
        std::fs::read_to_string(&feed_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&format!("\"cmd\":\"{cmd}\"")))
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
    let bound: u64 = IDLE_MS.parse().unwrap();

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-966 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    // A launch's words left in a box go first, out of the way of what is
    // read there below.
    let enter = |sid: &str| {
        assert!(tmux(&tmux_sock)
            .args(["send-keys", "-t", sid, "Enter"])
            .status()
            .unwrap()
            .success());
    };
    // A line typed into a pane as the board types, which tmux records as no
    // client's activity.
    let type_line = |sid: &str, line: &str| {
        assert!(tmux(&tmux_sock)
            .args(["send-keys", "-t", sid, "-l", line])
            .status()
            .unwrap()
            .success());
    };
    enter(&crown);
    wait_until(std::time::Duration::from_secs(10), "the crown's title out of its box", || {
        read_got().lines().any(|l| l.trim() == "coordinate")
    });
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    let sid = c.board().sessions.iter().find(|s| s.id == ws).unwrap().sid16();
    std::thread::sleep(std::time::Duration::from_millis(500));
    enter(&sid);
    wait_until(std::time::Duration::from_secs(10), "W's launch words out of its box", || {
        lines_with("mesimon-probe-966 worker") > 0
    });
    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.expect("the crown's ask");
    let person = |c: &mut TestClient| -> Option<String> {
        pending_of(c, Some(w))
            .into_iter()
            .find(|p| p.action == PendingAction::Ask && !p.in_flight)
            .and_then(|p| p.person)
    };
    // The row word as a TUI would read it, a snapshot after a push: any
    // push, the enqueue's included, so this does not tell which change
    // pushed (the daemon's `queued_person_push_tests` does).
    let pushed = |sub: &mut TestClient, c: &mut TestClient, word: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if sub.next_event(std::time::Duration::from_millis(500)).is_some()
                && person(c).as_deref() == Some(word)
            {
                return;
            }
        }
        panic!("no push brought `{word}`; the row reads {:?}", person(c));
    };
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
                expect_secs: None,
            },
        )
    };
    // W's stop wakes the crown, in the same checkout: the crown takes that
    // wake and goes idle, so nothing but the person holds W's ask.
    let w_stops = |c: &mut TestClient| {
        let woken = feed_count("crown_wake_sent");
        stop(c, ws);
        wait_until(std::time::Duration::from_secs(10), "the crown's wake", || {
            feed_count("crown_wake_sent") > woken
        });
        start(c, sa);
        stop(c, sa);
    };

    // ---- an active client holds the ask, and the row says until when -------
    let first = "mesimon-probe-966a at the idle";
    start(&mut c, ws);
    let (mut client, client_name) = attach_client(&tmux_sock, &sid);
    let _ = ask(&mut c, first);
    assert_eq!(asked(&mut c).status, "queued");
    let kept_before = feed_count("queued_ask_kept");
    // W's turn ends with the ask waiting on it, so the crown is not woken.
    stop(&mut c, ws);
    // A key from the client renews the hold; the pane sees nothing of it.
    let renewed = renew_activity(&tmux_sock, &sid, &client_name);
    let renewed_expiry = (renewed + 1) * 1000 + bound;
    wait_until(std::time::Duration::from_secs(5), "the row names the bound", || {
        person(&mut c).as_deref() == Some(WHEN_YOU_LEAVE_5S)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(epoch_ms() + 500 < renewed_expiry, "the window is still open");
    assert_eq!(lines_with("mesimon-probe-966a"), 0, "held while active: {}", read_got());
    assert_eq!(asked(&mut c).status, "queued");
    assert_eq!(read_got().lines().filter(|l| l.contains("F12")).count(), 0, "no key in the box");

    // ---- past the bound, the client still there: the words go, once -------
    wait_until(std::time::Duration::from_secs(15), "the ask past the bound", || {
        read_at(first).is_some()
    });
    // The stub's stamp, taken after it read the line: at or after the expiry
    // says the words were not recorded early; the release's own boundary is
    // `crownwake::client_hold`'s unit.
    let at = read_at(first).unwrap();
    assert!(at >= renewed_expiry, "stamped before the bound: {at} < {renewed_expiry}");
    assert_eq!(session_activity(&tmux_sock, &sid), renewed, "no input since the key");
    assert!(attach_listed(&tmux_sock, &sid), "the client still attached");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_are(first), 1, "once, on its own line: {}", read_got());
    assert_eq!(lines_with("mesimon-probe-966a"), 1, "nowhere else");
    assert_eq!(asked(&mut c).status, "sent");
    let kept_now = feed_count("queued_ask_kept");
    assert!(kept_now <= kept_before + 1, "the drain did not loop: {kept_before} → {kept_now}");
    // The turn those words started ends.
    start(&mut c, ws);
    w_stops(&mut c);

    // ---- a draft left under the idle client holds the next ask -------------
    let second = "mesimon-probe-966b after the draft";
    let draft = "half a thought";
    let mut sub = h.client("crown_ask_idle_client_sub");
    assert!(matches!(sub.request(Command::Subscribe), Response::Ok));
    type_line(&sid, draft);
    while sub.next_event(std::time::Duration::from_millis(300)).is_some() {}
    let kept_before = feed_count("queued_ask_kept");
    let _ = ask(&mut c, second);
    // No sample has read the box since the clients went idle, so the
    // enqueue's row has no word yet; a later snapshot has the draft's.
    pushed(&mut sub, &mut c, "after your draft");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with("mesimon-probe-966b"), 0, "nothing pasted into their draft");
    assert_eq!(lines_with(draft), 0, "the draft is theirs to send");
    assert_eq!(asked(&mut c).status, "queued");
    let kept_now = feed_count("queued_ask_kept");
    assert!(kept_now <= kept_before + 1, "the drain did not loop: {kept_before} → {kept_now}");
    // The person sends their words; the ask then goes on its own line.
    enter(&sid);
    wait_until(std::time::Duration::from_secs(10), "the ask after the draft", || {
        lines_are(second) == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_are(draft), 1, "the draft as they wrote it: {}", read_got());
    assert_eq!(lines_are(second), 1, "the ask on its own line, once");
    assert_eq!(lines_with(draft), 1, "never joined: {}", read_got());
    assert_eq!(lines_with("mesimon-probe-966b"), 1, "never joined");
    assert_eq!(asked(&mut c).status, "sent");
    start(&mut c, ws);
    w_stops(&mut c);

    // ---- a box that reads as no composer holds the next ask (round 1) -----
    let third = "mesimon-probe-966c past no composer";
    type_line(&sid, "@absent");
    enter(&sid);
    wait_until(std::time::Duration::from_secs(10), "the box gone", || lines_are("@absent") == 1);
    while sub.next_event(std::time::Duration::from_millis(300)).is_some() {}
    let kept_before = feed_count("queued_ask_kept");
    let _ = ask(&mut c, third);
    pushed(&mut sub, &mut c, "box unknown ∙ leave");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with("mesimon-probe-966c"), 0, "nothing typed past an unread box");
    assert_eq!(asked(&mut c).status, "queued");
    let kept_now = feed_count("queued_ask_kept");
    assert!(kept_now <= kept_before + 1, "the drain did not loop: {kept_before} → {kept_now}");
    // The box back, empty: the ask goes, once.
    type_line(&sid, "@shown");
    enter(&sid);
    wait_until(std::time::Duration::from_secs(10), "the ask into the empty box", || {
        lines_are(third) == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(lines_are(third), 1, "once, on its own line: {}", read_got());
    assert_eq!(lines_with("mesimon-probe-966c"), 1, "never joined");
    assert_eq!(asked(&mut c).status, "sent");
    client.kill().unwrap();
    let _ = client.wait();
}

/// A crown whose mod speaks `submit` (T-931) takes its wake at once with a
/// person in its pane and their draft in its box: the submit goes to the
/// model past the box (measured on Claude Code 2.1.309), so nothing holds
/// it, the row never says it waits for them, and the draft stays theirs.
#[test]
fn a_mod_road_crown_takes_its_wake_with_a_person_at_its_box() {
    let Some(h) = Harness::boot_bare(
        "crown_mod_attended",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    std::fs::write(h.dir.join("mod-submit-aside"), "").unwrap();
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_mod_attended");
    let got = h.dir.join("got.txt");
    let submitted = h.dir.join("submitted.txt");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let wake_rows = |c: &mut TestClient, t: ulid::Ulid| -> Vec<mesimon_core::command::Pending> {
        pending_of(c, Some(t))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-931 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // A person in the crown's pane, half a prompt in its box.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // W1 delivers: the wake goes down the crown's mod at once, the person
    // still there and their draft still in the box.
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered = format!("{kw1} \"mesimon-probe-931 worker\" delivered (commit {h1}");
    let t0 = std::time::Instant::now();
    let mut held = Vec::new();
    wait_until(std::time::Duration::from_secs(10), "the wake through the crown's submit", || {
        held.extend(wake_rows(&mut c, a).into_iter().filter_map(|p| p.held));
        read_file(&submitted).contains(&delivered)
    });
    assert!(t0.elapsed() < std::time::Duration::from_secs(4), "at once: {:?}", t0.elapsed());
    // `sending` is the submit out and not yet answered (T-936), the
    // board's own word: no person holds it.
    assert!(held.iter().all(|w| w == "sending"), "nothing held it for the person: {held:?}");
    let frames = read_file(&h.dir.join(format!("mod-{sa}.ndjson")));
    assert!(
        frames
            .lines()
            .any(|l| l.contains("\"submit\"") && l.contains(&format!("delivered (commit {h1}"))),
        "{frames}"
    );
    assert!(
        !read_file(&got).contains("half a thought"),
        "the draft is theirs: {}",
        read_file(&got)
    );
    let screen = tmux(&tmux_sock).args(["capture-pane", "-p", "-t", &crown]).output().unwrap();
    assert!(String::from_utf8_lossy(&screen.stdout).contains("half a thought"), "still in the box");
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(std::time::Duration::from_secs(5), "crown_wake_sent in the feed", || {
        read_file(&feed).contains("\"crown_wake_sent\"")
            && read_file(&feed).contains("\"prompt_by_mod\"")
    });
    client.kill().unwrap();
    let _ = client.wait();
}

/// T-936: a crown's wake goes by its mod's `submit` past a person in its
/// pane (T-931), and Claude Code refuses it after their Esc ("the session
/// was stopped and nobody has sent input since", T-894). Nothing is typed
/// under the person, empty box or not, and what they type next is theirs
/// alone. The wake was not told at the submit: refused, it is owed again,
/// held for the person on the paste road, and nothing is kept unsent.
#[test]
fn a_mod_road_crown_s_refused_wake_is_never_pasted_under_a_person() {
    let Some(h) = Harness::boot_bare(
        "crown_mod_refused",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_mod_refused");
    let got = h.dir.join("got.txt");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // A person in the crown's pane, its box empty; the crown's mod refuses
    // a submit as Claude Code does after their Esc.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    std::fs::write(h.dir.join("mod-stopped-submit"), "").unwrap();

    // W1 delivers: the submit is refused, and nothing is pasted under them.
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered = format!("{kw1} \"mesimon-probe-936 worker\" delivered (commit {h1}");
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(std::time::Duration::from_secs(10), "the refused wake owed again", || {
        read_file(&feed).contains("\"crown_wake_refused\"")
    });
    wait_until(std::time::Duration::from_secs(10), "held for the person in the pane", || {
        pending_of(&mut c, Some(a)).iter().any(|p| {
            p.action == mesimon_core::command::PendingAction::CrownWake
                && p.held.as_deref() == Some(WHEN_YOU_LEAVE)
        })
    });
    assert!(!read_file(&feed).contains("prompt_pasted_after_refusal"));
    // Told at most at the frame's take (T-990), never after its refusal.
    let at_ms = |cmd: &str| -> Vec<u64> {
        read_file(&feed)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|l| l["cmd"] == cmd)
            .filter_map(|l| l["at_ms"].as_u64())
            .collect()
    };
    let refused = at_ms("crown_wake_refused")[0];
    assert!(at_ms("crown_wake_sent").iter().all(|t| *t <= refused), "nothing told after it");
    assert!(c.board().sessions.iter().all(|s| s.id != sa || s.unsent.is_none()), "not unsent");
    // What the person types next is theirs alone.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let screen = tmux(&tmux_sock).args(["capture-pane", "-p", "-t", &crown]).output().unwrap();
    let screen = String::from_utf8_lossy(&screen.stdout).to_string();
    assert!(screen.contains("half a thought") && !screen.contains("delivered"), "{screen}");
    assert!(!read_file(&got).contains(&delivered), "nothing typed: {}", read_file(&got));
    client.kill().unwrap();
    let _ = client.wait();
}

/// T-945 on T-936: a mod-road crown's refused wake goes back to the paste
/// road, held for the client in its pane while tmux records activity from
/// it, then for the draft the person left. Past the bound the client holds
/// nothing, but the draft still does: nothing is typed into it, and once
/// they send it and the box reads empty, the wake is pasted once, the
/// client still attached.
#[test]
fn a_refused_mod_wake_goes_past_an_idle_client_only_into_an_empty_box() {
    let Some(h) = Harness::boot_bare(
        "crown_mod_refused_idle",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
            ("MESIMON_PERSON_IDLE_MS", IDLE_MS),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_mod_refused_idle");
    let got = h.dir.join("got.txt");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let held = |c: &mut TestClient, t: ulid::Ulid| -> Option<String> {
        pending_of(c, Some(t))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-945 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The spawn's title, left in the crown's box, goes first.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the title out of the box", || {
        read_file(&got).lines().any(|l| l.trim() == "coordinate")
    });
    let (mut client, _) = attach_client(&tmux_sock, &crown);
    std::fs::write(h.dir.join("mod-stopped-submit"), "").unwrap();
    let h1 = commit(&h.repo, "w1.txt");
    start(&mut c, ws1);
    stop(&mut c, ws1);
    let delivered = format!("{kw1} \"mesimon-probe-945 worker\" delivered (commit {h1}");
    let feed = h.paths.state_dir.join("activity.jsonl");
    wait_until(std::time::Duration::from_secs(10), "the refused wake owed again", || {
        read_file(&feed).contains("\"crown_wake_refused\"")
    });
    wait_until(std::time::Duration::from_secs(10), "held for the client", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE_5S)
    });
    // Their draft, typed as the board types: no client's activity.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    // Past the bound the draft holds it, the client still in.
    wait_until(std::time::Duration::from_secs(15), "the draft holds it", || {
        held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!read_file(&got).contains(&delivered), "nothing typed: {}", read_file(&got));
    assert!(!read_file(&got).contains("half a thought"), "no Enter sent their draft");
    // They send it: the box reads empty, and the wake is pasted once.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(15), "the wake pasted into the empty box", || {
        read_file(&got).lines().filter(|l| l.contains(&delivered)).count() == 1
    });
    let got_now = read_file(&got);
    assert!(got_now.lines().any(|l| l == "half a thought"), "{got_now}");
    assert!(got_now.lines().all(|l| !(l.contains("half a thought") && l.contains(&delivered))));
    assert!(attach_listed(&tmux_sock, &crown), "the client still attached");
    client.kill().unwrap();
    let _ = client.wait();
}

/// Is any tmux client on `session`?
fn attach_listed(tmux_sock: &std::path::Path, session: &str) -> bool {
    tmux(tmux_sock)
        .args(["list-clients", "-t", session, "-F", "#{client_name}"])
        .output()
        .is_ok_and(|o| !o.stdout.is_empty())
}

/// T-936, Astra's raised-hand specimen. A worker's hand wakes a mod-road
/// crown that a person's Esc stopped (`Idle{Interrupted}`, the one state in
/// which Claude Code refuses a plugin's input, T-894). While the person is
/// in the crown's pane the wake is held as before T-931: no submit and no
/// paste, the worker's hand still up and the wake still owed. Once they
/// leave, one paste into the empty box delivers it, and only then does the
/// hand come down. And a submit taken (told there, its hand down, T-990)
/// and still pending as a person comes in and types, refused after their
/// Esc, types nothing under them: the wake is owed again, held for them,
/// and one safe delivery after they send their words and leave settles it.
#[test]
fn a_crown_a_person_stopped_holds_its_wake_and_its_hand() {
    let Some(h) = Harness::boot_bare(
        "crown_mod_esc",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "1500"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_mod_esc");
    let got = h.dir.join("got.txt");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let end_turn = SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| *s == end_turn);
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let wake_rows = |c: &mut TestClient, t: ulid::Ulid| -> Vec<mesimon_core::command::Pending> {
        pending_of(c, Some(t))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };
    let attach = |crown: &str| {
        let client = tmux(&tmux_sock)
            .args(["-C", "attach", "-t", crown])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("a control-mode tmux client");
        wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
            tmux(&tmux_sock)
                .args(["list-clients", "-t", crown, "-F", "#{client_name}"])
                .output()
                .is_ok_and(|o| !o.stdout.is_empty())
        });
        client
    };

    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    let raise = |c: &mut TestClient, why: &str| match c
        .send(Principal::Agent { session: ws1 }, Command::AgentRaiseHand { reason: why.into() })
    {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    };
    let hand_up = |c: &mut TestClient| c.board().ticket(w1).unwrap().raised.is_some();
    let raised = format!("{kw1} \"mesimon-probe-936 worker\" raised its hand");
    let frames = || read_file(&h.dir.join(format!("mod-{sa}.ndjson")));
    // The frames are JSON, the title's quotes escaped in them.
    let submitted = || {
        frames()
            .lines()
            .filter(|l| l.contains("\"submit\"") && l.contains("raised its hand"))
            .count()
    };
    let pasted = || read_file(&got).lines().filter(|l| l.contains(&raised)).count();
    // Claude Code refuses the mod's submit, as after a person's Esc.
    std::fs::write(h.dir.join("mod-stopped-submit"), "").unwrap();

    // ---- A. stopped by a person's Esc, the person in the pane: held ---------
    start(&mut c, sa);
    c.await_state(sa, "stopped by the person", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted }
    });
    let mut client = attach(&crown);
    raise(&mut c, "which key?");
    wait_until(std::time::Duration::from_secs(10), "the wake held for the person", || {
        wake_rows(&mut c, a).first().is_some_and(|p| p.held.as_deref() == Some(WHEN_YOU_LEAVE))
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!((submitted(), pasted()), (0, 0), "nothing went: {}", frames());
    assert!(hand_up(&mut c), "the worker's hand is still up");
    assert_eq!(wake_rows(&mut c, a).len(), 1, "the wake is still owed");

    // ---- C. they leave: one paste into the empty box settles it -------------
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(10), "the wake typed into the empty box", || {
        pasted() == 1
    });
    assert_eq!(submitted(), 0, "typed, never submitted to a refusing mod");
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c) && wake_rows(&mut c, a).is_empty()
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- B. a submit pending as a person comes in and types -----------------
    std::fs::write(h.dir.join("mod-hold-submit"), "").unwrap();
    raise(&mut c, "which key, again?");
    wait_until(std::time::Duration::from_secs(10), "the submit on its way", || submitted() == 1);
    let mut client = attach(&crown);
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    std::fs::write(h.dir.join("mod-release-submit"), "").unwrap();
    wait_until(std::time::Duration::from_secs(10), "the refused wake owed again", || {
        wake_rows(&mut c, a).first().is_some_and(|p| p.held.is_some())
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let screen = tmux(&tmux_sock).args(["capture-pane", "-p", "-t", &crown]).output().unwrap();
    let screen = String::from_utf8_lossy(&screen.stdout).to_string();
    assert!(screen.contains("half a thought") && !screen.contains("raised its hand"), "{screen}");
    assert_eq!(pasted(), 1, "nothing typed under them");
    assert!(!hand_up(&mut c), "down at the take (T-990); the refusal owes the wake again");
    assert_eq!(wake_rows(&mut c, a).len(), 1, "the wake is still owed");
    assert!(c.board().sessions.iter().all(|s| s.id != sa || s.unsent.is_none()));

    // ---- D. they send their words and leave: one delivery settles both ------
    for name in ["mod-hold-submit", "mod-release-submit", "mod-stopped-submit"] {
        std::fs::remove_file(h.dir.join(name)).unwrap();
    }
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(5), "their words sent", || {
        read_file(&got).lines().any(|l| l.contains("half a thought"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(10), "one delivery of the wake", || {
        submitted() + pasted() >= 3
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c) && wake_rows(&mut c, a).is_empty()
    });
}

/// A Codex crown's wake and a person at its box (T-918), on the synthetic
/// runtime, whose screen the test draws (`fake_codex_runtime.py` echoes no
/// paste at an idle). The wake is pasted only on a sample that read the
/// crown's clients and box together, and its Enter, a beat later, only on a
/// sample from after the paste showing the box as pasted and nobody in the
/// pane, or the box drawn otherwise with nobody seen since the paste. In
/// order: an ordinary delivery, its box's top scrolled away; a person in
/// the pane, then their draft left behind; a person who comes in between
/// the paste and its Enter; a parked crown woken with the words; a person
/// who changes the pasted words, whose box is left as they made it, the
/// wake told nothing and owed again; a person's own prompt sent before the
/// paste, and after clearing the pasted wake, neither of which tells it, so
/// it is owed and delivered after; a restart while the Enter waits on a
/// person, after which the box is still the board's, never a draft; and
/// the crown taken off while its Enter waits, which presses nothing.
/// Codex wraps the box mid-token (`boxed`).
#[test]
fn a_codex_crown_s_wake_waits_for_a_person_and_never_sends_their_words() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex_box",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_codex_box");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let held = |c: &mut TestClient, t: ulid::Ulid| -> Option<String> {
        pending_of(c, Some(t))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let draws = std::cell::Cell::new(0u32);
    let control = |id: uuid::Uuid, mut value: serde_json::Value| {
        // Each instruction differs from the last, or the runtime skips it.
        draws.set(draws.get() + 1);
        value["n"] = json!(draws.get());
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    // The box as Codex draws it: the `›` row, wrapped rows indented two
    // cells where the row is full, inside a word as it must, the cursor
    // after the last word; empty, its placeholder.
    // The width is one at which every break of a wrapped box falls inside a
    // word, so a box read back joined at a space is never the wake.
    let rows_of = |text: &str| -> Vec<String> {
        let flat: Vec<char> =
            text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
        let mid_word =
            |w: usize| (w..flat.len()).step_by(w).all(|i| flat[i - 1] != ' ' && flat[i] != ' ');
        let width = (40..=60).rev().find(|w| mid_word(*w)).expect("a width that breaks a word");
        flat.chunks(width).map(|c| c.iter().collect()).collect()
    };
    let boxed = |text: &str| -> (String, [usize; 2]) {
        let rows = rows_of(text);
        let x = 2 + rows.last().unwrap().chars().count();
        let y = rows.len();
        let body = rows
            .iter()
            .enumerate()
            .map(|(i, r)| if i == 0 { format!("› {r}") } else { format!("  {r}") })
            .collect::<Vec<_>>()
            .join("\r\n");
        (format!("Synthetic provider fixture\r\n{body}\r\nanything the user wants"), [x, y])
    };
    let draw = |id: uuid::Uuid, text: &str, state: serde_json::Value| {
        let (screen, cursor) = if text.is_empty() {
            ("Synthetic provider fixture\r\n› \r\nanything the user wants".to_string(), [2, 1])
        } else {
            boxed(text)
        };
        control(id, json!({"state": state, "screen": screen, "cursor": cursor}));
    };
    let idle = json!({"state":"idle","stop_reason":"end_turn"});

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-918 worker");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Codex,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("the Codex crown's spawn: {other:?}"),
    };
    c.await_state(sa, "the Codex crown at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    wait_until(std::time::Duration::from_secs(10), "the crown's prefill", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_prefill)
    });
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::ClaudeCode
        }),
        Response::Ok
    ));
    let input = h.dir.join(format!("{sa}.input"));
    let bytes = || std::fs::read(&input).unwrap_or_default();
    // The crown's pastes, each as the box would hold it, and its Enters
    // (a newline outside a paste).
    let pastes = || -> Vec<String> {
        let b = bytes();
        let s = String::from_utf8_lossy(&b).to_string();
        s.split("\x1b[200~")
            .skip(1)
            .filter_map(|p| p.split_once("\x1b[201~").map(|(t, _)| t.to_string()))
            .collect()
    };
    let enters = || -> usize {
        let b = bytes();
        let s = String::from_utf8_lossy(&b).to_string();
        let mut n = 0;
        for (i, part) in s.split("\x1b[200~").enumerate() {
            let outside =
                if i == 0 { part } else { part.split_once("\x1b[201~").map_or("", |p| p.1) };
            n += outside.chars().filter(|c| matches!(c, '\r' | '\n')).count();
        }
        n
    };
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let finished = format!("{kw} \"mesimon-probe-918 worker\" finished its turn");
    let wakes = |p: &[String]| p.iter().filter(|t| t.contains(&finished)).count();
    // The crown takes its turn on the words: the Enter began one, which
    // ends here, the box empty again.
    let crown_turn_ends = |c: &mut TestClient| {
        c.await_state(sa, "the crown's turn on the wake", |s| *s == SessionState::Running);
        draw(sa, "", idle.clone());
        c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
        wait_until(std::time::Duration::from_secs(10), "the wake acked", || {
            c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
        });
    };

    // ---- ordinary: nobody there; the Enter waits for the paste to show -----
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the wake pasted", || wakes(&pastes()) == 1);
    let wake1 = pastes().into_iter().find(|t| t.contains(&finished)).unwrap();
    let before = enters();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "no Enter before the box shows the words");
    // A long box's `›` row scrolled off the screen, the prompt above it in
    // the history read in its place: nobody came, so it is ours all the same.
    let rows = rows_of(&wake1);
    assert!(rows.len() > 1, "the wake wraps");
    let tail = rows[1..].iter().map(|r| format!("  {r}")).collect::<Vec<_>>().join("\r\n");
    let x = 2 + rows.last().unwrap().chars().count();
    let screen = format!("Synthetic provider fixture\r\n› go\r\n{tail}\r\nanything the user wants");
    control(sa, json!({"state": idle.clone(), "screen": screen, "cursor": [x, rows.len()]}));
    wait_until(std::time::Duration::from_secs(10), "the wake's Enter", || enters() == before + 1);
    crown_turn_ends(&mut c);

    // ---- a person in the pane, then their draft left behind ---------------
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &sa.simple().to_string()[..16]])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the wake held for the person", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(wakes(&pastes()), 1, "nothing pasted while they are in");
    draw(sa, "half a thought", idle.clone());
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(10), "the wake held for the draft", || {
        held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(wakes(&pastes()), 1, "nothing pasted into their draft");
    // They send it (the runtime's turn on it is not this test's), and the box
    // is empty: the wake goes in.
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(10), "the wake pasted after the draft", || {
        wakes(&pastes()) == 2
    });

    // ---- a person who comes in between the paste and its Enter ------------
    let wake2 = pastes().into_iter().filter(|t| t.contains(&finished)).nth(1).unwrap();
    let before = enters();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &sa.simple().to_string()[..16]])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &sa.simple().to_string()[..16], "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    draw(sa, &wake2, idle.clone());
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(enters(), before, "no Enter while a person is in the pane");
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(10), "the Enter once they left", || {
        enters() == before + 1
    });
    crown_turn_ends(&mut c);

    // ---- a parked crown: woken with no words, the wake owed for its box ---
    // (T-936) Its launch carries nothing, and nothing goes on a person's
    // ask road: the wake waits, owed and untold, through a restart, for the
    // live pane's guarded road, the box's paste and its Enter.
    let sent = || {
        std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .matches("\"crown_wake_sent\"")
            .count()
    };
    let told_before = sent();
    assert!(matches!(c.request(Command::SleepSession { id: sa }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the crown parked", || {
        c.board()
            .sessions
            .iter()
            .any(|s| s.id == sa && s.state == SessionState::Sleeping && !s.codex_stopping)
    });
    // The woken runtime draws no box yet: nothing can be pasted.
    control(
        sa,
        json!({"state": idle, "screen": "Synthetic provider fixture\r\n  starting\r\n", "cursor": null}),
    );
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(20), "the parked crown woken", || {
        c.board().sessions.iter().any(|s| s.id == sa && s.state != SessionState::Sleeping)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(wakes(&pastes()), 2, "nothing rode the launch");
    assert_eq!(sent(), told_before, "not told at the launch");
    assert!(
        pending_of(&mut c, Some(a))
            .iter()
            .all(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "no person's ask filed for the crown"
    );
    drop(c);
    h.restart();
    let mut c = h.client("crown_codex_box_parked");
    assert!(
        pending_of(&mut c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "the wake owed after the restart"
    );
    assert_eq!(sent(), told_before, "nothing told across the restart");
    // The box comes up: the wake is pasted, shown, and its Enter tells it.
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(20), "the wake pasted into the woken box", || {
        wakes(&pastes()) == 3
    });
    let wake3 = pastes().into_iter().filter(|t| t.contains(&finished)).nth(2).unwrap();
    let before = enters();
    draw(sa, &wake3, idle.clone());
    wait_until(std::time::Duration::from_secs(10), "the woken crown's Enter", || enters() > before);
    wait_until(std::time::Duration::from_secs(5), "told at its Enter", || sent() > told_before);
    crown_turn_ends(&mut c);

    // ---- a person changes the pasted words: their box, the wake owed again
    let feed = h.paths.state_dir.join("activity.jsonl");
    let told = || {
        std::fs::read_to_string(&feed).unwrap_or_default().matches("\"crown_wake_sent\"").count()
    };
    let attach = || {
        let client = tmux(&tmux_sock)
            .args(["-C", "attach", "-t", &sa.simple().to_string()[..16]])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("a control-mode tmux client");
        wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
            tmux(&tmux_sock)
                .args([
                    "list-clients",
                    "-t",
                    &sa.simple().to_string()[..16],
                    "-F",
                    "#{client_name}",
                ])
                .output()
                .is_ok_and(|o| !o.stdout.is_empty())
        });
        client
    };
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the fourth wake pasted", || {
        wakes(&pastes()) == 4
    });
    let wake4 = pastes().into_iter().filter(|t| t.contains(&finished)).nth(3).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let (before, len, said) = (enters(), bytes().len(), told());
    let mut client = attach();
    draw(sa, &format!("{wake4} and my own words"), idle.clone());
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(enters(), before, "no Enter while they are in");
    client.kill().unwrap();
    let _ = client.wait();
    // The Enter's `Theirs` drops the entry (`pending_submit`), and only then
    // does the row wait on their draft: until it, the box is the Enter's to
    // judge and the row says `sending`.
    wait_until(
        std::time::Duration::from_secs(10),
        "the wake owed again, after their draft",
        || {
            c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
                && held(&mut c, a).as_deref() == Some("after your draft")
        },
    );
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "their words are never sent by the wake");
    assert_eq!(bytes().len(), len, "nothing typed into the box they changed");
    assert_eq!(told(), said, "a wake never entered is not told");
    // They send their words (not this test's turn); the empty box takes the
    // wake again, and its Enter tells it.
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(10), "the wake pasted again", || {
        wakes(&pastes()) == 5
    });
    let wake5 = pastes().into_iter().filter(|t| t.contains(&finished)).nth(4).unwrap();
    draw(sa, &wake5, idle.clone());
    wait_until(std::time::Duration::from_secs(10), "its Enter", || enters() == before + 1);
    wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
    crown_turn_ends(&mut c);

    // A person's own prompt, typed in the pane and sent (T-918 review): the
    // runtime begins a turn on it.
    let person_sends = |words: &str| {
        let sid = sa.simple().to_string();
        let sent = tmux(&tmux_sock).args(["send-keys", "-t", &sid[..16], words, "Enter"]).status();
        assert!(sent.is_ok_and(|s| s.success()), "send-keys");
    };
    let last_wake = || pastes().into_iter().rfind(|t| t.contains(&finished)).unwrap();
    // The wake owed again after a person's turn: held while they stay,
    // pasted once they leave, and told at its own Enter.
    let delivered_again = |c: &mut TestClient, client: &mut std::process::Child, n: usize| {
        wait_until(std::time::Duration::from_secs(10), "the wake owed again", || {
            held(c, a).as_deref() == Some(WHEN_YOU_LEAVE)
        });
        let said = told();
        client.kill().unwrap();
        let _ = client.wait();
        wait_until(std::time::Duration::from_secs(10), "the owed wake pasted", || {
            wakes(&pastes()) == n + 1
        });
        let before = enters();
        draw(sa, &last_wake(), idle.clone());
        wait_until(std::time::Duration::from_secs(10), "its Enter", || enters() == before + 1);
        wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
        crown_turn_ends(c);
    };

    // ---- a person's own prompt before the paste: never told, owed again ---
    // The box hides its cursor (a frame mid-draw), so the wake is owed and
    // its paste waits for the composer.
    control(
        sa,
        json!({"state": idle.clone(),
               "screen": "Synthetic provider fixture\r\n› \r\nanything the user wants",
               "cursor": null}),
    );
    let (n, said) = (wakes(&pastes()), told());
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the wake owed, its paste waiting", || {
        held(&mut c, a).as_deref() == Some("sending")
    });
    let mut client = attach();
    person_sends("my own question");
    crown_turn_ends(&mut c);
    assert_eq!(wakes(&pastes()), n, "the wake never went in");
    assert_eq!(told(), said, "the person's turn did not tell the wake");
    delivered_again(&mut c, &mut client, n);

    // ---- after the paste, a person clears the box and sends their own -----
    start(&mut c, ws);
    stop(&mut c, ws);
    let n = wakes(&pastes()) + 1;
    wait_until(std::time::Duration::from_secs(10), "the wake pasted", || wakes(&pastes()) == n);
    let said = told();
    let mut client = attach();
    draw(sa, "", idle.clone());
    // The runtime takes the drawing first: a control it reads after the
    // Enter would put its state back to idle.
    std::thread::sleep(std::time::Duration::from_millis(500));
    person_sends("something else entirely");
    crown_turn_ends(&mut c);
    assert_eq!(told(), said, "their turn on their own words did not tell the wake");
    delivered_again(&mut c, &mut client, n);

    // ---- a restart while the Enter waits on a person ----------------------
    start(&mut c, ws);
    stop(&mut c, ws);
    let n = wakes(&pastes()) + 1;
    wait_until(std::time::Duration::from_secs(10), "the wake pasted", || wakes(&pastes()) == n);
    let wake6 = last_wake();
    let mut client = attach();
    draw(sa, &wake6, idle.clone());
    wait_until(std::time::Duration::from_secs(10), "the Enter held for the person", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE)
    });
    let (before, said) = (enters(), told());
    h.restart();
    let mut c = h.client("crown_codex_box_again");
    wait_until(std::time::Duration::from_secs(15), "the next daemon holds it too", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "no Enter while they are in, across the restart");
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(15), "the Enter once they left", || {
        assert_ne!(
            held(&mut c, a).as_deref(),
            Some("after your draft"),
            "the board's own words read as a person's draft"
        );
        enters() == before + 1
    });
    assert_eq!(wakes(&pastes()), n, "the box's words were not pasted again");
    wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
    crown_turn_ends(&mut c);

    // ---- (T-936) a tier switch on the crown: its stop, one wake, no words -
    // The switch parks the crown and marks it; a wake owed meanwhile waits
    // for the stop's confirmation and the tier's one wake, rides neither,
    // and goes on no person's queue; the live pane's box then delivers it.
    let reviewer = mesimon_core::tier::Tier {
        id: "01REVIEW".into(),
        name: "reviewer".into(),
        provider: mesimon_core::board::AgentProvider::Codex,
        model: "gpt-6-astra".into(),
        effort: mesimon_core::tier::Effort::High,
        description: String::new(),
    };
    assert!(matches!(
        c.request(Command::SaveTier {
            scope: mesimon_core::tier::TierScope::Board,
            tier: reviewer
        }),
        Response::Ok
    ));
    let launches = || {
        std::fs::read_to_string(h.dir.join("runtime-launches.jsonl"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    let crown_rec =
        |c: &mut TestClient| c.board().sessions.iter().find(|s| s.id == sa).cloned().unwrap();
    let (launched, said, pasted) = (launches(), told(), wakes(&pastes()));
    control(sa, json!({"stop_delay": 2}));
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert!(matches!(
        c.request(Command::SetTicketTier { id: a, tier: Some("01REVIEW".into()) }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(15), "the crown parked for the switch", || {
        let rec = crown_rec(&mut c);
        rec.tier_wake && rec.state == SessionState::Sleeping
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let rows = pending_of(&mut c, Some(a));
    assert!(
        rows.iter().any(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "the wake owed while the crown switches: {rows:?}"
    );
    assert!(
        rows.iter().all(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "no person's ask filed for the crown: {rows:?}"
    );
    assert_eq!(told(), said, "not told across the switch");
    wait_until(std::time::Duration::from_secs(20), "the crown woken on the new tier", || {
        let rec = crown_rec(&mut c);
        !rec.tier_wake && rec.tier == "01REVIEW" && rec.state != SessionState::Sleeping
    });
    wait_until(std::time::Duration::from_secs(10), "the new runtime up", || {
        launches() == launched + 1
    });
    assert_eq!(wakes(&pastes()), pasted, "nothing rode the relaunch");
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(20), "the wake pasted into the new box", || {
        wakes(&pastes()) == pasted + 1
    });
    let before = enters();
    draw(sa, &last_wake(), idle.clone());
    wait_until(std::time::Duration::from_secs(10), "its Enter", || enters() > before);
    wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
    assert_eq!(launches(), launched + 1, "still one wake");
    crown_turn_ends(&mut c);

    // ---- the crown taken off while its Enter waits: nothing is pressed ----
    start(&mut c, ws);
    stop(&mut c, ws);
    let n = wakes(&pastes()) + 1;
    wait_until(std::time::Duration::from_secs(10), "the wake pasted", || wakes(&pastes()) == n);
    let mut client = attach();
    draw(sa, &last_wake(), idle.clone());
    let before = enters();
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the seat owes nothing", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
    });
    client.kill().unwrap();
    let _ = client.wait();
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert_eq!(enters(), before, "no Enter into a seat that is no crown's");
}

/// T-945 on a Codex crown, the synthetic runtime drawing its box: a client
/// attached to the crown's pane holds the paste while tmux records activity
/// from it, and past the bound the wake is pasted with the client still in.
/// The client was there at the paste, so the box is the board's only while
/// it reads exactly as pasted: its Enter goes then, and a box that reads
/// otherwise, changed or clipped, is the person's, its Enter never pressed.
/// So too when that client edits the box and leaves just after the paste,
/// with or without a restart (the outcome under the scheduling observed),
/// and the reverse: a client attached at the park who left before the
/// paste's own sample leaves the unedited wake the board's.
#[test]
fn a_codex_crown_s_wake_goes_past_an_idle_client_and_enters_only_its_own_box() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex_idle",
        Some(STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_PERSON_IDLE_MS", IDLE_MS),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_codex_idle");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let held = |c: &mut TestClient, t: ulid::Ulid| -> Option<String> {
        pending_of(c, Some(t))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let draws = std::cell::Cell::new(0u32);
    let control = |id: uuid::Uuid, mut value: serde_json::Value| {
        draws.set(draws.get() + 1);
        value["n"] = json!(draws.get());
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    // The box as Codex draws it (see the T-918 test above): wrapped inside
    // a word, the cursor after the last word; empty, its placeholder.
    let rows_of = |text: &str| -> Vec<String> {
        let flat: Vec<char> =
            text.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
        let mid_word =
            |w: usize| (w..flat.len()).step_by(w).all(|i| flat[i - 1] != ' ' && flat[i] != ' ');
        let width = (40..=60).rev().find(|w| mid_word(*w)).expect("a width that breaks a word");
        flat.chunks(width).map(|c| c.iter().collect()).collect()
    };
    let draw = |id: uuid::Uuid, text: &str, state: serde_json::Value| {
        let (screen, cursor) = if text.is_empty() {
            ("Synthetic provider fixture\r\n› \r\nanything the user wants".to_string(), [2, 1])
        } else {
            let rows = rows_of(text);
            let x = 2 + rows.last().unwrap().chars().count();
            let body = rows
                .iter()
                .enumerate()
                .map(|(i, r)| if i == 0 { format!("› {r}") } else { format!("  {r}") })
                .collect::<Vec<_>>()
                .join("\r\n");
            (
                format!("Synthetic provider fixture\r\n{body}\r\nanything the user wants"),
                [x, rows.len()],
            )
        };
        control(id, json!({"state": state, "screen": screen, "cursor": cursor}));
    };
    let idle = json!({"state":"idle","stop_reason":"end_turn"});

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-945 worker");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
    let sa = match c.request(Command::SpawnSession {
        ticket: a,
        kind: SessionKind::Codex,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("the Codex crown's spawn: {other:?}"),
    };
    c.await_state(sa, "the Codex crown at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    wait_until(std::time::Duration::from_secs(10), "the crown's prefill", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_prefill)
    });
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::ClaudeCode
        }),
        Response::Ok
    ));
    let input = h.dir.join(format!("{sa}.input"));
    let bytes = || std::fs::read(&input).unwrap_or_default();
    let pastes = || -> Vec<String> {
        let s = String::from_utf8_lossy(&bytes()).to_string();
        s.split("\x1b[200~")
            .skip(1)
            .filter_map(|p| p.split_once("\x1b[201~").map(|(t, _)| t.to_string()))
            .collect()
    };
    let enters = || -> usize {
        let s = String::from_utf8_lossy(&bytes()).to_string();
        let mut n = 0;
        for (i, part) in s.split("\x1b[200~").enumerate() {
            let outside =
                if i == 0 { part } else { part.split_once("\x1b[201~").map_or("", |p| p.1) };
            n += outside.chars().filter(|c| matches!(c, '\r' | '\n')).count();
        }
        n
    };
    let feed = h.paths.state_dir.join("activity.jsonl");
    let told = || {
        std::fs::read_to_string(&feed).unwrap_or_default().matches("\"crown_wake_sent\"").count()
    };
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let finished = format!("{kw} \"mesimon-probe-945 worker\" finished its turn");
    let wakes = || pastes().iter().filter(|t| t.contains(&finished)).count();
    let last_wake = || pastes().into_iter().rfind(|t| t.contains(&finished)).unwrap();
    let crown = sa.simple().to_string()[..16].to_string();
    let (mut client, _) = attach_client(&tmux_sock, &crown);

    // ---- the client fresh: held; idle past the bound: pasted, still in ----
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the row names the bound", || {
        held(&mut c, a).as_deref() == Some(WHEN_YOU_LEAVE_5S)
    });
    assert_eq!(wakes(), 0, "nothing pasted while the client is active");
    wait_until(std::time::Duration::from_secs(15), "the wake pasted past the bound", || {
        wakes() == 1
    });
    // The box exactly as pasted: the Enter goes, the client still in.
    let (before, said) = (enters(), told());
    draw(sa, &last_wake(), idle.clone());
    wait_until(std::time::Duration::from_secs(10), "the wake's Enter", || enters() == before + 1);
    wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
    assert!(attach_listed(&tmux_sock, &crown), "the client still attached");
    c.await_state(sa, "the crown's turn on the wake", |s| *s == SessionState::Running);
    draw(sa, "", idle.clone());
    c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(10), "the wake acked", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
    });

    // ---- the pasted words changed with a client there: theirs ---------------
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the second wake pasted", || wakes() == 2);
    let (before, said, len) = (enters(), told(), bytes().len());
    draw(sa, &format!("{} and my own words", last_wake()), idle.clone());
    wait_until(std::time::Duration::from_secs(10), "owed again, after their draft", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
            && held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "their words are never sent by the wake");
    assert_eq!(bytes().len(), len, "nothing typed into the box they changed");
    assert_eq!(told(), said, "a wake never entered is not told");

    // ---- a clipped box with a client there: no ownership, no Enter ---------
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(15), "the wake pasted again", || wakes() == 3);
    let (before, said) = (enters(), told());
    let rows = rows_of(&last_wake());
    assert!(rows.len() > 1, "the wake wraps");
    let tail = rows[1..].iter().map(|r| format!("  {r}")).collect::<Vec<_>>().join("\r\n");
    let x = 2 + rows.last().unwrap().chars().count();
    let screen = format!("Synthetic provider fixture\r\n› go\r\n{tail}\r\nanything the user wants");
    control(sa, json!({"state": idle.clone(), "screen": screen, "cursor": [x, rows.len()]}));
    wait_until(std::time::Duration::from_secs(10), "the clipped box is theirs", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
            && held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "no Enter into a box nobody can show is the wake");
    assert_eq!(told(), said, "not told");

    // The idle client there at the paste edits the box and leaves within
    // milliseconds of it. What this proves is the outcome under the
    // scheduling observed: no Enter and nothing told. It does not fence the
    // order against a facts sample landing between the paste and the edit
    // (which latches the same way); a mutation turning off the paste's own
    // latch fails it, and the paste's latch is the one that answers when no
    // sample came between.
    let pasted_then_left = |n: usize, client: &mut std::process::Child| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while wakes() < n {
            assert!(std::time::Instant::now() < deadline, "the wake pasted");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        client.kill().unwrap();
        let _ = client.wait();
        draw(sa, "my own words instead", idle.clone());
    };
    // ---- edited and gone before the next sample: theirs --------------------
    let (before, said) = (enters(), told());
    draw(sa, "", idle.clone());
    pasted_then_left(4, &mut client);
    wait_until(std::time::Duration::from_secs(10), "owed again, after their words", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
            && held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "their words are never entered by the wake");
    assert_eq!(told(), said, "not told");

    // ---- the same, with a restart: the same answer ------------------------
    // The restart comes after the edit; whether the last daemon classified
    // the box before it went is not fenced, and either way the box is theirs
    // (`resume_crown_entering` reads every box as a person may have been).
    let (mut client, _) = attach_client(&tmux_sock, &crown);
    let (before, said) = (enters(), told());
    draw(sa, "", idle.clone());
    pasted_then_left(5, &mut client);
    h.restart();
    let mut c = h.client("crown_codex_idle_again");
    wait_until(std::time::Duration::from_secs(15), "owed again across the restart", || {
        c.board().sessions.iter().any(|s| s.id == sa && !s.pending_submit)
            && held(&mut c, a).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(enters(), before, "never entered across the restart");
    assert_eq!(told(), said, "not told across the restart");

    // ---- attached at the park, gone before the paste: the board's box ------
    // A box with its cursor hidden (a frame mid-draw) parks the wake and
    // holds its paste (`sending`) while the client, idle past the bound, is
    // still attached. They leave; the sample that lets the paste go sees
    // nobody, so the unchanged wake, its long box's top scrolled away, is
    // the board's, and its Enter goes.
    let (mut client, _) = attach_client(&tmux_sock, &crown);
    control(
        sa,
        json!({"state": idle.clone(),
               "screen": "Synthetic provider fixture\r\n› \r\nanything the user wants",
               "cursor": null}),
    );
    wait_until(std::time::Duration::from_secs(20), "parked past the bound, its paste held", || {
        held(&mut c, a).as_deref() == Some("sending")
    });
    assert!(attach_listed(&tmux_sock, &crown), "the client still in at the park");
    let n = wakes();
    client.kill().unwrap();
    let _ = client.wait();
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let (before, said) = (enters(), told());
    draw(sa, "", idle.clone());
    wait_until(std::time::Duration::from_secs(15), "the parked wake pasted", || wakes() == n + 1);
    let rows = rows_of(&last_wake());
    assert!(rows.len() > 1, "the wake wraps");
    let tail = rows[1..].iter().map(|r| format!("  {r}")).collect::<Vec<_>>().join("\r\n");
    let x = 2 + rows.last().unwrap().chars().count();
    let screen = format!("Synthetic provider fixture\r\n› go\r\n{tail}\r\nanything the user wants");
    control(sa, json!({"state": idle.clone(), "screen": screen, "cursor": [x, rows.len()]}));
    wait_until(std::time::Duration::from_secs(10), "its Enter", || enters() == before + 1);
    wait_until(std::time::Duration::from_secs(10), "told at its Enter", || told() == said + 1);
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-81 landing");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // A person sends the crown's ask here (step 2): supervised, so the
    // queue does not send it first (autonomous by default since T-610).
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
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
            expect_secs: None,
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
            "{worker} delivered (merge_state needs_rebase, merge: person, column {column}) ∙ \
             get_ticket key={kw} for state and notes"
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
            expect_secs: None,
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
    // T-938: the line says what the turn's end proves, and names the ask:
    // the mod's `entered` handed it and its words started this turn, so it
    // reads `taken`; a paste no receipt named was sent, its receipt
    // unconfirmed, and the line claims no more.
    if test_road() == "mod" {
        assert!(
            answer.starts_with(&format!("{worker} ended its turn after your ask went in (ask ")),
            "{answer}"
        );
        assert!(answer.contains(": taken, merge_state needs_rebase → ahead"), "{answer}");
    } else {
        assert!(answer.starts_with(&format!("{worker} ended a turn after ask ")), "{answer}");
        assert!(
            answer.contains(" was sent; receipt unconfirmed (merge_state needs_rebase → ahead"),
            "{answer}"
        );
    }
    assert!(answer.ends_with(&format!("∙ get_ticket key={kw} for state and notes")), "{answer}");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 3. the person merges: the board tells the crown's worker in the
    //         same step (T-613), and the crown hears the landing once, when
    //         that notice turn is over (T-596) -------------------------------
    match c.request(Command::MergeTicket { id: w }) {
        Response::Merge {
            outcome: mesimon_core::command::MergeOutcome::Merged, notified, ..
        } => {
            assert!(notified, "a worker the crown started is told by the board");
        }
        other => panic!("merge: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the merged notice to reach the worker", || {
        lines_with("has been merged").len() == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 2, "held while the notice is on its way");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 3
    });
    let merged = lines_with(&worker).pop().unwrap();
    assert_eq!(
        merged,
        format!(
            "{worker} merged and finished its turn (merge_state ahead → merged) ∙ get_ticket \
             key={kw} for state and notes"
        )
    );
    start(&mut c, sa);
    stop(&mut c, sa);
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-91 merged by hand");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-92 merged while down");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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

/// A restart between a worker's launch and its first prompt (T-722): the
/// crown started it, its `SessionStart` came, and the daemon went down (a
/// person's `U`) before the brief was pasted. The new daemon used to call
/// the brief abandoned and tell the crown at once that the worker `finished
/// its turn (nothing new to merge)` — a turn that never ran, on a worker
/// sitting at an empty composer. Now the brief goes again, and the crown
/// hears nothing until the turn the brief starts has ended.
#[test]
fn a_brief_resent_after_a_restart_is_not_heard_as_a_finished_turn() {
    owed_across_a_restart("crown_owed", false);
}

/// The same restart at a worker whose conversation took a prompt in the
/// window (T-722): nothing is sent again, and its words still unsent are
/// words pending on the ticket, so the restart's look at it is silent too.
#[test]
fn a_brief_left_unsent_after_a_restart_is_not_heard_as_a_finished_turn() {
    owed_across_a_restart("crown_unsent", true);
}

fn owed_across_a_restart(name: &str, took: bool) {
    // A pane started while `slow` exists comes up four seconds late, so the
    // worker's brief cannot be pasted before the restart; the crown's pane
    // is up at once.
    let stub = format!(
        "#!/bin/sh\n[ -e \"$(dirname \"$0\")/slow\" ] && sleep 4\n{COMPOSER}{}",
        RECORDING_STUB.trim_start_matches("#!/bin/sh\n")
    );
    let Some(h) = Harness::boot_bare(
        name,
        Some(&stub),
        &[
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };

    let a = c.create_ticket("coordinate");
    let title = "mesimon-probe-722 owed across a restart";
    let w = c.create_ticket(title);
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));

    std::fs::write(h.dir.join("slow"), "").unwrap();
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(c.board().live_agent(w).unwrap().pending_submit, "the brief is owed");
    drop(c);

    h.restart_after(|| {
        if took {
            std::fs::write(&transcript, "{}\n").unwrap();
        }
    });
    let mut c = h.client(&format!("{name}_2"));
    let brief = || lines_with("mesimon-probe-722 owed across a restart");
    if took {
        // Past the late pane's composer: nothing goes, and the seat says so.
        std::thread::sleep(std::time::Duration::from_millis(6000));
        assert!(brief().is_empty(), "a prompted seat is not sent the brief again");
        assert!(c.board().live_agent(w).unwrap().unsent.is_some(), "brief not sent");
    } else {
        wait_until(std::time::Duration::from_secs(20), "the resent brief", || !brief().is_empty());
        // Time for a restored worker's look (`hear_restored`) and a wake.
        std::thread::sleep(std::time::Duration::from_millis(2500));
    }
    let worker = format!("{kw} \"{title}\"");
    assert!(
        lines_with(&worker).is_empty(),
        "no wake before the brief's turn:\n{}",
        std::fs::read_to_string(&got).unwrap()
    );

    // The brief's turn runs and ends: that is the turn the crown hears of.
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(10), "the finished turn's wake", || {
        lines_with(&worker).len() == 1
    });
    // The crown's pane holds the title its own start typed, so the first
    // line it reads begins with it.
    assert!(
        lines_with(&worker)[0].contains(&format!("{worker} finished its turn")),
        "{:?}",
        lines_with(&worker)
    );
}

/// A worker thinking across a restart is not heard as finished (T-731): the
/// crown started it, its turn ran, and the daemon went down while the model
/// thought after a tool result, transcript still for over a minute. The new
/// daemon read the quiet tail as the turn gone idle and told the crown the
/// worker had `finished its turn (nothing new to merge, … after a restart)`.
/// While Claude Code's session file reads `busy` the record stays `Unknown`;
/// with no file the tail's guess is `Idle` at Low, and that is no turn's
/// end either. The crown hears the turn when it really ends.
#[test]
fn a_turn_thinking_across_a_restart_is_not_heard_as_finished() {
    let Some(h) = Harness::boot_with_env(
        "crown_think",
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
    let mut c = h.client("crown_think");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };

    let a = c.create_ticket("coordinate");
    let title = "mesimon-probe-731 thinking across a restart";
    let w = c.create_ticket(title);
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;

    // The turn rests on a tool's result, the model thinking on it.
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let lines = [
        serde_json::json!({"uuid":"p", "type":"user", "message":{"content":"brief"}}),
        serde_json::json!({"uuid":"c", "type":"assistant", "message":{"stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}),
        serde_json::json!({"uuid":"r", "type":"user",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}),
    ];
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&transcript, body).unwrap();
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    // The brief's own ack follows its paste (T-987): a prompt before it is
    // a person's, and leaves the brief unsent.
    if test_road() == "hooks" {
        wait_until(std::time::Duration::from_secs(15), "the brief's paste", || {
            lines_with(title).iter().any(|l| l.starts_with(title))
        });
    }
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    let sessions = h.dir.join("claude-home").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let session_file = sessions.join(format!("{}.json", std::process::id()));
    std::fs::write(
        &session_file,
        serde_json::json!({"pid": std::process::id(), "sessionId": ws, "status": "busy",
            "statusUpdatedAt": 900})
        .to_string(),
    )
    .unwrap();
    let quiet = || {
        let ago = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        std::fs::File::options().write(true).open(&transcript).unwrap().set_modified(ago).unwrap();
    };
    drop(c);
    let worker = format!("{kw} \"{title}\"");

    // ---- 1. Claude Code calls it busy: Unknown, and the crown hears nothing
    h.restart_after(quiet);
    let mut c = h.client("crown_think_2");
    std::thread::sleep(std::time::Duration::from_millis(4000));
    let state = c.board().live_agent(w).unwrap().state.clone();
    assert!(matches!(state, SessionState::Unknown { .. }), "{state:?}");
    assert!(lines_with(&worker).is_empty(), "{}", std::fs::read_to_string(&got).unwrap());
    drop(c);

    // ---- 2. no session file: the tail's guess is no turn's end ------------
    std::fs::remove_file(&session_file).unwrap();
    h.restart_after(quiet);
    let mut c = h.client("crown_think_3");
    wait_until(std::time::Duration::from_secs(10), "the tail's guess", || {
        matches!(c.board().live_agent(w).unwrap().state, SessionState::Idle { .. })
    });
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert!(lines_with(&worker).is_empty(), "{}", std::fs::read_to_string(&got).unwrap());

    // ---- 3. the turn goes on and ends: that is the finished turn ----------
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", r#"{"tool_name":"Bash"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(10), "the finished turn's wake", || {
        lines_with(&worker).len() == 1
    });
    assert!(
        lines_with(&worker)[0].contains(&format!("{worker} finished its turn")),
        "{:?}",
        lines_with(&worker)
    );
}

/// A worker mid-turn across a restart is not heard as delivered (T-756):
/// the crown started it in a worktree, its turn committed and went on, and
/// the daemon went down between two tool calls. The new daemon's tail read
/// the quiet turn as idle at Low and, the branch ahead, told the crown
/// `delivered (…, after a restart)` a second after it started, while the
/// worker was still working. The tail's guess is no turn's end: the crown
/// hears the delivery once, at the turn's real `Stop`, still `after a
/// restart`.
#[test]
fn a_worker_mid_turn_across_a_restart_delivers_at_its_real_end() {
    let Some(h) = Harness::boot_with_env(
        "crown_midturn",
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
    let mut c = h.client("crown_midturn");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };

    let a = c.create_ticket("coordinate");
    let title = "mesimon-probe-756 mid-turn across a restart";
    let w = c.create_ticket(title);
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let seat = c.board().live_agent(w).unwrap().clone();
    let ws = seat.id;

    // The turn commits, calls a tool, and rests on its result.
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let lines = [
        serde_json::json!({"uuid":"p", "type":"user", "message":{"content":"brief"}}),
        serde_json::json!({"uuid":"c", "type":"assistant", "message":{"stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}),
        serde_json::json!({"uuid":"r", "type":"user",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}),
    ];
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&transcript, body).unwrap();
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    commit(std::path::Path::new(&seat.cwd), "b.txt");
    let quiet = || {
        let ago = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        std::fs::File::options().write(true).open(&transcript).unwrap().set_modified(ago).unwrap();
    };
    drop(c);
    let worker = format!("{kw} \"{title}\"");

    // ---- 1. the tail's guess, with the branch ahead: no delivery yet ------
    h.restart_after(quiet);
    let mut c = h.client("crown_midturn_2");
    wait_until(std::time::Duration::from_secs(10), "the tail's guess", || {
        matches!(c.board().live_agent(w).unwrap().state, SessionState::Idle { .. })
    });
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert!(lines_with(&worker).is_empty(), "{}", std::fs::read_to_string(&got).unwrap());

    // ---- 2. the turn goes on and ends: that is the delivery, once ---------
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", r#"{"tool_name":"Bash"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let said = lines_with(&worker);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains(&format!("{worker} delivered")), "{said:?}");
    assert!(said[0].contains("after a restart"), "{said:?}");
}

/// A delivery held for the train is not heard while its worker is still
/// `Unknown` after a restart (T-776): the crown started the worker in a
/// worktree, its branch was behind, the train asked the rebase and held
/// the delivery for it, and the daemon went down in that rebase turn. The
/// new daemon read the worker `Unknown` (no hook, nothing in the
/// transcript), let the hold go as one the train would not take, and told
/// the crown `delivered (…, after a restart)` a second after it started,
/// while the turn was on. The hold waits for the restart's look, at the
/// turn's real `Stop`: one line, still `after a restart`.
#[test]
fn a_held_delivery_waits_for_a_worker_unknown_across_a_restart() {
    // The flags (and the train) every second rather than every ten.
    let Some(h) = Harness::boot_with_env(
        "crown_unknown",
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
    let mut c = h.client("crown_unknown");
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

    let a = c.create_ticket("coordinate");
    let title = "mesimon-probe-776 unknown across a restart";
    let w = c.create_ticket(title);
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let worker = format!("{kw} \"{title}\"");

    // ---- 1. delivered behind main: the train asks the rebase and holds it --
    std::fs::write(h.repo.join("base1.txt"), "base\n").unwrap();
    git(&h.repo, &["add", "base1.txt"]);
    git(&h.repo, &["commit", "-qm", "base1"]);
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    let ask = format!("Rebase your current branch {branch} onto main");
    wait_until(std::time::Duration::from_secs(15), "the train's rebase ask", || {
        !lines_with(&ask).is_empty()
    });
    wait_until(std::time::Duration::from_secs(5), "the hold in the feed", || {
        feed_count("\"crown_wake_deferred\"") == 1
    });
    assert!(lines_with(&worker).is_empty(), "a delivery the train takes is silent");

    // ---- 2. the rebase turn runs across a restart: Unknown, no word -------
    start(&mut c, ws);
    drop(c);
    h.restart_after(|| {});
    let mut c = h.client("crown_unknown_2");
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert!(
        matches!(c.board().live_agent(w).unwrap().state, SessionState::Unknown { .. }),
        "{:?}",
        c.board().live_agent(w).unwrap().state
    );
    assert!(lines_with(&worker).is_empty(), "{}", std::fs::read_to_string(&got).unwrap());

    // ---- 3. the turn ends still behind: that is the delivery, once ---------
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let said = lines_with(&worker);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains(&format!("{worker} delivered")), "{said:?}");
    assert!(said[0].contains("needs_rebase"), "{said:?}");
    assert!(said[0].contains("after a restart"), "{said:?}");
}

/// A merge's wake waits for the merge notice's turn across a restart
/// (T-1004): the crown merged, the notice went in and its turn began, and
/// the daemon went down in that turn. The new daemon read the worker
/// `Unknown`, which no reading of `working` takes, let the held landing go,
/// and told the crown `merged and finished its turn (…, after a restart)`
/// a second after it started, while the turn ran; the turn's real end then
/// found the merge told and said nothing. The landing waits for that end:
/// one line, at the `Stop`, still `after a restart`.
#[test]
fn a_merge_notice_turn_across_a_restart_wakes_once_at_its_end() {
    merge_notice_across_a_restart("crown_notice_restart", After::RunsOn);
}

/// The bound on that wait (T-1004): a notice turn whose end fell in the
/// restart's gap leaves a seat no hook will ever speak for (an idle agent
/// sends none) and nothing in its transcript to date the close. Read
/// `Unknown` when the restart's look settles (`MESIMON_RECHECK_MS`), the
/// board knows the merge and not the end: the crown hears `merged (…, after
/// a restart)` once, inside the window, with no `finished`, and the next
/// turn's end is a finished turn, not the merge step's.
#[test]
fn a_merge_notice_turn_ended_in_the_restart_s_gap_wakes_once_in_the_window() {
    merge_notice_across_a_restart("crown_notice_gap", After::NextPrompt);
}

/// The same bound on a seat that was only silent (T-1006's cold read): the
/// notice turn ran on past the window with no hook, and the window told the
/// merge alone. That turn's own `Stop`, with no prompt between, is its end:
/// one `finished its turn`, and no second `merged`.
#[test]
fn a_merge_notice_turn_silent_past_the_window_finishes_once_at_its_stop() {
    merge_notice_across_a_restart("crown_notice_silent", After::SameStop);
}

/// A merge mark taken on no end loses nothing (T-1006's second read): the
/// notice turn reads `running` (the tail's seed, at Low) when the window
/// settles, so the landing stays held; then the tail reads an abort record
/// as an idle at Low, which takes the mark on no proven end. The crown hears
/// the merge alone there, and the turn going on to its real `Stop` says
/// `finished its turn` once, with no second `merged`.
#[test]
fn a_merge_notice_turn_guessed_idle_after_the_window_finishes_once_at_its_stop() {
    merge_notice_across_a_restart("crown_notice_guessed", After::GuessedIdle);
}

/// A merge told alone and its turn's end, owed together, are one line
/// (T-1006's third read): the crown is busy while the window lets the merge
/// go and while that turn's own `Stop` lands, so both wait for it. Freed,
/// it hears one `merged and finished its turn`, never a bare `merged`.
#[test]
fn a_merge_and_its_end_owed_to_a_busy_crown_are_one_line() {
    merge_notice_across_a_restart("crown_notice_busy", After::CrownBusy);
}

/// What the notice's turn does after the restart.
#[derive(Clone, Copy, PartialEq)]
enum After {
    /// As `SameStop`, with the crown working from before the window settles
    /// until after the `Stop`.
    CrownBusy,
    /// Running off its transcript past the window, then a Low idle guess,
    /// then on to its `Stop`.
    GuessedIdle,
    /// A tool call, then its `Stop`, inside the window.
    RunsOn,
    /// Nothing until the window settles, then a new prompt and its `Stop`.
    NextPrompt,
    /// Nothing until the window settles, then its own `Stop`.
    SameStop,
}

fn merge_notice_across_a_restart(name: &str, after: After) {
    let Some(h) = Harness::boot_with_env(
        name,
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
            ("MESIMON_RECHECK_MS", "8000"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
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

    let a = c.create_ticket("coordinate");
    let title = "mesimon-probe-1004 merge notice across a restart";
    let w = c.create_ticket(title);
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    let worker = format!("{kw} \"{title}\"");
    let notice = format!("Your branch {branch} has been merged into main");
    // The seat's transcript, for the case the tail reads after the restart.
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let append = |line: serde_json::Value| {
        use std::io::Write;
        let mut f = std::fs::File::options().create(true).append(true).open(&transcript).unwrap();
        writeln!(f, "{line}").unwrap();
    };
    if after == After::GuessedIdle {
        append(json!({"uuid":"p", "type":"user", "message":{"content":"brief"}}));
        let start = format!(
            r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
            transcript.display()
        );
        hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
        c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    }

    // ---- 1. delivered, then merged by the crown: the notice goes ----------
    commit(&path, "one.txt");
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
    let seen = read(&mut c, sa, &kw).unwrap().seen;
    match c
        .send(Principal::Agent { session: sa }, Command::AgentMergeTicket { key: kw.clone(), seen })
    {
        Response::AgentMerged { notice, .. } => assert_eq!(notice, "sent"),
        other => panic!("merge_ticket: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the merged notice", || {
        lines_with(&notice).len() == 1
    });
    // The notice's turn takes the words, and the landing waits for it.
    start(&mut c, ws);
    if after == After::GuessedIdle {
        // It calls a tool, in flight across the restart.
        append(json!({"uuid":"n", "type":"user", "message":{"content":"merged"}}));
        append(json!({"uuid":"c", "type":"assistant", "message":{"stop_reason":"tool_use",
            "content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}));
    }
    wait_until(std::time::Duration::from_secs(5), "the hold in the feed", || {
        std::fs::read_to_string(&feed_path)
            .is_ok_and(|f| f.contains("\"crown_wake_deferred:merge_step\""))
    });
    assert_eq!(lines_with(&worker).len(), 1, "held while the notice turn runs");
    drop(c);

    // ---- 2. the daemon restarts in that turn: Unknown, no word ------------
    h.restart_after(|| {});
    let mut c = h.client(&format!("{name}_2"));
    std::thread::sleep(std::time::Duration::from_millis(3000));
    let seat = c.board().live_agent(w).unwrap().clone();
    if after == After::GuessedIdle {
        // The tail seeds the tool in flight: running, at Low.
        assert!(
            seat.state == SessionState::Running && seat.confidence == Confidence::Low,
            "{:?} {:?}",
            seat.state,
            seat.confidence
        );
    } else {
        assert!(matches!(seat.state, SessionState::Unknown { .. }), "{:?}", seat.state);
    }
    assert_eq!(lines_with(&worker).len(), 1, "{}", std::fs::read_to_string(&got).unwrap());
    let merged = format!("{worker} merged and finished its turn (merge_state ahead → merged");
    let bare = format!("{worker} merged (merge_state ahead → merged");

    if after == After::GuessedIdle {
        // ---- 3. running when the window settles (8 s): still held ---------
        std::thread::sleep(std::time::Duration::from_millis(7000));
        assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running);
        assert_eq!(lines_with(&worker).len(), 1, "held while it reads running");
        // ---- 4. an abort record read as idle at Low: the merge alone -------
        append(json!({"uuid":"a", "type":"user",
            "message":{"content":"[Request interrupted by user]"}}));
        wait_until(std::time::Duration::from_secs(10), "the merge at the guess", || {
            lines_with(&worker).len() == 2
        });
        let seat = c.board().live_agent(w).unwrap().clone();
        assert!(matches!(seat.state, SessionState::Idle { .. }), "{:?}", seat.state);
        assert_eq!(seat.confidence, Confidence::Low);
        let said = lines_with(&worker);
        assert!(said[1].starts_with(&bare), "the merge, not the end: {said:?}");
        assert!(!said[1].contains("finished"), "{said:?}");
        assert!(said[1].contains("after a restart"), "{said:?}");
        // ---- 5. the turn goes on and ends: finished, once -----------------
        start(&mut c, sa);
        stop(&mut c, sa);
        hook_send(&hook_sock, &ws.to_string(), "PostToolUse", r#"{"tool_name":"Bash"}"#);
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        stop(&mut c, ws);
        wait_until(std::time::Duration::from_secs(10), "the turn's finish", || {
            lines_with(&worker).len() == 3
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let said = lines_with(&worker);
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(said[2].starts_with(&format!("{worker} finished its turn")), "{said:?}");
        assert_eq!(said.iter().filter(|l| l.contains(&format!("{worker} merged"))).count(), 1);
        return;
    }

    if after == After::RunsOn {
        // ---- 3. the turn goes on and ends: that is the wake, once ---------
        hook_send(&hook_sock, &ws.to_string(), "PostToolUse", r#"{"tool_name":"Bash"}"#);
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert_eq!(lines_with(&worker).len(), 1, "held while the turn runs on");
        stop(&mut c, ws);
        wait_until(std::time::Duration::from_secs(10), "the wake at the turn's end", || {
            lines_with(&worker).len() == 2
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let said = lines_with(&worker);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[1].starts_with(&merged), "{said:?}");
        assert!(said[1].contains("after a restart"), "{said:?}");
        return;
    }

    if after == After::CrownBusy {
        // The crown at work: the wakes are owed and wait for it.
        let wakes = |cause: &str| {
            std::fs::read_to_string(&feed_path)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|e| {
                    e["kind"] == "crown_wake" && e["worker"] == w.to_string() && e["cause"] == cause
                })
                .count()
        };
        start(&mut c, sa);
        // ---- 3. the window lets the merge go, owed to the busy crown ------
        wait_until(std::time::Duration::from_secs(15), "the merge owed", || wakes("merged") == 1);
        // ---- 4. the same turn's Stop: its end folds into the merge --------
        stop(&mut c, ws);
        wait_until(std::time::Duration::from_secs(10), "the finish owed", || {
            wakes("finished") == 1
        });
        assert_eq!(lines_with(&worker).len(), 1, "nothing reached the busy crown");
        // ---- 5. the crown is free: one line, the merge and its end --------
        stop(&mut c, sa);
        wait_until(std::time::Duration::from_secs(10), "the one line", || {
            lines_with(&worker).len() == 2
        });
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let said = lines_with(&worker);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[1].starts_with(&merged), "{said:?}");
        assert!(said[1].contains("after a restart"), "{said:?}");
        return;
    }

    // ---- 3. no hook comes: the window's end tells the merge alone ---------
    // The stub writes no transcript, so the seat stays `Unknown`; the
    // restart's look settles 8 s after the start.
    wait_until(std::time::Duration::from_secs(15), "the wake at the window's end", || {
        lines_with(&worker).len() == 2
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let said = lines_with(&worker);
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[1].starts_with(&bare), "the merge, not the end: {said:?}");
    assert!(!said[1].contains("finished"), "{said:?}");
    assert!(said[1].contains("after a restart"), "{said:?}");
    assert!(
        matches!(c.board().live_agent(w).unwrap().state, SessionState::Unknown { .. }),
        "let go on the window, not on an end: {:?}",
        c.board().live_agent(w).unwrap().state
    );

    // ---- 4. the turn's end, its own or the next prompt's: finished, once ---
    start(&mut c, sa);
    stop(&mut c, sa);
    if after == After::NextPrompt {
        start(&mut c, ws);
    }
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the turn's finish", || {
        lines_with(&worker).len() == 3
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let said = lines_with(&worker);
    assert_eq!(said.len(), 3, "{said:?}");
    assert!(said[2].starts_with(&format!("{worker} finished its turn")), "{said:?}");
    assert_eq!(said.iter().filter(|l| l.contains(&format!("{worker} merged"))).count(), 1);
}

/// The crown sends its own asks (T-550), where the person lets it: in
/// supervised mode the words wait for `^y` as T-413 built them; autonomous
/// (the default since T-610), an ask to an agent the crown STARTED goes by
/// the queue once that agent is idle — the
/// feed's actor is the agent, the card lights `♛ sent`, and the turn that
/// takes the words wakes the crown as a person's send would. An agent a
/// person started is held whatever the mode says, words that would wake a
/// parked agent need a free budget seat and hold one while they wait, and
/// supervising or uncrowning holds what had not gone yet.
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

    let a = c.create_ticket("coordinate");
    let p = c.create_ticket("the person's own");
    let w = c.create_ticket("mesimon-probe-91 worker");
    let w2 = c.create_ticket("second worker");
    let w3 = c.create_ticket("third worker");
    let (kp, kw, kw2, kw3) =
        (key_of(&mut c, p), key_of(&mut c, w), key_of(&mut c, w2), key_of(&mut c, w3));
    let sa = c.spawn_claude(a);
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
                expect_secs: None,
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
    let ask_as = |c: &mut TestClient, key: &str, text: &str, deliver: Deliver| {
        let v = read(c, sa, key).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: key.into(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { held_for_person, held_because, wake, .. } => {
                (held_for_person, held_because.unwrap_or_default(), wake)
            }
            other => panic!("ask_agent {key}: {other:?}"),
        }
    };
    let ask = |c: &mut TestClient, key: &str, text: &str| {
        let (held, why, _) = ask_as(c, key, text, Deliver::Idle);
        (held, why)
    };
    let row = |c: &mut TestClient, t: ulid::Ulid| {
        pending_of(c, Some(t)).into_iter().find(|p| p.is_queued_ask() && !p.in_flight)
    };

    // ---- supervised: held for a person, even on the crown's own worker -----
    assert!(c.board().crown_mode.sends(), "autonomous by default");
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let (held, why) = ask(&mut c, &kw, "mesimon-probe-92 held");
    assert!(held && why.contains("Crown mode: supervised"), "{why}");
    let r = row(&mut c, w).expect("held on W's card");
    assert!(!r.sends && r.by.is_some() && r.waits_on.is_empty(), "{r:?}");
    settle();
    assert!(!landed("mesimon-probe-92"), "held words do not go on their own");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));

    // ---- autonomous: refused to an agent, persisted, printed ----------------
    match c.send(
        Principal::Agent { session: sa },
        Command::SetCrownMode { mode: CrownMode::Autonomous },
    ) {
        Response::Err { .. } => {}
        other => panic!("an agent making it autonomous: {other:?}"),
    }
    assert!(!c.board().crown_mode.sends());
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    assert!(c.board().crown_mode.sends());
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_mode = \"autonomous\""), "{file}");

    // A person's agent waits for its person whatever the mode says.
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
    assert_eq!(said_on(&mut c, w).as_deref(), Some("mesimon-probe-94 then push"), "and what");
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
        ask_ends(&format!("{kw} \"mesimon-probe-91 worker\"")).iter().any(|l| landed(l))
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
            expect_secs: None,
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
    let (held, why, wake) = ask_as(&mut c, &kw, "mesimon-probe-96 wake for this", Deliver::Idle);
    assert!(held && why.contains("budget is spent") && why.contains(&kw2), "{why}");
    // T-880: the receipt says the words are W's wake, a person's send away.
    assert_eq!(wake, Some(AskWake::WhenSent));
    settle();
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Sleeping, "nobody woke it");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    // Words that would wake W onto a checkout another ticket's agent holds
    // wait for a person (T-583), as a start there would be refused.
    let q = c.create_ticket("the person's on the checkout");
    let kq = key_of(&mut c, q);
    let sq = c.spawn_claude(q);
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
    let (held, why, wake) = ask_as(&mut c, &kw, "mesimon-probe-97 wake for this", Deliver::Idle);
    assert!(!held, "{why}");
    assert_eq!(wake, Some(AskWake::WhenSent), "queued on the checkout, the wake is still owed");
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

    // ---- words sent now to a parked worker are its wake (T-880) -------------
    // The receipt says the agent woke, so the crown calls no start_agent.
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("sleep_agent again: {other:?}"),
    }
    c.await_state(ws, "sleeping", |s| *s == SessionState::Sleeping);
    let (held, why, wake) = ask_as(&mut c, &kw, "mesimon-probe-97b woken by words", Deliver::Now);
    assert!(!held, "{why}");
    assert_eq!(wake, Some(AskWake::Woken));
    assert_ne!(c.board().live_agent(w).unwrap().state, SessionState::Sleeping, "awake now");
    match start_agent(&mut c, &kw, "shared_checkout") {
        Response::Err { message } => assert!(message.contains("already has an agent"), "{message}"),
        other => panic!("a start after the words woke it: {other:?}"),
    }
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(15), "the waking words to land", || {
        landed("mesimon-probe-97b woken by words")
    });
    start(&mut c, ws);
    stop(&mut c, ws);

    // ---- supervised again, or the crown leaving, holds what had not gone ----
    start(&mut c, ws);
    let (held, _) = ask(&mut c, &kw, "mesimon-probe-98 not now");
    assert!(!held);
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let r = row(&mut c, w).expect("still on the card");
    assert!(!r.sends && r.waits_on.is_empty(), "held for the person now: {r:?}");
    stop(&mut c, ws);
    settle();
    assert!(!landed("mesimon-probe-98"), "supervised, nothing more goes");
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: w }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-81 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
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
    // An ask past the note cap is refused in words that say the count, the
    // limit and where the rest goes (T-896), never cut: a 10 KiB ask is
    // ordinary and queues as any other.
    {
        use mesimon_core::command::PROMPT_MAX_BYTES;
        let v = read(&mut c, sa, &kw).unwrap();
        let long = "x".repeat(PROMPT_MAX_BYTES + 1);
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: long,
                seen: v.seen.clone(),
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::Err { message } => {
                assert!(message.contains(&format!("{} bytes", PROMPT_MAX_BYTES + 1)), "{message}");
                assert!(
                    message.contains(&format!("limit is {PROMPT_MAX_BYTES} bytes")),
                    "{message}"
                );
                assert!(message.contains("write_note"), "{message}");
            }
            other => panic!("a 32 KiB ask was taken: {other:?}"),
        }
        assert!(pending_of(&mut c, Some(w)).is_empty(), "nothing queued by the refusal");
        let ten_kib = format!("mesimon-probe-80 {}", "y".repeat(10 * 1024));
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: ten_kib.clone(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { .. } => {}
            other => panic!("a 10 KiB ask was refused: {other:?}"),
        }
        wait_until(std::time::Duration::from_secs(10), "the 10 KiB ask in W's box", || {
            landed(&ten_kib)
        });
        hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
        start(&mut c, ws);
        stop(&mut c, ws);
        wait_until(std::time::Duration::from_secs(20), "the answer's wake on the crown", || {
            ask_ends(&format!("{kw} \"mesimon-probe-81 worker\"")).iter().any(|l| landed(l))
        });
        start(&mut c, sa);
        stop(&mut c, sa);
    }
    let answered = ask_ends(&format!("{kw} \"mesimon-probe-81 worker\""));
    let answers = || answered.iter().map(|l| read_got().matches(l.as_str()).count()).sum::<usize>();
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
                expect_secs: None,
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

    // ---- an ack that outlives the in-flight window (T-938) -----------------
    // No ack inside the window is no ack: the record stays at the last word
    // the board saw, no later prompt pays for it, and the end of the turn
    // the words went into lifts the block without calling them taken. On
    // the mod pass the stand-in engine's `entered` came with the submit,
    // which runs as a turn of its own: its end takes it.
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
    let word = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.map(|a| a.status);
    if test_road() == "mod" {
        assert_eq!(word(&mut c).as_deref(), Some("handed"));
        stop(&mut c, ws);
        answer_heard(&mut c, before);
        assert_eq!(word(&mut c).as_deref(), Some("taken"));
    } else {
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the window closed unacked");
        // A resend for its idle meanwhile is refused, naming the first (a
        // steer mid-turn goes beside it, T-974).
        match ask(&mut c, "mesimon-probe-85b again", Deliver::Idle) {
            Response::Err { message } => {
                assert!(message.contains("(sent,") && message.contains("is refused"), "{message}")
            }
            other => panic!("a second ask while the first is out: {other:?}"),
        }
        stop(&mut c, ws);
        settle();
        assert_eq!(answers(), before, "no turn of the ask's own ended");
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the last word the board saw");
        assert!(!landed("mesimon-probe-85b"), "the refused ask sent nothing");
    }

    // ---- supervised: held for a person, the send preset to now --------------
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
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

/// T-938, round 4: an ask whose own turn ends on an error is settled by
/// that end, a hook's `StopFailure`, as an `end_turn` settles it. On the mod
/// road the `entered` handed it and the turn it started has ended: `taken`,
/// which says nothing of whether the work succeeded. On either road the
/// crown's next ask to the worker goes, where it was refused for good.
#[test]
fn an_ask_whose_turn_fails_settles_and_the_next_ask_goes() {
    let Some(h) = Harness::boot_with_env(
        "crown_ask_failed",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_ask_failed");
    let got = h.dir.join("got.txt");
    let landed = |probe: &str| std::fs::read_to_string(&got).unwrap_or_default().contains(probe);
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let mod_road = test_road() == "mod";
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-9401 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
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
        landed(&format!("{kw} \"mesimon-probe-9401 worker\" finished its turn"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    let word = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.map(|a| a.status);
    let ask = |c: &mut TestClient, text: &str| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { asked: Some(a), .. } => a,
            other => panic!("ask_agent {text:?}: {other:?}"),
        }
    };

    let first = ask(&mut c, "mesimon-probe-9402 then the API fails");
    // The crown's own wake holds the shared checkout's queue for its window
    // while the stub acks nothing for it (T-936), so the wait is generous.
    wait_until(std::time::Duration::from_secs(25), "the words in W's pane", || {
        landed("mesimon-probe-9402 then the API fails")
    });
    if mod_road {
        wait_until(std::time::Duration::from_secs(10), "the mod's entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
    }
    start(&mut c, ws);
    let body = json!({ "error": "server_error", "last_assistant_message": "API Error" });
    hook_send_with(
        &hook_sock,
        &ws.to_string(),
        "StopFailure",
        Some("server_error"),
        &body.to_string(),
    );
    c.await_state(ws, "failed", |s| *s == SessionState::Failed { reason: FailReason::Server });
    if mod_road {
        wait_until(std::time::Duration::from_secs(10), "the failed end takes it", || {
            word(&mut c).as_deref() == Some("taken")
        });
    }
    let next = ask(&mut c, "mesimon-probe-9403 after the failure");
    assert_ne!(next.ask, first.ask, "the failed turn's end lifted the block");
}

/// T-938: the crown's ask is one record, which the receipt and `get_ticket`'s
/// `asked` both read in one word, and a second ask is refused while the
/// first is out. The T-930 sequence first: `now` into a working agent reads
/// `sent`, a resend with `immediately` is refused naming it and puts nothing
/// in, and the words reach the agent once. On the mod pass the stand-in
/// engine holds the submit's result to the turn's end as Claude Code does:
/// the running turn's end leaves it `sent` and still refused, `entered`
/// hands it, and the end of the turn it started takes it, the wake naming
/// the ask. On the hook set the paste's own prompt hook hands it, and the
/// end of the turn it went into wakes the crown with it still `handed`.
/// Then a person's prompt racing the paste's ack leaves it `sent`; a
/// restart keeps an ask still out at its word (T-974), and the turn's end
/// still settles it, resent by nothing; and on the mod pass the mod's
/// refusals drop it, by the agent, with no slower road taken.
#[test]
fn a_crown_ask_says_one_true_word_and_a_second_is_refused_while_it_is_out() {
    // Round 3: an Enter that fails after its paste went in, on the test's ask.
    let fail_enter = std::env::temp_dir().join(format!("msmn-fail-enter-{}", std::process::id()));
    let _ = std::fs::remove_file(&fail_enter);
    let Some(h) = Harness::boot_with_env(
        "crown_ask_words",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_TEST_FAIL_ENTER", fail_enter.to_str().unwrap()),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_ask_words");
    let got = h.dir.join("got.txt");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| read_got().contains(probe);
    let copies = |probe: &str| read_got().matches(probe).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));
    let mod_road = test_road() == "mod";
    let switch = |name: &str, on: bool| {
        let f = h.dir.join(name);
        if on {
            std::fs::write(f, "").unwrap();
        } else {
            let _ = std::fs::remove_file(f);
        }
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-938 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Each wake that reaches the crown's own pane (T-937's file) is taken as
    // a crown takes one, a turn on it, once it is there (T-971). A wake left
    // untaken is owed for the daemon's 10 s ack window and holds this shared
    // checkout's queue (T-936): W's next ask waited it out on a timer, five
    // times a run, and the run's length sat at nextest's kill.
    let got_crown = h.dir.join(format!("got-{ka}.txt"));
    let crown_lines = || std::fs::read_to_string(&got_crown).unwrap_or_default().lines().count();
    let taken = std::cell::Cell::new(0);
    let crown_takes = |c: &mut TestClient| {
        wait_until(std::time::Duration::from_secs(10), "a wake in the crown's pane", || {
            crown_lines() > taken.get()
        });
        taken.set(crown_lines());
        start(c, sa);
        stop(c, sa);
    };
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "W's finish on the crown", || {
        landed(&format!("{kw} \"mesimon-probe-938 worker\" finished its turn"))
    });
    crown_takes(&mut c);

    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked;
    let word = |c: &mut TestClient| asked(c).map(|a| a.status);
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
                expect_secs: None,
            },
        )
    };
    let receipt = |r: Response| match r {
        Response::AgentAsked { asked, .. } => asked.expect("the record rides the receipt"),
        other => panic!("ask_agent: {other:?}"),
    };
    let wake_for = |id: &str, said: &str| {
        let head = format!("{kw} \"mesimon-probe-938 worker\"");
        let line = if said == "sent" {
            format!("{head} ended a turn after ask {id} was sent; receipt unconfirmed")
        } else {
            format!("{head} ended its turn after your ask went in (ask {id}: {said}")
        };
        wait_until(std::time::Duration::from_secs(20), &format!("the wake for ask {said}"), || {
            landed(&line)
        });
    };
    let refused = |r: Response, first: &str| match r {
        Response::Err { message } => {
            assert!(message.contains(first) && message.contains("is refused"), "{message}")
        }
        other => panic!("a second ask while the first is out: {other:?}"),
    };

    // ---- the T-930 sequence --------------------------------------------------
    switch("mod-hold-entered", mod_road);
    start(&mut c, ws);
    let first = receipt(ask(&mut c, "mesimon-probe-9381 now", Deliver::Now));
    assert_eq!(first.status, "sent", "{first:?}");
    assert!(!first.ask.is_empty());
    wait_until(std::time::Duration::from_secs(5), "the words in W's turn", || {
        landed("mesimon-probe-9381 now")
    });
    // A resend for the agent's idle is refused (a steer mid-turn is not a
    // resend: T-974's test has it).
    refused(ask(&mut c, "mesimon-probe-9382 again", Deliver::Idle), &first.ask);
    assert_eq!(word(&mut c).as_deref(), Some("sent"), "the refusal left the first as it was");
    if mod_road {
        // Held behind the running turn: its end is not this ask's, and
        // neither is an unrelated prompt's turn after it (correction 3).
        stop(&mut c, ws);
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"));
        refused(ask(&mut c, "mesimon-probe-9382 again", Deliver::Now), &first.ask);
        start(&mut c, ws);
        stop(&mut c, ws);
        crown_takes(&mut c);
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "an unrelated prompt hands nothing");
        refused(ask(&mut c, "mesimon-probe-9382 again", Deliver::Now), &first.ask);
        switch("mod-release", true);
        wait_until(std::time::Duration::from_secs(10), "the mod's entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
        start(&mut c, ws);
        stop(&mut c, ws);
        wake_for(&first.ask, "taken");
        assert_eq!(word(&mut c).as_deref(), Some("taken"));
        switch("mod-hold-entered", false);
        switch("mod-release", false);
    } else {
        // A prompt hook names no words (correction 2): the paste stays
        // `sent`, and the turn's proven end lifts the block.
        hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "a prompt hook hands nothing");
        stop(&mut c, ws);
        wake_for(&first.ask, "sent");
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the last word a receipt proved");
    }
    settle();
    assert!(!landed("mesimon-probe-9382"), "a refused ask sent nothing");
    assert_eq!(copies("mesimon-probe-9381 now"), 1, "one copy reached the agent");
    crown_takes(&mut c);

    // ---- a person's prompt in the paste's ack window ---------------------------
    if !mod_road {
        start(&mut c, ws);
        let raced = receipt(ask(&mut c, "mesimon-probe-9383 raced", Deliver::Now));
        wait_until(std::time::Duration::from_secs(5), "the crown's words", || {
            landed("mesimon-probe-9383 raced")
        });
        match c.request(Command::PromptSession {
            ticket: w,
            text: "mesimon-probe-9384 the person's".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }) {
            Response::Ok | Response::Queued { .. } => {}
            other => panic!("the person's prompt: {other:?}"),
        }
        wait_until(std::time::Duration::from_secs(5), "the person's words", || {
            landed("mesimon-probe-9384")
        });
        hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the ack may be the person's");
        refused(ask(&mut c, "mesimon-probe-9385 meanwhile", Deliver::Idle), &raced.ask);
        stop(&mut c, ws);
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the last word the board saw");
        crown_takes(&mut c);
    }

    // ---- a restart: an ask still `sent` is kept at its word (T-974) -------------
    switch("mod-hold-entered", mod_road);
    start(&mut c, ws);
    let out = receipt(ask(&mut c, "mesimon-probe-9386 before a restart", Deliver::Now));
    assert_eq!(out.status, "sent");
    wait_until(std::time::Duration::from_secs(5), "the words in W's turn", || {
        landed("mesimon-probe-9386")
    });
    h.restart();
    let mut c = h.client("crown_ask_words_2");
    let kept = asked(&mut c).expect("the ask out was kept");
    assert_eq!((kept.status.as_str(), kept.ask.as_str()), ("sent", out.ask.as_str()));
    settle();
    assert_eq!(copies("mesimon-probe-9386"), 1, "nothing resent");
    refused(ask(&mut c, "mesimon-probe-9386b again", Deliver::Idle), &out.ask);
    stop(&mut c, ws);
    if mod_road {
        // The held submit waits for its own turn, across the restart too.
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("sent"));
        switch("mod-release", true);
        wait_until(std::time::Duration::from_secs(10), "the mod's entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
        start(&mut c, ws);
        stop(&mut c, ws);
        wake_for(&out.ask, "taken");
        switch("mod-release", false);
    }
    switch("mod-hold-entered", false);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a restart: an ask a turn carried comes back at its word (T-974) --------
    start(&mut c, ws);
    let carried = receipt(ask(&mut c, "mesimon-probe-9387 carried", Deliver::Now));
    wait_until(std::time::Duration::from_secs(5), "the words in W's turn", || {
        landed("mesimon-probe-9387")
    });
    // The prompt hook a paste would fire; on the mod road the bridge may
    // still be coming back from the restart, and the words took the paste.
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    if mod_road {
        wait_until(std::time::Duration::from_secs(10), "the ask handed", || {
            word(&mut c).as_deref() == Some("handed")
        });
    } else {
        // The hook marks the turn after it; the ask stays `sent`.
        settle();
    }
    h.restart();
    let mut c = h.client("crown_ask_words_3");
    let back = asked(&mut c).expect("the turn's ask was kept");
    let kept_word = if mod_road { "handed" } else { "sent" };
    assert_eq!((back.status.as_str(), back.ask.as_str()), (kept_word, carried.ask.as_str()));
    assert!(back.why.is_empty(), "its word, not a lost outcome: {back:?}");
    settle();
    assert_eq!(copies("mesimon-probe-9387"), 1, "nothing resent");
    // The turn's end settles it across the restart and wakes the crown.
    stop(&mut c, ws);
    wake_for(&carried.ask, if mod_road { "taken" } else { "sent" });
    start(&mut c, sa);
    stop(&mut c, sa);
    let again = receipt(ask(&mut c, "mesimon-probe-9388 after", Deliver::Idle));
    assert_ne!(again.ask, carried.ask);

    // ---- the mod's refusals drop it, by the agent ------------------------------
    if mod_road {
        crown_takes(&mut c);
        wait_until(std::time::Duration::from_secs(10), "the idle ask's turn", || {
            landed("mesimon-probe-9388")
        });
        start(&mut c, ws);
        stop(&mut c, ws);
        crown_takes(&mut c);
        switch("mod-draft", true);
        start(&mut c, ws);
        let over = receipt(ask(&mut c, "mesimon-probe-9389 over a draft", Deliver::Immediately));
        wait_until(std::time::Duration::from_secs(10), "the refused fill", || {
            word(&mut c).as_deref() == Some("dropped")
        });
        let dropped = asked(&mut c).unwrap();
        assert_eq!((dropped.ask.as_str(), dropped.by.as_str()), (over.ask.as_str(), "agent"));
        assert!(dropped.why.contains("draft"), "{dropped:?}");
        settle();
        let frames = std::fs::read_to_string(h.dir.join(format!("mod-{ws}.ndjson"))).unwrap();
        assert!(
            frames
                .lines()
                .filter(|l| l.contains("mesimon-probe-9389"))
                .all(|l| l.contains("\"fill\"")),
            "no submit after the refused fill: {frames}"
        );
        switch("mod-draft", false);
        stop(&mut c, ws);
        crown_takes(&mut c);
        switch("mod-drop-submit", true);
        let _ = receipt(ask(&mut c, "mesimon-probe-9390 refused", Deliver::Idle));
        wait_until(std::time::Duration::from_secs(10), "the refused submit", || {
            word(&mut c).as_deref() == Some("dropped")
        });
        assert_eq!(asked(&mut c).unwrap().by, "agent");
        switch("mod-drop-submit", false);
        // A refusal after the owed window still drops that ask (correction
        // 4): the submit is held past `INFLIGHT_MS`, then refused late. The
        // wait is for this send's own expiry: a new `submit_unacked` for W
        // after the feed's count before it (round 2, C).
        let feed_path = h.paths.state_dir.join("activity.jsonl");
        let unacked = || {
            std::fs::read_to_string(&feed_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("\"submit_unacked\"") && l.contains(&w.to_string()))
                .count()
        };
        switch("mod-hold-entered", true);
        let before = unacked();
        let late = receipt(ask(&mut c, "mesimon-probe-9391 refused late", Deliver::Idle));
        wait_until(std::time::Duration::from_secs(20), "this send's owed window to close", || {
            unacked() > before
        });
        assert_eq!(word(&mut c).as_deref(), Some("sent"), "the mod took the frame; it waits");
        write_whole(&h.dir.join("mod-release"), "dropped");
        wait_until(std::time::Duration::from_secs(10), "the late refusal", || {
            word(&mut c).as_deref() == Some("dropped")
        });
        let dropped = asked(&mut c).unwrap();
        assert_eq!((dropped.ask.as_str(), dropped.by.as_str()), (late.ask.as_str(), "agent"));
        switch("mod-hold-entered", false);
        switch("mod-release", false);
        start(&mut c, ws);
        stop(&mut c, ws);
        crown_takes(&mut c);

        // ---- a prompt's mark, then a late confirmed refusal (round 2, B) ----
        // The fill's words went in, an unrelated prompt hook marked the turn
        // after them, and the mod's refusal came after its owed window: the
        // ask is dropped, and the turn's end says nothing of it.
        start(&mut c, ws);
        let marked = receipt(ask(&mut c, "mesimon-probe-9392 marked", Deliver::Immediately));
        // A fill types no Enter, so the stub has no line of it: the frame
        // in the stand-in engine's record is the sign it went.
        let mod_record = h.dir.join(format!("mod-{ws}.ndjson"));
        wait_until(std::time::Duration::from_secs(10), "the fill frame", || {
            std::fs::read_to_string(&mod_record).is_ok_and(|f| f.contains("mesimon-probe-9392"))
        });
        settle();
        hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
        settle();
        let fill = std::fs::read_to_string(h.dir.join(format!("mod-{ws}.ndjson")))
            .unwrap()
            .lines()
            .rfind(|l| l.contains("mesimon-probe-9392"))
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .expect("the fill frame");
        hook_send_road(
            &hook_sock,
            &ws.to_string(),
            "ModFill",
            fill["id"].as_str(),
            None,
            r#"{"outcome":"refused","reason":"draft"}"#,
            Some("mod"),
        );
        wait_until(std::time::Duration::from_secs(10), "the late refusal of the fill", || {
            word(&mut c).as_deref() == Some("dropped")
        });
        stop(&mut c, ws);
        settle();
        assert!(
            !landed(&format!("ask {}", marked.ask)),
            "no turn end speaks for a dropped ask: {}",
            read_got()
        );
        crown_takes(&mut c);

        // ---- handed, then the pane replaced before its turn ends (round 2, B)
        // A submit the mod entered, at a worker then parked and woken into a
        // new pane: the successor's turn end is not the ask's, the ask is
        // unknown, and nothing blocks for good.
        let handed = receipt(ask(&mut c, "mesimon-probe-9393 before a new pane", Deliver::Idle));
        wait_until(std::time::Duration::from_secs(25), "the mod's entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
        assert!(matches!(c.request(Command::SleepSession { id: ws }), Response::Ok));
        c.await_state(ws, "asleep", |s| *s == SessionState::Sleeping);
        match c.request(Command::WakeSession { id: ws }) {
            Response::Spawned { .. } => {}
            other => panic!("the wake: {other:?}"),
        }
        start(&mut c, ws);
        stop(&mut c, ws);
        crown_takes(&mut c);
        settle();
        let gone = asked(&mut c).unwrap();
        assert_eq!((gone.status.as_str(), gone.ask.as_str()), ("unknown", handed.ask.as_str()));
        assert!(gone.why.contains("replaced"), "{gone:?}");
        assert!(
            !landed(&format!("ask {}", handed.ask)),
            "the successor's turn carries none of it: {}",
            read_got()
        );
        let next = receipt(ask(&mut c, "mesimon-probe-9394 after", Deliver::Idle));
        assert_ne!(next.ask, handed.ask, "no permanent block");
        wait_until(std::time::Duration::from_secs(25), "the next ask entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
        start(&mut c, ws);
        stop(&mut c, ws);
        wake_for(&next.ask, "taken");
        crown_takes(&mut c);

        // ---- a submit refused after a stop goes in by paste (item 10) -----
        // Claude Code holds a plugin's input after a person's Esc (T-894),
        // and the words go in by the paste road into the empty box: the
        // ask's route is the paste's from there, so it reads sent, blocks a
        // second ask, and the paste's own turn end releases it, with no
        // `entered` of the refused frame to wait for.
        switch("mod-stopped-submit", true);
        let pasted = receipt(ask(&mut c, "mesimon-probe-9395 after a stop", Deliver::Idle));
        wait_until(std::time::Duration::from_secs(25), "the words by paste", || {
            std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl"))
                .unwrap_or_default()
                .contains("\"prompt_pasted_after_refusal\"")
                && landed("mesimon-probe-9395 after a stop")
        });
        switch("mod-stopped-submit", false);
        assert_eq!(word(&mut c).as_deref(), Some("sent"));
        refused(ask(&mut c, "mesimon-probe-9396 again", Deliver::Now), &pasted.ask);
        start(&mut c, ws);
        stop(&mut c, ws);
        wake_for(&pasted.ask, "sent");
        crown_takes(&mut c);
        let after = receipt(ask(&mut c, "mesimon-probe-9397 next", Deliver::Idle));
        assert_ne!(after.ask, pasted.ask, "the paste's turn end lifted the block");
        assert!(!landed("mesimon-probe-9396"), "the refused words reach nobody");
        wait_until(std::time::Duration::from_secs(25), "the next ask entered", || {
            word(&mut c).as_deref() == Some("handed")
        });
        start(&mut c, ws);
        stop(&mut c, ws);
        wake_for(&after.ask, "taken");
        crown_takes(&mut c);

        // ---- that paste's Enter fails after the words went in (round 3, 2)
        // The refused submit's words are pasted and the Enter after them
        // fails: whether the agent has them is not known, so the ask reads
        // unknown, never dropped, and nothing is tried again.
        switch("mod-stopped-submit", true);
        std::fs::write(&fail_enter, "").unwrap();
        let half = receipt(ask(&mut c, "mesimon-probe-9398 half sent", Deliver::Idle));
        wait_until(std::time::Duration::from_secs(25), "the paste's failure read", || {
            word(&mut c).as_deref() == Some("unknown")
        });
        let _ = std::fs::remove_file(&fail_enter);
        switch("mod-stopped-submit", false);
        let read_half = asked(&mut c).unwrap();
        assert_eq!(read_half.ask, half.ask);
        assert!(read_half.why.contains("failed part-way"), "{read_half:?}");
        settle();
        assert_eq!(word(&mut c).as_deref(), Some("unknown"), "not overwritten as dropped");
        // The box holds the words the Enter never sent: the person's to send.
        assert!(c.board().live_agent(w).is_some_and(|s| s.unsent.is_some()), "kept unsent");

        // ---- a parked seat's frame is its live pane's (round 3, 1) ---------
        // Words to a parked worker go down its mod as a frame in the pane its
        // wake made, bound there at the dispatch. An `entered` for that frame
        // while the seat has no pane, or once a new pane replaced it, hands
        // nothing, and the tick resolves the ask, so nothing blocks for good.
        let v = read(&mut c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
        ) {
            Response::AgentTicket { .. } => {}
            other => panic!("sleep_agent: {other:?}"),
        }
        c.await_state(ws, "sleeping", |s| *s == SessionState::Sleeping);
        switch("mod-hold-entered", true);
        let parked = receipt(ask(&mut c, "mesimon-probe-9399 to a parked seat", Deliver::Now));
        assert_eq!(parked.status, "sent", "{parked:?}");
        hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
        let mod_record = h.dir.join(format!("mod-{ws}.ndjson"));
        let frame_of = |probe: &str| {
            std::fs::read_to_string(&mod_record)
                .unwrap_or_default()
                .lines()
                .rfind(|l| l.contains(probe))
                .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["id"].to_string())
        };
        wait_until(std::time::Duration::from_secs(15), "the parked words' frame", || {
            frame_of("mesimon-probe-9399").is_some()
        });
        let old = frame_of("mesimon-probe-9399").unwrap().trim_matches('"').to_string();
        let entered = |fid: &str| {
            hook_send_road(
                &hook_sock,
                &ws.to_string(),
                "ModSubmit",
                Some(fid),
                None,
                r#"{"outcome":"entered"}"#,
                Some("mod"),
            )
        };
        assert!(matches!(c.request(Command::SleepSession { id: ws }), Response::Ok));
        c.await_state(ws, "asleep", |s| *s == SessionState::Sleeping);
        switch("mod-hold-entered", false);
        entered(&old);
        settle();
        assert_ne!(word(&mut c).as_deref(), Some("handed"), "no pane: the report is no one's");
        match c.request(Command::WakeSession { id: ws }) {
            Response::Spawned { .. } => {}
            other => panic!("the wake: {other:?}"),
        }
        hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
        entered(&old);
        settle();
        assert_ne!(word(&mut c).as_deref(), Some("handed"), "a new pane: still no one's");
        wait_until(std::time::Duration::from_secs(10), "the tick resolves it", || {
            word(&mut c).as_deref() == Some("unknown")
        });
        let next = receipt(ask(&mut c, "mesimon-probe-9400 after", Deliver::Idle));
        assert_ne!(next.ask, parked.ask, "no permanent block");
    }
}

/// T-974: a steer goes beside the crown's ask still out. The worker's turn
/// runs on the crown's first ask (`handed` by the mod's `entered` on the mod
/// pass, `sent` by its paste on the hook set); `immediately` into that turn
/// is admitted as an ask of its own, its receipt its own id, the first in
/// `earlier` at its word, and its words reach the agent mid-turn. `idle` is
/// still refused while the first is out, naming it; the steer's prompt hook
/// leaves the turn the first's, so its end speaks of the first and wakes
/// the crown with it. Then a restart between the ask going out and the
/// turn's end keeps the ask's word, and the end still settles it and wakes
/// the crown.
#[test]
fn a_crown_steer_goes_beside_its_ask_out_and_a_restart_keeps_the_ask() {
    let Some(h) = Harness::boot_with_env(
        "crown_steer",
        Some(RAW_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_steer");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let landed = |probe: &str| read_got().contains(probe);
    let sent_now = |probe: &str| read_got().contains(&format!("{probe}\u{18}\u{13}"));
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));
    let mod_road = test_road() == "mod";

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-974 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
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
        landed(&format!("{kw} \"mesimon-probe-974 worker\" finished its turn"))
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked;
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
                expect_secs: None,
            },
        )
    };
    let receipt = |r: Response| match r {
        Response::AgentAsked { asked, .. } => asked.expect("the record rides the receipt"),
        other => panic!("ask_agent: {other:?}"),
    };
    let refused = |r: Response, first: &str| match r {
        Response::Err { message } => {
            assert!(message.contains(first) && message.contains("is refused"), "{message}")
        }
        other => panic!("an idle ask while the first is out: {other:?}"),
    };
    // The word the first ask's turn proves: the mod's `entered` hands it on
    // the mod pass; a paste's prompt hook names no words on the hook set.
    let out_word = if mod_road { "handed" } else { "sent" };
    let wake_for = |id: &str| {
        let head = format!("{kw} \"mesimon-probe-974 worker\"");
        let line = if mod_road {
            format!("{head} ended its turn after your ask went in (ask {id}: taken")
        } else {
            format!("{head} ended a turn after ask {id} was sent; receipt unconfirmed")
        };
        wait_until(std::time::Duration::from_secs(20), "the wake for the first ask", || {
            landed(&line)
        });
    };
    // The crown's first ask, at the idle worker: its turn runs on it.
    let first_ask = |c: &mut TestClient, probe: &str| {
        let first = receipt(ask(c, probe, Deliver::Idle));
        wait_until(std::time::Duration::from_secs(25), "the first ask's words", || landed(probe));
        start(c, ws);
        wait_until(std::time::Duration::from_secs(10), "the first ask out", || {
            asked(c).is_some_and(|v| v.ask == first.ask && v.status == out_word)
        });
        first
    };

    // ---- a steer beside the ask out ------------------------------------------
    let first = first_ask(&mut c, "mesimon-probe-9741 first");
    let steer = receipt(ask(&mut c, "mesimon-probe-9742 steer", Deliver::Immediately));
    assert_ne!(steer.ask, first.ask, "the steer is an ask of its own");
    assert_eq!(steer.status, "sent", "{steer:?}");
    let earlier: Vec<_> =
        steer.earlier.iter().map(|e| (e.ask.as_str(), e.status.as_str())).collect();
    assert_eq!(earlier, [(first.ask.as_str(), out_word)], "the first keeps its word");
    wait_until(std::time::Duration::from_secs(5), "the steer's words and send-now", || {
        sent_now("mesimon-probe-9742 steer")
    });
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "mid-turn");
    let sent_line = || {
        std::fs::read_to_string(&feed_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("\"ask_agent_sent\"") && l.contains(&steer.ask))
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .next()
    };
    wait_until(std::time::Duration::from_secs(5), "the steer's send names its ask", || {
        sent_line().is_some()
    });
    let line = sent_line().unwrap();
    assert_eq!(line["outcome"], "immediately", "{line}");
    assert_eq!(line["beside"], serde_json::json!([format!("{} {out_word}", first.ask)]), "{line}");
    // Idle still keeps the one-ask rule, naming the first.
    refused(ask(&mut c, "mesimon-probe-9743 idle", Deliver::Idle), &first.ask);
    // The steer's own prompt hook takes the turn from nobody.
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    settle();
    let now = asked(&mut c).unwrap();
    assert_eq!(now.ask, steer.ask);
    let earlier: Vec<_> = now.earlier.iter().map(|e| (e.ask.as_str(), e.status.as_str())).collect();
    assert_eq!(earlier, [(first.ask.as_str(), out_word)], "the steer's ack moved nothing");
    stop(&mut c, ws);
    wake_for(&first.ask);
    assert!(!landed(&format!("ask {}", steer.ask)), "the turn's end is the first's");
    // The feed's wake names the ask its turn answered, and its word (round 1).
    let named = format!("\"ask\":\"{} {}\"", first.ask, if mod_road { "taken" } else { "sent" });
    wait_until(std::time::Duration::from_secs(5), "the wake's feed line names the first", || {
        std::fs::read_to_string(&feed_path).unwrap_or_default().lines().any(|l| {
            l.contains("\"kind\":\"crown_wake\"")
                && l.contains("\"cause\":\"answered\"")
                && l.contains(&named)
        })
    });
    let after = asked(&mut c).unwrap();
    let first_word = if mod_road { "taken" } else { "sent" };
    assert_eq!(after.earlier[0].status, first_word, "{after:?}");
    assert!(!landed("mesimon-probe-9743"), "a refused ask sent nothing");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a restart between the ask going out and its turn's end -------------
    let carried = first_ask(&mut c, "mesimon-probe-9744 before a restart");
    h.restart();
    let mut c = h.client("crown_steer_2");
    let back = asked(&mut c).expect("the ask out was kept");
    assert_eq!((back.status.as_str(), back.ask.as_str()), (out_word, carried.ask.as_str()));
    assert!(back.why.is_empty(), "its word, not a lost outcome: {back:?}");
    settle();
    assert_eq!(asked(&mut c).unwrap().status, out_word, "the tick leaves it to the turn's end");
    refused(ask(&mut c, "mesimon-probe-9745 idle", Deliver::Idle), &carried.ask);
    stop(&mut c, ws);
    wake_for(&carried.ask);
    if mod_road {
        assert_eq!(asked(&mut c).unwrap().status, "taken");
    }
    assert_eq!(read_got().matches("mesimon-probe-9744").count(), 1, "nothing resent");
    let next = receipt(ask(&mut c, "mesimon-probe-9746 after", Deliver::Idle));
    assert_ne!(next.ask, carried.ask, "the turn's end lifted the block");
    // The crown's wake holds the shared checkout's queue for its owed
    // window (T-936) before the ask goes.
    wait_until(std::time::Duration::from_secs(25), "the idle ask's words", || {
        landed("mesimon-probe-9746")
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- a restart while the worker stands at a dialog (round 1) -------------
    // A dialog is mid-turn: the restart's look at it answers nothing, the
    // ask and its turn stay open, and the end after the answer takes it
    // and wakes the crown once, naming it.
    let at_dialog = first_ask(&mut c, "mesimon-probe-9747 then a dialog");
    let permission = r#"{"tool_name":"Bash"}"#;
    hook_send(&hook_sock, &ws.to_string(), "PermissionRequest", permission);
    c.await_state(ws, "at the dialog", |s| matches!(s, SessionState::RequiresAction { .. }));
    h.restart();
    let mut c = h.client("crown_steer_3");
    let back = asked(&mut c).expect("the ask out was kept");
    assert_eq!((back.status.as_str(), back.ask.as_str()), (out_word, at_dialog.ask.as_str()));
    c.await_state(ws, "still at the dialog", |s| matches!(s, SessionState::RequiresAction { .. }));
    // The restart's look runs on the tick for a worker at a dialog.
    settle();
    settle();
    let wakes = |id: &str| read_got().matches(&format!("ask {id}")).count();
    assert_eq!(wakes(&at_dialog.ask), 0, "the look at a dialog answers nothing");
    assert_eq!(asked(&mut c).unwrap().status, out_word);
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", permission);
    c.await_state(ws, "past the dialog", |s| *s == SessionState::Running);
    stop(&mut c, ws);
    wake_for(&at_dialog.ask);
    settle();
    assert_eq!(wakes(&at_dialog.ask), 1, "one end, one wake: {}", read_got());
    if mod_road {
        assert_eq!(asked(&mut c).unwrap().status, "taken");
    }
    let after = receipt(ask(&mut c, "mesimon-probe-9748 after the dialog", Deliver::Idle));
    assert_ne!(after.ask, at_dialog.ask, "nothing blocks after the end");
    if !mod_road {
        return;
    }

    // ---- a held submit, a steer, and its entered in one stretch (round 1) ---
    // ST2 on the real mod: the engine's Stop and the held submit's prompt
    // came 0.3 s apart, so the board saw one working stretch. The submit's
    // `entered` takes the turn's mark from the steer, the one end takes it
    // and names it, and the same words reached the agent twice.
    wait_until(std::time::Duration::from_secs(25), "the idle ask's words", || {
        landed("mesimon-probe-9748")
    });
    wait_until(std::time::Duration::from_secs(10), "the idle ask handed", || {
        asked(&mut c).is_some_and(|v| v.ask == after.ask && v.status == "handed")
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    start(&mut c, sa);
    stop(&mut c, sa);
    let switch = |name: &str, on: bool| {
        let f = h.dir.join(name);
        if on {
            std::fs::write(f, "").unwrap();
        } else {
            let _ = std::fs::remove_file(f);
        }
    };
    switch("mod-hold-entered", true);
    start(&mut c, ws);
    let twice = "mesimon-probe-9749 twice";
    let held = receipt(ask(&mut c, twice, Deliver::Now));
    assert_eq!(held.status, "sent", "held to the running turn's end");
    wait_until(std::time::Duration::from_secs(5), "the held submit's words", || landed(twice));
    let steer = receipt(ask(&mut c, twice, Deliver::Immediately));
    let earlier: Vec<_> =
        steer.earlier.iter().map(|e| (e.ask.as_str(), e.status.as_str())).collect();
    assert_eq!(earlier, [(held.ask.as_str(), "sent")], "the crown is told the submit is out");
    wait_until(std::time::Duration::from_secs(5), "the steer's send-now", || sent_now(twice));
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    switch("mod-release", true);
    let held_word = |c: &mut TestClient| {
        asked(c).and_then(|v| v.earlier.into_iter().find(|e| e.ask == held.ask)).map(|e| e.status)
    };
    wait_until(std::time::Duration::from_secs(10), "the held submit entered", || {
        held_word(&mut c).as_deref() == Some("handed")
    });
    stop(&mut c, ws);
    wake_for(&held.ask);
    settle();
    assert!(!landed(&format!("ask {}", steer.ask)), "the one end is the submit's: {}", read_got());
    assert_eq!(held_word(&mut c).as_deref(), Some("taken"));
    assert_eq!(read_got().matches(twice).count(), 2, "the same words, as the steer and the submit");
    switch("mod-hold-entered", false);
    switch("mod-release", false);
    let next = receipt(ask(&mut c, "mesimon-probe-9750 after", Deliver::Idle));
    assert_ne!(next.ask, held.ask, "nothing blocks after the end");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-71 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
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
    let answered = ask_ends(&format!("{kw} \"mesimon-probe-71 worker\""));
    let answers = || answered.iter().map(|l| read_got().matches(l.as_str()).count()).sum::<usize>();
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
                expect_secs: None,
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
            uploads: Vec::new(),
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
                uploads: Vec::new(),
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

    // ---- a person has W's pane open: no key of the board's goes in (T-958) ---
    let sid = ws.simple().to_string()[..16].to_string();
    let tmux_sock = h.paths.tmux_sock();
    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.expect("the crown's ask");
    let client = person_attaches(&tmux_sock, &sid);
    if test_road() == "mod" {
        // The mod fills the words, and the send-now over them is a key:
        // withheld, and the ask unknown. Every byte W's pane read from here
        // on is checked for the send-now's keys, wherever they fall.
        let mark = read_got().len();
        assert_eq!(
            road_of(ask(&mut c, "mesimon-probe-76 under hands", Deliver::Immediately)),
            (false, AskRoad::SentImmediately)
        );
        feed_has("daemon", "prompt_send_now_failed", Some("a_person_attached"));
        wait_until(std::time::Duration::from_secs(5), "the words filled", || {
            landed("mesimon-probe-76 under hands")
        });
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let delta = read_got()[mark..].to_string();
        assert!(
            !delta.contains('\u{18}') && !delta.contains('\u{13}'),
            "no send-now key under their hands: {delta:?}"
        );
        let out = asked(&mut c);
        assert_eq!(out.status, "unknown");
        assert!(out.why.contains("a person has its pane open"), "{out:?}");
        person_leaves(&tmux_sock, &sid, client);
    } else {
        // A person's own send straight to the pane is refused, not typed.
        match c.request(Command::PromptSession {
            ticket: w,
            text: "mesimon-probe-77 by hand".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }) {
            Response::Err { message } => {
                assert!(message.contains("a person has its pane open"), "{message}");
                assert!(message.contains("were not sent"), "{message}");
            }
            other => panic!("a send under a person's hands: {other:?}"),
        }
        feed_has("daemon", "prompt_not_sent", Some("a_person_attached"));
        // The crown's ask keeps its words on the queue, and says so.
        match ask(&mut c, "mesimon-probe-76 under hands", Deliver::Immediately) {
            Response::Err { message } => {
                assert!(message.contains("a person has its pane open"), "{message}");
                assert!(message.contains("stay queued"), "{message}");
            }
            other => panic!("ask_agent under a person's hands: {other:?}"),
        }
        feed_has("daemon", "queued_ask_kept", Some("a_person_attached"));
        assert_eq!(asked(&mut c).status, "queued");
        // A person's direct send now, ahead of that queued ask, is refused
        // before any key: nothing was said ahead of it, so the ask stays
        // queued and its record with it (round 2).
        match c.request(Command::PromptSession {
            ticket: w,
            text: "mesimon-probe-79 ahead".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }) {
            Response::Err { message } => assert!(message.contains("were not sent"), "{message}"),
            other => panic!("a send ahead of the queue: {other:?}"),
        }
        assert_eq!(asked(&mut c).status, "queued", "the queued ask outlives the refusal");
        assert!(pending_of(&mut c, Some(w)).iter().any(|p| !p.in_flight), "still on the queue");
        // And a column's send: W's refusal leaves its ask as it was. (The
        // crown's own pane may share the column; it is no one's ask here.)
        let column = c.board().ticket(w).unwrap().column.clone();
        match c.request(Command::PromptColumn {
            column,
            text: "mesimon-probe-80 by column".into(),
            queued: false,
            accept_plan: false,
        }) {
            Response::Asked { failed, .. } => assert!(failed >= 1, "W's send refused"),
            other => panic!("a column's send: {other:?}"),
        }
        assert_eq!(asked(&mut c).status, "queued", "the column's refusal keeps it too");
        assert!(pending_of(&mut c, Some(w)).iter().any(|p| !p.in_flight), "still on the queue");
        let by_column = read_got().matches("mesimon-probe-80").count();
        // W at its idle takes words, and the person is still there.
        stop(&mut c, ws);
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert!(!landed("mesimon-probe-76"), "nothing typed under their hands");
        assert!(!landed("mesimon-probe-77"));
        assert!(!landed("mesimon-probe-79"));
        assert_eq!(asked(&mut c).status, "queued");
        // The person gone, the queued words go, once; the refused ones never.
        person_leaves(&tmux_sock, &sid, client);
        wait_until(std::time::Duration::from_secs(10), "the queued words", || {
            landed("mesimon-probe-76 under hands")
        });
        assert_eq!(asked(&mut c).status, "sent");
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_eq!(read_got().matches("mesimon-probe-76").count(), 1, "the queued ask once");
        assert!(!landed("mesimon-probe-79"), "the refused send never");
        assert_eq!(
            read_got().matches("mesimon-probe-80").count(),
            by_column,
            "nor W's column send"
        );
        // The turn those words started ends, and the next one runs.
        start(&mut c, ws);
        stop(&mut c, ws);
        start(&mut c, ws);
        // A person who opens the pane just before an ask drains: W idle,
        // the crown's `idle` ask drains in the same writer turn it is made
        // in, while the pane sample (every 2 s) still says nobody is there,
        // so the paste's own read refuses it and the entry goes back
        // (`queued_ask_kept`). A sample that lands inside the milliseconds
        // between the attach and the ask holds it first instead, so the
        // specimen goes again until the dispatch's refusal is seen. Either
        // way the words stay queued while the person is there and go once
        // after they leave.
        let kept = || {
            std::fs::read_to_string(&feed_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("\"cmd\":\"queued_ask_kept\""))
                .count()
        };
        let wakes_sent = || {
            std::fs::read_to_string(&feed_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains("\"cmd\":\"crown_wake_sent\""))
                .count()
        };
        let mut refused_at_dispatch = false;
        for n in 0..5 {
            let probe = format!("mesimon-probe-78{n} at the idle");
            // W's stop wakes the crown, in the same checkout: the crown
            // takes that wake and goes idle, so nothing holds the queue.
            let woken = wakes_sent();
            stop(&mut c, ws);
            wait_until(std::time::Duration::from_secs(10), "the crown's wake", || {
                wakes_sent() > woken
            });
            start(&mut c, sa);
            stop(&mut c, sa);
            let before = kept();
            let client = person_attaches(&tmux_sock, &sid);
            let _ = ask(&mut c, &probe, Deliver::Idle);
            std::thread::sleep(std::time::Duration::from_millis(2000));
            assert!(!landed(&probe), "nothing typed under their hands");
            assert_eq!(asked(&mut c).status, "queued");
            refused_at_dispatch = kept() > before;
            person_leaves(&tmux_sock, &sid, client);
            wait_until(std::time::Duration::from_secs(10), "the idle ask", || landed(&probe));
            std::thread::sleep(std::time::Duration::from_millis(1500));
            assert_eq!(read_got().matches(probe.as_str()).count(), 1, "once");
            // The turn those words start runs; the next try's stop ends it.
            start(&mut c, ws);
            if refused_at_dispatch {
                break;
            }
        }
        assert!(refused_at_dispatch, "the dispatch's own refusal kept the entry");
    }
    stop(&mut c, ws);

    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-7"), "the feed never carries the words");
}

/// A queued ask whose road would type is kept, not lost, when the last
/// key's guard refuses it (T-958, round 1). A mod that speaks `submit` and
/// not `fill` takes `immediately` by the paste road, so a person in W's
/// pane refuses the crown's ask at its dispatch: the entry goes back, the
/// record reads `queued`, the feed says `queued_ask_kept`, and nothing is
/// typed. With the person gone the drain sends it once, by the `submit`, at
/// W's idle. The mod road alone: the hook road has no `submit` to tell.
#[test]
fn a_submit_only_mod_s_immediately_ask_is_kept_under_a_person() {
    if test_road() != "mod" {
        return;
    }
    let Some(h) = Harness::boot_with_env(
        "crown_submit_only",
        Some(RAW_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    // Every mod on this board speaks `submit` and not `fill`.
    std::fs::write(h.dir.join("mod-speaks"), "ping,submit,answer").unwrap();
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_submit_only");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let copies =
        |probe: &str| std::fs::read_to_string(&got).unwrap_or_default().matches(probe).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-958 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    let sid = ws.simple().to_string()[..16].to_string();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.expect("the crown's ask");

    let client = person_attaches(&tmux_sock, &sid);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-9581 kept".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Immediately,
            expect_secs: None,
        },
    ) {
        Response::Err { message } => {
            assert!(message.contains("a person has its pane open"), "{message}");
            assert!(message.contains("stay queued"), "{message}");
        }
        other => panic!("ask_agent under a person's hands: {other:?}"),
    }
    assert_eq!(asked(&mut c).status, "queued");
    wait_until(std::time::Duration::from_secs(5), "the kept line in the feed", || {
        std::fs::read_to_string(&feed_path).unwrap_or_default().lines().any(|l| {
            l.contains("\"cmd\":\"queued_ask_kept\"")
                && l.contains("\"outcome\":\"a_person_attached\"")
        })
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(copies("mesimon-probe-9581"), 0, "nothing typed under their hands");
    let frames =
        std::fs::read_to_string(h.dir.join(format!("mod-{ws}.ndjson"))).unwrap_or_default();
    assert!(!frames.contains("mesimon-probe-9581"), "nor framed: {frames}");

    // The person gone, W's idle: the drain sends it once, by the submit.
    person_leaves(&tmux_sock, &sid, client);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the kept words", || {
        copies("mesimon-probe-9581 kept") == 1
    });
    let status = asked(&mut c).status;
    assert!(matches!(status.as_str(), "sent" | "handed"), "out with the agent: {status}");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(copies("mesimon-probe-9581 kept"), 1, "one delivery");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-92 on the train");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    // The turn's end asks the train for a look at once (T-678), so its ask
    // can land a tick before the crown's hold reaches the feed.
    wait_until(std::time::Duration::from_secs(5), "the hold in the feed", || {
        feed_count("\"crown_wake_deferred\"") == 2
    });
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
    // Held and then given up, or — when the train's own look, asked at the
    // turn's end (T-678), refused the merge before the crown's look landed
    // — never held: the crown hears the same line either way.
    assert!((2..=3).contains(&feed_count("\"crown_wake_deferred\"")));
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-96 notified");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
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
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("the worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    let sw = c.spawn_claude(w);
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
                expect_secs: None,
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
/// wakes the crown with `ended its turn after your ask went in`.
#[test]
fn the_crown_answers_a_question_where_the_person_lets_it() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        printf 'Which auth provider?\\n\\342\\235\\257 1. Okta\\n  2. Auth0\\n  \
                        3. Type something.\\nEnter to select \\302\\267 Esc to cancel\\n'\n\
                        cat | while IFS= read -r line; do \
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

    let a = c.create_ticket("coordinate");
    let p = c.create_ticket("the person's own");
    let w = c.create_ticket("mesimon-probe-569 worker");
    let (ka, kp, kw) = (key_of(&mut c, a), key_of(&mut c, p), key_of(&mut c, w));
    let sa = c.spawn_claude(a);
    let sp = c.spawn_claude(p);
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
                expect_secs: None,
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

    // ---- supervised (autonomous by default): no wake, the refusal names the row
    assert!(c.board().crown_mode.answers(), "autonomous by default");
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_a1"));
    c.await_state(ws, "asking", asking);
    settle();
    assert!(!landed("asks a question"), "supervised, the person is the one to wake");
    let why = refused(&mut c, &kw, "toolu_a1", Some(0), None);
    assert!(why.contains("Settings → Agents → Crown mode"), "{why}");
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &question("toolu_a1"));
    c.await_state(ws, "running", |s| *s == SessionState::Running);

    // ---- autonomous: an agent may not switch it, a person does, and it is kept
    match c.send(
        Principal::Agent { session: sa },
        Command::SetCrownMode { mode: CrownMode::Autonomous },
    ) {
        Response::Err { .. } => {}
        other => panic!("an agent making it autonomous: {other:?}"),
    }
    assert!(!c.board().crown_mode.answers());
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_mode = \"autonomous\""), "{file}");
    assert!(!file.contains("crown_answers"), "autonomous leaves no legacy key: {file}");

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
    assert_eq!(said_on(&mut c, w).as_deref(), Some("Okta"), "and the answer under its title");
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
        landed(&format!("{kw} \"mesimon-probe-569 worker\" ended its turn after your ask went in"))
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

    // ---- supervised again: the crown's answer is refused once more ---------
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_a3"));
    c.await_state(ws, "asking", asking);
    let why = refused(&mut c, &kw, "toolu_a3", Some(0), None);
    assert!(why.contains("Crown mode is supervised"), "{why}");
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
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-571 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
                expect_secs: None,
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

/// The crown's answer types nothing under a person's hands (T-958). A
/// person has the worker's pane open, on its one-question dialog, and the
/// crown answers it in words: on the hook road no key goes in, the receipt
/// is `unknown { a_person_attached }`, the feed line says so, and the dialog
/// stands as it was; on the mod road the answer is one frame, no key at
/// all, and goes. With the person gone, the same answer walks its keys and
/// is `answered`.
#[test]
fn the_crown_s_answer_types_nothing_under_a_person_s_hands() {
    const STUB: &str = include_str!("common/fake_claude_dialog.py");
    let Some(h) = Harness::boot_bare(
        "crown_answer_held",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_answer_held");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-958 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    let sid = ws.simple().to_string()[..16].to_string();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);

    let input = json!({ "questions": [
        { "question": "Which color?", "header": "Color", "multiSelect": false,
          "options": [{ "label": "Blue", "description": "" },
                      { "label": "Green", "description": "" }] }
    ]});
    std::fs::write(h.dir.join(format!("dialog-{kw}.json")), input.to_string()).unwrap();
    let frame = json!({
        "tool_name": "AskUserQuestion", "tool_use_id": "toolu_c1", "tool_input": input
    })
    .to_string();
    let pane = |c: &mut TestClient| match c.request(Command::PaneTail { session: ws, lines: 60 }) {
        Response::PaneTail { lines, .. } => lines.join("\n"),
        _ => String::new(),
    };
    wait_until(std::time::Duration::from_secs(10), "the question drawn", || {
        pane(&mut c).contains("Which color?")
    });
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &frame);
    c.await_state(ws, "asking", |s| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    });
    let keys = h.dir.join(format!("keys-{kw}"));
    let typed = || std::fs::read_to_string(&keys).unwrap_or_default();
    let answered = h.dir.join(format!("answered-{kw}.json"));
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let call = |c: &mut TestClient| {
        let v = read(c, sa, &kw).unwrap();
        let cmd = Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: v.seen,
            request: "toolu_c1".into(),
            index: None,
            text: Some("Mauve please".into()),
            answers: None,
        };
        let sock = h.paths.orch_sock();
        std::thread::spawn(move || {
            TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
        })
    };

    // ---- a person has the pane open --------------------------------------------
    let client = person_attaches(&tmux_sock, &sid);
    let before = typed();
    let receipt = call(&mut c);
    if test_road() == "mod" {
        // One frame down the mod, no key: nothing lands under their hands.
        wait_until(std::time::Duration::from_secs(10), "the mod's answer", || answered.exists());
        match receipt.join().unwrap() {
            Response::AgentAnswered { outcome, reason, .. } => {
                assert_eq!((outcome.as_str(), reason), ("answered", None));
            }
            other => panic!("answer_agent: {other:?}"),
        }
        assert_eq!(typed(), before, "no key reaches the pane");
        person_leaves(&tmux_sock, &sid, client);
        return;
    }
    match receipt.join().unwrap() {
        Response::AgentAnswered { outcome, reason, .. } => {
            assert_eq!(outcome, "unknown");
            assert_eq!(reason.as_deref(), Some("a_person_attached"));
        }
        other => panic!("answer_agent: {other:?}"),
    }
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(typed(), before, "no key under their hands");
    assert!(!answered.exists(), "the dialog was not answered");
    assert!(pane(&mut c).contains("Type something."), "the field stands as it was");
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(
        feed.lines().any(|l| l.contains("\"cmd\":\"answer_agent\"")
            && l.contains("\"outcome\":\"unknown\"")
            && l.contains("\"reason\":\"a_person_attached\"")),
        "{feed}"
    );

    // ---- the person gone: the same answer walks its keys -----------------------
    person_leaves(&tmux_sock, &sid, client);
    let receipt = call(&mut c);
    wait_until(std::time::Duration::from_secs(30), "the dialog answered", || answered.exists());
    let took: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&answered).unwrap()).unwrap();
    assert_eq!(took, json!({ "Which color?": "Mauve please" }));
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &frame);
    match receipt.join().unwrap() {
        Response::AgentAnswered { outcome, reason, answer, .. } => {
            assert_eq!(
                (outcome.as_str(), reason, answer.as_str()),
                ("answered", None, "Mauve please")
            );
        }
        other => panic!("answer_agent: {other:?}"),
    }
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
    // T-991: a design plan, its frame and its last paragraph past the head.
    let plan = format!(
        "1. Read the code\n2. Write the code\n\n{}\n┌──────┐\n│ rail │\n└──────┘\n\n\
         The last paragraph names mesimon-probe-991-tail.\n",
        "- a step of the design, one line of it\n".repeat(16)
    );
    let plan = plan.as_str();
    assert!(plan.find('┌').unwrap() > 512, "the frame past the head");
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
        json!({ "tool_name": "ExitPlanMode", "tool_use_id": request, "tool_input": { "plan": plan } })
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
        std::fs::write(dialog, json!({ "plan": plan }).to_string()).unwrap();
        wait_until(std::time::Duration::from_secs(10), "the plan drawn", || {
            pane(c, sid).contains("Would you like to proceed?")
        });
        hook_send(&hook_sock, &sid.to_string(), "PreToolUse", &frame(request));
        c.await_state(sid, "on its plan", at_plan);
    };

    let a = c.create_ticket("coordinate");
    let p = c.create_ticket("the person's own");
    let w = c.create_ticket("mesimon-probe-582 worker");
    let (ka, kp, kw) = (key_of(&mut c, a), key_of(&mut c, p), key_of(&mut c, w));
    let sa = c.spawn_claude(a);
    let sp = c.spawn_claude(p);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    for s in [sa, sp] {
        start(&mut c, s);
        stop(&mut c, s);
    }

    // ---- on by default: no person switched anything ----------------------
    assert!(c.board().crown_mode.answers(), "a fresh board lets the crown answer and accept");
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
                expect_secs: None,
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
    let notes_before = read(&mut c, sa, &kw).unwrap().notes.len();
    assert_eq!(notes_before, 0, "a title-only ticket");
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
    // T-991: the head, marked cut, and the plan's size; with `plan`, the
    // whole of it, its lines and its frame kept. No note is written for it.
    let head = needs.plan.as_deref().expect("the head");
    let head = head.strip_suffix('…').expect("marked cut");
    assert!(head.len() <= 512 && plan.starts_with(head), "{head}");
    assert!(!head.contains('┌') && !head.contains("probe-991-tail"), "the tail is not in it");
    assert_eq!(needs.plan_bytes, Some(plan.len() as u64));
    let whole = match c.send(
        Principal::Agent { session: sa },
        Command::AgentReadTicket { key: kw.clone(), plan: true },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you.expect("the plan"),
        other => panic!("get_ticket with plan: {other:?}"),
    };
    assert_eq!(whole.plan.as_deref(), Some(plan), "the markdown whole, the frame kept");
    assert_eq!(whole.plan_bytes, None, "nothing cut");
    assert_eq!(whole.request.as_deref(), Some("toolu_w1"));
    assert_eq!(read(&mut c, sa, &kw).unwrap().notes.len(), notes_before, "no note for a plan");

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
        expect_secs: None,
    };
    match c.send(Principal::Agent { session: sa }, ask) {
        Response::Err { message } => assert!(
            message.contains("stopped on a plan") && message.contains("accept_plan accepts it"),
            "{message}"
        ),
        other => panic!("ask_agent on a plan: {other:?}"),
    }
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let why = refused(&mut c, &kw, "toolu_w1");
    assert!(why.contains("Crown mode is supervised") && why.contains("raise_hand"), "{why}");
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
        landed(&format!("{kw} \"mesimon-probe-582 worker\" ended its turn after your ask went in"))
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

/// A crown (A) and the worker it started (W) on a board whose panes run the
/// stand-in for Claude Code's dialog (`fake_claude_dialog.py`), W stopped on
/// nothing yet; A is running, so a wake waits for its idle. For T-943's plan
/// sent back.
struct PlanBed {
    h: Harness,
    c: TestClient,
    w: ulid::Ulid,
    ka: String,
    kw: String,
    sa: uuid::Uuid,
    ws: uuid::Uuid,
}

const BED_PLAN: &str = "1. Read the code\n2. Write the code";

impl PlanBed {
    fn boot(name: &str) -> Option<Self> {
        Self::boot_with(name, &[])
    }

    /// `boot` with more of the daemon's environment: a road forced for
    /// every pane, say.
    fn boot_with(name: &str, extra: &[(&str, &str)]) -> Option<Self> {
        const STUB: &str = include_str!("common/fake_claude_dialog.py");
        let mut env = vec![("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")];
        env.extend_from_slice(extra);
        let h = Harness::boot_bare(name, Some(STUB), &env)?;
        init_repo(&h.repo, "a.txt", "hello\n");
        let mut c = h.client(name);
        let a = c.create_ticket("coordinate");
        let w = c.create_ticket("mesimon-probe-943 worker");
        let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
        let sa = c.spawn_claude(a);
        assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut bed = PlanBed { h, c, w, ka, kw, sa, ws: uuid::Uuid::nil() };
        bed.start(sa);
        bed.stop(sa);
        let v = read(&mut bed.c, sa, &bed.kw).unwrap();
        assert!(matches!(
            bed.c.send(
                Principal::Agent { session: sa },
                Command::AgentStartTicket {
                    key: bed.kw.clone(),
                    seen: v.seen,
                    plan: true,
                    tier: None,
                    workspace: Some("worktree".into()),
                    expect_secs: None,
                },
            ),
            Response::AgentStarted { .. }
        ));
        wait_until(std::time::Duration::from_secs(15), "W's start to land", || {
            bed.c.board().live_agent(w).is_some()
        });
        bed.ws = bed.c.board().live_agent(w).expect("W holds a seat").id;
        std::thread::sleep(std::time::Duration::from_millis(500));
        bed.start(bed.ws);
        bed.start(sa);
        Some(bed)
    }

    fn start(&mut self, sid: uuid::Uuid) {
        let sock = self.h.paths.hook_sock();
        hook_send(&sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        self.c.await_state(sid, "running", |s| *s == SessionState::Running);
    }

    fn stop(&mut self, sid: uuid::Uuid) {
        let sock = self.h.paths.hook_sock();
        hook_send(&sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        self.c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    }

    fn frame(request: &str) -> String {
        json!({ "tool_name": "ExitPlanMode", "tool_use_id": request, "tool_input": { "plan": BED_PLAN } })
            .to_string()
    }

    /// W's hook frame `event` for the tool call `request` of `tool`.
    fn hook(&self, event: &str, tool: &str, request: &str) {
        let body = match tool {
            "ExitPlanMode" => Self::frame(request),
            _ => json!({ "tool_name": tool, "tool_use_id": request, "tool_input": {} }).to_string(),
        };
        hook_send(&self.h.paths.hook_sock(), &self.ws.to_string(), event, &body);
    }

    fn pane(&mut self) -> String {
        match self.c.request(Command::PaneTail { session: self.ws, lines: 60 }) {
            Response::PaneTail { lines, .. } => lines.join("\n"),
            _ => String::new(),
        }
    }

    /// W's plan drawn in its pane, then the hook that stops it on it.
    fn show_plan(&mut self, request: &str) {
        self.show_plan_in(None, request);
    }

    /// `show_plan`, with the call and its end written to `transcript` by
    /// the stand-in as Claude Code writes them (T-957).
    fn show_plan_in(&mut self, transcript: Option<&std::path::Path>, request: &str) {
        let dialog = self.h.dir.join(format!("dialog-{}.json", self.kw));
        let mut input = json!({ "plan": BED_PLAN });
        if let Some(transcript) = transcript {
            input["transcript"] = json!(transcript);
            input["tool_use_id"] = json!(request);
        }
        std::fs::write(dialog, input.to_string()).unwrap();
        wait_until(std::time::Duration::from_secs(10), "the plan drawn", || {
            self.pane().contains("Would you like to proceed?")
        });
        self.hook("PreToolUse", "ExitPlanMode", request);
        let at_plan = |s: &SessionState| {
            *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan }
        };
        self.c.await_state(self.ws, "on its plan", at_plan);
    }

    /// W's transcript (T-957), named by the engine's `SessionStart` and
    /// written by the stand-in; W is running again after it.
    fn transcribe(&mut self) -> std::path::PathBuf {
        let transcript = self.h.dir.join(format!("transcript-{}.jsonl", self.kw));
        let prompt = json!({ "uuid": "p", "type": "user", "message": { "content": "brief" } });
        std::fs::write(&transcript, format!("{prompt}\n")).unwrap();
        let body = json!({ "session_id": "x", "transcript_path": transcript, "cwd": "/tmp" });
        let sock = self.h.paths.hook_sock();
        hook_send_with(
            &sock,
            &self.ws.to_string(),
            "SessionStart",
            Some("startup"),
            &body.to_string(),
        );
        self.c.await_state(self.ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
        self.start(self.ws);
        transcript
    }

    /// The dialog gone from W's pane, the way a refusal leaves it, and the
    /// agent's next call clearing its plan.
    fn clear(&mut self, next: &str) {
        let _ = std::fs::remove_file(self.h.dir.join(format!("dialog-{}.json", self.kw)));
        wait_until(std::time::Duration::from_secs(10), "the dialog gone", || {
            !self.pane().contains("Would you like to proceed?")
        });
        self.hook("PreToolUse", "Read", next);
        self.c.await_state(self.ws, "running", |s| *s == SessionState::Running);
    }

    fn keys(&self) -> String {
        std::fs::read_to_string(self.h.dir.join(format!("keys-{}", self.kw))).unwrap_or_default()
    }

    fn typed(&self) -> String {
        std::fs::read_to_string(self.h.dir.join("got.txt")).unwrap_or_default()
    }

    fn feed(&self) -> String {
        std::fs::read_to_string(self.h.paths.state_dir.join("activity.jsonl")).unwrap_or_default()
    }

    fn changed(&self) -> std::path::PathBuf {
        self.h.dir.join(format!("changed-{}.json", self.kw))
    }

    /// A person's keys in W's pane, through tmux.
    fn person_keys(&self, keys: &[&str]) {
        let sid = &self.ws.simple().to_string()[..16];
        let ok = tmux(&self.h.paths.tmux_sock())
            .args(["send-keys", "-t", sid])
            .args(keys)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "send-keys {keys:?}");
    }

    fn revise_cmd(&mut self, key: &str, request: &str, text: &str) -> Command {
        let v = read(&mut self.c, self.sa, key).unwrap();
        Command::AgentRevisePlan {
            key: key.into(),
            seen: v.seen,
            request: request.into(),
            text: text.into(),
        }
    }

    fn refused(&mut self, key: &str, request: &str, text: &str) -> String {
        let cmd = self.revise_cmd(key, request, text);
        match self.c.send(Principal::Agent { session: self.sa }, cmd) {
            Response::Err { message } => message,
            other => panic!("accept_plan with change {key} {request}: {other:?}"),
        }
    }

    /// The call on a connection of its own: the receipt waits for the walk,
    /// as long as the shim waits for it (75 s; the window alone is 30 s).
    fn call(&mut self, request: &str, text: &str) -> std::thread::JoinHandle<Response> {
        let kw = self.kw.clone();
        let cmd = self.revise_cmd(&kw, request, text);
        let (sock, sa) = (self.h.paths.orch_sock(), self.sa);
        std::thread::spawn(move || {
            let mut client = TestClient::connect(&sock);
            client.write.set_read_timeout(Some(std::time::Duration::from_secs(75))).unwrap();
            client.send(Principal::Agent { session: sa }, cmd)
        })
    }
}

/// The receipt of a plan sent back: its outcome and reason.
fn revised(receipt: std::thread::JoinHandle<Response>) -> (String, Option<String>) {
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { outcome, reason, .. } => (outcome, reason),
        other => panic!("accept_plan with change: {other:?}"),
    }
}

/// The crown sends a plan back with words (T-943), `accept_plan`'s `change`
/// on the wire as `agent_revise_plan`. Refusals in words, typing nothing:
/// a dialog that changed, blank, two lines, past 1000 bytes, characters a
/// dialog cannot take, supervised mode. Then the walk: two arrows to the
/// `Tell Claude what to change` row, one paste of the words exactly, one
/// Enter, which the stand-in records with the row it was on; the receipt
/// waits for the dialog to leave (the agent's next tool call) and says
/// `input_sent`, never more; `♛ sent plan back`, the card's line and the feed
/// line carry the words; and the agent's next plan is a new request that
/// `get_ticket` reads as `needs_you: plan` and that wakes the crown again.
#[test]
fn the_crown_sends_a_plan_back_with_words() {
    let Some(mut b) = PlanBed::boot("crown_revise") else { return };
    let (ka, kw, ws, w) = (b.ka.clone(), b.kw.clone(), b.ws, b.w);
    b.show_plan("toolu_w1");
    let wake = format!("{kw} \"mesimon-probe-943 worker\" stops on a plan");
    b.stop(b.sa);
    wait_until(std::time::Duration::from_secs(10), "the plan's wake on the crown", || {
        b.typed().contains(&wake)
    });
    b.start(b.sa);

    // ---- refusals in words, none typing a key or writing a feed line --------
    let before = b.keys();
    let why = b.refused(&kw, "toolu_old", "Use echo.");
    assert!(why.contains("plan changed; read get_ticket again"), "{why}");
    for (text, says) in [
        ("", "change is blank"),
        ("  ", "change is blank"),
        ("first\nsecond", "change is one line"),
        ("red \u{1b}[31m", "plain words only"),
    ] {
        let why = b.refused(&kw, "toolu_w1", text);
        assert!(why.contains(says), "{text:?}: {why}");
    }
    let why = b.refused(&kw, "toolu_w1", &"a".repeat(1001));
    assert!(why.contains("1001 bytes") && why.contains("at most 1000"), "{why}");
    let why = b.refused(&ka, "toolu_w1", "Use echo.");
    assert!(why.contains("own ticket"), "{why}");
    assert!(matches!(
        b.c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let why = b.refused(&kw, "toolu_w1", "Use echo.");
    assert!(why.contains("Crown mode is supervised"), "{why}");
    assert!(matches!(
        b.c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    assert_eq!(b.keys(), before, "a refusal types nothing");
    assert!(!b.feed().contains("\"accept_plan\""), "a refusal writes nothing");

    // ---- the walk: two arrows, the paste, the Enter on the feedback row -----
    let words = "Use echo in Bash instead of the Write tool.";
    let receipt = b.call("toolu_w1", words);
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back in W's pane", || {
        changed.exists()
    });
    let took: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&changed).unwrap()).unwrap();
    assert_eq!(took, json!({ "row": "Tell Claude what to change", "text": words }));
    let walked = b.keys()[before.len()..].to_string();
    assert_eq!(walked.matches("\\x1b[200~").count(), 1, "one paste: {walked}");
    assert!(walked.contains(&format!("\\x1b[200~{words}\\x1b[201~")), "the words: {walked}");
    assert_eq!(walked.matches("\\r").count(), 1, "one Enter: {walked}");
    assert_eq!(walked.matches("[B").count(), 2, "two arrows down: {walked}");
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert!(!receipt.is_finished(), "the receipt waits for the dialog to leave");
    // The replanning agent's next call is the refusal road's edge (T-447).
    b.hook("PreToolUse", "Edit", "toolu_edit");
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    assert!(
        touches(&mut b.c).iter().any(|t| t.ticket == w && t.action == "sent plan back"),
        "♛ sent plan back on the card"
    );
    assert!(
        b.feed().lines().any(|l| l.contains("\"cmd\":\"accept_plan\"")
            && l.contains("\"actor\":\"agent\"")
            && l.contains("\"outcome\":\"input_sent\"")
            && l.contains(&format!("\"answer\":\"{words}\""))),
        "{}",
        b.feed()
    );
    assert!(!b.feed().contains("\"outcome\":\"changed\""), "nothing claims the words were read");
    let detail = b.c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.detail.clone());
    assert_eq!(detail, Some(format!("plan sent back by {ka}: {words}")));

    // ---- the agent's next plan: a new request, needs-you, the crown woken ---
    b.hook("PostToolUse", "Edit", "toolu_edit");
    b.stop(b.sa);
    b.show_plan("toolu_w2");
    wait_until(std::time::Duration::from_secs(10), "the next plan's wake on the crown", || {
        b.typed().matches(&wake).count() == 2
    });
    b.start(b.sa);
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the next plan");
    assert_eq!(needs.reason, "plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w2"));

    // ---- replanned at once, inside the attention settle: no state edge -----
    // The rig's worker (2.1.310) called `ExitPlanMode` again under a second
    // after the Enter, so its record never left `RequiresAction{Plan}`; the
    // new request wakes the crown all the same.
    std::fs::remove_file(&changed).unwrap();
    let receipt = b.call("toolu_w2", "Name the file b.txt instead.");
    wait_until(std::time::Duration::from_secs(10), "the second plan sent back", || {
        changed.exists()
    });
    b.stop(b.sa);
    std::fs::write(
        b.h.dir.join(format!("dialog-{kw}.json")),
        json!({ "plan": BED_PLAN }).to_string(),
    )
    .unwrap();
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w3");
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    wait_until(std::time::Duration::from_secs(10), "the replanned plan's wake", || {
        b.typed().matches(&wake).count() == 3
    });
    let at_plan = SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan };
    assert_eq!(
        b.c.board().sessions.iter().find(|s| s.id == ws).map(|s| s.state.clone()),
        Some(at_plan.clone())
    );

    // ---- the same dialog said again wakes nobody again ----------------------
    // The crown is idle, so a second wake would land at once. The feed's
    // hook line says the daemon received a repeat (its handler entered);
    // the board reads after it are served by the same writer, so they come
    // after its handling.
    let hook_sock = b.h.paths.hook_sock();
    let mod_road = test_road() == "mod";
    let ingested = |b: &PlanBed, session: uuid::Uuid, event: &str| {
        b.feed()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"hook\"")
                    && l.contains(&format!("\"session\":\"{session}\""))
                    && l.contains(&format!("\"event\":\"{event}\""))
                    && l.contains("\"road\":\"mod\"") == mod_road
            })
            .count()
    };
    let (pre, perm) = (ingested(&b, ws, "PreToolUse"), ingested(&b, ws, "PermissionRequest"));
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w3");
    b.hook("PermissionRequest", "ExitPlanMode", "toolu_w3");
    let unkeyed = json!({ "tool_name": "ExitPlanMode", "tool_input": { "plan": BED_PLAN } });
    hook_send(&hook_sock, &ws.to_string(), "PermissionRequest", &unkeyed.to_string());
    wait_until(std::time::Duration::from_secs(10), "the repeated frames ingested", || {
        ingested(&b, ws, "PreToolUse") == pre + 1
            && ingested(&b, ws, "PermissionRequest") == perm + 2
    });

    // ---- a person's agent replanning in place wakes nobody through it -------
    let p = b.c.create_ticket("the person's own");
    let kp = key_of(&mut b.c, p);
    let sp = b.c.spawn_claude(p);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let send = |event: &str, request: &str| {
        let frame = PlanBed::frame(request);
        hook_send(&hook_sock, &sp.to_string(), event, &frame);
    };
    hook_send(&hook_sock, &sp.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    b.c.await_state(sp, "running", |s| *s == SessionState::Running);
    send("PreToolUse", "toolu_p1");
    b.c.await_state(sp, "on its plan", |s| *s == at_plan);
    send("PreToolUse", "toolu_p2");
    // The replan judged: the dialog the board holds for P is the new one.
    wait_until(std::time::Duration::from_secs(10), "P's replan read", || {
        read(&mut b.c, b.sa, &kp)
            .ok()
            .and_then(|v| v.needs_you)
            .is_some_and(|n| n.request.as_deref() == Some("toolu_p2"))
    });
    assert_eq!(b.typed().matches(&wake).count(), 3, "a repeat of the same plan wakes once");
    let owed = |b: &PlanBed, worker: ulid::Ulid| {
        b.feed()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{worker}\""))
                    && l.contains("\"cause\":\"planned\"")
            })
            .count()
    };
    assert_eq!(owed(&b, w), 3, "three plans owed three wakes: {}", b.feed());
    assert!(
        !b.typed().contains(&format!("{kp} \"the person's own\" stops on a plan")),
        "a person's agent's plans wake nobody: {}",
        b.typed()
    );
    assert_eq!(owed(&b, p), 0, "no wake owed for the person's ticket");

    b.start(b.sa);
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the replanned plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w3"));
}

/// A new plan while the wake for the one before it is still owed (T-943's
/// composition, as the rig's worker did it): the crown sends a plan back
/// from inside its own turn, the worker replans at once, and the new plan's
/// obligation waits for the crown's idle with the earlier one's. The two
/// fold into one notification, which names the worker and that it stops on
/// a plan, not a request; the crown then reads the current request with
/// `get_ticket`, an accept of the old one is refused, and its accept of the
/// current one goes through.
#[test]
fn a_new_plan_while_its_wake_is_owed_is_one_wake_then_the_current_request() {
    let Some(mut b) = PlanBed::boot("crown_revise_owed") else { return };
    let (kw, ws, w) = (b.kw.clone(), b.ws, b.w);
    let wake = format!("{kw} \"mesimon-probe-943 worker\" stops on a plan");
    let owed = |b: &PlanBed| {
        b.feed()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"planned\"")
            })
            .count()
    };
    let sent = |b: &PlanBed| b.feed().matches("\"cmd\":\"crown_wake_sent\"").count();
    // The crown is in its turn (the bed leaves it running): W's plan is owed.
    b.show_plan("toolu_w1");
    wait_until(std::time::Duration::from_secs(10), "the first plan's wake owed", || owed(&b) == 1);
    assert_eq!(b.typed().matches(&wake).count(), 0, "owed, not sent: the crown is busy");
    // Still in its turn, the crown sends the plan back; W replans at once.
    let receipt = b.call("toolu_w1", "Use echo in Bash instead of the Write tool.");
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back", || changed.exists());
    std::fs::write(
        b.h.dir.join(format!("dialog-{kw}.json")),
        json!({ "plan": BED_PLAN }).to_string(),
    )
    .unwrap();
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w2");
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    wait_until(std::time::Duration::from_secs(10), "the new plan's wake owed", || owed(&b) == 2);
    assert_eq!(b.typed().matches(&wake).count(), 0, "still owed: the crown's turn runs");
    // The crown's turn ends: one notification for the worker. The feed is
    // flushed on the tick, so the send is read once it shows. Then, with the
    // wake acknowledged by the crown's next prompt, the writer's own list of
    // what is pending holds no wake for the crown: owed or still entering, a
    // second obligation would be there even while the crown is busy.
    let crown_wakes = |b: &mut PlanBed| -> Vec<mesimon_core::command::Pending> {
        pending_of(&mut b.c, None)
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };
    assert!(!crown_wakes(&mut b).is_empty(), "the busy crown's wake is listed as pending");
    let before = sent(&b);
    b.stop(b.sa);
    wait_until(std::time::Duration::from_secs(10), "the wake on the crown", || {
        b.typed().matches(&wake).count() == 1 && sent(&b) > before
    });
    b.start(b.sa);
    let wakes = crown_wakes(&mut b);
    assert!(wakes.is_empty(), "the two plans' obligations fold into one: {wakes:?}");
    assert_eq!(sent(&b), before + 1);
    assert_eq!(b.typed().matches(&wake).count(), 1);
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the new plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w2"));
    // The new request is the one an accept takes; the old one is refused.
    let v = read(&mut b.c, b.sa, &kw).unwrap();
    let stale = Command::AgentAcceptPlan {
        key: kw.clone(),
        seen: v.seen.clone(),
        request: "toolu_w1".into(),
    };
    match b.c.send(Principal::Agent { session: b.sa }, stale) {
        Response::Err { message } => assert!(message.contains("plan changed"), "{message}"),
        other => panic!("accept_plan on the plan sent back: {other:?}"),
    }
    let accepted = b.h.dir.join(format!("accepted-{kw}"));
    let accept =
        Command::AgentAcceptPlan { key: kw.clone(), seen: v.seen, request: "toolu_w2".into() };
    let (sock, sa) = (b.h.paths.orch_sock(), b.sa);
    let receipt = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, accept)
    });
    wait_until(std::time::Duration::from_secs(10), "the new plan accepted", || accepted.exists());
    b.hook("PostToolUse", "ExitPlanMode", "toolu_w2");
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { outcome, .. } => assert_eq!(outcome, "accepted"),
        other => panic!("accept_plan on the new plan: {other:?}"),
    }
    b.c.await_state(ws, "running the plan", |s| *s == SessionState::Running);
}

/// T-938 on T-943: the one outstanding crown ask is the ask record alone.
/// An `ask_agent` while the worker stands on a plan is refused before any
/// record is made, and the plan sent back (`accept_plan`'s change) is a
/// dialog's answer, not an ask: neither counts as the outstanding ask, so an
/// ask at the next plan is refused only for the dialog, and once the plan
/// is accepted an ask goes out.
#[test]
fn a_refused_ask_and_a_plan_sent_back_are_not_the_outstanding_ask() {
    let Some(mut b) = PlanBed::boot("crown_revise_ask") else { return };
    let (kw, ws) = (b.kw.clone(), b.ws);
    let ask = |b: &mut PlanBed, text: &str| {
        let v = read(&mut b.c, b.sa, &kw).unwrap();
        b.c.send(
            Principal::Agent { session: b.sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Now,
                expect_secs: None,
            },
        )
    };
    let at_dialog = |r: Response| match r {
        Response::Err { message } => {
            assert!(
                message.contains("stopped on a plan") && !message.contains("is refused while"),
                "{message}"
            )
        }
        other => panic!("ask_agent at a plan: {other:?}"),
    };
    b.show_plan("toolu_w1");
    wait_until(std::time::Duration::from_secs(10), "the plan on the card", || {
        read(&mut b.c, b.sa, &kw).unwrap().needs_you.is_some()
    });
    at_dialog(ask(&mut b, "mesimon-probe-9431 at the plan"));
    assert!(read(&mut b.c, b.sa, &kw).unwrap().asked.is_none(), "a refused ask leaves no record");
    let receipt = b.call("toolu_w1", "Use echo in Bash instead of the Write tool.");
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back", || changed.exists());
    std::fs::write(
        b.h.dir.join(format!("dialog-{kw}.json")),
        json!({ "plan": BED_PLAN }).to_string(),
    )
    .unwrap();
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w2");
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    wait_until(std::time::Duration::from_secs(10), "the new plan on the card", || {
        read(&mut b.c, b.sa, &kw).unwrap().needs_you.and_then(|n| n.request).as_deref()
            == Some("toolu_w2")
    });
    at_dialog(ask(&mut b, "mesimon-probe-9432 at the new plan"));
    assert!(read(&mut b.c, b.sa, &kw).unwrap().asked.is_none(), "the plan sent back is no ask");
    let v = read(&mut b.c, b.sa, &kw).unwrap();
    let accepted = b.h.dir.join(format!("accepted-{kw}"));
    let accept =
        Command::AgentAcceptPlan { key: kw.clone(), seen: v.seen, request: "toolu_w2".into() };
    let (sock, sa) = (b.h.paths.orch_sock(), b.sa);
    let receipt = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, accept)
    });
    wait_until(std::time::Duration::from_secs(10), "the new plan accepted", || accepted.exists());
    b.hook("PostToolUse", "ExitPlanMode", "toolu_w2");
    assert!(matches!(receipt.join().unwrap(), Response::AgentPlanAccepted { .. }));
    b.c.await_state(ws, "running the plan", |s| *s == SessionState::Running);
    match ask(&mut b, "mesimon-probe-9433 after the plan") {
        Response::AgentAsked { asked: Some(a), .. } => assert_eq!(a.status, "sent", "{a:?}"),
        other => panic!("an ask after the plan: {other:?}"),
    }
}

/// A new plan while the crown's wake for the old one is still entering
/// (T-943 on T-936, Astra's specimen). Every pane is on the mod road, so the
/// crown's wake goes down its mod as a `submit` frame, told at its take
/// (T-990); the crown's mod holds its `entered`. Meanwhile the
/// crown sends the plan back and the worker replans. When the old frame
/// enters and settles, the new plan's obligation is still owed and its
/// request is the one `get_ticket` reads: the crown is woken for it again.
#[test]
fn a_new_plan_while_the_old_wake_is_entering_outlives_its_settle() {
    let Some(mut b) =
        PlanBed::boot_with("crown_revise_entering", &[("MESIMON_CLAUDE_ROAD", "mod")])
    else {
        return;
    };
    let (kw, sa) = (b.kw.clone(), b.sa);
    let wake = format!("{kw} \"mesimon-probe-943 worker\" stops on a plan");
    let crown_wakes = |b: &mut PlanBed| -> usize {
        pending_of(&mut b.c, None)
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .count()
    };
    let submits = |b: &PlanBed| -> usize {
        std::fs::read_to_string(b.h.dir.join(format!("mod-{sa}.ndjson")))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "submit" && v.to_string().contains("stops on a plan"))
            .count()
    };
    let (hold, release) = (b.h.dir.join("mod-hold-submit"), b.h.dir.join("mod-release-submit"));
    // The crown is busy: W's first plan is owed; the crown's mod will hold
    // the frame the wake goes down as.
    b.show_plan("toolu_w1");
    std::fs::write(&hold, "").unwrap();
    b.stop(sa);
    wait_until(std::time::Duration::from_secs(10), "the wake's frame held by the mod", || {
        submits(&b) == 1
    });
    wait_until(std::time::Duration::from_secs(5), "told at the take (T-990)", || {
        crown_wakes(&mut b) == 0
    });
    assert_eq!(b.typed().matches(&wake).count(), 0, "not entered yet");
    // While it is entering, the crown sends the plan back; W replans.
    let receipt = b.call("toolu_w1", "Use echo in Bash instead of the Write tool.");
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back", || changed.exists());
    std::fs::write(
        b.h.dir.join(format!("dialog-{kw}.json")),
        json!({ "plan": BED_PLAN }).to_string(),
    )
    .unwrap();
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w2");
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    // The old frame enters and settles.
    std::fs::write(&release, "").unwrap();
    wait_until(std::time::Duration::from_secs(25), "the old wake entered", || {
        b.typed().matches(&wake).count() == 1
    });
    // The new plan's obligation outlived that settle: the crown hears of it
    // again, by a frame of its own, and reads the new request.
    wait_until(std::time::Duration::from_secs(20), "the new plan's wake", || {
        b.typed().matches(&wake).count() == 2
    });
    assert_eq!(submits(&b), 2, "a frame of its own");
    let needs = read(&mut b.c, sa, &kw).unwrap().needs_you.expect("the new plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w2"));
    for name in [&hold, &release] {
        let _ = std::fs::remove_file(name);
    }
}

/// A plan sent back whose replan ends on an API error inside the receipt's
/// window (T-943 on T-936, Fable's specimen), in the order a crown meets
/// it: woken for the plan, it sends the plan back from its own turn, and
/// the worker's replan fails. The error ends the turn and drops the dialog's
/// projection, so the receipt says `input_sent` at once, not at the
/// window's end; T-936 owes the crown the failed turn's wake, which it hears
/// at its idle. The crown reads two lines that agree: the words went in and
/// the dialog left, and the worker's turn then ended on an error, with its
/// class, which `get_ticket` reads too.
#[test]
fn a_plan_sent_back_whose_replan_fails_reads_two_lines_that_agree() {
    let Some(mut b) = PlanBed::boot("crown_revise_failed") else { return };
    let (kw, ws, sa) = (b.kw.clone(), b.ws, b.sa);
    let wake = format!("{kw} \"mesimon-probe-943 worker\" stops on a plan");
    b.show_plan("toolu_w1");
    b.stop(sa);
    wait_until(std::time::Duration::from_secs(10), "the plan's wake", || b.typed().contains(&wake));
    b.start(sa);
    let receipt = b.call("toolu_w1", "Use echo in Bash instead of the Write tool.");
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back", || changed.exists());
    let body = json!({ "error": "server_error", "last_assistant_message": "API Error" });
    hook_send_with(
        &b.h.paths.hook_sock(),
        &ws.to_string(),
        "StopFailure",
        Some("server_error"),
        &body.to_string(),
    );
    b.c.await_state(ws, "failed", |s| *s == SessionState::Failed { reason: FailReason::Server });
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    b.stop(sa);
    let failed =
        format!("{kw} \"mesimon-probe-943 worker\" ended its turn on an error (server error");
    wait_until(std::time::Duration::from_secs(10), "the failed turn's wake", || {
        b.typed().contains(&failed)
    });
    let view = read(&mut b.c, sa, &kw).unwrap();
    let state = view.state.expect("W's state");
    assert_eq!((state.state.as_str(), state.failed.as_deref()), ("failed", Some("server error")));
    assert!(view.needs_you.is_none(), "no plan stands");
}

/// A plan sent back stops at any hand but the crown's (T-943), and the
/// receipt then says `unknown`, never `input_sent`: words a person typed
/// into the field first get no paste and no Enter; a person attaching
/// between the paste and the Enter gets no Enter; a person's Esc, an
/// unrelated error ending the call, and a plan replaced, each mid-walk, stop
/// it; and an Enter the dialog did not take is `unknown { dialog_stands }`
/// once the window is out, never retried.
#[test]
fn a_plan_sent_back_stops_at_any_hand_but_the_crown_s() {
    let Some(mut b) = PlanBed::boot("crown_revise_hands") else { return };
    let words = "Use echo in Bash instead of the Write tool.";
    let pastes = |b: &PlanBed| b.keys().matches("\\x1b[200~").count();
    let enters = |b: &PlanBed| b.keys().matches("\\r").count();
    let delay = b.h.dir.join(format!("paste-delay-{}", b.kw));

    // ---- a person's words already in the field ------------------------------
    b.show_plan("toolu_w1");
    b.person_keys(&["Down", "Down"]);
    b.person_keys(&["-l", "my own note"]);
    wait_until(std::time::Duration::from_secs(10), "their words drawn", || {
        b.pane().contains("my own note")
    });
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w1", words);
    assert_eq!(revised(receipt), ("unknown".to_string(), Some("field_touched".to_string())));
    assert_eq!((pastes(&b), enters(&b)), (p0, e0), "no paste, no Enter");
    assert!(!b.changed().exists());
    assert!(b.pane().contains("my own note"), "their words are left as they were");
    b.clear("toolu_r1");

    // ---- a person attaches between the paste and the Enter ------------------
    std::fs::write(&delay, "3").unwrap();
    b.show_plan("toolu_w2");
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w2", words);
    wait_until(std::time::Duration::from_secs(10), "the crown's paste", || pastes(&b) == p0 + 1);
    let sid = b.ws.simple().to_string()[..16].to_string();
    let tmux_sock = b.h.paths.tmux_sock();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &sid])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    let (outcome, reason) = revised(receipt);
    assert_eq!(outcome, "unknown");
    assert!(reason.as_deref().is_some_and(|r| r.starts_with("a_person_attached")), "{reason:?}");
    wait_until(std::time::Duration::from_secs(10), "the words drawn late", || {
        b.pane().contains(words)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(enters(&b), e0, "no Enter under their hands");
    assert!(!b.changed().exists());
    client.kill().unwrap();
    let _ = client.wait();
    b.clear("toolu_r2");

    // ---- a person's Esc mid-walk: no hook says it, the screen does ----------
    b.show_plan("toolu_w3");
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w3", words);
    wait_until(std::time::Duration::from_secs(10), "the crown's paste", || pastes(&b) == p0 + 1);
    b.person_keys(&["Escape"]);
    let (outcome, _) = revised(receipt);
    assert_eq!(outcome, "unknown");
    assert_eq!(enters(&b), e0, "no Enter after the Esc");
    b.clear("toolu_r3");

    // ---- an unrelated error ends the plan's call mid-walk -------------------
    b.show_plan("toolu_w4");
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w4", words);
    wait_until(std::time::Duration::from_secs(10), "the crown's paste", || pastes(&b) == p0 + 1);
    b.hook("PostToolUseFailure", "ExitPlanMode", "toolu_w4");
    let (outcome, _) = revised(receipt);
    assert_eq!(outcome, "unknown");
    assert_eq!(enters(&b), e0, "no Enter once the call ended");
    b.clear("toolu_r4");

    // ---- the plan replaced mid-walk -----------------------------------------
    b.show_plan("toolu_w5");
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w5", words);
    wait_until(std::time::Duration::from_secs(10), "the crown's paste", || pastes(&b) == p0 + 1);
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w6");
    let (outcome, _) = revised(receipt);
    assert_eq!(outcome, "unknown");
    assert_eq!(enters(&b), e0, "no Enter into the plan that replaced it");
    let needs = read(&mut b.c, b.sa, &b.kw.clone()).unwrap().needs_you.expect("the new plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w6"));
    b.clear("toolu_r5");

    // ---- an Enter the dialog did not take: unknown once the window is out ---
    std::fs::remove_file(&delay).unwrap();
    std::fs::write(b.h.dir.join(format!("keep-{}", b.kw)), "").unwrap();
    b.show_plan("toolu_w7");
    let (p0, e0) = (pastes(&b), enters(&b));
    let receipt = b.call("toolu_w7", words);
    wait_until(std::time::Duration::from_secs(10), "the Enter", || enters(&b) == e0 + 1);
    assert_eq!(revised(receipt), ("unknown".to_string(), Some("dialog_stands".to_string())));
    assert_eq!((pastes(&b), enters(&b)), (p0 + 1, e0 + 1), "never retried");
    assert!(!b.feed().contains("\"outcome\":\"changed\""));
    b.clear("toolu_r7");

    // ---- a screen read that fails at the window's end: no observation ------
    // The worker's tmux session is renamed after the Enter, so every capture
    // of it fails while its pane lives on: the board saw nothing leave, so
    // the receipt is `unknown` and the request and the plan stay readable,
    // and the next call on them is not refused as a changed plan.
    b.show_plan("toolu_w8");
    let e0 = enters(&b);
    let receipt = b.call("toolu_w8", words);
    wait_until(std::time::Duration::from_secs(10), "the Enter", || enters(&b) == e0 + 1);
    let sid = b.ws.simple().to_string()[..16].to_string();
    let rename = |from: &str, to: &str| {
        let ok = tmux(&b.h.paths.tmux_sock())
            .args(["rename-session", "-t", &format!("={from}"), to])
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "rename-session {from} {to}");
    };
    rename(&sid, "msmn-e2e-hidden");
    let outcome = revised(receipt);
    rename("msmn-e2e-hidden", &sid);
    assert_eq!(outcome, ("unknown".to_string(), Some("pane_unreachable".to_string())));
    let needs = read(&mut b.c, b.sa, &b.kw.clone()).unwrap().needs_you.expect("still on its plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w8"), "the same request");
    assert_eq!(needs.plan.as_deref(), Some(BED_PLAN), "the same plan");
    // The field holds the crown's words now: the next walk stops on them,
    // but it is not refused at the call as a plan that changed.
    let again = b.call("toolu_w8", words);
    assert_eq!(revised(again), ("unknown".to_string(), Some("field_touched".to_string())));
}

/// A plan sent back leaves needs-you with its dialog, and only on a screen
/// that shows it gone (T-955). Neither road sends a `PreToolUse` for an
/// ordinary tool and the plan's rejection fires no hook, so on 2.1.310's mod
/// road the window's screen read is the one departure (the board's own
/// daemon, 19:11). A sibling tool's end is not one, before the Enter or
/// after it (T-595): with the plan still drawn the request, the plan and the
/// record stay. With the composer drawn and no frame at all, the record
/// leaves `RequiresAction{Plan}`, `get_ticket` reads no `needs_you`, and
/// `ask_agent` is admitted (taken into the queue, not yet read by the agent).
#[test]
fn a_plan_sent_back_leaves_needs_you_only_on_a_screen_without_it() {
    let Some(mut b) = PlanBed::boot("crown_revise_leaves") else { return };
    let kw = b.kw.clone();
    let words = "Use echo in Bash instead of the Write tool.";
    let at_plan = SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan };
    let changed = b.changed();
    let state = |b: &mut PlanBed| {
        b.c.board().sessions.iter().find(|s| s.id == b.ws).map(|s| s.state.clone())
    };
    let ended = |b: &PlanBed| {
        b.feed()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"hook\"")
                    && l.contains(&format!("\"session\":\"{}\"", b.ws))
                    && l.contains("\"event\":\"PostToolUse\"")
            })
            .count()
    };

    // ---- a sibling's end before the Enter and after it: the plan stands -----
    let keep = b.h.dir.join(format!("keep-{kw}"));
    std::fs::write(&keep, "").unwrap();
    b.show_plan("toolu_w1");
    b.hook("PostToolUse", "Bash", "toolu_s0");
    let receipt = b.call("toolu_w1", words);
    wait_until(std::time::Duration::from_secs(10), "the Enter on the words", || changed.exists());
    let seen = ended(&b);
    b.hook("PostToolUse", "Bash", "toolu_s1");
    wait_until(std::time::Duration::from_secs(10), "the sibling's end ingested", || {
        ended(&b) > seen
    });
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the plan stands");
    assert_eq!(needs.request.as_deref(), Some("toolu_w1"));
    assert_eq!(needs.plan.as_deref(), Some(BED_PLAN));
    assert_eq!(state(&mut b), Some(at_plan.clone()));
    assert!(!receipt.is_finished(), "no departure receipt on a sibling's end");
    assert_eq!(revised(receipt), ("unknown".to_string(), Some("dialog_stands".to_string())));
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the plan still stands");
    assert_eq!(needs.request.as_deref(), Some("toolu_w1"));
    assert_eq!(state(&mut b), Some(at_plan.clone()));
    // The person ends the turn and starts the next.
    std::fs::remove_file(&keep).unwrap();
    std::fs::remove_file(b.h.dir.join(format!("dialog-{kw}.json"))).unwrap();
    wait_until(std::time::Duration::from_secs(10), "the dialog gone", || {
        !b.pane().contains("Would you like to proceed?")
    });
    b.stop(b.ws);
    b.start(b.ws);

    // ---- the composer drawn and no frame at all: the plan leaves ------------
    std::fs::remove_file(&changed).unwrap();
    b.show_plan("toolu_w2");
    let receipt = b.call("toolu_w2", words);
    wait_until(std::time::Duration::from_secs(10), "the second Enter", || changed.exists());
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    b.c.await_state(b.ws, "running once the screen shows the composer", |s| {
        *s == SessionState::Running
    });
    assert!(read(&mut b.c, b.sa, &kw).unwrap().needs_you.is_none(), "no plan stands");
    b.hook("PostToolUse", "Bash", "toolu_s2");

    // ---- the crown's words are admitted, not refused as at a plan -----------
    let v = read(&mut b.c, b.sa, &kw).unwrap();
    match b.c.send(
        Principal::Agent { session: b.sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-955 one more line".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { .. } => {}
        other => panic!("ask_agent after a plan sent back: {other:?}"),
    }
}

/// A plan a person sends back in the pane leaves needs-you (T-957), with no
/// crown walk and only what Claude Code sends: the person's words on the
/// feedback row, or an Esc, fire no hook (T-447), and neither the hook set
/// nor the classic mod relays the next ordinary tool's `PreToolUse`. The
/// stand-in writes the transcript as 2.1.310 does, the plan's own call and
/// then its result; that result is the departure. With words the record
/// runs again; with an Esc, whose interrupt follows the result, it rests
/// interrupted, under the flap pin too. Either way `get_ticket` reads no
/// `needs_you`: `ask_agent` is admitted (taken into the queue) after the
/// words, and delivered into the composer once the Esc leaves the seat idle.
#[test]
fn a_plan_a_person_sends_back_in_the_pane_leaves_needs_you() {
    let Some(mut b) = PlanBed::boot("crown_person_refuses") else { return };
    let (kw, ws) = (b.kw.clone(), b.ws);
    let transcript = b.transcribe();
    let ask = |b: &mut PlanBed, text: &str| {
        let v = read(&mut b.c, b.sa, &kw).unwrap();
        assert!(v.needs_you.is_none(), "no plan stands: {:?}", v.needs_you);
        match b.c.send(
            Principal::Agent { session: b.sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { .. } => {}
            other => panic!("ask_agent after a plan the person sent back: {other:?}"),
        }
    };
    let departures = |b: &PlanBed| {
        b.feed()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| {
                v["kind"] == "state_decision"
                    && v["session"] == ws.to_string()
                    && v["source"] == "dialog_result"
            })
            .count()
    };

    // ---- the person's words on `Tell Claude what to change` ----------------
    b.show_plan_in(Some(&transcript), "toolu_p1");
    b.person_keys(&["Down", "Down"]);
    b.person_keys(&["Name the file b.txt instead."]);
    b.person_keys(&["Enter"]);
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the plan sent back in W's pane", || {
        changed.exists()
    });
    b.c.await_state(ws, "running once the plan's result is written", |s| {
        *s == SessionState::Running
    });
    assert_eq!(departures(&b), 1, "the transcript's result was the departure: {}", b.feed());
    ask(&mut b, "mesimon-probe-957 after words");

    // ---- the person's Esc on the next plan ---------------------------------
    // The interrupt record right after the plan's result is the turn's end:
    // one High end, interrupted, which the flap pin (armed by this many
    // changes in 20 s) lets through, as it drops the Low interrupt hint.
    b.show_plan_in(Some(&transcript), "toolu_p2");
    b.person_keys(&["Escape"]);
    b.c.await_state(ws, "interrupted once the plan's result is written", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted }
    });
    assert_eq!(departures(&b), 2, "{}", b.feed());
    let esc = b
        .feed()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .rfind(|v| v["kind"] == "state_decision" && v["source"] == "dialog_result")
        .expect("the Esc's end");
    let pinned = esc["decision"]["before"]["pinned_until"].as_u64();
    assert!(pinned.is_some_and(|until| until > esc["at_ms"].as_u64().unwrap()), "pinned: {esc}");
    // At rest off the plan, the seat takes the words queued after the first
    // refusal: they reach its composer.
    assert!(read(&mut b.c, b.sa, &kw).unwrap().needs_you.is_none(), "no plan stands");
    wait_until(std::time::Duration::from_secs(10), "the queued ask typed into W", || {
        b.typed().contains("mesimon-probe-957 after words")
    });
}

/// T-957 round 9: a refusal on disk before its hooks are heard. The writer
/// was busy: W, at rest with no cursor on its transcript, stops on a plan,
/// and the person sends it back (words, then on the next plan an Esc), all
/// written before the prompt's and the plan's hooks are consumed. So the
/// plan's result is stamped before the daemon's spell. The record names the
/// plan's call from its hook, and the first read finds the call and its
/// result by that identity, against no clock: the plan leaves needs-you
/// (running after words, interrupted after the Esc), and `ask_agent` is
/// admitted.
#[test]
fn a_refusal_written_before_its_hooks_still_clears_the_plan() {
    let Some(mut b) = PlanBed::boot("crown_refusal_early") else { return };
    let (kw, ws) = (b.kw.clone(), b.ws);
    let transcript = b.transcribe();
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    for (request, esc) in [("toolu_e1", false), ("toolu_e2", true)] {
        // At rest: a tail read forgets the cursor.
        b.stop(ws);
        std::thread::sleep(std::time::Duration::from_millis(2500));
        // The plan drawn and its call written, no hook yet.
        let dialog = b.h.dir.join(format!("dialog-{kw}.json"));
        let input = json!({ "plan": BED_PLAN, "transcript": transcript, "tool_use_id": request });
        std::fs::write(&dialog, input.to_string()).unwrap();
        wait_until(std::time::Duration::from_secs(10), "the plan drawn", || {
            b.pane().contains("Would you like to proceed?")
        });
        if esc {
            b.person_keys(&["Escape"]);
        } else {
            b.person_keys(&["Down", "Down"]);
            b.person_keys(&["Name the file b.txt instead."]);
            b.person_keys(&["Enter"]);
        }
        wait_until(std::time::Duration::from_secs(10), "the refusal written", || {
            std::fs::read_to_string(&transcript)
                .unwrap_or_default()
                .contains(&format!("\"tool_use_id\": \"{request}\""))
        });
        // Now the busy writer consumes the turn's hooks. The prompt's alone
        // makes W running, so the wait is for the plan's committed leave:
        // the hook holds W on it, and the read of the result ends it after
        // the settle (T-1002).
        let leaves = |b: &PlanBed| {
            b.feed()
                .lines()
                .filter(|l| {
                    l.contains("\"kind\":\"session_state\"")
                        && l.contains(&ws.to_string())
                        && l.contains("\"from\":\"requires_action\"")
                })
                .count()
        };
        let before = leaves(&b);
        hook_send(
            &b.h.paths.hook_sock(),
            &ws.to_string(),
            "UserPromptSubmit",
            r#"{"prompt":"go"}"#,
        );
        b.hook("PreToolUse", "ExitPlanMode", request);
        wait_until(std::time::Duration::from_secs(10), "the plan's leave", || leaves(&b) > before);
        let off = if esc { interrupted.clone() } else { SessionState::Running };
        b.c.await_state(ws, "off the plan its result on disk ended", |s| *s == off);
        let v = read(&mut b.c, b.sa, &kw).unwrap();
        assert!(v.needs_you.is_none(), "no plan stands (esc: {esc}): {:?}", v.needs_you);
        match b.c.send(
            Principal::Agent { session: b.sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: format!("mesimon-probe-957 after {request}"),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { .. } => {}
            other => panic!("ask_agent after a refusal read by identity: {other:?}"),
        }
        // The words go in once the seat rests; the next case starts on it.
        if !esc {
            b.stop(ws);
            wait_until(std::time::Duration::from_secs(10), "the ask typed into W", || {
                b.typed().contains(&format!("mesimon-probe-957 after {request}"))
            });
            b.start(ws);
        }
    }
}

/// A replan inside the leave a plan sent back started (T-955 with T-943's
/// re-stop). The window's screen read shows the composer, so the record's
/// machine is given the departure with its 1500 ms settle; the worker's next
/// `ExitPlanMode` comes inside it, on the frames the engine sends. The leave
/// is cancelled, never committed: the record stays on its plan through the
/// deadline, `get_ticket` reads the new request, and the crown is woken once
/// for it, by the re-stop, with no second wake owed.
#[test]
fn a_replan_inside_a_sent_back_plan_s_leave_keeps_it_and_wakes_once() {
    let Some(mut b) = PlanBed::boot("crown_revise_restop") else { return };
    let (kw, ws, w) = (b.kw.clone(), b.ws, b.w);
    let at_plan = SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan };
    let wake = format!("{kw} \"mesimon-probe-943 worker\" stops on a plan");
    let owed = |b: &PlanBed| {
        b.feed()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"planned\"")
            })
            .count()
    };
    b.stop(b.sa);
    b.show_plan("toolu_w1");
    wait_until(std::time::Duration::from_secs(10), "the plan's wake", || {
        b.typed().matches(&wake).count() == 1
    });
    b.start(b.sa);
    let receipt = b.call("toolu_w1", "Name the file b.txt instead.");
    let changed = b.changed();
    wait_until(std::time::Duration::from_secs(10), "the Enter on the words", || changed.exists());
    b.stop(b.sa);
    let before = owed(&b);
    assert_eq!(revised(receipt), ("input_sent".to_string(), None));
    let left = std::time::Instant::now();
    std::fs::write(
        b.h.dir.join(format!("dialog-{kw}.json")),
        json!({ "plan": BED_PLAN }).to_string(),
    )
    .unwrap();
    b.hook("PreToolUse", "ExitPlanMode", "toolu_w2");
    assert!(left.elapsed() < std::time::Duration::from_millis(1500), "inside the settle");
    wait_until(std::time::Duration::from_secs(10), "the replan's wake", || {
        b.typed().matches(&wake).count() == 2
    });
    // The departure was a pending leave, and the replan's frame cancelled it.
    // The wake may be typed before the replan's decision line is written.
    let decisions = |b: &PlanBed| -> Vec<serde_json::Value> {
        b.feed()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "state_decision" && v["session"] == ws.to_string())
            .collect()
    };
    let departure =
        |d: &[serde_json::Value]| d.iter().rposition(|v| v["source"] == "plan_sent_back");
    wait_until(std::time::Duration::from_secs(10), "the replan's decision", || {
        let d = decisions(&b);
        departure(&d).is_some_and(|at| d.len() > at + 1)
    });
    let decisions = decisions(&b);
    let at = departure(&decisions).expect("the departure given to the machine");
    assert_eq!(decisions[at]["decision"]["after"]["pending"]["state"]["state"], "running");
    let next = &decisions[at + 1];
    assert_eq!(next["source"], "PreToolUse");
    assert!(next["decision"]["after"]["pending"].is_null(), "the leave cancelled: {next}");
    // An observation past the settle's deadline.
    while left.elapsed() < std::time::Duration::from_millis(2500) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        b.c.board().sessions.iter().find(|s| s.id == ws).map(|s| s.state.clone()),
        Some(at_plan)
    );
    let departed = decisions[at]["seq"].as_u64().unwrap();
    let left_plan =
        b.feed().lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).any(
            |v| {
                v["kind"] == "session_state"
                    && v["session"] == ws.to_string()
                    && v["seq"].as_u64().is_some_and(|q| q > departed)
            },
        );
    assert!(!left_plan, "no committed leave: {}", b.feed());
    b.start(b.sa);
    let needs = read(&mut b.c, b.sa, &kw).unwrap().needs_you.expect("the new plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_w2"));
    assert_eq!(b.typed().matches(&wake).count(), 2, "one wake for the new plan");
    assert_eq!(owed(&b), before + 1, "one wake owed for it: {}", b.feed());
}

/// T-933: a worker stopped on a plan or a question across a daemon restart
/// keeps its dialog. The hook stream filled the dialog the crown reads and
/// answers, and a restart emptied it: `needs_you` carried the reason alone
/// and `accept_plan` was refused. The new daemon rebuilds it off the
/// transcript, the unanswered call with its `tool_use` id, so the crown
/// reads `request` and `plan` (or the questions) again and its accept and
/// its answer go through. A dialog the transcript shows answered in the
/// restart's window is not brought back.
#[test]
fn a_worker_s_dialog_outlives_a_daemon_restart() {
    use mesimon_core::board::Reason;
    const STUB: &str = include_str!("common/fake_claude_dialog.py");
    const PLAN: &str = "1. Read the code\n2. Write the code";
    let Some(h) = Harness::boot_bare(
        "crown_redialog",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_redialog");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let pane = |c: &mut TestClient, sid: uuid::Uuid| match c
        .request(Command::PaneTail { session: sid, lines: 60 })
    {
        Response::PaneTail { lines, .. } => lines.join("\n"),
        _ => String::new(),
    };
    let questions = json!({ "questions": [{
        "question": "Which auth provider?",
        "header": "Auth",
        "options": [
            { "label": "Okta", "description": "" },
            { "label": "Auth0", "description": "" }
        ],
        "multiSelect": false
    }]});
    let plan_input = json!({ "plan": PLAN });

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-933 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's worktree start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).expect("W holds a seat").id;

    // W's transcript, which the hook stream's `SessionStart` names.
    let transcript = h.dir.join(format!("{ws}.jsonl"));
    let append = |records: &[serde_json::Value]| {
        use std::io::Write;
        let mut f =
            std::fs::OpenOptions::new().create(true).append(true).open(&transcript).unwrap();
        for r in records {
            writeln!(f, "{r}").unwrap();
        }
    };
    let call = |id: &str, name: &str, input: &serde_json::Value| {
        json!({ "uuid": format!("call-{id}"), "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": id, "name": name, "input": input }] } })
    };
    let result = |id: &str| {
        json!({ "uuid": format!("result-{id}"), "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": id, "content": "ok" }] } })
    };
    append(&[json!({ "uuid": "p", "type": "user", "message": { "content": "brief" } })]);
    let session_start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &session_start);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    start(&mut c, ws);

    // W stops on `tool` with `input`: drawn in its pane, written to its
    // transcript, and the hook that stops it.
    let stop_on = |c: &mut TestClient, tool: &str, id: &str, input: &serde_json::Value| {
        let reason = if tool == "ExitPlanMode" { Reason::Plan } else { Reason::Question };
        std::fs::write(h.dir.join(format!("dialog-{kw}.json")), input.to_string()).unwrap();
        let drawn = if tool == "ExitPlanMode" { "Would you like to proceed?" } else { "Okta" };
        wait_until(std::time::Duration::from_secs(10), "the dialog drawn", || {
            pane(c, ws).contains(drawn)
        });
        append(&[call(id, tool, input)]);
        let frame = json!({ "tool_name": tool, "tool_use_id": id, "tool_input": input });
        hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &frame.to_string());
        c.await_state(ws, "on its dialog", |s| *s == SessionState::RequiresAction { reason });
        let needs = read(c, sa, &kw).unwrap().needs_you.expect("the dialog");
        assert_eq!(needs.request.as_deref(), Some(id), "the hook stream's dialog");
    };
    let edge = |tool: &str, id: &str, input: &serde_json::Value| {
        json!({ "tool_name": tool, "tool_use_id": id, "tool_input": input }).to_string()
    };
    let read_after_restart = |name: &str| {
        let mut c = h.client(name);
        let mut needs = None;
        wait_until(std::time::Duration::from_secs(10), "the dialog rebuilt", || {
            needs = read(&mut c, sa, &kw).unwrap().needs_you;
            needs.as_ref().is_some_and(|n| n.request.is_some())
        });
        (c, needs.unwrap())
    };

    // ---- 1. a plan: rebuilt, read and accepted -----------------------------
    stop_on(&mut c, "ExitPlanMode", "toolu_r1", &plan_input);
    drop(c);
    h.restart();
    let (mut c, needs) = read_after_restart("crown_redialog_2");
    assert_eq!(needs.reason, "plan");
    assert_eq!(needs.request.as_deref(), Some("toolu_r1"), "the transcript's call id");
    assert_eq!(needs.plan.as_deref(), Some(PLAN), "the markdown, lines kept");
    let accepted = h.dir.join(format!("accepted-{kw}"));
    let v = read(&mut c, sa, &kw).unwrap();
    let cmd =
        Command::AgentAcceptPlan { key: kw.clone(), seen: v.seen, request: "toolu_r1".into() };
    let sock = h.paths.orch_sock();
    let receipt = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    wait_until(std::time::Duration::from_secs(10), "the plan accepted in W's pane", || {
        accepted.exists()
    });
    append(&[result("toolu_r1")]);
    hook_send(
        &hook_sock,
        &ws.to_string(),
        "PostToolUse",
        &edge("ExitPlanMode", "toolu_r1", &plan_input),
    );
    match receipt.join().unwrap() {
        Response::AgentPlanAccepted { outcome, reason, .. } => {
            assert_eq!((outcome.as_str(), reason), ("accepted", None))
        }
        other => panic!("accept_plan after a restart: {other:?}"),
    }
    c.await_state(ws, "running the plan", |s| *s == SessionState::Running);

    // ---- 2. a question: rebuilt, read and answered -------------------------
    stop_on(&mut c, "AskUserQuestion", "toolu_r2", &questions);
    drop(c);
    h.restart();
    let (mut c, needs) = read_after_restart("crown_redialog_3");
    assert_eq!(needs.reason, "question");
    assert_eq!(needs.request.as_deref(), Some("toolu_r2"));
    assert_eq!(needs.questions[0].text, "Which auth provider?");
    let v = read(&mut c, sa, &kw).unwrap();
    let cmd = Command::AgentAnswerTicket {
        key: kw.clone(),
        seen: v.seen,
        request: "toolu_r2".into(),
        index: Some(0),
        text: None,
        answers: None,
    };
    let sock = h.paths.orch_sock();
    let receipt = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    let answered = h.dir.join(format!("answered-{kw}.json"));
    wait_until(std::time::Duration::from_secs(10), "the answer in W's pane", || answered.exists());
    append(&[result("toolu_r2")]);
    hook_send(
        &hook_sock,
        &ws.to_string(),
        "PostToolUse",
        &edge("AskUserQuestion", "toolu_r2", &questions),
    );
    match receipt.join().unwrap() {
        Response::AgentAnswered { outcome, answer, .. } => {
            assert_eq!((outcome.as_str(), answer.as_str()), ("answered", "Okta"))
        }
        other => panic!("answer_agent after a restart: {other:?}"),
    }
    c.await_state(ws, "running on the answer", |s| *s == SessionState::Running);

    // ---- 3. answered while no daemon was up: not brought back --------------
    stop_on(&mut c, "ExitPlanMode", "toolu_r3", &plan_input);
    drop(c);
    h.restart_after(|| append(&[result("toolu_r3")]));
    let mut c = h.client("crown_redialog_4");
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let needs = read(&mut c, sa, &kw).unwrap().needs_you;
    assert!(
        needs.as_ref().is_none_or(|n| n.request.is_none() && n.plan.is_none()),
        "an answered plan is gone: {needs:?}"
    );
    let v = read(&mut c, sa, &kw).unwrap();
    let cmd =
        Command::AgentAcceptPlan { key: kw.clone(), seen: v.seen, request: "toolu_r3".into() };
    match c.send(Principal::Agent { session: sa }, cmd) {
        Response::Err { message } => assert!(message.contains("plan changed"), "{message}"),
        other => panic!("accept_plan on an answered plan: {other:?}"),
    }

    // ---- 4. T-936: a long plan, whole to 64 KiB, its head marked cut past it -
    // A 26 KiB design plan was refused a dialog by a 16 KiB bound, on the
    // hook stream's road and the transcript's alike. T-991: a read rides its
    // 512-byte head and its size; a read with `plan` rides it as T-936 had it.
    let plan_of = |bytes: usize| -> String {
        let mut plan = String::new();
        for n in 0.. {
            if plan.len() >= bytes {
                break;
            }
            plan.push_str(&format!("{n}. a step of the design | a table row | its cost |\n"));
        }
        plan
    };
    let whole_of = |c: &mut TestClient| match c.send(
        Principal::Agent { session: sa },
        Command::AgentReadTicket { key: kw.clone(), plan: true },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you.expect("the plan, whole"),
        other => panic!("get_ticket with plan: {other:?}"),
    };
    let long = plan_of(26 * 1024);
    let huge = plan_of(80 * 1024);
    for (id, plan, name) in
        [("toolu_r4", &long, "crown_redialog_5"), ("toolu_r5", &huge, "crown_redialog_6")]
    {
        start(&mut c, ws);
        let input = json!({ "plan": plan });
        stop_on(&mut c, "ExitPlanMode", id, &input);
        let first = read(&mut c, sa, &kw).unwrap().needs_you.expect("the plan");
        let first_whole = whole_of(&mut c);
        drop(c);
        h.restart();
        let (mut again, needs) = read_after_restart(name);
        let whole = whole_of(&mut again);
        c = again;
        for needs in [first, needs] {
            assert_eq!(needs.request.as_deref(), Some(id), "accept_plan has its request");
            assert_eq!(needs.plan_bytes, Some(plan.len() as u64));
            let shown = needs.plan.expect("the plan's head");
            let head = shown.strip_suffix('…').expect("its head, marked");
            assert!(head.len() <= 512 && plan.starts_with(head), "{head}");
        }
        for needs in [first_whole, whole] {
            assert_eq!(needs.request.as_deref(), Some(id), "accept_plan has its request");
            let shown = needs.plan.expect("the plan's words");
            if plan.len() <= 64 * 1024 {
                assert_eq!(&shown, plan, "whole");
                assert_eq!(needs.plan_bytes, None);
            } else {
                let kib = plan.len().div_ceil(1024);
                assert!(shown.starts_with(&plan[..60 * 1024]), "its head");
                assert!(shown.ends_with(&format!("its first 64 KiB of {kib} KiB")), "marked cut");
                assert_eq!(needs.plan_bytes, Some(plan.len() as u64));
            }
        }
        if plan.len() > 64 * 1024 {
            continue;
        }
        // The 26 KiB plan, read whole, is accepted on the road section 1's took.
        let accepted = h.dir.join(format!("accepted-{kw}"));
        let _ = std::fs::remove_file(&accepted);
        let v = read(&mut c, sa, &kw).unwrap();
        let cmd = Command::AgentAcceptPlan { key: kw.clone(), seen: v.seen, request: id.into() };
        let sock = h.paths.orch_sock();
        let receipt = std::thread::spawn(move || {
            TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
        });
        wait_until(std::time::Duration::from_secs(10), "the long plan accepted", || {
            accepted.exists()
        });
        append(&[result(id)]);
        hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &edge("ExitPlanMode", id, &input));
        match receipt.join().unwrap() {
            Response::AgentPlanAccepted { outcome, reason, .. } => {
                assert_eq!((outcome.as_str(), reason), ("accepted", None))
            }
            other => panic!("accept_plan on the long plan: {other:?}"),
        }
        c.await_state(ws, "running the long plan", |s| *s == SessionState::Running);
    }
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("split the writer");
    let p = c.create_ticket("a person's own");
    let (kw, kp) = (key_of(&mut c, w), key_of(&mut c, p));
    let sa = c.spawn_claude(a);
    let sp = c.spawn_claude(p);
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
                expect_secs: None,
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-94 waits in the background");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
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
            expect_secs: None,
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
        uploads: Vec::new(),
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
            expect_secs: None,
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
        ask_ends(&worker).iter().any(|l| !lines_with(l).is_empty())
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
        mesimon_core::train::seat(&board, w, false),
        mesimon_core::train::Seat::Busy,
        "never merged under a worker that may be mid-work"
    );
    let view = read(&mut c, sa, &kw).unwrap().background.expect("still background");
    assert!(view.since_secs.is_some_and(|s| s >= 6), "{view:?}");
}

/// A worker whose only work is a background command (T-944): the command
/// sends nothing while it runs, so the park clock (`MESIMON_PARK_STALE_MS`,
/// ten minutes in life) demotes `Idle{Background}` to `Idle{Monitoring}`.
/// The stretch's stamp (`tasks_since`) is the same before and after the
/// demote, the card still counts the task and `get_ticket` still says
/// `background`; the crown's lingering wake is due `MESIMON_LINGER_MS` from
/// the original park, not from the demote; it comes once per stretch, and
/// a foreground turn begins a new one.
#[test]
fn a_demoted_park_on_a_background_command_lingers_from_its_turn_end() {
    use mesimon_core::board::StopReason;
    const PARK_STALE: u64 = 3_000;
    const LINGER: u64 = 8_000;
    let Some(h) = Harness::boot_with_env(
        "crown_linger_demoted",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_PARK_STALE_MS", "3000"),
            ("MESIMON_LINGER_MS", "8000"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_linger_demoted");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let lines_with = |needle: &str| -> usize {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .count()
    };
    let send = |sid: uuid::Uuid, event: &str, body: &str| {
        hook_send(&hook_sock, &sid.to_string(), event, body);
    };
    let backgrounded = SessionState::Idle { stop_reason: StopReason::Background };
    let monitoring = SessionState::Idle { stop_reason: StopReason::Monitoring };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-944 runs one command");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
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
            expect_secs: None,
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

    // The lingering wakes the feed holds for this worker, by their stamp.
    let linger_wakes = || -> Vec<u64> {
        std::fs::read_to_string(&feed_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| {
                l.contains("\"kind\":\"crown_wake\"")
                    && l.contains(&format!("\"worker\":\"{w}\""))
                    && l.contains("\"cause\":\"lingering\"")
            })
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()?["at_ms"].as_u64())
            .collect()
    };
    // One turn that leaves one background command running, and its park's
    // stamp: the stretch's start.
    let park_on_command = |c: &mut TestClient, n: u32| -> u64 {
        send(ws, "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        send(
            ws,
            "PostToolUse",
            &format!(
                r#"{{"tool_name":"Bash","tool_response":{{"stdout":"","stderr":"","backgroundTaskId":"cmd{n}"}}}}"#
            ),
        );
        send(
            ws,
            "Stop",
            &format!(
                r#"{{"stop_hook_active":false,"background_tasks":[{{"id":"cmd{n}","type":"shell","status":"running"}}]}}"#
            ),
        );
        c.await_state(ws, "parked on its command", |s| {
            *s == SessionState::Idle { stop_reason: StopReason::Background }
        });
        let rec = c.board().live_agent(w).cloned().unwrap();
        assert_eq!(rec.tasks_running, Some(1), "the card's count");
        let since = rec.tasks_since.expect("the stretch's stamp");
        assert_eq!(Some(since), rec.state_changed_at, "the stretch begins at the park");
        since
    };

    // ---- 1. the park demotes, and keeps its stretch -------------------------
    let parked = park_on_command(&mut c, 1);
    c.await_state(ws, "the park clock's demote", |s| {
        *s == SessionState::Idle { stop_reason: StopReason::Monitoring }
    });
    let rec = c.board().live_agent(w).cloned().unwrap();
    assert!(rec.state_changed_at.is_some_and(|at| at >= parked + PARK_STALE), "{rec:?}");
    assert_eq!(rec.tasks_since, Some(parked), "the demote keeps the stretch's stamp");
    assert_eq!(rec.tasks_running, Some(1), "the card still counts the command");
    let view = read(&mut c, sa, &kw).unwrap().background.expect("background on get_ticket");
    assert_eq!(view.tasks, 1);
    assert!(view.since_secs.is_some_and(|s| s >= PARK_STALE / 1000), "{view:?}");

    // ---- 2. due LINGER from the park, not from the demote --------------------
    wait_until(std::time::Duration::from_secs(20), "the lingering wake", || {
        !linger_wakes().is_empty()
    });
    let at = linger_wakes()[0];
    let late = at.saturating_sub(parked);
    assert!(
        (LINGER..PARK_STALE + LINGER - 500).contains(&late),
        "due {LINGER} ms from the park, not {} ms: came at {late} ms",
        PARK_STALE + LINGER
    );
    let worker = format!("{kw} \"mesimon-probe-944 runs one command\"");
    let lingering = format!("{worker} has been idle with 1 background task for ");
    wait_until(std::time::Duration::from_secs(10), "the wake in the crown's pane", || {
        lines_with(&lingering) == 1
    });
    send(sa, "UserPromptSubmit", r#"{"prompt":"wake"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    send(sa, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // ---- 3. once per stretch ---------------------------------------------------
    std::thread::sleep(std::time::Duration::from_millis(LINGER + 1_000));
    assert_eq!(linger_wakes().len(), 1, "no second wake without a foreground turn");
    assert_eq!(lines_with(&lingering), 1);
    assert_eq!(c.board().live_agent(w).unwrap().state, monitoring, "nothing decided by time");

    // ---- 4. a foreground turn ends the stretch; the next park is new ----------
    let again = park_on_command(&mut c, 2);
    assert!(again > parked + LINGER, "a new stretch, not the old stamp");
    assert_eq!(c.board().live_agent(w).unwrap().state, backgrounded);
    wait_until(std::time::Duration::from_secs(20), "the new stretch's wake", || {
        linger_wakes().len() == 2
    });
    assert!(linger_wakes()[1] >= again + LINGER, "{:?} vs {again}", linger_wakes());
}

/// T-613: the crown merges where the train will not. `get_ticket` on a
/// worktree ticket says who merges its branch (`merge: by, why`) — the crown
/// with the train off, the train once armed, the crown again once the
/// ticket is taken off it by `t`, a person under supervised mode — and the
/// delivered wake line carries the word. `merge_ticket` is refused in words
/// on the crown's own ticket, with nothing to merge, under supervised mode
/// and at `needs_rebase` (the rebase word), and otherwise takes the ff-only
/// road (none of it on a board that merges through pull requests, T-810,
/// where the forge is named and `merge_ticket` asks for a PR): the branch
/// lands, the card reads `♛ merged`, the feed names the
/// agent's `merge_ticket` with its outcome, and the merged notice goes to
/// the worker where the pref is on, so the crown's `merged` wake waits for
/// that notice turn (T-596). With the pref off (this board's `prefs.json`,
/// read by the daemon at the merge), no notice and the wake comes at once.
#[test]
fn the_crown_merges_where_the_train_will_not() {
    let Some(h) = Harness::boot_with_env(
        "crown_merges",
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
    let mut c = h.client("crown_merges");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-97 merged by the crown");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    assert!(v.merge.is_none(), "no branch yet, so nobody is named to merge it");
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    let worker = format!("{kw} \"mesimon-probe-97 merged by the crown\"");
    let merged = || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };
    let merge = |c: &mut TestClient, key: &str, seen: Option<String>| {
        c.send(
            Principal::Agent { session: sa },
            Command::AgentMergeTicket { key: key.into(), seen },
        )
    };
    let refused = |resp: Response, words: &[&str]| {
        let Response::Err { message } = resp else { panic!("not refused: {resp:?}") };
        for w in words {
            assert!(message.contains(w), "the refusal names {w:?}: {message}");
        }
    };
    let who = |c: &mut TestClient, key: &str| {
        let v = read(c, sa, key).unwrap();
        let m = v.merge.expect("a worktree ticket says who merges it");
        (m.by, m.why, v.seen)
    };
    let wait_state = |c: &mut TestClient, word: &str| {
        wait_until(
            std::time::Duration::from_secs(10),
            &format!("the flags to read the branch {word}"),
            || read(c, sa, &kw).unwrap().merge_state.as_deref() == Some(word),
        );
    };
    let notice = format!("Your branch {branch} has been merged into main");

    // ---- 1. nothing to merge yet, and the crown's own ticket --------------
    let (by, why, seen) = who(&mut c, &kw);
    assert_eq!((by.as_str(), why.as_str()), ("crown", "the train is off"));
    refused(merge(&mut c, &kw, seen), &[&kw, "no commits to merge"]);
    let own = read(&mut c, sa, &ka).unwrap();
    refused(merge(&mut c, &ka, own.seen), &["this session's own ticket"]);
    // The worker reads who merges on its own ticket too.
    let mine = read(&mut c, ws, &kw).unwrap();
    assert_eq!(mine.merge.map(|m| m.by).as_deref(), Some("crown"));
    // A turn with nothing new: the card lands in REVIEW, the train's column,
    // and the crown hears the finish (T-591).
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the finish's wake", || {
        lines_with(&worker).len() == 1
    });
    // The crown's first line carries its typed title in front (no Enter
    // was sent for it), so the first wake is matched inside it.
    assert!(
        lines_with(&worker)[0].contains(&format!("{worker} finished its turn")),
        "{}",
        lines_with(&worker)[0]
    );
    assert_eq!(c.board().ticket(w).unwrap().column, "REVIEW");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 2. who merges, by the board's state ------------------------------
    // Armed, the train has nothing to take yet (no commits), so nothing
    // moves while the words are read.
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let (by, why, _) = who(&mut c, &kw);
    assert_eq!(by, "train", "{why}");
    // Working, the card sits in IN PROGRESS, where the train only asks for
    // rebases; the turn's end puts it back in REVIEW, and that is where it
    // delivers (T-795). The turn ends in 3, as the delivery.
    start(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the turn to pull the card", || {
        c.board().ticket(w).unwrap().column == "IN PROGRESS"
    });
    let (by, why, _) = who(&mut c, &kw);
    assert_eq!(
        (by.as_str(), why.as_str()),
        ("train", "the train is armed and reaches REVIEW, where this turn's end puts it")
    );
    assert!(matches!(c.request(Command::SetManualMerge { id: w, on: true }), Response::Ok));
    let (by, why, _) = who(&mut c, &kw);
    assert_eq!((by.as_str(), why.as_str()), ("crown", "out of the train (t)"));
    assert!(matches!(c.request(Command::SetManualMerge { id: w, on: false }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: false, merge_notice: false }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let (by, why, seen) = who(&mut c, &kw);
    assert_eq!((by.as_str(), why.as_str()), ("person", "crown mode is supervised"));
    refused(merge(&mut c, &kw, seen), &["Crown mode is supervised", "m on the ticket's page"]);
    // The refused attempt is in the feed, as the merge road's is (T-782).
    assert_eq!(feed_count(&h, "merge_ticket", w), 1);
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));

    // ---- 3. the delivery: the wake line says who merges -------------------
    // The pref off, in this board's own prefs.json: the daemon reads it at
    // the merge, no board open.
    std::fs::write(h.paths.prefs_file(), r#"{"merge_train_notice": false}"#).unwrap();
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 2
    });
    assert_eq!(
        lines_with(&worker)[1],
        format!(
            "{worker} delivered (merge_state clean → ahead, merge: crown, ahead 0 → 1) ∙ \
             get_ticket key={kw} for state and notes"
        )
    );
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 4. behind its base: the rebase word ------------------------------
    std::fs::write(h.repo.join("base1.txt"), "base\n").unwrap();
    git(&h.repo, &["add", "base1.txt"]);
    git(&h.repo, &["commit", "-qm", "base1"]);
    wait_state(&mut c, "needs_rebase");
    let (_, _, seen) = who(&mut c, &kw);
    refused(merge(&mut c, &kw, seen), &[&kw, "behind its base", "ask_agent", "merge_state ahead"]);
    assert!(!merged());
    // The worker rebases and ends its turn: delivered again, at the new tip.
    start(&mut c, ws);
    git(&path, &["rebase", "-q", "main"]);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the rebased delivery's wake", || {
        lines_with(&worker).len() == 3
    });
    let line = lines_with(&worker)[2].clone();
    assert!(line.starts_with(&format!("{worker} delivered (")), "{line}");
    assert!(line.contains("merge: crown"), "{line}");
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_state(&mut c, "ahead");

    // ---- 5. the crown merges, pref off: no notice, the wake at once -------
    // T-810: not on a board that merges through pull requests, where the
    // forge merges every worktree branch and the crown is asked for none.
    assert!(matches!(
        c.request(Command::SetMergeInto { into: MergeInto::PullRequests }),
        Response::Ok
    ));
    let (by, why, seen) = who(&mut c, &kw);
    assert_eq!((by.as_str(), why.as_str()), ("forge", "this board merges through pull requests"));
    refused(merge(&mut c, &kw, seen), &[&kw, "open one from its branch", "merge it on GitHub"]);
    assert!(!merged());
    assert!(matches!(c.request(Command::SetMergeInto { into: MergeInto::Main }), Response::Ok));
    let (by, _, seen) = who(&mut c, &kw);
    assert_eq!(by, "crown");
    match merge(&mut c, &kw, seen) {
        Response::AgentMerged { key, detail, notice, seen } => {
            assert_eq!(key, kw);
            assert!(detail.contains(&format!("{branch} merged into main")), "{detail}");
            assert_eq!(notice, "off");
            assert!(seen.is_some(), "the receipt carries the fresh stamp");
        }
        other => panic!("merge_ticket: {other:?}"),
    }
    assert!(merged());
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w).map(|t| t.action.as_str()),
        Some("merged"),
        "the card lights ♛ merged"
    );
    wait_until(std::time::Duration::from_secs(5), "the merge in the feed", || {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains("\"cmd\":\"merge_ticket\"")
                    && l.contains("\"outcome\":\"merged\"")
                    && l.contains("\"actor\":\"agent\"")
            })
        })
    });
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 4
    });
    assert_eq!(
        lines_with(&worker)[3],
        format!(
            "{worker} merged (merge_state ahead → merged) ∙ get_ticket key={kw} for state and \
             notes"
        )
    );
    assert!(lines_with(&notice).is_empty(), "the pref is off: no notice");
    let (_, _, seen) = who(&mut c, &kw);
    refused(merge(&mut c, &kw, seen), &[&kw, "already merged"]);
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 6. pref on (the default): the notice goes, the wake waits for it -
    std::fs::remove_file(h.paths.prefs_file()).unwrap();
    commit(&path, "two.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the second delivery's wake", || {
        lines_with(&worker).len() == 5
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_state(&mut c, "ahead");
    let (_, _, seen) = who(&mut c, &kw);
    match merge(&mut c, &kw, seen) {
        Response::AgentMerged { notice, .. } => assert_eq!(notice, "sent"),
        other => panic!("merge_ticket: {other:?}"),
    }
    assert!(merged());
    wait_until(std::time::Duration::from_secs(10), "the merged notice", || {
        lines_with(&notice).len() == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 5, "held while the notice is on its way");
    start(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 5, "held while the notice turn runs");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the wake at the turn's end", || {
        lines_with(&worker).len() == 6
    });
    assert_eq!(
        lines_with(&worker)[5],
        format!(
            "{worker} merged and finished its turn (merge_state ahead → merged) ∙ get_ticket \
             key={kw} for state and notes"
        )
    );
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert_eq!(feed.matches("\"crown_wake_deferred:merge_step\"").count(), 1, "{feed}");
    assert!(feed.contains("\"merge_notified\""), "{feed}");
}

/// T-613: a person's `m` on a worker the crown started tells the worker by
/// itself — the merged notice goes in the same step where the pref is on,
/// the reply says `notified`, so the dialog offers no second press — and the
/// crown's `merged` wake waits for that notice turn (T-596), so the person's
/// gesture and the crown's close-out no longer collide. A person's own
/// worker keeps the two steps: its merge reply is not `notified`, and no
/// notice goes until the person's second `m`.
#[test]
fn a_hand_merge_of_a_crown_started_worker_tells_it_by_itself() {
    let Some(h) = Harness::boot_with_env(
        "crown_hand_merge",
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
    let mut c = h.client("crown_hand_merge");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-98 told by the board");
    let p = c.create_ticket("mesimon-probe-99 a person's own");
    let kw = key_of(&mut c, w);
    for id in [w, p] {
        assert!(matches!(
            c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
            Response::Ok
        ));
    }
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    // W: the crown's worker. P: the person's own, spawned by hand.
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: p,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    let wt_w = wait_attached(&mut c, w);
    let wt_p = wait_attached(&mut c, p);
    let path_w = std::path::PathBuf::from(wt_w.path.expect("a path"));
    let path_p = std::path::PathBuf::from(wt_p.path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "both parked starts to land", || {
        c.board().live_agent(w).is_some() && c.board().live_agent(p).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    let ps = c.board().live_agent(p).unwrap().id;
    assert!(c.board().live_agent(w).unwrap().started_by.is_some());
    assert!(c.board().live_agent(p).unwrap().started_by.is_none());
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kw} \"mesimon-probe-98 told by the board\"");
    let merged = |branch: &str| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };
    let wait_ahead = |c: &mut TestClient, id: ulid::Ulid| {
        wait_until(
            std::time::Duration::from_secs(10),
            "the flags to read the branch ahead",
            || wt_of(c, id).is_some_and(|w| w.ahead > 0 && !w.merged && !w.needs_rebase),
        );
    };

    // ---- 1. the crown's worker delivers; the person presses m -------------
    commit(&path_w, "work.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    wait_ahead(&mut c, w);
    match c.request(Command::MergeTicket { id: w }) {
        Response::Merge {
            outcome: mesimon_core::command::MergeOutcome::Merged, notified, ..
        } => {
            assert!(notified, "the board told the crown's worker in the same step");
        }
        other => panic!("m on W: {other:?}"),
    }
    assert!(merged(&wt_w.branch));
    let notice_w = format!("Your branch {} has been merged into main", wt_w.branch);
    wait_until(std::time::Duration::from_secs(10), "the merged notice to W", || {
        lines_with(&notice_w).len() == 1
    });
    // The crown's wake waits for the notice turn, then says so.
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 1, "held while the notice is on its way");
    start(&mut c, ws);
    std::thread::sleep(std::time::Duration::from_millis(2000));
    assert_eq!(lines_with(&worker).len(), 1, "held while the notice turn runs");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the wake at the turn's end", || {
        lines_with(&worker).len() == 2
    });
    assert_eq!(
        lines_with(&worker)[1],
        format!(
            "{worker} merged and finished its turn (merge_state ahead → merged) ∙ get_ticket \
             key={kw} for state and notes"
        )
    );

    // ---- 2. the person's own worker keeps the two steps -------------------
    // W's merge moved main under P: P rebases in its turn first.
    commit(&path_p, "mine.txt");
    start(&mut c, ps);
    git(&path_p, &["rebase", "-q", "main"]);
    stop(&mut c, ps);
    wait_ahead(&mut c, p);
    match c.request(Command::MergeTicket { id: p }) {
        Response::Merge {
            outcome: mesimon_core::command::MergeOutcome::Merged, notified, ..
        } => {
            assert!(!notified, "a person's worker is told by the person's second m");
        }
        other => panic!("m on P: {other:?}"),
    }
    assert!(merged(&wt_p.branch));
    let notice_p = format!("Your branch {} has been merged into main", wt_p.branch);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(lines_with(&notice_p).is_empty(), "no notice until the second m");
    assert!(matches!(
        c.request(Command::MergeToAgent {
            id: p,
            request: mesimon_core::command::MergeRequest::MergedNotice,
        }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(10), "the second m's notice to P", || {
        lines_with(&notice_p).len() == 1
    });
    // P is nobody's worker under the crown: its merge woke nothing (a wake
    // line quotes the title; P's own typed title carries no quotes).
    assert!(lines_with("a person's own\" ").is_empty());
}

/// The crown watches a ticket it did not start (T-712): a person's worker on
/// a worktree. Unwatched, its delivery wakes nobody (T-527's rule). The row
/// is on by default (T-732) and its off is written to `columns.toml`; the
/// watch is refused while Settings → Agents → Crown watches tickets is off,
/// in words naming the row, and on the crown's own ticket; on, the watched
/// ticket's delivery, finished turn and raised hand wake the crown with the
/// lines a started worker's make, a hand merge wakes it `merged` and ends
/// the watch, `get_ticket` with the key reads `watched`, `unwatch` ends one
/// by hand, the row turned off ends every one, and `crown.json` carries the
/// watch.
#[test]
fn the_crown_watches_a_ticket_a_person_started() {
    let Some(h) = Harness::boot_with_env(
        "crown_watch",
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
    let mut c = h.client("crown_watch");
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
    let feed_has = |cmd: &str, ticket: ulid::Ulid| {
        std::fs::read_to_string(&feed_path).is_ok_and(|feed| {
            feed.lines().any(|l| {
                l.contains(&format!("\"cmd\":\"{cmd}\""))
                    && l.contains(&format!("\"ticket\":\"{ticket}\""))
            })
        })
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    // The probe of a turn's end and the flags' sample run off the writer;
    // give both time to land before asserting that nothing did.
    let settle = || std::thread::sleep(std::time::Duration::from_millis(2500));

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-93 watched");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    // W: the person's own, spawned by hand.
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: w,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    let wt = wait_attached(&mut c, w);
    let path = std::path::PathBuf::from(wt.path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    assert!(c.board().live_agent(w).unwrap().started_by.is_none(), "a person's agent");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kw} \"mesimon-probe-93 watched\"");
    let watch = |c: &mut TestClient, key: &str, unwatch: bool| {
        let v = read(c, sa, key).unwrap();
        c.send(
            Principal::Agent { session: sa },
            Command::AgentWatchTicket { key: key.into(), unwatch, seen: v.seen },
        )
    };
    let off_words = format!(
        "Settings → Agents → Crown watches tickets is off; a person turns it on, or says when \
         {kw} is done"
    );

    // ---- 1. unwatched, a person's worker wakes nobody (T-527) ---------------
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    settle();
    assert!(lines_with(&worker).is_empty(), "{}", std::fs::read_to_string(&got).unwrap());

    // ---- 2. the row is off: refused in words naming it, nothing watched -----
    assert!(c.board().crown_watches, "on by default");
    assert!(matches!(c.request(Command::SetCrownWatches { on: false }), Response::Ok));
    assert!(!c.board().crown_watches);
    assert!(!mesimon_daemon::store::read_columns_scalars(&h.paths).crown_watches);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_watches = false"), "{file}");
    match watch(&mut c, &kw, false) {
        Response::Err { message } => assert_eq!(message, off_words),
        other => panic!("a watch while the row is off: {other:?}"),
    }
    assert!(!read(&mut c, sa, &kw).unwrap().watched);
    assert!(touches(&mut c).iter().all(|t| t.ticket != w), "no touch for a refusal");

    // ---- 3. on: the crown's own ticket is refused, so is an unwatch of a
    // ticket not watched; the watch lands, reads back, and is in the ledger
    assert!(matches!(c.request(Command::SetCrownWatches { on: true }), Response::Ok));
    assert!(c.board().crown_watches);
    assert!(mesimon_daemon::store::read_columns_scalars(&h.paths).crown_watches);
    let file = std::fs::read_to_string(h.paths.board_dir.join("board/columns.toml")).unwrap();
    assert!(file.contains("crown_watches = true"), "{file}");
    match watch(&mut c, &ka, false) {
        Response::Err { message } => assert!(message.contains("own ticket"), "{message}"),
        other => panic!("watching the crown's own ticket: {other:?}"),
    }
    match watch(&mut c, &kw, true) {
        Response::Err { message } => assert_eq!(message, format!("{kw} is not watched")),
        other => panic!("unwatching an unwatched ticket: {other:?}"),
    }
    // A subscriber hears the watch (T-719): it flips no board field, so the
    // touch's bolt reaches the TUI only on a broadcast of the arm's own.
    let mut sub = h.client("crown_watch_sub");
    assert!(matches!(sub.request(Command::Subscribe), Response::Ok));
    while sub.next_event(std::time::Duration::from_millis(300)).is_some() {}
    match watch(&mut c, &kw, false) {
        Response::AgentTicket { ticket } => assert!(ticket.watched, "the receipt says so"),
        other => panic!("watch: {other:?}"),
    }
    assert!(
        sub.next_event(std::time::Duration::from_secs(5)).is_some(),
        "the watch broadcasts BoardChanged"
    );
    assert!(read(&mut c, sa, &kw).unwrap().watched, "the keyed read says so");
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w).map(|t| t.action.as_str()),
        Some("watched"),
        "the card lights"
    );
    wait_until(std::time::Duration::from_secs(5), "the watch in crown.json", || {
        std::fs::read_to_string(h.paths.crown_file())
            .is_ok_and(|t| t.contains("\"watched\"") && t.contains(&w.to_string()))
    });
    wait_until(std::time::Duration::from_secs(5), "the watch's feed line", || {
        feed_has("watch_ticket", w)
    });
    // Watching again changes nothing.
    assert!(matches!(watch(&mut c, &kw, false), Response::AgentTicket { .. }));

    // ---- 4. a delivery the crown never heard of: delivered -----------------
    commit(&path, "two.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).len() == 1
    });
    // The crown's first paste lands behind its launch words (the title,
    // typed with no Enter), so the first line is read by its tail.
    let line = lines_with(&worker).remove(0);
    assert!(line.contains(&format!("{worker} delivered (merge_state ahead, merge: ")), "{line}");
    assert!(line.ends_with(&format!("∙ get_ticket key={kw} for state and notes")), "{line}");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 5. a turn with nothing new: finished (T-591) ----------------------
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the finish's wake", || {
        lines_with(&worker).len() == 2
    });
    assert!(
        lines_with(&worker)[1].starts_with(&format!("{worker} finished its turn (nothing new")),
        "{:?}",
        lines_with(&worker)
    );
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 6. its hand goes up: raised, the reason never on the line ---------
    match c.send(
        Principal::Agent { session: ws },
        Command::AgentRaiseHand { reason: "mesimon-probe-94 the reason".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand on W: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the hand's wake", || {
        lines_with(&worker).len() == 3
    });
    assert!(lines_with(&worker)[2].starts_with(&format!("{worker} raised its hand")));
    assert!(!std::fs::read_to_string(&got).unwrap().contains("mesimon-probe-94"));
    assert!(!c.board().ticket(w).unwrap().hand_raised(), "told, the hand is down (T-861)");
    // A watched ticket's reason is the crown's to read as a started one's
    // (T-922).
    let told = read(&mut c, sa, &kw).unwrap().told_hand.map(|h| h.reason);
    assert_eq!(told.as_deref(), Some("mesimon-probe-94 the reason"));
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 7. merged in a terminal: one merged line, and the watch ends ------
    wait_until(std::time::Duration::from_secs(10), "the flags to read the branch ahead", || {
        wt_of(&mut c, w).is_some_and(|x| x.ahead > 0 && !x.merged && !x.needs_rebase)
    });
    git(&h.repo, &["merge", "--ff-only", "-q", &wt.branch]);
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).len() == 4
    });
    assert_eq!(
        lines_with(&worker)[3],
        format!(
            "{worker} merged (merge_state ahead → merged) ∙ get_ticket key={kw} for state and \
             notes"
        )
    );
    wait_until(std::time::Duration::from_secs(5), "the watch's end in the feed", || {
        feed_has("watch_ended", w)
    });
    assert!(!read(&mut c, sa, &kw).unwrap().watched, "the merge ended the watch");
    start(&mut c, sa);
    stop(&mut c, sa);
    // A turn after the merge, unwatched: silent.
    start(&mut c, ws);
    stop(&mut c, ws);
    settle();
    assert_eq!(lines_with(&worker).len(), 4, "{:?}", lines_with(&worker));

    // ---- 8. watched again and unwatched by hand: silent after --------------
    assert!(matches!(watch(&mut c, &kw, false), Response::AgentTicket { .. }));
    match watch(&mut c, &kw, true) {
        Response::AgentTicket { ticket } => assert!(!ticket.watched),
        other => panic!("unwatch: {other:?}"),
    }
    assert_eq!(
        touches(&mut c).iter().find(|t| t.ticket == w).map(|t| t.action.as_str()),
        Some("unwatched")
    );
    wait_until(std::time::Duration::from_secs(5), "the unwatch's feed line", || {
        feed_has("unwatch_ticket", w)
    });
    start(&mut c, ws);
    stop(&mut c, ws);
    settle();
    assert_eq!(lines_with(&worker).len(), 4, "unwatched, so silent");

    // ---- 9. the row turned off ends a standing watch -----------------------
    assert!(matches!(watch(&mut c, &kw, false), Response::AgentTicket { .. }));
    assert!(matches!(c.request(Command::SetCrownWatches { on: false }), Response::Ok));
    assert!(!read(&mut c, sa, &kw).unwrap().watched, "off ends every watch");
    wait_until(std::time::Duration::from_secs(5), "the drop in the feed", || {
        std::fs::read_to_string(&feed_path)
            .is_ok_and(|feed| feed.contains("\"cmd\":\"crown_watch_dropped\""))
    });
    match watch(&mut c, &kw, false) {
        Response::Err { message } => assert_eq!(message, off_words),
        other => panic!("a watch after the row went off: {other:?}"),
    }

    // ---- 10. the crown's words (T-734): held for the person unwatched, -----
    // sent as to a worker it started while watched, held again at the end
    let ask = |c: &mut TestClient, text: &str, deliver: Deliver| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { road, held_because, .. } => {
                (road.unwrap(), held_because.unwrap_or_default())
            }
            other => panic!("ask_agent: {other:?}"),
        }
    };
    let row = |c: &mut TestClient| {
        pending_of(c, Some(w)).into_iter().find(|p| p.is_queued_ask() && !p.in_flight)
    };
    let landed = |needle: &str| std::fs::read_to_string(&got).unwrap_or_default().contains(needle);
    assert!(matches!(c.request(Command::SetCrownWatches { on: true }), Response::Ok));
    assert!(c.board().crown_mode.sends(), "autonomous by default");
    let (road, why) = ask(&mut c, "mesimon-probe-95 held", Deliver::Idle);
    assert_eq!(road, AskRoad::HeldForPerson);
    assert!(why.contains("a person started this agent"), "{why}");
    assert!(!row(&mut c).expect("held on W's card").sends);
    // Watched: the queue takes the words, as for a worker the crown started.
    assert!(matches!(watch(&mut c, &kw, false), Response::AgentTicket { .. }));
    start(&mut c, ws);
    let (road, _) = ask(&mut c, "mesimon-probe-96 queued", Deliver::Idle);
    assert_eq!(road, AskRoad::Queued, "a busy agent takes them at its idle");
    assert!(row(&mut c).expect("queued on W's card").sends);
    // The unwatch hands the waiting words back to the person.
    assert!(matches!(watch(&mut c, &kw, true), Response::AgentTicket { .. }));
    assert!(!row(&mut c).expect("still on W's card").sends, "held again");
    hook_send(&hook_sock, &ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    settle();
    assert!(!landed("mesimon-probe-96"), "held words do not go on their own");
    assert!(matches!(
        c.request(Command::TakeQueuedAsk { ticket: w }),
        Response::PromptTakenBack { .. }
    ));
    // Watched again, an idle agent takes words sent now, and the turn that
    // answers wakes the crown as a started worker's does.
    assert!(matches!(watch(&mut c, &kw, false), Response::AgentTicket { .. }));
    let (road, _) = ask(&mut c, "mesimon-probe-97 now", Deliver::Now);
    assert_eq!(road, AskRoad::SentNow);
    wait_until(std::time::Duration::from_secs(5), "the words in W's pane", || {
        landed("mesimon-probe-97 now")
    });
    assert!(row(&mut c).is_none(), "nothing parked");
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"x"}"#);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(20), "the answer's wake on the crown", || {
        ask_ends(&worker).iter().any(|l| landed(l))
    });

    // Nothing of the crown's reached W's pane but its asks: its other words
    // there are the wake lines, which name W and land at the crown.
    let feed = std::fs::read_to_string(&feed_path).unwrap();
    assert!(!feed.contains("mesimon-probe-93"), "the feed never carries the words:\n{feed}");
}

/// A start the crown asks for whose worktree cannot be cut is told to the
/// crown (T-777): the receipt said the tree was being cut, and the crown's
/// next prompt says it never was, in the card's own words, which end on the
/// fix. The card reads that sentence alone, never git's `ambiguous argument
/// 'HEAD'` behind a stage word, and the crown's `get_ticket` carries it.
#[test]
fn a_crown_start_whose_worktree_cannot_be_cut_wakes_the_crown_with_why() {
    let Some(h) = Harness::boot_with_env(
        "crown_unstarted",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    // A repository with no commits yet: `main` is unborn.
    std::fs::create_dir_all(&h.repo).unwrap();
    git(&h.repo, &["init", "-q", "-b", "main"]);
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_unstarted");
    let got = h.dir.join("got.txt");
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-777 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: false, .. } => {}
        other => panic!("start_agent {kw}: {other:?}"),
    }
    let why = "no commits on main yet — make a first commit, then start again";
    let sentence = format!(
        "{kw} \"mesimon-probe-777 worker\" could not start (its worktree: {why}) ∙ get_ticket \
         key={kw} for state and notes"
    );
    wait_until(std::time::Duration::from_secs(10), "the crown to hear the failed start", || {
        // The stub reads the crown's unsubmitted title on the same line.
        std::fs::read_to_string(&got).unwrap_or_default().lines().any(|l| l.ends_with(&sentence))
    });
    let item = match c.request(Command::Snapshot) {
        Response::Board { worktrees, .. } => worktrees.into_iter().find(|i| i.ticket == w),
        other => panic!("snapshot: {other:?}"),
    }
    .expect("W's binding");
    assert_eq!((item.status.as_str(), item.detail.as_deref()), ("error", Some(why)));
    assert_eq!(read(&mut c, sa, &kw).unwrap().worktree_error.as_deref(), Some(why));
    assert!(c.board().live_agent(w).is_none(), "no agent was started");
}

/// T-802: a worker the crown started stops on a usage limit. The card needs
/// you (QUOTA, the limit's words under the title), the crown hears `hit a
/// usage limit`, and the worker's `get_ticket` says why. A prompt that runs
/// takes the card back to work. T-808: limited again, the turn has ended —
/// `sleep_agent` parks it and `start_agent` wakes it on another tier.
#[test]
fn a_usage_limit_needs_you_and_wakes_the_crown_that_started_the_worker() {
    let Some(h) = Harness::boot_with_env(
        "crown_limit",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_limit");
    let got = h.dir.join("got.txt");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-802 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));

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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's worktree start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    // W's conversation, as Claude's store keeps it: what `--resume` reads,
    // so a wake carries it on rather than starting a new one (T-920).
    let projects = h.dir.join("claude-home").join("projects").join("msmn");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(projects.join(format!("{ws}.jsonl")), "{\"type\":\"user\"}\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);

    let words = "You've reached your Fable limit. Run /usage-credits to continue or switch \
                 models with /model.";
    let body = json!({ "error": "rate_limit", "last_assistant_message": words }).to_string();
    hook_send_with(&hook_sock, &ws.to_string(), "StopFailure", Some("rate_limit"), &body);
    let quota = SessionState::RequiresAction { reason: mesimon_core::board::Reason::QuotaResume };
    c.await_state(ws, "on the limit", |s| *s == quota);
    let board = c.board();
    let rec = board.sessions.iter().find(|s| s.id == ws).expect("W's record");
    assert!(mesimon_core::attention::is_attention(&rec.state), "it needs you");
    assert!(rec.detail.as_deref().is_some_and(|d| d.contains("Fable limit")), "{:?}", rec.detail);

    let wake = format!("{kw} \"mesimon-probe-802 worker\" hit a usage limit");
    wait_until(std::time::Duration::from_secs(10), "the limit's wake on the crown", || {
        std::fs::read_to_string(&got).unwrap_or_default().contains(&wake)
    });
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.expect("the limit");
    assert_eq!(needs.reason, "quota");

    // The person's prompt runs: the wait is over.
    start(&mut c, ws);

    // ---- T-808: the limit ended the turn; the crown parks it and moves it ---
    // Limited again, the worker is no turn in progress: the checkout and the
    // train are free of it, `sleep_agent` parks it with its conversation, and
    // `start_agent` naming another tier wakes it there.
    hook_send_with(&hook_sock, &ws.to_string(), "StopFailure", Some("rate_limit"), &body);
    c.await_state(ws, "on the limit again", |s| *s == quota);
    let board = c.board();
    let rec = board.sessions.iter().find(|s| s.id == ws).expect("W's record");
    assert!(!mesimon_core::quiet::is_working(rec), "the turn has ended");
    assert!(mesimon_core::quiet::holds_against_words(rec), "words still wait");
    assert_ne!(c.board().ticket(w).unwrap().column, "REVIEW", "no done move");
    let deep = mesimon_core::tier::Tier {
        id: "01DEEP".into(),
        name: "deep".into(),
        provider: mesimon_core::board::AgentProvider::ClaudeCode,
        model: "opus".into(),
        effort: mesimon_core::tier::Effort::Max,
        description: String::new(),
    };
    let save = Command::SaveTier { scope: mesimon_core::tier::TierScope::Board, tier: deep };
    assert!(matches!(c.request(save), Response::Ok));
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("sleep_agent over a usage limit: {other:?}"),
    }
    c.await_state(ws, "parked", |s| *s == SessionState::Sleeping);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: Some("deep".into()),
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        // T-920: the wake sent nothing, and the receipt says so.
        Response::AgentStarted { tier, woken, prompt, .. } => {
            assert_eq!(tier, "deep");
            assert!(woken);
            assert_eq!(prompt, Some(StartPrompt::None), "a wake submits nothing");
        }
        other => panic!("start_agent on another tier: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "W woken on the deep tier", || {
        c.board().live_agent(w).is_some_and(|s| s.id == ws && s.state != SessionState::Sleeping)
    });
    let board = c.board();
    let rec = board.live_agent(w).expect("W's seat");
    assert_eq!(rec.tier, "01DEEP");
    let model = rec.argv.iter().position(|a| a == "--model").and_then(|at| rec.argv.get(at + 1));
    assert_eq!(model.map(String::as_str), Some("opus"), "{:?}", rec.argv);
    // The same conversation in the same worktree: the record, its resume and
    // its cwd are the ones it parked with.
    assert!(rec.argv.iter().any(|a| a == "--resume"), "{:?}", rec.argv);
    let cwd = rec.cwd.clone();
    assert!(cwd.contains("/worktrees/"), "{cwd}");

    // ---- T-920: up and idle, it stays idle until words reach it ------------
    let lines = || std::fs::read_to_string(&got).unwrap_or_default().lines().count();
    let before = lines();
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    c.await_state(ws, "idle after the wake", |s| matches!(s, SessionState::Idle { .. }));
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let board = c.board();
    let rec = board.live_agent(w).expect("W's seat");
    assert!(matches!(rec.state, SessionState::Idle { .. }), "no turn began: {:?}", rec.state);
    assert_eq!(lines(), before, "nothing was typed into W or the crown");

    // The crown's words, sent once, are what carry it on: one paste into W.
    let probe = "mesimon-probe-920 carry on where the limit stopped you";
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: probe.into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { held_for_person: false, wake: None, .. } => {}
        other => panic!("ask_agent to the woken worker: {other:?}"),
    }
    let count = || std::fs::read_to_string(&got).unwrap_or_default().matches(probe).count();
    wait_until(std::time::Duration::from_secs(10), "the crown's words to reach W", || count() > 0);
    start(&mut c, ws);
    hook_send(&hook_sock, &ws.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "W's carried-on turn ended", |s| matches!(s, SessionState::Idle { .. }));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(count(), 1, "the words went once");
    let rec = c.board().live_agent(w).cloned().expect("W's seat");
    assert_eq!((rec.id, rec.cwd.as_str()), (ws, cwd.as_str()), "one conversation, one worktree");

    // ---- T-920: words a person holds wait for the person, after a wake too --
    // Parked again with the crown's ask held for a person (supervised), the
    // wake's receipt says the next turn is the person's send, and that send
    // reaches the woken pane: an ask queued for the parked seat follows the
    // record into its new pane rather than dropping as a failed wake.
    let v = read(&mut c, sa, &kw).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: kw.clone(), seen: v.seen }
        ),
        Response::AgentTicket { .. }
    ));
    c.await_state(ws, "parked again", |s| *s == SessionState::Sleeping);
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let held = "mesimon-probe-921 held for the person";
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: held.into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { held_for_person: true, wake: Some(AskWake::WhenSent), .. } => {}
        other => panic!("a held ask to the parked worker: {other:?}"),
    }
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { woken: true, prompt, .. } => {
            assert_eq!(prompt, Some(StartPrompt::HeldForPerson));
        }
        other => panic!("start_agent over a held ask: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "W woken again", || {
        c.board().live_agent(w).is_some_and(|s| s.id == ws && s.state != SessionState::Sleeping)
    });
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    c.await_state(ws, "idle after the second wake", |s| matches!(s, SessionState::Idle { .. }));
    let held_count = || std::fs::read_to_string(&got).unwrap_or_default().matches(held).count();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(held_count(), 0, "held words wait for the person");
    assert!(matches!(c.request(Command::SendQueuedAsk { ticket: w }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the person's send to reach W", || {
        held_count() > 0
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(held_count(), 1, "once");
}

/// T-936: a worker the crown started whose turn ends on an error wakes the
/// crown once, the reason as the board saw it in the line, and its
/// `get_ticket` says why. A failure is no delivery and no finished turn. A
/// repeated hook wakes nobody; a failure read again refines the reason of
/// the wake still owed; a wake owed across a restart keeps the reason it
/// was given, after the worker moved on; and a later failure of its own
/// wakes the crown again.
#[test]
fn a_failed_turn_wakes_the_crown_once_with_why() {
    let Some(h) = Harness::boot_with_env(
        "crown_failed",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_failed");
    let got = h.dir.join("got.txt");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let fail = |c: &mut TestClient, sid: uuid::Uuid, class: &str, reason: FailReason| {
        let body = json!({ "error": class, "last_assistant_message": "API Error" }).to_string();
        hook_send_with(&hook_sock, &sid.to_string(), "StopFailure", Some(class), &body);
        c.await_state(sid, "failed", |s| *s == SessionState::Failed { reason });
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's worktree start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kw} \"mesimon-probe-936 worker\" ");
    let lines = || -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&worker))
            .map(str::to_string)
            .collect()
    };
    let failed_line = |reason: &str| format!("{worker}ended its turn on an error ({reason}");

    // ---- 1. a failure wakes the crown, with the reason -----------------------
    start(&mut c, ws);
    fail(&mut c, ws, "server_error", FailReason::Server);
    wait_until(std::time::Duration::from_secs(10), "the failure's wake on the crown", || {
        lines().iter().any(|l| l.contains(&failed_line("server error")))
    });
    let state = read(&mut c, sa, &kw).unwrap().state.expect("W's state");
    assert_eq!((state.state.as_str(), state.failed.as_deref()), ("failed", Some("server error")));
    start(&mut c, sa);
    stop(&mut c, sa);
    // The same hook again changes nothing.
    fail(&mut c, ws, "server_error", FailReason::Server);

    // ---- 2. owed while the crown works: refined, then across a restart -----
    start(&mut c, sa);
    start(&mut c, ws);
    fail(&mut c, ws, "unknown", FailReason::Unknown);
    fail(&mut c, ws, "invalid_request", FailReason::InvalidRequest);
    drop(c);
    h.restart();
    let mut c = h.client("crown_failed_2");
    // The worker moves on before the wake goes: the line says what it failed on.
    start(&mut c, ws);
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(15), "the owed failure's wake", || {
        lines().len() >= 2
    });
    let second = lines()[1].clone();
    assert!(second.contains(&failed_line("invalid request")), "{second}");
    assert!(second.contains("after a restart"), "{second}");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 3. a later failure of its own wakes the crown again -----------------
    fail(&mut c, ws, "max_output_tokens", FailReason::MaxOutputTokens);
    wait_until(std::time::Duration::from_secs(10), "the second failure's wake", || {
        lines().len() >= 3
    });
    let all = lines();
    assert!(all[2].contains(&failed_line("max output tokens")), "{all:?}");
    // One line per failure, and none says the worker delivered or finished.
    assert_eq!(all.len(), 3, "{all:?}");
    assert!(all.iter().all(|l| l.contains("ended its turn on an error")), "{all:?}");
}

/// T-936: what the crown was told of a parked worker in DONE is not news
/// after a restart, with the same crown or a new one. W1 delivered and
/// merged and was told both; W2 finished a turn and was told, then ended
/// another with background tasks running (an edge into work that no turn's
/// end closed, as a Codex seat's lost observation is), and was parked. A
/// bare restart says nothing of either, nor does a handover and a restart
/// after it; a worker that delivers and merges after the handover still
/// wakes the crown worn now. A turn genuinely ended in the restart's gap
/// is heard (`a_worker_mid_turn_across_a_restart_delivers_at_its_real_end`).
#[test]
fn a_told_worker_parked_in_done_is_not_news_after_a_restart() {
    let Some(h) = Harness::boot_with_env(
        "crown_parked_news",
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
    let mut c = h.client("crown_parked_news");
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
    let a = c.create_ticket("the first crown");
    let b = c.create_ticket("the second crown");
    let w1 = c.create_ticket("mesimon-probe-936 merged and parked");
    let w2 = c.create_ticket("mesimon-probe-936 finished and parked");
    let w3 = c.create_ticket("mesimon-probe-936 merged after the handover");
    let (k1, k2, k3) = (key_of(&mut c, w1), key_of(&mut c, w2), key_of(&mut c, w3));
    let sa = c.spawn_claude(a);
    let sb = c.spawn_claude(b);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let start_agent = |c: &mut TestClient, key: &str, workspace: &str| {
        let v = read(c, sa, key).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: key.into(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some(workspace.into()),
                expect_secs: None,
            },
        ) {
            Response::AgentStarted { .. } => {}
            other => panic!("start_agent {key}: {other:?}"),
        }
    };
    start_agent(&mut c, &k1, "worktree");
    start_agent(&mut c, &k2, "worktree");
    start_agent(&mut c, &k3, "worktree");
    let p1 = std::path::PathBuf::from(wait_attached(&mut c, w1).path.expect("a path"));
    wait_attached(&mut c, w2);
    let p3 = std::path::PathBuf::from(wait_attached(&mut c, w3).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the three starts to land", || {
        [w1, w2, w3].iter().all(|w| c.board().live_agent(*w).is_some())
    });
    let seat = |c: &mut TestClient, w: ulid::Ulid| c.board().live_agent(w).unwrap().id;
    let (ws1, ws2, ws3) = (seat(&mut c, w1), seat(&mut c, w2), seat(&mut c, w3));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = |key: &str, title: &str| format!("{key} \"mesimon-probe-936 {title}\" ");
    let (n1, n2, n3) = (
        worker(&k1, "merged and parked"),
        worker(&k2, "finished and parked"),
        worker(&k3, "merged after the handover"),
    );
    // A delivery and its merge, each a line the crown takes in a turn.
    let deliver_and_merge =
        |c: &mut TestClient, crown, key: &str, path: &std::path::Path, sid, name: &str| {
            // On the base as it stands, so the merge is a fast-forward.
            git(path, &["rebase", "-q", "main"]);
            commit(path, &format!("{key}.txt"));
            wait_until(std::time::Duration::from_secs(15), "the flags to read it ahead", || {
                read(c, crown, key).unwrap().merge_state.as_deref() == Some("ahead")
            });
            start(c, sid);
            stop(c, sid);
            wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
                lines_with(&format!("{name}delivered")).len() == 1
            });
            start(c, crown);
            stop(c, crown);
            let branch = read(c, crown, key).unwrap().branch.expect("a branch");
            git(&h.repo, &["merge", "--ff-only", "-q", &branch]);
            wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
                lines_with(&format!("{name}merged")).len() == 1
            });
            start(c, crown);
            stop(c, crown);
        };

    // ---- 1. W1 delivered and merged, told; parked and in DONE --------------
    deliver_and_merge(&mut c, sa, &k1, &p1, ws1, &n1);
    // ---- 2. W2 finished, told; then work no turn's end closed; parked -------
    start(&mut c, ws2);
    stop(&mut c, ws2);
    wait_until(std::time::Duration::from_secs(10), "W2's finish", || {
        lines_with(&format!("{n2}finished its turn")).len() == 1
    });
    start(&mut c, sa);
    stop(&mut c, sa);
    start(&mut c, ws2);
    hook_send(
        &hook_sock,
        &ws2.to_string(),
        "PostToolUse",
        r#"{"tool_name":"Bash","tool_response":{"stdout":"","stderr":"","backgroundTaskId":"loop1"}}"#,
    );
    hook_send(
        &hook_sock,
        &ws2.to_string(),
        "Stop",
        r#"{"stop_hook_active":false,"background_tasks":[{"id":"loop1","type":"shell","status":"running"}]}"#,
    );
    let background =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Background };
    c.await_state(ws2, "idle with a task", |s| *s == background);
    for (w, sid) in [(w1, ws1), (w2, ws2)] {
        assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
        c.await_state(sid, "parked", |s| *s == SessionState::Sleeping);
        assert!(matches!(
            c.request(Command::MoveTicket { id: w, column: "DONE".into(), before: None }),
            Response::Ok
        ));
    }
    let told = (lines_with(&n1).len(), lines_with(&n2).len());
    assert_eq!(told, (2, 1), "{}", std::fs::read_to_string(&got).unwrap_or_default());

    // ---- 3. a bare restart: nothing of either -------------------------------
    drop(c);
    h.restart();
    let mut c = h.client("crown_parked_news_2");
    // A fence the restart's looks come before: the crown's own turn ends.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    start(&mut c, sa);
    stop(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert_eq!(
        (lines_with(&n1).len(), lines_with(&n2).len()),
        told,
        "after a bare restart: {}",
        std::fs::read_to_string(&got).unwrap_or_default()
    );

    // ---- 4. a handover and a restart: nothing of either, W3 still heard -----
    assert!(matches!(c.request(Command::CrownTicket { id: b }), Response::Ok));
    start(&mut c, sb);
    stop(&mut c, sb);
    drop(c);
    h.restart();
    let mut c = h.client("crown_parked_news_3");
    start(&mut c, sb);
    stop(&mut c, sb);
    deliver_and_merge(&mut c, sb, &k3, &p3, ws3, &n3);
    assert_eq!(
        (lines_with(&n1).len(), lines_with(&n2).len()),
        told,
        "after a handover and a restart: {}",
        std::fs::read_to_string(&got).unwrap_or_default()
    );
}

/// What a start_agent wake's receipt says waits for the agent (T-920), read
/// off the daemon's own state, and that whatever waited goes exactly once.
/// Queued: the crown's ask to a parked worker on the shared checkout waits
/// behind the crown's own running turn, the wake moves it to the new pane,
/// and it goes once that turn ends. A resume parked on its worktree's
/// rebuild: with nothing waiting it reads none and nothing goes; with the
/// crown's words already riding that resume it reads queued, and a second
/// start adds no second paste.
#[test]
fn a_wake_s_receipt_names_what_waits_and_it_goes_once() {
    let Some(h) = Harness::boot_with_env(
        "crown_wake_words",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_wake_words");
    let got = h.dir.join("got.txt");
    let count =
        |probe: &str| std::fs::read_to_string(&got).unwrap_or_default().matches(probe).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"source":"resume"}"#);
        c.await_state(sid, "up after the wake", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("checkout worker");
    let w2 = c.create_ticket("worktree worker");
    let (kw, kw2) = (key_of(&mut c, w), key_of(&mut c, w2));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        )
    };
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
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { held_for_person, road, wake, .. } => {
                (held_for_person, road, wake)
            }
            other => panic!("ask_agent {key}: {other:?}"),
        }
    };
    let park = |c: &mut TestClient, key: &str, sid: uuid::Uuid| {
        let v = read(c, sa, key).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: key.into(), seen: v.seen },
        ) {
            Response::AgentTicket { .. } => {}
            other => panic!("sleep_agent {key}: {other:?}"),
        }
        c.await_state(sid, "parked", |s| *s == SessionState::Sleeping);
    };

    // ---- queued: the crown's ask waits behind the crown's own turn ---------
    match start_agent(&mut c, &kw, "shared_checkout") {
        Response::AgentStarted { prompt, .. } => assert_eq!(prompt, Some(StartPrompt::Brief)),
        other => panic!("start W: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W's seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    park(&mut c, &kw, ws);
    // The crown's turn runs on the checkout W shares, so the queue holds W's
    // words: they would wake W, and a wake waits for a quiet checkout.
    start(&mut c, sa);
    let probe = "mesimon-probe-922 the queued words";
    let (held, road, wake) = ask(&mut c, &kw, probe);
    assert!(!held, "the board lets the crown send to its own worker");
    assert_eq!(road, Some(AskRoad::Queued));
    assert_eq!(wake, Some(AskWake::WhenSent), "still asleep: the checkout is busy");
    settle();
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Sleeping);
    match start_agent(&mut c, &kw, "shared_checkout") {
        Response::AgentStarted { session_started: true, woken: true, prompt, .. } => {
            assert_eq!(prompt, Some(StartPrompt::Queued), "the crown's words wait for it");
        }
        other => panic!("start_agent over a queued ask: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "W woken", || {
        c.board().live_agent(w).is_some_and(|s| s.id == ws && s.state.has_pane())
    });
    up(&mut c, ws);
    settle();
    assert_eq!(count(probe), 0, "the crown's turn still holds the checkout");
    let r = pending_of(&mut c, Some(w)).into_iter().find(|p| p.is_queued_ask());
    assert!(r.is_some_and(|r| r.action == PendingAction::Ask), "aimed at W's pane now");
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(10), "the queued words to reach W", || {
        count(probe) > 0
    });
    settle();
    assert_eq!(count(probe), 1, "once");

    // ---- a resume parked on its worktree's rebuild -------------------------
    match start_agent(&mut c, &kw2, "worktree") {
        Response::AgentStarted { session_started: false, prompt, .. } => {
            assert_eq!(prompt, Some(StartPrompt::Brief), "its brief, once the cut lands");
        }
        other => panic!("start W2: {other:?}"),
    }
    let tree = std::path::PathBuf::from(wait_attached(&mut c, w2).path.expect("a path"));
    let branch = wt_of(&mut c, w2).unwrap().branch;
    wait_until(std::time::Duration::from_secs(15), "W2's start to land", || {
        c.board().live_agent(w2).is_some_and(|s| s.state.has_pane())
    });
    let ws2 = c.board().live_agent(w2).unwrap().id;
    std::fs::write(tree.join("w2.txt"), "w2\n").unwrap();
    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "w2"]);
    git(&repo, &["merge", "-q", "--squash", &branch]);
    git(&repo, &["commit", "-qm", "w2 (#9)"]);
    wait_until(std::time::Duration::from_secs(20), "the board to read the squash", || {
        wt_of(&mut c, w2).is_some_and(|wt| wt.merged)
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws2);
    stop(&mut c, ws2);
    // Parked and archived, the landed tree is reclaimed and the branch kept,
    // so the next wake rebuilds the tree first (T-278).
    let reclaim = |c: &mut TestClient| {
        park(c, &kw2, ws2);
        assert!(matches!(c.request(Command::ArchiveTicket { id: w2 }), Response::Ok));
        wait_until(std::time::Duration::from_secs(20), "the archive to reclaim W2's tree", || {
            !tree.exists() && wt_of(c, w2).is_some_and(|wt| wt.status == "evicted")
        });
        assert!(matches!(c.request(Command::UnarchiveTicket { id: w2 }), Response::Ok));
    };

    // Nothing waiting: none, and nothing goes once it is up. Counted in W2's
    // own pane (T-937): the crown's wake for W2's turn writes `got.txt` too,
    // and it can go out in this window, held 10 s behind the crown's
    // unacknowledged wake for W, or pasted late under load.
    reclaim(&mut c);
    let got_w2 = h.dir.join(format!("got-{kw2}.txt"));
    let lines = || std::fs::read_to_string(&got_w2).unwrap_or_default().lines().count();
    match start_agent(&mut c, &kw2, "worktree") {
        Response::AgentStarted { session_started: false, woken: true, prompt, .. } => {
            assert_eq!(prompt, Some(StartPrompt::None));
        }
        other => panic!("a wake behind a rebuild: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(20), "W2 up in its rebuilt tree", || {
        c.board().live_agent(w2).is_some_and(|s| s.id == ws2 && s.state.has_pane())
    });
    let before = lines();
    up(&mut c, ws2);
    settle();
    assert_eq!(lines(), before, "nothing was sent");

    // The crown's words already ride the parked resume: queued, and a second
    // start neither drops them nor sends them twice.
    reclaim(&mut c);
    let probe = "mesimon-probe-923 words riding the rebuild";
    let (held, _, wake) = ask(&mut c, &kw2, probe);
    assert!(!held);
    assert_eq!(wake, Some(AskWake::Woken), "the words woke it, behind the rebuild");
    match start_agent(&mut c, &kw2, "worktree") {
        Response::AgentStarted { session_started: false, woken: true, prompt, .. } => {
            assert_eq!(prompt, Some(StartPrompt::Queued), "the words on the resume");
        }
        other => panic!("a second start behind the rebuild: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(20), "W2 up again", || {
        c.board().live_agent(w2).is_some_and(|s| s.id == ws2 && s.state.has_pane())
    });
    up(&mut c, ws2);
    wait_until(std::time::Duration::from_secs(10), "the words to reach W2", || count(probe) > 0);
    settle();
    assert_eq!(count(probe), 1, "once");
    let w2_got = std::fs::read_to_string(&got_w2).unwrap_or_default();
    assert_eq!(w2_got.matches(probe).count(), 1, "in W2's own pane: {w2_got}");
}

/// The crown answers a Codex worker's question (T-846). The worker's runtime
/// is the synthetic one (`fake_codex_runtime.py`), which publishes the
/// request the real app-server would and paints Codex 0.163.0's dialog as
/// captured: the question wakes the crown, `get_ticket` lists it with its
/// options and `answerable`, `answer_agent` presses Enter on the chosen row,
/// the receipt reads `answered` only once the request leaves the snapshot,
/// and the turn that took the answer wakes the crown at its end. Words and
/// a batch are refused for a codex in words.
#[test]
fn the_crown_answers_a_codex_worker_s_question() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    // The fake Codex runtime lives beside the Claude stub, named to the
    // daemon on a restart (its env is read at spawn).
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_codex");
    let got = h.dir.join("got.txt");
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
    let asking = |s: &SessionState| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    };
    let control = |id: uuid::Uuid, value: serde_json::Value| {
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    let screen = |row: usize| {
        let mark = |n: usize| if n == row { "›" } else { " " };
        format!(
            "  Question 1/1 (1 unanswered)\r\n  Which auth provider?\r\n\r\n  {} 1. Okta        \
             Hosted SSO.\r\n  {} 2. Auth0       Hosted too.\r\n    3. None of the above  \
             Optionally, add details in notes (tab)\r\n\r\n  tab to add notes | enter to submit \
             answer | esc to interrupt\r\n",
            mark(1),
            mark(2)
        )
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-846 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(20), "W's Codex seat", || {
        c.board().live_agent(w).is_some_and(|s| s.kind == SessionKind::Codex)
    });
    let ws = c.board().live_agent(w).unwrap().id;
    c.await_state(ws, "the synthetic Codex at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    wait_until(std::time::Duration::from_secs(10), "W's prefill", || {
        c.board().sessions.iter().any(|s| s.id == ws && !s.pending_prefill)
    });
    let input = h.dir.join(format!("{ws}.input"));
    let keys = || std::fs::read(&input).unwrap_or_default();

    // ---- the worker asks: the crown wakes, and reads the question ----------
    let question = serde_json::json!({"id":"\"q7\"","kind":"question","questions":[{
        "question":"Which auth provider?","header":"Auth","secret":false,
        "options":[{"label":"Okta","description":"Hosted SSO."},
                   {"label":"Auth0","description":"Hosted too."}]
    }]});
    control(
        ws,
        json!({"state":{"state":"requires_action","reason":"question"},"turn_id":"ask-turn",
               "hold":true,"ack":false,"request":question,"screen":screen(1),"cursor":null}),
    );
    c.await_state(ws, "asking", asking);
    let wake = format!("{kw} \"mesimon-probe-846 worker\" asks a question");
    wait_until(std::time::Duration::from_secs(10), "the question's wake on the crown", || {
        landed(&wake)
    });
    assert!(!typed().contains("Which auth provider"), "the question's words never ride a wake");
    start(&mut c, sa);
    let needs = read(&mut c, sa, &kw).unwrap().needs_you.expect("the question");
    assert_eq!(needs.request.as_deref(), Some("\"q7\""));
    assert_eq!(needs.questions[0].text, "Which auth provider?");
    assert_eq!(needs.questions[0].options, vec!["Okta".to_string(), "Auth0".to_string()]);
    assert_eq!(needs.answerable, Some(true));

    // ---- refusals in words: words and batches are a person's on a codex ---
    let answer_cmd = |c: &mut TestClient, index, text: Option<&str>| {
        let v = read(c, sa, &kw).unwrap();
        Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: v.seen,
            request: "\"q7\"".into(),
            index,
            text: text.map(str::to_string),
            answers: None,
        }
    };
    let cmd = answer_cmd(&mut c, None, Some("Something else"));
    match c.send(Principal::Agent { session: sa }, cmd) {
        Response::Err { message } => {
            assert!(message.contains("codex") && message.contains("index"), "{message}")
        }
        other => panic!("words into a codex: {other:?}"),
    }

    // ---- answered: Enter on the row the cursor stands on ------------------
    let before = keys().len();
    let cmd = answer_cmd(&mut c, Some(0), None);
    let sock = h.paths.orch_sock();
    let call = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    wait_until(std::time::Duration::from_secs(5), "the answer's Enter in W's pane", || {
        // The pane's tty reads tmux's `Enter` as a newline under `ICRNL`.
        keys()[before..].iter().any(|k| matches!(k, b'\n' | b'\r'))
    });
    assert!(!keys()[before..].windows(3).any(|k| k == b"\x1b[B"), "Okta is the cursor's row");
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(!call.is_finished(), "the receipt waits for the request to resolve");
    control(
        ws,
        json!({"state":{"state":"running"},"turn_id":"ask-turn","hold":true,"ack":false,
               "screen":"• Working (1s • esc to interrupt)\r\n\r\n› Ask Codex to do anything\r\n",
               "cursor":[2,2]}),
    );
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
    assert_eq!(said_on(&mut c, w).as_deref(), Some("Okta"));
    let line = format!("answered by {ka}: Okta");
    let detail = |c: &mut TestClient| {
        c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.detail.clone())
    };
    c.await_state(ws, "running on the answer", |s| *s == SessionState::Running);
    assert_eq!(detail(&mut c).as_deref(), Some(line.as_str()));

    // ---- the turn that took the answer wakes the crown when it ends --------
    stop(&mut c, sa);
    control(
        ws,
        json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"ask-turn",
               "hold":false,"ack":false,"screen":"Synthetic provider fixture\r\n› ","cursor":[2,1]}),
    );
    c.await_state(ws, "W done", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
    });
    wait_until(std::time::Duration::from_secs(10), "the answer's turn waking the crown", || {
        ask_ends(&format!("{kw} \"mesimon-probe-846 worker\"")).iter().any(|l| landed(l))
    });
}

/// T-936: a Codex seat is polled, so a failed turn is told by its turn id,
/// never by a state change. On the synthetic runtime: turn A fails; turn B
/// fails into the same state with no poll reading `Running` between them,
/// which changes no state, and wakes the crown a second time; turn B read
/// again wakes nobody, nor does its reason refined; and a new session that
/// reports a turn of the same id fails a turn of its own.
#[test]
fn a_codex_worker_s_failed_turns_are_told_by_turn() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex_failed",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_codex_failed");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let control = |id: uuid::Uuid, value: serde_json::Value| {
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 codex worker");
    let kw = key_of(&mut c, w);
    let failed_wakes = || -> usize {
        std::fs::read_to_string(&feed)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|l| {
                l["kind"] == "crown_wake" && l["cause"] == "failed" && l["worker"] == w.to_string()
            })
            .count()
    };
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
    let start = |c: &mut TestClient| -> uuid::Uuid {
        let v = read(c, sa, &kw).unwrap();
        assert!(matches!(
            c.send(
                Principal::Agent { session: sa },
                Command::AgentStartTicket {
                    key: kw.clone(),
                    seen: v.seen,
                    plan: false,
                    tier: None,
                    workspace: Some("worktree".into()),
                    expect_secs: None,
                },
            ),
            Response::AgentStarted { .. }
        ));
        wait_until(std::time::Duration::from_secs(20), "W's Codex seat", || {
            c.board().live_agent(w).is_some_and(|s| s.kind == SessionKind::Codex)
        });
        let ws = c.board().live_agent(w).unwrap().id;
        c.await_state(ws, "the synthetic Codex at its composer", |s| {
            matches!(s, SessionState::Idle { .. })
        });
        ws
    };
    let fail = |id: uuid::Uuid, turn: &str, reason: &str, screen: &str| {
        control(
            id,
            json!({"state":{"state":"failed","reason":reason},"turn_id":turn,
                   "screen":screen,"cursor":null}),
        );
    };
    let server = SessionState::Failed { reason: FailReason::Server };
    let ws = start(&mut c);

    // ---- 1. two turns fail, no poll reading Running between them -----------
    fail(ws, "turn-a", "server", "A failed");
    c.await_state(ws, "turn A failed", |s| *s == server);
    wait_until(std::time::Duration::from_secs(10), "turn A's wake", || failed_wakes() == 1);
    fail(ws, "turn-b", "server", "B failed");
    wait_until(std::time::Duration::from_secs(10), "turn B's wake", || failed_wakes() == 2);
    // ---- 2. turn B read again, and its reason refined: nobody woken ---------
    fail(ws, "turn-b", "server", "B failed, drawn again");
    fail(ws, "turn-b", "invalid_request", "B failed, read again");
    c.await_state(ws, "B refined", |s| {
        *s == SessionState::Failed { reason: FailReason::InvalidRequest }
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(failed_wakes(), 2, "turn B read again is no new failure");

    // ---- 3. a new session's turn of the same id is a failure of its own ----
    assert!(matches!(c.request(Command::KillSession { id: ws }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "W's seat gone", || {
        c.board().live_agent(w).is_none()
    });
    let ws2 = start(&mut c);
    assert_ne!(ws2, ws, "another session");
    fail(ws2, "turn-b", "server", "B failed in the new session");
    wait_until(std::time::Duration::from_secs(10), "the new session's wake", || {
        failed_wakes() == 3
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(failed_wakes(), 3);
}

/// T-936, Astra's read of the park's close: a Codex seat whose pane died
/// keeps `codex_stopping`, which reads as working, and that edge kept a
/// turn open where the seat had no pane. On the synthetic runtime: a
/// worker's finished turn is told; its observation is lost (`Unknown`,
/// working for a Codex seat); its runtime dies, so the pane-died hook
/// lands it `Exited` with `codex_stopping` set, and the ledger keeps no
/// turn open for it (the old rule kept it, read back at a restart and by
/// the next idle as a finished turn); after a restart the crown hears
/// nothing more of it. A turn that really ended in the gap is
/// still heard (`a_worker_mid_turn_across_a_restart_delivers_at_its_real_end`).
#[test]
fn a_codex_seat_whose_pane_died_has_no_turn_open_after_a_restart() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex_died",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_codex_died");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let control = |id: uuid::Uuid, value: serde_json::Value| {
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 codex worker");
    let kw = key_of(&mut c, w);
    let wakes_since = |at: u64| -> Vec<String> {
        std::fs::read_to_string(&feed)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|l| {
                l["kind"] == "crown_wake"
                    && l["worker"] == w.to_string()
                    && l["at_ms"].as_u64().is_some_and(|t| t >= at)
            })
            .map(|l| l["cause"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(20), "W's Codex seat", || {
        c.board().live_agent(w).is_some_and(|s| s.kind == SessionKind::Codex)
    });
    let ws = c.board().live_agent(w).unwrap().id;
    c.await_state(ws, "the synthetic Codex at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });

    // ---- 1. a turn finished, and told ---------------------------------------
    control(ws, json!({"state":{"state":"running"},"turn_id":"turn-1"}));
    c.await_state(ws, "working", |s| *s == SessionState::Running);
    control(ws, json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"turn-1"}));
    wait_until(std::time::Duration::from_secs(10), "the finish told", || {
        wakes_since(0) == ["finished"]
    });
    // ---- 2. its observation lost, then its pane dead ------------------------
    // What the crown's ledger keeps open: `crown.json`'s `turns_open`.
    let ledger = h.paths.state_dir.join("crown.json");
    let open = || {
        std::fs::read_to_string(&ledger)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["turns_open"].as_array().cloned())
            .unwrap_or_default()
            .contains(&json!(w.to_string()))
    };
    control(ws, json!({"state":{"state":"unknown"},"turn_id":"turn-1"}));
    c.await_state(ws, "observation lost", |s| matches!(s, SessionState::Unknown { .. }));
    wait_until(std::time::Duration::from_secs(10), "a lost observation opens a turn", open);
    let sid = c.board().sessions.iter().find(|s| s.id == ws).unwrap().sid16();
    let pid = tmux(&tmux_sock)
        .args(["display-message", "-p", "-t", &sid, "#{pane_pid}"])
        .output()
        .unwrap();
    let pid = String::from_utf8_lossy(&pid.stdout).trim().to_string();
    assert!(std::process::Command::new("kill").args(["-9", &pid]).status().unwrap().success());
    c.await_state(ws, "its pane dead", |s| matches!(s, SessionState::Exited { .. }));
    assert!(c.board().sessions.iter().any(|s| s.id == ws && s.codex_stopping));
    wait_until(std::time::Duration::from_secs(10), "no turn open without a pane", || !open());

    // ---- 3. a restart: nothing more of it ------------------------------------
    drop(c);
    let restarted =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
            as u64;
    h.restart();
    let _c = h.client("crown_codex_died_2");
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert_eq!(wakes_since(restarted), Vec::<String>::new(), "no line after the restart");
}

/// T-936, the second read: an explicit kill drops the open turn only for
/// the ticket's agent seat. Killing the ticket's shell under its running
/// agent leaves that agent's turn open, so its `Stop` is a finished turn.
#[test]
fn killing_a_shell_keeps_the_running_agent_s_turn() {
    kill_beside_a_running_agent("crown_kill_shell", false);
}

/// The same for an old agent's corpse dismissed while its replacement
/// works: the corpse holds no seat, so the replacement's turn stays open.
#[test]
fn dismissing_an_old_corpse_keeps_the_replacement_s_turn() {
    kill_beside_a_running_agent("crown_kill_corpse", true);
}

fn kill_beside_a_running_agent(name: &str, corpse: bool) {
    let Some(h) = Harness::boot_with_env(
        name,
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
    let got = h.dir.join("got.txt");
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let start_agent = |c: &mut TestClient| -> uuid::Uuid {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ) {
            Response::AgentStarted { .. } => {}
            other => panic!("start_agent: {other:?}"),
        }
        wait_until(std::time::Duration::from_secs(15), "W's seat", || {
            c.board().live_agent(w).is_some()
        });
        c.board().live_agent(w).unwrap().id
    };
    let mut ws = start_agent(&mut c);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let other = if corpse {
        // The first agent dies, and a replacement takes the seat.
        let old = ws;
        assert!(matches!(c.request(Command::KillSession { id: old }), Response::Ok));
        c.await_state(old, "the old agent dead", |s| matches!(s, SessionState::Exited { .. }));
        wait_until(std::time::Duration::from_secs(10), "the seat free", || {
            c.board().live_agent(w).is_none()
        });
        ws = start_agent(&mut c);
        assert_ne!(ws, old);
        std::thread::sleep(std::time::Duration::from_millis(500));
        old
    } else {
        match c.request(Command::SpawnSession {
            ticket: w,
            kind: SessionKind::Bash,
            submit_prompt: false,
            plan: false,
        }) {
            Response::Spawned { id, .. } => id,
            other => panic!("the ticket's shell: {other:?}"),
        }
    };
    // The agent is mid-turn when the other record is killed or dismissed.
    start(&mut c, ws);
    assert!(matches!(c.request(Command::KillSession { id: other }), Response::Ok));
    stop(&mut c, ws);
    let finished = format!("{kw} \"mesimon-probe-936 worker\" finished its turn");
    wait_until(std::time::Duration::from_secs(10), "the agent's finished turn", || {
        std::fs::read_to_string(&got).unwrap_or_default().contains(&finished)
    });
}

/// T-936, the second read: a crown's wake down its mod's `submit` is told
/// only on that frame's `entered`, whatever state the crown reads when the
/// refusal comes. Refused while it reads `EndTurn` (no Esc inferred
/// first), the wake is owed again with the hand up, and goes by the
/// guarded paste road into the empty box, nobody there: told only then.
#[test]
fn a_crown_wake_refused_at_end_turn_is_owed_until_it_goes() {
    refused_crown_wake("crown_refused_idle", false);
}

/// The same refusal landing after the take, while the crown reads
/// `Unknown`: the submit is out and taken, so told (T-990), a prompt goes
/// into the crown (which says nothing more of the wake), and the daemon
/// restarts mid-turn, keeping the taken wake for its frame. That frame's
/// refusal then lands, matched while the crown reads `Unknown`: the wake
/// is owed again, and the next delivery, typed, settles it, `after a
/// restart`.
#[test]
fn a_crown_wake_refused_while_unknown_is_owed_until_it_goes() {
    refused_crown_wake("crown_refused_unknown", true);
}

fn refused_crown_wake(name: &str, restart: bool) {
    let Some(h) = Harness::boot_bare(
        name,
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
    let got = h.dir.join("got.txt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let end_turn = SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| *s == end_turn);
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    // The plain start typed the title into the crown's box (T-961). This
    // stub's box is its tty's echo, so the title shows there only when the
    // stub painted before the keys came: a fast exec (Linux) shows it, and
    // the wake is rightly held behind it; a slow one (macOS) paints over
    // the echo, and the wake went onto a title nobody saw (T-1019). The
    // title is sent first, so the box is empty for the wake on either.
    wait_composer(&h, sa);
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    assert!(tmux(&h.paths.tmux_sock())
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(10), "the title out of the box", || {
        read_file(&got).lines().any(|l| l.trim() == "coordinate")
    });
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw1.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { session_started: true, .. } => {}
        other => panic!("start_agent {kw1}: {other:?}"),
    }
    let ws1 = c.board().live_agent(w1).expect("W1 holds a seat").id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let frames = || read_file(&h.dir.join(format!("mod-{sa}.ndjson")));
    let submitted = || {
        frames()
            .lines()
            .filter(|l| l.contains("\"submit\"") && l.contains("raised its hand"))
            .count()
    };
    let raised = format!("{kw1} \"mesimon-probe-936 worker\" raised its hand");
    let pasted = || read_file(&got).lines().filter(|l| l.contains(&raised)).count();
    let hand_up = |c: &mut TestClient| c.board().ticket(w1).unwrap().raised.is_some();
    let at = |cmd: &str| -> Option<u64> {
        read_file(&feed)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|l| l["cmd"] == cmd)
            .and_then(|l| l["at_ms"].as_u64())
    };
    std::fs::write(h.dir.join("mod-stopped-submit"), "").unwrap();
    if restart {
        std::fs::write(h.dir.join("mod-hold-submit"), "").unwrap();
    }
    match c.send(
        Principal::Agent { session: ws1 },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the wake down the crown's mod", || {
        submitted() == 1
    });
    if !restart {
        // Refused at `EndTurn`: owed again, then typed into the empty box.
        wait_until(std::time::Duration::from_secs(10), "the refusal", || {
            at("crown_wake_refused").is_some()
        });
        wait_until_says(
            std::time::Duration::from_secs(10),
            "the wake typed",
            || (pasted(), pending_of(&mut c, Some(a))),
            |(n, _)| *n == 1,
        );
        wait_until(std::time::Duration::from_secs(5), "the telling in the feed", || {
            at("crown_wake_sent").is_some()
        });
        // A take before the refusal tells it there too (T-990); the typed
        // delivery is told after the refusal either way.
        let last_sent = || {
            read_file(&feed)
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|l| l["cmd"] == "crown_wake_sent")
                .filter_map(|l| l["at_ms"].as_u64())
                .max()
        };
        let refused = at("crown_wake_refused").unwrap();
        wait_until(std::time::Duration::from_secs(5), "told after the refusal", || {
            last_sent().is_some_and(|sent| sent >= refused)
        });
        assert_eq!(submitted(), 1, "one submit; the second road is the guarded paste");
        wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
            !hand_up(&mut c)
        });
        return;
    }
    // The frame out, as the fake mod recorded it, and its take: told there
    // (T-990), as a paste is at the paste, and no delivery said in the
    // feed until its own receipt (T-936, A3); a prompt says nothing more.
    let first = frames()
        .lines()
        .filter(|l| l.contains("\"submit\"") && l.contains("raised its hand"))
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| v["id"].as_str().map(str::to_string))
        .expect("the submitted frame");
    let journal = h.paths.state_dir.join("daemon.log");
    let taken = format!("crown wake {first} in session {sa}: taken");
    wait_until(std::time::Duration::from_secs(10), "the take", || {
        read_file(&journal).contains(&taken)
    });
    wait_until(std::time::Duration::from_secs(5), "told at the take", || {
        !hand_up(&mut c) && at("crown_wake_sent").is_some()
    });
    start(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert!(at("crown_wake_delivered").is_none(), "no delivery read off a prompt");
    // A restart mid-turn: the crown reads `Unknown`, and the attempt is kept
    // for its frame, its seat's pane the same.
    drop(c);
    h.restart();
    let mut c = h.client(&format!("{name}_2"));
    let state = c.board().sessions.iter().find(|s| s.id == sa).map(|s| s.state.clone());
    assert!(matches!(state, Some(SessionState::Unknown { .. })), "{state:?}");
    // That frame's refusal lands while the crown reads `Unknown`: matched
    // to the taken wake the restart kept, and owed again.
    std::fs::write(h.dir.join("mod-release-submit"), "").unwrap();
    let refused = format!("crown wake {first} in session {sa}: refused, dropped");
    wait_until(std::time::Duration::from_secs(10), "the frame's refusal matched", || {
        read_file(&journal).contains(&refused)
    });
    let state = c.board().sessions.iter().find(|s| s.id == sa).map(|s| s.state.clone());
    assert!(matches!(state, Some(SessionState::Unknown { .. })), "refused while {state:?}");
    assert!(at("crown_wake_delivered").is_none(), "no delivery said");
    assert!(
        pending_of(&mut c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "the wake still owed"
    );
    // The crown's turn ends: one delivery, typed after the refusal,
    // settles both.
    for name in ["mod-hold-submit", "mod-release-submit", "mod-stopped-submit"] {
        std::fs::remove_file(h.dir.join(name)).unwrap();
    }
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(15), "the wake typed after the restart", || {
        pasted() == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c)
    });
    assert_eq!(submitted(), 1, "the refused frame's seat types the next");
    let typed = read_file(&got);
    let last = typed.lines().rfind(|l| l.contains("raised its hand")).unwrap_or_default();
    assert!(last.contains("after a restart"), "{last}");
}

/// T-936, the third read: a crown's submitted wake whose frame its bridge
/// never acknowledged (the bridge stopped) is waited for no longer than its
/// window (ten seconds). Retired `unconfirmed`, its wakes are owed again
/// with the hands as they were, and a second worker's wake that waited
/// behind it goes with them in the next attempt; a late report of the
/// retired frame settles nothing. T-990: a frame the mod took and never
/// answered is told at the take and sent again by nothing, and with no
/// crown turn between, the next worker's wake goes as its own prompt once
/// the first's owed entry is past its window.
#[test]
fn a_crown_wake_nobody_answers_is_retired_and_goes_again() {
    let Some(h) = Harness::boot_bare(
        "crown_unanswered",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_unanswered");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let journal = h.paths.state_dir.join("daemon.log");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let count = |cmd: &str| read_file(&feed).matches(&format!("\"cmd\":\"{cmd}\"")).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 first");
    let w2 = c.create_ticket("mesimon-probe-936 second");
    let (k1, k2) = (key_of(&mut c, w1), key_of(&mut c, w2));
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let mut seats = Vec::new();
    for (w, k) in [(w1, &k1), (w2, &k2)] {
        let v = read(&mut c, sa, k).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: k.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ) {
            Response::AgentStarted { .. } => {}
            other => panic!("start_agent {k}: {other:?}"),
        }
        wait_until(std::time::Duration::from_secs(15), "the worker's seat", || {
            c.board().live_agent(w).is_some()
        });
        let ws = c.board().live_agent(w).unwrap().id;
        up(ws);
        seats.push(ws);
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let raise = |c: &mut TestClient, ws: uuid::Uuid, why: &str| match c
        .send(Principal::Agent { session: ws }, Command::AgentRaiseHand { reason: why.into() })
    {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    };
    let hands =
        |c: &mut TestClient| [w1, w2].map(|w| c.board().ticket(w).unwrap().raised.is_some());
    let frames = || read_file(&h.dir.join(format!("mod-{sa}.ndjson")));
    // The submit frames the crown's mod took, with their words.
    let submits = || -> Vec<(String, String)> {
        frames()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "submit")
            .map(|v| (v["id"].as_str().unwrap_or("").to_string(), v.to_string()))
            .collect()
    };
    // The frames retired with `word`, in order.
    let retired = |word: &str| -> Vec<String> {
        read_file(&journal)
            .lines()
            .filter(|l| l.ends_with(word))
            .filter_map(|l| {
                l.split("crown wake ").nth(1).and_then(|r| r.split(' ').next()).map(str::to_string)
            })
            .collect()
    };
    let bridge_pid = std::fs::read_to_string(h.dir.join(format!("mod-bridge-{sa}.pid")))
        .unwrap()
        .trim()
        .to_string();
    let signal = |sig: &str| {
        assert!(std::process::Command::new("kill")
            .args([sig, &bridge_pid])
            .status()
            .unwrap()
            .success());
    };

    // ---- 1. never taken: the crown's bridge stopped ----------------------
    signal("-STOP");
    raise(&mut c, seats[0], "which key?");
    std::thread::sleep(std::time::Duration::from_millis(500));
    raise(&mut c, seats[1], "which branch?");
    // Handed to the stopped bridge's poll, its acknowledgement never came:
    // unconfirmed.
    wait_until(std::time::Duration::from_secs(20), "the unacknowledged attempt retired", || {
        retired("crown_wake_unconfirmed").len() == 1
    });
    let old = retired("crown_wake_unconfirmed")[0].clone();
    assert_eq!(hands(&mut c), [true, true], "hands as they were");
    assert_eq!(count("crown_wake_sent"), 0, "nothing told");
    assert!(c.board().sessions.iter().all(|s| s.id != sa || s.unsent.is_none()), "no copy kept");
    signal("-CONT");
    wait_until(std::time::Duration::from_secs(15), "the next attempt told", || {
        count("crown_wake_sent") == 1
    });
    let again = submits().into_iter().find(|(id, _)| *id != old).expect("a new attempt").1;
    assert!(again.contains(&k1) && again.contains(&k2), "both wakes in it: {again}");
    wait_until(std::time::Duration::from_secs(5), "both hands down", || {
        hands(&mut c) == [false, false]
    });
    // The retired frame's late report was no one's.
    let late = format!("mod submit {old} for session {sa}: entered, no owner");
    if submits().iter().any(|(id, _)| *id == old) {
        wait_until(std::time::Duration::from_secs(5), "the late report", || {
            read_file(&journal).contains(&late)
        });
    }

    // ---- 2. taken and never answered ---------------------------------------
    // The mod takes each frame and holds its report (T-990): the take is the
    // delivery, so nothing waits on the report, a crown turn or a clock.
    std::fs::write(h.dir.join("mod-hold-entered"), "").unwrap();
    let takes = || read_file(&journal).lines().filter(|l| l.ends_with(": taken")).count();
    let (taken0, frames0) = (takes(), submits().len());
    raise(&mut c, seats[0], "which key, again?");
    wait_until(std::time::Duration::from_secs(25), "the frame taken", || takes() == taken0 + 1);
    wait_until(std::time::Duration::from_secs(5), "told at the take", || {
        hands(&mut c) == [false, false] && count("crown_wake_sent") == 2
    });
    raise(&mut c, seats[1], "which branch, again?");
    wait_until(std::time::Duration::from_secs(25), "the next wake, no crown turn", || {
        takes() == taken0 + 2
    });
    wait_until(std::time::Duration::from_secs(5), "both told", || {
        hands(&mut c) == [false, false] && count("crown_wake_sent") == 3
    });
    assert_eq!(retired("crown_wake_unconfirmed").len(), 1, "no taken frame retired");
    let ours: Vec<(String, String)> = submits().into_iter().skip(frames0).collect();
    assert_eq!(ours.len(), 2, "nothing resent: {ours:?}");
    assert!(ours[0].1.contains(&k1) && !ours[0].1.contains(&k2), "{}", ours[0].1);
    assert!(ours[1].1.contains(&k2) && !ours[1].1.contains(&k1), "{}", ours[1].1);
    // The held reports land late: each its own frame's, settling nothing more.
    std::fs::write(h.dir.join("mod-release"), "").unwrap();
    wait_until(std::time::Duration::from_secs(15), "both receipts", || {
        let feed = read_file(&feed);
        ours.iter().all(|(id, _)| feed.contains(&format!("\"reason\":\"{id}\"")))
    });
    for (id, _) in &ours {
        let nobody = format!("mod submit {id} for session {sa}: entered, no owner");
        assert!(!read_file(&journal).contains(&nobody), "{id}'s receipt had its owner");
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(submits().len(), frames0 + 2, "nothing resent after the receipts");
    assert_eq!(count("crown_wake_sent"), 3, "nothing told again");
}

/// T-990: one worker's finished turn reaches the crown once, however long
/// its receipt takes. The crown reads idle, the wake goes, and the crown's
/// prompt and the mod's `entered` come long past the ten-second window, as
/// on 2026-10-25 under a writer turning at two to five seconds a message
/// (four frames retired `unconfirmed`, each later `entered`, five copies of
/// one event in the crown). On the mod pass the stand-in engine holds the
/// frame's `entered` (`mod-hold-entered`); the paste road's own ack waits
/// as long. Either way the crown's pane holds one copy and none retires.
#[test]
fn a_crown_wake_whose_receipt_is_late_is_sent_once() {
    let Some(h) = Harness::boot_with_env(
        "crown_wake_once",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_wake_once");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let journal = h.paths.state_dir.join("daemon.log");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let count = |cmd: &str| read_file(&feed).matches(&format!("\"cmd\":\"{cmd}\"")).count();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let mod_road = test_road() == "mod";

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-990 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    if mod_road {
        std::fs::write(h.dir.join("mod-hold-entered"), "").unwrap();
    }
    let got_crown = h.dir.join(format!("got-{ka}.txt"));
    let finished = format!("{kw} \"mesimon-probe-990 worker\" finished its turn");
    let copies = || read_file(&got_crown).matches(&finished).count();
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the finish in the crown's pane", || {
        copies() == 1
    });
    // The crown's receipt is late: twelve seconds before its prompt is
    // heard, then a turn, then the mod's `entered`, then its end.
    std::thread::sleep(std::time::Duration::from_secs(12));
    start(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_secs(2));
    if mod_road {
        // This frame's own receipt: taken and told before it, then its
        // `entered`, owned.
        let frame = read_file(&h.dir.join(format!("mod-{sa}.ndjson")))
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "submit")
            .filter(|v| v["text"].as_str().is_some_and(|t| t.contains(&finished)))
            .find_map(|v| v["id"].as_str().map(str::to_string))
            .expect("the finish's frame");
        assert!(read_file(&journal).contains(&format!("crown wake {frame} in session {sa}: taken")));
        std::fs::write(h.dir.join("mod-release"), "").unwrap();
        wait_until(std::time::Duration::from_secs(10), "the frame's own receipt", || {
            count("crown_wake_delivered") == 1
        });
        let nobody = format!("mod submit {frame} for session {sa}: entered, no owner");
        assert!(!read_file(&journal).contains(&nobody), "its receipt had its owner");
    }
    std::thread::sleep(std::time::Duration::from_secs(1));
    stop(&mut c, sa);
    // Room for a resend to land, were one owed.
    std::thread::sleep(std::time::Duration::from_secs(5));
    let retired: Vec<String> = read_file(&journal)
        .lines()
        .filter(|l| l.contains("crown wake ") && l.ends_with("crown_wake_unconfirmed"))
        .map(str::to_string)
        .collect();
    assert_eq!(copies(), 1, "one event, one prompt; retired: {retired:?}");
    assert!(retired.is_empty(), "nothing retired: {retired:?}");
    assert_eq!(count("crown_wake_sent"), 1, "told once");
}

/// T-936, the third read's heard-never: a wake that wakes a parked crown
/// parks its words on the launch, and the crown has heard nothing until
/// they are taken. Here a restart comes before they are, and a second one
/// after: the hand stays up and the wake owed through both, no copy of it
/// is left as unsent words, and one delivery into the crown's composer,
/// confirmed by its prompt, settles it.
#[test]
fn a_wake_parked_on_a_crown_s_launch_outlives_two_restarts() {
    let Some(h) = Harness::boot_with_env(
        "crown_parked_wake",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_parked_wake");
    let got = h.dir.join("got.txt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's seat", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let raised = format!("{kw} \"mesimon-probe-936 worker\" raised its hand");
    let typed = || read_file(&got).lines().filter(|l| l.contains(&raised)).count();
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let owed = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
    };
    let no_copy = |c: &mut TestClient| {
        c.board()
            .sessions
            .iter()
            .find(|s| s.id == sa)
            .is_none_or(|s| s.unsent.as_ref().is_none_or(|u| !u.text.contains(&raised)))
    };

    // ---- the crown parked; a hand wakes it with the words parked -----------
    assert!(matches!(c.request(Command::SleepSession { id: sa }), Response::Ok));
    c.await_state(sa, "the crown parked", |s| *s == SessionState::Sleeping);
    match c.send(
        Principal::Agent { session: ws },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the crown woken for it", || {
        c.board().sessions.iter().any(|s| s.id == sa && s.state != SessionState::Sleeping)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert!(hand_up(&mut c), "not told at the launch");
    assert!(!read_file(&feed).contains("\"crown_wake_sent\""), "nothing told");
    assert!(owed(&mut c), "the wake shown owed");

    // ---- two restarts before the words are taken -----------------------
    for n in 2..=3 {
        drop(c);
        h.restart();
        c = h.client(&format!("crown_parked_wake_{n}"));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(hand_up(&mut c), "the hand up after restart {n}");
        assert!(owed(&mut c), "the wake owed after restart {n}");
        assert!(no_copy(&mut c), "no copy of it kept unsent after restart {n}");
        assert_eq!(typed(), 0, "nothing typed yet");
    }

    // ---- the crown comes up: one delivery, confirmed, settles it ------------
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(15), "the wake typed into the crown", || {
        typed() == 1
    });
    start(&mut c, sa);
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c) && !owed(&mut c)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(typed(), 1, "once");
}

/// T-936, the fourth read: a duplicate report of a crown wake's frame,
/// after the wake was told and while its owed entry still waits for the
/// prompt, is still the wake's and does nothing. It never becomes an
/// ordinary delivery that clears the crown seat's unrelated unsent words.
#[test]
fn a_duplicate_wake_receipt_leaves_unrelated_unsent_words() {
    let Some(h) = Harness::boot_bare(
        "crown_duplicate_receipt",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_duplicate_receipt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let journal = h.paths.state_dir.join("daemon.log");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw1.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W1's seat", || {
        c.board().live_agent(w1).is_some()
    });
    let ws1 = c.board().live_agent(w1).unwrap().id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let unsent = |c: &mut TestClient| {
        c.board().sessions.iter().find(|s| s.id == sa).and_then(|s| s.unsent.clone())
    };

    // ---- a person's words to the crown, refused, kept unsent ---------------
    std::fs::write(h.dir.join("mod-drop-submit"), "").unwrap();
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: a,
            text: "mesimon-probe-936 the person's own words".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Ok
    ));
    wait_until(std::time::Duration::from_secs(10), "the person's words kept unsent", || {
        unsent(&mut c).is_some_and(|u| u.text.contains("the person's own words"))
    });
    std::fs::remove_file(h.dir.join("mod-drop-submit")).unwrap();

    // ---- a worker's hand: its wake entered, told once ----------------------
    match c.send(
        Principal::Agent { session: ws1 },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the wake told", || {
        read_file(&feed).contains("\"crown_wake_sent\"")
    });
    let frame = read_file(&h.dir.join(format!("mod-{sa}.ndjson")))
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["kind"] == "submit" && v.to_string().contains("raised its hand"))
        .find_map(|v| v["id"].as_str().map(str::to_string))
        .expect("the wake's frame");

    // ---- the same frame's report again: nobody's, the words untouched ------
    hook_send_road(
        &hook_sock,
        &sa.to_string(),
        "ModSubmit",
        Some(&frame),
        None,
        r#"{"outcome":"entered"}"#,
        Some("mod"),
    );
    let none = format!("mod submit {frame} for session {sa}: entered, no owner");
    wait_until(std::time::Duration::from_secs(10), "the duplicate read as no one's", || {
        read_file(&journal).contains(&none)
    });
    assert!(
        unsent(&mut c).is_some_and(|u| u.text.contains("the person's own words")),
        "the unrelated words stay: {:?}",
        unsent(&mut c)
    );
    assert_eq!(read_file(&feed).matches("\"crown_wake_sent\"").count(), 1, "told once");
}

/// T-936, the fourth read: a retired attempt's frame leaves the crown's
/// mod outbox even when an unrelated prompt took its owed entry first. The
/// crown's bridge is stopped, so the frame is never acknowledged; a prompt
/// into the crown takes the owed entry; the window passes and the attempt
/// retires, `unconfirmed`; and what a bridge polling now is handed is the
/// next attempt alone, never the retired frame beside it.
#[test]
fn a_retired_wake_frame_leaves_the_outbox() {
    let Some(h) = Harness::boot_bare(
        "crown_retired_outbox",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_retired_outbox");
    let journal = h.paths.state_dir.join("daemon.log");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw1.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W1's seat", || {
        c.board().live_agent(w1).is_some()
    });
    let ws1 = c.board().live_agent(w1).unwrap().id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let bridge = std::fs::read_to_string(h.dir.join(format!("mod-bridge-{sa}.pid")))
        .unwrap()
        .trim()
        .to_string();
    let signal = |sig: &str| {
        assert!(std::process::Command::new("kill")
            .args([sig, &bridge])
            .status()
            .unwrap()
            .success());
    };

    signal("-STOP");
    match c.send(
        Principal::Agent { session: ws1 },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    // An unrelated prompt takes the attempt's owed entry; its window holds.
    start(&mut c, sa);
    let retired = || -> Option<String> {
        read_file(&journal).lines().find(|l| l.ends_with("crown_wake_unconfirmed")).and_then(|l| {
            l.split("crown wake ").nth(1).and_then(|r| r.split(' ').next()).map(str::to_string)
        })
    };
    wait_until(std::time::Duration::from_secs(20), "the attempt retired", || retired().is_some());
    let old = retired().unwrap();
    // The next attempt goes once the crown's turn ends; a bridge polling now
    // is handed it alone.
    stop(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let frames = match c.send(
        Principal::Agent { session: sa },
        Command::ModNext { ack: None, pane: None, speaks: vec!["submit".into()] },
    ) {
        Response::ModFrames { frames } => frames,
        other => panic!("the outbox: {other:?}"),
    };
    let ids: Vec<&str> = frames.iter().filter_map(|f| f["id"].as_str()).collect();
    assert!(!ids.contains(&old.as_str()), "the retired frame is gone: {ids:?}");
    assert!(
        frames.iter().any(|f| f.to_string().contains("raised its hand")),
        "the next attempt: {frames:?}"
    );
    signal("-CONT");
}

/// T-936, the fourth read: a person's prompt into a crown its wake is
/// bringing up, before the wake's own paste and Enter, is the person's: the
/// wake goes back to owed, untold, and its hand stays up. The delivery that
/// tells it is its own paste, on the live pane's road, once the crown's
/// turn on the person's words is over.
#[test]
fn a_person_s_prompt_before_a_parked_wake_s_paste_leaves_it_owed() {
    let Some(h) = Harness::boot_with_env(
        "crown_parked_prompt",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_parked_prompt");
    let got = h.dir.join("got.txt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's seat", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let raised = format!("{kw} \"mesimon-probe-936 worker\" raised its hand");
    let typed = || read_file(&got).lines().filter(|l| l.contains(&raised)).count();
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let owed = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
    };

    assert!(matches!(c.request(Command::SleepSession { id: sa }), Response::Ok));
    c.await_state(sa, "the crown parked", |s| *s == SessionState::Sleeping);
    match c.send(
        Principal::Agent { session: ws },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the crown woken for it", || {
        c.board().sessions.iter().any(|s| s.id == sa && s.state != SessionState::Sleeping)
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    // A person's prompt reaches the crown before the wake's paste (which
    // waits for the pane's `SessionStart`).
    start(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert!(hand_up(&mut c), "the person's prompt is no delivery");
    assert!(owed(&mut c), "the wake owed again");
    assert!(!read_file(&feed).contains("\"crown_wake_sent\""), "nothing told");
    assert!(!read_file(&feed).contains("\"crown_wake_delivered\""), "no delivery said");
    // The crown's turn on their words ends: the wake goes by its own paste.
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(15), "the wake typed into the crown", || {
        typed() == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c) && !owed(&mut c)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(typed(), 1, "once");
}

/// T-990: a wake the crown's bridge took before a restart is told, and the
/// restart keeps it told. The mod takes the frame, puts the words in and
/// holds its `entered`; the daemon restarts; the report comes twelve
/// seconds later, to the new daemon. One copy in the crown's pane, nothing
/// retired, the report its own frame's, the wake told once.
#[test]
fn a_wake_taken_before_a_restart_is_told_once() {
    let Some(h) = Harness::boot_bare(
        "crown_offered_restart",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_offered_restart");
    let journal = h.paths.state_dir.join("daemon.log");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w1 = c.create_ticket("mesimon-probe-936 worker");
    let kw1 = key_of(&mut c, w1);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw1).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw1.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W1's seat", || {
        c.board().live_agent(w1).is_some()
    });
    let ws1 = c.board().live_agent(w1).unwrap().id;
    up(ws1);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // The crown's mod takes the frame, puts the words in and holds its
    // report.
    std::fs::write(h.dir.join("mod-hold-entered"), "").unwrap();
    match c.send(
        Principal::Agent { session: ws1 },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    let frame = || {
        read_file(&h.dir.join(format!("mod-{sa}.ndjson")))
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "submit" && v.to_string().contains("raised its hand"))
            .find_map(|v| v["id"].as_str().map(str::to_string))
    };
    wait_until(std::time::Duration::from_secs(10), "the frame handed to the mod", || {
        frame().is_some()
    });
    let frame = frame().unwrap();
    let taken = format!("crown wake {frame} in session {sa}: taken");
    wait_until(std::time::Duration::from_secs(10), "the take", || {
        read_file(&journal).contains(&taken)
    });
    let feed = h.paths.state_dir.join("activity.jsonl");
    let count = |cmd: &str| read_file(&feed).matches(&format!("\"cmd\":\"{cmd}\"")).count();
    wait_until(std::time::Duration::from_secs(5), "told at the take", || {
        count("crown_wake_sent") == 1 && c.board().ticket(w1).unwrap().raised.is_none()
    });
    drop(c);
    h.restart();
    let _c = h.client("crown_offered_restart_2");
    // Past a window the restart once gave it: nothing retires or resends.
    std::thread::sleep(std::time::Duration::from_secs(12));
    std::fs::write(h.dir.join("mod-release"), "").unwrap();
    // The restart's own look at the worker may owe it a wake of its own
    // (T-602): this frame's receipt is the one read.
    let receipt = format!("\"reason\":\"{frame}\"");
    wait_until(std::time::Duration::from_secs(15), "the frame's own receipt", || {
        read_file(&feed).contains(&receipt) && count("crown_wake_delivered") >= 1
    });
    std::thread::sleep(std::time::Duration::from_secs(2));
    let log = read_file(&journal);
    assert!(!log.contains("crown_wake_unconfirmed"), "nothing retired");
    let nobody = format!("mod submit {frame} for session {sa}: entered, no owner");
    assert!(!log.contains(&nobody), "the receipt had its owner");
    let submits = read_file(&h.dir.join(format!("mod-{sa}.ndjson")))
        .lines()
        .filter(|l| l.contains("\"submit\"") && l.contains("raised its hand"))
        .count();
    assert_eq!(submits, 1, "one frame");
    let got = read_file(&h.dir.join("got.txt"));
    assert_eq!(got.matches("raised its hand").count(), 1, "one copy in the pane");
}

/// T-990, round 2: every taken wake keeps what its refusal or a lost pane
/// needs, however many are out. Nine wakes go down the crown's mod, each
/// taken, told and left unanswered, with a restart after the fifth (the
/// records come back from `crown.json`). The first frame's refusal then
/// lands: owed again and typed once. Then the crown's pane goes: every
/// other taken wake still unanswered is owed again.
#[test]
fn nine_taken_wakes_each_keep_their_refusal_and_their_pane() {
    let Some(h) = Harness::boot_bare(
        "crown_nine_taken",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "mod"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_nine_taken");
    let journal = h.paths.state_dir.join("daemon.log");
    let got = h.dir.join("got.txt");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let up = |sid: uuid::Uuid| {
        let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
        wait_until(std::time::Duration::from_secs(10), "the mod's bridge", || pid.exists());
        hook_send_with(
            &hook_sock,
            &sid.to_string(),
            "SessionStart",
            Some("startup"),
            r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
        );
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-990 worker");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    up(sa);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
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
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "the worker's seat", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    up(ws);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let raise = |c: &mut TestClient, why: &str| match c
        .send(Principal::Agent { session: ws }, Command::AgentRaiseHand { reason: why.into() })
    {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    };
    // The frames taken, in order, as the journal says them.
    let taken = || -> Vec<String> {
        read_file(&journal)
            .lines()
            .filter(|l| l.ends_with(": taken"))
            .filter_map(|l| l.split("crown wake ").nth(1)?.split(' ').next().map(str::to_string))
            .collect()
    };
    let lines = |what: &str| read_file(&got).lines().filter(|l| l.contains(what)).count();

    // ---- nine taken, unanswered, a restart after the fifth -----------------
    std::fs::write(h.dir.join("mod-hold-entered"), "").unwrap();
    for i in 0..9 {
        if i == 5 {
            drop(c);
            h.restart();
            c = h.client("crown_nine_taken_2");
        }
        let before = taken().len();
        raise(&mut c, &format!("question {i}"));
        wait_until(std::time::Duration::from_secs(25), &format!("wake {i} taken"), || {
            taken().len() > before
        });
        // The crown's prompt for it clears its owed entry, and its turn ends.
        start(&mut c, sa);
        stop(&mut c, sa);
    }
    let frames = taken();
    assert!(frames.len() >= 9, "nine taken: {frames:?}");
    assert!(!read_file(&journal).contains("crown_wake_unconfirmed"), "none retired");

    // ---- the first frame's refusal: owed again, typed once -----------------
    let typed = lines("raised its hand");
    write_whole(&h.dir.join(format!("mod-release-{}", frames[0])), "dropped");
    let refused = format!("crown wake {} in session {sa}: refused, dropped", frames[0]);
    wait_until(std::time::Duration::from_secs(10), "the first frame's refusal matched", || {
        read_file(&journal).contains(&refused)
    });
    wait_until(std::time::Duration::from_secs(15), "the refused wake typed", || {
        lines("raised its hand") == typed + 1
    });
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(lines("raised its hand"), typed + 1, "typed once");

    // ---- the crown's pane goes: every other taken wake owed again ----------
    assert!(matches!(c.request(Command::KillSession { id: sa }), Response::Ok));
    let owed = || {
        read_file(&journal)
            .lines()
            .filter(|l| l.ends_with("taken, its pane gone; owed again"))
            .count()
    };
    wait_until(std::time::Duration::from_secs(15), "every other taken wake owed again", || {
        owed() == frames.len() - 1
    });
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert_eq!(owed(), frames.len() - 1, "each once");
    assert!(
        pending_of(&mut c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "the wake owed"
    );
}

/// Set up the fifth read's launch-road specimens: a hook-road crown with a
/// worker, the crown parked, then the worker's hand, which wakes the crown
/// with its wake parked on the launch. The pane is up and its
/// `SessionStart` not yet sent. Returns the crown's session, the worker's,
/// and the wake's line as the stub records it.
fn crown_woken_with_its_wake_parked(
    h: &Harness,
    c: &mut TestClient,
    name: &str,
) -> (ulid::Ulid, uuid::Uuid, ulid::Ulid, String) {
    let hook_sock = h.paths.hook_sock();
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket(&format!("mesimon-probe-936 {name}"));
    let kw = key_of(c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(c, sa);
    stop(c, sa);
    let v = read(c, sa, &kw).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
                expect_secs: None,
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(15), "W's seat", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(matches!(c.request(Command::SleepSession { id: sa }), Response::Ok));
    c.await_state(sa, "the crown parked", |s| *s == SessionState::Sleeping);
    match c.send(
        Principal::Agent { session: ws },
        Command::AgentRaiseHand { reason: "which key?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the crown woken for it", || {
        c.board().sessions.iter().any(|s| s.id == sa && s.state != SessionState::Sleeping)
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    let line = format!("{kw} \"mesimon-probe-936 {name}\" raised its hand");
    (a, sa, w, line)
}

/// T-936, the fifth read (2a): a person who attaches to the crown's pane
/// while its wake is bringing it up, before the launch pastes, has nothing
/// typed under them. The launch's paste meets the live pane's guard, the
/// wake goes back to owed with its hand up and its row saying it waits for
/// them, and once they leave the live pane's road delivers it, once.
#[test]
fn a_person_at_the_crown_before_its_launch_paste_gets_no_keys() {
    let Some(h) = Harness::boot_with_env(
        "crown_launch_person",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_launch_person");
    let (a, sa, w, line) = crown_woken_with_its_wake_parked(&h, &mut c, "launch person");
    let got = h.dir.join("got.txt");
    let typed = || {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&line))
            .count()
    };
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let held = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    // The pane comes up: its words would go now.
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(10), "the wake held for the person", || {
        held(&mut c).as_deref() == Some(WHEN_YOU_LEAVE)
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(typed(), 0, "nothing typed under them");
    assert!(hand_up(&mut c), "the hand still up");
    client.kill().unwrap();
    let _ = client.wait();
    wait_until(std::time::Duration::from_secs(15), "the wake typed once they left", || {
        typed() == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(typed(), 1, "once");
}

/// T-936, the fifth and sixth reads (2b, 2): once a crown wake's paste and
/// Enter went in, the board presses no more Enters for it. Here no prompt
/// hook follows; a person types a draft into the crown's box, and a
/// repeated `SessionStart` frame comes: no Enter sends their words. The
/// paste's entry expires (`paste_unacked`) with no unsent copy and nothing
/// owed again for the told wake, while a hand raised meanwhile stays owed
/// on its own, held on their draft.
#[test]
fn no_retry_enter_follows_a_wake_s_own_paste() {
    let Some(h) = Harness::boot_bare(
        "crown_launch_retry",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_launch_retry");
    let (a, sa, w, line) = crown_woken_with_its_wake_parked(&h, &mut c, "launch retry");
    let got = h.dir.join("got.txt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let typed = || read_got().lines().filter(|l| l.contains(&line)).count();
    let wake_rows = |c: &mut TestClient| -> Vec<mesimon_core::command::Pending> {
        pending_of(c, Some(a))
            .into_iter()
            .filter(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .collect()
    };
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(15), "the wake's paste and Enter", || typed() == 1);
    wait_until(std::time::Duration::from_secs(5), "told at its paste", || {
        c.board().ticket(w).unwrap().raised.is_none()
    });
    // No prompt hook comes. A person types into the box, and a repeated
    // Startup/Resume frame arrives.
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert!(!read_got().contains("half a thought"), "no Enter sent their draft: {}", read_got());
    // A hand raised meanwhile: news of its own.
    let ws = c.board().live_agent(w).unwrap().id;
    match c.send(
        Principal::Agent { session: ws },
        Command::AgentRaiseHand { reason: "which key, again?".into() },
    ) {
        Response::AgentRaised { .. } => {}
        other => panic!("raise_hand: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the paste's entry expired", || {
        std::fs::read_to_string(&feed).unwrap_or_default().contains("\"paste_unacked\"")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let rec = c.board().sessions.iter().find(|s| s.id == sa).cloned().unwrap();
    assert!(rec.unsent.is_none(), "no unsent copy: {:?}", rec.unsent);
    assert!(!read_got().contains("half a thought"), "their draft never sent");
    assert_eq!(typed(), 1, "the told wake not owed again, nor typed again");
    let rows = wake_rows(&mut c);
    assert_eq!(rows.len(), 1, "the new hand owed: {rows:?}");
    assert_eq!(rows[0].held.as_deref(), Some("after your draft"), "held on their draft");
    assert!(c.board().ticket(w).unwrap().raised.is_some(), "the new hand up");
}

/// T-936, the fifth read (2c): a crown launch abandoned on a dialog (here a
/// permission request, while its composer has not yet drawn) leaves no
/// unsent copy of the wake on the crown's seat. The wake is owed again, its
/// hand still up.
#[test]
fn a_crown_launch_abandoned_on_a_dialog_keeps_no_unsent_copy() {
    let Some(h) = Harness::boot_bare(
        "crown_launch_modal",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
            ("MESIMON_COMPOSER_WAIT_MS", "60000"),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_launch_modal");
    let (a, sa, w, _line) = crown_woken_with_its_wake_parked(&h, &mut c, "launch modal");
    let feed = h.paths.state_dir.join("activity.jsonl");
    // The pane is up and its words wait for a composer that never draws;
    // then a dialog comes.
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    std::thread::sleep(std::time::Duration::from_millis(700));
    hook_send(
        &hook_sock,
        &sa.to_string(),
        "PermissionRequest",
        r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
    );
    c.await_state(sa, "on the dialog", |s| matches!(s, SessionState::RequiresAction { .. }));
    wait_until(std::time::Duration::from_secs(10), "the launch's words abandoned", || {
        std::fs::read_to_string(&feed).unwrap_or_default().contains("\"prompt_submit_abandoned\"")
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let rec = c.board().sessions.iter().find(|s| s.id == sa).cloned().unwrap();
    assert!(rec.unsent.is_none(), "no unsent copy of the wake: {:?}", rec.unsent);
    assert!(c.board().ticket(w).unwrap().raised.is_some(), "the hand still up");
    assert!(
        pending_of(&mut c, Some(a))
            .iter()
            .any(|p| p.action == mesimon_core::command::PendingAction::CrownWake),
        "the wake owed again"
    );
}

/// T-936, the sixth read (1): a launch's cached screen is no evidence of
/// an empty box. A person comes into the woken crown's pane, types a draft
/// and leaves before the launch would paste; the pane's `SessionStart`
/// follows at once. The launch hands the wake back, and the live pane's
/// road reads the pane's clients and box on one fresh sample: the draft
/// holds it (`after your draft`), and nothing is typed. Once they send
/// their words and the box reads empty, the wake goes in, once.
#[test]
fn a_detached_draft_holds_a_crown_wake_the_launch_handed_back() {
    let Some(h) = Harness::boot_bare(
        "crown_launch_draft",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_launch_draft");
    let (a, sa, w, line) = crown_woken_with_its_wake_parked(&h, &mut c, "launch draft");
    let got = h.dir.join("got.txt");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let typed = || read_got().lines().filter(|l| l.contains(&line)).count();
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let held = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    // The box drawn empty, then a person's draft, typed while attached.
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    client.kill().unwrap();
    let _ = client.wait();
    // They left; the launch would paste now.
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(10), "the wake held on their draft", || {
        held(&mut c).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(typed(), 0, "nothing typed into their draft: {}", read_got());
    assert!(!read_got().contains("half a thought"), "no Enter sent their draft");
    assert!(hand_up(&mut c), "the hand still up");
    // They send their words: the box reads empty, and the wake goes once.
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(5), "their words sent", || {
        read_got().lines().any(|l| l.trim() == "half a thought")
    });
    wait_until(std::time::Duration::from_secs(15), "the wake typed into the empty box", || {
        typed() == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c)
    });
    assert!(
        read_got().lines().all(|l| !(l.contains("half a thought") && l.contains(&line))),
        "never joined: {}",
        read_got()
    );
}

/// T-945 on T-936's hand-back: a woken crown's launch hands its wake back
/// with a client still in its pane and a draft in its box. The client holds
/// it while tmux records activity from it, the draft past that; nothing is
/// typed into the draft, and once the person sends it the wake goes in
/// once, the client still attached.
#[test]
fn an_idle_client_and_a_draft_hold_a_wake_the_launch_handed_back() {
    let Some(h) = Harness::boot_bare(
        "crown_launch_idle",
        Some(BOX_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
            ("MESIMON_PERSON_IDLE_MS", IDLE_MS),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_launch_idle");
    let (a, sa, w, line) = crown_woken_with_its_wake_parked(&h, &mut c, "launch idle");
    let got = h.dir.join("got.txt");
    let read_got = || std::fs::read_to_string(&got).unwrap_or_default();
    let typed = || read_got().lines().filter(|l| l.contains(&line)).count();
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let held = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let (mut client, _) = attach_client(&tmux_sock, &crown);
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "-l", "half a thought"])
        .status()
        .unwrap()
        .success());
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(10), "held for the client", || {
        held(&mut c).as_deref() == Some(WHEN_YOU_LEAVE_5S)
    });
    wait_until(std::time::Duration::from_secs(15), "held for the draft past the bound", || {
        held(&mut c).as_deref() == Some("after your draft")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(typed(), 0, "nothing typed into their draft: {}", read_got());
    assert!(!read_got().contains("half a thought"), "no Enter sent their draft");
    assert!(hand_up(&mut c), "the hand still up");
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &crown, "Enter"])
        .status()
        .unwrap()
        .success());
    wait_until(std::time::Duration::from_secs(15), "the wake typed into the empty box", || {
        typed() == 1
    });
    assert!(read_got().lines().any(|l| l.trim() == "half a thought"), "{}", read_got());
    assert!(
        read_got().lines().all(|l| !(l.contains("half a thought") && l.contains(&line))),
        "never joined: {}",
        read_got()
    );
    assert!(attach_listed(&tmux_sock, &crown), "the client still attached");
    client.kill().unwrap();
    let _ = client.wait();
}

/// A Claude stub whose screen the test sets: `BOX_STUB`'s empty composer,
/// or, while a `blank` file stands beside it, a screen with no composer on
/// it at all. Every line it is given goes to `got.txt`. Bash, for `read
/// -t`: Ubuntu's `/bin/sh` is dash, which refuses the flag (T-1019).
const BLANKABLE_STUB: &str = "#!/bin/bash\nstty -icanon 2>/dev/null\ndir=$(dirname \"$0\")\nshown=\n\
    paint() { if [ -f \"$dir/blank\" ]; then s=blank; else s=box; fi; \
    [ \"$s\" = \"$shown\" ] && return; shown=$s; \
    if [ $s = blank ]; then printf '\\033[2J\\033[H  loading the session\\n'; \
    else printf '\\033[2J\\033[999;1H\\033[3A────────────────────\\n❯ \\n────────────────────\\n  ? for shortcuts\
\\033[999;1H\\033[2A\\033[3G'; fi; }\n\
    while true; do paint; if IFS= read -r -t 1 line; then \
    printf '%s\\n' \"$line\" >> \"$dir/got.txt\"; shown=; fi; done\n";

/// T-936, the seventh read: the paste road's own rule, which survives a
/// restart. A launch hands the crown's wake back while a person is in its
/// pane; the box then shows no composer and the daemon restarts. The idle
/// live crown, the person gone, reads as no composer at all: no keys. Once
/// the box draws empty, one delivery.
#[test]
fn a_crown_wake_after_a_restart_waits_for_an_empty_box() {
    let Some(h) = Harness::boot_bare(
        "crown_restart_box",
        Some(BLANKABLE_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_restart_box");
    let (a, sa, w, line) = crown_woken_with_its_wake_parked(&h, &mut c, "restart box");
    let got = h.dir.join("got.txt");
    let typed = || {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&line))
            .count()
    };
    let hand_up = |c: &mut TestClient| c.board().ticket(w).unwrap().raised.is_some();
    let held = |c: &mut TestClient| {
        pending_of(c, Some(a))
            .into_iter()
            .find(|p| p.action == mesimon_core::command::PendingAction::CrownWake)
            .and_then(|p| p.held)
    };
    let crown = c.board().sessions.iter().find(|s| s.id == sa).unwrap().sid16();
    let mut client = tmux(&tmux_sock)
        .args(["-C", "attach", "-t", &crown])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a control-mode tmux client");
    wait_until(std::time::Duration::from_secs(10), "the client on the crown's session", || {
        tmux(&tmux_sock)
            .args(["list-clients", "-t", &crown, "-F", "#{client_name}"])
            .output()
            .is_ok_and(|o| !o.stdout.is_empty())
    });
    // The launch hands the wake back; the person in the pane holds it.
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(10), "the wake held for the person", || {
        held(&mut c).as_deref() == Some(WHEN_YOU_LEAVE)
    });
    // The box shows no composer, and the daemon restarts.
    std::fs::write(h.dir.join("blank"), "").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    drop(c);
    h.restart();
    let mut c = h.client("crown_restart_box_2");
    let state = c.board().sessions.iter().find(|s| s.id == sa).map(|s| s.state.clone());
    assert!(matches!(state, Some(SessionState::Idle { .. })), "the crown idle: {state:?}");
    // The person leaves: nobody there, and no composer to read.
    client.kill().unwrap();
    let _ = client.wait();
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert_eq!(typed(), 0, "no keys into a box read as no composer");
    assert!(hand_up(&mut c), "the hand still up");
    assert!(
        held(&mut c).is_some()
            || pending_of(&mut c, Some(a))
                .iter()
                .any(|p| { p.action == mesimon_core::command::PendingAction::CrownWake }),
        "the wake still owed"
    );
    // The box draws empty: one delivery.
    std::fs::remove_file(h.dir.join("blank")).unwrap();
    wait_until(std::time::Duration::from_secs(15), "the wake typed into the empty box", || {
        typed() == 1
    });
    wait_until(std::time::Duration::from_secs(5), "the hand down once it went", || {
        !hand_up(&mut c)
    });
    std::thread::sleep(std::time::Duration::from_millis(1000));
    assert_eq!(typed(), 1, "once");
}

/// T-973: a turn the crown armed with `expect_secs` that runs past it, the
/// agent still working, wakes the crown once with the turn's length and the
/// expectation, and the crown's `get_ticket` carries `overrun` until the
/// turn ends. The same turn running on says nothing more, nothing is
/// stopped, and the turn's end takes `overrun` away. The crown's next ask
/// arms the next expectation, and a turn that ends inside its own wakes
/// nobody for it. A start's launch turn and an ask's turn are both timed.
#[test]
fn a_turn_past_its_expect_secs_wakes_the_crown_once() {
    let Some(h) = Harness::boot_with_env(
        "crown_overrun",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_overrun");
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
    // The crown takes the line it was woken with: one turn, then idle.
    let crown_turn = |c: &mut TestClient, sa: uuid::Uuid| {
        send(sa, "UserPromptSubmit", r#"{"prompt":"wake"}"#);
        c.await_state(sa, "the crown running", |s| *s == SessionState::Running);
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 runs long");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    std::thread::sleep(std::time::Duration::from_millis(500));
    crown_turn(&mut c, sa);

    // ---- 1. the start's launch turn runs past its 3 s ----------------------
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: Some(3),
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
    send(ws, "UserPromptSubmit", r#"{"prompt":"the brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    let worker = format!("{kw} \"mesimon-probe-973 runs long\"");
    let overrun = format!("{worker} has run its turn ");
    wait_until(std::time::Duration::from_secs(20), "the overrun wake", || {
        lines_with(&overrun).len() == 1
    });
    let line = lines_with(&overrun).remove(0);
    assert!(line.contains(", past the 3 s it expected"), "{line}");
    let o = read(&mut c, sa, &kw).unwrap().overrun.expect("overrun on get_ticket");
    assert_eq!(o.expected_secs, 3);
    assert!(o.running_secs >= 3, "{o:?}");
    crown_turn(&mut c, sa);

    // ---- 2. the same turn runs on: no second wake, nothing stopped ---------
    std::thread::sleep(std::time::Duration::from_millis(4500));
    assert_eq!(lines_with(&overrun).len(), 1, "one wake per expectation");
    assert_eq!(c.board().live_agent(w).unwrap().state, SessionState::Running, "nothing stopped");
    let o = read(&mut c, sa, &kw).unwrap().overrun.expect("still past it");
    assert!(o.running_secs >= 7, "{o:?}");

    // ---- 3. the turn ends: overrun gone ------------------------------------
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    let finished = format!("{worker} finished its turn");
    wait_until(std::time::Duration::from_secs(15), "the finished wake", || {
        !lines_with(&finished).is_empty()
    });
    crown_turn(&mut c, sa);
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none(), "the turn ended");

    // ---- 4. the crown's next ask arms the next expectation -----------------
    let answered = |lines_with: &dyn Fn(&str) -> Vec<String>| {
        ask_ends(&worker).iter().map(|l| lines_with(l).len()).sum::<usize>()
    };
    let ask = |c: &mut TestClient, text: &str| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: Some(3),
            },
        ) {
            Response::AgentAsked { held_for_person: false, .. } => {}
            other => panic!("ask_agent: {other:?}"),
        }
    };
    ask(&mut c, "mesimon-probe-973 ask one");
    wait_until(std::time::Duration::from_secs(10), "the first ask's words", || {
        !lines_with("mesimon-probe-973 ask one").is_empty()
    });
    send(ws, "UserPromptSubmit", r#"{"prompt":"mesimon-probe-973 ask one"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    wait_until(std::time::Duration::from_secs(20), "the second overrun wake", || {
        lines_with(&overrun).len() == 2
    });
    crown_turn(&mut c, sa);
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(15), "the answer's wake", || {
        answered(&lines_with) == 1
    });
    crown_turn(&mut c, sa);

    // ---- 5. a turn that ends inside its expectation wakes nobody for it ----
    ask(&mut c, "mesimon-probe-973 ask two");
    wait_until(std::time::Duration::from_secs(10), "the second ask's words", || {
        !lines_with("mesimon-probe-973 ask two").is_empty()
    });
    send(ws, "UserPromptSubmit", r#"{"prompt":"mesimon-probe-973 ask two"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(15), "the second answer's wake", || {
        answered(&lines_with) == 2
    });
    crown_turn(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(4500));
    assert_eq!(lines_with(&overrun).len(), 2, "a turn inside its expectation is no overrun");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
}

/// `sample_agent`, the crown's way in (T-973).
fn sample(
    c: &mut TestClient,
    session: uuid::Uuid,
    key: &str,
) -> Result<mesimon_core::command::AgentSampleView, String> {
    match c.send(Principal::Agent { session }, Command::AgentSampleTicket { key: key.into() }) {
        Response::AgentSample { sample } => Ok(sample),
        Response::Err { message } => Err(message),
        other => panic!("sample {key}: {other:?}"),
    }
}

/// T-973: the crown samples an agent it started: the turn's age, the tool
/// in flight (a call the transcript shows with no result), the last tools,
/// the last text and the pane's last rows (the stub's composer). Refused
/// on the crown's own ticket, for a person's agent, to an uncrowned worker
/// and with Settings › Crown › Samples agents off; nothing reaches the
/// sampled agent.
#[test]
fn the_crown_samples_an_agent_it_started() {
    let Some(h) = Harness::boot_with_env(
        "crown_sample",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_sample");
    let send = |sid: uuid::Uuid, event: &str, body: &str| {
        hook_send(&hook_sock, &sid.to_string(), event, body);
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 sampled");
    let p = c.create_ticket("mesimon-probe-973 a person's");
    let (ka, kw, kp) = (key_of(&mut c, a), key_of(&mut c, w), key_of(&mut c, p));
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    send(sa, "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(sa, "running", |s| *s == SessionState::Running);
    send(sa, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(sample(&mut c, sa, &kw).unwrap_err().contains("has no agent"));
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    // The transcript: an edit and a read came back, a word to the person,
    // then a command still running.
    let transcript = h.dir.join("sampled.jsonl");
    let call = |id: &str, name: &str| {
        json!({"uuid": format!("a-{id}"), "type": "assistant",
            "timestamp": "2026-10-25T00:00:00Z",
            "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": {}}]}})
    };
    let result = |id: &str| {
        json!({"uuid": format!("r-{id}"), "type": "user",
            "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": "ok"}]}})
    };
    let text = json!({"uuid": "t", "type": "assistant", "message": {"content": [
        {"type": "text", "text": "mesimon-probe-973 waiting on the loop"}]}});
    let records = [
        call("e1", "Edit"),
        result("e1"),
        call("r1", "Read"),
        result("r1"),
        text,
        call("b1", "Bash"),
    ];
    let body: String = records.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(&transcript, body).unwrap();
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
    std::thread::sleep(std::time::Duration::from_millis(500));
    send(ws, "UserPromptSubmit", r#"{"prompt":"the brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    let mut s = None;
    wait_until(std::time::Duration::from_secs(15), "a sample with the pane's rows", || {
        s = sample(&mut c, sa, &kw).ok();
        s.as_ref().and_then(|s| s.pane_tail.as_deref()).is_some_and(|t| t.contains("for shortcuts"))
    });
    let s = s.unwrap();
    assert_eq!(s.key, kw);
    assert_eq!(s.state, "working");
    assert!(s.turn_secs.is_some(), "{s:?}");
    let tool = s.current_tool.as_ref().expect("the call with no result");
    assert_eq!(tool.name, "Bash");
    assert!(tool.secs.is_some_and(|secs| secs > 0), "{tool:?}");
    assert_eq!(s.recent_tools, ["Bash", "Read", "Edit"]);
    assert_eq!(s.last_text.as_deref(), Some("mesimon-probe-973 waiting on the loop"));
    assert!(s.background.is_none());
    assert!(s.pane_tail.as_ref().unwrap().len() <= 4096);
    let got = std::fs::read_to_string(h.dir.join(format!("got-{w}.txt"))).unwrap_or_default();
    assert!(!got.contains("sample"), "nothing reaches the sampled agent: {got}");

    // ---- the refusals ------------------------------------------------------
    let own = sample(&mut c, sa, &ka).unwrap_err();
    assert!(own.contains("this session's own ticket"), "{own}");
    c.spawn_claude(p);
    wait_until(std::time::Duration::from_secs(10), "the person's agent", || {
        c.board().live_agent(p).is_some()
    });
    let persons = sample(&mut c, sa, &kp).unwrap_err();
    assert!(persons.contains("a person started"), "{persons}");
    let worker = sample(&mut c, ws, &ka).unwrap_err();
    assert!(worker.contains("only the crown"), "{worker}");
    assert!(matches!(c.request(Command::SetCrownSamples { on: false }), Response::Ok));
    assert!(!c.board().crown_samples);
    let off = sample(&mut c, sa, &kw).unwrap_err();
    assert!(off.contains("Settings → Crown → Samples agents is off"), "{off}");
    assert!(matches!(c.request(Command::SetCrownSamples { on: true }), Response::Ok));
    assert!(sample(&mut c, sa, &kw).is_ok(), "on again");
}

/// T-973, round 1: a sample's read lands through the call's own gate again
/// and finds the same seat in the same pane, or says nothing of it. Each
/// sample here waits on both sides of its capture (`MESIMON_SAMPLE_DELAY_MS`)
/// while the board changes: the switch turned off, the crown taken off,
/// the agent parked (the seat lost its pane), and the agent parked before
/// the capture (which then reads no pane) and woken after it into a new
/// pane under the same session.
#[test]
fn a_samples_landing_passes_the_gate_again_and_finds_the_same_pane() {
    let Some(h) = Harness::boot_with_env(
        "crown_sample_fence",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_SAMPLE_DELAY_MS", "1500"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let orch = h.paths.orch_sock();
    let mut c = h.client("crown_sample_fence");
    let send = |sid: uuid::Uuid, event: &str, body: &str| {
        hook_send(&hook_sock, &sid.to_string(), event, body);
    };
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 fenced");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
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
            expect_secs: None,
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
    send(ws, "UserPromptSubmit", r#"{"prompt":"the brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    // A sample on a second connection: its reply waits for the read.
    let in_flight = || {
        let orch = orch.clone();
        let kw = kw.clone();
        std::thread::spawn(move || {
            let mut c2 = TestClient::try_connect(&orch, std::time::Duration::from_secs(5))
                .expect("a second connection");
            sample(&mut c2, sa, &kw)
        })
    };
    let ms = |n| std::thread::sleep(std::time::Duration::from_millis(n));
    let pane = |c: &mut TestClient| c.board().live_agent(w).and_then(|r| r.pane_key.clone());

    // ---- unchanged, it lands ------------------------------------------------
    assert!(in_flight().join().unwrap().is_ok(), "a delayed sample of an unchanged seat");

    // ---- the switch turned off mid-read -------------------------------------
    let j = in_flight();
    ms(300);
    assert!(matches!(c.request(Command::SetCrownSamples { on: false }), Response::Ok));
    let off = j.join().unwrap().unwrap_err();
    assert!(off.contains("Samples agents is off"), "{off}");
    assert!(matches!(c.request(Command::SetCrownSamples { on: true }), Response::Ok));

    // ---- the crown taken off mid-read ---------------------------------------
    let j = in_flight();
    ms(300);
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    let gone = j.join().unwrap().unwrap_err();
    assert!(gone.contains("crown"), "no crown, no sample: {gone}");
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(sample(&mut c, sa, &kw).is_ok(), "crowned again");

    // ---- parked mid-read: the seat lost its pane ----------------------------
    let j = in_flight();
    ms(300);
    assert!(matches!(c.request(Command::SleepSession { id: ws }), Response::Ok));
    c.await_state(ws, "asleep", |s| *s == SessionState::Sleeping);
    let parked = j.join().unwrap().unwrap_err();
    assert!(parked.contains("changed during the sample"), "{parked}");

    // ---- read while parked, woken into a new pane before it lands -----------
    match c.request(Command::WakeSession { id: ws }) {
        Response::Spawned { .. } => {}
        other => panic!("the wake: {other:?}"),
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Idle again, so a person's park takes it.
    send(ws, "UserPromptSubmit", r#"{"prompt":"again"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    match c.request(Command::SleepSession { id: ws }) {
        Response::Ok => {}
        other => panic!("the second park: {other:?}"),
    }
    c.await_state(ws, "asleep", |s| *s == SessionState::Sleeping);
    let before = pane(&mut c);
    let j = in_flight();
    // The capture runs 1.5 s in, on a parked seat: no pane to read.
    ms(2200);
    match c.request(Command::WakeSession { id: ws }) {
        Response::Spawned { .. } => {}
        other => panic!("the second wake: {other:?}"),
    }
    let relaunched = j.join().unwrap().unwrap_err();
    assert!(relaunched.contains("changed during the sample"), "{relaunched}");
    assert_ne!(pane(&mut c), before, "the same session, another pane");
}

/// An ISO stamp for a fabricated transcript record, `ms` past the epoch.
fn stamp(ms: u64) -> String {
    let t = mesimon_core::usage::utc_of(ms / 1000);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year + 1900,
        t.mon + 1,
        t.mday,
        t.hour,
        t.min,
        t.sec,
        ms % 1000
    )
}

/// T-983 (after T-973's rounds 1 to 3): a begun expectation outlives a
/// restart only by its turn's own prompt id, found again in the worker's
/// transcript: the turn still running under it is timed from its begin, once,
/// `after a restart`. Round 3's counterexample (A's close, then B's prompt
/// and tool, in the gap) and a turn whose prompt record was not yet written
/// at the restart wake nobody. The crown taken off, or moved to another
/// ticket and back, takes every expectation with it.
#[test]
fn a_begun_expectation_outlives_a_restart_by_its_own_prompt() {
    expectation_restart("crown_overrun_restart", Part::Restart);
}

/// T-983, round 1: words typed mid-turn ride the running turn's prompt id,
/// so a frame from the worker's pane under another prompt id is another
/// turn, and it retires the begun expectation whether the board saw the
/// first turn end or not: live, A then B with no idle between; and after a
/// restart that found A open in the transcript, B's hook arriving before
/// B's record. And a begin whose frame carries no id stays unbound, never
/// taking the id an earlier turn left in the cache: its own permission
/// prompt's id binds it, and a restart then finds that turn.
#[test]
fn another_prompt_s_id_retires_a_begun_expectation() {
    expectation_restart("crown_overrun_retire", Part::Retire);
}

/// T-983, the mod pass: words down a `submit` whose `entered` comes only
/// after a restart still begin their turn, bound to the prompt relayed
/// before it; an `entered` that comes after another prompt has run (T-982's
/// fourth read: A, A's end, B, then A's report), with or without a restart
/// between, begins nothing.
#[test]
fn a_held_entered_begins_only_its_own_prompt_s_turn() {
    if test_road() != "mod" {
        return;
    }
    expectation_restart("crown_overrun_entered", Part::Entered);
}

/// T-983, round 2, the mod pass: a submit dispatched while the board has
/// heard no prompt id from its seat (right after a restart) counts every
/// id it hears, and two before its `entered` drop it: a missed overrun,
/// never a wrong one. The reader's trace (A's prompt, A's end, B's, then
/// A's late report), with and without a restart between, begins nothing;
/// and a dispatch into a running R whose id was never heard (R's id, then
/// A's, then A's report) does not begin either, the stated limit. A frame
/// from a pane the seat left is heard by nothing.
#[test]
fn an_unknown_baseline_counts_every_prompt_id() {
    if test_road() != "mod" {
        return;
    }
    expectation_restart("crown_overrun_unheard", Part::Unheard);
}

/// Which part of the restart cases one test runs (each under nextest's
/// 120 s limit).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Restart,
    Entered,
    Retire,
    Unheard,
}

fn expectation_restart(name: &str, part: Part) {
    let entered = part == Part::Entered;
    let Some(h) = Harness::boot_with_env(
        name,
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client(name);
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
    let crown_turn = |c: &mut TestClient, sa: uuid::Uuid| {
        send(sa, "UserPromptSubmit", r#"{"prompt":"wake"}"#);
        c.await_state(sa, "the crown running", |s| *s == SessionState::Running);
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let mod_road = test_road() == "mod";
    let switch = |name: &str, on: bool| {
        let f = h.dir.join(name);
        if on {
            std::fs::write(f, "").unwrap();
        } else {
            let _ = std::fs::remove_file(f);
        }
    };
    let transcript = h.dir.join("restart.jsonl");
    let record = |kind: &str, at: u64, extra: serde_json::Value| {
        let mut r = json!({"uuid": format!("{kind}-{at}"), "type": kind, "timestamp": stamp(at)});
        r.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let mut f =
            std::fs::OpenOptions::new().create(true).append(true).open(&transcript).unwrap();
        use std::io::Write;
        writeln!(f, "{r}").unwrap();
    };
    let prompt = |at: u64, pid: &str| {
        record("user", at, json!({"promptId": pid, "message": {"content": "go"}}))
    };
    // A frame under a prompt, as Claude Code sends every one (T-983), from
    // the worker's own pane: this test may run inside another tmux pane,
    // whose key a plain `hook_send` carries, and a frame from a pane the
    // seat left speaks for nothing (round 2).
    let wpane: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    let from = |sid: uuid::Uuid, event: &str, pid: &str, rest: serde_json::Value, pane: &str| {
        let mut body = rest;
        body["prompt_id"] = json!(pid);
        hook_send_from_pane(
            &hook_sock,
            &sid.to_string(),
            event,
            None,
            Some(pane),
            &body.to_string(),
        );
    };
    let under = |sid: uuid::Uuid, event: &str, pid: &str, rest: serde_json::Value| {
        let pane = wpane.borrow().clone().expect("the worker's pane key");
        from(sid, event, pid, rest, &pane);
    };
    // The same frame from a pane no seat has.
    let elsewhere = |sid: uuid::Uuid, event: &str, pid: &str, rest: serde_json::Value| {
        from(sid, event, pid, rest, "99999:%99");
    };
    let close = |at: u64| record("system", at, json!({"subtype": "turn_duration"}));

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 restarts");
    let b = c.create_ticket("mesimon-probe-973 another crown");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    std::thread::sleep(std::time::Duration::from_millis(500));
    crown_turn(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
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
    wait_until(std::time::Duration::from_secs(15), "the worker's pane key", || {
        c.board().sessions.iter().any(|s| s.id == ws && s.pane_key.is_some())
    });
    *wpane.borrow_mut() =
        c.board().sessions.iter().find(|s| s.id == ws).and_then(|s| s.pane_key.clone());
    // The ledger's expectation for W, as the daemon last wrote it: the
    // facts this test waits on, never a clock.
    let ledger = h.paths.state_dir.join("crown.json");
    let expect_of = || -> Option<serde_json::Value> {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&ledger).ok()?).ok()?;
        v["expects"].get(w.to_string()).cloned()
    };
    let bound_to = |pid: &str| {
        let pid = pid.to_string();
        wait_until(
            std::time::Duration::from_secs(15),
            &format!("begun and bound to {pid}"),
            || expect_of().is_some_and(|e| e["began_ms"].is_u64() && e["turn"]["id"] == json!(pid)),
        );
    };
    let begun_unbound = || {
        wait_until(std::time::Duration::from_secs(15), "begun and unbound", || {
            expect_of().is_some_and(|e| e["began_ms"].is_u64() && e.get("turn").is_none())
        });
    };
    let gone = |what: &str| {
        wait_until(std::time::Duration::from_secs(15), what, || expect_of().is_none());
    };
    // How many of W's frames of `event` the daemon has handled: it writes
    // each to the feed as it takes it, so a count past one sent is the
    // frame handled.
    let activity = h.paths.state_dir.join("activity.jsonl");
    let handled = |event: &str| -> usize {
        std::fs::read_to_string(&activity)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| {
                v["kind"] == "hook" && v["event"] == event && v["session"] == ws.to_string()
            })
            .count()
    };
    let after = |event: &str, n: usize| {
        wait_until(std::time::Duration::from_secs(15), &format!("{event} handled"), || {
            handled(event) > n
        });
    };
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    let t0 = epoch_ms();
    prompt(t0 - 60_000, "P0");
    close(t0 - 30_000);
    hook_send_with(&hook_sock, &ws.to_string(), "SessionStart", Some("startup"), &start);
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The brief's own ack follows its paste (T-987): a prompt before it is
    // a person's, and leaves the brief unsent.
    if !mod_road {
        wait_until(std::time::Duration::from_secs(15), "the brief's paste", || {
            lines_with("mesimon-probe-973").iter().any(|l| l.starts_with("mesimon-probe-973"))
        });
    }
    send(ws, "UserPromptSubmit", r#"{"prompt":"the brief"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    send(ws, "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
    let worker = format!("{kw} \"mesimon-probe-973 restarts\"");
    wait_until(std::time::Duration::from_secs(15), "the finished wake", || {
        !lines_with(&format!("{worker} finished its turn")).is_empty()
    });
    crown_turn(&mut c, sa);
    let overrun = format!("{worker} has run its turn ");
    let ask_by = |c: &mut TestClient, text: &str, secs: u64, deliver: Deliver| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver,
                expect_secs: Some(secs),
            },
        ) {
            Response::AgentAsked { held_for_person: false, .. } => {}
            other => panic!("ask_agent: {other:?}"),
        }
    };
    let ask = |c: &mut TestClient, text: &str, secs: u64| ask_by(c, text, secs, Deliver::Idle);
    // Back from a restart, the worker's next frame says it works, under the
    // prompt it runs, and the crown's that it is idle.
    let after_restart = |c: &mut TestClient, pid: &str| {
        under(ws, "PostToolUse", pid, json!({"tool_name": "Read", "tool_response": {}}));
        c.await_state(ws, "the worker running again", |s| *s == SessionState::Running);
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
    };

    let tidy = |c: &mut TestClient, pid: &str| {
        under(ws, "Stop", pid, json!({"stop_hook_active": false}));
        c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
        close(epoch_ms());
        std::thread::sleep(std::time::Duration::from_millis(2000));
        if matches!(
            c.board().live_agent(a).map(|r| r.state.clone()),
            Some(SessionState::Idle { .. })
        ) {
            crown_turn(c, sa);
        }
    };

    let mut base = 0;
    if part == Part::Retire {
        // Held reports (mod): the receipt comes only once released, so the
        // begin is the injected prompt's, in the order this test sets.
        let hold = |on: bool| {
            if mod_road {
                switch("mod-hold-entered", on);
            }
        };
        let release = || {
            if mod_road {
                let n = handled("ModSubmit");
                switch("mod-release", true);
                after("ModSubmit", n);
                switch("mod-release", false);
                switch("mod-hold-entered", false);
            }
        };

        // ---- R1. live: A, then a frame under B with no idle between --------
        hold(true);
        ask(&mut c, "mesimon-probe-983 retire live", 600);
        wait_until(std::time::Duration::from_secs(10), "R1's words", || {
            !lines_with("mesimon-probe-983 retire live").is_empty()
        });
        under(ws, "UserPromptSubmit", "RA", json!({"prompt": "mesimon-probe-983 retire live"}));
        c.await_state(ws, "A running", |s| *s == SessionState::Running);
        release();
        bound_to("RA");
        under(ws, "UserPromptSubmit", "RB", json!({"prompt": "a person's words"}));
        gone("A retired by B's id");
        assert_eq!(lines_with(&overrun).len(), 0, "B is not timed for A");
        assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
        tidy(&mut c, "RB");

        // ---- R1b. the late receipt (mod): the `entered` before the prompt ---
        if mod_road {
            hold(true);
            ask(&mut c, "mesimon-probe-983 receipt first", 600);
            wait_until(std::time::Duration::from_secs(10), "R1b's words", || {
                !lines_with("mesimon-probe-983 receipt first").is_empty()
            });
            release();
            begun_unbound();
            // From a pane the seat left, an id binds nothing (round 2): had
            // RX bound it, RA2 would retire it, and it would not stand.
            elsewhere(ws, "PostToolUse", "RX", json!({"tool_name": "Read", "tool_response": {}}));
            under(
                ws,
                "UserPromptSubmit",
                "RA2",
                json!({"prompt": "mesimon-probe-983 receipt first"}),
            );
            bound_to("RA2");
            tidy(&mut c, "RA2");
        }

        // ---- R2. a restart finds A open; B's hook comes before B's record --
        hold(true);
        ask(&mut c, "mesimon-probe-983 retire restart", 600);
        wait_until(std::time::Duration::from_secs(10), "R2's words", || {
            !lines_with("mesimon-probe-983 retire restart").is_empty()
        });
        prompt(epoch_ms(), "RC");
        under(ws, "UserPromptSubmit", "RC", json!({"prompt": "mesimon-probe-983 retire restart"}));
        c.await_state(ws, "A running", |s| *s == SessionState::Running);
        release();
        bound_to("RC");
        drop(c);
        h.restart();
        c = h.client("crown_overrun_retire_2");
        // The restore found RC open and kept it (rewritten, as restored).
        let _ = c.board();
        bound_to("RC");
        under(ws, "UserPromptSubmit", "RD", json!({"prompt": "a person's words"}));
        c.await_state(ws, "B running", |s| *s == SessionState::Running);
        gone("the restored A retired by B's id");
        assert_eq!(lines_with(&overrun).len(), 0, "the restored A is not B's");
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
        tidy(&mut c, "RD");

        // ---- R3. an id-less begin binds its own permission prompt's id ------
        // The cache holds RD's id; the begin's frame carries none, so it
        // stays unbound, a stale pane's id binds nothing, and the
        // permission prompt's id is the turn's. A restart then finds that
        // turn open and times it from its begin.
        hold(true);
        ask(&mut c, "mesimon-probe-983 idless", 8);
        wait_until(std::time::Duration::from_secs(10), "R3's words", || {
            !lines_with("mesimon-probe-983 idless").is_empty()
        });
        send(ws, "UserPromptSubmit", r#"{"prompt":"mesimon-probe-983 idless"}"#);
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        let begun = epoch_ms();
        release();
        begun_unbound();
        let permission = json!({"tool_name": "Bash", "tool_input": {"command": "true"}});
        // Had the stale pane's RX bound it, RE would retire it.
        elsewhere(ws, "PermissionRequest", "RX", permission.clone());
        prompt(begun, "RE");
        under(ws, "PermissionRequest", "RE", permission);
        bound_to("RE");
        drop(c);
        h.restart();
        c = h.client("crown_overrun_retire_3");
        after_restart(&mut c, "RE");
        wait_until(std::time::Duration::from_secs(25), "the bound turn's overrun", || {
            lines_with(&overrun).len() == 1
        });
        assert!(epoch_ms() >= begun + 8_000, "timed from the begin before the restart");
        assert!(lines_with(&overrun)[0].contains("after a restart"), "{:?}", lines_with(&overrun));
        return;
    }
    if part == Part::Unheard && mod_road {
        // A submit dispatched after a restart, before any id is heard: the
        // daemon's cache is empty, and the dispatch says the baseline is
        // unknown.
        let dispatch_unheard = |words: &str, name: &str| -> TestClient {
            h.restart();
            let mut c = h.client(name);
            send(sa, "Stop", r#"{"stop_hook_active":false}"#);
            c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
            // The worker's bridge polls this daemon on its own backoff (up to
            // 2 s after the restart): words sent now before it has go down
            // the paste road, with no frame. A pong comes back only through
            // a bridge that polled.
            wait_until(std::time::Duration::from_secs(15), "the worker's bridge", || {
                matches!(c.request(Command::ModPing { session: ws }), Response::ModPonged { .. })
            });
            switch("mod-hold-entered", true);
            ask_by(&mut c, words, 600, Deliver::Now);
            wait_until(std::time::Duration::from_secs(15), "the words", || {
                !lines_with(words).is_empty()
            });
            wait_until(std::time::Duration::from_secs(15), "unknown baseline", || {
                expect_of().is_some_and(|e| e.get("base").is_none() && e["frame"].is_string())
            });
            // Its report is held until the release; the crown's own wakes
            // go through at once (the switch holds every pane's submit).
            switch("mod-hold-entered", false);
            c
        };
        let release = || {
            let n = handled("ModSubmit");
            switch("mod-release", true);
            after("ModSubmit", n);
            switch("mod-release", false);
        };

        // ---- U1. A's prompt first, A's end, B's, then A's late report ------
        for restart in [false, true] {
            drop(c);
            c = dispatch_unheard(
                &format!("mesimon-probe-983 unheard {restart}"),
                "crown_overrun_unheard_1",
            );
            // From a pane the seat left, an id is heard by nothing: A's is
            // then the only one (below).
            elsewhere(ws, "PostToolUse", "UX", json!({"tool_name": "Read", "tool_response": {}}));
            if restart {
                drop(c);
                h.restart();
                c = h.client("crown_overrun_unheard_2");
                send(sa, "Stop", r#"{"stop_hook_active":false}"#);
                c.await_state(sa, "the crown idle again", |s| {
                    matches!(s, SessionState::Idle { .. })
                });
            }
            let (pa, pb) = (format!("UA{restart}"), format!("UB{restart}"));
            prompt(epoch_ms(), &pa);
            under(ws, "UserPromptSubmit", &pa, json!({"prompt": "A"}));
            c.await_state(ws, "A running", |s| *s == SessionState::Running);
            wait_until(std::time::Duration::from_secs(15), "A's id heard alone", || {
                expect_of().is_some_and(|e| e["heard"] == json!([pa]))
            });
            under(ws, "Stop", &pa, json!({"stop_hook_active": false}));
            c.await_state(ws, "A idle", |s| matches!(s, SessionState::Idle { .. }));
            close(epoch_ms());
            prompt(epoch_ms(), &pb);
            under(ws, "UserPromptSubmit", &pb, json!({"prompt": "a person's words"}));
            c.await_state(ws, "B running", |s| *s == SessionState::Running);
            release();
            gone(&format!("A's report begins nothing in B ({restart})"));
            assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
            tidy(&mut c, &pb);
        }

        // ---- U2. the stated limit: R's id, then A's, then A's report -------
        // R runs under an id this daemon never heard; the words go down a
        // submit held to R's end, and a second restart takes the owed
        // entry, so only the report could begin A. R's id and A's are both
        // new: ambiguous, and A does not begin.
        prompt(epoch_ms(), "UR");
        under(ws, "UserPromptSubmit", "UR", json!({"prompt": "a long one"}));
        c.await_state(ws, "R running", |s| *s == SessionState::Running);
        drop(c);
        c = dispatch_unheard("mesimon-probe-983 unheard R", "crown_overrun_unheard_4");
        drop(c);
        h.restart();
        c = h.client("crown_overrun_unheard_5");
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
        under(ws, "PostToolUse", "UR", json!({"tool_name": "Read", "tool_response": {}}));
        under(ws, "Stop", "UR", json!({"stop_hook_active": false}));
        c.await_state(ws, "R idle", |s| matches!(s, SessionState::Idle { .. }));
        close(epoch_ms());
        prompt(epoch_ms(), "UA");
        under(ws, "UserPromptSubmit", "UA", json!({"prompt": "mesimon-probe-983 unheard R"}));
        c.await_state(ws, "A running", |s| *s == SessionState::Running);
        wait_until(std::time::Duration::from_secs(15), "both ids new", || {
            expect_of().is_some_and(|e| e["heard"] == json!(["UR", "UA"]))
        });
        release();
        gone("ambiguous: A does not begin");
        return;
    }
    if !entered {
        // ---- 0. begun, then a restart: the same turn runs on under its prompt ----
        // Its prompt id is in the transcript with nothing after it that ends the
        // turn, so the expectation comes back with its count and wakes the crown
        // once, saying it may be late.
        ask(&mut c, "mesimon-probe-973 zero", 6);
        wait_until(std::time::Duration::from_secs(10), "the zeroth ask's words", || {
            !lines_with("mesimon-probe-973 zero").is_empty()
        });
        under(ws, "UserPromptSubmit", "Z", json!({"prompt": "mesimon-probe-973 zero"}));
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        let begun = epoch_ms();
        prompt(begun, "Z");
        drop(c);
        h.restart();
        c = h.client("crown_overrun_restart_1");
        after_restart(&mut c, "Z");
        wait_until(std::time::Duration::from_secs(25), "the restored turn's overrun", || {
            lines_with(&overrun).len() == 1
        });
        assert!(epoch_ms() >= begun + 6_000, "timed from the begin before the restart");
        assert_eq!(
            lines_with(&overrun).iter().filter(|l| l.contains("after a restart")).count(),
            1,
            "{:?}",
            lines_with(&overrun)
        );
        assert!(read(&mut c, sa, &kw).unwrap().overrun.is_some());
        std::thread::sleep(std::time::Duration::from_millis(2000));
        assert_eq!(lines_with(&overrun).len(), 1, "once");
        tidy(&mut c, "Z");
        base = lines_with(&overrun).len();

        // ---- 1. begun, then a restart: A's close and B's prompt in the gap ------
        // Round 3's counterexample: A's prompt, A's close a moment after the
        // begin, B's prompt and B's tool. A's prompt id is followed by a close
        // and another prompt's, so B's turn is not timed against A's deadline.
        ask(&mut c, "mesimon-probe-973 one", 3);
        wait_until(std::time::Duration::from_secs(10), "the first ask's words", || {
            !lines_with("mesimon-probe-973 one").is_empty()
        });
        prompt(epoch_ms(), "A");
        under(ws, "UserPromptSubmit", "A", json!({"prompt": "mesimon-probe-973 one"}));
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        drop(c);
        h.restart_after(|| {
            let now = epoch_ms();
            close(now);
            prompt(now + 1, "B");
            record(
                "assistant",
                now + 2,
                json!({"message": {"content": [
                {"type": "tool_use", "id": "b", "name": "Bash", "input": {}}]}}),
            );
        });
        c = h.client("crown_overrun_restart_2");
        after_restart(&mut c, "B");
        std::thread::sleep(std::time::Duration::from_millis(6_000));
        assert_eq!(lines_with(&overrun).len(), base, "B's turn is not A's");
        assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
        tidy(&mut c, "B");

        // ---- 2. begun, then a restart before its prompt's record is written ----
        // The record lands 80-180 ms after the hook (measured, 2.1.310): a
        // restart inside that finds no record of the turn, and keeps nothing.
        ask(&mut c, "mesimon-probe-973 two", 3);
        wait_until(std::time::Duration::from_secs(15), "the second ask's words", || {
            !lines_with("mesimon-probe-973 two").is_empty()
        });
        under(ws, "UserPromptSubmit", "C", json!({"prompt": "mesimon-probe-973 two"}));
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        drop(c);
        h.restart();
        c = h.client("crown_overrun_restart_3");
        // The socket is up before the ledger is read; the writer answers only
        // after it, so a snapshot dates the record past the restore.
        let _ = c.board();
        prompt(epoch_ms(), "C");
        after_restart(&mut c, "C");
        std::thread::sleep(std::time::Duration::from_millis(6_000));
        assert_eq!(lines_with(&overrun).len(), base, "a turn not found is not kept");
        assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
        tidy(&mut c, "C");
    }

    // ---- 4. the mod pass: a submit whose entered comes after a restart ------
    // Its prompt is relayed before its `entered`, in the engine's order: the
    // turn begins at the report, bound to that prompt.
    if entered && mod_road {
        switch("mod-hold-entered", true);
        ask(&mut c, "mesimon-probe-973 three", 4);
        wait_until(std::time::Duration::from_secs(15), "the third ask's words", || {
            !lines_with("mesimon-probe-973 three").is_empty()
        });
        drop(c);
        h.restart();
        c = h.client("crown_overrun_restart_5");
        send(sa, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sa, "the crown idle again", |s| matches!(s, SessionState::Idle { .. }));
        prompt(epoch_ms(), "T3");
        under(ws, "UserPromptSubmit", "T3", json!({"prompt": "mesimon-probe-973 three"}));
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        switch("mod-release", true);
        bound_to("T3");
        // Read where it stands, not by its wake: a wake still owed takes
        // the overrun into its own line (the higher cause).
        wait_until(std::time::Duration::from_secs(25), "the overrun of the held submit", || {
            read(&mut c, sa, &kw).unwrap().overrun.is_some()
        });
        crown_turn(&mut c, sa);
        switch("mod-hold-entered", false);
        switch("mod-release", false);
        tidy(&mut c, "T3");

        // ---- 4b. an entered that comes after another prompt has run ---------
        // The addendum (T-982's fourth read): A's turn runs and ends without
        // its report, B begins, and A's `entered` comes then. Two prompts new
        // since the dispatch: A's words' turn is not B's, and nothing begins.
        for restart in [false, true] {
            switch("mod-hold-entered", true);
            let words = format!("mesimon-probe-973 late {restart}");
            ask(&mut c, &words, 4);
            wait_until(std::time::Duration::from_secs(15), "the late ask's words", || {
                !lines_with(&words).is_empty()
            });
            if restart {
                drop(c);
                h.restart();
                c = h.client("crown_overrun_restart_late");
                send(sa, "Stop", r#"{"stop_hook_active":false}"#);
                c.await_state(sa, "the crown idle again", |s| {
                    matches!(s, SessionState::Idle { .. })
                });
            }
            let (pa, pb) = (format!("LA{restart}"), format!("LB{restart}"));
            prompt(epoch_ms(), &pa);
            under(ws, "UserPromptSubmit", &pa, json!({"prompt": words}));
            c.await_state(ws, "A running", |s| *s == SessionState::Running);
            under(ws, "Stop", &pa, json!({"stop_hook_active": false}));
            c.await_state(ws, "A idle", |s| matches!(s, SessionState::Idle { .. }));
            close(epoch_ms());
            prompt(epoch_ms(), &pb);
            under(ws, "UserPromptSubmit", &pb, json!({"prompt": "a person's words"}));
            c.await_state(ws, "B running", |s| *s == SessionState::Running);
            let n = handled("ModSubmit");
            switch("mod-release", true);
            after("ModSubmit", n);
            gone(&format!("B is not timed for A ({restart})"));
            assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
            switch("mod-hold-entered", false);
            switch("mod-release", false);
            tidy(&mut c, &pb);
        }

        return;
    }

    // ---- 5. the crown taken off, or moved and back, takes them --------------
    for transfer in [false, true] {
        ask(&mut c, &format!("mesimon-probe-973 four {transfer}"), 4);
        wait_until(std::time::Duration::from_secs(15), "the fourth ask's words", || {
            !lines_with(&format!("mesimon-probe-973 four {transfer}")).is_empty()
        });
        under(ws, "UserPromptSubmit", "F", json!({"prompt": "four"}));
        c.await_state(ws, "running", |s| *s == SessionState::Running);
        if transfer {
            assert!(matches!(c.request(Command::CrownTicket { id: b }), Response::Ok));
        } else {
            assert!(matches!(c.request(Command::Uncrown), Response::Ok));
        }
        assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
        std::thread::sleep(std::time::Duration::from_millis(6000));
        assert_eq!(lines_with(&overrun).len(), base, "the outgoing crown's expectation went");
        assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
        send(ws, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(ws, "idle", |s| matches!(s, SessionState::Idle { .. }));
        close(epoch_ms());
        std::thread::sleep(std::time::Duration::from_millis(2000));
        if matches!(
            c.board().live_agent(a).map(|r| r.state.clone()),
            Some(SessionState::Idle { .. })
        ) {
            crown_turn(&mut c, sa);
        }
    }
}

/// T-983: a Codex worker's begun expectation outlives a restart by the
/// runtime's turn id. A start's launch turn whose id comes one observation
/// after the edge into `Running` (Codex 0.164.0 sends `turn/started` a frame
/// after `thread/status/changed`) takes that id; after a restart it is owed
/// nothing until the runtime names its turn again, the same id, and then
/// wakes the crown once, `after a restart`. An ask's turn the runtime has
/// replaced by another during the restart wakes nobody.
#[test]
fn a_codex_expectation_outlives_a_restart_by_its_turn_id() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
                        cat | while IFS= read -r line; do \
                        printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_with_env(
        "crown_codex_expect",
        Some(STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    h.restart();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_codex_expect");
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
    let draws = std::cell::Cell::new(0u32);
    let control = |id: uuid::Uuid, mut value: serde_json::Value| {
        // Each instruction differs from the last, or the runtime skips it.
        draws.set(draws.get() + 1);
        value["n"] = json!(draws.get());
        let path = mesimon_daemon::agents::codex::snapshot_path(&h.paths, id);
        let control = std::path::PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    };
    let running = json!({"state": "running"});
    let idle = json!({"state": "idle", "stop_reason": "end_turn"});

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-983 codex");
    let kw = key_of(&mut c, w);
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    assert!(matches!(
        c.request(Command::SetAgentProvider {
            provider: mesimon_core::board::AgentProvider::Codex
        }),
        Response::Ok
    ));
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
                expect_secs: Some(8),
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(std::time::Duration::from_secs(20), "W's Codex seat", || {
        c.board().live_agent(w).is_some_and(|s| s.kind == SessionKind::Codex)
    });
    let ws = c.board().live_agent(w).unwrap().id;
    c.await_state(ws, "the synthetic Codex at its composer", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    let worker = format!("{kw} \"mesimon-probe-983 codex\"");
    let overrun = format!("{worker} has run its turn ");

    // ---- 1. the launch turn, its id a frame late, then a restart -----------
    control(ws, json!({"state": running}));
    c.await_state(ws, "the launch turn running", |s| *s == SessionState::Running);
    let begun = epoch_ms();
    control(ws, json!({"state": running, "turn_id": "launch-turn"}));
    wait_until(std::time::Duration::from_secs(10), "the turn id observed", || {
        c.board()
            .sessions
            .iter()
            .any(|s| s.id == ws && s.codex_turn_id.as_deref() == Some("launch-turn"))
    });
    drop(c);
    h.restart();
    let mut c = h.client("crown_codex_expect_2");
    stop(&mut c, sa);
    wait_until(std::time::Duration::from_secs(25), "the restored launch turn's overrun", || {
        lines_with(&overrun).len() == 1
    });
    assert!(epoch_ms() >= begun + 8_000, "timed from the begin before the restart");
    assert!(lines_with(&overrun)[0].contains("after a restart"), "{:?}", lines_with(&overrun));
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_some());
    control(ws, json!({"state": idle, "turn_id": "launch-turn"}));
    c.await_state(ws, "the launch turn over", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(10), "overrun gone with its turn", || {
        read(&mut c, sa, &kw).unwrap().overrun.is_none()
    });
    if matches!(c.board().live_agent(a).map(|r| r.state.clone()), Some(SessionState::Idle { .. })) {
        start(&mut c, sa);
        stop(&mut c, sa);
    }

    // ---- 2. an ask's turn, replaced by another while no daemon listened ----
    let probe = "mesimon-probe-983 replaced";
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: probe.into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: Some(6),
        },
    ) {
        Response::AgentAsked { held_for_person: false, .. } => {}
        other => panic!("ask_agent: {other:?}"),
    }
    let input = h.dir.join(format!("{ws}.input"));
    wait_until(std::time::Duration::from_secs(15), "the ask's words in W's pane", || {
        String::from_utf8_lossy(&std::fs::read(&input).unwrap_or_default()).contains(probe)
    });
    control(ws, json!({"state": running, "turn_id": "ask-turn"}));
    c.await_state(ws, "the ask's turn running", |s| *s == SessionState::Running);
    std::thread::sleep(std::time::Duration::from_millis(500));
    drop(c);
    h.restart_after(|| control(ws, json!({"state": running, "turn_id": "another-turn"})));
    let mut c = h.client("crown_codex_expect_3");
    stop(&mut c, sa);
    std::thread::sleep(std::time::Duration::from_millis(10_000));
    assert_eq!(lines_with(&overrun).len(), 1, "another turn is not the ask's");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
    control(ws, json!({"state": idle, "turn_id": "another-turn"}));
    c.await_state(ws, "the other turn over", |s| matches!(s, SessionState::Idle { .. }));
    if matches!(c.board().live_agent(a).map(|r| r.state.clone()), Some(SessionState::Idle { .. })) {
        start(&mut c, sa);
        stop(&mut c, sa);
    }

    // ---- 3. verified after a restart, then the runtime names another turn --
    // Round 1: A's end and B's start fell between two polls, so the seat
    // never left `Running`; B's id ends A's expectation, verified or not.
    let probe = "mesimon-probe-983 verified";
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: probe.into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: Some(8),
        },
    ) {
        Response::AgentAsked { held_for_person: false, .. } => {}
        other => panic!("ask_agent: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(15), "the third ask's words in W's pane", || {
        String::from_utf8_lossy(&std::fs::read(&input).unwrap_or_default()).contains(probe)
    });
    control(ws, json!({"state": running, "turn_id": "turn-a"}));
    c.await_state(ws, "A running", |s| *s == SessionState::Running);
    std::thread::sleep(std::time::Duration::from_millis(500));
    drop(c);
    h.restart();
    let mut c = h.client("crown_codex_expect_4");
    stop(&mut c, sa);
    // The runtime names A again (verified), then B, with no idle between.
    control(ws, json!({"state": running, "turn_id": "turn-a", "again": true}));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    control(ws, json!({"state": running, "turn_id": "turn-b"}));
    std::thread::sleep(std::time::Duration::from_millis(10_000));
    assert_eq!(lines_with(&overrun).len(), 1, "B is not timed for A");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
}

/// T-973, round 2: an expectation armed on words a wake carries, while the
/// wake waits for its worktree to be rebuilt (T-278), waits with it: held
/// across several ticks by the repository's init script, it is neither
/// lost nor begun, no `overrun` stands, and once the tree is back and the
/// words go in, the turn they begin wakes the crown exactly once.
#[test]
fn an_expectation_waits_for_its_wake_behind_a_worktree_rebuild() {
    let Some(h) = Harness::boot_with_env(
        "crown_overrun_rebuild",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let hold = h.dir.join("hold-init");
    let script = repo.join(mesimon_core::workspace::INIT_SCRIPT);
    std::fs::write(
        &script,
        format!("#!/bin/sh\nwhile [ -f '{}' ]; do sleep 0.2; done\n", hold.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_overrun_rebuild");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 rebuilt");
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
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let tree = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    let branch = wt_of(&mut c, w).unwrap().branch;
    wait_until(std::time::Duration::from_secs(15), "W's start to land", || {
        c.board().live_agent(w).is_some_and(|s| s.state.has_pane())
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::fs::write(tree.join("w.txt"), "w\n").unwrap();
    git(&tree, &["add", "w.txt"]);
    git(&tree, &["commit", "-qm", "w"]);
    git(&repo, &["merge", "-q", "--squash", &branch]);
    git(&repo, &["commit", "-qm", "w (#9)"]);
    wait_until(std::time::Duration::from_secs(20), "the board to read the squash", || {
        wt_of(&mut c, w).is_some_and(|wt| wt.merged)
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    // Parked and archived, the landed tree is reclaimed and the branch kept,
    // so the next wake rebuilds the tree first (T-278).
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
    ) {
        Response::AgentTicket { .. } => {}
        other => panic!("sleep_agent: {other:?}"),
    }
    c.await_state(ws, "parked", |s| *s == SessionState::Sleeping);
    assert!(matches!(c.request(Command::ArchiveTicket { id: w }), Response::Ok));
    wait_until(std::time::Duration::from_secs(20), "the archive to reclaim W's tree", || {
        !tree.exists() && wt_of(&mut c, w).is_some_and(|wt| wt.status == "evicted")
    });
    assert!(matches!(c.request(Command::UnarchiveTicket { id: w }), Response::Ok));

    // ---- the words ride a wake held behind the rebuild ---------------------
    std::fs::write(&hold, "").unwrap();
    let probe = "mesimon-probe-973 behind the rebuild";
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: probe.into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: Some(3),
        },
    ) {
        Response::AgentAsked { held_for_person: false, wake, .. } => {
            assert_eq!(wake, Some(AskWake::Woken), "the words woke it, behind the rebuild");
        }
        other => panic!("ask_agent: {other:?}"),
    }
    let worker = format!("{kw} \"mesimon-probe-973 rebuilt\"");
    let overrun = format!("{worker} has run its turn ");
    // Several ticks (each look runs every second) with the rebuild held.
    std::thread::sleep(std::time::Duration::from_millis(5000));
    assert!(!c.board().live_agent(w).is_some_and(|s| s.state.has_pane()), "still rebuilding");
    assert_eq!(lines_with(&overrun), 0, "nothing begun, nothing past");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());

    // ---- the tree back, the words in, their turn timed ----------------------
    std::fs::remove_file(&hold).unwrap();
    wait_until(std::time::Duration::from_secs(30), "W up in its rebuilt tree", || {
        c.board().live_agent(w).is_some_and(|s| s.id == ws && s.state.has_pane())
    });
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    c.await_state(ws, "up after the wake", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(std::time::Duration::from_secs(15), "the words to reach W", || {
        lines_with(probe) > 0
    });
    hook_send(
        &hook_sock,
        &ws.to_string(),
        "UserPromptSubmit",
        &format!(r#"{{"prompt":"{probe}"}}"#),
    );
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    wait_until(std::time::Duration::from_secs(20), "the overrun of the words' own turn", || {
        lines_with(&overrun) == 1
    });
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert_eq!(lines_with(&overrun), 1, "exactly one");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_some());
}

/// T-973, round 3: the expectation kept behind a held rebuild is the words
/// on the resume's own: the resume carries the crown's ask id, and the
/// replay tags that ask alone. Words a person sends while the tree is
/// rebuilt replace the crown's on the resume, and their turn wakes nobody
/// for the crown's ask. The crown taken off drops the expectation (an
/// explicit drop), and a parked seat killed before the replay makes the
/// replay fail and drop it too: no overrun in any of the three.
#[test]
fn an_expectation_behind_a_rebuild_is_its_own_words_alone() {
    let Some(h) = Harness::boot_with_env(
        "crown_overrun_rebuild_owned",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let hold = h.dir.join("hold-init");
    let script = repo.join(mesimon_core::workspace::INIT_SCRIPT);
    std::fs::write(
        &script,
        format!("#!/bin/sh\nwhile [ -f '{}' ]; do sleep 0.2; done\n", hold.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_overrun_rebuild_owned");
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

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-973 owned");
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
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let tree = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    let branch = wt_of(&mut c, w).unwrap().branch;
    wait_until(std::time::Duration::from_secs(15), "W's start to land", || {
        c.board().live_agent(w).is_some_and(|s| s.state.has_pane())
    });
    let ws = c.board().live_agent(w).unwrap().id;
    std::fs::write(tree.join("w.txt"), "w\n").unwrap();
    git(&tree, &["add", "w.txt"]);
    git(&tree, &["commit", "-qm", "w"]);
    git(&repo, &["merge", "-q", "--squash", &branch]);
    git(&repo, &["commit", "-qm", "w (#9)"]);
    wait_until(std::time::Duration::from_secs(20), "the board to read the squash", || {
        wt_of(&mut c, w).is_some_and(|wt| wt.merged)
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, ws);
    stop(&mut c, ws);
    // Parked and archived, the landed tree is reclaimed and the branch kept,
    // so the next wake rebuilds it (T-278); held there by the init script.
    let reclaim = |c: &mut TestClient| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
        ) {
            Response::AgentTicket { .. } => {}
            other => panic!("sleep_agent: {other:?}"),
        }
        c.await_state(ws, "parked", |s| *s == SessionState::Sleeping);
        assert!(matches!(c.request(Command::ArchiveTicket { id: w }), Response::Ok));
        wait_until(std::time::Duration::from_secs(20), "the archive to reclaim W's tree", || {
            !tree.exists() && wt_of(c, w).is_some_and(|wt| wt.status == "evicted")
        });
        assert!(matches!(c.request(Command::UnarchiveTicket { id: w }), Response::Ok));
        std::fs::write(&hold, "").unwrap();
    };
    let ask = |c: &mut TestClient, text: &str| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: Some(3),
            },
        ) {
            Response::AgentAsked { held_for_person: false, wake, .. } => {
                assert_eq!(wake, Some(AskWake::Woken), "the words woke it, behind the rebuild");
            }
            other => panic!("ask_agent: {other:?}"),
        }
    };
    // The tree back and the agent up, its words in and their turn running.
    let replay = |c: &mut TestClient, words: &str| {
        std::fs::remove_file(&hold).unwrap();
        wait_until(std::time::Duration::from_secs(30), "W up in its rebuilt tree", || {
            c.board().live_agent(w).is_some_and(|s| s.id == ws && s.state.has_pane())
        });
        hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
        c.await_state(ws, "up after the wake", |s| matches!(s, SessionState::Idle { .. }));
        wait_until(std::time::Duration::from_secs(15), "the words to reach W", || {
            lines_with(words) > 0
        });
        hook_send(
            &hook_sock,
            &ws.to_string(),
            "UserPromptSubmit",
            &format!(r#"{{"prompt":"{words}"}}"#),
        );
        c.await_state(ws, "running", |s| *s == SessionState::Running);
    };
    let worker = format!("{kw} \"mesimon-probe-973 owned\"");
    let overrun = format!("{worker} has run its turn ");
    let ms = |n| std::thread::sleep(std::time::Duration::from_millis(n));

    // ---- a person's words replace the crown's on the resume ----------------
    reclaim(&mut c);
    ask(&mut c, "mesimon-probe-973 crown A");
    ms(1500);
    if let Response::Err { message } = c.request(Command::PromptSession {
        ticket: w,
        text: "mesimon-probe-973 person B".into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    }) {
        panic!("the person's words: {message}");
    }
    ms(2500);
    replay(&mut c, "mesimon-probe-973 person B");
    assert_eq!(lines_with("mesimon-probe-973 crown A"), 0, "the crown's words never went in");
    ms(6000);
    assert_eq!(lines_with(&overrun), 0, "B's turn is not A's");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
    stop(&mut c, ws);

    // ---- the crown taken off while the rebuild holds ------------------------
    reclaim(&mut c);
    ask(&mut c, "mesimon-probe-973 crown C");
    ms(1500);
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    replay(&mut c, "mesimon-probe-973 crown C");
    ms(6000);
    assert_eq!(lines_with(&overrun), 0, "the outgoing crown's expectation went");
    stop(&mut c, ws);

    // ---- the parked seat killed: the replay fails ---------------------------
    reclaim(&mut c);
    ask(&mut c, "mesimon-probe-973 crown D");
    ms(1500);
    assert!(matches!(c.request(Command::KillSession { id: ws }), Response::Ok));
    std::fs::remove_file(&hold).unwrap();
    ms(8000);
    assert_eq!(lines_with("mesimon-probe-973 crown D"), 0, "nothing went in");
    assert_eq!(lines_with(&overrun), 0, "a failed replay times nothing");
    assert!(read(&mut c, sa, &kw).unwrap().overrun.is_none());
}

/// T-984: a person's prompt into a worker the crown's ask is waking, before
/// the board's first paste of the ask, is the person's, and it drops the
/// ask. The pane's composer is painted and reading; the barrier is the
/// wake's `SessionStart`, held back (Claude Code 2.1.310 can paint its box
/// before that hook fires), so the parked words have not gone in when the
/// person types and presses Enter in the pane. The ask reads `dropped`, by
/// the person, with why; no byte of the crown's and no retry Enter follow,
/// the hook arriving after it included. The person's turn then goes on
/// with a tool call (the resume's `SessionStart` reads as idle), and the
/// crown's next ask is admitted while it runs, queued, its words held until
/// that turn's end. The controls: that ask, and one
/// more on a wake's parked road, each followed by a prompt AFTER the
/// board's paste, take the ack road and speak of their ask at the turn end.
#[test]
fn a_person_s_prompt_before_a_parked_ask_s_paste_drops_it() {
    let Some(h) = Harness::boot_with_env(
        "crown_ask_parked_prompt",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_CLAUDE_ROAD", "hooks"),
            ("MESIMON_COMPOSER_WAIT_MS", "60000"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client("crown_ask_parked_prompt");
    let feed = h.paths.state_dir.join("activity.jsonl");
    let read_file = |p: &std::path::Path| std::fs::read_to_string(p).unwrap_or_default();
    let start_with = |c: &mut TestClient, sid: uuid::Uuid, prompt: &str| {
        let payload = json!({ "prompt": prompt }).to_string();
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", &payload);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| start_with(c, sid, "go");
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let settle = || std::thread::sleep(std::time::Duration::from_millis(1500));

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-984 worker");
    let (ka, kw) = (key_of(&mut c, a), key_of(&mut c, w));
    let head = format!("{kw} \"mesimon-probe-984 worker\"");
    let got_w = h.dir.join(format!("got-{kw}.txt"));
    let in_w = || read_file(&got_w);
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
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let got_crown = h.dir.join(format!("got-{ka}.txt"));
    let crown_said = || read_file(&got_crown);
    let taken = std::cell::Cell::new(0);
    let crown_takes = |c: &mut TestClient| {
        wait_until(std::time::Duration::from_secs(10), "a wake in the crown's pane", || {
            crown_said().lines().count() > taken.get()
        });
        taken.set(crown_said().lines().count());
        start(c, sa);
        stop(c, sa);
    };
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "W's finish on the crown", || {
        crown_said().contains(&format!("{head} finished its turn"))
    });
    crown_takes(&mut c);

    let asked = |c: &mut TestClient| read(c, sa, &kw).unwrap().asked.expect("an ask on W");
    let ask = |c: &mut TestClient, text: &str| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentAskTicket {
                key: kw.clone(),
                text: text.into(),
                seen: v.seen,
                plan: false,
                deliver: Deliver::Idle,
                expect_secs: None,
            },
        ) {
            Response::AgentAsked { asked, .. } => asked.expect("the record rides the receipt"),
            other => panic!("ask_agent {text}: {other:?}"),
        }
    };
    let park_w = |c: &mut TestClient| {
        let v = read(c, sa, &kw).unwrap();
        match c.send(
            Principal::Agent { session: sa },
            Command::AgentSleepTicket { key: kw.clone(), seen: v.seen },
        ) {
            Response::AgentTicket { .. } => {}
            other => panic!("sleep_agent: {other:?}"),
        }
        c.await_state(ws, "sleeping", |s| *s == SessionState::Sleeping);
    };
    let woken = |c: &mut TestClient| {
        wait_until(std::time::Duration::from_secs(15), "W woken", || {
            c.board().sessions.iter().any(|s| s.id == ws && s.state.has_pane())
        });
    };
    let wake_for = |id: &str| {
        let line = format!("{head} ended a turn after ask {id} was sent; receipt unconfirmed");
        wait_until(std::time::Duration::from_secs(20), "the wake for the ask", || {
            crown_said().contains(&line)
        });
    };

    // ---- the race: the person's Enter before the board's first paste ----
    park_w(&mut c);
    let first = ask(&mut c, "mesimon-probe-9841 the crown's words");
    assert_eq!(first.status, "sent", "{first:?}");
    woken(&mut c);
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!in_w().contains("mesimon-probe-9841"), "parked until SessionStart");
    let sid = c.board().sessions.iter().find(|s| s.id == ws).unwrap().sid16();
    let before = in_w();
    let fed_before = read_file(&feed).len();
    let person = "mesimon-probe-9842 the person's words";
    assert!(tmux(&tmux_sock)
        .args(["send-keys", "-t", &sid, "-l", person])
        .status()
        .unwrap()
        .success());
    assert!(tmux(&tmux_sock).args(["send-keys", "-t", &sid, "Enter"]).status().unwrap().success());
    wait_until(std::time::Duration::from_secs(5), "the person's line in W", || {
        in_w().contains(person)
    });
    // The stub fires no hook: the prompt's, as Claude Code would send it.
    start_with(&mut c, ws, person);
    settle();
    let dropped = asked(&mut c);
    assert_eq!(
        (dropped.status.as_str(), dropped.ask.as_str(), dropped.by.as_str()),
        ("dropped", first.ask.as_str(), "person"),
        "{dropped:?}"
    );
    assert_eq!(dropped.why, "a person's prompt arrived before these words were pasted");
    let fed = read_file(&feed)[fed_before..].to_string();
    assert!(
        fed.lines().any(|l| l.contains("\"crown_ask_dropped_by_hand\"") && l.contains(&first.ask)),
        "{fed}"
    );
    assert!(!fed.contains("\"prompt_submitted\""), "no ack of the board's words: {fed}");
    // The arming hook comes late: nothing of the crown's, and no Enter.
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    settle();
    settle();
    assert_eq!(in_w(), format!("{before}{person}\n"), "only the person's line reached W");
    // A resume's `SessionStart` reads as idle; the person's turn goes on
    // with a tool call of its own, no prompt, so W is working again.
    hook_send(
        &hook_sock,
        &ws.to_string(),
        "PreToolUse",
        r#"{"tool_name":"Bash","tool_input":{"command":"grep -n x"}}"#,
    );
    c.await_state(ws, "running again", |s| *s == SessionState::Running);

    // The crown asks again while the person's turn runs: admitted, queued
    // behind that turn by the ordinary guard, then pasted at its end, and
    // the prompt after the paste takes the ack road.
    let said = crown_said().len();
    let second = ask(&mut c, "mesimon-probe-9843 asked again");
    assert_ne!(second.ask, first.ask);
    assert_eq!(second.status, "queued", "{second:?}");
    assert_eq!(asked(&mut c).status, "queued");
    settle();
    assert!(!in_w().contains("mesimon-probe-9843"), "held while the turn runs");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the second ask in W", || {
        in_w().contains("mesimon-probe-9843")
    });
    assert!(!crown_said()[said..].contains(&format!("ask {}", first.ask)), "{}", crown_said());
    start_with(&mut c, ws, "mesimon-probe-9843 asked again");
    assert_eq!(asked(&mut c).status, "sent");
    stop(&mut c, ws);
    wake_for(&second.ask);
    crown_takes(&mut c);

    // ---- the control on the wake's parked road ----
    park_w(&mut c);
    let third = ask(&mut c, "mesimon-probe-9844 after the box");
    woken(&mut c);
    hook_send(&hook_sock, &ws.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    wait_until(std::time::Duration::from_secs(15), "the parked words pasted", || {
        in_w().contains("mesimon-probe-9844")
    });
    start_with(&mut c, ws, "mesimon-probe-9844 after the box");
    settle();
    let acked = asked(&mut c);
    assert_eq!((acked.status.as_str(), acked.ask.as_str()), ("sent", third.ask.as_str()));
    stop(&mut c, ws);
    wake_for(&third.ask);
}
