//! Setup (T-816) end to end against a real daemon: a fresh board waits for
//! it, `ApplySetup` writes the template's columns, the workflow, the
//! workspace a new ticket is stamped with, the agents' tools and the brief
//! in one turn, and a board that already has a ticket keeps its columns.
//!
//! No tmux and no agent: Setup is board state, so this runs everywhere.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use mesimon_core::board::{AgentTools, Board, MergeInto, Template, WorkspaceStrategy};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

fn names(b: &Board) -> Vec<String> {
    b.sorted_columns().iter().map(|c| c.name.clone()).collect()
}

fn setup(template: Template, noun: &str, workspace: WorkspaceStrategy) -> Command {
    Command::ApplySetup {
        template,
        noun: noun.into(),
        merge_into: template.merge_into(),
        workspace,
        agent_tools: AgentTools::Annotate,
    }
}

#[test]
fn setup_writes_the_board_once_and_keeps_a_ticket_s_columns() {
    let fixture = common::TestFixture::new("setup");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let init = std::process::Command::new("git").arg("-C").arg(&repo).args(["init", "-q"]).status();
    assert!(init.unwrap().success(), "git init");
    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let cols_path = repo.join(".mesimon/board/columns.toml");

    let _daemon = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    assert!(matches!(
        c.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "setup".into()
        }),
        Response::Hello { .. }
    ));

    // ---- a fresh board waits for Setup, on today's defaults ----------------
    let board = board_of(c.request(Command::Snapshot));
    assert!(board.setup_pending);
    assert!(!board.system_prompt);
    assert_eq!(board.workspace, None);
    assert_eq!(names(&board), ["TODO", "IN PROGRESS", "REVIEW", "DONE"]);

    // ---- one command writes every answer -----------------------------------
    let r = c.request(setup(Template::PullRequestsProduction, "MR", WorkspaceStrategy::Worktree));
    assert!(matches!(r, Response::Ok), "{r:?}");
    let board = board_of(c.request(Command::Snapshot));
    assert!(!board.setup_pending);
    assert!(board.system_prompt, "Setup showed the brief, and it is on");
    assert_eq!(board.merge_into, MergeInto::PullRequests);
    assert_eq!(board.workspace, Some(WorkspaceStrategy::Worktree));
    assert_eq!(names(&board), ["TODO", "IN PROGRESS", "MR OPEN", "MERGED", "PRODUCTION"]);
    assert!(board.columns.iter().all(|c| c.settings.agent_tools == AgentTools::Annotate));
    let text = std::fs::read_to_string(&cols_path).unwrap();
    assert!(!text.contains("setup_pending"), "{text}");
    assert!(text.contains("workspace = \"worktree\""), "{text}");
    assert!(text.contains("arrives_env = \"production\""), "{text}");

    // ---- a ticket minted with nothing chosen gets the board's workspace -----
    let id = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "Fix the login redirect".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.ticket(id).unwrap().workspace, Some(WorkspaceStrategy::Worktree));

    // ---- once a ticket exists the columns stay ------------------------------
    let r = c.request(setup(Template::Kanban, "PR", WorkspaceStrategy::SharedCheckout));
    assert!(matches!(r, Response::Ok), "{r:?}");
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(names(&board), ["TODO", "IN PROGRESS", "MR OPEN", "MERGED", "PRODUCTION"]);
    assert_eq!(board.merge_into, MergeInto::Main);
    assert_eq!(board.workspace, Some(WorkspaceStrategy::SharedCheckout));
    assert_eq!(
        board.ticket(id).unwrap().workspace,
        Some(WorkspaceStrategy::Worktree),
        "never retroactive"
    );
}

/// A folder with no repository cannot cut a worktree: Setup's worktree
/// answer lands as the shared checkout there.
#[test]
fn a_folder_with_no_repository_works_in_the_shared_checkout() {
    let fixture = common::TestFixture::new("setup-norepo");
    let repo = fixture.dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let sock = fixture.paths(&repo).orch_sock();
    let _daemon = fixture.daemon(&repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);
    let _ = c.request(Command::Hello {
        version: mesimon_core::command::PROTOCOL_VERSION,
        client: "setup".into(),
    });
    let r = c.request(setup(Template::Kanban, "PR", WorkspaceStrategy::Worktree));
    assert!(matches!(r, Response::Ok), "{r:?}");
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.workspace, Some(WorkspaceStrategy::SharedCheckout));
    assert!(!board.setup_pending);
}
