//! The board's own checkout, diffed end to end (T-221).
//!
//! The branch diff has an e2e inside `worktree_e2e`; this is its twin for the
//! target that has no worktree and no ticket. What it proves is the two things
//! the branch road cannot do: that HEAD-vs-working-tree is one row per path
//! whether the edit is staged or not, and that an untracked file — invisible
//! to every diff query mesimon used to make — opens as the adds it is.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::process::Command as Proc;

use mesimon_core::command::{Command, DiffTarget, Response};
use mesimon_core::diff::{Render, Sign};

#[test]
fn the_board_diffs_its_own_checkout() {
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let Some(h) = Harness::boot("diff", None) else { return };
    let repo = h.repo.clone();
    // The harness boots on a bare directory; a checkout needs a repository.
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "e2e@t"]);
    git(&repo, &["config", "user.name", "e2e"]);
    std::fs::write(repo.join("kept.txt"), "one\ntwo\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let mut c = h.client("diff");

    // What a shared-checkout agent leaves behind: an edit it staged and then
    // edited again, and a file it wrote but never added.
    std::fs::write(repo.join("kept.txt"), "one\nCHANGED\n").unwrap();
    git(&repo, &["add", "kept.txt"]);
    std::fs::write(repo.join("kept.txt"), "one\nCHANGED AGAIN\n").unwrap();
    std::fs::write(repo.join("fresh.md"), "# Fresh\n\nwritten by the agent\n").unwrap();

    let resp = c.request(Command::DiffList { target: DiffTarget::Checkout });
    let listed = files_of(&resp);
    match &resp {
        Response::DiffList { branch, base_oid, branch_oid, worktree_present, .. } => {
            assert_eq!(branch, "main");
            assert_eq!(base_oid.len(), 40, "the HEAD this was measured against");
            assert!(branch_oid.is_empty(), "the working tree is not a ref");
            assert!(worktree_present);
        }
        other => panic!("expected DiffList, got {other:?}"),
    }

    // Staged AND edited since is still one row: HEAD to the working tree.
    assert_eq!(listed.iter().filter(|f| f.path == "kept.txt").count(), 1, "{listed:?}");
    let kept = listed.iter().find(|f| f.path == "kept.txt").unwrap();
    assert_eq!(kept.status, "M");
    assert_eq!((kept.adds, kept.dels), (Some(1), Some(1)));

    // The untracked file is an add, not a sighting.
    let fresh = listed.iter().find(|f| f.path == "fresh.md").unwrap();
    assert_eq!(fresh.status, "A");
    assert!(fresh.untracked);
    assert_eq!(fresh.adds, Some(3));

    // …and it opens.
    let file = match c.request(Command::DiffFile {
        target: DiffTarget::Checkout,
        path: "fresh.md".into(),
        context: 3,
    }) {
        Response::DiffFile { file } => file,
        other => panic!("expected DiffFile, got {other:?}"),
    };
    assert_eq!(file.render, Render::Text);
    let lines: Vec<_> = file.hunks.iter().flat_map(|hk| hk.lines.iter()).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines.iter().all(|l| l.sign == Sign::Add), "a new file is nothing but adds");
    assert!(lines.iter().any(|l| l.text.contains("written by the agent")));

    // The tracked edit reads on the same road.
    match c.request(Command::DiffFile {
        target: DiffTarget::Checkout,
        path: "kept.txt".into(),
        context: 3,
    }) {
        Response::DiffFile { file } => {
            assert!(file.hunks[0].lines.iter().any(|l| l.text.contains("CHANGED AGAIN")));
        }
        other => panic!("expected DiffFile, got {other:?}"),
    }

    // A path the list did not name is refused — which is also what fences
    // `--no-index`, whose operands are plain paths.
    err_containing(
        c.request(Command::DiffFile {
            target: DiffTarget::Checkout,
            path: "../../etc/passwd".into(),
            context: 3,
        }),
        "no such file",
    );

    // And the branch target is untouched: still per-branch, still refused
    // where there is no worktree.
    err_containing(
        c.request(Command::DiffList { target: DiffTarget::Ticket { id: ulid::Ulid(999) } }),
        "no worktree",
    );
}

/// A row of the push / pull lists, opened over the wire: the commit against
/// its parent, read from the object store with nothing of the working tree
/// on it, and an id that is not a full hex name refused before git sees it.
#[test]
fn the_board_diffs_one_commit_of_its_history() {
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let Some(h) = Harness::boot("diffcommit", None) else { return };
    let repo = h.repo.clone();
    init_repo(&repo, "kept.txt", "one\ntwo\n");
    std::fs::write(repo.join("kept.txt"), "one\nTWO\n").unwrap();
    git(&repo, &["commit", "-qam", "shout two"]);
    let oid = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    // Uncommitted work beside it must not leak into the commit's diff.
    std::fs::write(repo.join("kept.txt"), "one\nTWO\nthree\n").unwrap();
    let mut c = h.client("diffcommit");

    let target = DiffTarget::Commit { oid: oid.clone(), repo: None };
    let resp = c.request(Command::DiffList { target: target.clone() });
    match &resp {
        Response::DiffList { branch_oid, worktree_present, .. } => {
            assert_eq!(branch_oid, &oid);
            assert!(!worktree_present);
        }
        other => panic!("expected DiffList, got {other:?}"),
    }
    let listed = files_of(&resp);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!((listed[0].adds, listed[0].dels), (Some(1), Some(1)));
    match c.request(Command::DiffFile { target, path: "kept.txt".into(), context: 3 }) {
        Response::DiffFile { file } => {
            let lines: Vec<_> = file.hunks.iter().flat_map(|hk| hk.lines.iter()).collect();
            assert!(lines.iter().any(|l| l.sign == Sign::Add && l.text == "TWO"));
            assert!(!lines.iter().any(|l| l.text == "three"), "the working tree stays out");
        }
        other => panic!("expected DiffFile, got {other:?}"),
    }
    err_containing(
        c.request(Command::DiffList {
            target: DiffTarget::Commit { oid: "HEAD~1".into(), repo: None },
        }),
        "not a commit id",
    );
}
