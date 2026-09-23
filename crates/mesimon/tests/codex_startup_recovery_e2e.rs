//! Real daemon/private tmux, synthetic runtime only: positive preselection
//! evidence authorizes a human startup retry; missing evidence never does.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::*;
use mesimon_core::board::{AgentProvider, SessionKind, SessionRecord, SessionState};
use mesimon_core::command::{Command, Response};
use serde_json::Value;
use std::time::Duration;

const RUNTIME: &str = r#"#!/usr/bin/env python3
import json,os,sys,time
from pathlib import Path
config=json.loads(Path(sys.argv[sys.argv.index('--config')+1]).read_text())
root=Path(__file__).parent
with (root/'launches.jsonl').open('a') as log: log.write(json.dumps(config)+'\n')
phase=(root/'phase').read_text()
thread=(config.get('resume') or 'synthetic-exact') if phase in ('selected','ready') else None
path=Path(config['snapshot_path'])
sequence=1
while True:
    value={'session':config['session'],'generation':config['generation'],'sequence':sequence,
           'heartbeat_ms':time.time_ns()//1000000,'thread_id':thread,'turn_id':None,
           'state':{'state':'idle','stop_reason':'unknown'} if phase=='ready' else {'state':'spawning'},
           'observation_hold':phase!='ready','history_path':None,'stopped':False}
    if phase!='legacy': value['launch_phase']='selected' if phase=='ready' else phase
    temporary=path.with_suffix('.tmp');temporary.write_text(json.dumps(value));temporary.replace(path)
    if phase!='ready':
        time.sleep(0.2)
        os._exit(9)
    print('Synthetic native composer',flush=True)
    time.sleep(0.1)
"#;

fn record(c: &mut TestClient, id: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|session| session.id == id).unwrap()
}

fn exercise(phase: &str, expected_resume: Option<Option<&str>>) {
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("codex-startup-recovery");
    let repo = fixture.dir.join("repo");
    init_repo(&repo, "README.md", "Synthetic startup recovery.\n");
    let script = fixture.dir.join("runtime.py");
    std::fs::write(&script, RUNTIME).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    std::fs::write(fixture.dir.join("phase"), phase).unwrap();
    fixture.set_env("MESIMON_CODEX_RUNTIME_BIN", &script);
    fixture.set_env("MESIMON_CODEX_BIN", fixture.dir.join("must-not-run-native"));
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    let paths = fixture.paths(&repo);
    let _daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Codex }),
        Response::Ok
    ));
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "startup recovery".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        response => panic!("create: {response:?}"),
    };
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("spawn: {response:?}"),
    };
    wait_until(Duration::from_secs(10), "failed runtime keeps its reserved seat", || {
        record(&mut c, id).codex_stopping
    });
    let old_generation = record(&mut c, id).codex_generation;
    assert!(matches!(c.request(Command::WakeSession { id }), Response::Err { .. }));
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::ClaudeCode }),
        Response::Ok
    ));
    let first = c.request(Command::ResumeSession { id, confirm: true });
    let Some(expected_resume) = expected_resume else {
        for response in [first, c.request(Command::ResumeSession { id, confirm: true })] {
            match response {
                Response::Err { message } => {
                    assert!(message.contains("refusing a fresh conversation"), "{message}")
                }
                response => {
                    panic!("uncertain startup must not replace possible history: {response:?}")
                }
            }
        }
        // Not `codex_stopping` any more: the sweep may already have proved
        // the runtime gone and released the checkout (T-405). What the rule
        // is about survives either way — the record, its identity, and the
        // single launch behind it.
        assert_eq!(record(&mut c, id).codex_generation, old_generation);
        assert_eq!(
            std::fs::read_to_string(fixture.dir.join("launches.jsonl")).unwrap().lines().count(),
            1
        );
        return;
    };
    match first {
        Response::Err { message } => {
            assert!(message.contains("resume again to acknowledge"), "{message}");
            assert!(
                message.contains(if expected_resume.is_none() {
                    "retry startup"
                } else {
                    "exact conversation"
                }),
                "{message}"
            );
        }
        response => panic!("first gesture must show risk: {response:?}"),
    }
    std::fs::write(fixture.dir.join("phase"), "ready").unwrap();
    assert!(
        matches!(c.request(Command::ResumeSession { id, confirm:true }), Response::Spawned { fresh, .. } if fresh == expected_resume.is_none())
    );
    wait_until(Duration::from_secs(10), "replacement startup is observed", || {
        let observed = record(&mut c, id);
        observed.codex_thread_id.is_some() && matches!(observed.state, SessionState::Idle { .. })
    });
    let resumed = record(&mut c, id);
    assert_eq!(resumed.kind, SessionKind::Codex);
    assert_ne!(resumed.codex_generation, old_generation);
    assert!(!resumed.codex_stopping);
    let configs: Vec<Value> = std::fs::read_to_string(fixture.dir.join("launches.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[1]["resume"].as_str(), expected_resume);
    assert!(std::fs::read_to_string(paths.activity_log())
        .unwrap()
        .contains("unknown child processes may remain"));
}

/// T-405: a crashed runtime on a LIVE ticket must not own the checkout for
/// good. T-357 released only the records whose ticket a person had deleted,
/// and no gesture lowers `codex_stopping` otherwise — so every queued ask on
/// the board waited forever behind a card that showed nothing working. On the
/// same positive evidence the orphan sweep deletes an orphan for, the record
/// here merely loses its cleanup flags and stays on its ticket's rail.
#[test]
fn a_crashed_runtime_on_a_live_ticket_releases_the_checkout() {
    if !require_tmux() {
        return;
    }
    let fixture = TestFixture::new("codex-cleanup-release");
    let repo = fixture.dir.join("repo");
    init_repo(&repo, "README.md", "Synthetic cleanup release.\n");
    let script = fixture.dir.join("runtime.py");
    std::fs::write(&script, RUNTIME).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    std::fs::write(fixture.dir.join("phase"), "before_selection").unwrap();
    fixture.set_env("MESIMON_CODEX_RUNTIME_BIN", &script);
    fixture.set_env("MESIMON_CODEX_BIN", fixture.dir.join("must-not-run-native"));
    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    // The clock, not the rule: a live ticket's cleanup must STAND a while
    // before the sweep looks for its runtime, and an e2e cannot wait 60 s.
    fixture.set_env("MESIMON_CODEX_CLEANUP_STALE_MS", "0");
    let paths = fixture.paths(&repo);
    let _daemon = fixture.daemon(&repo);
    let mut c = TestClient::connect(&paths.orch_sock());
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Codex }),
        Response::Ok
    ));
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "cleanup release".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        response => panic!("create: {response:?}"),
    };
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("spawn: {response:?}"),
    };
    wait_until(Duration::from_secs(10), "the failed runtime claims the checkout", || {
        let rec = record(&mut c, id);
        rec.codex_stopping && mesimon_core::quiet::is_working(&rec)
    });
    // The ticket is still on the board — this is the case T-357 left out.
    assert!(c.board().ticket(ticket).is_some());
    wait_until(Duration::from_secs(60), "the sweep to release the checkout", || {
        !record(&mut c, id).codex_stopping
    });
    let released = record(&mut c, id);
    assert!(!mesimon_core::quiet::is_working(&released), "the checkout is free: {released:?}");
    assert!(!released.observation_hold, "the observation hold goes with it");
    assert_eq!(released.ticket, ticket, "the corpse stays on its ticket's rail");
    // The feed says which release it was: the record stayed, so this is not
    // the orphan's `CodexOrphanReleased`.
    wait_until(Duration::from_secs(10), "the release to reach the feed", || {
        std::fs::read_to_string(paths.activity_log())
            .unwrap_or_default()
            .contains("CodexCleanupReleased")
    });
}

#[test]
fn positively_preselected_startup_retries_only_after_human_confirmation() {
    exercise("before_selection", Some(None));
}
#[test]
fn pending_selection_without_identity_cannot_start_fresh() {
    exercise("selection_pending", None);
}
#[test]
fn legacy_missing_launch_phase_cannot_start_fresh() {
    exercise("legacy", None);
}
#[test]
fn saved_native_identity_resumes_exactly_even_if_daemon_missed_the_projection() {
    exercise("selected", Some(Some("synthetic-exact")));
}
