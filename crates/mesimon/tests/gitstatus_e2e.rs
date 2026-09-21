//! T-124 e2e: the board's own checkout on the wire. A bare `origin`, a clone
//! as the repo → the snapshot's `git` names the branch and its upstream, in
//! sync → a commit on the clone reads as `ahead` → a push from a SECOND clone
//! is invisible until `Command::GitFetch` (no fetch by default) → `behind`
//! → a dirty file reads as `changed`.
//!
//! Same harness shape as worktree_e2e: the repo is made BEFORE the daemon
//! boots (a boot-time sample of an empty dir is honestly "not a repo").

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::command::{Command, RepoGit, Response};

/// Poll the snapshot until `want` holds of its git state; the sample runs on
/// the 10 s bucket, and a fetch press re-samples when it lands.
fn wait_git(c: &mut TestClient, what: &str, mut want: impl FnMut(&RepoGit) -> bool) -> RepoGit {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let g = git_of(c.request(Command::Snapshot));
        if want(&g) {
            return g;
        }
        assert!(Instant::now() < deadline, "never saw {what}; last: {g:?}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn the_checkout_stands_on_the_wire() {
    if !common::require_tmux() {
        return;
    }
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let fixture = common::TestFixture::new("git");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let origin = dir.join("origin.git");
    let repo = dir.join("repo");
    let other = dir.join("other");
    git(&dir, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    let clone = |into: &std::path::Path| {
        git(
            &dir,
            &[
                "-c",
                "protocol.file.allow=always",
                "clone",
                "-q",
                origin.to_str().unwrap(),
                into.to_str().unwrap(),
            ],
        );
        git(into, &["config", "user.email", "e2e@t"]);
        git(into, &["config", "user.name", "e2e"]);
    };
    clone(&repo);
    std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    clone(&other);

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", "/bin/true");
    // A 1 s bucket, so the headless clause below is asserted in seconds.
    fixture.set_env("MESIMON_WT_REFRESH_TICKS", "4");

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
            client: "git".into()
        }),
        Response::Hello { .. }
    ));

    // ---- boot: sampled, on main, tracking origin/main, in sync -------------
    let g = wait_git(&mut c, "the boot sample", |g| g.sampled);
    assert_eq!(g.branch, "main");
    assert!(!g.detached);
    assert_eq!(g.upstream.as_deref(), Some("origin/main"));
    assert_eq!((g.ahead, g.behind, g.changed), (0, 0, 0), "{g:?}");
    assert_eq!(g.fetch_every_secs, 0, "the periodic fetch is opt-in");
    assert!(!g.fetching);
    assert_eq!(g.fetched_at_ms, 0);
    assert_eq!(g.to_push, Some(vec![]));
    assert_eq!(g.to_pull, Some(vec![]));

    // ---- a commit here, with no board attached: unseen (T-251) -------------
    // Nobody subscribed and no train is armed, so the bucket forks nothing:
    // a headless daemon must not run `git status` per repo forever for a
    // header nobody has open. Two buckets pass and the boot sample stands.
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "two"]);
    std::thread::sleep(Duration::from_millis(2500));
    let g = git_of(c.request(Command::Snapshot));
    assert_eq!(g.ahead, 0, "a headless daemon sampled the checkout: {g:?}");

    // ---- a board attaches: the sample catches up, then keeps its bucket ----
    // A second, subscribed connection stands in for the board; its subscribe
    // fires one sample at once, and the tick samples for as long as it stays.
    let mut watcher = TestClient::connect(&sock);
    assert!(matches!(
        watcher.request(Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: "watch".into()
        }),
        Response::Hello { .. }
    ));
    assert!(matches!(watcher.request(Command::Subscribe), Response::Ok));
    let g = wait_git(&mut c, "ahead 1 once a board is attached", |g| g.ahead == 1);
    assert!(!g.fetching);
    assert_eq!(g.fetched_at_ms, 0, "no fetch happens on its own: {g:?}");

    // ---- a fetch press: push due, and the remote answered -------------------
    // `ahead == 1` is already the tick's doing; the fetch is what stamps
    // `fetched_at_ms`, so wait for the sample the FETCH produced.
    assert!(matches!(c.request(Command::GitFetch), Response::Ok));
    let g = wait_git(&mut c, "the fetch to land", |g| g.fetched_at_ms > 0 && !g.fetching);
    assert_eq!(g.ahead, 1, "{g:?}");
    assert_eq!(g.behind, 0);
    let outgoing = g.to_push.as_ref().unwrap();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].subject, "two");
    assert_eq!(outgoing[0].oid.len(), 40);
    assert_eq!(g.to_pull, Some(vec![]));
    assert!(g.fetched_at_ms > 0, "the file remote answered: {g:?}");
    assert!(g.fetch_error.is_none(), "{g:?}");

    // ---- a push from elsewhere: invisible until a fetch --------------------
    std::fs::write(other.join("c.txt"), "three\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-qm", "three"]);
    git(&other, &["push", "-q", "origin", "main"]);
    // Without a fetch the tracking ref has not moved: still ahead 1, behind 0.
    std::thread::sleep(Duration::from_millis(300));
    let g = git_of(c.request(Command::Snapshot));
    assert_eq!((g.ahead, g.behind), (1, 0), "no fetch happens on its own: {g:?}");
    assert!(matches!(c.request(Command::GitFetch), Response::Ok));
    let g = wait_git(&mut c, "behind 1 after the fetch", |g| g.behind == 1);
    assert_eq!(g.ahead, 1, "{g:?}");
    assert_eq!(g.to_push.as_ref().unwrap()[0].subject, "two");
    assert_eq!(g.to_pull.as_ref().unwrap()[0].subject, "three");
    assert!(g.fetch_error.is_none(), "{g:?}");
    // The fetch wrote the tracking ref and nothing else: no FETCH_HEAD.
    assert!(!repo.join(".git/FETCH_HEAD").exists(), "FETCH_HEAD must not be written");

    // ---- a dirty tree: changed --------------------------------------------
    std::fs::write(repo.join("a.txt"), "hello again\n").unwrap();
    std::fs::write(repo.join("untracked.txt"), "x\n").unwrap();
    assert!(matches!(c.request(Command::GitFetch), Response::Ok));
    let g = wait_git(&mut c, "changed 2", |g| g.changed == 2);
    assert_eq!((g.ahead, g.behind), (1, 1), "{g:?}");

    // ---- cleanup ----------------------------------------------------------
    drop(watcher);
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
