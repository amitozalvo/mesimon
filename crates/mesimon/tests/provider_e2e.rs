//! Provider selection, ownership and observation safety through the real daemon.
//! The fixture runtime publishes explicitly synthetic observations. Native
//! compatibility is checked separately by ci/codex-state-e2e.py.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::PathBuf;
use std::time::Duration;

use mesimon_core::board::{
    AgentProvider, SessionKind, SessionRecord, SessionState, StopReason, WorkspaceStrategy,
};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::{json, Value};

const FAKE_RUNTIME: &str = r#"#!/usr/bin/env python3
import json,os,select,signal,sys,time,tty
from pathlib import Path
config=json.loads(Path(sys.argv[sys.argv.index('--config')+1]).read_text())
root=Path(__file__).parent
with (root/'runtime-launches.jsonl').open('a') as log:
    log.write(json.dumps(config)+'\n')
path=Path(config['snapshot_path']); control=Path(str(path)+'.control')
thread=config['resume'] or ('synthetic-'+config['session'])
state={'state':'idle','stop_reason':'unknown'}; turn=None; sequence=1
prior=None; submits=0; pending=b''; pasted=False; stopping=False
def stop(*_):
    global stopping
    stopping=True
signal.signal(signal.SIGTERM,stop)
tty.setcbreak(0)
print('\x1b[?2004hSynthetic provider fixture\r\n› \r\n100% context left · ? for shortcuts',flush=True)
while not stopping:
    try: instruction=json.loads(control.read_text())
    except (FileNotFoundError,ValueError): instruction={}
    if instruction!=prior:
        if 'state' in instruction: state=instruction['state']
        if 'turn_id' in instruction: turn=instruction['turn_id']
        sequence+=1; prior=instruction
    if instruction.get('publish',True):
        value={'session':config['session'],'generation':config['generation'],'sequence':sequence,
               'heartbeat_ms':time.time_ns()//1000000,'thread_id':thread,'turn_id':turn,
               'state':state,'observation_hold':instruction.get('hold',False),'history_path':None,'stopped':False}
        temporary=path.with_suffix('.tmp')
        temporary.write_text(json.dumps(value));temporary.replace(path)
    if select.select([sys.stdin],[],[],0.1)[0]:
        data=os.read(0,65536)
        if not data: break
        with (root/(config['session']+'.input')).open('ab') as log: log.write(data)
        pending+=data
        while pending:
            if pending.startswith(b'\x1b[200~'):
                pasted=True;pending=pending[6:];continue
            if pending.startswith(b'\x1b[201~'):
                pasted=False;pending=pending[6:];continue
            if pending.startswith(b'\x1b') and len(pending)<6: break
            byte,pending=pending[:1],pending[1:]
            if byte in (b'\n',b'\r') and not pasted:
                submits+=1;turn='synthetic-turn-'+str(submits)
                state={'state':'running'};sequence+=1
                (root/(config['session']+'.submits')).write_text(str(submits))
time.sleep(min(3,max(0,instruction.get("stop_delay",0))))
value.update(stopped=True,sequence=sequence+1,heartbeat_ms=time.time_ns()//1000000)
temporary=path.with_suffix('.tmp');temporary.write_text(json.dumps(value));temporary.replace(path)
"#;

struct Fixture {
    owner: TestFixture,
    repo: PathBuf,
    paths: mesimon_daemon::Paths,
}

impl Fixture {
    fn boot(name: &str) -> Option<Self> {
        if !require_tmux() {
            return None;
        }
        let owner = TestFixture::new(name);
        let repo = owner.dir.join("repo");
        init_repo(&repo, "README.md", "Provider fixture.\n");
        let runtime = owner.dir.join("fake-codex-runtime.py");
        std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
        std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        owner.set_env("MESIMON_CODEX_RUNTIME_BIN", runtime);
        owner.set_env("MESIMON_CODEX_BIN", owner.dir.join("must-not-launch-real-codex"));
        owner.set_env("MESIMON_PANE_QUIET_MS", "600000");
        owner.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
        let paths = owner.paths(&repo);
        let _daemon = owner.daemon(&repo);
        wait_until(Duration::from_secs(10), "provider daemon socket", || {
            paths.orch_sock().exists()
        });
        Some(Self { owner, repo, paths })
    }

    fn client(&self) -> TestClient {
        TestClient::connect(&self.paths.orch_sock())
    }

    fn control(&self, id: uuid::Uuid, value: Value) {
        let path = mesimon_daemon::agents::codex::snapshot_path(&self.paths, id);
        let control = PathBuf::from(format!("{}.control", path.display()));
        mesimon_daemon::agents::codex::write_json(&control, &value).unwrap();
    }

    fn claude_idle(&self, c: &mut TestClient, id: uuid::Uuid) {
        let history = self.owner.dir.join(format!("{id}.jsonl"));
        std::fs::write(&history, "{}\n").unwrap();
        hook_send_with(
            &self.paths.hook_sock(),
            &id.to_string(),
            "SessionStart",
            Some("startup"),
            &json!({"session_id":id,"transcript_path":history,"cwd":self.repo}).to_string(),
        );
        c.await_state(id, "Claude input ready", |s| matches!(s, SessionState::Idle { .. }));
    }

    fn claude_start(&self, c: &mut TestClient, id: uuid::Uuid) {
        hook_send(&self.paths.hook_sock(), &id.to_string(), "UserPromptSubmit", "{}");
        c.await_state(id, "Claude running", |s| *s == SessionState::Running);
    }

    fn claude_stop(&self, c: &mut TestClient, id: uuid::Uuid) {
        hook_send(&self.paths.hook_sock(), &id.to_string(), "Stop", "{}");
        c.await_state(id, "Claude completed", |s| {
            *s == (SessionState::Idle { stop_reason: StopReason::EndTurn })
        });
    }

    fn codex_ready(&self, c: &mut TestClient, id: uuid::Uuid) {
        c.await_state(id, "synthetic Codex input ready", |s| {
            matches!(s, SessionState::Idle { .. })
        });
        wait_until(Duration::from_secs(10), "Codex prefill delivery", || {
            !session(c, id).pending_prefill
        });
    }

    fn restart(&self, c: &mut TestClient) -> TestClient {
        assert!(matches!(c.request(Command::Shutdown), Response::Ok));
        wait_until(Duration::from_secs(10), "owned daemon shutdown", || {
            !self.paths.orch_sock().exists()
        });
        let _daemon = self.owner.daemon(&self.repo);
        wait_until(Duration::from_secs(10), "replacement daemon", || {
            self.paths.orch_sock().exists()
        });
        self.client()
    }
}

fn ticket(c: &mut TestClient, title: &str, workspace: Option<WorkspaceStrategy>) -> ulid::Ulid {
    match c.request(Command::CreateTicket { column: "TODO".into(), title: title.into(), workspace })
    {
        Response::Created { id, .. } => id,
        response => panic!("create ticket: {response:?}"),
    }
}

fn select(c: &mut TestClient, provider: AgentProvider) {
    assert!(matches!(c.request(Command::SetAgentProvider { provider }), Response::Ok));
    assert_eq!(c.board().agent_provider, provider);
}

fn spawn(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("spawn agent: {response:?}"),
    }
}

fn session(c: &mut TestClient, id: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == id).expect("session")
}

#[test]
fn project_switches_preserve_old_running_and_sleeping_sessions_and_scoped_tools() {
    let Some(h) = Fixture::boot("providers") else { return };
    let mut c = h.client();
    assert_eq!(c.board().agent_provider, AgentProvider::ClaudeCode);
    let a = ticket(&mut c, "old Claude", None);
    let claude = spawn(&mut c, a);
    h.claude_idle(&mut c, claude);
    select(&mut c, AgentProvider::Codex);
    let b = ticket(&mut c, "new Codex", None);
    let codex = spawn(&mut c, b);
    h.codex_ready(&mut c, codex);
    let thread = session(&mut c, codex).codex_thread_id.unwrap();
    assert_eq!(session(&mut c, codex).kind, SessionKind::Codex);
    for id in [claude, codex] {
        assert!(matches!(
            c.send(Principal::Agent { session: id }, Command::AgentGetTicket),
            Response::AgentTicket { .. }
        ));
        assert!(matches!(
            c.send(
                Principal::Agent { session: id },
                Command::SetAgentProvider { provider: AgentProvider::ClaudeCode }
            ),
            Response::Err { .. }
        ));
    }
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: a,
            kind: SessionKind::Codex,
            submit_prompt: false
        }),
        Response::Err { .. }
    ));
    h.claude_start(&mut c, claude);
    h.claude_stop(&mut c, claude);
    assert!(matches!(c.request(Command::SleepSession { id: claude }), Response::Ok));
    assert!(matches!(
        c.request(Command::WakeSession { id: claude }),
        Response::Spawned { fresh: false, .. }
    ));
    assert_eq!(session(&mut c, claude).kind, SessionKind::Claude);
    h.claude_idle(&mut c, claude);
    h.control(
        codex,
        json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"synthetic-finished"}),
    );
    c.await_state(codex, "Codex complete", |s| {
        *s == (SessionState::Idle { stop_reason: StopReason::EndTurn })
    });
    assert!(matches!(c.request(Command::SleepSession { id: codex }), Response::Ok));
    wait_until(Duration::from_secs(10), "Codex worker fully stopped", || {
        !session(&mut c, codex).codex_stopping
    });
    select(&mut c, AgentProvider::ClaudeCode);
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: b,
            kind: SessionKind::Claude,
            submit_prompt: false
        }),
        Response::Err { .. }
    ));
    assert!(matches!(
        c.request(Command::WakeSession { id: codex }),
        Response::Spawned { fresh: false, .. }
    ));
    c.await_state(codex, "resumed Codex", |s| matches!(s, SessionState::Idle { .. }));
    assert_eq!(session(&mut c, codex).codex_thread_id.as_deref(), Some(thread.as_str()));
    let launches = std::fs::read_to_string(h.owner.dir.join("runtime-launches.jsonl")).unwrap();
    assert!(launches
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .any(|v| v["resume"] == thread));
    let d = ticket(&mut c, "new Claude again", None);
    let new_claude = spawn(&mut c, d);
    assert_eq!(session(&mut c, new_claude).kind, SessionKind::Claude);
    c = h.restart(&mut c);
    assert_eq!(c.board().agent_provider, AgentProvider::ClaudeCode);
    assert_eq!(session(&mut c, codex).kind, SessionKind::Codex);
    assert_eq!(session(&mut c, codex).codex_thread_id.as_deref(), Some(thread.as_str()));
    assert_eq!(session(&mut c, claude).kind, SessionKind::Claude);
}

#[test]
fn queued_start_keeps_first_provider_even_when_its_words_are_replaced() {
    let Some(h) = Fixture::boot("providerqueue") else { return };
    let mut c = h.client();
    let holder = ticket(&mut c, "holder", None);
    let id = spawn(&mut c, holder);
    h.claude_idle(&mut c, id);
    h.claude_start(&mut c, id);
    select(&mut c, AgentProvider::Codex);
    let waiting = ticket(&mut c, "queued Codex", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "first words".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    select(&mut c, AgentProvider::ClaudeCode);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "replacement words".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    h.claude_stop(&mut c, id);
    wait_until(Duration::from_secs(15), "captured Codex queued start", || {
        c.board().live_agent(waiting).is_some()
    });
    let record = c.board().live_agent(waiting).unwrap().clone();
    assert_eq!(record.kind, SessionKind::Codex);
    wait_until(Duration::from_secs(15), "one queued submit", || {
        h.owner.dir.join(format!("{}.submits", record.id)).exists()
    });
    let text = std::fs::read_to_string(h.owner.dir.join(format!("{}.input", record.id))).unwrap();
    assert!(text.contains("replacement words"), "{text:?}");
    assert!(!text.contains("first words"), "{text:?}");
    assert_eq!(
        std::fs::read_to_string(h.owner.dir.join(format!("{}.submits", record.id))).unwrap(),
        "1"
    );
}

#[test]
fn provisioning_captures_provider_and_excludes_a_competing_agent() {
    let Some(h) = Fixture::boot("providerprovision") else { return };
    let mut c = h.client();
    let hook = h.repo.join(".git/hooks/post-checkout");
    std::fs::write(&hook,"#!/bin/sh\nbase=$(dirname \"$0\")\ntouch \"$base/entered\"\ni=0\nwhile [ ! -e \"$base/released\" ] && [ \"$i\" -lt 100 ]; do sleep 0.05; i=$((i+1)); done\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "provision Codex", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: t,
            kind: SessionKind::Claude,
            submit_prompt: false
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(5), "owned provisioning hook", || {
        h.repo.join(".git/hooks/entered").exists()
    });
    select(&mut c, AgentProvider::ClaudeCode);
    // A repeated request coalesces into the already accepted provision; it
    // must neither replace its captured provider nor reserve a second seat.
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: t,
            kind: SessionKind::Claude,
            submit_prompt: false
        }),
        Response::Provisioning
    ));
    std::fs::write(h.repo.join(".git/hooks/released"), "").unwrap();
    wait_until(Duration::from_secs(15), "captured provisioned provider", || {
        c.board().live_agent(t).is_some()
    });
    assert_eq!(c.board().live_agent(t).unwrap().kind, SessionKind::Codex);
    assert_eq!(c.board().sessions.iter().filter(|s| s.ticket == t && s.kind.is_agent()).count(), 1);
}

#[test]
fn lost_codex_observation_holds_the_checkout_and_recovers_without_duplicate_submit() {
    let Some(h) = Fixture::boot("providerhold") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let mut obsolete = h.client();
    match obsolete
        .request(Command::Hello { version: 1, client: "obsolete-provider-fixture".into() })
    {
        Response::Err { message } => assert!(message.contains("protocol 1 unsupported")),
        response => panic!("old client must be refused before provider snapshots: {response:?}"),
    }
    let holder = ticket(&mut c, "Codex observed", None);
    let id = spawn(&mut c, holder);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"publish":false}));
    c.await_state(id, "lost observation", |s| matches!(s, SessionState::Unknown { .. }));
    assert!(session(&mut c, id).observation_hold);
    select(&mut c, AgentProvider::ClaudeCode);
    let waiting = ticket(&mut c, "must wait", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "after observation recovers".into(),
            queued: true
        }),
        Response::Queued { .. }
    ));
    std::thread::sleep(Duration::from_millis(1800));
    assert!(c.board().live_agent(waiting).is_none());
    assert_eq!(pending_of(&mut c, Some(waiting)).len(), 1);
    h.control(
        id,
        json!({"publish":true,"state":{"state":"idle","stop_reason":"unknown"},"hold":false}),
    );
    wait_until(Duration::from_secs(15), "audited recovery releases queued start", || {
        c.board().live_agent(waiting).is_some()
    });
    assert_eq!(c.board().live_agent(waiting).unwrap().kind, SessionKind::Claude);
}

#[test]
fn sleeping_codex_holds_checkout_and_refuses_wake_until_worker_stops() {
    let Some(h) = Fixture::boot("providerstop") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let holder = ticket(&mut c, "stop owns checkout", None);
    let id = spawn(&mut c, holder);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"stop_delay":2}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    let stopping = session(&mut c, id);
    assert_eq!(stopping.state, SessionState::Sleeping);
    assert!(stopping.codex_stopping);
    assert!(matches!(c.request(Command::WakeSession { id }), Response::Err { .. }));
    select(&mut c, AgentProvider::ClaudeCode);
    let waiting = ticket(&mut c, "wait for server teardown", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "after owned server stops".into(),
            queued: true,
        }),
        Response::Queued { .. }
    ));
    std::thread::sleep(Duration::from_millis(400));
    assert!(session(&mut c, id).codex_stopping);
    assert!(c.board().live_agent(waiting).is_none());
    wait_until(Duration::from_secs(10), "stopped server releases checkout", || {
        !session(&mut c, id).codex_stopping && c.board().live_agent(waiting).is_some()
    });
    assert_eq!(c.board().live_agent(waiting).unwrap().kind, SessionKind::Claude);
}
