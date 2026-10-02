//! The mod road's foundation (T-574), end to end with the real binaries: a
//! Claude launch on the mod road, the real `mesimon mod-bridge` polling the
//! daemon from inside the pane, a ping that crosses the whole road and back
//! (daemon → bridge → the mod's stand-in → `mesimon hook --road mod` →
//! daemon), a daemon restart and a killed bridge with nothing lost, the
//! delivery ledger's rules, and the shadow's lines.
//!
//! The stub cannot load a mod, so the harness runs `fake_claude_mod.py`
//! beside it on the mod road (`TestFixture::spawn`): the stand-in engine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::io::{BufRead, Write};
use std::time::Duration;

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::road::Road;
use mesimon_core::Principal;

const STUB: &str = "#!/bin/sh\nexec sleep 300\n";

fn boot(name: &str) -> Option<Harness> {
    Harness::boot_with_env(name, Some(STUB), &[("MESIMON_CLAUDE_ROAD", "mod")])
}

fn spawn(c: &mut TestClient, title: &str, kind: SessionKind) -> (ulid::Ulid, uuid::Uuid) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.iter().find(|t| t.title == title).expect("ticket").id;
    match c.request(Command::SpawnSession { ticket, kind, submit_prompt: false, plan: false }) {
        Response::Spawned { id, .. } => (ticket, id),
        other => panic!("spawn failed: {other:?}"),
    }
}

/// The frames the stand-in engine acted on, once each.
fn received(h: &Harness, session: uuid::Uuid) -> Vec<serde_json::Value> {
    std::fs::read_to_string(h.dir.join(format!("mod-{session}.ndjson")))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn pid(h: &Harness, what: &str, session: uuid::Uuid) -> Option<i32> {
    std::fs::read_to_string(h.dir.join(format!("mod-{what}-{session}.pid")))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn kill9(pid: i32) {
    let status = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    assert!(status.is_ok_and(|s| s.success()), "kill -9 {pid}");
}

fn ping(h: &Harness, session: uuid::Uuid) -> Response {
    h.client("ping").request(Command::ModPing { session })
}

fn disagreements(h: &Harness) -> Vec<serde_json::Value> {
    std::fs::read_to_string(h.paths.activity_log())
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("\"kind\":\"road_disagree\""))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_ping_crosses_the_whole_road_and_survives_a_restart_and_a_dead_bridge() {
    let Some(h) = boot("modbridge-ping") else { return };
    let mut c = h.client("modbridge");
    let (_, sid) = spawn(&mut c, "the mod road", SessionKind::Claude);

    // The launch took the mod: recorded, flagged, laid under the state dir.
    let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
    assert_eq!(rec.road, Road::Mod);
    let at = rec.argv.iter().position(|a| a == "--plugin-dir").expect("--plugin-dir");
    let folder = std::path::PathBuf::from(&rec.argv[at + 1]);
    assert!(folder.starts_with(&h.paths.state_dir), "never a checkout: {}", folder.display());
    assert!(folder.join("hooks/register.ts").is_file());
    assert!(rec.argv.iter().any(|a| a == "--settings"), "the hook set stays in shadow");
    // A shell on the same board never gets the mod.
    let (_, shell) = spawn(&mut c, "a shell", SessionKind::Bash);
    let shell = c.board().sessions.into_iter().find(|s| s.id == shell).unwrap();
    assert_eq!(shell.road, Road::Hooks);

    wait_until(Duration::from_secs(15), "the bridge to start", || pid(&h, "bridge", sid).is_some());
    match ping(&h, sid) {
        Response::ModPonged { ms } => assert!(ms < 5_000, "{ms}"),
        other => panic!("ping: {other:?}"),
    }
    let got = received(&h, sid);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["kind"], "ping");

    // A restart with the bridge alive: the bridge waits the daemon out and
    // polls the new one; a ping sent at once is held for it.
    h.restart();
    match ping(&h, sid) {
        Response::ModPonged { .. } => {}
        other => panic!("ping after a restart: {other:?}"),
    }
    let got = received(&h, sid);
    assert_eq!(got.len(), 2, "each ping once: {got:?}");
    assert_ne!(got[0]["id"], got[1]["id"]);

    // A bridge that dies is respawned; the ping waits for the new one.
    let first = pid(&h, "bridge", sid).unwrap();
    kill9(first);
    match ping(&h, sid) {
        Response::ModPonged { .. } => {}
        other => panic!("ping across a respawn: {other:?}"),
    }
    assert_ne!(pid(&h, "bridge", sid), Some(first), "a new bridge");
    assert_eq!(received(&h, sid).len(), 3);
    assert!(disagreements(&h).is_empty(), "{:?}", disagreements(&h));
}

/// One raw line on a connection, without waiting for the answer.
fn write(c: &mut TestClient, principal: Principal, command: Command) {
    let env = Envelope { principal, command };
    writeln!(c.write, "{}", serde_json::to_string(&env).unwrap()).unwrap();
}

/// The next answer on a connection, or `None` when none comes in `within`.
fn read(c: &mut TestClient, within: Duration) -> Option<Response> {
    c.read.get_ref().set_read_timeout(Some(within)).unwrap();
    let mut line = String::new();
    match c.read.read_line(&mut line) {
        Ok(n) if n > 0 => Some(serde_json::from_str(&line).unwrap()),
        _ => None,
    }
}

fn frames(resp: Response) -> Vec<serde_json::Value> {
    match resp {
        Response::ModFrames { frames } => frames,
        other => panic!("expected frames: {other:?}"),
    }
}

#[test]
fn the_ledger_resends_until_acked_and_one_poll_holds_the_seat() {
    let Some(h) = boot("modbridge-ledger") else { return };
    let mut c = h.client("ledger");
    let (_, sid) = spawn(&mut c, "the ledger", SessionKind::Claude);
    // The test is the bridge here: the stand-in engine and its bridge go.
    wait_until(Duration::from_secs(15), "the bridge to start", || pid(&h, "bridge", sid).is_some());
    kill9(pid(&h, "engine", sid).unwrap());
    kill9(pid(&h, "bridge", sid).unwrap());
    std::thread::sleep(Duration::from_millis(300));
    let agent = Principal::Agent { session: sid };

    // Only the session's own bridge polls, and only a person pings.
    err_containing(
        c.request(Command::ModNext { ack: None, pane: None, speaks: vec![] }),
        "agent principal",
    );
    err_containing(c.send(agent.clone(), Command::ModPing { session: sid }), "not available");

    let mut person = h.client("person");
    write(&mut person, Principal::Local, Command::ModPing { session: sid });
    let mut a = h.client("a");
    let first =
        frames(a.send(agent.clone(), Command::ModNext { ack: None, pane: None, speaks: vec![] }));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["kind"], "ping");
    let id = first[0]["id"].as_str().unwrap().to_string();
    drop(a);

    // Never acked, so a new poll gets it again.
    let mut b = h.client("b");
    let again =
        frames(b.send(agent.clone(), Command::ModNext { ack: None, pane: None, speaks: vec![] }));
    assert_eq!(again[0]["id"], id.as_str());

    // Acked, so the next poll waits.
    write(&mut b, agent.clone(), Command::ModNext { ack: Some(id), pane: None, speaks: vec![] });
    assert!(read(&mut b, Duration::from_millis(400)).is_none(), "parked");

    // A poll from another pane is refused and takes nothing.
    let mut stray = h.client("stray");
    err_containing(
        stray.send(
            agent.clone(),
            Command::ModNext { ack: None, pane: Some("1:%999".into()), speaks: vec![] },
        ),
        "not this session's pane",
    );
    assert!(read(&mut b, Duration::from_millis(200)).is_none(), "still parked");

    // A newer poll takes the seat; the older is told.
    let mut newer = h.client("newer");
    write(&mut newer, agent.clone(), Command::ModNext { ack: None, pane: None, speaks: vec![] });
    err_containing(read(&mut b, Duration::from_secs(5)).expect("superseded"), "superseded");

    // Nobody answered the ping: its sender hears so.
    err_containing(read(&mut person, Duration::from_secs(10)).expect("ping answer"), "no pong");
}

#[test]
fn the_shadow_pairs_the_twins_and_names_a_frame_without_one() {
    let Some(h) = boot("modbridge-shadow") else { return };
    let mut c = h.client("shadow");
    let (_, sid) = spawn(&mut c, "the shadow", SessionKind::Claude);
    let s = sid.to_string();
    let sock = h.paths.hook_sock();
    let both = |event: &str, reason: Option<&str>, body: &str| {
        hook_send_road(&sock, &s, event, reason, None, &mod_body(event, body), Some("mod"));
        hook_send_road(&sock, &s, event, reason, None, body, None);
    };
    both("SessionStart", Some("startup"), r#"{"source":"startup","session_id":"x"}"#);
    both("UserPromptSubmit", None, r#"{"prompt":"go"}"#);
    both(
        "PreToolUse",
        None,
        r#"{"session_id":"x","cwd":"/r","tool_name":"AskUserQuestion","tool_use_id":"t1","tool_input":{"questions":[]}}"#,
    );
    both("PostToolUse", None, r#"{"tool_name":"Read","tool_use_id":"t2"}"#);
    both("Stop", None, r#"{"stop_hook_active":false}"#);
    // Events the mod never relays are never expected from it.
    hook_send_road(&sock, &s, "GateDenied", Some("board"), None, "{}", None);
    std::thread::sleep(Duration::from_millis(2_600));
    assert!(disagreements(&h).is_empty(), "{:?}", disagreements(&h));

    // One frame per road without a twin, and one event told two ways.
    hook_send_road(&sock, &s, "Stop", None, None, r#"{"stop_hook_active":true}"#, None);
    hook_send_road(&sock, &s, "SubagentStop", None, None, "{}", Some("mod"));
    hook_send_road(&sock, &s, "Notification", None, None, r#"{"n":1}"#, None);
    hook_send_road(&sock, &s, "Notification", None, None, r#"{"n":2}"#, Some("mod"));
    wait_until(Duration::from_secs(10), "the shadow's three lines", || {
        disagreements(&h).len() >= 3
    });
    std::thread::sleep(Duration::from_millis(600));
    let mut lines: Vec<(String, String, u64)> = disagreements(&h)
        .iter()
        .map(|l| {
            (
                l["cmd"].as_str().unwrap().to_string(),
                l["outcome"].as_str().unwrap().to_string(),
                l["count"].as_u64().unwrap(),
            )
        })
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("road_disagree:Notification".into(), "differs".into(), 1),
            ("road_disagree:Stop".into(), "no_mod_twin".into(), 1),
            ("road_disagree:SubagentStop".into(), "no_hooks_twin".into(), 1),
        ]
    );
    assert!(disagreements(&h).iter().all(|l| l["session"] == s.as_str()));
}
