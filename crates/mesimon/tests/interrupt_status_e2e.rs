//! The RECORDLESS Esc (live 2026-09-04): pressed before the first assistant
//! output, Claude Code hands the prompt back to the box and writes nothing to
//! the transcript — no hook, no record, and the pane keeps painting, so the
//! only catch was the 60 s pane-quiet probe. Claude Code's own
//! `~/.claude/sessions/<pid>.json` flips `status` to `idle` at the keypress;
//! a Running session of ours must demote off that file, and only off a stamp
//! newer than the turn it is Running on. Real tmux, in-process daemon, a stub
//! agent that paints forever, a fabricated `~/.claude` (`MESIMON_CLAUDE_HOME`).
//!
//! And the same file flips `idle` at the end of EVERY turn, ahead of the Stop
//! hook, so a late Stop read as an Esc (simbly T-11, 2026-09-05): the second
//! test writes the closed turn into the transcript first and expects
//! `Idle{EndTurn}`, still Medium, still only off a stamp newer than the spell.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::{Duration, Instant};

use mesimon_core::board::{Confidence, SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};

#[test]
fn a_status_file_gone_idle_demotes_running_while_the_pane_still_paints() {
    // Post-interrupt Claude Code: the pane paints forever. Quiet never trips,
    // and the threshold is parked past the horizon to prove it is not this.
    const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n";
    let Some(h) =
        Harness::boot_with_env("intrstatus", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("intrstatus");

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "intrstatus".into() });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let transcript = h.dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let start_body = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"{}","source":"startup"}}"#,
        transcript.display(),
        h.dir.join("repo").display()
    );
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", &start_body);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    let state = |c: &mut TestClient| {
        let rec = c.board().sessions.iter().find(|r| r.id == sid).expect("record").clone();
        (rec.state, rec.confidence)
    };
    wait_until(Duration::from_secs(5), "UserPromptSubmit promoted to Running", || {
        state(&mut c).0 == SessionState::Running
    });
    let now_ms = || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
            as u64
    };

    // The file: sessionId is the record's (mesimon mints the conversation
    // id), the pid is a live one (this process), the status is idle — but
    // stamped a minute BEFORE the prompt: the previous turn's write. It must
    // not demote a turn that started after it.
    let sessions = h.dir.join("claude-home").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join(format!("{}.json", std::process::id()));
    let write = |at: u64| {
        std::fs::write(
            &file,
            format!(
                r#"{{"pid":{},"sessionId":"{sid}","cwd":"/x","status":"idle","statusUpdatedAt":{at},"updatedAt":{at}}}"#,
                std::process::id()
            ),
        )
        .unwrap();
    };
    write(now_ms() - 60_000);
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(state(&mut c).0, SessionState::Running, "a stale idle must not demote");

    // The Esc: the same file, stamped now. No hook, no transcript record, the
    // pane still painting.
    write(now_ms());
    let deadline = Instant::now() + Duration::from_secs(12);
    let (final_state, conf) = loop {
        let s = state(&mut c);
        if s.0 != SessionState::Running {
            break s;
        }
        assert!(Instant::now() < deadline, "never left Running after the status file went idle");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(final_state, SessionState::Idle { stop_reason: StopReason::Interrupted });
    assert_eq!(conf, Confidence::Medium, "a daemon-side probe is inference");

    // A new prompt: Running again, and the file's idle — now older than this
    // spell — is the last turn's word, not this one's.
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_until(Duration::from_secs(5), "second prompt promoted to Running", || {
        state(&mut c).0 == SessionState::Running
    });
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(state(&mut c).0, SessionState::Running, "the old idle must not re-demote");

    let _ = c.request(Command::KillSession { id: sid });
}

#[test]
fn a_status_file_gone_idle_over_a_closed_turn_is_end_turn_not_interrupted() {
    const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do echo tick; sleep 0.3; done\n";
    let Some(h) =
        Harness::boot_with_env("intrdone", Some(STUB), &[("MESIMON_PANE_QUIET_MS", "600000")])
    else {
        return;
    };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("intrdone");

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "intrdone".into() });
    let ticket = c.board().tickets.first().expect("ticket").id;
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let transcript = h.dir.join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let start_body = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"{}","source":"startup"}}"#,
        transcript.display(),
        h.dir.join("repo").display()
    );
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", &start_body);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    let state = |c: &mut TestClient| {
        let rec = c.board().sessions.iter().find(|r| r.id == sid).expect("record").clone();
        (rec.state, rec.confidence)
    };
    wait_until(Duration::from_secs(5), "UserPromptSubmit promoted to Running", || {
        state(&mut c).0 == SessionState::Running
    });
    let now_ms = || {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
            as u64
    };
    let iso = mesimon_daemon::journal::iso_utc;

    // The turn closes the way current Claude Code writes it: the assistant's
    // end_turn record, then the stop-hook summary, then a latch — and the
    // Stop hook itself never arrives (lost in a stalled exec).
    let closed = now_ms();
    std::fs::write(
        &transcript,
        format!(
            "{{\"uuid\":\"u1\",\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"stop_reason\":\"end_turn\",\"content\":[{{\"type\":\"text\",\"text\":\"done\"}}]}}}}\n\
             {{\"uuid\":\"u2\",\"type\":\"system\",\"subtype\":\"stop_hook_summary\",\"timestamp\":\"{}\"}}\n\
             {{\"type\":\"agent-name\"}}\n",
            iso(closed),
            iso(closed + 400)
        ),
    )
    .unwrap();
    let sessions = h.dir.join("claude-home").join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let file = sessions.join(format!("{}.json", std::process::id()));
    std::fs::write(
        &file,
        format!(
            r#"{{"pid":{},"sessionId":"{sid}","cwd":"/x","status":"idle","statusUpdatedAt":{at},"updatedAt":{at}}}"#,
            std::process::id(),
            at = closed + 450
        ),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    let (final_state, conf) = loop {
        let s = state(&mut c);
        if s.0 != SessionState::Running {
            break s;
        }
        assert!(Instant::now() < deadline, "never left Running after the status file went idle");
        std::thread::sleep(Duration::from_millis(250));
    };
    assert_eq!(final_state, SessionState::Idle { stop_reason: StopReason::EndTurn });
    assert_eq!(conf, Confidence::Medium, "still a daemon-side probe");
    wait_until(Duration::from_secs(5), "automove put the ticket in REVIEW", || {
        c.board().tickets.first().expect("ticket").column == "REVIEW"
    });

    let _ = c.request(Command::KillSession { id: sid });
}
