//! The archive reclaims a worktree whose work has landed (T-278): an archived
//! ticket with a merged branch and nothing awake loses its directory (and
//! its branch where `branch -d` allows it) through the delete's own
//! teardown road; unmerged work keeps its tree exactly as before; and a
//! restored ticket gets its worktree back the way a first spawn does — a
//! fresh branch where the old one went, the same branch (`Evicted` replays)
//! where a squash left it standing — the wake included. The header's X offer
//! takes the same road (T-481: it once skipped the reclaim and left no feed
//! row), and an archived tree whose work landed after the archive, or whose
//! teardown a restart swallowed, is reclaimed by the daemon's own sweep.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mesimon_core::board::{ColumnOffers, SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, MergeOutcome, Response};

const STUB: &str = "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

fn branch_exists(repo: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
        .output()
        .unwrap()
        .status
        .success()
}

/// A worktree ticket with a spawned (stub) claude and one commit on its branch.
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

/// Kill the ticket's claude and wait for the reaper: the archive gate wants
/// nothing on the ticket holding a pane.
fn quiet(c: &mut TestClient, id: ulid::Ulid, sid: uuid::Uuid) {
    let _ = c.request(Command::KillSession { id: sid });
    wait_until(Duration::from_secs(15), "the ticket to go quiet", || {
        c.board().sessions.iter().filter(|s| s.ticket == id).all(|s| !s.state.has_pane())
    });
}

/// `m` until the merge goes through (the sample may still be catching up).
fn merge(c: &mut TestClient, id: ulid::Ulid) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match c.request(Command::MergeTicket { id }) {
            Response::Merge { outcome: MergeOutcome::Merged, .. } => return,
            Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
                assert!(Instant::now() < deadline, "merge never went through: {detail}");
                std::thread::sleep(Duration::from_millis(250));
            }
            other => panic!("unexpected merge response: {other:?}"),
        }
    }
}

/// Merged into main by `m`, archived: the directory goes, the branch goes
/// (`-d` on an ancestor), the binding goes — and a restore + spawn builds a
/// fresh worktree on a fresh branch of the same name.
#[test]
fn archive_reclaims_a_landed_worktree() {
    let Some(h) = Harness::boot("arch-reclaim", Some(STUB)) else { return };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-reclaim");
    let (id, sid, branch, path) = ready(&mut c, "alpha");
    quiet(&mut c, id, sid);
    merge(&mut c, id);
    assert!(path.is_dir() && branch_exists(&repo, &branch), "the merge alone removes nothing");

    assert!(matches!(c.request(Command::ArchiveTicket { id }), Response::Ok));
    wait_until(Duration::from_secs(20), "the archive to reclaim the worktree", || {
        !path.exists() && !branch_exists(&repo, &branch) && wt_of(&mut c, id).is_none()
    });
    let board = c.board();
    let t = board.ticket(id).expect("archived, not deleted");
    assert!(t.is_archived());
    assert!(
        board.sessions.iter().any(|s| s.ticket == id),
        "the session record is the ticket's history and stays"
    );

    // Restore, spawn: a first spawn's road — provisioned fresh off the base,
    // the same name, no work on it.
    assert!(matches!(c.request(Command::UnarchiveTicket { id }), Response::Ok));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: id,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    let wt = wait_attached(&mut c, id);
    assert_eq!(wt.branch, branch, "the same branch name, minted again");
    assert_eq!(wt.path.as_deref(), Some(path.to_str().unwrap()), "the same directory");
    assert_eq!(wt.ahead, 0, "a fresh branch off the base");
    wait_until(Duration::from_secs(15), "the parked spawn to land in the new tree", || {
        c.board().sessions.iter().any(|s| s.ticket == id && s.state.has_pane())
    });
    let live =
        c.board().sessions.into_iter().find(|s| s.ticket == id && s.state.has_pane()).unwrap();
    assert_eq!(live.cwd, path.to_str().unwrap(), "and it stands in the rebuilt worktree");
}

/// Unmerged work is why the archive is reversible: nothing is touched.
#[test]
fn archive_keeps_an_unmerged_worktree() {
    let Some(h) = Harness::boot("arch-keep", Some(STUB)) else { return };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-keep");
    let (id, sid, branch, path) = ready(&mut c, "beta");
    quiet(&mut c, id, sid);
    assert!(matches!(c.request(Command::ArchiveTicket { id }), Response::Ok));
    // Several ticks: a teardown would have run by now.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(path.join("beta.txt").is_file(), "the tree stands");
    assert!(branch_exists(&repo, &branch), "the branch stands");
    let wt = wt_of(&mut c, id).expect("the binding stands");
    assert_eq!(wt.status, "attached");
    assert!(matches!(c.request(Command::UnarchiveTicket { id }), Response::Ok));
    assert_eq!(wt_of(&mut c, id).unwrap().status, "attached", "and a restore finds it as it was");
}

/// A PR squashed on the forge (T-267's verdict): archived, the directory
/// goes but `branch -d` refuses the branch, so the binding stays `Evicted`
/// — and a wake on the restored ticket rebuilds the tree on that branch
/// before the pane opens, with the record's cwd following it.
#[test]
fn archive_after_a_squash_keeps_the_branch_and_the_wake_rebuilds_the_tree() {
    let Some(h) = Harness::boot_with_env(
        "arch-squash",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-squash");
    let (id, sid, branch, path) = ready(&mut c, "gamma");
    git(&repo, &["merge", "-q", "--squash", &branch]);
    git(&repo, &["commit", "-qm", "gamma (#12)"]);
    wait_until(Duration::from_secs(20), "the board to read the squash", || {
        wt_of(&mut c, id).is_some_and(|w| w.merged)
    });
    quiet(&mut c, id, sid);

    assert!(matches!(c.request(Command::ArchiveTicket { id }), Response::Ok));
    wait_until(Duration::from_secs(20), "the archive to reclaim the directory", || {
        !path.exists() && wt_of(&mut c, id).is_some_and(|w| w.status == "evicted")
    });
    assert!(branch_exists(&repo, &branch), "git refused -d on the squashed branch: it stands");

    // Restore, wake: the parked resume rebuilds the tree on the kept branch.
    assert!(matches!(c.request(Command::UnarchiveTicket { id }), Response::Ok));
    assert!(matches!(
        c.request(Command::ResumeSession { id: sid, confirm: false }),
        Response::Provisioning
    ));
    let wt = wait_attached(&mut c, id);
    assert_eq!(wt.branch, branch);
    assert_eq!(wt.path.as_deref(), Some(path.to_str().unwrap()));
    wait_until(Duration::from_secs(15), "the parked wake to open a pane", || {
        c.board().sessions.iter().any(|s| s.id == sid && s.state.has_pane())
    });
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert_eq!(rec.cwd, path.to_str().unwrap(), "the record's cwd followed the rebuilt tree");
    assert!(path.join("gamma.txt").is_file(), "the branch's own work is checked out again");
}

/// The header's X offer archives down `a a`'s road (T-481): the landed tree
/// goes, the unmerged one stays, and each ticket gets its own feed row. The
/// slow bucket is pushed out of reach, so the sweep cannot stand in for it.
#[test]
fn the_offer_reclaims_like_a_single_archive() {
    let Some(h) = Harness::boot_with_env(
        "arch-offer",
        Some(STUB),
        &[("MESIMON_ARCHIVE_SUGGEST_MS", "1"), ("MESIMON_WT_REFRESH_TICKS", "1000000")],
    ) else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-offer");
    let (landed, sl, landed_branch, landed_path) = ready(&mut c, "delta");
    let (open, so, open_branch, open_path) = ready(&mut c, "epsilon");
    quiet(&mut c, landed, sl);
    quiet(&mut c, open, so);
    merge(&mut c, landed);

    // The offer prices TODO, where both tickets sit, and takes it whole.
    let (board, _) = snapshot_of(c.request(Command::Snapshot));
    let mut settings = board.column("TODO").unwrap().settings.clone();
    settings.offers = Some(ColumnOffers::Archive);
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: "TODO".into(), settings }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(6), "the offer to price both tickets", || {
        snapshot_of(c.request(Command::Snapshot)).1.archive_tickets == 2
    });
    match c.request(Command::ArchiveAll) {
        Response::Archived { archived, skipped } => assert_eq!((archived, skipped), (2, 0)),
        other => panic!("archive all: {other:?}"),
    }
    wait_until(Duration::from_secs(20), "the offer to reclaim the landed worktree", || {
        !landed_path.exists()
            && !branch_exists(&repo, &landed_branch)
            && wt_of(&mut c, landed).is_none()
    });
    assert!(open_path.join("epsilon.txt").is_file(), "unmerged work keeps its tree");
    assert!(branch_exists(&repo, &open_branch));
    assert_eq!(wt_of(&mut c, open).expect("its binding stands").status, "attached");
    wait_until(Duration::from_secs(5), "a feed row per archived ticket", || {
        feed_count(&h, "archive_all", landed) == 1
            && feed_count(&h, "archive_all", open) == 1
            && feed_count(&h, "worktree_torn_down", landed) == 1
    });
    assert_eq!(feed_count(&h, "worktree_torn_down", open), 0);
}

/// Work that lands after its ticket was archived — a PR merged later, a
/// `git merge` by hand — is a merge only the worktree sample can see; when
/// it lands, the daemon's sweep reclaims the tree the archive kept.
#[test]
fn the_sweep_reclaims_an_archived_tree_whose_work_landed_later() {
    let Some(h) =
        Harness::boot_with_env("arch-sweep", Some(STUB), &[("MESIMON_WT_REFRESH_TICKS", "4")])
    else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-sweep");
    let (id, sid, branch, path) = ready(&mut c, "eta");
    quiet(&mut c, id, sid);
    assert!(matches!(c.request(Command::ArchiveTicket { id }), Response::Ok));
    // Past a sample or two: an unmerged archived tree is left alone.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(path.join("eta.txt").is_file(), "unmerged: the archive keeps the tree");

    git(&repo, &["merge", "-q", "--no-ff", "-m", "eta", &branch]);
    wait_until(Duration::from_secs(20), "the sweep to reclaim the late-landed tree", || {
        !path.exists() && !branch_exists(&repo, &branch) && wt_of(&mut c, id).is_none()
    });
    wait_until(Duration::from_secs(5), "the sweep's teardown in the feed", || {
        feed_count(&h, "worktree_torn_down", id) == 1
    });
    assert!(c.board().ticket(id).unwrap().is_archived(), "archived, not deleted");
}

/// A tree the last daemon left standing — archived with its work unmerged,
/// then landed while no daemon ran — goes when the next one starts. The
/// slow bucket is pushed out of reach, so only the start's sweep can do it.
#[test]
fn a_restart_reclaims_a_tree_whose_archived_work_landed_meanwhile() {
    let Some(h) =
        Harness::boot_with_env("arch-boot", Some(STUB), &[("MESIMON_WT_REFRESH_TICKS", "1000000")])
    else {
        return;
    };
    let repo = h.repo.clone();
    init_repo(&repo, "a.txt", "hello\n");
    let mut c = h.client("arch-boot");
    let (id, sid, branch, path) = ready(&mut c, "zeta");
    quiet(&mut c, id, sid);
    assert!(matches!(c.request(Command::ArchiveTicket { id }), Response::Ok));
    std::thread::sleep(Duration::from_millis(1000));
    assert!(path.join("zeta.txt").is_file(), "unmerged: the archive keeps the tree");

    assert!(matches!(c.request(Command::Shutdown), Response::Ok));
    wait_until(Duration::from_secs(10), "the daemon to leave", || !h.paths.orch_sock().exists());
    git(&repo, &["merge", "-q", "--no-ff", "-m", "zeta", &branch]);
    h.restart();
    let mut c = h.client("arch-boot-2");
    wait_until(Duration::from_secs(20), "the start's sweep to reclaim the tree", || {
        !path.exists() && !branch_exists(&repo, &branch) && wt_of(&mut c, id).is_none()
    });
    wait_until(Duration::from_secs(5), "the teardown in the feed", || {
        feed_count(&h, "worktree_torn_down", id) == 1
    });
}
