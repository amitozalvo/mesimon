//! The subscription quota (T-327), end to end: a board that asks for
//! claude's quota gets the daemon to launch the claude CLI through the pane
//! launcher, ask it `get_usage` over the SDK's stream-json, and carry the
//! answer on the snapshot — and to write it to the machine's shared file,
//! where every other board's daemon reads it. What a board asked for goes
//! with its connection: nobody attached, nothing wanted, nothing launched.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::command::{Command, Response};
use mesimon_core::usage::{Usage, Wants};

/// A claude that, asked in print mode, answers one `get_usage` with the
/// author's shape of reading; launched any other way it is a pane that
/// sleeps. It notes each probe so the test can count launches.
const STUB: &str = r#"#!/bin/sh
if [ "$1" = "-p" ]; then
  echo probe >> "$HOME/probes"
  read request
  case "$request" in *get_usage*) ;; *) exit 3 ;; esac
  echo '{"type":"system","subtype":"init"}'
  echo '{"type":"control_response","response":{"subtype":"success","request_id":"mesimon-usage","response":{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"limits":[{"kind":"session","group":"session","percent":42,"resets_at":"2099-01-01T00:00:00Z","severity":"normal","is_active":false},{"kind":"weekly_scoped","group":"weekly","percent":64,"resets_at":"2099-01-03T08:00:00Z","severity":"warning","is_active":true,"scope":{"model":{"display_name":"Fable"}}}]}}}}'
  sleep 30
  exit 0
fi
exec sleep 300
"#;

fn usage_of(c: &mut TestClient) -> Usage {
    match c.request(Command::Snapshot) {
        Response::Board { usage, .. } => usage,
        other => panic!("not a board: {other:?}"),
    }
}

#[test]
fn a_board_that_asks_gets_claudes_quota_and_shares_it() {
    let Some(h) = Harness::boot("usage", Some(STUB)) else { return };
    let home = h.dir.join("home");
    let mut c = h.client("usage");

    // Nothing asked, nothing read: a daemon with no board reads no quota.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(usage_of(&mut c), Usage::default());
    assert!(!home.join("probes").exists(), "no probe before a board asks");

    let asked = c.request(Command::SetUsageWants { claude: true, codex: false });
    assert!(matches!(asked, Response::Ok), "{asked:?}");
    assert_eq!(usage_of(&mut c).wants, Wants { claude: true, codex: false });

    let mut reading = None;
    wait_until(Duration::from_secs(30), "claude's quota on the snapshot", || {
        reading = usage_of(&mut c).claude.reading;
        reading.is_some()
    });
    let reading = reading.unwrap();
    assert_eq!(reading.plan.as_deref(), Some("max"));
    let got: Vec<_> = reading.windows.iter().map(|w| (w.label.clone(), w.percent_word())).collect();
    assert_eq!(got, [("5h".to_string(), "42%".to_string()), ("Fable".into(), "64%".into())]);
    assert_eq!(reading.headline().map(|w| w.label.as_str()), Some("Fable"), "the warning");

    // The machine's file holds it for every other board's daemon.
    let shared = home.join(".local/state/mesimon/usage.json");
    let text = std::fs::read_to_string(&shared).expect("the shared file");
    assert!(text.contains("\"Fable\""), "{text}");
    assert_eq!(std::fs::read_to_string(home.join("probes")).unwrap().lines().count(), 1);

    // A fresh reading is not read again by the clock: well under the floor.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(std::fs::read_to_string(home.join("probes")).unwrap().lines().count(), 1);

    // What a board wanted leaves with it.
    drop(c);
    let mut other = h.client("usage-other");
    wait_until(Duration::from_secs(10), "the closed board's wants to go", || {
        usage_of(&mut other).wants == Wants::default()
    });
    // The reading itself stays: it is the machine's, not the board's.
    assert!(usage_of(&mut other).claude.reading.is_some());
}
