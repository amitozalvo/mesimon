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

use mesimon_core::board::SessionKind;
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
    wait_until(Duration::from_secs(30), "the startup probe's verdict", || {
        road_json(&h).is_some_and(|v| v["road"] == "mod")
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
    wait_until(Duration::from_secs(30), "the startup probe's verdict", || {
        road_json(&h).is_some_and(|v| v["probe"] == "claude 2.1.286 is older than 2.1.287")
    });
    assert_eq!(road_json(&h).unwrap()["road"], "hooks");
    let lines = road_lines(&h);
    assert!(lines.iter().all(|l| l.contains("claude_road:hooks")), "{lines:?}");
    let rec = launch(&h);
    assert_eq!(rec.road, Road::Hooks);
    assert!(!rec.argv.iter().any(|a| a == "--plugin-dir"), "{:?}", rec.argv);
}
