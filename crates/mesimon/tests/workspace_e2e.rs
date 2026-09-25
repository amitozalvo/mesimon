//! T-225 e2e: a board on a WORKSPACE — a root with repositories nested one
//! level under it — on the wire. A meta repo tracking one file over two child
//! repos → the snapshot's `git.repos` names the children and `changed` is the
//! sum across all three → `DiffList { Checkout }` is one list, the children's
//! rows prefixed by their name, and `DiffFile` opens through the prefix. And
//! (T-368) a worktree ticket there gets one worktree per repo under one
//! container, merged leg by leg. And (T-455) each child's push / pull lists ride
//! the snapshot, a listed commit opens in the repo it was listed under, and a
//! `GitFetch` press fetches each child from its own remote.
//!
//! The harness boots on a bare directory and the boot sample sees no repo;
//! the next sample is the 10 s bucket's, so the census is waited for (a
//! `GitFetch` press would re-sample at once, but it refuses without an
//! upstream, and a workspace's meta repo has none). The bucket samples only
//! while a board is attached (T-251), so the client subscribes first.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::PathBuf;
use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, DiffTarget, MergeOutcome, Response};

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
    assert!(matches!(c.request(Command::Subscribe), Response::Ok));

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

    // ---- each repo's push / pull lists (T-455) -----------------------------
    // `api` tracks a local branch and is one ahead; `web` has a `gitlab/main`
    // nobody linked and is one ahead of it.
    let api = root.join("api");
    git(&api, &["branch", "upstream"]);
    git(&api, &["branch", "-q", "--set-upstream-to=upstream", "main"]);
    git(&api, &["commit", "-qam", "api outgoing"]);
    let web = root.join("web");
    git(&web, &["update-ref", "refs/remotes/gitlab/main", "HEAD"]);
    git(&web, &["commit", "-qam", "web outgoing"]);
    let deadline = Instant::now() + Duration::from_secs(25);
    let g = loop {
        let g = git_of(c.request(Command::Snapshot));
        if g.nested.iter().map(|s| s.ahead).sum::<u32>() == 2 {
            break g;
        }
        assert!(Instant::now() < deadline, "never saw api and web one ahead each; last: {g:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let (api_sync, web_sync) = (&g.nested[0], &g.nested[1]);
    assert_eq!((api_sync.name.as_str(), api_sync.upstream.as_deref()), ("api", Some("upstream")));
    assert_eq!((web_sync.upstream.as_deref(), web_sync.by_name), (Some("gitlab/main"), true));
    let pushed = &api_sync.to_push.as_ref().unwrap()[0];
    assert_eq!(pushed.subject, "api outgoing");

    // A listed commit opens in the repo it was listed under, and only there.
    let open = |c: &mut TestClient, repo: Option<&str>| {
        let target = DiffTarget::Commit { oid: pushed.oid.clone(), repo: repo.map(Into::into) };
        c.request(Command::DiffList { target })
    };
    let resp = open(&mut c, Some("api"));
    let paths: Vec<String> = files_of(&resp).into_iter().map(|f| f.path).collect();
    assert_eq!(paths, ["server.ts"]);
    for repo in [None, Some("web"), Some("../api"), Some("nope")] {
        let resp = open(&mut c, repo);
        assert!(matches!(resp, Response::Err { .. }), "{repo:?}: {resp:?}");
    }

    // ---- a fetch press reaches every repo (T-455) ---------------------------
    // `web` gets a real remote under its `gitlab` name, and somebody else
    // pushes to it; `api` is re-linked to a remote that is not there. The
    // meta has no upstream at all, and the press is still accepted.
    let remote = h.dir.join("web-remote.git");
    git(&h.dir, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(&web, &["remote", "add", "gitlab", remote.to_str().unwrap()]);
    git(&web, &["push", "-q", "gitlab", "main"]);
    let other = h.dir.join("web-other");
    git(&h.dir, &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    git(&other, &["config", "user.email", "e2e@t"]);
    git(&other, &["config", "user.name", "e2e"]);
    git(&other, &["commit", "-q", "--allow-empty", "-m", "web incoming"]);
    git(&other, &["push", "-q", "origin", "main"]);
    git(&api, &["remote", "add", "origin", "/nonexistent/msmn-e2e-remote.git"]);
    git(&api, &["config", "branch.main.remote", "origin"]);
    git(&api, &["config", "branch.main.merge", "refs/heads/main"]);
    assert!(matches!(c.request(Command::GitFetch), Response::Ok));
    let deadline = Instant::now() + Duration::from_secs(25);
    let g = loop {
        let g = git_of(c.request(Command::Snapshot));
        let web = g.nested.iter().find(|s| s.name == "web").unwrap();
        let api = g.nested.iter().find(|s| s.name == "api").unwrap();
        if web.behind == 1 && !web.fetching && api.fetch_error.is_some() {
            break g;
        }
        assert!(Instant::now() < deadline, "never saw web fetched and api refused; last: {g:?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    let web_sync = g.nested.iter().find(|s| s.name == "web").unwrap();
    assert_eq!(web_sync.to_pull.as_ref().unwrap()[0].subject, "web incoming");
    assert!(web_sync.fetched_at_ms > 0 && web_sync.fetch_error.is_none(), "{web_sync:?}");
    assert!(!web.join(".git/FETCH_HEAD").exists(), "mesimon's fetch writes no FETCH_HEAD");
    let api_sync = g.nested.iter().find(|s| s.name == "api").unwrap();
    assert_eq!(api_sync.fetched_at_ms, 0, "a failed fetch is not a fetch: {api_sync:?}");
    assert!(g.fetch_error.is_none(), "the meta was never fetched: {g:?}");
}

/// Stub claude: records its cwd beside itself, dies politely on TERM. The
/// harness writes it to `<dir>/claude-stub.sh`, so `$0` is that path.
const STUB: &str = "#!/bin/sh\npwd >> \"$(dirname \"$0\")/claude-cwd.log\"\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n";

/// T-368: a worktree ticket on a workspace cuts one worktree per repo — the
/// meta's own as the container, every nested repo's inside it, all on the
/// ticket's branch, based on the branch each checkout stands on — and the
/// agent starts in the container. Work in one repo is that repo's `ahead`;
/// `m` fast-forwards leg by leg and the DONE gate lifts when every touched
/// leg has landed; a base that moved is named for the rebase; a discard
/// delete takes every leg, every branch and the container.
#[test]
fn a_workspace_ticket_gets_a_worktree_per_repo() {
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let Some(h) = Harness::boot_with_env(
        "workspace-wt",
        Some(STUB),
        &[("MESIMON_WT_REFRESH_TICKS", "4"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    let root = h.repo.clone();
    init_repo(&root, "CLAUDE.md", "# workspace\n");
    std::fs::write(root.join(".gitignore"), "*/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore children"]);
    init_repo(&root.join("api"), "server.ts", "one\n");
    init_repo(&root.join("web"), "page.tsx", "hello\n");
    let mut c = h.client("workspace-wt");
    assert!(matches!(c.request(Command::Subscribe), Response::Ok));

    // ---- the spawn is accepted and parks on provisioning ----------------
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "Fix thing".into(),
        workspace: None,
        tier: None,
    });
    let id = c.board().tickets[0].id;
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
    let wt = wait_attached(&mut c, id);
    assert_eq!(wt.branch, "msmn/T-1-fix-thing");
    let names: Vec<&str> = wt.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["", "api", "web"], "the root leg first, then census order");
    assert!(wt.repos.iter().all(|r| r.base == "main"), "{:?}", wt.repos);
    assert!(!wt.merged && wt.ahead == 0, "{wt:?}");
    let container = PathBuf::from(wt.path.as_deref().expect("attached: the container's path"));
    assert_eq!(
        container.canonicalize().unwrap(),
        h.paths.worktrees_root().join("T-1-fix-thing").canonicalize().unwrap()
    );
    let leg_repo = |leg: &str| if leg.is_empty() { root.clone() } else { root.join(leg) };
    for leg in ["", "api", "web"] {
        let repo = leg_repo(leg);
        let tree = container.join(leg);
        assert!(
            git(&repo, &["worktree", "list", "--porcelain"]).contains("T-1-fix-thing"),
            "{leg:?} lists no worktree"
        );
        assert!(git(&repo, &["branch", "--list", "msmn/T-1-fix-thing"]).contains("fix-thing"));
        assert_eq!(
            git(&tree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "msmn/T-1-fix-thing",
            "{leg:?}"
        );
    }
    assert!(container.join("CLAUDE.md").is_file() && container.join("api/server.ts").is_file());
    // The agent stands in the container.
    let cwd_log = h.dir.join("claude-cwd.log");
    wait_until(Duration::from_secs(10), "the stub's pwd", || {
        std::fs::read_to_string(&cwd_log).is_ok_and(|l| l.lines().any(|x| !x.is_empty()))
    });
    let pwd = std::fs::read_to_string(&cwd_log).unwrap().lines().next().unwrap().to_string();
    assert_eq!(
        std::fs::canonicalize(&pwd).unwrap(),
        std::fs::canonicalize(&container).unwrap(),
        "the session's cwd is the container"
    );
    // The journal measured it.
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap();
    assert!(
        journal.lines().any(|l| l.contains("provisioned T-1: 3 repos in ") && l.ends_with(" ms")),
        "{journal}"
    );

    // ---- work in api only: api is ahead, web is silent ------------------
    std::fs::write(container.join("api/feature.ts"), "agent work\n").unwrap();
    git(&container.join("api"), &["add", "."]);
    git(&container.join("api"), &["commit", "-qm", "api work"]);
    wait_until(Duration::from_secs(15), "api ahead 1", || {
        wt_of(&mut c, id).is_some_and(|w| {
            w.ahead == 1
                && w.repos.iter().any(|r| r.name == "api" && r.ahead == 1 && !r.merged)
                && w.repos.iter().any(|r| r.name == "web" && r.ahead == 0)
        })
    });
    // The agent's own view says the same, per repo.
    let sid = c.board().sessions.iter().find(|s| s.ticket == id).unwrap().id;
    match c.send(mesimon_core::Principal::Agent { session: sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket } => {
            assert_eq!(ticket.merge_state.as_deref(), Some("ahead"));
            let api = ticket.repos.iter().find(|r| r.name == "api").expect("api leg");
            assert_eq!((api.base.as_str(), api.merge_state.as_str()), ("main", "ahead"));
            let web = ticket.repos.iter().find(|r| r.name == "web").expect("web leg");
            assert_eq!(web.merge_state, "clean");
        }
        other => panic!("expected the agent's ticket view, got {other:?}"),
    }

    // ---- merge: api's main moves, web is untouched, DONE lifts ----------
    match c.request(Command::MoveTicket { id, column: "DONE".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("unmerged"), "{message}"),
        other => panic!("expected the DONE gate, got {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: sid });
    let deadline = Instant::now() + Duration::from_secs(15);
    let detail = loop {
        match c.request(Command::MergeTicket { id }) {
            Response::Merge { outcome: MergeOutcome::Merged, detail } => break detail,
            Response::Merge { outcome: MergeOutcome::Refused, detail } => {
                assert!(Instant::now() < deadline, "merge never accepted: {detail}");
                std::thread::sleep(Duration::from_millis(300));
            }
            other => panic!("expected Merged, got {other:?}"),
        }
    };
    assert!(detail.contains("api/main"), "the landed legs are named: {detail}");
    assert!(root.join("api/feature.ts").is_file(), "api's main moved");
    assert_eq!(
        git(&root.join("api"), &["rev-parse", "main"]),
        git(&container.join("api"), &["rev-parse", "HEAD"])
    );
    assert_eq!(
        git(&root.join("web"), &["rev-parse", "main"]),
        git(&root.join("web"), &["rev-parse", "msmn/T-1-fix-thing"]),
        "web was never touched"
    );
    assert!(matches!(
        c.request(Command::MoveTicket { id, column: "DONE".into(), before: None }),
        Response::Ok
    ));

    // ---- web's main moves and web's leg diverges: the rebase names web --
    std::fs::write(root.join("web/page.tsx"), "main moved\n").unwrap();
    git(&root.join("web"), &["commit", "-aqm", "main moved"]);
    std::fs::write(container.join("web/feature.tsx"), "web work\n").unwrap();
    git(&container.join("web"), &["add", "."]);
    git(&container.join("web"), &["commit", "-qm", "web work"]);
    match c.request(Command::MergeTicket { id }) {
        Response::Merge { outcome: MergeOutcome::NeedsRebase, detail } => {
            assert!(detail.contains("web"), "{detail}");
            assert!(!detail.contains("api"), "api has landed and is not named: {detail}");
        }
        other => panic!("expected NeedsRebase, got {other:?}"),
    }
    wait_until(Duration::from_secs(15), "web needs a rebase on the wire", || {
        wt_of(&mut c, id).is_some_and(|w| {
            w.needs_rebase
                && w.repos.iter().any(|r| r.name == "web" && r.needs_rebase && r.ahead == 1)
                && w.repos.iter().any(|r| r.name == "api" && r.merged)
        })
    });

    // ---- a discard delete takes every leg, every branch, the container --
    match c.request(Command::DeleteTicket { id, discard_worktree: false }) {
        Response::Err { message } => assert!(message.contains("unmerged"), "{message}"),
        other => panic!("expected the delete gate, got {other:?}"),
    }
    assert!(matches!(
        c.request(Command::DeleteTicket { id, discard_worktree: true }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(25), "teardown of every leg", || {
        !container.exists()
            && ["", "api", "web"].iter().all(|leg| {
                let repo = leg_repo(leg);
                !git(&repo, &["branch", "--list", "msmn/T-1-fix-thing"]).contains("fix-thing")
                    && !git(&repo, &["worktree", "list", "--porcelain"]).contains("T-1-fix-thing")
            })
    });
}
