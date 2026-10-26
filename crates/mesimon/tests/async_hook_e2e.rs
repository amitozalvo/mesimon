//! Turn-end children hold delivery, parking and the merge train (T-1044).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use mesimon_core::board::{CrownMode, SessionState, StopReason};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const STUB: &str = r#"#!/usr/bin/env python3
import os,subprocess,time
from pathlib import Path
root=Path(__file__).parent
key=os.environ['MESIMON_TICKET']
# Terminal infrastructure shares the pane group, even if born after a prompt.
subprocess.Popen(['python3','-c','import time; time.sleep(120)'])
print('❯',flush=True)
(root/('ready-'+key)).touch()
child=None
while True:
    if child is None and (root/('go-'+key)).exists():
        child=subprocess.Popen(['python3','-c',
            'import time,sys; from pathlib import Path; p=Path(sys.argv[1]); '\
            'exec("while not p.exists(): time.sleep(.05)")',str(root/('release-'+key))], start_new_session=True)
        (root/('child-'+key)).write_text(str(child.pid))
    if child is not None: child.poll()
    time.sleep(.05)
"#;

struct Case {
    h: Harness,
    c: TestClient,
    worker: ulid::Ulid,
    sid: uuid::Uuid,
    key: String,
    branch: String,
    dir: PathBuf,
}
impl Case {
    fn boot(name: &str, cap: &str) -> Option<Self> {
        let h = Harness::boot_with_env(
            name,
            Some(STUB),
            &[
                ("MESIMON_ASYNC_HOOK_CAP_MS", cap),
                ("MESIMON_WT_REFRESH_TICKS", "4"),
                ("MESIMON_PANE_QUIET_MS", "600000"),
            ],
        )?;
        init_repo(&h.repo, "base.txt", "base\n");
        let mut c = h.client(name);
        let crown = c.create_ticket("coordinate");
        let cs = c.spawn_claude(crown);
        assert!(matches!(c.request(Command::CrownTicket { id: crown }), Response::Ok));
        assert!(matches!(
            c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
            Response::Ok
        ));
        let dir = h.stub.as_ref().unwrap().parent().unwrap().to_path_buf();
        wait_until(Duration::from_secs(15), "crown stub", || dir.join("ready-T-1").exists());
        hook_send(&h.paths.hook_sock(), &cs.to_string(), "UserPromptSubmit", "{}");
        hook_send(&h.paths.hook_sock(), &cs.to_string(), "Stop", "{}");
        c.await_state(cs, "idle crown", |s| {
            matches!(s, SessionState::Idle { stop_reason: StopReason::EndTurn })
        });
        let worker = c.create_ticket("review my commit");
        let key = c.board().ticket(worker).unwrap().short_key.clone();
        let seen = match c.send(
            Principal::Agent { session: cs },
            Command::AgentReadTicket { key: key.clone(), full: false, plan: false },
        ) {
            Response::AgentTicket { ticket } => ticket.seen,
            other => panic!("{other:?}"),
        };
        assert!(matches!(
            c.send(
                Principal::Agent { session: cs },
                Command::AgentStartTicket {
                    key: key.clone(),
                    seen,
                    plan: false,
                    tier: None,
                    workspace: Some("worktree".into()),
                    expect_secs: None,
                }
            ),
            Response::AgentStarted { .. }
        ));
        let wt = wait_attached(&mut c, worker);
        wait_until(Duration::from_secs(15), "worker", || c.board().live_agent(worker).is_some());
        let sid = c.board().live_agent(worker).unwrap().id;
        wait_until(Duration::from_secs(15), "worker stub", || {
            dir.join(format!("ready-{key}")).exists()
        });
        let path = PathBuf::from(wt.path.unwrap());
        std::fs::write(path.join("work.txt"), "work\n").unwrap();
        git(&path, &["add", "."]);
        git(&path, &["commit", "-qm", "work"]);
        // Any launch-time infrastructure is older than the turn, even at
        // ps's whole-second precision (the mod parity road has a bridge).
        std::thread::sleep(Duration::from_secs(2));
        let mut case = Self { h, c, worker, sid, key, branch: wt.branch, dir };
        case.send("UserPromptSubmit");
        case.c.await_state(sid, "working", |s| *s == SessionState::Running);
        Some(case)
    }
    fn send(&self, event: &str) {
        hook_send(
            &self.h.paths.hook_sock(),
            &self.sid.to_string(),
            event,
            r#"{"background_tasks":[],"stop_hook_active":false}"#,
        );
    }
    fn feed(&self) -> String {
        std::fs::read_to_string(self.h.paths.activity_log()).unwrap_or_default()
    }
    fn child(&self) {
        std::fs::write(self.dir.join(format!("go-{}", self.key)), "").unwrap();
        wait_until(Duration::from_secs(10), "review child", || {
            self.dir.join(format!("child-{}", self.key)).exists()
        });
    }
    fn release(&self) {
        std::fs::write(self.dir.join(format!("release-{}", self.key)), "").unwrap();
    }
    fn merged(&self) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&self.h.repo)
            .args(["merge-base", "--is-ancestor", &self.branch, "main"])
            .status()
            .unwrap()
            .success()
    }
    fn wakes(&self) -> usize {
        self.feed()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|r| {
                r["kind"] == "crown_wake" && r.to_string().contains(&self.worker.to_string())
            })
            .count()
    }
    fn held(&mut self) {
        wait_until(Duration::from_secs(8), "async child to hold Stop", || {
            self.feed().contains("async_hook_wait")
        });
        assert_eq!(self.c.board().live_agent(self.worker).unwrap().state, SessionState::Running);
        assert_eq!(self.wakes(), 0, "delivery was announced before the child ended");
        match self.c.request(Command::SleepSession { id: self.sid }) {
            Response::Err { message } => assert!(message.contains("child processes"), "{message}"),
            other => panic!("park killed the pending review: {other:?}"),
        }
    }
}

#[test]
fn a_clean_child_exit_delivers_after_the_grace_and_allows_a_park() {
    let Some(mut t) = Case::boot("async-clean", "600000") else { return };
    t.child();
    t.send("Stop");
    t.held();
    let released = Instant::now();
    t.release();
    t.c.await_state(t.sid, "clean delivery", |s| {
        *s == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
    assert!(released.elapsed() >= Duration::from_secs(3), "the rewake grace was skipped");
    assert!(released.elapsed() < Duration::from_secs(12), "a clean exit waited for the cap");
    wait_until(Duration::from_secs(10), "crown delivery after child exit", || t.wakes() == 1);
    assert!(matches!(t.c.request(Command::SleepSession { id: t.sid }), Response::Ok));
    assert!(!t.feed().contains("async_hook_cap"));
}

#[test]
fn a_findings_turn_reaches_the_board_before_the_train_or_the_crown() {
    let Some(mut t) = Case::boot("async-train", "600000") else { return };
    assert!(matches!(
        t.c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    t.child();
    t.send("Stop");
    t.held();
    std::thread::sleep(Duration::from_secs(3));
    assert!(!t.merged(), "train merged behind a review");
    t.release();
    // The hook exits first; its findings enter later, inside the grace.
    std::thread::sleep(Duration::from_millis(700));
    t.send("UserPromptSubmit");
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(t.c.board().live_agent(t.worker).unwrap().state, SessionState::Running);
    assert!(!t.merged());
    assert_eq!(t.wakes(), 0);
    t.send("Stop");
    wait_until(Duration::from_secs(20), "train after findings turn", || t.merged());
    wait_until(Duration::from_secs(15), "crown after landing", || t.wakes() > 0);
}

#[test]
fn a_never_ending_child_releases_at_the_named_cap_and_records_it() {
    let Some(mut t) = Case::boot("async-cap", "4000") else { return };
    t.child();
    t.send("Stop");
    t.held();
    t.c.await_state(t.sid, "cap release", |s| {
        *s == SessionState::Idle { stop_reason: StopReason::EndTurn }
    });
    assert!(t.feed().contains("async_hook_cap"));
    assert!(t.feed().contains("4000 ms elapsed"));
    t.release();
}
