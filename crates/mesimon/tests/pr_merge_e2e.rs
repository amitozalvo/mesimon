//! A pull request merged somewhere else, end to end (T-267): the ticket's
//! branch is squashed into the base — not one of its commits an ancestor of
//! anything afterwards — and the board reads it as merged anyway.
//!
//! The squash is made in the board's own checkout because that is what a
//! `git pull` leaves behind; the fetch-only half (the same patch on
//! `origin/main` and local `main` never moving) is a unit test beside
//! `compute_flags`, where a bare remote is cheap.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, MergeOutcome, Response};

/// A worktree ticket with a spawned agent and one commit on its branch.
fn ready(c: &mut TestClient, title: &str) -> (ulid::Ulid, uuid::Uuid, String, PathBuf) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let id = c.board().tickets.iter().find(|t| t.title == title).unwrap().id;
    assert!(matches!(
        c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: id,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    let wt = wait_attached(c, id);
    let path = PathBuf::from(wt.path.clone().expect("an attached binding has a path"));
    wait_until(Duration::from_secs(15), "the parked spawn to land", || {
        c.board().sessions.iter().any(|s| s.ticket == id)
    });
    let sid = c.board().sessions.iter().find(|s| s.ticket == id).unwrap().id;
    std::fs::write(path.join(format!("{title}.txt")), format!("{title}\n")).unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-qm", title]);
    (id, sid, wt.branch, path)
}

/// The author's workflow: claude opens the PR, the forge squashes it, the
/// author pulls. mesimon must then stop calling the ticket unmerged — the
/// card's mark, the DONE gate and `m` all read the one answer.
#[test]
fn a_squash_merged_branch_reads_merged() {
    const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do :; done\n";
    // The flags on a 1 s cadence: the squash must be seen without a wait.
    let Some(h) = Harness::boot_with_env(
        "pr-merge",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("pr-merge");
    let (id, sid, branch, _path) = ready(&mut c, "alpha");

    // Before the merge: unmerged, and DONE says so.
    wait_until(Duration::from_secs(15), "the branch to read ahead", || {
        worktrees_of(&mut c).iter().any(|w| w.ticket == id && w.ahead > 0)
    });
    let before = worktrees_of(&mut c).into_iter().find(|w| w.ticket == id).unwrap();
    assert!(!before.merged, "nothing has merged it yet");
    assert!(before.merged_in.is_empty());
    match c.request(Command::MoveTicket { id, column: "DONE".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("unmerged"), "{message}"),
        other => panic!("DONE while unmerged: {other:?}"),
    }

    // The forge's merge, as `git pull` leaves it: one commit carrying the
    // branch's whole diff, and the branch itself an ancestor of nothing.
    git(&repo, &["merge", "-q", "--squash", &branch]);
    git(&repo, &["commit", "-qm", "alpha (#12)"]);
    let squash = String::from_utf8_lossy(
        &std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert!(
        !std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .output()
            .unwrap()
            .status
            .success(),
        "a squash leaves no ancestor behind — that is the whole problem"
    );

    wait_until(Duration::from_secs(20), "the board to read the squash", || {
        worktrees_of(&mut c).iter().any(|w| w.ticket == id && w.merged)
    });
    let after = worktrees_of(&mut c).into_iter().find(|w| w.ticket == id).unwrap();
    assert_eq!(after.merged_in, "main", "it landed on the base");
    assert_eq!(after.merged_oid, squash, "and the commit that carries it is named");
    assert!(!after.needs_rebase, "so it stops asking to be rebased");

    // `m` agrees with the card rather than offering to merge it again — the
    // quiet gate has first say, so the agent goes first (a live claude
    // refuses every merge, merged or not).
    let _ = c.request(Command::KillSession { id: sid });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match c.request(Command::MergeTicket { id }) {
            Response::Merge { outcome: MergeOutcome::AlreadyMerged, detail } => {
                assert!(detail.contains("already in main"), "{detail}");
                break;
            }
            // The session may still be winding down through the reaper.
            Response::Merge { outcome: MergeOutcome::Refused, detail } => {
                assert!(Instant::now() < deadline, "never settled: {detail}");
                std::thread::sleep(Duration::from_millis(250));
            }
            other => panic!("merge on a merged branch: {other:?}"),
        }
    }
    // And the gate opens: the work is landed.
    assert!(
        matches!(
            c.request(Command::MoveTicket { id, column: "DONE".into(), before: None }),
            Response::Ok
        ),
        "DONE after the squash"
    );
}
