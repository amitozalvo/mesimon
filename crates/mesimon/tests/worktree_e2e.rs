//! M4a worktree e2e: real git repo → per-ticket worktree strategy → lazy
//! provisioning on spawn (stub claude records cwd) → clean merge → DONE gate
//! lifts → delete → grace → teardown removes worktree + branch. Second ticket:
//! conflicting branch → merge refuses → DONE gate blocks → discard delete.
//!
//! Same harness shape as m3_e2e: in-process daemon, real tmux, stub claude.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::process::Command as Proc;
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, SessionKind, WorkspaceStrategy};
use mesimon_core::command::{Command, DiffTarget, MergeOutcome, Response, WorktreeItem};

fn board_of(resp: Response) -> (Board, Vec<WorktreeItem>) {
    match resp {
        Response::Board { board, worktrees, .. } => (board, worktrees),
        other => panic!("expected board, got {other:?}"),
    }
}

/// Every pane on the private server: `(session name, cwd, dead)`.
fn list_panes(sock: &std::path::Path) -> Vec<(String, String, bool)> {
    let out = tmux(sock)
        .args(["list-panes", "-a", "-F", "#{session_name}|#{pane_current_path}|#{pane_dead}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '|');
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next()? == "1"))
        })
        .collect()
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let out = Proc::new("git").arg("-C").arg(repo).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn wait_wt_status(
    c: &mut TestClient,
    ticket: ulid::Ulid,
    want: &str,
    timeout: Duration,
) -> WorktreeItem {
    let deadline = Instant::now() + timeout;
    loop {
        let (_, wts) = board_of(c.request(Command::Snapshot));
        let item = wts.iter().find(|w| w.ticket == ticket).cloned();
        if let Some(w) = &item {
            if w.status == want {
                return w.clone();
            }
        }
        assert!(Instant::now() < deadline, "binding never reached {want}; last: {item:?}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[test]
fn m4_worktree_lifecycle() {
    if !common::require_tmux() {
        return;
    }
    if Proc::new("git").arg("--version").output().is_err() {
        eprintln!("git not installed; skipping");
        return;
    }
    let fixture = common::TestFixture::new("wt");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "e2e@t"]);
    git(&repo, &["config", "user.name", "e2e"]);
    std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let repo_canon = paths.repo_root.clone();

    // Stub claude: records its cwd, dies politely on TERM.
    let cwd_log = dir.join("claude-cwd.log");
    let stub = dir.join("claude-stub.sh");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\npwd >> \"{}\"\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n",
            cwd_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);

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
            client: "wt".into()
        }),
        Response::Hello { .. }
    ));

    // ---- ticket 1: worktree strategy, lazy provision on spawn -------------
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "Fix thing".into(),
        workspace: None,
    });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let t1 = board.tickets[0].id;
    assert!(matches!(
        c.request(Command::SetWorkspace { id: t1, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    // First spawn queues provisioning and parks the spawn.
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: t1,
            kind: SessionKind::Claude,
            submit_prompt: false
        }),
        Response::Provisioning
    ));
    let wt = wait_wt_status(&mut c, t1, "attached", Duration::from_secs(10));
    assert_eq!(wt.branch, "msmn/T-1-fix-thing");
    // A fresh branch is trivially an ancestor of main — that must read as
    // "no work yet", never "merged" (dogfood regression 2026-08-30).
    assert!(!wt.merged, "fresh branch must not read merged");
    // The parked spawn replayed: the session exists and its cwd is the worktree.
    let deadline = Instant::now() + Duration::from_secs(10);
    let sid = loop {
        let (b, _) = board_of(c.request(Command::Snapshot));
        if let Some(s) = b.sessions.iter().find(|s| s.ticket == t1) {
            break s.id;
        }
        assert!(Instant::now() < deadline, "parked spawn never replayed");
        std::thread::sleep(Duration::from_millis(150));
    };
    let (b, _) = board_of(c.request(Command::Snapshot));
    let rec = b.sessions.iter().find(|s| s.id == sid).unwrap();
    assert!(
        rec.cwd.contains("worktrees") && rec.cwd.ends_with("T-1-fix-thing"),
        "cwd should be the worktree: {}",
        rec.cwd
    );
    // The ticket's terminal (T-273) stands in the worktree too, as a named
    // tmux session that is no session of the ticket: the board lists it
    // nowhere, and the token goes back on `TerminalEnd`.
    let term1 = format!("msmn-term-{t1}");
    match c.request(Command::OpenTerminal { ticket: Some(t1) }) {
        Response::Attach { argv } => assert_eq!(argv.last().map(String::as_str), Some(&*term1)),
        other => panic!("expected the terminal's attach argv, got {other:?}"),
    }
    let panes = list_panes(&paths.tmux_sock());
    let term = panes.iter().find(|(name, _, _)| *name == term1).expect("the terminal's pane");
    assert_eq!(term.1, rec.cwd, "the terminal stands in the worktree");
    let (b, _) = board_of(c.request(Command::Snapshot));
    assert_eq!(b.sessions.len(), 1, "the terminal is no session: {:?}", b.sessions);
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    // Workspace is locked now.
    assert!(matches!(
        c.request(Command::SetWorkspace { id: t1, workspace: None }),
        Response::Err { .. }
    ));
    // Stub really started there (give the pane a moment to run the stub).
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cwd_log.exists() {
        assert!(Instant::now() < deadline, "stub never logged cwd");
        std::thread::sleep(Duration::from_millis(100));
    }

    // ---- work + merge (clean) --------------------------------------------
    let wt_path = std::path::PathBuf::from(
        String::from_utf8_lossy(&std::fs::read(&cwd_log).unwrap())
            .lines()
            .next()
            .unwrap()
            .to_string(),
    );
    std::fs::write(wt_path.join("b.txt"), "agent work\n").unwrap();
    git(&wt_path, &["add", "."]);
    git(&wt_path, &["commit", "-qm", "agent work"]);

    // ---- M4b: the diff wire, end to end (connection-thread serving) -------
    std::fs::write(wt_path.join("stray.txt"), "never added\n").unwrap();
    match c.request(Command::DiffList { target: DiffTarget::Ticket { id: t1 } }) {
        Response::DiffList { branch, files, worktree_present, branch_oid, .. } => {
            assert_eq!(branch, "msmn/T-1-fix-thing");
            assert!(worktree_present);
            assert_eq!(branch_oid.len(), 40);
            let b = files.iter().find(|f| f.path == "b.txt").expect("committed file listed");
            assert_eq!(b.status, "A");
            assert_eq!((b.adds, b.dels), (Some(1), Some(0)));
            // The un-added agent file is invisible to git diff — the status
            // call must surface it as a display-only row.
            let stray = files.iter().find(|f| f.path == "stray.txt").expect("untracked row");
            assert!(stray.untracked);
            assert_eq!(stray.status, "");
        }
        other => panic!("expected DiffList, got {other:?}"),
    }
    match c.request(Command::DiffFile {
        target: DiffTarget::Ticket { id: t1 },
        path: "b.txt".into(),
        context: 3,
    }) {
        Response::DiffFile { file } => {
            assert_eq!(file.render, mesimon_core::diff::Render::Text);
            assert!(
                file.hunks[0]
                    .lines
                    .iter()
                    .any(|l| l.sign == mesimon_core::diff::Sign::Add && l.text == "agent work"),
                "hunks: {:?}",
                file.hunks
            );
        }
        other => panic!("expected DiffFile, got {other:?}"),
    }
    // No binding → the mesimon-worded refusal, served off the writer thread.
    match c.request(Command::DiffList { target: DiffTarget::Ticket { id: ulid::Ulid(999) } }) {
        Response::Err { message } => assert!(message.contains("no worktree"), "{message}"),
        other => panic!("expected refusal, got {other:?}"),
    }
    std::fs::remove_file(wt_path.join("stray.txt")).unwrap();

    // Merge refused while the (stub) session is Running.
    match c.request(Command::MergeTicket { id: t1 }) {
        Response::Merge { outcome, .. } => assert_eq!(outcome, MergeOutcome::Refused),
        other => panic!("expected merge refusal, got {other:?}"),
    }
    // DONE gate blocks while unmerged.
    match c.request(Command::MoveTicket { id: t1, column: "DONE".into(), before: None }) {
        Response::Err { message } => assert!(message.contains("unmerged"), "{message}"),
        other => panic!("expected DONE gate, got {other:?}"),
    }
    // Kill the session (quiet ticket), then merge cleanly.
    let _ = c.request(Command::KillSession { id: sid });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match c.request(Command::MergeTicket { id: t1 }) {
            Response::Merge { outcome: MergeOutcome::Merged, .. } => break,
            Response::Merge { outcome: MergeOutcome::Refused, detail } => {
                // Session may still be winding down through the reaper.
                assert!(Instant::now() < deadline, "merge never went through: {detail}");
                std::thread::sleep(Duration::from_millis(300));
            }
            other => panic!("unexpected merge response: {other:?}"),
        }
    }
    assert!(repo_canon.join("b.txt").is_file(), "merge must land in main checkout");
    // DONE gate lifts.
    assert!(matches!(
        c.request(Command::MoveTicket { id: t1, column: "DONE".into(), before: None }),
        Response::Ok
    ));
    // Delete (merged — no discard needed) → grace → teardown.
    assert!(matches!(
        c.request(Command::DeleteTicket { id: t1, discard_worktree: false }),
        Response::Ok
    ));
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let gone_dir = !wt_path.exists();
        let gone_branch =
            !git(&repo, &["branch", "--list", "msmn/T-1-fix-thing"]).contains("T-1-fix-thing");
        if gone_dir && gone_branch {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "teardown incomplete: dir gone={gone_dir} branch gone={gone_branch}"
        );
        std::thread::sleep(Duration::from_millis(300));
    }

    // ---- ticket 2: conflict path + discard delete -------------------------
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "clash".into(),
        workspace: None,
    });
    let (board, _) = board_of(c.request(Command::Snapshot));
    let t2 = board.tickets.iter().find(|t| t.title == "clash").unwrap().id;
    let _ =
        c.request(Command::SetWorkspace { id: t2, workspace: Some(WorkspaceStrategy::Worktree) });
    let _ = c.request(Command::SpawnSession {
        ticket: t2,
        kind: SessionKind::Bash,
        submit_prompt: false,
    });
    let wt2 = wait_wt_status(&mut c, t2, "attached", Duration::from_secs(10));
    assert_eq!(wt2.branch, "msmn/T-2-clash");
    // Diverge the same file on both sides.
    std::fs::write(repo_canon.join("a.txt"), "main side\n").unwrap();
    git(&repo, &["commit", "-aqm", "main change"]);
    let wt2_path = {
        let out = git(&repo, &["worktree", "list", "--porcelain"]);
        out.lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .find(|p| p.ends_with("T-2-clash"))
            .unwrap()
            .to_string()
    };
    let wt2_path = std::path::PathBuf::from(wt2_path);
    std::fs::write(wt2_path.join("a.txt"), "branch side\n").unwrap();
    git(&wt2_path, &["commit", "-aqm", "branch change"]);
    // Kill the bash session so the ticket is quiet, then merge → NeedsRebase
    // (main moved past the branch — ff-only policy sends the agent to rebase;
    // mesimon never mints merge commits).
    let (b, _) = board_of(c.request(Command::Snapshot));
    let sid2 = b.sessions.iter().find(|s| s.ticket == t2).unwrap().id;
    let _ = c.request(Command::KillSession { id: sid2 });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match c.request(Command::MergeTicket { id: t2 }) {
            Response::Merge { outcome: MergeOutcome::NeedsRebase, .. } => break,
            Response::Merge { outcome: MergeOutcome::Refused, .. } => {
                assert!(Instant::now() < deadline, "needs-rebase verdict never arrived");
                std::thread::sleep(Duration::from_millis(300));
            }
            other => panic!("unexpected merge response: {other:?}"),
        }
    }
    // Delete without discard refuses; discard delete proceeds to teardown.
    match c.request(Command::DeleteTicket { id: t2, discard_worktree: false }) {
        Response::Err { message } => assert!(message.contains("unmerged"), "{message}"),
        other => panic!("expected delete gate, got {other:?}"),
    }
    // A terminal standing in the worktree (T-273) does not hold the teardown
    // up and does not survive it: killed before the directory goes.
    let term2 = format!("msmn-term-{t2}");
    assert!(matches!(
        c.request(Command::OpenTerminal { ticket: Some(t2) }),
        Response::Attach { .. }
    ));
    assert!(list_panes(&paths.tmux_sock()).iter().any(|(name, _, _)| *name == term2));
    assert!(matches!(c.request(Command::TerminalEnd), Response::Ok));
    assert!(matches!(
        c.request(Command::DeleteTicket { id: t2, discard_worktree: true }),
        Response::Ok
    ));
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let gone_dir = !wt2_path.exists();
        let gone_branch =
            !git(&repo, &["branch", "--list", "msmn/T-2-clash"]).contains("T-2-clash");
        if gone_dir && gone_branch {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "discard teardown incomplete: dir gone={gone_dir} branch gone={gone_branch}"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(
        !list_panes(&paths.tmux_sock()).iter().any(|(name, _, _)| *name == term2),
        "the worktree's terminal went with the worktree"
    );

    // ---- cleanup ----------------------------------------------------------
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
}
