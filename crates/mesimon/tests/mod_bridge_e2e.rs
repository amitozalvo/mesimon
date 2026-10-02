//! The mod road's foundation (T-574), end to end with the real binaries: a
//! Claude launch on the mod road, the real `mesimon mod-bridge` polling the
//! daemon from inside the pane, a ping that crosses the whole road and back
//! (daemon → bridge → the mod's stand-in → `mesimon hook --road mod` →
//! daemon), a daemon restart and a killed bridge with nothing lost, the
//! delivery ledger's rules, and the mod's frames as the session's only ones
//! (T-577: no hook set rides a mod launch).
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

/// The feed's `hook` lines for one session: (event, reason, road).
fn hook_lines(h: &Harness, session: &str) -> Vec<(String, Option<String>, Option<String>)> {
    std::fs::read_to_string(h.paths.activity_log())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|l| l["kind"] == "hook" && l["session"] == session)
        .map(|l| {
            let word = |k: &str| l[k].as_str().map(str::to_string);
            (word("event").unwrap_or_default(), word("reason"), word("road"))
        })
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
    // No hook set rides it (T-577): the mod relays every event, holds the
    // gate and serves the tools, so no settings file, no MCP server and no
    // allow rule; the pane carries the gate's roots and the tools' tier.
    for flag in ["--settings", "--mcp-config", "--allowedTools"] {
        assert!(!rec.argv.iter().any(|a| a == flag), "{flag}: {:?}", rec.argv);
    }
    assert!(!h.paths.state_dir.join("hooks").join(format!("{sid}.json")).exists());
    let start = pane_start(&h.paths.tmux_sock(), &rec);
    for var in [
        "MESIMON_MOD_GATE_BOARD=",
        "MESIMON_MOD_GATE_STATE=",
        "MESIMON_MOD_GATE_ALLOW=",
        "MESIMON_MOD_TOOLS=full",
    ] {
        assert!(start.contains(var), "{var}: {start}");
    }
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
fn the_mods_frames_are_the_sessions_and_the_hook_sets_are_not() {
    let Some(h) = boot("modbridge-frames") else { return };
    let mut c = h.client("frames");
    let (_, sid) = spawn(&mut c, "the frames", SessionKind::Claude);
    let s = sid.to_string();
    let sock = h.paths.hook_sock();
    let state =
        |c: &mut TestClient| c.board().sessions.into_iter().find(|r| r.id == sid).unwrap().state;
    let by_mod = |event: &str, reason: Option<&str>, body: &str| {
        hook_send_road(&sock, &s, event, reason, None, body, Some("mod"));
    };
    by_mod("SessionStart", Some("startup"), r#"{"source":"startup","session_id":"x"}"#);
    by_mod("UserPromptSubmit", None, r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(10), "working on the mod's frames", || {
        matches!(state(&mut c), mesimon_core::board::SessionState::Running)
    });
    // A hook-set frame for this pane came from nothing mesimon launched:
    // dropped, not ingested, not in the feed.
    hook_send_road(&sock, &s, "Stop", None, None, r#"{"stop_hook_active":false}"#, None);
    std::thread::sleep(Duration::from_millis(600));
    assert!(matches!(state(&mut c), mesimon_core::board::SessionState::Running));
    by_mod("Stop", None, r#"{"stop_hook_active":false}"#);
    wait_until(Duration::from_secs(10), "idle on the mod's Stop", || {
        matches!(state(&mut c), mesimon_core::board::SessionState::Idle { .. })
    });
    // The mod's refusal of a write reaches the feed, as the gate's did.
    by_mod("GateDenied", Some("board_dir"), r#"{"file_path":"/r/.mesimon/x"}"#);
    wait_until(Duration::from_secs(5), "the gate's line", || {
        hook_lines(&h, &s).iter().any(|(e, _, _)| e == "GateDenied")
    });
    let lines = hook_lines(&h, &s);
    let mod_line =
        |e: &str, r: Option<&str>| (e.to_string(), r.map(str::to_string), Some("mod".into()));
    assert_eq!(
        lines,
        vec![
            mod_line("SessionStart", Some("startup")),
            mod_line("UserPromptSubmit", None),
            mod_line("Stop", None),
            mod_line("GateDenied", Some("board_dir")),
        ]
    );
}

/// A mod whose reads of its pane variables failed before one succeeded says
/// so (T-594): a feed line on its ticket, and a journal line with the error.
/// A failed read was once kept for the process's life, and the mod relayed
/// nothing.
#[test]
fn a_load_failure_the_mod_reports_is_a_feed_line_on_its_ticket() {
    let Some(h) = boot("modbridge-loadfail") else { return };
    let mut c = h.client("loadfail");
    let (ticket, sid) = spawn(&mut c, "the load failure", SessionKind::Claude);
    let body = r#"{"reads":2,"error":"Error: the dispatch was abandoned","at":"SessionStart"}"#;
    hook_send_road(
        &h.paths.hook_sock(),
        &sid.to_string(),
        "ModLoadFailed",
        Some("recovered"),
        None,
        body,
        Some("mod"),
    );
    let line = || {
        std::fs::read_to_string(h.paths.activity_log())
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .find(|l| l["cmd"] == "mod_load_failed")
    };
    wait_until(Duration::from_secs(10), "the feed line", || line().is_some());
    let line = line().unwrap();
    assert_eq!(line["ticket"], ticket.to_string());
    let outcome = line["outcome"].as_str().unwrap();
    assert!(outcome.contains("2 read(s)") && outcome.contains("at SessionStart"), "{outcome}");
    assert!(outcome.contains("the dispatch was abandoned"), "{outcome}");
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap_or_default();
    assert!(journal.contains(&format!("mod of session {sid}: 2 read(s)")), "{journal}");
}

/// T-593: a park takes the pane at once, and the `SessionEnd` it causes
/// lands after it: Claude Code waits for the mod's awaited relay, so the
/// frame arrives once the record is already parked. On the mod road it is
/// the session's only `SessionEnd` (T-577): taken from a parked record, into
/// the feed by the mod, and the park stands.
#[test]
fn a_parks_session_end_lands_by_the_mod_after_the_pane_is_gone() {
    let Some(h) = boot("modbridge-park") else { return };
    let mut c = h.client("park");
    let (_, sid) = spawn(&mut c, "the park", SessionKind::Claude);
    let s = sid.to_string();
    let sock = h.paths.hook_sock();
    let by_mod = |event: &str, reason: Option<&str>, body: &str| {
        hook_send_road(&sock, &s, event, reason, None, body, Some("mod"));
    };
    by_mod("UserPromptSubmit", None, r#"{"prompt":"go"}"#);
    by_mod("Stop", None, "{}");
    c.await_state(sid, "idle", |s| matches!(s, mesimon_core::board::SessionState::Idle { .. }));
    assert!(matches!(c.request(Command::SleepSession { id: sid }), Response::Ok));
    c.await_state(sid, "parked", |s| *s == mesimon_core::board::SessionState::Sleeping);
    std::thread::sleep(Duration::from_millis(700));
    by_mod("SessionEnd", Some("other"), r#"{"reason":"other","session_id":"x"}"#);
    wait_until(Duration::from_secs(10), "the SessionEnd line", || {
        hook_lines(&h, &s).iter().any(|(e, _, _)| e == "SessionEnd")
    });
    assert!(hook_lines(&h, &s).contains(&(
        "SessionEnd".to_string(),
        Some("other".to_string()),
        Some("mod".to_string())
    )));
    std::thread::sleep(Duration::from_millis(500));
    let rec = c.board().sessions.into_iter().find(|r| r.id == sid).unwrap();
    assert_eq!(rec.state, mesimon_core::board::SessionState::Sleeping, "the park stands");
}
