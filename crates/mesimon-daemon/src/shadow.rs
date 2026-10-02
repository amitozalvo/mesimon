//! Shadow mode (T-574): the mod's frames held against the hook set's.
//!
//! Under the mod road both roads report every hook-set event: Claude Code
//! runs the generated settings hook and, in-process, the mod relays the same
//! event through `mesimon hook --road mod`. The daemon trusts the hook set's
//! frame and ingests it as ever; the mod's is only compared. T-573 measured
//! the two byte-identical for every event but `PreToolUse`, whose mod side is
//! the tool envelope, so both are read through the same projection there.
//!
//! Pure and time-injected: frames are stamped when the hook socket accepted
//! them and swept against the time the tick was SENT, so a writer stall that
//! holds a twin in the channel cannot make it look late.

use std::collections::{HashMap, VecDeque};

use mesimon_core::road::{pre_tool_use_projection, Road};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// How long a frame waits for its twin before the shadow says so.
pub const TWIN_WINDOW_MS: u64 = 2_000;
/// How many unpaired frames the shadow holds at once; past it the oldest go
/// unreported (a mod that never loaded fills it in seconds on a busy turn).
pub const PENDING_CAP: usize = 4_096;
/// One line per session, event and outcome per minute, carrying the count
/// since the last: a mod that never loaded must not fill the feed.
pub const REPORT_EVERY_MS: u64 = 60_000;

/// What a pair must agree on besides the payload: the session, the event,
/// and the registration that fired (`--reason`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub session: uuid::Uuid,
    pub event: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Both roads reported the event, with different payloads.
    Differs,
    /// The hook set reported it and the mod did not.
    NoModTwin,
    /// The mod reported it and the hook set did not.
    NoHooksTwin,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Differs => "differs",
            Outcome::NoModTwin => "no_mod_twin",
            Outcome::NoHooksTwin => "no_hooks_twin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disagreement {
    pub session: uuid::Uuid,
    pub event: String,
    pub outcome: Outcome,
    pub count: u32,
}

struct Pending {
    key: Key,
    road: Road,
    digest: [u8; 32],
    at: u64,
}

#[derive(Default)]
struct Tally {
    last_line: Option<u64>,
    unreported: u32,
}

#[derive(Default)]
pub struct Shadow {
    pending: VecDeque<Pending>,
    tallies: HashMap<(uuid::Uuid, String, Outcome), Tally>,
}

/// What a frame is compared by: the payload with its keys sorted, hashed, or
/// for `PreToolUse` the three fields both roads carry.
pub fn digest(event: &str, payload: &Value) -> [u8; 32] {
    let value =
        if event == "PreToolUse" { pre_tool_use_projection(payload) } else { payload.clone() };
    let mut text = String::new();
    canonical(&value, &mut text);
    Sha256::digest(text.as_bytes()).into()
}

/// JSON with every object's keys in order, whatever order the writer used.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                canonical(&map[k], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(v, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

impl Shadow {
    /// A frame arrived. Its twin, if already waiting, is consumed with it;
    /// otherwise it waits for one.
    pub fn offer(&mut self, key: Key, road: Road, digest: [u8; 32], at: u64) {
        if let Some(i) =
            self.pending.iter().position(|p| p.road != road && p.key == key && p.digest == digest)
        {
            self.pending.remove(i);
            return;
        }
        if self.pending.len() >= PENDING_CAP {
            self.pending.pop_front();
        }
        self.pending.push_back(Pending { key, road, digest, at });
    }

    /// The frames that waited out the window, as the lines the feed is owed
    /// now. `now` is the tick's send time.
    pub fn sweep(&mut self, now: u64) -> Vec<Disagreement> {
        let mut expired = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if now.saturating_sub(self.pending[i].at) >= TWIN_WINDOW_MS {
                if let Some(p) = self.pending.remove(i) {
                    expired.push(p);
                }
            } else {
                i += 1;
            }
        }
        let mut expired: VecDeque<Pending> = expired.into();
        while let Some(p) = expired.pop_front() {
            // A frame of the other road for the same event, expired too or
            // still waiting: the two are one event the roads told
            // differently.
            let other = match expired.iter().position(|q| q.road != p.road && q.key == p.key) {
                Some(j) => expired.remove(j),
                None => self
                    .pending
                    .iter()
                    .position(|q| q.road != p.road && q.key == p.key)
                    .and_then(|j| self.pending.remove(j)),
            };
            let outcome = match (other, p.road) {
                (Some(_), _) => Outcome::Differs,
                (None, Road::Hooks) => Outcome::NoModTwin,
                (None, Road::Mod) => Outcome::NoHooksTwin,
            };
            self.tallies.entry((p.key.session, p.key.event, outcome)).or_default().unreported += 1;
        }
        let mut lines = Vec::new();
        self.tallies.retain(|(session, event, outcome), tally| {
            let due = tally.last_line.is_none_or(|t| now.saturating_sub(t) >= REPORT_EVERY_MS);
            if tally.unreported > 0 && due {
                lines.push(Disagreement {
                    session: *session,
                    event: event.clone(),
                    outcome: *outcome,
                    count: tally.unreported,
                });
                tally.unreported = 0;
                tally.last_line = Some(now);
            }
            // A quiet key is forgotten once its minute is up.
            tally.unreported > 0
                || tally.last_line.is_some_and(|t| now.saturating_sub(t) < REPORT_EVERY_MS)
        });
        lines.sort_by(|a, b| (a.session, &a.event).cmp(&(b.session, &b.event)));
        lines
    }

    /// A session went away: nothing of it is waiting any more.
    pub fn forget(&mut self, session: uuid::Uuid) {
        self.pending.retain(|p| p.key.session != session);
        self.tallies.retain(|(s, _, _), _| *s != session);
    }

    #[cfg(test)]
    fn waiting(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(event: &str) -> Key {
        Key { session: uuid::Uuid::nil(), event: event.into(), reason: None }
    }

    #[test]
    fn twins_pair_in_either_order_and_leave_nothing() {
        let mut s = Shadow::default();
        let d = digest("Stop", &json!({"stop_hook_active": false}));
        s.offer(key("Stop"), Road::Mod, d, 0);
        s.offer(key("Stop"), Road::Hooks, d, 40);
        s.offer(key("Stop"), Road::Hooks, d, 100);
        s.offer(key("Stop"), Road::Mod, d, 160);
        assert_eq!(s.waiting(), 0);
        assert!(s.sweep(10_000).is_empty());
    }

    #[test]
    fn parallel_frames_of_one_event_pair_by_payload_not_by_order() {
        let mut s = Shadow::default();
        let a = digest("PostToolUse", &json!({"tool_use_id": "a"}));
        let b = digest("PostToolUse", &json!({"tool_use_id": "b"}));
        s.offer(key("PostToolUse"), Road::Hooks, a, 0);
        s.offer(key("PostToolUse"), Road::Hooks, b, 1);
        s.offer(key("PostToolUse"), Road::Mod, b, 2);
        s.offer(key("PostToolUse"), Road::Mod, a, 3);
        assert_eq!(s.waiting(), 0);
    }

    #[test]
    fn key_order_does_not_matter_and_the_reason_does() {
        assert_eq!(digest("Stop", &json!({"a": 1, "b": [2, {"c": 3, "d": 4}]})), {
            let v: Value = serde_json::from_str(r#"{"b":[2,{"d":4,"c":3}],"a":1}"#).unwrap();
            digest("Stop", &v)
        });
        let mut s = Shadow::default();
        let d = digest("SessionStart", &json!({}));
        let startup = Key { reason: Some("startup".into()), ..key("SessionStart") };
        let resume = Key { reason: Some("resume".into()), ..key("SessionStart") };
        s.offer(startup, Road::Hooks, d, 0);
        s.offer(resume, Road::Mod, d, 0);
        assert_eq!(s.waiting(), 2);
    }

    #[test]
    fn pre_tool_use_is_compared_through_the_projection() {
        let hook = json!({"session_id": "s", "cwd": "/c", "tool_name": "AskUserQuestion",
            "tool_use_id": "t1", "tool_input": {"questions": []}});
        let modded = json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion",
            "tool_use_id": "t1", "tool_input": {"questions": []}});
        assert_eq!(digest("PreToolUse", &hook), digest("PreToolUse", &modded));
        // Elsewhere every field counts.
        assert_ne!(digest("PostToolUse", &hook), digest("PostToolUse", &modded));
    }

    #[test]
    fn a_lone_frame_is_reported_by_the_road_that_missed_it() {
        let mut s = Shadow::default();
        let d = digest("Stop", &json!({}));
        s.offer(key("Stop"), Road::Hooks, d, 0);
        s.offer(key("SubagentStop"), Road::Mod, d, 0);
        assert!(s.sweep(1_999).is_empty(), "the window is two seconds");
        let lines = s.sweep(2_000);
        let words: Vec<_> = lines.iter().map(|l| (l.event.as_str(), l.outcome)).collect();
        assert_eq!(
            words,
            vec![("Stop", Outcome::NoModTwin), ("SubagentStop", Outcome::NoHooksTwin)]
        );
        assert!(lines.iter().all(|l| l.count == 1));
    }

    #[test]
    fn two_payloads_for_one_event_are_one_differs() {
        let mut s = Shadow::default();
        s.offer(key("Notification"), Road::Hooks, digest("Notification", &json!({"a": 1})), 0);
        s.offer(key("Notification"), Road::Mod, digest("Notification", &json!({"a": 2})), 10);
        let lines = s.sweep(5_000);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].outcome, Outcome::Differs);
        assert_eq!(s.waiting(), 0);
    }

    #[test]
    fn a_twin_queued_before_the_tick_is_never_late_whatever_the_writer_did() {
        // Accepted at 1 500 and 1 700, the tick sent at 1 600: processed
        // ten seconds later in a stalled writer, the sweep still runs at the
        // tick's own time and the twin arrives behind it in the channel.
        let mut s = Shadow::default();
        let d = digest("Stop", &json!({}));
        s.offer(key("Stop"), Road::Hooks, d, 1_500);
        assert!(s.sweep(1_600).is_empty());
        s.offer(key("Stop"), Road::Mod, d, 1_700);
        assert!(s.sweep(4_000).is_empty());
    }

    #[test]
    fn a_flood_is_one_line_a_minute_with_its_count() {
        let mut s = Shadow::default();
        let d = digest("PostToolUse", &json!({}));
        for i in 0..5 {
            s.offer(key("PostToolUse"), Road::Hooks, d, i * 100);
        }
        let first = s.sweep(3_000);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].count, 5);
        for i in 0..3 {
            s.offer(key("PostToolUse"), Road::Hooks, d, 10_000 + i);
        }
        assert!(s.sweep(20_000).is_empty(), "held until the minute is up");
        let later = s.sweep(63_000);
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].count, 3);
        assert!(s.sweep(200_000).is_empty());
    }

    #[test]
    fn the_cap_drops_the_oldest_and_forget_drops_a_session() {
        let mut s = Shadow::default();
        let d = digest("Stop", &json!({}));
        for i in 0..(PENDING_CAP as u64 + 10) {
            s.offer(key("Stop"), Road::Hooks, d, i);
        }
        assert_eq!(s.waiting(), PENDING_CAP);
        s.forget(uuid::Uuid::nil());
        assert_eq!(s.waiting(), 0);
    }
}
