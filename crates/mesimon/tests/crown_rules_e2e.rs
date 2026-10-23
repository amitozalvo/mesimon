//! The crown's rules are said once per crowning (T-930), not on every
//! receipt: the first call a crowned session makes after it was crowned,
//! after its conversation compacted or cleared, and after the daemon
//! started comes back with them beside the call's own answer, whichever
//! tool it is, and every later call comes back without them. The crown's
//! own `get_ticket` carries them as ever, so that read is the telling and
//! adds nothing. A person may crown an agent already at work, uncrown it
//! and crown it again; each crowning starts fresh. A resume keeps the
//! conversation that read them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use mesimon_core::command::{Command, Response};
use mesimon_core::mcp::crown_view;
use mesimon_core::Principal;

const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";

/// One `list_board` from `session`: the rules beside it, or `None`.
fn listed(c: &mut TestClient, session: uuid::Uuid) -> Option<String> {
    match c.send_raw(Principal::Agent { session }, Command::AgentListBoard) {
        Response::CrownRules { rules, inner } => {
            assert!(matches!(*inner, Response::AgentBoard { .. }), "{inner:?}");
            Some(rules)
        }
        Response::AgentBoard { .. } => None,
        other => panic!("list_board: {other:?}"),
    }
}

#[test]
fn the_crown_is_told_its_rules_once_per_crowning() {
    let Some(h) = Harness::boot("crown-rules", Some(STUB)) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown-rules");
    let a = c.create_ticket("coordinate");
    let b = c.create_ticket("other work");
    // An agent already at work before anyone crowns it.
    let sa = c.spawn_claude(a);
    let sb = c.spawn_claude(b);
    assert_eq!(listed(&mut c, sa), None, "an uncrowned session is told nothing");

    // ---- crowned mid-run: the first call carries them, the next does not --
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(listed(&mut c, sa), Some(crown_view()), "the first crown call, whichever tool");
    assert_eq!(listed(&mut c, sa), None, "said once");
    assert_eq!(listed(&mut c, sb), None, "a worker is never told the crown's rules");
    // A refusal carries them as well when it is the first call.
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    match c.send_raw(
        Principal::Agent { session: sa },
        Command::AgentReadTicket { key: "T-999".into() },
    ) {
        Response::CrownRules { rules, inner } => {
            assert_eq!(rules, crown_view());
            assert!(matches!(*inner, Response::Err { .. }), "{inner:?}");
        }
        other => panic!("a refused first call: {other:?}"),
    }
    assert_eq!(listed(&mut c, sa), None);

    // ---- the crown's own get_ticket is the telling, and adds nothing ----
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    match c.send_raw(Principal::Agent { session: sa }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => assert_eq!(ticket.crown, Some(crown_view())),
        other => panic!("the crown's own read: {other:?}"),
    }
    assert_eq!(listed(&mut c, sa), None, "its own read told it");

    // ---- another crowning, another agent: fresh ----------------------------
    assert!(matches!(c.request(Command::CrownTicket { id: b }), Response::Ok));
    assert_eq!(listed(&mut c, sb), Some(crown_view()), "the new crown is told");
    assert_eq!(listed(&mut c, sa), None, "the old one is no crown");
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(listed(&mut c, sa), Some(crown_view()), "crowned again: told again");

    // ---- a compaction, a clear: told again; a resume keeps them ------------
    hook_send(&hook_sock, &sa.to_string(), "PreCompact", r#"{"trigger":"manual"}"#);
    wait_until(std::time::Duration::from_secs(5), "the compaction window", || {
        c.sessions_of(a).first().is_some_and(|r| r.compacting)
    });
    assert_eq!(listed(&mut c, sa), Some(crown_view()), "a compaction forgets them");
    hook_send(&hook_sock, &sa.to_string(), "PostCompact", r#"{"trigger":"manual"}"#);
    assert_eq!(listed(&mut c, sa), None);
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"resume"}"#);
    assert_eq!(listed(&mut c, sa), None, "a resume keeps the conversation that read them");
    hook_send(&hook_sock, &sa.to_string(), "SessionStart", r#"{"source":"clear"}"#);
    // The frame rides the hook socket, the call the board's: the first call
    // after the frame lands is the one that carries them.
    wait_until(std::time::Duration::from_secs(5), "a clear to forget them", || {
        listed(&mut c, sa).is_some()
    });
    assert_eq!(listed(&mut c, sa), None);

    // ---- a restart: every crown is told again ------------------------------
    drop(c);
    h.restart();
    let mut c = h.client("crown-rules-after");
    assert_eq!(listed(&mut c, sa), Some(crown_view()), "a new daemon tells the crown again");
    assert_eq!(listed(&mut c, sa), None);
}
