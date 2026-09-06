//! M2 attention e2e: real `mesimon hook` binary → daemon ingest → state
//! machine → subscriber push. Uses an in-process daemon (like the M1 e2e) and
//! the real built binary for the hook side (CARGO_BIN_EXE lives here).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

/// Wall-clock budgets are honest on a quiet laptop and flaky on a shared
/// runner. Only the timing bounds relax — nothing about behaviour does.
fn is_ci() -> bool {
    std::env::var_os("MESIMON_CI").is_some()
}

use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, Reason, SessionKind, SessionState};
use mesimon_core::command::{Command, GraceItem, Response};

fn board_of(resp: Response) -> (Board, Vec<GraceItem>) {
    match resp {
        Response::Board { board, grace, .. } => (board, grace),
        other => panic!("expected board, got {other:?}"),
    }
}

#[test]
fn m2_attention_headless() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("hook");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.clone();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let tmux_sock = paths.tmux_sock();

    // The in-process daemon's current_exe() is the TEST binary, which has no
    // `hook` subcommand — point the pane-died notify at the real one.
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    // Child configuration must be complete before launch, not changed mid-test.
    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 120\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "e2e".into() }),
        Response::Hello { .. }
    ));

    // A second, subscribed connection watches for pushes.
    let mut watcher = TestClient::connect(&sock);
    assert!(matches!(
        watcher.request(Command::Hello { version: 1, client: "watch".into() }),
        Response::Hello { .. }
    ));
    assert!(matches!(watcher.request(Command::Subscribe), Response::Ok));

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "attn".into(),
        workspace: None,
    });
    // A pre-existing REVIEW ticket: automoved arrivals must land ABOVE it.
    let _ = c.request(Command::CreateTicket {
        column: "REVIEW".into(),
        title: "decoy".into(),
        workspace: None,
    });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let ticket = board.tickets.iter().find(|t| t.title == "attn").expect("ticket").id;

    // A bash session stands in for the agent pane; hook frames come from us.
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    // Drain the spawn's own BoardChanged push.
    assert!(watcher.next_event(Duration::from_secs(2)).is_some());

    // SessionStart → running (bash spawns Running already; frame is harmless),
    // then PermissionRequest → requires_action{permission}, pushed unprompted.
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t.jsonl","cwd":"/tmp"}"#,
    );
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "PermissionRequest",
        None,
        r#"{"tool_name":"Bash","tool_input":{"command":"npm test"},"prompt_id":"p1"}"#,
    );
    assert!(
        watcher.next_event(Duration::from_secs(2)).is_some(),
        "attention transition must push without any client request"
    );

    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid).expect("session");
    assert_eq!(rec.state, SessionState::RequiresAction { reason: Reason::Permission });
    assert!(rec.waiting_since.is_some(), "attention items carry waiting_since");
    assert_eq!(rec.detail.as_deref(), Some("Bash(npm test)"));
    assert_eq!(rec.transcript_path.as_deref(), Some("/tmp/t.jsonl"));
    // Automove: requires_action is not "working" — the ticket stays in TODO.
    assert_eq!(board.ticket(ticket).unwrap().column, "TODO");

    // The human-deny path: nothing fires but the next prompt; settle clears.
    watcher.drain_events();
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{"session_id":"x"}"#);
    assert!(
        watcher.next_event(Duration::from_secs(3)).is_some(),
        "settled leave must push from the tick wheel"
    );
    // ...but not necessarily THAT push first. `changed` on the wheel is also
    // raised by the RSS refresh, whose figures move whenever the machine is
    // busy — so under a parallel suite a resource push can beat the 1500 ms
    // settle here and the snapshot below reads a state that has not left yet.
    // The event assertion above still stands; the state is polled for.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (board, rec) = loop {
        let (board, _) = board_of(c.request(Command::Snapshot));
        let rec = board.sessions.iter().find(|s| s.id == sid).unwrap().clone();
        if rec.state == SessionState::Running || Instant::now() >= deadline {
            break (board, rec);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(rec.state, SessionState::Running);
    assert!(rec.waiting_since.is_none());
    assert!(rec.detail.is_none());
    // Automove: running drags the TODO ticket to IN PROGRESS.
    assert_eq!(board.ticket(ticket).unwrap().column, "IN PROGRESS");

    // Hook binary cost sanity (debug build — the 5 ms p99 budget is a release
    // number; this catches order-of-magnitude regressions only).
    let mut worst = Duration::ZERO;
    for _ in 0..10 {
        let t0 = Instant::now();
        hook_send_with(&hook_sock, &sid.to_string(), "Stop", None, r#"{"stop_hook_active":true}"#);
        worst = worst.max(t0.elapsed());
    }
    // The real budget is 5 ms p99 (14 §1.7); 150 ms is the debug-build slack.
    // A shared CI runner adds scheduling noise that says nothing about the
    // hook, so the tight bound stays a local signal.
    let budget = Duration::from_millis(if is_ci() { 1000 } else { 150 });
    assert!(worst < budget, "hook exec took {worst:?} (budget {budget:?})");

    // Automove: a real Stop (the cost-loop frames set stop_hook_active, which
    // the machine's re-entrancy guard drops) → idle{end_turn} after the
    // 1500 ms leave settle drags the IN PROGRESS ticket to REVIEW.
    hook_send_with(&hook_sock, &sid.to_string(), "Stop", None, r#"{"stop_hook_active":false}"#);
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let (board, _) = board_of(c.request(Command::Snapshot));
        if board.ticket(ticket).unwrap().column == "REVIEW" {
            let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
            assert_eq!(
                rec.state,
                SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
            );
            // Automoved tickets land at the TOP of the destination column,
            // above the pre-existing decoy.
            let review: Vec<_> = board.column_tickets("REVIEW").iter().map(|t| t.id).collect();
            assert_eq!(review.first(), Some(&ticket), "automove must land at top of REVIEW");
            assert_eq!(review.len(), 2, "decoy still in REVIEW");
            break;
        }
        assert!(Instant::now() < deadline, "end_turn never automoved the ticket to REVIEW");
        std::thread::sleep(Duration::from_millis(100));
    }

    // Automove: review feedback reopens the work — running again drags the
    // REVIEW ticket back to IN PROGRESS.
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{"session_id":"x"}"#);
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let (board, _) = board_of(c.request(Command::Snapshot));
        if board.ticket(ticket).unwrap().column == "IN PROGRESS" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "running never automoved the ticket back to IN PROGRESS"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // A turn that starts with no prompt at all (T-228, 2026-09-05): a `!`
    // bash command in Claude Code puts its output into the conversation and
    // the model takes a turn on it, and no `UserPromptSubmit` fires. The first
    // frame of that turn is the lead's own PostToolUse, and it must read as
    // working — the ticket comes back from REVIEW — where before it was held
    // inert against a hook-stated end_turn and the card sat done for the
    // whole turn.
    hook_send_with(&hook_sock, &sid.to_string(), "Stop", None, r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(4), "end_turn automoves the ticket to REVIEW", || {
        board_of(c.request(Command::Snapshot)).0.ticket(ticket).unwrap().column == "REVIEW"
    });
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "PostToolUse",
        None,
        r#"{"tool_name":"Bash","tool_input":{"command":"gcloud logging read"},"tool_response":{}}"#,
    );
    wait_until(
        Duration::from_secs(4),
        "a promptless turn's first tool frame reads as working and reopens the ticket",
        || {
            let (board, _) = board_of(c.request(Command::Snapshot));
            let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
            rec.state == SessionState::Running
                && board.ticket(ticket).unwrap().column == "IN PROGRESS"
        },
    );

    // Absent socket must be silently fine (rule 7) — daemon-down is invisible.
    hook_send_with(
        &std::path::PathBuf::from("/tmp/msmn-no-such.sock"),
        &sid.to_string(),
        "Stop",
        None,
        "{}",
    );

    // Claude spawn: enters Spawning with a generated 0600 settings file.
    // The stub ignores its args and stays alive so reconcile sees a live pane.
    let claude_sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("claude spawn failed: {other:?}"),
    };
    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == claude_sid).unwrap();
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.argv.iter().any(|a| a == "--settings"));
    // T-84: the MCP server travels on argv and is installed nowhere. The blob
    // names this binary, the daemon's own socket, and the record uuid — so a
    // session mesimon did not spawn can never reach these tools.
    let mcp_pos = rec.argv.iter().position(|a| a == "--mcp-config").expect("--mcp-config on argv");
    let blob: serde_json::Value = serde_json::from_str(&rec.argv[mcp_pos + 1]).unwrap();
    let server = &blob["mcpServers"]["mesimon"];
    assert_eq!(server["type"], "stdio");
    let args: Vec<&str> =
        server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(args[0], "mcp");
    assert!(args.contains(&claude_sid.to_string().as_str()), "bound to the record uuid");
    // Subtractive magic check: the user's own MCP servers still load.
    assert!(!rec.argv.iter().any(|a| a == "--strict-mcp-config"));
    let settings = state_dir.join("hooks").join(format!("{claude_sid}.json"));
    assert!(settings.is_file(), "settings file written");
    let mode = std::os::unix::fs::MetadataExt::mode(&settings.metadata().unwrap()) & 0o777;
    assert_eq!(mode, 0o600, "settings must be 0600");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    let n: usize =
        parsed["hooks"].as_object().unwrap().values().map(|a| a.as_array().unwrap().len()).sum();
    assert_eq!(n, 32, "31 observer entries plus the PreToolUse gate");
    // Prefill: the ticket title is typed into the fresh pane, never submitted
    // (the pty echoes it even though the stub never reads stdin).
    let claude_sid16 = rec.sid16();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let cap = Proc::new("tmux")
            .args([
                "-S",
                &tmux_sock.display().to_string(),
                "capture-pane",
                "-p",
                "-t",
                &claude_sid16,
            ])
            .output()
            .expect("tmux capture-pane");
        // capture-pane trims trailing spaces, so match without the one we send.
        if String::from_utf8_lossy(&cap.stdout).contains("attn") {
            break;
        }
        assert!(Instant::now() < deadline, "ticket-title prefill never appeared in the pane");
        std::thread::sleep(Duration::from_millis(100));
    }
    // ...and the pane's cursor is still on that first line: mesimon typed the
    // title and stopped there. This is the README's zero-injection default,
    // asserted rather than assumed.
    let cursor_y = |sid16: &str| -> String {
        let out = Proc::new("tmux")
            .args([
                "-S",
                &tmux_sock.display().to_string(),
                "display-message",
                "-p",
                "-t",
                sid16,
                "#{cursor_y}",
            ])
            .output()
            .expect("tmux display-message");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(cursor_y(&claude_sid16), "0", "a plain spawn must never press Enter");
    // A ticket holds ONE claude (2026-09-02): while this one is alive a second
    // is refused, and the message names the gesture that reaches the seat.
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
    }) {
        Response::Err { message } => {
            assert!(message.contains("already has a claude"), "{message}");
            assert!(message.contains("focus"), "{message}");
        }
        other => panic!("a second claude on one ticket must be refused, got {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: claude_sid });
    // Killed is Exited, not live: the seat is free again.

    // The composer's Shift+Enter (submit_prompt): the SAME prefill, plus an
    // Enter that mesimon owes the session. It is not paid at spawn — T-5 arm C
    // (2026-08-31): an Enter sent with the text is eaten by Claude's paste
    // detection. `SessionStart` STARTS the payment, and `UserPromptSubmit`
    // ends it; in between mesimon keeps pressing, because that frame can beat
    // Claude's input loop by milliseconds (dogfood 2026-08-31).
    let submit_sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("claude spawn failed: {other:?}"),
    };
    let (board, _) = board_of(c.request(Command::Snapshot));
    let submit_rec = board.sessions.iter().find(|s| s.id == submit_sid).unwrap();
    assert!(submit_rec.pending_submit, "the Enter is owed, not yet paid");
    let submit_sid16 = submit_rec.sid16();
    let deadline = Instant::now() + Duration::from_secs(3);
    while cursor_y(&submit_sid16) == "0" && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(cursor_y(&submit_sid16), "0", "the Enter must NOT land at spawn time");

    hook_send_with(
        &hook_sock,
        &submit_sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t2.jsonl","cwd":"/tmp"}"#,
    );
    // The first press lands on the SessionStart edge...
    hook_send_with(
        &hook_sock,
        &submit_sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t2.jsonl","cwd":"/tmp"}"#,
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    while cursor_y(&submit_sid16) == "0" {
        assert!(Instant::now() < deadline, "SessionStart never started the delivery");
        std::thread::sleep(Duration::from_millis(50));
    }
    // ...and because this stub never acks, mesimon must press AGAIN. That
    // retry is the whole fix: one press on the SessionStart edge is a race
    // Claude's startup can win.
    let deadline = Instant::now() + Duration::from_secs(4);
    while cursor_y(&submit_sid16) == "1" {
        assert!(
            Instant::now() < deadline,
            "an unacknowledged Enter was never retried — the delivery is one-shot again"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == submit_sid).unwrap();
    assert!(rec.pending_submit, "still owed until Claude acknowledges it");

    // UserPromptSubmit IS the ack (T-5): the pressing stops, and stays stopped.
    hook_send_with(&hook_sock, &submit_sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let (board, _) = board_of(c.request(Command::Snapshot));
        let rec = board.sessions.iter().find(|s| s.id == submit_sid).unwrap();
        if !rec.pending_submit {
            break;
        }
        assert!(Instant::now() < deadline, "the ack never closed the offer");
        std::thread::sleep(Duration::from_millis(50));
    }
    let settled = cursor_y(&submit_sid16);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(cursor_y(&submit_sid16), settled, "an acknowledged prompt must stop the pressing");
    let _ = c.request(Command::KillSession { id: submit_sid });

    // pane-died: SIGKILL the process behind a fresh bash pane; the tmux hook
    // must push exited{crashed} with no polling anywhere.
    let sid2 = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Bash,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let (board, _) = board_of(c.request(Command::Snapshot));
    let sid16 = board.sessions.iter().find(|s| s.id == sid2).unwrap().sid16();
    let pid_out = Proc::new("tmux")
        .args([
            "-S",
            &tmux_sock.display().to_string(),
            "display-message",
            "-p",
            "-t",
            &sid16,
            "#{pane_pid}",
        ])
        .output()
        .expect("tmux display-message");
    let pid = String::from_utf8_lossy(&pid_out.stdout).trim().to_string();
    assert!(!pid.is_empty(), "pane pid");
    watcher.drain_events();
    let t0 = Instant::now();
    let _ = Proc::new("kill").args(["-9", &pid]).status().unwrap();
    assert!(
        watcher.next_event(Duration::from_secs(2)).is_some(),
        "pane death must push without polling"
    );
    let latency = t0.elapsed();
    let (board, _) = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid2).unwrap();
    assert_eq!(
        rec.state,
        SessionState::Exited { reason: mesimon_core::board::ExitReason::Crashed }
    );
    let budget = Duration::from_millis(if is_ci() { 5000 } else { 1000 });
    assert!(latency < budget, "pane-died push took {latency:?} (budget {budget:?})");

    let _ = c.request(Command::KillSession { id: sid });
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();

    // The activity feed carries the transitions and the board mutations.
    let feed = std::fs::read_to_string(state_dir.join("activity.jsonl")).expect("feed exists");
    assert!(
        feed.lines().any(|l| l.contains(r#""kind":"session_state""#)
            && l.contains(r#""to":"requires_action""#)
            && l.contains(r#""reason":"permission""#)),
        "feed must carry the permission transition"
    );
    assert!(
        feed.lines().any(|l| l.contains(r#""kind":"board""#) && l.contains("create_ticket")),
        "feed must carry board mutations"
    );
    assert!(
        feed.lines().any(|l| l.contains(r#""kind":"board""#) && l.contains("automove")),
        "feed must carry automoves"
    );
}
