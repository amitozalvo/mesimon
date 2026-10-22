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
use mesimon_core::command::{AgentTicketView, Command, PendingAction, Response};
use mesimon_core::Principal;
use serde_json::{json, Value};

const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");

struct Fixture {
    owner: TestFixture,
    repo: PathBuf,
    paths: mesimon_daemon::Paths,
}

impl Fixture {
    fn boot(name: &str) -> Option<Self> {
        Self::boot_with_spawn_failure(name, false)
    }

    fn boot_with_spawn_failure(name: &str, enable_spawn_failure: bool) -> Option<Self> {
        Self::boot_configured(name, enable_spawn_failure, &[])
    }

    fn boot_configured(
        name: &str,
        enable_spawn_failure: bool,
        env: &[(&str, &str)],
    ) -> Option<Self> {
        Self::boot_layout(name, enable_spawn_failure, env, true)
    }

    fn boot_layout(
        name: &str,
        enable_spawn_failure: bool,
        env: &[(&str, &str)],
        root_repo: bool,
    ) -> Option<Self> {
        if !require_tmux() {
            return None;
        }
        let owner = TestFixture::new(name);
        let repo = owner.dir.join("repo");
        let git_root = if root_repo { repo.clone() } else { repo.join("child") };
        init_repo(&git_root, "README.md", "Provider fixture.\n");
        let runtime = owner.dir.join("fake-codex-runtime.py");
        std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
        std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        owner.set_env("MESIMON_CODEX_RUNTIME_BIN", runtime);
        owner.set_env("MESIMON_CODEX_BIN", owner.dir.join("must-not-launch-real-codex"));
        owner.set_env("MESIMON_CODEX_HOME", owner.dir.join("synthetic-codex-home"));
        owner.set_env("MESIMON_PANE_QUIET_MS", "600000");
        owner.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
        if enable_spawn_failure {
            let wrapper = owner.dir.join("tmux-with-spawn-failure.py");
            let tmux = mesimon_backend_tmux::tmux_bin();
            let fail = owner.dir.join("fail-spawn");
            std::fs::write(&wrapper, format!(
                "#!/usr/bin/env python3\nimport os,sys\nfrom pathlib import Path\nif 'new-session' in sys.argv and Path({fail}).exists(): sys.exit(71)\nos.execv({tmux}, [{tmux}]+sys.argv[1:])\n",
                fail = serde_json::to_string(&fail).unwrap(),
                tmux = serde_json::to_string(&tmux).unwrap(),
            )).unwrap();
            std::fs::set_permissions(&wrapper, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .unwrap();
            owner.set_env("MESIMON_TMUX_BIN", wrapper);
        }
        for (key, value) in env {
            owner.set_env(key, value);
        }
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
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        response => panic!("create ticket: {response:?}"),
    }
}

fn select(c: &mut TestClient, provider: AgentProvider) {
    assert!(matches!(c.request(Command::SetAgentProvider { provider }), Response::Ok));
    assert_eq!(c.board().agent_provider, provider);
}

/// An archived ticket's parked record is off the snapshot (T-804), and
/// asked for by its ticket.
fn session(c: &mut TestClient, id: uuid::Uuid) -> SessionRecord {
    let board = c.board();
    if let Some(rec) = board.sessions.iter().find(|s| s.id == id) {
        return rec.clone();
    }
    let archived: Vec<_> = c.archived().iter().map(|t| t.id).collect();
    archived.into_iter().flat_map(|t| c.sessions_of(t)).find(|s| s.id == id).expect("session")
}

#[test]
fn project_switches_preserve_old_running_and_sleeping_sessions_and_scoped_tools() {
    let Some(h) = Fixture::boot("providers") else { return };
    let mut c = h.client();
    assert_eq!(c.board().agent_provider, AgentProvider::ClaudeCode);
    let a = ticket(&mut c, "old Claude", None);
    let claude = c.spawn_claude(a);
    h.claude_idle(&mut c, claude);
    select(&mut c, AgentProvider::Codex);
    let b = ticket(&mut c, "new Codex", None);
    let codex = c.spawn_claude(b);
    h.codex_ready(&mut c, codex);
    assert!(!h.repo.join(".codex").exists(), "shared checkout is not ours to prepare");
    let native_before = session(&mut c, codex);
    let foreign =
        json!({"session_id":claude, "transcript_path":h.owner.dir.join("foreign-claude.jsonl")})
            .to_string();
    hook_send_with(
        &h.paths.hook_sock(),
        &codex.to_string(),
        "SessionEnd",
        Some("prompt_input_exit"),
        &foreign,
    );
    hook_send(&h.paths.hook_sock(), &codex.to_string(), "PermissionRequest", &foreign);
    std::thread::sleep(Duration::from_millis(400));
    let native_after = session(&mut c, codex);
    assert_eq!(native_after.state, native_before.state, "Claude hooks cannot change Codex state");
    assert_eq!(native_after.codex_thread_id, native_before.codex_thread_id);
    assert_eq!(native_after.transcript_path, native_before.transcript_path);
    assert_eq!(native_after.claude_session_id, None);
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
            submit_prompt: false,
            plan: false
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
            submit_prompt: false,
            plan: false
        }),
        Response::Err { .. }
    ));
    let woke = c.request(Command::WakeSession { id: codex });
    assert!(matches!(woke, Response::Spawned { fresh: false, .. }), "{woke:?}");
    c.await_state(codex, "resumed Codex", |s| matches!(s, SessionState::Idle { .. }));
    assert_eq!(session(&mut c, codex).codex_thread_id.as_deref(), Some(thread.as_str()));
    let launches = std::fs::read_to_string(h.owner.dir.join("runtime-launches.jsonl")).unwrap();
    assert!(launches
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .any(|v| v["resume"] == thread));
    let d = ticket(&mut c, "new Claude again", None);
    let new_claude = c.spawn_claude(d);
    assert_eq!(session(&mut c, new_claude).kind, SessionKind::Claude);
    c = h.restart(&mut c);
    assert_eq!(c.board().agent_provider, AgentProvider::ClaudeCode);
    assert_eq!(session(&mut c, codex).kind, SessionKind::Codex);
    assert_eq!(session(&mut c, codex).codex_thread_id.as_deref(), Some(thread.as_str()));
    assert_eq!(session(&mut c, claude).kind, SessionKind::Claude);
}

/// A column's timer applies to Codex too, but an idle-looking observation
/// hold and an active turn must survive it. Wake keeps the exact thread.
#[test]
fn column_autosleep_parks_codex_only_when_observed_quiet_and_resumes_its_thread() {
    let Some(h) = Fixture::boot_configured(
        "codexautosleep",
        false,
        &[("MESIMON_INACTIVITY_PARK_TICKS", "1"), ("MESIMON_INACTIVITY_MINUTE_MS", "3000")],
    ) else {
        return;
    };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let mut ids = Vec::new();
    for title in ["quiet", "held", "working"] {
        let t = ticket(&mut c, title, None);
        let id = c.spawn_claude(t);
        h.codex_ready(&mut c, id);
        ids.push(id);
    }
    let [quiet, held, working] = ids[..] else { unreachable!() };
    h.control(held, json!({"hold":true}));
    h.control(working, json!({"state":{"state":"running"},"turn_id":"working-turn"}));
    c.await_state(working, "active Codex turn", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(10), "observation hold", || {
        session(&mut c, held).observation_hold
    });
    let thread = session(&mut c, quiet).codex_thread_id.unwrap();
    std::thread::sleep(Duration::from_secs(4));
    assert!(session(&mut c, quiet).state.has_pane(), "timer defaults off");
    for column in c.board().columns {
        let mut settings = column.settings;
        settings.sleep_after_minutes = 1;
        assert!(matches!(
            c.request(Command::SetColumnSettings { name: column.name, settings }),
            Response::Ok
        ));
    }
    c.await_state(quiet, "Codex automatically sleeping", |s| *s == SessionState::Sleeping);
    assert!(session(&mut c, held).state.has_pane(), "observation hold prevents auto-sleep");
    assert_eq!(session(&mut c, working).state, SessionState::Running);
    wait_until(Duration::from_secs(10), "automatic Codex cleanup", || {
        !session(&mut c, quiet).codex_stopping
    });
    assert_eq!(session(&mut c, quiet).codex_thread_id.as_deref(), Some(thread.as_str()));
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(feed
        .lines()
        .any(|line| { line.contains("\"autosleep\"") && line.contains("\"automation\"") }));
    assert!(matches!(
        c.request(Command::WakeSession { id: quiet }),
        Response::Spawned { fresh: false, .. }
    ));
    h.codex_ready(&mut c, quiet);
    assert_eq!(session(&mut c, quiet).codex_thread_id.as_deref(), Some(thread.as_str()));
    let launches = std::fs::read_to_string(h.owner.dir.join("runtime-launches.jsonl")).unwrap();
    assert!(launches
        .lines()
        .any(|line| { serde_json::from_str::<Value>(line).unwrap()["resume"] == thread }));
    c.await_state(quiet, "resumed Codex sleeps again", |s| *s == SessionState::Sleeping);
    h.control(held, json!({"hold":false}));
    c.await_state(held, "released observation hold sleeps", |s| *s == SessionState::Sleeping);
    assert_eq!(session(&mut c, working).state, SessionState::Running);
}

fn starts_with_any_status_line(footer: &str) {
    let Some(h) = Fixture::boot("providercontextstart") else { return };
    std::fs::write(
        h.owner.dir.join("initial-screen.json"),
        json!({
            "screen":format!("Synthetic provider fixture\r\n› \r\n{footer}")
        })
        .to_string(),
    )
    .unwrap();
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let immediate = ticket(&mut c, "immediate start", None);
    let id = match c.request(Command::SpawnSession {
        ticket: immediate,
        kind: SessionKind::Codex,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("immediate spawn: {response:?}"),
    };
    c.await_state(id, "immediate prompt accepted", |s| *s == SessionState::Running);
    let waiting = ticket(&mut c, "queued start", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "queued instructions".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    assert!(c.board().live_agent(waiting).is_none());
    h.control(id, json!({"state":{"state":"idle","stop_reason":"end_turn"}}));
    wait_until(Duration::from_secs(15), "queued prompt accepted", || {
        c.board().live_agent(waiting).is_some_and(|s| s.state == SessionState::Running)
    });
    let queued = c.board().live_agent(waiting).unwrap().id;
    std::thread::sleep(Duration::from_millis(1200));
    for (sid, title) in [(id, "immediate start"), (queued, "queued start")] {
        let record = session(&mut c, sid);
        assert!(!record.pending_prefill && !record.pending_submit && !record.codex_submit_sent);
        let input = std::fs::read_to_string(h.owner.dir.join(format!("{sid}.input"))).unwrap();
        assert_eq!(input.matches(title).count(), 1, "{input:?}");
        if sid == queued {
            assert_eq!(input.matches("queued instructions").count(), 1, "{input:?}");
        }
        assert_eq!(
            std::fs::read_to_string(h.owner.dir.join(format!("{sid}.submits"))).unwrap(),
            "1"
        );
    }
}

#[test]
fn arbitrary_status_line_submits_immediate_and_queued_starts_once() {
    starts_with_any_status_line("my completely arbitrary status");
}

#[test]
fn no_status_line_submits_immediate_and_queued_starts_once() {
    starts_with_any_status_line("");
}

#[test]
fn a_hidden_cursor_keeps_initial_input_out_of_a_startup_dialog() {
    let Some(h) = Fixture::boot("providercursorhold") else { return };
    std::fs::write(
        h.owner.dir.join("initial-screen.json"),
        json!({
            "screen":"› old composer\r\n? for shortcuts\r\nUnknown startup dialog\r\n› 1. Continue",
            "cursor":null
        })
        .to_string(),
    )
    .unwrap();
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "wait for the actual composer", None);
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Codex,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("spawn: {response:?}"),
    };
    c.await_state(id, "structured idle with a native modal", |s| {
        matches!(s, SessionState::Idle { .. })
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(session(&mut c, id).pending_prefill);
    assert!(!h.owner.dir.join(format!("{id}.input")).exists());
    h.control(id, json!({"screen":"Synthetic provider fixture\r\n› ","cursor":[2,1]}));
    c.await_state(id, "composer accepts prompt after dialog closes", |s| {
        *s == SessionState::Running
    });
    assert_eq!(std::fs::read_to_string(h.owner.dir.join(format!("{id}.submits"))).unwrap(), "1");
}

#[test]
fn codex_worktree_launches_expose_existing_trust_without_writing_settings() {
    let Some(h) = Fixture::boot("provider-worktree-layer") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "reuse repository trust", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Codex,
            submit_prompt: false,
            plan: false,
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(15), "Codex worktree launch", || {
        c.board().live_agent(ticket).is_some()
    });
    let id = c.board().live_agent(ticket).unwrap().id;
    h.codex_ready(&mut c, id);
    let rec = session(&mut c, id);
    let cwd = PathBuf::from(&rec.cwd);
    let layer = cwd.join(".codex");
    assert!(layer.is_dir());
    assert_eq!(std::fs::read_dir(&layer).unwrap().count(), 0, "no config file or trust entry");
    assert!(git(&cwd, &["status", "--porcelain"]).is_empty(), "empty directory keeps Git clean");
    assert!(!h.repo.join(".codex").exists());

    // A missing directory on wake covers worktrees provisioned by older
    // versions too. Existing config, files and symlinks belong to the user.
    let settings = "# User settings\napproval_policy = \"on-request\"\n";
    let elsewhere = h.owner.dir.join("existing-codex");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("config.toml"), settings).unwrap();
    for existing in ["missing", "config", "symlink", "file"] {
        assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
        wait_until(Duration::from_secs(10), "Codex stopped before worktree wake", || {
            !session(&mut c, id).codex_stopping
        });
        match existing {
            "missing" => std::fs::remove_dir(&layer).unwrap(),
            "config" => std::fs::write(layer.join("config.toml"), settings).unwrap(),
            "symlink" => {
                std::fs::remove_file(layer.join("config.toml")).unwrap();
                std::fs::remove_dir(&layer).unwrap();
                std::os::unix::fs::symlink(&elsewhere, &layer).unwrap();
            }
            "file" => {
                std::fs::remove_file(&layer).unwrap();
                std::fs::write(&layer, "user file").unwrap();
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            c.request(Command::WakeSession { id }),
            Response::Spawned { fresh: false, .. }
        ));
        h.codex_ready(&mut c, id);
        assert_eq!(session(&mut c, id).codex_thread_id, rec.codex_thread_id);
        match existing {
            "missing" => assert_eq!(std::fs::read_dir(&layer).unwrap().count(), 0),
            "config" => {
                assert_eq!(std::fs::read_to_string(layer.join("config.toml")).unwrap(), settings);
            }
            "symlink" => {
                assert_eq!(std::fs::read_link(&layer).unwrap(), elsewhere);
                assert_eq!(std::fs::read_to_string(layer.join("config.toml")).unwrap(), settings);
                assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1);
            }
            "file" => assert_eq!(std::fs::read_to_string(&layer).unwrap(), "user file"),
            _ => unreachable!(),
        }
    }
    let config: Value = serde_json::from_slice(
        &std::fs::read(h.paths.hooks_dir().join(format!("{id}.codex.json"))).unwrap(),
    )
    .unwrap();
    assert!(!config["config_flags"].to_string().contains("trust_level"));
    assert!(!h.owner.dir.join("synthetic-codex-home").exists(), "no user trust configuration");
    assert!(!h.repo.join(".codex").exists());
}

#[test]
fn codex_does_not_add_a_project_layer_to_a_plain_folder_workspace() {
    let Some(h) = Fixture::boot_layout("provider-folder-layer", false, &[], false) else {
        return;
    };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "folder workspace", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Codex,
            submit_prompt: false,
            plan: false,
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(15), "Codex folder workspace launch", || {
        c.board().live_agent(ticket).is_some()
    });
    let id = c.board().live_agent(ticket).unwrap().id;
    h.codex_ready(&mut c, id);
    let cwd = PathBuf::from(session(&mut c, id).cwd);
    assert!(cwd.join("child/.git").is_file(), "nested repo has its own Git worktree");
    assert!(!cwd.join(".codex").exists(), "container has no main checkout to inherit from");
    assert!(!cwd.join("child/.codex").exists(), "only the launch cwd is prepared");
    assert!(!h.repo.join(".codex").exists());
}

#[test]
fn worktree_trust_calls_for_the_person_before_delivering_the_queued_prompt() {
    use mesimon_core::board::Reason;
    let Some(h) = Fixture::boot("providerworktreetrust") else { return };
    std::fs::write(
        h.owner.dir.join("initial-screen.json"),
        json!({
            "screen":"  Trust this folder? Codex can read, edit, and run files here, subject to\r\n  your permission settings.\r\n› 1. Trust and continue\r\n  2. Quit",
            "cursor":null
        })
        .to_string(),
    )
    .unwrap();
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "start after my trust decision", Some(WorkspaceStrategy::Worktree));
    let mut notifications = mesimon_core::notify::Differ::default();
    notifications.scan(&c.board(), false, Default::default());
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Codex,
            submit_prompt: true,
            plan: false,
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(15), "Codex starts in the provisioned worktree", || {
        c.board().live_agent(ticket).is_some()
    });
    let id = c.board().live_agent(ticket).unwrap().id;
    c.await_state(id, "folder trust is visible on the board", |state| {
        *state == SessionState::RequiresAction { reason: Reason::Trust }
    });
    let board = c.board();
    assert!(mesimon_core::attention::attention_queue(&board).iter().any(|s| s.id == id));
    assert_eq!(
        notifications.scan(&board, false, Default::default()),
        vec![mesimon_core::notify::Event::needs_you(
            ticket,
            &board.ticket(ticket).unwrap().short_key,
            "TRUST"
        )]
    );
    let held = session(&mut c, id);
    assert_eq!(held.detail.as_deref(), Some("Codex asks to trust this folder"));
    assert!(held.pending_prefill && held.pending_submit);
    assert!(!h.owner.dir.join(format!("{id}.input")).exists());
    assert_eq!(board.ticket(ticket).unwrap().column, "TODO");

    // Folder trust may lead to hook review, still before the first turn.
    h.control(id, json!({"screen":"Hooks need review\r\n1. Review hooks\r\n2. Trust all and continue", "cursor":null}));
    std::thread::sleep(Duration::from_millis(1800));
    assert_eq!(session(&mut c, id).state, SessionState::RequiresAction { reason: Reason::Trust });
    assert!(
        !h.owner.dir.join(format!("{id}.input")).exists(),
        "no answer or prompt in either dialog"
    );

    h.control(id, json!({"screen":"Synthetic provider fixture\r\n› ","cursor":[2,1]}));
    c.await_state(id, "composer accepts the original prompt after trust", |state| {
        *state == SessionState::Running
    });
    let running = session(&mut c, id);
    assert!(running.detail.is_none());
    assert!(!running.pending_prefill && !running.pending_submit);
    let input = std::fs::read_to_string(h.owner.dir.join(format!("{id}.input"))).unwrap();
    assert_eq!(input.matches("start after my trust decision").count(), 1);
    assert_eq!(std::fs::read_to_string(h.owner.dir.join(format!("{id}.submits"))).unwrap(), "1");
}

#[test]
fn queued_start_keeps_first_provider_even_when_its_words_are_replaced() {
    let Some(h) = Fixture::boot("providerqueue") else { return };
    let mut c = h.client();
    let holder = ticket(&mut c, "holder", None);
    let id = c.spawn_claude(holder);
    h.claude_idle(&mut c, id);
    h.claude_start(&mut c, id);
    select(&mut c, AgentProvider::Codex);
    let waiting = ticket(&mut c, "queued Codex", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "first words".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    select(&mut c, AgentProvider::ClaudeCode);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "replacement words".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
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

/// A real census result backed only by this fixture's synthetic history.
fn external_candidate(h: &Fixture, c: &mut TestClient) -> uuid::Uuid {
    let directory = h.owner.dir.join("synthetic-codex-home/sessions/2026/09/10");
    std::fs::create_dir_all(&directory).unwrap();
    let metadata = json!({"type":"session_meta", "payload":{
        "id":"synthetic-external-reservation-candidate", "cwd":h.paths.repo_root,
        "source":"cli", "cli_version":"0.153.4"}});
    std::fs::write(directory.join("external-reservation.jsonl"), format!("{metadata}\n")).unwrap();
    rescan_external(c)
        .into_iter()
        .find(|item| item.conversation_id == "synthetic-external-reservation-candidate")
        .expect("owned external history discovered")
        .id
}

fn assert_external_cannot_displace_reservation(
    c: &mut TestClient,
    ticket: ulid::Ulid,
    selector: uuid::Uuid,
) {
    let response =
        c.request(Command::AttachExternal { claude_session_id: selector, ticket: Some(ticket) });
    assert!(
        matches!(response, Response::Err { ref message } if message.contains("already provisioning")),
        "external adoption displaced an accepted reservation: {response:?}"
    );
    assert!(!c.board().sessions.iter().any(|record| record.ticket == ticket
        && record.provenance == mesimon_core::board::Provenance::Adopted));
}

#[test]
fn provisioning_captures_provider_and_excludes_a_competing_agent() {
    let Some(h) = Fixture::boot("providerprovision") else { return };
    let mut c = h.client();
    let external = external_candidate(&h, &mut c);
    let hook = h.repo.join(".git/hooks/post-checkout");
    std::fs::write(&hook,"#!/bin/sh\nbase=$(dirname \"$0\")\ntouch \"$base/entered\"\ni=0\nwhile [ ! -e \"$base/released\" ] && [ \"$i\" -lt 100 ]; do sleep 0.05; i=$((i+1)); done\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "provision Codex", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: t,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(5), "owned provisioning hook", || {
        h.repo.join(".git/hooks/entered").exists()
    });
    select(&mut c, AgentProvider::ClaudeCode);
    assert_external_cannot_displace_reservation(&mut c, t, external);
    // A repeated request coalesces into the already accepted provision; it
    // must neither replace its captured provider nor reserve a second seat.
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: t,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false
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
    let id = c.spawn_claude(holder);
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
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
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
    let id = c.spawn_claude(holder);
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
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
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

#[test]
fn killed_codex_keeps_agent_seat_until_its_server_cleanup_is_acknowledged() {
    let Some(h) = Fixture::boot("providerkillseat") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "one agent until cleanup", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"stop_delay":2}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(c.request(Command::KillSession { id }), Response::Ok));
    assert!(session(&mut c, id).codex_stopping);
    select(&mut c, AgentProvider::ClaudeCode);
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Err { message } => assert!(message.contains("stopping")),
        response => panic!("replacement must wait for stopped server: {response:?}"),
    }
    assert_eq!(c.board().live_agent(ticket).unwrap().id, id);
    wait_until(Duration::from_secs(10), "killed Codex server cleanup", || {
        !session(&mut c, id).codex_stopping
    });
    let replacement = c.spawn_claude(ticket);
    assert_eq!(session(&mut c, replacement).kind, SessionKind::Claude);
    assert_eq!(
        c.board()
            .sessions
            .iter()
            .filter(|s| s.ticket == ticket && (s.state.is_live() || s.codex_stopping))
            .count(),
        1
    );
}

#[test]
fn a_deleted_tickets_crashed_codex_record_is_released_once_no_known_owner_remains() {
    // T-357: the runtime dies without ever reporting `stopped`, so its record
    // keeps `codex_stopping`. Deleting the ticket keeps the record as cleanup
    // evidence (on purpose), but with no ticket no gesture can reach it, and
    // `is_working` held a laptop awake and a shared checkout busy for days.
    // The daemon now releases such a record itself — only on the positive
    // evidence a human resume would rely on, and never while a known owner
    // is still present.
    let Some(h) = Fixture::boot("providerorphan") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let holder = ticket(&mut c, "crashed runtime on a deleted ticket", None);
    let id = c.spawn_claude(holder);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"stop_ack": false}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(c.request(Command::KillSession { id }), Response::Ok));
    wait_until(Duration::from_secs(10), "runtime pane died without a stop ack", || {
        let s = session(&mut c, id);
        s.codex_stopping && !s.state.has_pane()
    });
    std::thread::sleep(Duration::from_millis(1200));
    assert!(session(&mut c, id).codex_stopping, "a crash never confirms cleanup");
    // A listener on an old runtime endpoint is a known owner: the sweep must
    // refuse while it stands, through the undo window and a retry.
    let config: Value = serde_json::from_slice(
        &std::fs::read(h.paths.hooks_dir().join(format!("{id}.codex.json"))).unwrap(),
    )
    .unwrap();
    let endpoint = PathBuf::from(config["proxy_socket"].as_str().unwrap());
    let _ = std::fs::remove_file(&endpoint);
    let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
    assert!(matches!(
        c.request(Command::DeleteTicket { id: holder, discard_worktree: false }),
        Response::Ok
    ));
    std::thread::sleep(Duration::from_secs(13));
    assert!(c.board().ticket(holder).is_none(), "the undo window has closed");
    assert!(
        c.board().sessions.iter().any(|s| s.id == id && s.codex_stopping),
        "a live endpoint is a known owner; the record must stay"
    );
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap();
    assert!(journal.contains("codex orphan kept"), "{journal}");
    drop(listener);
    std::fs::remove_file(&endpoint).unwrap();
    wait_until(Duration::from_secs(40), "orphaned cleanup record released", || {
        c.board().sessions.iter().all(|s| s.id != id)
    });
    // The feed flushes on the wheel, one tick behind the snapshot.
    wait_until(Duration::from_secs(5), "release reaches the feed", || {
        std::fs::read_to_string(h.paths.activity_log()).unwrap().contains("CodexOrphanReleased")
    });
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap();
    assert!(journal.contains("codex orphan released"), "{journal}");
}

#[test]
fn archiving_sleeping_codex_preserves_worktree_until_server_cleanup_finishes() {
    let Some(h) = Fixture::boot("providerarchivestop") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "merged cleanup still owns cwd", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Codex,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(15), "Codex worktree provisioned", || {
        c.board().live_agent(ticket).is_some()
    });
    let id = c.board().live_agent(ticket).unwrap().id;
    h.codex_ready(&mut c, id);
    let path = PathBuf::from(session(&mut c, id).cwd);
    std::fs::write(path.join("landed.txt"), "owned fixture commit\n").unwrap();
    git(&path, &["add", "landed.txt"]);
    git(&path, &["commit", "-qm", "landed"]);
    let branch = git(&path, &["branch", "--show-current"]);
    git(&h.repo, &["merge", "--ff-only", branch.trim()]);
    h.control(id, json!({"stop_delay":2}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    assert!(matches!(c.request(Command::ArchiveTicket { id: ticket }), Response::Ok));
    std::thread::sleep(Duration::from_millis(600));
    assert!(session(&mut c, id).codex_stopping, "server has not confirmed cleanup");
    assert!(path.is_dir(), "archive must preserve cwd while owned server stops");
    wait_until(Duration::from_secs(15), "archive teardown follows server cleanup", || {
        !session(&mut c, id).codex_stopping && !path.exists()
    });
}

#[test]
fn an_accepted_exited_resume_reserves_the_provisioning_seat_across_provider_switches() {
    let Some(h) = Fixture::boot("providerresumeseat") else { return };
    let mut c = h.client();
    let external = external_candidate(&h, &mut c);
    select(&mut c, AgentProvider::Codex);
    let ticket =
        ticket(&mut c, "resume original after rebuilding", Some(WorkspaceStrategy::Worktree));
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Codex,
            submit_prompt: false,
            plan: false
        }),
        Response::Provisioning
    ));
    wait_until(Duration::from_secs(15), "first worktree provisioned", || {
        c.board().live_agent(ticket).is_some()
    });
    let id = c.board().live_agent(ticket).unwrap().id;
    h.codex_ready(&mut c, id);
    let original = session(&mut c, id);
    let path = PathBuf::from(&original.cwd);
    assert!(matches!(c.request(Command::KillSession { id }), Response::Ok));
    wait_until(Duration::from_secs(10), "original server stopped", || {
        !session(&mut c, id).codex_stopping
    });
    assert!(matches!(c.request(Command::ArchiveTicket { id: ticket }), Response::Ok));
    wait_until(Duration::from_secs(15), "merged empty worktree reclaimed", || !path.exists());
    assert!(matches!(c.request(Command::UnarchiveTicket { id: ticket }), Response::Ok));
    let hook = h.repo.join(".git/hooks/post-checkout");
    std::fs::write(&hook, "#!/bin/sh\nbase=$(dirname \"$0\")\ntouch \"$base/entered\"\ni=0\nwhile [ ! -e \"$base/released\" ] && [ \"$i\" -lt 100 ]; do sleep 0.05; i=$((i+1)); done\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let resumed = c.request(Command::ResumeSession { id, confirm: false });
    assert!(matches!(resumed, Response::Provisioning), "{resumed:?}");
    wait_until(Duration::from_secs(5), "resume provisioning entered barrier", || {
        h.repo.join(".git/hooks/entered").exists()
    });
    select(&mut c, AgentProvider::ClaudeCode);
    assert_external_cannot_displace_reservation(&mut c, ticket, external);
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Err { message } => assert!(message.contains("resume is already provisioning")),
        response => panic!("accepted original resume reserves provider: {response:?}"),
    }
    let resumed = c.request(Command::ResumeSession { id, confirm: false });
    assert!(matches!(resumed, Response::Provisioning), "{resumed:?}");
    std::fs::write(h.repo.join(".git/hooks/released"), "").unwrap();
    wait_until(Duration::from_secs(15), "original record resumes after rebuild", || {
        c.board().live_agent(ticket).is_some_and(|record| record.id == id)
    });
    h.codex_ready(&mut c, id);
    let resumed = session(&mut c, id);
    assert_eq!(resumed.kind, SessionKind::Codex);
    assert_eq!(resumed.codex_thread_id, original.codex_thread_id);
    assert_eq!(c.board().sessions.iter().filter(|record| record.ticket == ticket).count(), 1);
}

#[test]
fn daemon_handover_keeps_sent_codex_prompt_held_without_pressing_enter_again() {
    let Some(h) = Fixture::boot("providersenthold") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "original title", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"ack":false}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "send exactly once".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(10), "submitted native Enter awaiting acknowledgement", || {
        let record = session(&mut c, id);
        record.pending_submit && record.codex_submit_sent
    });
    c = h.restart(&mut c);
    std::thread::sleep(Duration::from_millis(800));
    let record = session(&mut c, id);
    assert!(record.pending_submit && record.codex_submit_sent && record.observation_hold);
    assert_eq!(std::fs::read_to_string(h.owner.dir.join(format!("{id}.submits"))).unwrap(), "1");
    h.control(id, json!({"state":{"state":"running"},"turn_id":"accepted-after-handover"}));
    wait_until(Duration::from_secs(10), "observed turn acknowledges owed submission", || {
        let record = session(&mut c, id);
        !record.pending_submit && !record.codex_submit_sent && record.state == SessionState::Running
    });
}

fn submits(h: &Fixture, id: uuid::Uuid) -> u32 {
    std::fs::read_to_string(h.owner.dir.join(format!("{id}.submits")))
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn fed(h: &Fixture, cmd: &str, ticket: ulid::Ulid) -> usize {
    let needle = format!("\"cmd\":\"{cmd}\"");
    std::fs::read_to_string(h.paths.activity_log())
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains(&needle) && l.contains(&ticket.to_string()))
        .count()
}

fn steer(c: &mut TestClient, ticket: ulid::Ulid, text: &str) -> Response {
    c.request(Command::PromptSession {
        ticket,
        text: text.into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    })
}

/// A running Codex seat takes words mid-turn as its composer does (T-845):
/// pasted, then one Enter, which Codex sends as a `turn/steer`; the runtime's
/// count of taken steers is the ack. The turn goes on — no new turn, no
/// parked words waiting for its end — and the seat takes the next at once.
#[test]
fn a_steer_goes_into_a_running_codex_turn_and_is_acked_by_the_runtime() {
    let Some(h) = Fixture::boot("providersteer") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "a long turn", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"state":{"state":"running"},"turn_id":"long-turn"}));
    c.await_state(id, "the turn running", |s| *s == SessionState::Running);
    let before = submits(&h, id);
    assert!(matches!(steer(&mut c, ticket, "also check the docs"), Response::Ok));
    wait_until(Duration::from_secs(10), "the steer taken by the running turn", || {
        fed(&h, "codex_steered", ticket) == 1
    });
    assert_eq!(fed(&h, "prompt_submitted", ticket), 1);
    assert_eq!(submits(&h, id), before + 1, "one Enter");
    let input = std::fs::read_to_string(h.owner.dir.join(format!("{id}.input"))).unwrap();
    assert_eq!(input.matches("also check the docs").count(), 1, "{input:?}");
    let record = session(&mut c, id);
    assert_eq!(record.state, SessionState::Running);
    assert_eq!(record.codex_turn_id.as_deref(), Some("long-turn"));
    assert!(!record.pending_submit, "nothing parked for the turn's end");
    // The seat is free for the next steer straight away.
    assert!(matches!(steer(&mut c, ticket, "and the changelog"), Response::Ok));
    wait_until(Duration::from_secs(10), "the second steer", || {
        fed(&h, "codex_steered", ticket) == 2
    });
    assert_eq!(submits(&h, id), before + 2);
}

/// A dialog over a running Codex turn is the person's (T-568): a steer is
/// refused in the board's words and nothing reaches the pane.
#[test]
fn a_codex_dialog_refuses_a_steer_in_the_boards_words() {
    let Some(h) = Fixture::boot("providersteerdialog") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "a turn with a dialog", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(
        id,
        json!({"state":{"state":"running"},"turn_id":"asking-turn",
            "screen":"Allow command?\r\n› 1. Yes\r\n  2. No","cursor":null}),
    );
    c.await_state(id, "the turn running", |s| *s == SessionState::Running);
    std::thread::sleep(Duration::from_millis(300));
    match steer(&mut c, ticket, "words for the box") {
        Response::Err { message } => {
            assert_eq!(message, mesimon_core::command::ANSWER_IN_PANE_FIRST)
        }
        response => panic!("a steer into a dialog: {response:?}"),
    }
    std::thread::sleep(Duration::from_millis(800));
    let input = std::fs::read_to_string(h.owner.dir.join(format!("{id}.input"))).unwrap();
    assert!(!input.contains("words for the box"), "{input:?}");
    assert!(!session(&mut c, id).pending_submit);
}

/// A steer the runtime never counts is pressed once — the box emptied, so
/// the words went — and given up when the presses run out, never pressed
/// into an empty box again (T-845).
#[test]
fn an_unacked_codex_steer_is_pressed_once_and_given_up() {
    let Some(h) = Fixture::boot("providersteerlost") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "a turn that never says", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"state":{"state":"running"},"turn_id":"quiet-turn","steer_ack":false}));
    c.await_state(id, "the turn running", |s| *s == SessionState::Running);
    let before = submits(&h, id);
    assert!(matches!(steer(&mut c, ticket, "are you there"), Response::Ok));
    match steer(&mut c, ticket, "a second while the first is owed") {
        Response::Err { message } => assert!(message.contains("already waiting"), "{message}"),
        response => panic!("a second steer: {response:?}"),
    }
    wait_until(Duration::from_secs(15), "the steer given up", || {
        fed(&h, "prompt_submit_gave_up", ticket) == 1
    });
    assert_eq!(submits(&h, id), before + 1, "one Enter, none into the empty box");
    assert_eq!(fed(&h, "codex_steered", ticket), 0);
}

/// A turn in flight across a `U` handover reads `Running` at once, never
/// `Unknown` until its next change (T-827): T-824 sat grey and "unavailable"
/// on the board through seven minutes of a test run, its own work holding
/// the observation. Its end then lands as any end does.
#[test]
fn daemon_handover_restores_a_codex_turn_held_by_its_own_work() {
    let Some(h) = Fixture::boot("providerturnheld") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "a long test run", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"state":{"state":"running"},"turn_id":"long-turn","hold":true}));
    c.await_state(id, "the turn running", |s| *s == SessionState::Running);
    c = h.restart(&mut c);
    c.await_state(id, "the same turn after the handover", |s| *s == SessionState::Running);
    h.control(id, json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"long-turn"}));
    c.await_state(id, "its end", |s| {
        *s == (SessionState::Idle { stop_reason: StopReason::EndTurn })
    });
}

/// T-843: a Codex seat held a minute past its turn's close is written to the
/// journal once, with the runtime's reasons, and its release once more.
#[test]
fn a_codex_seat_held_past_its_turn_and_its_release_reach_the_journal() {
    let Some(h) = Fixture::boot("providerheldturn") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "a turn that ended", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    let since = mesimon_core::clock::now_ms() - 61_000;
    h.control(
        id,
        json!({"state":{"state":"running"},"turn_id":"closed-turn","hold":true,
            "hold_reasons":{"items":["commandExecution"],"turn_closed":true,"since_ms":since,"closed_since_ms":since}}),
    );
    c.await_state(id, "the turn running", |s| *s == SessionState::Running);
    let log = h.paths.daemon_log();
    let journal = || std::fs::read_to_string(&log).unwrap_or_default();
    wait_until(Duration::from_secs(10), "the held line", || journal().contains("held "));
    let line = journal();
    assert!(line.contains("past its turn's close — items commandExecution"), "{line}");
    // The same turn, still held: said once.
    h.control(
        id,
        json!({"state":{"state":"running"},"turn_id":"closed-turn","hold":true,
            "hold_reasons":{"items":["commandExecution"],"turn_closed":true,"since_ms":since,"closed_since_ms":since},
            "screen":"Synthetic provider fixture\r\n› still"}),
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(journal().matches("past its turn's close").count(), 1);
    h.control(
        id,
        json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"closed-turn",
            "hold":false,"releases":1,"last_release":"items commandExecution"}),
    );
    c.await_state(id, "its end", |s| {
        *s == (SessionState::Idle { stop_reason: StopReason::EndTurn })
    });
    wait_until(Duration::from_secs(10), "the release line", || {
        journal().contains("released a turn held past its close — items commandExecution")
    });
    assert_eq!(journal().matches("released a turn").count(), 1);
}

/// Words parked for a held Codex seat reached no box, so the shutdown hands
/// them over as `unsent` and the next daemon pastes them on the seat's next
/// idle edge, with one Enter (T-849).
#[test]
fn daemon_handover_keeps_unpasted_codex_words_and_sends_them_once() {
    let Some(h) = Fixture::boot("providerunsent") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "title only", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"hold":true}));
    wait_until(Duration::from_secs(10), "observation gates input", || {
        session(&mut c, id).observation_hold
    });
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "handed over words".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Ok
    ));
    assert!(session(&mut c, id).pending_submit);
    c = h.restart(&mut c);
    let record = session(&mut c, id);
    assert!(record.pending_submit && record.unsent.is_none(), "{record:?}");
    wait_until(Duration::from_secs(10), "the resend on the feed", || {
        fed(&h, "prompt_resent", ticket) == 1
    });
    assert_eq!(submits(&h, id), 0, "nothing goes into a held seat");
    h.control(id, json!({"hold":false}));
    wait_until(Duration::from_secs(10), "the handed-over words submitted", || submits(&h, id) == 1);
    let input = std::fs::read_to_string(h.owner.dir.join(format!("{id}.input"))).unwrap();
    assert_eq!(input.matches("handed over words").count(), 1, "{input}");
    h.control(id, json!({"state":{"state":"running"},"turn_id":"handed-over-turn"}));
    wait_until(Duration::from_secs(10), "the new turn acks them", || {
        let record = session(&mut c, id);
        !record.pending_submit && record.state == SessionState::Running
    });
    assert_eq!(submits(&h, id), 1);
}

#[test]
fn resumed_codex_reports_native_trust_without_fresh_prefill_and_recovers_to_idle() {
    let Some(h) = Fixture::boot("provider-wake-trust") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let ticket = ticket(&mut c, "wake trust fixture", None);
    let id = c.spawn_claude(ticket);
    h.codex_ready(&mut c, id);
    let thread = session(&mut c, id).codex_thread_id;
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    wait_until(Duration::from_secs(10), "Codex worker stopped before wake", || {
        !session(&mut c, id).codex_stopping
    });
    h.control(
        id,
        json!({"state":{"state":"idle","stop_reason":"unknown"},
        "screen":"Hooks need review\r\n1. Review hooks\r\n2. Trust all and continue", "cursor":null}),
    );
    assert!(matches!(
        c.request(Command::WakeSession { id }),
        Response::Spawned { fresh: false, .. }
    ));
    c.await_state(id, "native trust on exact resume", |state| {
        *state == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Trust }
    });
    let held = session(&mut c, id);
    assert!(!held.pending_prefill, "wake must not become a fresh title submission");
    assert_eq!(held.codex_thread_id, thread);
    assert_eq!(c.board().ticket(ticket).unwrap().column, "TODO");
    h.control(
        id,
        json!({"state":{"state":"idle","stop_reason":"unknown"},
        "screen":"Synthetic provider fixture\r\n› \r\n100% context left · ? for shortcuts"}),
    );
    c.await_state(id, "native composer restored after trust", |state| {
        *state == SessionState::Idle { stop_reason: StopReason::Unknown }
    });
    assert_eq!(session(&mut c, id).codex_thread_id, thread);
    assert_eq!(
        c.board().ticket(ticket).unwrap().column,
        "TODO",
        "trust resolution cannot complete a turn"
    );
    assert!(
        !h.owner.dir.join(format!("{id}.submits")).exists(),
        "trust resolution cannot press Enter"
    );
}

#[test]
fn uncertain_cleanup_requires_new_human_acknowledgement_and_retains_old_evidence() {
    let Some(h) = Fixture::boot_with_spawn_failure("provideruncertainresume", true) else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let holder = ticket(&mut c, "unknown descendants hold checkout", None);
    let id = c.spawn_claude(holder);
    h.codex_ready(&mut c, id);
    let old = session(&mut c, id);
    h.control(id, json!({"stop_delay": 1, "stop_ack": false}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    match c.request(Command::ResumeSession { id, confirm: true }) {
        Response::Err { message } => assert!(message.contains("remains live"), "{message}"),
        response => panic!("live runtime cannot be replaced: {response:?}"),
    }
    select(&mut c, AgentProvider::ClaudeCode);
    let waiting = ticket(&mut c, "queued behind uncertain cleanup", None);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket: waiting,
            text: "wait".into(),
            queued: true,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(session(&mut c, id).codex_stopping);
    assert!(c.board().live_agent(waiting).is_none());
    assert!(matches!(c.request(Command::WakeSession { id }), Response::Err { .. }));
    assert!(matches!(
        c.send(
            Principal::Automation { rule: "fixture recovery".into() },
            Command::ResumeSession { id, confirm: true }
        ),
        Response::Err { .. }
    ));
    let warning = |response| match response {
        Response::NeedsConfirm { message } => {
            assert!(message.contains("cleanup is unverified"), "{message}");
            assert!(message.contains("unknown child processes may remain"), "{message}");
            assert!(message.contains("resume again to acknowledge"), "{message}");
        }
        response => panic!("first confirmation must offer the risk, never bypass it: {response:?}"),
    };
    warning(c.request(Command::ResumeSession { id, confirm: true }));
    // Known owner reappearance between gestures revokes the old offer.
    let config: Value = serde_json::from_slice(
        &std::fs::read(h.paths.hooks_dir().join(format!("{id}.codex.json"))).unwrap(),
    )
    .unwrap();
    let endpoint = PathBuf::from(config["proxy_socket"].as_str().unwrap());
    let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
    match c.request(Command::ResumeSession { id, confirm: true }) {
        Response::Err { message } => assert!(message.contains("endpoint"), "{message}"),
        response => panic!("an old native listener must refuse recovery: {response:?}"),
    }
    drop(listener);
    std::fs::remove_file(endpoint).unwrap();
    warning(c.request(Command::ResumeSession { id, confirm: true }));
    // A new daemon cannot inherit an unconfirmed user gesture. The queued
    // start it CAN inherit (T-418): the entry is back on the card, and it
    // keeps waiting behind the unverified cleanup exactly as before.
    c = h.restart(&mut c);
    warning(c.request(Command::ResumeSession { id, confirm: true }));
    let parked = pending_of(&mut c, Some(waiting));
    assert_eq!(parked.len(), 1, "the queued start survives the restart: {parked:?}");
    assert_eq!(parked[0].action, PendingAction::Start);
    assert!(c.board().live_agent(waiting).is_none(), "and still waits on the cleanup");
    assert_eq!(session(&mut c, id).codex_generation, old.codex_generation);
    assert!(session(&mut c, id).codex_stopping);
    h.control(id, json!({}));
    let fail_spawn = h.owner.dir.join("fail-spawn");
    std::fs::write(&fail_spawn, "synthetic failure before a native process starts").unwrap();
    match c.request(Command::ResumeSession { id, confirm: true }) {
        Response::Err { message } => {
            assert!(message.contains("resume spawn failed"), "{message}");
            assert!(!message.contains("rollback failed"), "{message}");
        }
        response => panic!("injected spawn failure must retain old generation: {response:?}"),
    }
    let restored: Value = serde_json::from_slice(
        &std::fs::read(h.paths.hooks_dir().join(format!("{id}.codex.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(restored["generation"], json!(old.codex_generation.unwrap()));
    assert!(session(&mut c, id).codex_stopping);
    std::fs::remove_file(fail_spawn).unwrap();
    warning(c.request(Command::ResumeSession { id, confirm: false }));
    assert!(matches!(
        c.request(Command::ResumeSession { id, confirm: true }),
        Response::Spawned { id: resumed, fresh: false } if resumed == id
    ));
    h.codex_ready(&mut c, id);
    let resumed = session(&mut c, id);
    assert_eq!(resumed.kind, SessionKind::Codex);
    assert_eq!(resumed.codex_thread_id, old.codex_thread_id);
    assert_ne!(resumed.codex_generation, old.codex_generation);
    assert!(!resumed.codex_stopping);
    // The checkout is proved quiet at last, and only now does the start
    // that waited through the cleanup — and the restart — go (T-418), on
    // the provider it was queued with.
    wait_until(
        Duration::from_secs(15),
        "the restored start to go once the cleanup verified",
        || c.board().live_agent(waiting).is_some(),
    );
    assert_eq!(c.board().live_agent(waiting).unwrap().kind, SessionKind::Claude);
    assert_eq!(
        c.board()
            .sessions
            .iter()
            .filter(|s| s.ticket == holder && (s.state.is_live() || s.codex_stopping))
            .count(),
        1
    );
    let feed = std::fs::read_to_string(h.paths.activity_log()).unwrap();
    assert!(feed.contains("CleanupUnverifiedResume"));
    assert!(feed.contains("unknown child processes may remain"));
    let retained = std::fs::read_dir(h.paths.hooks_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .find(|value| value["unknown_descendants_may_remain"] == true)
        .expect("old cleanup evidence retained separately from new runtime generation");
    assert_eq!(retained["snapshot"]["generation"], json!(old.codex_generation.unwrap()));
    assert_eq!(retained["snapshot"]["stopped"], false);
}

/// T-443: a Codex seat owed a tier switch is parked at its idle, waits for
/// its runtime to confirm the stop (a Codex wake cannot acknowledge
/// unverified cleanup), and is then woken on the new tier's config — the
/// same thread resumed.
#[test]
fn a_codex_tier_switch_parks_waits_for_the_stop_and_wakes_on_the_new_tier() {
    use mesimon_core::tier::{Effort, Tier, TierScope};
    let Some(h) = Fixture::boot("providertier") else { return };
    let mut c = h.client();
    let reviewer = Tier {
        id: "01REVIEW".into(),
        name: "reviewer".into(),
        provider: AgentProvider::Codex,
        model: "gpt-6-astra".into(),
        effort: Effort::High,
        description: String::new(),
    };
    assert!(matches!(
        c.request(Command::SaveTier { scope: TierScope::Board, tier: reviewer }),
        Response::Ok
    ));
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "codex switch", None);
    let id = c.spawn_claude(t);
    h.codex_ready(&mut c, id);
    let thread = session(&mut c, id).codex_thread_id.clone();
    assert!(thread.is_some(), "a thread to resume");
    h.control(id, json!({"stop_delay":2}));
    std::thread::sleep(Duration::from_millis(250));
    assert!(matches!(
        c.request(Command::SetTicketTier { id: t, tier: Some("01REVIEW".into()) }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(15), "the park for the switch", || {
        let rec = session(&mut c, id);
        rec.tier_wake && rec.state == SessionState::Sleeping
    });
    assert!(session(&mut c, id).codex_stopping, "the stop is not confirmed yet");
    wait_until(Duration::from_secs(20), "the wake on the new tier", || {
        let rec = session(&mut c, id);
        !rec.tier_wake && rec.tier == "01REVIEW" && rec.state != SessionState::Sleeping
    });
    let rec = session(&mut c, id);
    assert_eq!(rec.codex_thread_id, thread, "the same thread");
    let config = std::fs::read_to_string(h.paths.hooks_dir().join(format!("{id}.codex.json")))
        .expect("the runtime's config");
    assert!(config.contains(r#"model=\"gpt-6-astra\""#), "{config}");
    assert!(config.contains(r#"model_reasoning_effort=\"high\""#), "{config}");
}

/// Codex 0.163.0's question dialog as the fixture paints it (T-846), the
/// cursor on `row` (1 or 2), in the pane's own line ends.
fn codex_question_screen(row: usize) -> String {
    let mark = |n: usize| if n == row { "›" } else { " " };
    format!(
        "  Question 1/1 (1 unanswered)\r\n  Choose fixture color?\r\n\r\n  {} 1. Blue               \
         Use blue for the fixture.\r\n  {} 2. Green              Use green for the fixture.\r\n    \
         3. None of the above  Optionally, add details in notes (tab)\r\n\r\n  tab to add notes | \
         enter to submit answer | esc to interrupt\r\n",
        mark(1),
        mark(2)
    )
}

/// Whether `keys` hold an Enter: the pane's tty reads tmux's `Enter` as a
/// newline under `ICRNL`, a raw one as a return.
fn entered(keys: &[u8]) -> bool {
    keys.contains(&b'\n') || keys.contains(&b'\r')
}

/// T-846: a Codex question's request rides the runtime's snapshot into the
/// card Remote Control and the crown read, the board's answer lands as the
/// keys the native dialog takes, and the stop ends on the request's
/// resolution, never on a timer. A secret question and an approval project
/// no question card, and the crown is refused in words.
#[test]
fn a_codex_question_is_carried_to_the_board_and_answered_by_keys_until_resolved() {
    let Some(h) = Fixture::boot("codex_question") else { return };
    let mut c = h.client();
    let read = |c: &mut TestClient, session: uuid::Uuid, key: &str| -> AgentTicketView {
        match c.send(Principal::Agent { session }, Command::AgentReadTicket { key: key.into() }) {
            Response::AgentTicket { ticket } => ticket,
            other => panic!("read {key}: {other:?}"),
        }
    };
    // The crown is a Claude stub seat; its worker a Codex seat in a worktree
    // of its own, since the crown holds the checkout.
    select(&mut c, AgentProvider::ClaudeCode);
    let crown = ticket(&mut c, "coordinate", None);
    let worker = ticket(&mut c, "mesimon-probe-846 codex worker", None);
    let sa = c.spawn_claude(crown);
    assert!(matches!(c.request(Command::CrownTicket { id: crown }), Response::Ok));
    h.claude_idle(&mut c, sa);
    select(&mut c, AgentProvider::Codex);
    let kw = c.board().ticket(worker).unwrap().short_key.clone();
    let view = read(&mut c, sa, &kw);
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: view.seen,
                plan: false,
                tier: None,
                workspace: Some("worktree".into()),
            },
        ),
        Response::AgentStarted { .. }
    ));
    wait_until(Duration::from_secs(20), "the Codex worker's seat", || {
        c.board().live_agent(worker).is_some_and(|s| s.kind == SessionKind::Codex)
    });
    let ws = c.board().live_agent(worker).unwrap().id;
    h.codex_ready(&mut c, ws);
    let input = h.owner.dir.join(format!("{ws}.input"));
    let typed = || std::fs::read(&input).unwrap_or_default();
    let asking = |s: &SessionState| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    };

    // ---- the question stands: the snapshot carries it, the card reads it
    let question = json!({"id":"\"q1\"","kind":"question","questions":[{
        "question":"Choose fixture color?","header":"Color","secret":false,
        "options":[
            {"label":"Blue","description":"Use blue for the fixture."},
            {"label":"Green","description":"Use green for the fixture."}
        ]
    }]});
    let at = |row: usize, request: &Value| {
        json!({"state":{"state":"requires_action","reason":"question"},"turn_id":"ask-turn",
               "hold":true,"ack":false,"request":request,
               "screen":codex_question_screen(row),"cursor":null})
    };
    h.control(ws, at(1, &question));
    c.await_state(ws, "asking", asking);
    let needs = read(&mut c, sa, &kw).needs_you.expect("the question");
    assert_eq!(needs.reason, "question");
    assert_eq!(needs.request.as_deref(), Some("\"q1\""));
    assert_eq!(needs.questions.len(), 1);
    assert_eq!(needs.questions[0].text, "Choose fixture color?");
    assert_eq!(needs.questions[0].options, vec!["Blue".to_string(), "Green".to_string()]);
    assert_eq!(needs.answerable, Some(true));

    // ---- the answer walks the dialog: Down to Green, Enter -----------------
    let before = typed().len();
    let view = read(&mut c, sa, &kw);
    let cmd = Command::AgentAnswerTicket {
        key: kw.clone(),
        seen: view.seen,
        request: "\"q1\"".into(),
        index: Some(1),
        text: None,
        answers: None,
    };
    let sock = h.paths.orch_sock();
    let call = std::thread::spawn(move || {
        TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
    });
    wait_until(Duration::from_secs(5), "Down in the Codex pane", || {
        typed()[before..].windows(3).any(|w| w == b"\x1b[B")
    });
    assert!(!entered(&typed()[before..]), "no Enter before the cursor is seen on Green");
    h.control(ws, at(2, &question));
    wait_until(Duration::from_secs(5), "Enter on Green", || entered(&typed()[before..]));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!call.is_finished(), "the receipt waits for the request to resolve, not a timer");
    assert!(asking(&session(&mut c, ws).state), "still needs you until Codex resolves it");

    // ---- Codex resolves the request: the stop ends, the receipt says so ----
    h.control(
        ws,
        json!({"state":{"state":"running"},"turn_id":"ask-turn","hold":true,"ack":false,
               "screen":"• Working (1s • esc to interrupt)\r\n\r\n› Ask Codex to do anything\r\n",
               "cursor":[2,2]}),
    );
    match call.join().unwrap() {
        Response::AgentAnswered { key, outcome, reason, answer, .. } => {
            assert_eq!((key.as_str(), outcome.as_str()), (kw.as_str(), "answered"));
            assert_eq!((reason, answer.as_str()), (None, "Green"));
        }
        other => panic!("answer_agent: {other:?}"),
    }
    c.await_state(ws, "working on the answer", |s| *s == SessionState::Running);
    let feed = std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap();
    assert!(
        feed.lines().any(|l| l.contains("\"cmd\":\"answer_agent\"")
            && l.contains("\"answer\":\"Green\"")
            && l.contains("\"outcome\":\"answered\"")),
        "{feed}"
    );

    // ---- a secret question stays the pane's -------------------------------
    let secret = json!({"id":"\"q2\"","kind":"question","questions":[{
        "question":"Paste the token","header":"Token","secret":true,"options":[]
    }]});
    h.control(
        ws,
        json!({"state":{"state":"requires_action","reason":"secret"},"turn_id":"ask-turn",
               "hold":true,"ack":false,"request":secret,
               "screen":"  Paste the token\r\n  › \r\n","cursor":null}),
    );
    c.await_state(ws, "a secret", |s| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Secret }
    });
    let needs = read(&mut c, sa, &kw).needs_you.expect("the stop");
    assert_eq!((needs.reason.as_str(), needs.request, needs.answerable), ("secret", None, None));
    assert!(needs.questions.is_empty(), "a secret's words never leave the pane");
    let view = read(&mut c, sa, &kw);
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: view.seen,
            request: "\"q2\"".into(),
            index: Some(0),
            text: None,
            answers: None,
        },
    ) {
        Response::Err { message } => assert!(message.contains("secret"), "{message}"),
        other => panic!("a secret answered: {other:?}"),
    }

    // ---- an approval is a person's: the crown is refused in words --------
    let approval = json!({"id":"\"a1\"","kind":"approval","what":"command",
        "command":"printf MESIMON > out.txt","cwd":"/work","reason":"May I?",
        "decisions":["accept","decline"]});
    h.control(
        ws,
        json!({"state":{"state":"requires_action","reason":"permission"},"turn_id":"ask-turn",
               "hold":true,"ack":false,"request":approval,
               "screen":"  Would you like to run the following command?\r\n\r\n› 1. Yes, proceed (y)\r\n  \
                         2. No, and tell Codex what to do differently (esc)\r\n\r\n  Press enter to \
                         confirm or esc to cancel\r\n","cursor":null}),
    );
    c.await_state(ws, "a permission", |s| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Permission }
    });
    let needs = read(&mut c, sa, &kw).needs_you.expect("the stop");
    assert_eq!(needs.reason, "permission");
    assert!(needs.questions.is_empty() && needs.request.is_none());
    let view = read(&mut c, sa, &kw);
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: view.seen,
            request: "\"a1\"".into(),
            index: Some(0),
            text: None,
            answers: None,
        },
    ) {
        Response::Err { message } => {
            assert!(
                message.contains("permission") && message.contains("never the crown"),
                "{message}"
            )
        }
        other => panic!("a permission answered by the crown: {other:?}"),
    }
    h.control(
        ws,
        json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"ask-turn","hold":false,
               "ack":false,"screen":"Synthetic provider fixture\r\n› ","cursor":[2,1]}),
    );
    c.await_state(ws, "done", |s| *s == SessionState::Idle { stop_reason: StopReason::EndTurn });
}

fn plans(h: &Fixture, id: uuid::Uuid) -> u32 {
    std::fs::read_to_string(h.owner.dir.join(format!("{id}.plans")))
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn input(h: &Fixture, id: uuid::Uuid) -> String {
    std::fs::read_to_string(h.owner.dir.join(format!("{id}.input"))).unwrap_or_default()
}

fn initial(h: &Fixture, value: Value) {
    std::fs::write(h.owner.dir.join("initial-screen.json"), value.to_string()).unwrap();
}

fn plan_ask(c: &mut TestClient, ticket: ulid::Ulid, text: &str, queued: bool) -> Response {
    c.request(Command::PromptSession {
        ticket,
        text: text.into(),
        queued,
        immediately: false,
        accept_plan: false,
        plan: true,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    })
}

fn planned_start(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Codex,
        submit_prompt: true,
        plan: true,
    }) {
        Response::Spawned { id, .. } => id,
        response => panic!("planned start: {response:?}"),
    }
}

/// T-847: Codex has no launch flag for plan mode, so a planned start types
/// `/plan` into the composer ahead of the title, and the title and the
/// brief wait until the thread is seen planning. A `/plan` that did not take
/// is pressed again; the one that does lets the words go, once.
#[test]
fn a_planned_codex_start_types_plan_first_and_its_words_wait_for_the_mode() {
    let Some(h) = Fixture::boot("providerplan") else { return };
    initial(&h, json!({"plan_ack": false}));
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "planned start", None);
    let id = planned_start(&mut c, t);
    assert!(session(&mut c, id).codex_plan_owed);
    wait_until(Duration::from_secs(15), "the /plan line", || plans(&h, id) >= 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!input(&h, id).contains("planned start"), "the words wait: {:?}", input(&h, id));
    assert_eq!(submits(&h, id), 0);
    let rec = session(&mut c, id);
    assert!(rec.codex_plan_owed && rec.pending_submit && rec.pending_prefill, "{rec:?}");
    // The thread answers the next `/plan`, as the server's
    // `thread/settings/updated` does: the words go behind it.
    h.control(id, json!({"plan_ack": true}));
    c.await_state(id, "the planned turn", |s| *s == SessionState::Running);
    let typed = input(&h, id);
    let (plan_at, title_at) = (typed.rfind("/plan").unwrap(), typed.find("planned start").unwrap());
    assert!(plan_at < title_at, "{typed:?}");
    assert_eq!(plans(&h, id), 2);
    assert_eq!(submits(&h, id), 1);
    assert_eq!(typed.matches("planned start").count(), 1, "{typed:?}");
    assert!(!session(&mut c, id).codex_plan_owed);
    assert_eq!(fed(&h, "codex_plan_mode", t), 1);
}

/// T-847: a live Codex pane takes plan mode where it stands — `/plan`, then
/// the words, no relaunch — and only between turns: sent now mid-turn the
/// ask is refused as a claude's is, queued it waits for the idle. The
/// footer's `Plan mode` is witness enough where the snapshot says nothing.
#[test]
fn a_codex_pane_asked_in_plan_mode_waits_its_idle_and_types_plan_without_a_relaunch() {
    let Some(h) = Fixture::boot("providerplanpane") else { return };
    initial(&h, json!({"mode_on_snapshot": false, "plan_footer": true}));
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "pane plan", None);
    let id = c.spawn_claude(t);
    h.codex_ready(&mut c, id);
    h.control(id, json!({"state":{"state":"running"},"turn_id":"synthetic-busy"}));
    c.await_state(id, "a turn running", |s| *s == SessionState::Running);
    match plan_ask(&mut c, t, "plan the fix", false) {
        Response::Err { message } => assert!(message.contains("mid-turn"), "{message}"),
        response => panic!("plan mode mid-turn: {response:?}"),
    }
    assert!(matches!(plan_ask(&mut c, t, "plan the fix", true), Response::Queued { .. }));
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(plans(&h, id), 0, "nothing typed into a running turn");
    h.control(
        id,
        json!({"state":{"state":"idle","stop_reason":"end_turn"},"turn_id":"synthetic-busy"}),
    );
    wait_until(Duration::from_secs(20), "the planned ask lands", || submits(&h, id) >= 1);
    let typed = input(&h, id);
    assert!(typed.find("/plan").unwrap() < typed.find("plan the fix").unwrap(), "{typed:?}");
    assert_eq!(plans(&h, id), 1);
    let launches = std::fs::read_to_string(h.owner.dir.join("runtime-launches.jsonl")).unwrap();
    assert_eq!(launches.lines().count(), 1, "the pane was not relaunched");
    assert!(!session(&mut c, id).codex_plan_owed);
}

/// T-847: a parked Codex seat asked in plan mode wakes and types `/plan`
/// before the words, as a start does.
#[test]
fn a_parked_codex_seat_wakes_into_plan_mode_before_its_words() {
    let Some(h) = Fixture::boot("providerplanwake") else { return };
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "wake plan", None);
    let id = c.spawn_claude(t);
    h.codex_ready(&mut c, id);
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    wait_until(Duration::from_secs(10), "the stop confirmed", || {
        let rec = session(&mut c, id);
        rec.state == SessionState::Sleeping && !rec.codex_stopping
    });
    assert_eq!(plans(&h, id), 0);
    assert!(matches!(plan_ask(&mut c, t, "plan after the wake", false), Response::Spawned { .. }));
    wait_until(Duration::from_secs(20), "the planned wake's turn", || submits(&h, id) >= 1);
    let typed = input(&h, id);
    assert!(typed.find("/plan").unwrap() < typed.find("plan after the wake").unwrap(), "{typed:?}");
    assert_eq!(plans(&h, id), 1);
    let launches = std::fs::read_to_string(h.owner.dir.join("runtime-launches.jsonl")).unwrap();
    assert_eq!(launches.lines().count(), 2, "one start and one wake");
}

/// T-847: a `/plan` never seen to take, twice, is given up on: the words go
/// into the box unsubmitted for the person, never into a default-mode turn
/// that would do the work it was asked to plan, and the seat holds nothing.
#[test]
fn a_codex_plan_mode_never_seen_leaves_the_words_unsubmitted() {
    let Some(h) = Fixture::boot("providerplannever") else { return };
    initial(&h, json!({"plan_ack": false}));
    let mut c = h.client();
    select(&mut c, AgentProvider::Codex);
    let t = ticket(&mut c, "never plans", None);
    let id = planned_start(&mut c, t);
    wait_until(Duration::from_secs(30), "the words in the box", || {
        input(&h, id).contains("never plans")
    });
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(plans(&h, id), 2);
    assert_eq!(submits(&h, id), 0, "no Enter under the words");
    let rec = session(&mut c, id);
    assert!(!rec.codex_plan_owed && !rec.pending_submit && !rec.pending_prefill, "{rec:?}");
    assert_eq!(fed(&h, "codex_plan_unconfirmed", t), 1);
    assert!(c.board().live_agent(t).is_some_and(|s| !mesimon_core::quiet::is_working(s)));
}
