//! The road is `auto`, always (T-588): every Claude launch asks for the mod
//! where the Claude Code it would run is new enough and validates it, and
//! the probe that decides is warmed when the shell environment lands, before
//! any launch, so a fresh board's first agent already takes the mod.
//!
//! The seam names `auto` here only because `TestFixture` always sets one,
//! so a stub is never probed unasked; the stub plays Claude Code's two probe
//! commands (`--version`, `plugin validate`) and otherwise sleeps. It is
//! booted bare: a composer painted first would come before its version line.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{SessionKind, SessionState};
use mesimon_core::command::{Command, Response};
use mesimon_core::road::Road;

fn stub(version: &str) -> String {
    format!(
        "#!/bin/sh\ncase \"$1\" in\n  --version) echo '{version} (Claude Code)'; exit 0 ;;\n  \
         plugin) echo '✔ Validation passed'; exit 0 ;;\nesac\nexec sleep 300\n"
    )
}

fn boot(name: &str, version: &str) -> Option<Harness> {
    Harness::boot_bare(name, Some(&stub(version)), &[("MESIMON_CLAUDE_ROAD", "auto")])
}

fn road_json(h: &Harness) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(h.paths.state_dir.join("mod/road.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn road_lines(h: &Harness) -> Vec<String> {
    std::fs::read_to_string(h.paths.activity_log())
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("\"cmd\":\"claude_road"))
        .map(str::to_string)
        .collect()
}

fn launch(h: &Harness) -> mesimon_core::board::SessionRecord {
    let mut c = h.client("auto");
    assert!(c.board().sessions.is_empty(), "the probe ran before any launch");
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "an agent".into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets[0].id;
    let id = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    c.board().sessions.into_iter().find(|s| s.id == id).expect("the record")
}

#[test]
fn a_new_claude_code_is_probed_before_any_launch_and_launches_on_the_mod() {
    let Some(h) = boot("mod-auto-new", "2.1.287") else { return };
    // The verdict file is written at once; the feed line at the tick's end.
    wait_until(Duration::from_secs(30), "the startup probe's verdict and feed line", || {
        road_json(&h).is_some_and(|v| v["road"] == "mod") && !road_lines(&h).is_empty()
    });
    let v = road_json(&h).unwrap();
    assert_eq!(v["setting"], "auto");
    assert_eq!(v["probe"], "claude 2.1.287, the mod validated");
    let lines = road_lines(&h);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("claude_road:mod"), "{lines:?}");
    let rec = launch(&h);
    assert_eq!(rec.road, Road::Mod);
    assert!(rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
}

#[test]
fn a_claude_code_too_old_for_mods_launches_on_the_hook_set() {
    let Some(h) = boot("mod-auto-old", "2.1.286") else { return };
    wait_until(Duration::from_secs(30), "the startup probe's verdict and feed line", || {
        road_json(&h).is_some_and(|v| v["probe"] == "claude 2.1.286 is older than 2.1.287")
            && !road_lines(&h).is_empty()
    });
    assert_eq!(road_json(&h).unwrap()["road"], "hooks");
    let lines = road_lines(&h);
    assert!(lines.iter().all(|l| l.contains("claude_road:hooks")), "{lines:?}");
    let rec = launch(&h);
    assert_eq!(rec.road, Road::Hooks);
    assert!(!rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
}

/// A stand-in for a Claude Code whose mods are off (T-598): it answers the
/// probe as `stub` does (`plugin test` with `test`'s words), and as a
/// session paints its composer and keeps every line it reads. It loads no
/// mod, so on the mod road it reports nothing, as 2.1.288 did with its
/// remote flag off.
fn silent_stub(test: &str) -> String {
    format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  --version*) echo '2.1.288 (Claude Code)'; exit 0 ;;\n  \
         'plugin validate') echo '✔ Validation passed'; exit 0 ;;\n  \
         'plugin test') {test} ;;\nesac\n{COMPOSER}stty -icanon 2>/dev/null\n\
         while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n"
    )
}

const TEST_PASSES: &str = "echo ' 1 pass'; exit 0";
const TEST_MODS_OFF: &str = "echo 'claude plugin test: hooks modules are turned off in this \
     process: the rollout switch was saved off by an earlier session and is not refreshed yet.' \
     >&2; exit 1";

fn feed_has(h: &Harness, cmd: &str) -> bool {
    std::fs::read_to_string(h.paths.activity_log())
        .unwrap_or_default()
        .contains(&format!("\"cmd\":\"{cmd}\""))
}

fn probe_json(h: &Harness) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(h.paths.state_dir.join("mod/probe.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn ticket_with_brief(c: &mut TestClient, title: &str, brief: &str) -> ulid::Ulid {
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: brief.into() }),
        Response::NoteWritten { .. }
    ));
    ticket
}

fn composed(c: &mut TestClient, ticket: ulid::Ulid) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    }
}

fn record(c: &mut TestClient, sid: uuid::Uuid) -> mesimon_core::board::SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == sid).expect("the record")
}

/// T-598: Claude Code 2.1.288 turned mods off by a remote flag that
/// `claude plugin validate` cannot see. A launch on the mod alone then
/// reports nothing; under `auto` it is relaunched on the hook set, its words
/// kept for the new pane, the probe learns that mods are off, and the next
/// launch takes the hook set at once.
#[test]
fn a_mod_launch_that_never_reports_is_relaunched_on_the_hook_set() {
    let env = [("MESIMON_CLAUDE_ROAD", "auto"), ("MESIMON_MOD_BRIDGE_WAIT_MS", "1500")];
    let Some(h) = Harness::boot_bare("mod-auto-off", Some(&silent_stub(TEST_PASSES)), &env) else {
        return;
    };
    // The probe passes: this Claude Code's flag is still cached on.
    wait_until(Duration::from_secs(30), "the startup probe", || {
        road_json(&h).is_some_and(|v| v["road"] == "mod")
    });
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("auto-off");
    let ticket =
        ticket_with_brief(&mut c, "mods off", "## Brief\n\nmesimon-598-brief arrives once");
    let sid = composed(&mut c, ticket);
    let first = record(&mut c, sid);
    assert_eq!(first.road, Road::Mod);
    assert!(first.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", first.argv);
    assert!(!first.argv.iter().any(|a| a == "--settings"), "{:?}", first.argv);

    // No SessionStart and no bridge: the pane shows its composer, so the
    // relaunch comes a bridge wait after the launch, on the hook set.
    wait_until(Duration::from_secs(10), "the relaunch on the hook set", || {
        let rec = record(&mut c, sid);
        rec.road == Road::Hooks && rec.argv.iter().any(|a| a == "--settings")
    });
    let rec = record(&mut c, sid);
    assert!(!rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
    assert!(rec.argv.iter().any(|a| a == "--mcp-config"), "{:?}", rec.argv);
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.pending_submit, "the words are still owed");
    wait_until(Duration::from_secs(5), "the feed's two lines", || {
        feed_has(&h, "claude_road_relaunch") && feed_has(&h, "claude_road_fallback")
    });
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap_or_default();
    assert!(journal.contains("relaunched on the hook set"), "{journal}");
    let probe = probe_json(&h).expect("probe.json");
    assert_eq!(probe["verdict"], "mods_off", "{probe}");
    assert_eq!(probe["version"], "2.1.288", "{probe}");
    let road = road_json(&h).unwrap();
    assert_eq!(road["road"], "hooks");
    assert_eq!(road["mods_off"], true, "{road}");
    assert!(road["probe"].as_str().unwrap().contains("mods are off"), "{road}");

    // The hook set's SessionStart arms the words, which go once.
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-598.jsonl","cwd":"/tmp"}"#,
    );
    let got = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    wait_until(Duration::from_secs(10), "the brief by paste", || {
        got().contains("mesimon-598-brief")
    });
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the ack", || !record(&mut c, sid).pending_submit);
    assert_eq!(got().matches("mesimon-598-brief").count(), 1, "{}", got());

    // The next launch takes the hook set at once.
    let other = ticket_with_brief(&mut c, "the next one", "## Brief\n\nmesimon-598-next");
    let next = composed(&mut c, other);
    let rec = record(&mut c, next);
    assert_eq!(rec.road, Road::Hooks);
    assert!(rec.argv.iter().any(|a| a == "--settings"), "{:?}", rec.argv);
    let _ = c.request(Command::KillSession { id: sid });
    let _ = c.request(Command::KillSession { id: next });
}

/// T-650: a Team or Enterprise account. Claude Code seats
/// `cc-plugin-sec-default` outermost there, whose `classic.*` hook hands
/// every classic event past a person's plugins: the mod loads, its bridge
/// polls and its tools serve, and `SessionStart`, `UserPromptSubmit` and
/// `Stop` never reach it. The probe passes (nothing in `claude plugin test`
/// sees the seating), so under `auto` the launch is judged by its silence:
/// a bridge that polled with no `SessionStart` a bridge wait later is
/// relaunched with the hook set beside its mod, the words kept for the new
/// pane and sent down the mod once the hook set's `SessionStart` arms them,
/// the probe learns that the hook events do not reach the mod, and the next
/// launch carries both at once.
#[test]
fn a_mod_launch_whose_bridge_polls_and_hears_no_session_start_is_relaunched_with_the_hook_set() {
    let env = [
        ("MESIMON_CLAUDE_ROAD", "auto"),
        ("MESIMON_MOD_BRIDGE_WAIT_MS", "1500"),
        // The stand-in engine beside the stub: it brings the bridge up and
        // relays the mod's own reports, and no classic event (a test sends
        // those itself, and this one does not until the relaunch).
        (FAKE_MOD, "1"),
    ];
    let Some(h) = Harness::boot_bare("mod-auto-deaf", Some(&silent_stub(TEST_PASSES)), &env) else {
        return;
    };
    wait_until(Duration::from_secs(30), "the startup probe", || {
        road_json(&h).is_some_and(|v| v["road"] == "mod")
    });
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("auto-deaf");
    let ticket =
        ticket_with_brief(&mut c, "deaf mod", "## Brief\n\nmesimon-650-brief arrives once");
    let sid = composed(&mut c, ticket);
    let first = record(&mut c, sid);
    assert_eq!(first.road, Road::Mod);
    assert!(first.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", first.argv);
    assert!(!first.argv.iter().any(|a| a == "--settings"), "{:?}", first.argv);

    // The bridge polls (the mod is up) and no SessionStart follows: a bridge
    // wait after that first poll the launch is relaunched with the hook set
    // beside its mod, and the journal's reason says the bridge had polled.
    wait_until(Duration::from_secs(10), "the relaunch with the hook set", || {
        record(&mut c, sid).argv.iter().any(|a| a == "--settings")
    });
    let rec = record(&mut c, sid);
    assert_eq!(rec.road, Road::Mod);
    assert!(rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
    assert!(
        !rec.argv.iter().any(|a| a == "--mcp-config"),
        "the tools stay the mod's: {:?}",
        rec.argv
    );
    assert_eq!(rec.state, SessionState::Spawning);
    assert!(rec.pending_submit, "the words are still owed");
    wait_until(Duration::from_secs(5), "the feed's two lines", || {
        feed_has(&h, "claude_road_relaunch") && feed_has(&h, "claude_road_fallback")
    });
    let journal = std::fs::read_to_string(h.paths.daemon_log()).unwrap_or_default();
    assert!(journal.contains("after its bridge first polled"), "{journal}");
    assert!(journal.contains("relaunched with the hook set beside its mod"), "{journal}");
    let probe = probe_json(&h).expect("probe.json");
    assert_eq!(probe["verdict"], "classic_off", "{probe}");
    assert_eq!(probe["version"], "2.1.288", "{probe}");
    let road = road_json(&h).unwrap();
    assert_eq!(road["road"], "mod", "{road}");
    assert_eq!(road["mods_off"], true, "{road}");
    assert!(road["probe"].as_str().unwrap().contains("hook events do not reach the mod"), "{road}");

    // The new pane's mod comes up too; the hook set's SessionStart arms the
    // words, which go down the mod's `submit`, whole and once, and the
    // mod's `entered` clears the mark before the hook set's ack.
    wait_until(Duration::from_secs(10), "the new pane's bridge", || {
        matches!(c.request(Command::ModPing { session: sid }), Response::ModPonged { .. })
    });
    hook_send_with(
        &hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-650.jsonl","cwd":"/tmp"}"#,
    );
    let got = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    wait_until(Duration::from_secs(10), "the brief down the mod", || {
        got().contains("mesimon-650-brief")
    });
    let frames = std::fs::read_to_string(h.dir.join(format!("mod-{sid}.ndjson"))).unwrap();
    assert!(frames.contains("\"kind\":\"submit\""), "{frames}");
    wait_until(Duration::from_secs(5), "the mod's entered clears the mark", || {
        record(&mut c, sid).unsent.is_none()
    });
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the ack", || !record(&mut c, sid).pending_submit);
    assert_eq!(got().matches("mesimon-650-brief").count(), 1, "{}", got());

    // The next launch carries both at once.
    let other = ticket_with_brief(&mut c, "the next one", "## Brief\n\nmesimon-650-next");
    let next = composed(&mut c, other);
    let rec = record(&mut c, next);
    assert_eq!(rec.road, Road::Mod);
    assert!(rec.argv.iter().any(|a| a == "--settings"), "{:?}", rec.argv);
    assert!(rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
    let _ = c.request(Command::KillSession { id: sid });
    let _ = c.request(Command::KillSession { id: next });
}

/// T-598: the probe's load step. A Claude Code that refuses `claude plugin
/// test` in the flag's words has mods off, and every launch takes the hook
/// set from the first.
#[test]
fn a_probe_that_finds_mods_off_launches_on_the_hook_set() {
    let stub = silent_stub(TEST_MODS_OFF);
    let env = [("MESIMON_CLAUDE_ROAD", "auto")];
    let Some(h) = Harness::boot_bare("mod-auto-probe-off", Some(&stub), &env) else { return };
    wait_until(Duration::from_secs(30), "the startup probe's verdict", || {
        road_json(&h).is_some_and(|v| v["mods_off"] == true)
    });
    let v = road_json(&h).unwrap();
    assert_eq!(v["road"], "hooks");
    assert!(v["probe"].as_str().unwrap().starts_with("claude 2.1.288: mods are off"), "{v}");
    assert_eq!(probe_json(&h).unwrap()["verdict"], "mods_off");
    let rec = launch(&h);
    assert_eq!(rec.road, Road::Hooks);
    assert!(!rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
}
