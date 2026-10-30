//! A parked conversation releases a plain (non-Git) checkout without archive.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use mesimon_core::board::SessionState;
use mesimon_core::command::{Command, Deliver, Response};
use mesimon_core::Principal;
use std::time::Duration;

fn holders(c: &mut TestClient, crown: uuid::Uuid) -> Vec<String> {
    match c.send(Principal::Agent { session: crown }, Command::AgentListBoard) {
        Response::AgentBoard { board } => board.checkout_held_by,
        other => panic!("list_board: {other:?}"),
    }
}

fn park(c: &mut TestClient, crown: uuid::Uuid, key: &str) {
    let seen = c.agent_read(crown, key).unwrap().seen;
    let reply = c.send(
        Principal::Agent { session: crown },
        Command::AgentSleepTicket { key: key.into(), seen },
    );
    assert!(matches!(reply, Response::AgentTicketUpdated { .. }), "{reply:?}");
}

#[test]
fn a_parked_done_worker_releases_a_non_git_checkout_without_archive() {
    const STUB: &str =
        "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot("crown-checkout-release", Some(STUB)) else { return };
    assert!(!h.repo.join(".git").exists(), "the reported case has no Git worktrees");
    let mut c = h.client("crown-checkout-release");
    let crown_ticket = c.create_ticket("coordinate");
    let first = c.create_ticket("first worker");
    let next = c.create_ticket("next worker");
    let first_key = c.key_of(first);
    let next_key = c.key_of(next);
    let crown = c.spawn_claude(crown_ticket);
    assert!(matches!(c.request(Command::CrownTicket { id: crown_ticket }), Response::Ok));
    h.one_turn(&mut c, crown);
    let reply = c.crown_start(crown, &first_key, "shared_checkout");
    assert!(matches!(reply, Response::AgentStarted { woken: false, .. }), "{reply:?}");
    let first_session = c.board().live_agent(first).unwrap().id;
    h.one_turn(&mut c, first_session);
    assert_eq!(holders(&mut c, crown), vec![first_key.clone()], "idle still owns the checkout");
    park(&mut c, crown, &first_key);
    c.await_state(first_session, "parked", |s| *s == SessionState::Sleeping);
    match c.crown_start(crown, &first_key, "shared_checkout") {
        Response::Err { message } => assert!(message.contains("still stopping"), "{message}"),
        other => panic!("same worker woke before its reaper finished: {other:?}"),
    }
    let seen = c.agent_read(crown, &first_key).unwrap().seen;
    let reply = c.send(
        Principal::Agent { session: crown },
        Command::AgentMoveTicket {
            to_column: "DONE".into(),
            key: Some(first_key.clone()),
            seen,
            before: None,
            idempotency_key: None,
        },
    );
    assert!(matches!(reply, Response::AgentMoved { .. }), "{reply:?}");
    wait_until(Duration::from_secs(10), "safe parking to release the checkout", || {
        holders(&mut c, crown).is_empty()
    });
    let board = c.board();
    assert_eq!(board.ticket(first).unwrap().column, "DONE");
    assert!(!board.ticket(first).unwrap().is_archived());
    assert_eq!(board.live_agent(first).unwrap().id, first_session);
    let reply = c.crown_start(crown, &next_key, "shared_checkout");
    assert!(matches!(reply, Response::AgentStarted { woken: false, .. }), "{reply:?}");
    let next_session = c.board().live_agent(next).unwrap().id;
    h.one_turn(&mut c, next_session);
    assert_eq!(holders(&mut c, crown), vec![next_key.clone()]);
    match c.crown_start(crown, &first_key, "shared_checkout") {
        Response::Err { message } => assert!(message.contains(&next_key), "{message}"),
        other => panic!("old worker resumed over the new owner: {other:?}"),
    }
    match c.crown_ask(crown, &first_key, "continue", Deliver::Idle, None) {
        Response::AgentAsked { held_for_person, held_because, .. } => {
            assert!(held_for_person);
            assert!(held_because.unwrap().contains(&next_key));
        }
        other => panic!("ask must not resume over the new owner: {other:?}"),
    }
    assert!(matches!(c.request(Command::DropQueuedAsk { ticket: first }), Response::Ok));
    assert_eq!(c.board().live_agent(first).unwrap().state, SessionState::Sleeping);
    park(&mut c, crown, &next_key);
    wait_until(Duration::from_secs(10), "next worker's safe parking", || {
        holders(&mut c, crown).is_empty()
    });
    let reply = c.crown_start(crown, &first_key, "shared_checkout");
    assert!(matches!(reply, Response::AgentStarted { woken: true, .. }), "{reply:?}");
    assert_eq!(c.board().live_agent(first).unwrap().id, first_session, "conversation seat kept");
}

#[test]
fn a_failed_reap_keeps_the_checkout_until_pane_absence_is_confirmed() {
    use std::os::unix::fs::PermissionsExt;
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!("msmn-reap-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let failed = scratch.0.join("fail-kill");
    let unknown = scratch.0.join("fail-list");
    let attempts = scratch.0.join("attempts");
    let wrapper = scratch.0.join("tmux.sh");
    let tmux = mesimon_backend_tmux::tmux_bin();
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n\
         if [ \"$arg\" = kill-session ] && [ -e '{}' ]; then echo attempt >> '{}'; exit 1; fi\n\
         if [ \"$arg\" = list-panes ] && [ -e '{}' ]; then exit 1; fi\n\
         done\nexec '{}' \"$@\"\n",
            failed.display(),
            attempts.display(),
            unknown.display(),
            tmux.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    // The worker really survives SIGTERM; a Sleeping badge is not proof
    // that it stopped. Its private tmux session must still be removed.
    const STUB: &str = "#!/bin/sh\ntrap '' TERM\nexec sleep 300\n";
    let Some(h) = Harness::boot_with_env(
        "crown-reap-failure",
        Some(STUB),
        &[("MESIMON_TMUX_BIN", wrapper.to_str().unwrap())],
    ) else {
        return;
    };
    let mut c = h.client("crown-reap-failure");
    let crown_ticket = c.create_ticket("coordinate");
    let worker = c.create_ticket("worker");
    let next = c.create_ticket("next worker");
    let key = c.key_of(worker);
    let next_key = c.key_of(next);
    let mut crown = c.spawn_claude(crown_ticket);
    assert!(matches!(c.request(Command::CrownTicket { id: crown_ticket }), Response::Ok));
    h.one_turn(&mut c, crown);
    assert!(matches!(c.crown_start(crown, &key, "shared_checkout"), Response::AgentStarted { .. }));
    let session = c.board().live_agent(worker).unwrap().id;
    h.one_turn(&mut c, session);
    let sid = c.board().live_agent(worker).unwrap().sid16();
    std::fs::write(&failed, "fail").unwrap();
    park(&mut c, crown, &key);
    let count = || std::fs::read_to_string(&attempts).unwrap_or_default().lines().count();
    wait_until(Duration::from_secs(12), "the first failed removal", || count() > 0);
    assert!(holders(&mut c, crown).contains(&key));
    let real = |args: &[&str]| {
        std::process::Command::new(&tmux)
            .arg("-S")
            .arg(h.paths.tmux_sock())
            .args(args)
            .output()
            .unwrap()
    };
    let pane = real(&["display-message", "-p", "-t", &sid, "#{pane_dead}"]);
    assert!(pane.status.success());
    assert_eq!(String::from_utf8_lossy(&pane.stdout).trim(), "0", "the old process is alive");
    assert!(matches!(c.crown_start(crown, &next_key, "shared_checkout"), Response::Err { .. }));

    std::fs::write(&unknown, "unavailable").unwrap();
    let before = count();
    wait_until(Duration::from_secs(12), "failed removal and unavailable inspection", || {
        count() > before
    });
    assert!(holders(&mut c, crown).contains(&key));
    // A daemon restart loses the in-memory reaper, so an unavailable
    // startup listing must reconstruct the parked worker's reservation.
    drop(c);
    h.restart();
    c = h.client("crown-reap-restarted");
    let successor = c.create_ticket("coordinate after restart");
    crown = c.spawn_claude(successor);
    assert!(matches!(c.request(Command::CrownTicket { id: successor }), Response::Ok));
    assert!(holders(&mut c, crown).contains(&key));
    assert!(matches!(c.crown_start(crown, &next_key, "shared_checkout"), Response::Err { .. }));

    // An external removal is still not enough while the daemon cannot
    // inspect it. Only the next successful listing releases the hold.
    assert!(real(&["kill-session", "-t", &sid]).status.success());
    let before = count();
    wait_until(Duration::from_secs(12), "unknown absence remains held", || count() > before);
    assert!(holders(&mut c, crown).contains(&key));
    std::fs::remove_file(&unknown).unwrap();
    wait_until(Duration::from_secs(12), "confirmed absence releases a failed removal", || {
        !holders(&mut c, crown).contains(&key)
    });
    std::fs::remove_file(&failed).unwrap();
    assert!(matches!(
        c.crown_start(crown, &next_key, "shared_checkout"),
        Response::AgentStarted { .. }
    ));
}
