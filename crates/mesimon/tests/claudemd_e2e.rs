//! The CLAUDE.md offer and the agent-tools switch (T-217), end to end against
//! a real daemon: the offer stands on a repo that never said the words, Enter
//! writes the file, "never ask again" stamps the board, and turning the tools
//! off withdraws the offer and survives a restart.
//!
//! No tmux and no agent — both switches are board state and one file, so this
//! one runs everywhere. The half that needs a pane (a spawn carrying no
//! `--mcp-config`) lives in `mcp_e2e.rs`, where the stub agent already is.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::allow_attributes_without_reason, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::claudemd;
use mesimon_core::command::{ClaudeMdAction, Command, Response};

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
fn the_claude_md_offer_writes_once_and_can_be_put_away() {
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-claudemd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();
    let cols = repo.join(".mesimon/board/columns.toml");
    // The daemon canonicalizes (`/tmp` is `/private/tmp` here), and the path
    // the snapshot reports is the canonical one.
    let md = paths.repo_root.join("CLAUDE.md");

    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "claudemd".into() }),
        Response::Hello { .. }
    ));

    // ---- a repo that never said the words ---------------------------------
    let s = status(c.request(Command::Snapshot));
    assert!(!s.present, "a fresh repo has nothing to say about MESIMON_TICKET");
    assert!(!s.exists, "and no CLAUDE.md at all");
    assert_eq!(s.path, md.display().to_string(), "the dialog names the file it would write");
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.mcp_tools, "the tools ship on");
    assert!(!board.claude_md_ignored);

    // ---- Enter writes it, once --------------------------------------------
    assert!(matches!(c.request(Command::ClaudeMd { action: ClaudeMdAction::Apply }), Response::Ok));
    let body = std::fs::read_to_string(&md).unwrap();
    assert_eq!(body, claudemd::SNIPPET, "a missing file becomes the snippet alone");
    let s = status(c.request(Command::Snapshot));
    assert!(s.present, "the write withdraws the offer");
    assert!(s.exists);

    // A second Apply is a no-op, not a second copy: the marker it wrote is
    // what refuses it.
    assert!(matches!(c.request(Command::ClaudeMd { action: ClaudeMdAction::Apply }), Response::Ok));
    assert_eq!(std::fs::read_to_string(&md).unwrap(), body, "applying twice writes once");

    // ---- "never ask again" is a stamp on the board, not on the file -------
    std::fs::write(&md, "# House rules\n").unwrap();
    // Let the mtime gate see a different file.
    std::thread::sleep(Duration::from_millis(20));
    let before = std::fs::read_to_string(&md).unwrap();
    assert!(matches!(
        c.request(Command::ClaudeMd { action: ClaudeMdAction::Ignore }),
        Response::Ok
    ));
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
    // A scalar after a table is a TOML error; both new keys sit above.
    let table = raw.find("[[columns]]").expect("the columns table");
    assert!(raw.find("mcp_tools").unwrap() < table, "{raw}");
    assert!(raw.find("claude_md_ignored").unwrap() < table, "{raw}");

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();

    // ---- and both survive the restart --------------------------------------
    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never came back");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello { version: 1, client: "claudemd".into() }),
        Response::Hello { .. }
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert!(!board.mcp_tools, "a consent flag may not be lost to a restart");
    assert!(board.claude_md_ignored, "and neither may 'never ask again'");
    // The file it would write is still named, because doctor prints the
    // snippet whatever the stamp says — the stamp only silences the board.
    assert!(!status(c.request(Command::Snapshot)).path.is_empty());

    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    for d in [&dir, &state_dir, &rt_dir] {
        sweep(d);
    }
}

/// An agent may not reach either command — the tier is a compile-time match,
/// and this is the runtime half of the same statement.
#[test]
fn the_agent_tier_is_denied_both() {
    use mesimon_core::mcp::agent_allows;
    assert!(!agent_allows(&Command::SetMcpTools { on: false }));
    assert!(!agent_allows(&Command::SetMcpTools { on: true }));
    assert!(!agent_allows(&Command::ClaudeMd { action: ClaudeMdAction::Apply }));
    assert!(!agent_allows(&Command::ClaudeMd { action: ClaudeMdAction::Ignore }));
}
