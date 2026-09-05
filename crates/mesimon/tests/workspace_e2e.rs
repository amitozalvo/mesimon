//! T-225 e2e: a board on a WORKSPACE — a root with repositories nested one
//! level under it — on the wire. A meta repo tracking one file over two child
//! repos → the snapshot's `git.repos` names the children and `changed` is the
//! sum across all three → `DiffList { Checkout }` is one list, the children's
//! rows prefixed by their name, and `DiffFile` opens through the prefix → a
//! worktree ticket's spawn is refused in words instead of minting a worktree
//! of the meta repo.
//!
//! The harness boots on a bare directory and the boot sample sees no repo;
//! the next sample is the 10 s bucket's, so the census is waited for (a
//! `GitFetch` press would re-sample at once, but it refuses without an
//! upstream, and a workspace's meta repo has none).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, DiffTarget, Response};

#[test]
fn a_workspace_of_repos_stands_on_the_wire() {
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let Some(h) = Harness::boot("workspace", None) else { return };
    let root = h.repo.clone();
    // The author's shape: a meta repo that tracks its own notes and ignores
    // every child, over independent repositories.
    init_repo(&root, "CLAUDE.md", "# workspace\n");
    std::fs::write(root.join(".gitignore"), "*/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore children"]);
    init_repo(&root.join("api"), "server.ts", "one\ntwo\n");
    init_repo(&root.join("web"), "page.tsx", "hello\n");
    // A worktree of `web` kept under the root, the way the author keeps
    // `.wt/`: a gitfile, whose owner is `web`, never a repo of its own.
    git(&root.join("web"), &["worktree", "add", "-q", "../.wt-web", "-b", "feedback"]);
    let mut c = h.client("workspace");

    // ---- the census and the summed count ----------------------------------
    // Two files in `api` (an edit and a stray), one in `web`, one at the
    // root. The root's `*/` hides `.wt-web/` from its own status, as it
    // hides the children — a workspace's meta repo sees none of them.
    std::fs::write(root.join("api/server.ts"), "one\nCHANGED\n").unwrap();
    std::fs::write(root.join("api/stray.md"), "new\n").unwrap();
    std::fs::write(root.join("web/page.tsx"), "hello world\n").unwrap();
    std::fs::write(root.join("CLAUDE.md"), "# workspace\n\nmore\n").unwrap();
    // The bucket's first sample can land mid-setup (tick 1 is 250 ms after
    // boot); the one that counts is the first to see the finished tree.
    let deadline = Instant::now() + Duration::from_secs(25);
    let g = loop {
        let g = git_of(c.request(Command::Snapshot));
        if g.sampled && g.repos == ["api", "web"] && g.changed == 4 {
            break g;
        }
        assert!(
            Instant::now() < deadline,
            "never saw root (CLAUDE.md) + api (2) + web (1) = 4 across 2 repos; last: {g:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(g.branch, "main", "the root's own branch still rides, for doctor");
    assert!(g.upstream.is_none() && g.ahead == 0, "{g:?}");

    // ---- one list, prefixed by repo -----------------------------------------
    let resp = c.request(Command::DiffList { target: DiffTarget::Checkout });
    match &resp {
        Response::DiffList { branch, base_oid, .. } => {
            assert_eq!(branch, "main", "the root is a repo: its branch leads, as on the header");
            assert_eq!(base_oid.len(), 40, "the meta's HEAD, for its own rows");
        }
        other => panic!("expected DiffList, got {other:?}"),
    }
    let listed = files_of(&resp);
    let paths: Vec<&str> = listed.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["CLAUDE.md", "api/server.ts", "api/stray.md", "web/page.tsx"],
        "the root's rows first and bare, then each child under its name, in census order"
    );
    let stray = listed.iter().find(|f| f.path == "api/stray.md").unwrap();
    assert_eq!(stray.status, "A", "a child's untracked file is an add, as on one repo");
    assert!(stray.untracked);

    // ---- a file opens through its prefix -------------------------------------
    let resp = c.request(Command::DiffFile {
        target: DiffTarget::Checkout,
        path: "api/server.ts".into(),
        context: 3,
    });
    match resp {
        Response::DiffFile { file } => {
            assert_eq!(file.path, "api/server.ts", "the prefix rides back on the answer");
            assert_eq!(file.hunks.len(), 1, "{file:?}");
        }
        other => panic!("expected DiffFile, got {other:?}"),
    }
    // A path the child's list does not name is refused there, like anywhere.
    let resp = c.request(Command::DiffFile {
        target: DiffTarget::Checkout,
        path: "api/../CLAUDE.md".into(),
        context: 3,
    });
    assert!(matches!(resp, Response::Err { .. }), "{resp:?}");

    // ---- a worktree ticket is refused in words ------------------------------
    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "Fix thing".into() });
    let id = match c.request(Command::Snapshot) {
        Response::Board { board, .. } => board.tickets[0].id,
        other => panic!("expected board, got {other:?}"),
    };
    assert!(matches!(
        c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    match c.request(Command::SpawnSession {
        ticket: id,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Err { message } => {
            assert!(message.contains("workspace of 2 repos"), "{message}");
            assert!(message.contains("shared checkout"), "{message}");
        }
        other => panic!("a worktree of the meta repo must not be minted: {other:?}"),
    }
    // Nothing was provisioned: no binding, no branch, no directory.
    match c.request(Command::Snapshot) {
        Response::Board { worktrees, .. } => assert!(worktrees.is_empty(), "{worktrees:?}"),
        other => panic!("expected board, got {other:?}"),
    }
    assert!(!h.paths.worktrees_root().exists(), "no worktree root was made");
}
