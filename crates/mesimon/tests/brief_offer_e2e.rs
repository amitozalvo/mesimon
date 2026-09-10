//! The agent-brief offer and the two board switches (T-217, T-224), end to end
//! against a real daemon: the offer stands on a repo that never said the
//! words, Enter turns the brief on and persists it, "never ask again" stamps
//! the board, turning the tools off is its own consent flag, and all three
//! survive a restart.
//!
//! No tmux and no agent — every one of these is board state, so this one runs
//! everywhere. The half that needs a pane (a spawn's argv carrying the flag,
//! or not) lives in `brief_e2e.rs`, where the stub agent already is.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::allow_attributes_without_reason, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::command::{Command, Response};

mod common;
use common::*;

/// The snapshot's answer about the repo's CLAUDE.md.
fn status(resp: Response) -> mesimon_core::command::ClaudeMdStatus {
    match resp {
        Response::Board { claude_md, .. } => claude_md,
        other => panic!("expected a board: {other:?}"),
    }
}

#[test]
fn the_brief_offer_switches_persist_and_survive_a_restart() {
    let fixture = common::TestFixture::new("brief-offer");
    let dir = fixture.dir.clone();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let cols = repo.join(".mesimon/board/columns.toml");
    // The daemon canonicalizes (`/tmp` is `/private/tmp` here), and the path
    // the snapshot reports is the canonical one.
    let md = paths.repo_root.join("CLAUDE.md");

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
            client: "brief".into()
        }),
        Response::Hello { .. }
    ));

    // ---- a repo that never said the words ---------------------------------
    let s = status(c.request(Command::Snapshot));
    assert!(!s.present, "a fresh repo has nothing to say about MESIMON_TICKET");
    assert_eq!(s.path, md.display().to_string(), "doctor names the file");
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.mcp_tools, "the tools ship on");
    assert!(!board.system_prompt, "the brief ships OFF: it is opt-in");
    assert!(!board.claude_md_ignored);

    // ---- Enter turns the brief on; nothing on disk but the board ----------
    assert!(matches!(c.request(Command::SetSystemPrompt { on: true }), Response::Ok));
    assert!(board_of(c.request(Command::Snapshot)).system_prompt);
    assert!(!md.exists(), "the brief writes no CLAUDE.md");
    let raw = std::fs::read_to_string(&cols).unwrap();
    assert!(raw.contains("system_prompt = true"), "{raw}");
    // Idempotent, and off is a switch too.
    assert!(matches!(c.request(Command::SetSystemPrompt { on: true }), Response::Ok));
    assert!(matches!(c.request(Command::SetSystemPrompt { on: false }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert!(!board.system_prompt);
    assert!(board.claude_md_ignored, "turning it off is an answer: the offer stays down");
    assert!(matches!(c.request(Command::SetSystemPrompt { on: true }), Response::Ok));

    // ---- "never ask again" is a stamp on the board, not on any file --------
    std::fs::write(&md, "# House rules\n").unwrap();
    let before = std::fs::read_to_string(&md).unwrap();
    assert!(matches!(c.request(Command::IgnoreBriefOffer), Response::Ok));
    assert!(
        board_of(c.request(Command::Snapshot)).system_prompt,
        "the stamp leaves the switch alone"
    );
    assert_eq!(std::fs::read_to_string(&md).unwrap(), before, "ignore touches no file");
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.claude_md_ignored);
    assert!(
        std::fs::read_to_string(&cols).unwrap().contains("claude_md_ignored = true"),
        "the stamp is persisted, or the offer comes back on the next restart"
    );

    // ---- the tools switch --------------------------------------------------
    assert!(matches!(c.request(Command::SetMcpTools { on: false }), Response::Ok));
    assert!(!board_of(c.request(Command::Snapshot)).mcp_tools);
    let raw = std::fs::read_to_string(&cols).unwrap();
    assert!(raw.contains("mcp_tools = false"), "{raw}");
    // A scalar after a table is a TOML error; all three keys sit above.
    let table = raw.find("[[columns]]").expect("the columns table");
    assert!(raw.find("mcp_tools").unwrap() < table, "{raw}");
    assert!(raw.find("claude_md_ignored").unwrap() < table, "{raw}");
    assert!(raw.find("system_prompt").unwrap() < table, "{raw}");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();

    // ---- and all three survive the restart ---------------------------------
    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never came back");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "brief".into()
        }),
        Response::Hello { .. }
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert!(!board.mcp_tools, "a consent flag may not be lost to a restart");
    assert!(board.system_prompt, "and neither may the brief's");
    assert!(board.claude_md_ignored, "nor 'never ask again'");
    // The file is still named, because doctor prints the snippet whatever
    // the stamp says — the stamp only silences the board.
    assert!(!status(c.request(Command::Snapshot)).path.is_empty());

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}

/// An agent may not reach any of the three — the tier is a compile-time
/// match, and this is the runtime half of the same statement. A tier that
/// could write its own system prompt, or switch its tools back on, is not one.
#[test]
fn the_agent_tier_is_denied_all_three() {
    use mesimon_core::mcp::agent_allows;
    assert!(!agent_allows(&Command::SetMcpTools { on: false }));
    assert!(!agent_allows(&Command::SetMcpTools { on: true }));
    assert!(!agent_allows(&Command::SetSystemPrompt { on: true }));
    assert!(!agent_allows(&Command::SetSystemPrompt { on: false }));
    assert!(!agent_allows(&Command::IgnoreBriefOffer));
}
