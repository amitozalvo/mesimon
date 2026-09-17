//! Explicit, read-only diagnostics and offline replay. Never starts a daemon.
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use mesimon_core::board::{SessionKind, SessionRecord};
use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
use mesimon_daemon::agents::{ObservationMode, ResumePolicy};
use mesimon_daemon::state_replay::{replay, Scenario};
use serde_json::{json, Value};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("replay") => {
            anyhow::ensure!(args.len() > 1, "usage: mesimon state replay <scenario.json> ...");
            let mut failed = false;
            for path in &args[1..] {
                let scenario: Scenario = serde_json::from_slice(&std::fs::read(path)?)
                    .with_context(|| format!("scenario {path}"))?;
                let report = replay(&scenario)?;
                failed |= !report.passed;
                println!("{}", serde_json::to_string(&report)?);
            }
            anyhow::ensure!(!failed, "state replay assertions failed");
            Ok(())
        }
        Some("explain") => explain(&args[1..]),
        Some("compatibility") => {
            println!("{}", serde_json::to_string_pretty(&compatibility(&args[1..])?)?);
            Ok(())
        }
        _ => bail!("usage: mesimon state replay <files...> | explain [session-prefix] [--repo path] | compatibility [claude|codex] <version>"),
    }
}

pub(crate) const CODEX_TESTED_VERSION: &str = "0.153.4";

fn compatibility(args: &[String]) -> Result<Value> {
    let (provider, version) = match args {
        [version] => ("claude", version.as_str()),
        [provider, version] if matches!(provider.as_str(), "claude" | "claude_code" | "codex") => {
            (provider.as_str(), version.as_str())
        }
        _ => bail!("usage: mesimon state compatibility [claude|codex] <version>"),
    };
    if provider == "codex" {
        let manifest: Value =
            serde_json::from_str(include_str!("../../../docs/codex-compatibility.json"))?;
        let matched = manifest["versions"].get(version);
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        return Ok(codex_compatibility(&manifest, version, matched, now_ms));
    }
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/claude-compatibility.json"))?;
    let matched = manifest["versions"].get(version);
    Ok(json!({"provider": "claude_code", "claude_version": version,
        "status": if matched.is_some() { "partially_observed" } else { "untested" },
        "evidence": matched, "policy": "No automatic model calls or configuration changes"}))
}

fn codex_compatibility(
    manifest: &Value,
    version: &str,
    matched: Option<&Value>,
    now_ms: u64,
) -> Value {
    let age_days = matched
        .and_then(|evidence| evidence["captured_at_unix_ms"].as_u64())
        .and_then(|captured| now_ms.checked_sub(captured))
        .map(|age| age / 86_400_000);
    json!({"provider": "codex", "codex_version": version,
        "tested_version": CODEX_TESTED_VERSION,
        "status": if matched.is_some() { "partially_observed" } else { "untested" },
        "manifest": "docs/codex-compatibility.json",
        "adapter_revision": manifest["adapter_revision"],
        "evidence_age_days": age_days,
        "evidence": matched,
        "policy": "No automatic model calls or configuration changes"})
}

fn capabilities(kind: SessionKind) -> Value {
    mesimon_daemon::agents::adapter(kind).map_or(Value::Null, |adapter| {
        let caps = adapter.capabilities();
        json!({"observation": match caps.observation {
            ObservationMode::Hooks => "hooks",
            ObservationMode::Structured => "structured",
        }, "resume": match caps.resume {
            ResumePolicy::FreshWhenHistoryMissing => "fresh_when_history_missing",
            ResumePolicy::ExactOnly => "exact_session_only",
        }})
    })
}

fn codex_runtime(path: &Path, session: &SessionRecord, now_ms: u64) -> Value {
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path).and_then(|file| file.take(65_537).read_to_end(&mut bytes));
    match read {
        Err(error) => {
            return json!({"status": if error.kind() == std::io::ErrorKind::NotFound {
            "missing" } else { "unreadable" }, "observed": false})
        }
        Ok(_) if bytes.len() > 65_536 => return json!({"status": "oversized", "observed": false}),
        _ => {}
    }
    let Ok(snapshot) = serde_json::from_slice::<mesimon_daemon::agents::codex::Snapshot>(&bytes)
    else {
        return json!({"status": "invalid", "observed": false});
    };
    let identity_matches =
        snapshot.session == session.id && Some(snapshot.generation) == session.codex_generation;
    // Native /new and /resume can select a conversation in the same owned
    // runtime. Its provider adapter verifies selection; the daemon may not yet
    // have persisted the new projection when diagnostics reads the artifact.
    let conversation_matches =
        session.codex_thread_id.as_ref().is_none_or(|id| Some(id) == snapshot.thread_id.as_ref());
    let age_ms = now_ms.saturating_sub(snapshot.heartbeat_ms);
    let future = snapshot.heartbeat_ms > now_ms.saturating_add(1_000);
    let fresh = age_ms <= 5_000 && !future;
    let sequence_current = snapshot.sequence >= session.codex_observed_seq;
    let status = if !identity_matches {
        "identity_mismatch"
    } else if !sequence_current {
        "older_sequence"
    } else if snapshot.stopped {
        "stopped"
    } else if future {
        "future_heartbeat"
    } else if !fresh {
        "stale"
    } else if !conversation_matches {
        "current_conversation_changed"
    } else {
        "current"
    };
    json!({"status": status, "observed": identity_matches && sequence_current && (fresh || snapshot.stopped),
        "session": snapshot.session, "generation": snapshot.generation, "sequence": snapshot.sequence,
        "thread_id": snapshot.thread_id, "turn_id": snapshot.turn_id,
        "launch_phase": snapshot.launch_phase,
        "title": snapshot.title,
        "heartbeat_ms": snapshot.heartbeat_ms, "age_ms": age_ms, "fresh": fresh,
        "identity_matches": identity_matches, "conversation_matches": conversation_matches,
        "sequence_current": sequence_current,
        "state": snapshot.state, "observation_hold": snapshot.observation_hold,
        "stopped": snapshot.stopped})
}

fn request(stream: &mut BufReader<UnixStream>, command: Command) -> Result<Response> {
    let mut bytes = serde_json::to_vec(&Envelope { principal: Principal::Local, command })?;
    bytes.push(b'\n');
    stream.get_mut().write_all(&bytes)?;
    let mut line = String::new();
    stream.read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

fn explain(args: &[String]) -> Result<()> {
    let mut repo = std::env::current_dir()?;
    let mut prefix = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--repo" {
            index += 1;
            repo = args.get(index).context("--repo needs a path")?.into();
        } else {
            anyhow::ensure!(prefix.is_none(), "only one session prefix is accepted");
            prefix = Some(args[index].as_str());
        }
        index += 1;
    }
    let paths = mesimon_daemon::Paths::for_repo(&repo)?;
    let socket = UnixStream::connect(paths.orch_sock())
        .context("no reachable daemon for this repository; explain does not start one")?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut stream = BufReader::new(socket);
    request(
        &mut stream,
        Command::Hello { version: PROTOCOL_VERSION, client: "state-explain".into() },
    )?;
    let Response::Board { board, .. } = request(&mut stream, Command::Snapshot)? else {
        bail!("daemon did not return a board")
    };
    let records: Vec<_> = board
        .sessions
        .iter()
        .filter(|s| {
            prefix.is_none_or(|p| s.id.to_string().starts_with(p) || s.sid16().starts_with(p))
        })
        .collect();
    anyhow::ensure!(!records.is_empty(), "no matching sessions");
    anyhow::ensure!(prefix.is_none() || records.len() == 1, "ambiguous session prefix");
    let mut log = read_tail(&paths.activity_log().with_extension("jsonl.1"))?;
    log.extend(read_tail(&paths.activity_log())?);
    let now_ms =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let rows: Vec<_> = records.iter().map(|session| {
        let ticket = board.ticket(session.ticket);
        let history: Vec<_> = log.iter().filter(|entry| {
            matches!(entry["kind"].as_str(), Some("state_decision" | "movement_decision" | "session_state" | "hook" | "codex_observation"))
                && (entry["session"].as_str() == Some(&session.id.to_string())
                    || entry["ticket"].as_str() == Some(&session.ticket.to_string()))
        }).rev().take(30).cloned().collect();
        let move_now = ticket.and_then(|t| board.column(&t.column)).map(|c|
            mesimon_core::automove::explain(&c.settings, &session.state, session.confidence));
        json!({"session": session.id, "ticket": session.ticket, "column": ticket.map(|t| &t.column),
            "provider": session.kind.provider(), "session_kind": session.kind,
            "project_provider_for_new_sessions": board.agent_provider,
            "capabilities": capabilities(session.kind),
            "observation_hold": session.kind == SessionKind::Codex && session.observation_hold,
            "provider_identity": {"claude_session_id": session.claude_session_id,
                "codex_thread_id": session.codex_thread_id, "codex_turn_id": session.codex_turn_id,
                "codex_generation": session.codex_generation, "codex_observed_sequence": session.codex_observed_seq},
            "runtime_observation": if session.kind == SessionKind::Codex {
                codex_runtime(&mesimon_daemon::agents::codex::snapshot_path(&paths, session.id), session, now_ms)
            } else { Value::Null },
            "state": session.state, "confidence": session.confidence,
            "state_changed_at": session.state_changed_at, "movement_eligibility_now": move_now,
            "history_newest_first": history,
            "history_note": "Bounded diagnostic history; event times are historical, not necessarily still pending. Missing history is not proof that no event happened.",
            "movement_note": "Column/state eligibility alone does not authorize movement; observation holds and shared-checkout safety gates still apply."})
    }).collect();
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}

fn read_tail(path: &Path) -> Result<Vec<Value>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    let start = file.metadata()?.len().saturating_sub(1024 * 1024);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .skip(usize::from(start > 0))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_keeps_legacy_claude_and_marks_codex_evidence_scope() {
        let legacy = compatibility(&["unknown-version".into()]).unwrap();
        let explicit = compatibility(&["claude".into(), "unknown-version".into()]).unwrap();
        assert_eq!(legacy, explicit);
        assert_eq!(legacy["claude_version"], "unknown-version");
        let codex = compatibility(&["codex".into(), "0.153.4".into()]).unwrap();
        assert_eq!(codex["status"], "partially_observed");
        assert!(codex["evidence"]["scope"].as_str().unwrap().contains("not proof"));
        let unknown = compatibility(&["codex".into(), "9.0.0".into()]).unwrap();
        assert_eq!(unknown["status"], "untested");
        assert!(unknown["evidence"].is_null());
        assert!(compatibility(&["future".into(), "1".into()]).is_err());
    }

    #[test]
    fn codex_manifest_lookup_retains_capture_scope_and_does_not_borrow_version_evidence() {
        let manifest: Value =
            serde_json::from_str(include_str!("../../../docs/codex-compatibility.json")).unwrap();
        assert_eq!(manifest["schema"], 1);
        let evidence = &manifest["versions"][CODEX_TESTED_VERSION];
        let captured = evidence["captured_at_unix_ms"].as_u64().unwrap();
        let report = codex_compatibility(
            &manifest,
            CODEX_TESTED_VERSION,
            Some(evidence),
            captured + 3 * 86_400_000,
        );
        assert_eq!(report["evidence_age_days"], 3);
        assert_eq!(report["evidence"]["end_to_end"]["manual_compaction"]["status"], "passed");
        assert_eq!(
            report["evidence"]["end_to_end"]["structured_write_guard"]["status"],
            "retrospectively_verified"
        );
        assert_eq!(
            report["evidence"]["end_to_end"]["plan_and_queued_prompt"]["original_runner_status"],
            "inconclusive"
        );
        assert!(!report["evidence"]["not_yet_observed"].as_array().unwrap().is_empty());
        let future_clock =
            codex_compatibility(&manifest, CODEX_TESTED_VERSION, Some(evidence), captured - 1);
        assert!(future_clock["evidence_age_days"].is_null());
        for version in ["0.153.3", "0.153.5", "9.0.0"] {
            let report = compatibility(&["codex".into(), version.into()]).unwrap();
            assert_eq!(report["status"], "untested");
            assert!(report["evidence"].is_null());
            assert!(report["evidence_age_days"].is_null());
        }
    }

    #[test]
    fn capabilities_distinguish_exact_resume_and_shells() {
        assert_eq!(capabilities(SessionKind::Codex)["resume"], "exact_session_only");
        assert_eq!(capabilities(SessionKind::Claude)["observation"], "hooks");
        assert!(capabilities(SessionKind::Bash).is_null());
    }

    #[test]
    fn runtime_diagnostic_rejects_missing_stale_foreign_and_oversized_evidence() {
        use mesimon_core::board::{SessionState, StopReason};
        let id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("msmn-state-runtime-{id}.json"));
        let mut record = SessionRecord::new(
            id,
            SessionKind::Codex,
            ulid::Ulid::new(),
            vec![],
            "/fixture".into(),
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        record.codex_generation = Some(7);
        record.codex_thread_id = Some("opaque-thread-id".into());
        record.codex_observed_seq = 2;
        let mut snapshot = mesimon_daemon::agents::codex::Snapshot {
            launch_phase: Default::default(),
            session: id,
            generation: 7,
            sequence: 2,
            heartbeat_ms: 1_000,
            thread_id: record.codex_thread_id.clone(),
            turn_id: Some("turn".into()),
            state: record.state.clone(),
            observation_hold: false,
            history_path: None,
            title: None,
            plan: None,
            plan_key: None,
            stopped: false,
        };
        assert_eq!(codex_runtime(&path, &record, 1_200)["status"], "missing");
        let save = |snapshot: &mesimon_daemon::agents::codex::Snapshot| {
            std::fs::write(&path, serde_json::to_vec(snapshot).unwrap()).unwrap();
        };
        save(&snapshot);
        let current = codex_runtime(&path, &record, 1_200);
        assert_eq!(current["age_ms"], 200);
        assert_eq!(current["observed"], true);
        assert_eq!(codex_runtime(&path, &record, 6_001)["status"], "stale");
        assert_eq!(codex_runtime(&path, &record, 6_001)["observed"], false);
        snapshot.heartbeat_ms = 3_000;
        save(&snapshot);
        assert_eq!(codex_runtime(&path, &record, 1_000)["status"], "future_heartbeat");
        snapshot.generation = 8;
        save(&snapshot);
        assert_eq!(codex_runtime(&path, &record, 3_100)["status"], "identity_mismatch");
        snapshot.generation = 7;
        snapshot.thread_id = Some("another-thread".into());
        save(&snapshot);
        assert_eq!(codex_runtime(&path, &record, 3_100)["observed"], true);
        assert_eq!(codex_runtime(&path, &record, 3_100)["status"], "current_conversation_changed");
        snapshot.thread_id = record.codex_thread_id.clone();
        snapshot.sequence = 1;
        save(&snapshot);
        assert_eq!(codex_runtime(&path, &record, 3_100)["status"], "older_sequence");
        std::fs::write(&path, vec![b' '; 65_537]).unwrap();
        assert_eq!(codex_runtime(&path, &record, 3_100)["status"], "oversized");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn diagnostic_window_can_begin_inside_a_unicode_record() {
        let path =
            std::env::temp_dir().join(format!("msmn-state-tail-{}.jsonl", std::process::id()));
        let mut text = "é".repeat(600_000);
        text.push_str("\n{\"kind\":\"hook\"}\n");
        std::fs::write(&path, text).expect("fixture");
        let rows = read_tail(&path).expect("bounded Unicode tail");
        std::fs::remove_file(&path).expect("cleanup");
        assert_eq!(rows, vec![json!({"kind": "hook"})]);
    }
}
