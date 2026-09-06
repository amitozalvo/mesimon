//! Tags (T-83) end to end against a real daemon: `SetTag` replaces within a
//! group and clears with `None`, the name is sanitized at the boundary, a bad
//! group is refused, the tags survive the disk round-trip, and `[[tags]]`
//! lands before `[archived]` in the file that gets written — and a column
//! sorted `by tag` (T-283) comes out in the order the picker's row draws,
//! which carrying a tag along that row is what changes.
//!
//! No tmux and no agent — tags are pure board state, so this one runs
//! everywhere.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::board::{SortBy, MAX_TAGS_PER_GROUP};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

#[test]
fn tags_round_trip_through_the_daemon_and_the_disk() {
    let fixture = common::TestFixture::new("tags");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();

    // This test builds the vocabulary from nothing; the starter tags a fresh
    // board is offered would sit in group 1 ahead of every name it registers.
    fixture.set_env("MESIMON_NO_TAG_SEED", "1");
    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "tags".into() }),
        Response::Hello { .. }
    ));

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "tag me".into(),
        workspace: None,
    });
    let board = board_of(c.request(Command::Snapshot));
    let id = board.tickets[0].id;
    let key = board.tickets[0].short_key.clone();
    let ticket_toml = repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");

    // ---- one per group ----------------------------------------------------
    let set = |c: &mut TestClient, group: u8, name: Option<&str>| {
        c.request(Command::SetTag { id, group, name: name.map(|s| s.to_string()) })
    };
    assert!(matches!(set(&mut c, 1, Some("BUG")), Response::Ok));
    assert!(matches!(set(&mut c, 2, Some("STAGING")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tags.len(), 2);
    assert_eq!(t.tag_in(1).unwrap().name, "BUG");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING");

    // Setting the same group again REPLACES — a group is an axis.
    assert!(matches!(set(&mut c, 1, Some("REGR")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tags.len(), 2, "replaced, not appended");
    assert_eq!(t.tag_in(1).unwrap().name, "REGR");

    // The registry is board-level and PERSISTED: nothing wears BUG any more,
    // and BUG is still in the vocabulary. Using a name once is what puts it
    // there — that is all "create on the fly" means — and only an explicit
    // retire takes it out. Registry order is creation order.
    assert_eq!(board.group_tags(1), vec!["BUG", "REGR"]);

    // It is offered on a ticket that never wore it, which is the point of a
    // registry rather than a per-ticket accident.
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "other".into(),
        workspace: None,
    });
    let board = board_of(c.request(Command::Snapshot));
    let other = board.tickets.iter().find(|t| t.title == "other").unwrap().id;
    assert_eq!(board.group_tags(1), vec!["BUG", "REGR"]);
    assert!(matches!(
        c.request(Command::SetTag { id: other, group: 1, name: Some("BUG".into()) }),
        Response::Ok
    ));

    // ---- the boundary sanitizes and refuses --------------------------------
    err_containing(set(&mut c, 0, Some("x")), "1-10");
    err_containing(set(&mut c, 11, Some("x")), "1-10");
    err_containing(set(&mut c, 3, Some("   ")), "empty");
    // Control chars and the drawn-structure range never reach a card row.
    assert!(matches!(set(&mut c, 3, Some("a\nb\u{2500}c")), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.ticket(id).unwrap().tag_in(3).unwrap().name, "abc");
    // And the length cap holds.
    assert!(matches!(set(&mut c, 4, Some(&"x".repeat(80))), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.ticket(id).unwrap().tag_in(4).unwrap().name.len() <= 24);

    // ---- the file on disk --------------------------------------------------
    let raw = std::fs::read_to_string(&ticket_toml).expect("ticket.toml");
    assert!(raw.contains("[[tags]]"), "tags reached the disk:\n{raw}");
    assert!(raw.contains("REGR"));
    // A scalar after a table is a TOML serialize error, so this ordering is
    // not cosmetic — it is the thing that keeps `save_ticket` from failing.
    let tags_at = raw.find("[[tags]]").unwrap();
    for scalar in ["id =", "short_key =", "title =", "column =", "created_at ="] {
        let at = raw.find(scalar).unwrap_or_else(|| panic!("{scalar} missing from\n{raw}"));
        assert!(at < tags_at, "{scalar} must serialize before [[tags]]");
    }

    // ---- creating, colouring, renaming, and the cap -------------------------
    // Registering is its own gesture: it puts a name in the vocabulary and
    // touches no ticket.
    assert!(matches!(
        c.request(Command::RegisterTag { group: 5, name: "SOLO".into() }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(5), vec!["SOLO"]);
    assert!(board.tickets.iter().all(|t| t.tag_in(5).is_none()), "registering tags nothing");
    err_containing(c.request(Command::RegisterTag { group: 5, name: "SOLO".into() }), "already");

    // Colour is a registry property, so it is set once and every card follows.
    assert_eq!(board.tag_def(5, "SOLO").unwrap().color, None, "unchosen by default");
    assert!(matches!(
        c.request(Command::SetTagColor { group: 5, name: "SOLO".into(), color: 3 }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tag_def(5, "SOLO").unwrap().tint(), 3);

    // Renaming carries the wearers.
    assert!(matches!(
        c.request(Command::SetTag { id, group: 5, name: Some("SOLO".into()) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::RenameTag { group: 5, from: "SOLO".into(), to: "DUET".into() }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(5), vec!["DUET"]);
    assert_eq!(board.ticket(id).unwrap().tag_in(5).unwrap().name, "DUET", "the wearer followed");
    assert_eq!(board.tag_def(5, "DUET").unwrap().tint(), 3, "and kept its colour");

    // A group is a capped set, not a list — and the cap is read from the
    // constant, not counted out here: the number moved once already.
    for i in 0..(MAX_TAGS_PER_GROUP - 1) {
        assert!(matches!(
            c.request(Command::RegisterTag { group: 5, name: format!("f{i}") }),
            Response::Ok
        ));
    }
    err_containing(c.request(Command::RegisterTag { group: 5, name: "overflow".into() }), "full");

    // ---- the registry on disk ----------------------------------------------
    let cols = std::fs::read_to_string(repo.join(".mesimon/board/columns.toml")).unwrap();
    // The stamp the writer says it writes — read from the constant, not a
    // literal, so a later bump moves this with the code instead of breaking
    // a test about tags (T-217 bumped it to 3 and this said 2).
    assert!(
        cols.contains(&format!("schema_version = {}", mesimon_daemon::store::COLUMNS_SCHEMA)),
        "the registry file is not stamped with the writer's own schema:\n{cols}"
    );
    assert!(cols.contains("[[tags]]"), "registry reached the disk:\n{cols}");
    assert!(cols.contains("REGR") && cols.contains("BUG"));
    let tags_at = cols.find("[[tags]]").unwrap();
    for scalar in ["schema_version =", "next_key ="] {
        assert!(cols.find(scalar).unwrap() < tags_at, "{scalar} must precede [[tags]]");
    }

    // ---- clearing ----------------------------------------------------------
    assert!(matches!(set(&mut c, 1, None), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert!(t.tag_in(1).is_none(), "cleared");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING", "other axes untouched");

    // ---- survives a daemon restart (the real migration test) ---------------
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    // This test builds the vocabulary from nothing; the starter tags a fresh
    // board is offered would sit in group 1 ahead of every name it registers.
    fixture.set_env("MESIMON_NO_TAG_SEED", "1");
    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never reappeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "tags".into() }),
        Response::Hello { .. }
    ));
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(id).unwrap();
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING", "tags reloaded from disk");
    assert_eq!(t.tag_in(3).unwrap().name, "abc");
    assert!(t.tag_in(1).is_none());
    // The REGISTRY reloaded too — the vocabulary is not rebuilt from tickets.
    assert_eq!(board.group_tags(1), vec!["BUG", "REGR"], "registry survived the restart");
    assert_eq!(board.group_tags(2), vec!["STAGING"]);

    // ---- retiring takes the pips with it -----------------------------------
    // `other` wears BUG; `id` does not. Retiring must clear the one and leave
    // the other alone, and drop the name from the cycle for good.
    err_containing(c.request(Command::ForgetTag { group: 1, name: "NOPE".into() }), "no tag");
    assert!(matches!(c.request(Command::ForgetTag { group: 1, name: "BUG".into() }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["REGR"], "retired from the vocabulary");
    assert!(
        board.ticket(other).unwrap().tag_in(1).is_none(),
        "a retired tag leaves no orphan pip on the card"
    );
    assert_eq!(board.ticket(id).unwrap().tag_in(2).unwrap().name, "STAGING", "other axes intact");
    // And it stays retired across a reload.
    let raw = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(!raw.contains("\"BUG\""), "the wearer's file was rewritten:\n{raw}");

    // ---- moving a tag: along its axis, and onto another -------------------
    // Along the row it is pure order — what the picker draws and what a
    // repeated digit walks — so no ticket file is touched.
    assert!(matches!(
        c.request(Command::RegisterTag { group: 1, name: "FTR".into() }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["REGR", "FTR"]);
    assert!(matches!(
        c.request(Command::MoveTag { group: 1, name: "FTR".into(), to_group: 1, to_index: 0 }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["FTR", "REGR"], "the axis reordered");

    // Onto another axis it is refused, never resolved, when a wearer already
    // has something there: one tag per group is what lets a digit address an
    // axis, and the alternative is dropping a tag off a card silently.
    assert!(matches!(
        c.request(Command::SetTag { id, group: 1, name: Some("REGR".into()) }),
        Response::Ok
    ));
    err_containing(
        c.request(Command::MoveTag { group: 1, name: "REGR".into(), to_group: 2, to_index: 0 }),
        "already wear a tag on axis 2",
    );
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["FTR", "REGR"], "a refusal changes nothing");
    assert_eq!(board.ticket(id).unwrap().tag_in(1).unwrap().name, "REGR");

    // A free axis takes it, and the wearers travel with it.
    assert!(matches!(
        c.request(Command::MoveTag { group: 1, name: "REGR".into(), to_group: 7, to_index: 0 }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.group_tags(1), vec!["FTR"]);
    assert_eq!(board.group_tags(7), vec!["REGR"]);
    let t = board.ticket(id).unwrap();
    assert!(t.tag_in(1).is_none(), "no pip left on the axis it came from");
    assert_eq!(t.tag_in(7).unwrap().name, "REGR", "the wearer followed");
    assert_eq!(t.tag_in(2).unwrap().name, "STAGING", "and its other axes are untouched");
    // Both files were rewritten: the registry, and the ticket that wore it.
    let cols = std::fs::read_to_string(repo.join(".mesimon/board/columns.toml")).unwrap();
    assert!(cols.contains("group = 7"), "the registry recorded the new axis:\n{cols}");
    let raw = std::fs::read_to_string(&ticket_toml).unwrap();
    assert!(raw.contains("group = 7"), "the wearer's file followed:\n{raw}");

    // ---- a column sorts by the picker's row (T-283) ------------------------
    // Axis 6 is free, and the two names are registered out of alphabetical
    // order on purpose: what the sort follows is the ROW, and `MoveTag` is
    // the only thing that arranges it.
    assert!(matches!(
        c.request(Command::AddColumn { name: "SORT".into(), after: None }),
        Response::Ok
    ));
    for name in ["LATE", "EARLY"] {
        assert!(matches!(
            c.request(Command::RegisterTag { group: 6, name: name.into() }),
            Response::Ok
        ));
    }
    let mut made: Vec<ulid::Ulid> = Vec::new();
    for title in ["one", "two", "three"] {
        match c.request(Command::CreateTicket {
            column: "SORT".into(),
            title: title.into(),
            workspace: None,
        }) {
            Response::Created { id, .. } => made.push(id),
            other => panic!("create: {other:?}"),
        }
    }
    let (one, two, three) = (made[0], made[1], made[2]);
    assert!(matches!(
        c.request(Command::SetTag { id: one, group: 6, name: Some("EARLY".into()) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SetTag { id: two, group: 6, name: Some("LATE".into()) }),
        Response::Ok
    ));
    let sorted = |c: &mut TestClient| -> Vec<ulid::Ulid> {
        board_of(c.request(Command::Snapshot)).column_tickets("SORT").iter().map(|t| t.id).collect()
    };
    assert_eq!(sorted(&mut c), vec![one, two, three], "creation order to start");
    assert!(matches!(
        c.request(Command::SortColumn { column: "SORT".into(), by: SortBy::Tag }),
        Response::Ok
    ));
    assert_eq!(
        sorted(&mut c),
        vec![two, one, three],
        "LATE first because it is the row's first cell, not because L sorts before E — \
         and the untagged card sinks"
    );

    // Carry EARLY to the head of the row and its cards rise with it. This is
    // the whole feature: the picker is where the order is chosen.
    assert!(matches!(
        c.request(Command::MoveTag { group: 6, name: "EARLY".into(), to_group: 6, to_index: 0 }),
        Response::Ok
    ));
    let was = board_of(c.request(Command::Snapshot)).ticket(one).unwrap().order.clone();
    assert!(matches!(
        c.request(Command::SortColumn { column: "SORT".into(), by: SortBy::Tag }),
        Response::Ok
    ));
    assert_eq!(sorted(&mut c), vec![one, two, three], "the cards followed the row");
    // And the new order reached the disk, not just the snapshot.
    let board = board_of(c.request(Command::Snapshot));
    let t = board.ticket(one).unwrap();
    assert_ne!(t.order, was, "the fractional index was rewritten");
    let raw = std::fs::read_to_string(
        repo.join(".mesimon/board/tickets").join(&t.short_key).join("ticket.toml"),
    )
    .unwrap();
    assert!(raw.contains(&format!("order = \"{}\"", t.order)), "the file followed:\n{raw}");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
