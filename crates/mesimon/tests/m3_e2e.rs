//! M3 adoption + sleep e2e: fabricated `~/.claude` tree → census → attach →
//! takeover via `--resume` (stub claude records its argv), then hook-driven
//! Idle → sleep (pane gone, transcript copied, state latched) → wake.
//!
//! Same harness shape as the M2 hook e2e: in-process daemon, real tmux,
//! real hook binary via CARGO_BIN_EXE. One test fn — env seams are
//! process-global.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::os::unix::net::UnixStream;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Provenance, SessionKind, SessionState, StopReason};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

/// Poll snapshots until the session reaches a state, or panic at deadline.
fn wait_state(
    c: &mut TestClient,
    id: uuid::Uuid,
    timeout: Duration,
    pred: impl Fn(&SessionState) -> bool,
) -> SessionState {
    let deadline = Instant::now() + timeout;
    loop {
        let board = board_of(c.request(Command::Snapshot));
        let state = board.sessions.iter().find(|s| s.id == id).map(|s| s.state.clone());
        if let Some(s) = &state {
            if pred(s) {
                return s.clone();
            }
        }
        assert!(Instant::now() < deadline, "state never reached; last: {state:?}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

const FOREIGN_SID: &str = "cafe0000-1111-4e6f-8b1a-2c3d4e5f6a7b";

#[test]
fn m3_adoption_and_sleep() {
    if !common::require_tmux() {
        return;
    }
    let fixture = common::TestFixture::new("m3");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let state_dir = paths.state_dir.clone();
    let tmux_sock = paths.tmux_sock();
    let repo_canon = paths.repo_root.display().to_string();

    // Fabricated claude home: one foreign transcript in this repo's cwd.
    let claude_home = dir.join("claude-home");
    let proj = claude_home.join("projects").join("-gibberish-slug-never-parsed");
    std::fs::create_dir_all(&proj).unwrap();
    let transcript = proj.join(format!("{FOREIGN_SID}.jsonl"));
    std::fs::write(
        &transcript,
        format!(
            "{{\"sessionId\":\"{FOREIGN_SID}\",\"cwd\":\"{repo_canon}\",\"type\":\"user\",\"uuid\":\"u0\",\"message\":{{}}}}\n\
             {{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"foreign work in flight\"}}]}}}}\n"
        ),
    )
    .unwrap();

    // Stub claude: records argv, dies politely on TERM.
    let argv_log = dir.join("claude-argv.log");
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\necho \"$@\" >> \"{}\"\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n",
            argv_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    fixture.set_env("MESIMON_CLAUDE_HOME", &claude_home);
    fixture.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    // 1 s guard cadence so the sleeping-survives-server-death regression below
    // fits in test time (real cadence 15 s).
    fixture.set_env("MESIMON_SERVER_GUARD_TICKS", "4");

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "m3".into()
        }),
        Response::Hello { .. }
    ));
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "adopt".into(),
        workspace: None,
    });
    let ticket = board_of(c.request(Command::Snapshot)).tickets[0].id;

    // --- Census: lazy, drawer-open only.
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.sessions.is_empty());
    let external = match c.request(Command::RescanExternal) {
        Response::Board { external, .. } => external,
        other => panic!("rescan: {other:?}"),
    };
    assert_eq!(external.len(), 1, "census must find the foreign transcript");
    let item = &external[0];
    assert_eq!(item.id.to_string(), FOREIGN_SID);
    assert_eq!(item.preview.as_deref(), Some("foreign work in flight"));
    assert!(!item.running_elsewhere);

    // --- Import: mints its own ticket (named from the preview here — no
    // pid-file name exists), record is observe-only: no pane, no argv.
    let foreign: uuid::Uuid = FOREIGN_SID.parse().unwrap();
    let obs = match c.request(Command::AttachExternal { claude_session_id: foreign, ticket: None })
    {
        Response::Spawned { id, .. } => id,
        other => panic!("attach: {other:?}"),
    };
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == obs).unwrap();
    assert_eq!(rec.provenance, Provenance::Adopted);
    assert_eq!(rec.claude_session_id, Some(foreign));
    assert!(rec.argv.is_empty());
    assert_eq!(rec.detail.as_deref(), Some("foreign work in flight"));
    assert_ne!(rec.ticket, ticket, "import must mint its own ticket");
    let minted = board.ticket(rec.ticket).expect("minted ticket");
    assert_eq!(minted.title, "foreign work in flight");
    assert_eq!(board.tickets.len(), 2);
    // Observe-only records refuse focus.
    assert!(matches!(c.request(Command::FocusStart { session: obs }), Response::Err { .. }));
    // And the agent tier: no argv of ours means no tool config was ever
    // handed out, so a call claiming this record is nobody mesimon started.
    assert!(matches!(
        c.send(Principal::Agent { session: obs }, Command::AgentGetTicket),
        Response::Err { .. }
    ));
    // A second attach of the same claude session refuses (no orphan ticket).
    assert!(matches!(
        c.request(Command::AttachExternal { claude_session_id: foreign, ticket: None }),
        Response::Err { .. }
    ));
    assert_eq!(board_of(c.request(Command::Snapshot)).tickets.len(), 2);

    // --- Daemon restart: an observe-only record has no pane by design —
    // reconcile must not demote it to exited{crashed}.
    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    let daemon_repo2 = repo.clone();
    let daemon = fixture.daemon(&daemon_repo2);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if sock.exists() {
            if let Ok(s) = UnixStream::connect(&sock) {
                drop(s);
                break;
            }
        }
        assert!(Instant::now() < deadline, "daemon did not come back");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "m3b".into()
        }),
        Response::Hello { .. }
    ));
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == obs).expect("record survived restart");
    assert!(
        rec.state.is_live(),
        "observe-only record must survive a daemon restart, got {:?}",
        rec.state
    );

    // --- Takeover: spawn `claude --resume <foreign>` with our hooks.
    let resumed = match c.request(Command::ResumeSession { id: obs, confirm: false }) {
        Response::Spawned { id, .. } => id,
        other => panic!("resume: {other:?}"),
    };
    assert_eq!(resumed, obs);
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == obs).unwrap();
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.argv.iter().any(|a| a == "--resume"));
    assert!(rec.argv.iter().any(|a| a == "--settings"), "takeover must inject hooks");
    assert!(rec.argv.iter().any(|a| a == "--mcp-config"), "takeover must inject the tools");
    // T-240: a taken-over session keeps `Adopted` for life, and the gate on the
    // agent tier once read provenance alone — so the tools it was just handed
    // were refused on every call. The record is now one mesimon launched.
    match c.send(Principal::Agent { session: obs }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket, .. } => assert_eq!(ticket.title, "foreign work in flight"),
        other => panic!("a taken-over session must reach its tools, got {other:?}"),
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let logged = std::fs::read_to_string(&argv_log).unwrap_or_default();
        if logged.contains("--resume") && logged.contains(FOREIGN_SID) {
            break;
        }
        assert!(Instant::now() < deadline, "stub never saw --resume; log: {logged}");
        std::thread::sleep(Duration::from_millis(100));
    }
    // Prefill is fresh-spawn-only: a resume must not retype the ticket title
    // into the restored conversation.
    let title = &board.tickets.iter().find(|t| t.id == rec.ticket).expect("takeover ticket").title;
    let cap = Proc::new("tmux")
        .args(["-S", &tmux_sock.display().to_string(), "capture-pane", "-p", "-t", &rec.sid16()])
        .output()
        .expect("tmux capture-pane");
    let pane = String::from_utf8_lossy(&cap.stdout).to_string();
    assert!(!pane.contains(title.as_str()), "resume must not prefill; pane: {pane}");
    // Double-resume guard path 1: it is live under mesimon now.
    assert!(matches!(
        c.request(Command::ResumeSession { id: obs, confirm: false }),
        Response::Err { .. }
    ));

    // --- Sleep: hooks say Idle first (SessionStart carries the transcript).
    hook_send_with(
        &hook_sock,
        &obs.to_string(),
        "SessionStart",
        Some("resume"),
        &format!("{{\"session_id\":\"{FOREIGN_SID}\",\"transcript_path\":\"{}\",\"cwd\":\"{repo_canon}\"}}", transcript.display()),
    );
    hook_send_with(&hook_sock, &obs.to_string(), "Stop", None, r#"{"stop_hook_active":false}"#);
    wait_state(&mut c, obs, Duration::from_secs(5), |s| {
        matches!(s, SessionState::Idle { stop_reason: StopReason::EndTurn })
    });

    assert!(matches!(c.request(Command::SleepSession { id: obs }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == obs).unwrap();
    assert_eq!(rec.state, SessionState::Sleeping);
    assert!(rec.detail.is_none(), "transcript copy holds a conversation — no warning");
    let copy = state_dir.join("transcripts").join(format!("{obs}.jsonl"));
    assert!(copy.is_file(), "sleep must copy the transcript");
    // The pane goes away (TERM'd stub → pane-died harvest, reaper backstop)
    // and the record stays Sleeping — the machine latch swallows the death.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let panes = Proc::new("tmux")
            .args([
                "-S",
                &tmux_sock.display().to_string(),
                "list-panes",
                "-a",
                "-F",
                "#{session_name}",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        if !panes.contains(&rec.sid16()) {
            break;
        }
        assert!(Instant::now() < deadline, "pane never reaped: {panes}");
        std::thread::sleep(Duration::from_millis(250));
    }
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(
        board.sessions.iter().find(|s| s.id == obs).unwrap().state,
        SessionState::Sleeping,
        "pane death must not flip a sleeping record"
    );
    // Sleeping the only session empties the private tmux server, which then
    // exits (exit-empty). The server-alive guard must not touch a Sleeping
    // record — it has no pane to lose. Regression: guard keyed off is_live
    // re-minted slept machines as Unknown{SupervisorDead}, and the tail
    // backfill flipped them back — a "?"/"✓" flicker forever.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let alive = Proc::new("tmux")
            .args(["-S", &tmux_sock.display().to_string(), "has-session"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "empty private server never exited");
        std::thread::sleep(Duration::from_millis(250));
    }
    // > 2 guard periods at the 1 s seam.
    std::thread::sleep(Duration::from_millis(2500));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(
        board.sessions.iter().find(|s| s.id == obs).unwrap().state,
        SessionState::Sleeping,
        "tmux server death must not flip a sleeping record"
    );

    // Sleeping refuses focus; a slept session is skipped by the census (known id).
    assert!(matches!(c.request(Command::FocusStart { session: obs }), Response::Err { .. }));
    let external = match c.request(Command::RescanExternal) {
        Response::Board { external, .. } => external,
        other => panic!("rescan: {other:?}"),
    };
    assert!(external.is_empty(), "attached session must not re-surface in the drawer");

    // --- Wake: same record, argv replayed (already a --resume argv).
    std::fs::write(&argv_log, "").unwrap();
    let woke = match c.request(Command::WakeSession { id: obs }) {
        Response::Spawned { id, .. } => id,
        other => panic!("wake: {other:?}"),
    };
    assert_eq!(woke, obs);
    wait_state(&mut c, obs, Duration::from_secs(3), |s| matches!(s, SessionState::Spawning));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let logged = std::fs::read_to_string(&argv_log).unwrap_or_default();
        if logged.contains("--resume") {
            break;
        }
        assert!(Instant::now() < deadline, "wake never respawned the stub");
        std::thread::sleep(Duration::from_millis(100));
    }

    // --- Re-import after exit: killing the pane leaves an Exited record; the
    // census re-lists the conversation (not shadow-banned by its own corpse),
    // and ResumeExternal reuses the dead record and its ticket — no duplicate.
    let board = board_of(c.request(Command::Snapshot));
    let home_ticket = board.sessions.iter().find(|s| s.id == obs).unwrap().ticket;
    let tickets_before = board.tickets.len();
    assert!(matches!(c.request(Command::KillSession { id: obs }), Response::Ok));
    // Let the TERM'd stub die and its pane-died frame land before respawning.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let panes = Proc::new("tmux")
            .args([
                "-S",
                &tmux_sock.display().to_string(),
                "list-panes",
                "-a",
                "-F",
                "#{session_name}",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        if !panes.contains(&board.sessions.iter().find(|s| s.id == obs).unwrap().sid16()) {
            break;
        }
        assert!(Instant::now() < deadline, "killed pane never reaped: {panes}");
        std::thread::sleep(Duration::from_millis(250));
    }
    std::thread::sleep(Duration::from_millis(500));
    let external = match c.request(Command::RescanExternal) {
        Response::Board { external, .. } => external,
        other => panic!("rescan: {other:?}"),
    };
    assert_eq!(external.len(), 1, "exited import must re-surface in the drawer");
    std::fs::write(&argv_log, "").unwrap();
    let back = match c.request(Command::ResumeExternal {
        claude_session_id: foreign,
        ticket: None,
        confirm: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("re-import resume: {other:?}"),
    };
    assert_eq!(back, obs, "re-import must reuse the dead record, not mint a new one");
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.len(), tickets_before, "re-import must not mint a ticket");
    let rec = board.sessions.iter().find(|s| s.id == obs).unwrap();
    assert_eq!(rec.ticket, home_ticket, "the conversation lands back on its ticket");
    assert!(rec.argv.iter().any(|a| a == "--resume"));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let logged = std::fs::read_to_string(&argv_log).unwrap_or_default();
        if logged.contains("--resume") && logged.contains(FOREIGN_SID) {
            break;
        }
        assert!(Instant::now() < deadline, "re-import never respawned the stub; log: {logged}");
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = c.request(Command::KillSession { id: obs });

    // --- Resume with no transcript: a spawned session killed before its
    // first prompt never wrote one. The old contract refused up front, which
    // beat the thing it was written against — spawning `claude --resume <id>`,
    // watching it exit 1 and filing that as a crash — but it left the record
    // permanently un-enterable while its row went on offering "resume". Now
    // it starts a FRESH conversation in the same record: nothing is lost,
    // because there was nothing. The argv is what proves the original concern
    // still holds — no doomed `--resume` is ever spawned.
    let ghost = match c.request(Command::SpawnSession {
        ticket: home_ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("ghost spawn: {other:?}"),
    };
    assert!(matches!(c.request(Command::KillSession { id: ghost }), Response::Ok));
    match c.request(Command::ResumeSession { id: ghost, confirm: false }) {
        Response::Spawned { fresh, .. } => {
            assert!(fresh, "a resume with nothing to resume is a fresh conversation")
        }
        other => panic!("ghost resume must start fresh, got {other:?}"),
    }
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == ghost).unwrap();
    assert!(rec.argv.iter().any(|a| a == "--session-id"), "argv: {:?}", rec.argv);
    assert!(
        !rec.argv.iter().any(|a| a == "--resume"),
        "never spawn a resume that can only exit 1: {:?}",
        rec.argv
    );
    let hosting = rec.claude_session_id.expect("record points at its new conversation");
    assert_ne!(hosting, ghost, "a newly minted id, never the one with no transcript");

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
