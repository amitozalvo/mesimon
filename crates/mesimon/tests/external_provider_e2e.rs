//! Provider-aware external adoption uses synthetic histories only. Observe-only
//! native history is not proof of inactivity and never grants scoped MCP access.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use mesimon_core::board::{AgentProvider, Provenance, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::json;
use std::time::Duration;

#[test]
fn codex_external_opaque_identity_survives_provider_switch_and_restart_without_false_idle() {
    if !require_tmux() {
        return;
    }
    let owner = TestFixture::new("codex-external-observe");
    let repo = owner.dir.join("repo");
    init_repo(&repo, "README.md", "External provider fixture.\n");
    let paths = owner.paths(&repo);
    let home = owner.dir.join("codex-home");
    let rollouts = home.join("sessions/2026/09/10");
    std::fs::create_dir_all(&rollouts).unwrap();
    let history = rollouts.join("filename-is-not-native-identity.jsonl");
    let opaque = "synthetic-codex-conversation:external:alpha";
    let head = json!({"timestamp":"2026-09-10T00:00:00Z", "type":"session_meta",
        "payload":{"id":opaque,"cwd":paths.repo_root,"source":"cli","cli_version":"0.153.4"}});
    let reply = json!({"timestamp":"2026-09-10T00:01:00Z","type":"response_item",
        "payload":{"type":"message","role":"assistant",
            "content":[{"type":"output_text","text":"Synthetic external reply"}]}});
    // A completed-looking history is display data only; it is deliberately
    // followed by an uncompleted tool call and no structured owned runtime.
    let tool = json!({"timestamp":"2026-09-10T00:02:00Z","type":"response_item",
        "payload":{"type":"function_call","name":"exec_command","call_id":"fixture-call",
            "arguments":"{\"cmd\":\"sleep 60\"}"}});
    std::fs::write(&history, format!("{head}\n{reply}\n{tool}\n")).unwrap();
    let original_history = std::fs::read(&history).unwrap();
    let mut foreign_head = head.clone();
    foreign_head["payload"]["id"] = "synthetic-unrelated-history".into();
    foreign_head["payload"]["cwd"] =
        owner.dir.join("other-repository").display().to_string().into();
    std::fs::write(rollouts.join("outside-this-repo.jsonl"), format!("{foreign_head}\n{reply}\n"))
        .unwrap();
    let mut system_head = head.clone();
    system_head["payload"]["id"] = "synthetic-system-title-thread".into();
    system_head["payload"]["thread_source"] = "system".into();
    std::fs::write(rollouts.join("system-thread.jsonl"), format!("{system_head}\n{reply}\n"))
        .unwrap();
    owner.set_env("MESIMON_CODEX_HOME", &home);
    owner.set_env("CODEX_HOME", &home);
    owner.set_env("MESIMON_CLAUDE_HOME", owner.dir.join("empty-claude-home"));
    owner.set_env("MESIMON_CODEX_BIN", owner.dir.join("never-launch-codex"));
    owner.set_env("MESIMON_PANE_QUIET_MS", "0");
    let daemon = owner.daemon(&repo);
    let mut client = TestClient::connect(&paths.orch_sock());
    let external = match client.request(Command::RescanExternal) {
        Response::Board { external, .. } => external,
        response => panic!("rescan failed: {response:?}"),
    };
    assert_eq!(external.len(), 1);
    let item = &external[0];
    assert_eq!(item.provider, AgentProvider::Codex);
    assert_eq!(item.conversation_id, opaque);
    assert_eq!(item.preview.as_deref(), Some("Synthetic external reply"));
    assert_ne!(item.id.to_string(), opaque);
    let selector = item.id;
    let id = match client
        .request(Command::AttachExternal { claude_session_id: selector, ticket: None })
    {
        Response::Spawned { id, .. } => id,
        response => panic!("attach failed: {response:?}"),
    };
    let check = |client: &mut TestClient| {
        let board = client.board();
        let record = board.sessions.iter().find(|record| record.id == id).unwrap();
        assert_eq!(record.kind, SessionKind::Codex);
        assert_eq!(record.codex_thread_id.as_deref(), Some(opaque));
        assert!(record.claude_session_id.is_none());
        assert_eq!(record.provenance, Provenance::Adopted);
        assert!(record.argv.is_empty());
        assert!(
            matches!(record.state, SessionState::Unknown { .. }),
            "unobserved native history became {:?}",
            record.state
        );
        assert!(record.observation_hold);
        assert!(
            mesimon_core::quiet::is_working(record),
            "unobserved external session released checkout"
        );
    };
    check(&mut client);
    assert!(matches!(client.request(Command::FocusStart { session: id }), Response::Err { .. }));
    let ticket = client.board().sessions.iter().find(|record| record.id == id).unwrap().ticket;
    for queued in [false, true] {
        let response = client.request(Command::PromptSession {
            ticket,
            text: "Never silently park this external prompt".into(),
            queued,
            accept_plan: false,
        });
        assert!(
            matches!(response, Response::Err { ref message } if message.contains("resume it to take over")),
            "{response:?}"
        );
        let board = client.board();
        assert!(!board.sessions.iter().find(|record| record.id == id).unwrap().pending_submit);
    }

    assert!(matches!(
        client.send(Principal::Agent { session: id }, Command::AgentGetTicket),
        Response::Err { .. }
    ));
    assert!(matches!(
        client.request(Command::AttachExternal { claude_session_id: selector, ticket: None }),
        Response::Err { .. }
    ));
    for provider in [AgentProvider::Codex, AgentProvider::ClaudeCode] {
        assert!(matches!(client.request(Command::SetAgentProvider { provider }), Response::Ok));
        check(&mut client);
    }
    std::thread::sleep(Duration::from_secs(2));
    check(&mut client);
    let external = match client.request(Command::RescanExternal) {
        Response::Board { external, .. } => external,
        response => panic!("second rescan failed: {response:?}"),
    };
    assert!(external.is_empty(), "attached opaque conversation appeared twice");
    assert!(matches!(client.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
    let daemon = owner.daemon(&repo);
    let mut client = TestClient::connect(&paths.orch_sock());
    check(&mut client);
    std::thread::sleep(Duration::from_secs(1));
    check(&mut client);
    assert_eq!(
        std::fs::read(&history).unwrap(),
        original_history,
        "adoption rewrote native conversation"
    );
    assert!(matches!(client.request(Command::KillSession { id }), Response::Ok));
    let board = client.board();
    let record = board.sessions.iter().find(|record| record.id == id).unwrap();
    assert!(matches!(record.state, SessionState::Exited { .. }));
    assert!(!record.codex_stopping, "observe-only record waited for nonexistent runtime stop ack");
    assert!(!mesimon_core::quiet::is_working(record));
    assert!(matches!(client.request(Command::KillSession { id }), Response::Ok));
    let board = client.board();
    let record = board.sessions.iter().find(|record| record.id == id).unwrap();
    assert!(!record.codex_stopping, "dismissal introduced a runtime stop wait");
    assert!(!mesimon_core::quiet::is_working(record));
    assert!(matches!(client.request(Command::Shutdown), Response::Ok));
    daemon.join().unwrap();
}
