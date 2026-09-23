//! `x x` on a ticket page — sleep, then wake before the process has finished
//! going down — is one gesture, not a fight (T-381). Real tmux, in-process
//! daemon, a stub that ignores SIGTERM the way a claude running its exit
//! hooks does: alive, pid file and all, for a while after being told to die.
//!
//! Two things went wrong on the work computer. The wake read the daemon's
//! OWN dying process as "running elsewhere (pid N)" and asked for a confirm
//! — and the confirm's kill-session then took that process down under a
//! fresh pane, whose record was next marked crashed by the killed process's
//! own `SessionEnd{other}` straggler. Every Enter on the corpse repeated the
//! pair: a refusal, a confirmed kill of the live claude, the next straggler.
//!
//! The test drives exactly that: a live pid file naming the record, a sleep,
//! an immediate wake, the new pane's `SessionStart`, and then the stragglers
//! — and the record stays live. A death frame from the NEW pane, and a real
//! death, still land, so the rule is not a blindfold.
//!
//! What tells a straggler apart is the pane id it carries (T-245): a wake
//! reuses the session name and the session uuid, never the pane. The test
//! process sits outside every pane, so the stragglers spell the old pane
//! with `--pane`, the way `TMUX_PANE` would inside it.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{ExitReason, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

/// Ignores SIGTERM (the children inherit the disposition), so the sleep's
/// signal leaves it standing until the wake's kill-session SIGHUPs it.
const STUB: &str = "#!/bin/sh\ntrap '' TERM\nwhile true; do sleep 1; done\n";

fn pane_pid(sock: &std::path::Path, sid16: &str) -> i32 {
    let out =
        tmux(sock).args(["display-message", "-p", "-t", sid16, "#{pane_pid}"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().expect("pane_pid")
}

/// `<server pid>:<pane id>` — the key the daemon records and a hook frame
/// carries. The server pid is in it because this test's wake kills the
/// server's LAST session: the server exits, the wake starts a fresh one, and
/// the new pane is `%0` again, same as the old.
fn pane_key(sock: &std::path::Path, sid16: &str) -> String {
    let out = tmux(sock)
        .args(["display-message", "-p", "-t", sid16, "#{pid}:#{pane_id}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn alive(pid: i32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn wake_over_a_dying_pane_is_not_elsewhere_and_its_stragglers_do_not_land() {
    let Some(h) = Harness::boot("wakestrag", Some(STUB)) else { return };
    let mut c = h.client("wakestrag");
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let claude_home = h.dir.join("claude-home");

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "restart me".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.iter().find(|t| t.title == "restart me").unwrap().id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let sid16 = c.board().sessions.iter().find(|s| s.id == sid).unwrap().sid16();

    // A conversation to come back to, in Claude's own store: the wake is a
    // real `--resume`, the road the user's `x` takes.
    let projects = claude_home.join("projects").join("msmn");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(
        projects.join(format!("{sid}.jsonl")),
        "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n",
    )
    .unwrap();

    // Idle through the hooks — the stub emits none of its own.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));

    // The pid file a live claude keeps for its conversation — the file the
    // double-resume guard reads. The pane's process IS the agent's.
    let old_pid = pane_pid(&tmux_sock, &sid16);
    let old_pane = pane_key(&tmux_sock, &sid16);
    let sessions = claude_home.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join(format!("{old_pid}.json")),
        format!(r#"{{"sessionId":"{sid}","pid":{old_pid},"cwd":"{}"}}"#, h.repo.display()),
    )
    .unwrap();

    // ---- x: sleep. The process ignores the SIGTERM and stays up. ---------
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "sleeping", |s| *s == SessionState::Sleeping);
    assert!(
        alive(old_pid),
        "the stub must outlive the sleep's SIGTERM for this test to mean anything"
    );

    // ---- x: wake, at once. Our own dying pane is not "elsewhere". --------
    match c.request(Command::WakeSession { id: sid }) {
        Response::Spawned { id, fresh } => {
            assert_eq!(id, sid, "the wake re-enters the record");
            assert!(!fresh, "a transcript exists, so the wake is a resume");
        }
        other => {
            panic!("a wake over the daemon's own dying pane must not ask for a confirm: {other:?}")
        }
    }
    let new_pid = pane_pid(&tmux_sock, &sid16);
    let new_pane = pane_key(&tmux_sock, &sid16);
    assert_ne!(new_pid, old_pid, "a fresh pane under the same name");
    assert_ne!(new_pane, old_pane, "a fresh pane key under the same name");
    assert_eq!(
        c.board().sessions.iter().find(|s| s.id == sid).unwrap().pane_key.as_deref(),
        Some(new_pane.as_str()),
        "the record remembers the pane the wake spawned"
    );
    wait_until(Duration::from_secs(5), "the old process to be gone", || !alive(old_pid));

    // The new pane says it is reading; the record leaves `Spawning`.
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("resume"), "{}");
    c.await_state(sid, "idle after resume", |s| matches!(s, SessionState::Idle { .. }));

    // ---- the stragglers: the killed process's exit hook, and the ---------
    // pane-died its pane would have sent, both naming this record and
    // both from the OLD pane.
    let old = Some(old_pane.as_str());
    hook_send_from_pane(&hook_sock, &sid.to_string(), "SessionEnd", Some("other"), old, "{}");
    hook_send_from_pane(&hook_sock, &sid16, "PaneDied", Some("143"), old, "");
    std::thread::sleep(Duration::from_millis(800));
    let state = c.board().sessions.iter().find(|s| s.id == sid).unwrap().state.clone();
    assert!(
        matches!(state, SessionState::Idle { .. }),
        "a death frame from another pane is the previous tenant's: {state:?}"
    );
    assert!(alive(new_pid), "the woken process was never touched");
    // ...and Enter on it is a focus, never another resume-with-confirm.
    err_containing(
        c.request(Command::ResumeSession { id: sid, confirm: false }),
        "session is live",
    );

    // ---- a death from the NEW pane lands, at once ----------------------
    // The same frame with the live pane's id is the record's own death: no
    // window, no tmux ask. (The stub is still up; the daemon's harvest
    // kill-session then finishes it.)
    let new = Some(new_pane.as_str());
    hook_send_from_pane(&hook_sock, &sid16, "PaneDied", Some("1"), new, "");
    let dead = c.await_state(sid, "exited", |s| !s.is_live());
    assert_eq!(
        dead,
        SessionState::Exited { reason: ExitReason::Crashed },
        "a death frame from the record's own pane is a death"
    );
    wait_until(Duration::from_secs(5), "the harvest to take the pane", || !alive(new_pid));

    let _ = c.request(Command::Shutdown);
}
