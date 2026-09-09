//! Explicit, read-only diagnostics and offline replay. Never starts a daemon.
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use mesimon_core::command::{Command, Envelope, Response, PROTOCOL_VERSION};
use mesimon_core::Principal;
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
            let manifest: Value = serde_json::from_str(include_str!("../../../docs/claude-compatibility.json"))?;
            let version = args.get(1).context("usage: mesimon state compatibility <claude-version>")?;
            let matched = manifest["versions"].get(version);
            println!("{}", serde_json::to_string_pretty(&json!({"claude_version": version,
                "status": if matched.is_some() { "partially_observed" } else { "untested" },
                "evidence": matched, "policy": "No automatic model calls or configuration changes"}))?);
            Ok(())
        }
        _ => bail!("usage: mesimon state replay <files...> | explain [session-prefix] [--repo path] | compatibility <claude-version>"),
    }
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
    let rows: Vec<_> = records.iter().map(|session| {
        let ticket = board.ticket(session.ticket);
        let history: Vec<_> = log.iter().filter(|entry| {
            matches!(entry["kind"].as_str(), Some("state_decision" | "movement_decision" | "session_state" | "hook"))
                && (entry["session"].as_str() == Some(&session.id.to_string())
                    || entry["ticket"].as_str() == Some(&session.ticket.to_string()))
        }).rev().take(30).cloned().collect();
        let move_now = ticket.and_then(|t| board.column(&t.column)).map(|c|
            mesimon_core::automove::explain(&c.settings, &session.state, session.confidence));
        json!({"session": session.id, "ticket": session.ticket, "column": ticket.map(|t| &t.column),
            "state": session.state, "confidence": session.confidence,
            "monitor_task_ids": session.monitor_task_ids,
            "state_changed_at": session.state_changed_at, "movement_eligibility_now": move_now,
            "history_newest_first": history,
            "history_note": "Bounded diagnostic history; event times are historical, not necessarily still pending. Missing history is not proof that no event happened."})
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
