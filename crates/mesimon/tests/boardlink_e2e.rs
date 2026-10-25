//! Boards that reach each other (T-733): two daemons on one machine, one
//! `links.toml` between them. A person links the boards from one side and
//! both list each other; the crown of one asks the crown of the other with
//! `ask_board`, the words land on that crown's card by the queue, wrapped in
//! the board's `Board ask` template, and go into its pane at its idle; the
//! reply is the same tool back, read as a reply on both sides; a supervised
//! board holds the words for a person's send; a board with no crown, or no
//! daemon up, refuses in words; and the feed never carries the words.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::CrownMode;
use mesimon_core::command::{
    AskRoad, BoardTouch, Command, LinkView, PeerAnswer, Response, TouchAction,
};
use mesimon_core::Principal;

mod common;
use common::*;

const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                    printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

fn links_of(c: &mut TestClient) -> Vec<LinkView> {
    match c.request(Command::Snapshot) {
        Response::Board { links, .. } => links,
        other => panic!("snapshot: {other:?}"),
    }
}

fn touches_of(c: &mut TestClient) -> Vec<BoardTouch> {
    match c.request(Command::Snapshot) {
        Response::Board { board_touches, .. } => board_touches,
        other => panic!("snapshot: {other:?}"),
    }
}

fn said_on(c: &mut TestClient, ticket: ulid::Ulid) -> Option<String> {
    match c.request(Command::Snapshot) {
        Response::Board { crown_said, .. } => {
            crown_said.into_iter().find(|s| s.ticket == ticket).map(|s| s.words)
        }
        other => panic!("snapshot: {other:?}"),
    }
}

fn ask_board(c: &mut TestClient, from: uuid::Uuid, board: &str, text: &str) -> Response {
    c.send(
        Principal::Agent { session: from },
        Command::AgentAskBoard { board: board.into(), text: text.into() },
    )
}

#[test]
fn two_crowns_reach_each_other_through_one_links_file() {
    let Some(a) = Harness::boot("linka", Some(STUB)) else { return };
    // The second daemon shares the first's HOME, so both read one state
    // root and one `links.toml`, as two boards on one machine do.
    let home = a.dir.join("home");
    let Some(b) = Harness::boot_with_env("linkb", Some(STUB), &[("HOME", home.to_str().unwrap())])
    else {
        return;
    };
    let mut ca = a.client("linka");
    let mut cb = b.client("linkb");
    let got_a = a.stub.as_ref().unwrap().with_file_name("got.txt");
    let got_b = b.stub.as_ref().unwrap().with_file_name("got.txt");
    let landed = |got: &std::path::Path, probe: &str| {
        std::fs::read_to_string(got).unwrap_or_default().contains(probe)
    };

    // Each board: a ticket whose agent wears the crown, idle.
    let ta = ca.create_ticket("Core release");
    let sa = ca.spawn_claude(ta);
    assert!(matches!(ca.request(Command::CrownTicket { id: ta }), Response::Ok));
    let tb = cb.create_ticket("Relay release");
    let sb = cb.spawn_claude(tb);
    assert!(matches!(cb.request(Command::CrownTicket { id: tb }), Response::Ok));
    let kb = cb.board().ticket(tb).unwrap().short_key.clone();
    a.one_turn(&mut ca, sa);
    b.one_turn(&mut cb, sb);
    let b_root = b.repo.canonicalize().unwrap().to_string_lossy().into_owned();
    let a_root = a.repo.canonicalize().unwrap().to_string_lossy().into_owned();
    // Both fixtures name their repository `repo`: the name is the
    // directory's, and the path settles two of one name.
    let name = mesimon_core::boardlink::board_name(&b_root);

    // Unlinked, the crown reaches no board, and is told how a person links one.
    match ask_board(&mut ca, sa, &name, "mesimon-probe-733 x") {
        Response::Err { message } => {
            assert!(message.contains("reaches no other board"), "{message}");
            assert!(message.contains("Reaches boards"), "{message}");
        }
        other => panic!("unlinked ask: {other:?}"),
    }
    assert!(links_of(&mut ca).is_empty() && links_of(&mut cb).is_empty());

    // A person links from A, by B's path as typed: both boards list each
    // other, and the crown's list_board carries it.
    assert!(matches!(
        ca.request(Command::LinkBoard { path: b.repo.to_string_lossy().into_owned() }),
        Response::Ok
    ));
    let la = links_of(&mut ca);
    assert_eq!(la.len(), 1, "{la:?}");
    assert_eq!(la[0].path, b_root);
    assert_eq!(la[0].name, name);
    let lb = links_of(&mut cb);
    assert_eq!(lb.len(), 1, "B reads the same file: {lb:?}");
    assert_eq!(lb[0].path, a_root);
    match ca.request(Command::LinkBoard { path: b.repo.to_string_lossy().into_owned() }) {
        Response::Err { message } => assert!(message.contains("linked already"), "{message}"),
        other => panic!("a second link: {other:?}"),
    }
    match ca.request(Command::LinkBoard { path: a.repo.to_string_lossy().into_owned() }) {
        Response::Err { message } => {
            assert!(message.contains("does not reach itself"), "{message}")
        }
        other => panic!("a self link: {other:?}"),
    }
    match ca.send(Principal::Agent { session: sa }, Command::AgentListBoard) {
        Response::AgentBoard { board } => {
            assert_eq!(board.linked_boards.len(), 1);
            assert_eq!(board.linked_boards[0].path, b_root);
        }
        other => panic!("list_board: {other:?}"),
    }

    // A peer is admitted for one command and nothing else.
    match ca.send(Principal::Peer { board: b_root.clone() }, Command::Snapshot) {
        Response::Err { message } => assert!(message.contains("only asks"), "{message}"),
        other => panic!("a peer's snapshot: {other:?}"),
    }
    // A board not linked with this one is refused by name.
    match ca.send(
        Principal::Peer { board: "/nowhere/else".into() },
        Command::PeerAsk { crown_key: "T-9".into(), crown_title: "x".into(), text: "x".into() },
    ) {
        Response::Err { message } => assert!(message.contains("does not reach else"), "{message}"),
        other => panic!("an unlinked peer: {other:?}"),
    }

    // The ask: A's crown to B's. B is autonomous and its crown idle, so the
    // words go by the queue at once, through the Board ask template.
    let road = match ask_board(&mut ca, sa, &name, "mesimon-probe-733 cut the relay release") {
        Response::AgentAskedBoard { board, asked: PeerAnswer { road, crown, reply } } => {
            assert_eq!(board, name);
            assert_eq!(crown, kb);
            assert!(!reply);
            road
        }
        other => panic!("the ask: {other:?}"),
    };
    assert_eq!(road, AskRoad::Queued);
    wait_until(std::time::Duration::from_secs(10), "the ask to land in B's pane", || {
        landed(&got_b, "mesimon-probe-733 cut the relay release")
    });
    let text = std::fs::read_to_string(&got_b).unwrap();
    assert!(
        text.contains(&format!("The crown of the {name} board (T-1 \"Core release\") asks:")),
        "the template names the asking board and its crown: {text}"
    );
    assert!(text.contains(&format!("ask_board board={name} replies")), "{text}");
    // What each board shows: A's crown card says where the words went, B's
    // crown card says whose they were; the header's touches say which way.
    assert_eq!(
        said_on(&mut ca, ta).as_deref(),
        Some(format!("→ {name}: mesimon-probe-733 cut the relay release").as_str())
    );
    assert!(
        touches_of(&mut ca).iter().any(|t| t.action == TouchAction::Sent && t.path == b_root),
        "{:?}",
        touches_of(&mut ca)
    );
    assert!(
        touches_of(&mut cb).iter().any(|t| t.action == TouchAction::Received && t.path == a_root),
        "{:?}",
        touches_of(&mut cb)
    );
    wait_until(std::time::Duration::from_secs(5), "B's crown card to say whose words", || {
        said_on(&mut cb, tb).is_some_and(|w| w.starts_with(&format!("{name}: mesimon-probe-733")))
    });
    // The paste is acked like any other; the queue clears.
    hook_send(&b.paths.hook_sock(), &sb.to_string(), "UserPromptSubmit", "{}");
    wait_until(std::time::Duration::from_secs(5), "B's queue to clear", || {
        pending_of(&mut cb, Some(tb)).is_empty()
    });

    // The reply: B's crown asks A's back; both sides read it as the reply.
    match ask_board(&mut cb, sb, &name, "mesimon-probe-734 relay is live") {
        Response::AgentAskedBoard { asked: PeerAnswer { reply, road, .. }, .. } => {
            assert!(reply, "an open ask from A makes this a reply");
            assert_eq!(road, AskRoad::Queued);
        }
        other => panic!("the reply: {other:?}"),
    }
    wait_until(std::time::Duration::from_secs(10), "the reply to land in A's pane", || {
        landed(&got_a, "mesimon-probe-734 relay is live")
    });
    let text = std::fs::read_to_string(&got_a).unwrap();
    assert!(
        text.contains(&format!("The crown of the {name} board replied: mesimon-probe-734")),
        "{text}"
    );
    assert!(touches_of(&mut ca).iter().any(|t| t.action == TouchAction::GotReply));
    assert!(touches_of(&mut cb).iter().any(|t| t.action == TouchAction::Replied));
    hook_send(&a.paths.hook_sock(), &sa.to_string(), "UserPromptSubmit", "{}");
    wait_until(std::time::Duration::from_secs(5), "A's queue to clear", || {
        pending_of(&mut ca, Some(ta)).is_empty()
    });

    // The feeds name the roads and never the words. The feed is flushed off
    // the hot path, so the lines are waited for.
    let feed_a = a.paths.state_dir.join("activity.jsonl");
    let has = |needle: &str| {
        std::fs::read_to_string(&feed_a).is_ok_and(|feed| feed.lines().any(|l| l.contains(needle)))
    };
    wait_until(std::time::Duration::from_secs(5), "A's feed lines for the roads", || {
        has("\"ask_board\"") && has("\"board_ask_received\"") && has("\"board_ask_sent\"")
    });
    assert!(
        std::fs::read_to_string(&feed_a)
            .unwrap()
            .lines()
            .any(|l| l.contains("\"ask_board\"") && l.contains("\"queued\"")),
        "the ask's line carries its road"
    );
    for h in [&a, &b] {
        let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
        assert!(!feed.contains("mesimon-probe-73"), "the feed never carries the words");
    }

    // Supervised, B holds the words for a person's send.
    assert!(matches!(
        cb.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    match ask_board(&mut ca, sa, &name, "mesimon-probe-735 held") {
        Response::AgentAskedBoard { asked, .. } => assert_eq!(asked.road, AskRoad::HeldForPerson),
        other => panic!("the held ask: {other:?}"),
    }
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert!(!landed(&got_b, "mesimon-probe-735"), "held, not sent");
    let p = pending_of(&mut cb, Some(tb));
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(matches!(cb.request(Command::SendQueuedAsk { ticket: tb }), Response::Ok));
    wait_until(std::time::Duration::from_secs(10), "the person's send to land", || {
        landed(&got_b, "mesimon-probe-735 held")
    });
    hook_send(&b.paths.hook_sock(), &sb.to_string(), "UserPromptSubmit", "{}");

    // No crown there: refused, naming how a person crowns one.
    assert!(matches!(cb.request(Command::Uncrown), Response::Ok));
    match ask_board(&mut ca, sa, &name, "mesimon-probe-736 nobody") {
        Response::Err { message } => assert!(message.contains("has no crown"), "{message}"),
        other => panic!("no crown: {other:?}"),
    }
    assert!(touches_of(&mut ca).iter().any(|t| t.action == TouchAction::Refused));

    // Not open: B's daemon gone, the ask says so and nothing waits.
    let _ = cb.request(Command::Shutdown);
    wait_until(std::time::Duration::from_secs(10), "B's daemon to leave", || {
        !b.paths.orch_sock().exists()
    });
    match ask_board(&mut ca, sa, &name, "mesimon-probe-737 closed") {
        Response::Err { message } => assert!(message.contains("is not open"), "{message}"),
        other => panic!("closed board: {other:?}"),
    }

    // Unlinking from A takes the pair off for both.
    assert!(matches!(ca.request(Command::UnlinkBoard { path: b_root.clone() }), Response::Ok));
    assert!(links_of(&mut ca).is_empty());
    match ca.request(Command::UnlinkBoard { path: b_root }) {
        Response::Err { message } => assert!(message.contains("is not linked"), "{message}"),
        other => panic!("a second unlink: {other:?}"),
    }
}
