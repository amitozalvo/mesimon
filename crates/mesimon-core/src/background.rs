//! Transient task liveness. No history is restored after a daemon restart.
use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Liveness {
    #[default]
    None,
    Monitoring,
    Working,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// A tool result named the task: the agent armed it on purpose. The
    /// tool's classification then stands against every later row (`Task::armed`).
    Started,
    Updated,
    Completed,
    /// A `Stop` snapshot row. Claude Code lists what is in flight, so a row
    /// is a start — except a `monitor` nobody armed (see [`is_monitor_kind`]).
    Listed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Task {
    liveness: Liveness,
    owner: Option<String>,
    /// A tool result armed it ([`Transition::Started`]). Its classification
    /// then outranks every payload label that follows: Claude Code spells
    /// the Bash tool's background commands and the Monitor tool's watches
    /// both `shell`, so only the tool that started a task can say which it
    /// is. A status still ends it.
    armed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    tasks: BTreeMap<String, Task>,
}

pub fn is_live_status(status: &str) -> bool {
    !matches!(status, "idle" | "completed" | "failed" | "stopped" | "cancelled" | "interrupted")
}

/// The kind a command the LEAD's Bash tool put in the background is armed
/// with (T-483) — `run_in_background`, or a Ctrl+B on a running command. It
/// is work: Claude Code wakes the lead when it ends, as it does for a
/// background agent, and while it runs the machine is busy on the agent's
/// behalf. The payload's `shell` label for the same task would read it as a
/// watch, which released the keep-awake hold under a running `release.sh`.
pub const BACKGROUND_COMMAND: &str = "background_command";

pub fn classify(kind: Option<&str>) -> Liveness {
    if kind == Some(BACKGROUND_COMMAND) {
        return Liveness::Working;
    }
    let norm: String = kind
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if norm.contains("monitor")
        || matches!(norm.as_str(), "shell" | "localbash" | "backgroundshell")
    {
        Liveness::Monitoring
    } else {
        // Missing and future task types conservatively count as agent work.
        Liveness::Working
    }
}

/// Claude Code spells two task families `monitor` in a hook payload: the
/// watches an agent arms through the Monitor tool, and the ambient
/// housekeeping it keeps for itself — live updates for a published artifact,
/// a comment thread, presence on a page, a plugin monitor — which its own
/// tasks panel hides and which lives as long as the session does. The payload
/// drops the `ambient` flag, so the two are told apart by provenance: a
/// Monitor tool result registers its task id, and a `monitor` row a `Stop`
/// lists without one is ambient and counts for nothing (T-408: a session
/// that had published an artifact read "monitoring" after every turn, for
/// good). A `shell` row keeps its snapshot classification unless a tool armed
/// it: the Monitor tool's watches are shells, and so are the Bash tool's
/// background commands ([`BACKGROUND_COMMAND`]).
pub fn is_monitor_kind(kind: Option<&str>) -> bool {
    kind.unwrap_or_default().to_ascii_lowercase().contains("monitor")
}

impl Registry {
    pub fn clear(&mut self) {
        self.tasks.clear();
    }

    /// How many tasks are in flight: what an idle agent left running, said
    /// on the card and to the crown (T-599).
    pub fn count(&self) -> usize {
        self.tasks.len()
    }

    pub fn liveness(&self) -> Liveness {
        if self.tasks.values().any(|t| t.liveness == Liveness::Working) {
            Liveness::Working
        } else if self.tasks.is_empty() {
            Liveness::None
        } else {
            Liveness::Monitoring
        }
    }

    /// Replace this owner's snapshot, retaining descendants owned by another
    /// agent: children may outlive their parent. Snapshot rows are then
    /// recorded as [`Transition::Listed`]: starts, because Stop explicitly
    /// lists in-flight tasks, save for the ambient `monitor` rows.
    pub fn retain_snapshot(&mut self, owner: Option<&str>, ids: &[&str]) {
        self.tasks.retain(|id, task| task.owner.as_deref() != owner || ids.contains(&id.as_str()));
    }

    pub fn record(
        &mut self,
        id: &str,
        kind: Option<&str>,
        status: Option<&str>,
        transition: Transition,
        owner: Option<&str>,
    ) {
        if id.is_empty() || id.len() > 128 {
            return;
        }
        let classification = classify(kind);
        let known = self.tasks.get(id).map(|t| (t.liveness, t.armed));
        if transition == Transition::Completed || status.is_some_and(|s| !is_live_status(s)) {
            self.tasks.remove(id);
            return;
        }
        // A nested snapshot is the whole session's: Claude Code fills a
        // `SubagentStop`'s `background_tasks` from the same registry as the
        // lead's `Stop`, so its rows name the lead's tasks too, and `owner`
        // says whose stop it was, not whose task. It may still introduce a
        // nested agent's child (below), but it never takes back a task this
        // registry already holds (T-483: an internal agent stopping four
        // seconds after the lead dropped the lead's running `release.sh`,
        // and the card fell from monitoring to idle).
        if transition == Transition::Listed && owner.is_some() && known.is_some() {
            return;
        }
        if owner.is_some() && (kind.is_none() || classification == Liveness::Monitoring) {
            self.tasks.remove(id);
            return;
        }
        if transition == Transition::Updated && status.is_none() && known.is_none() {
            return;
        }
        if transition == Transition::Listed && is_monitor_kind(kind) && known.is_none() {
            // Ambient housekeeping (`is_monitor_kind`): no tool armed it.
            return;
        }
        let task = match known {
            Some((liveness, true)) if transition != Transition::Started => {
                Task { liveness, owner: owner.map(str::to_string), armed: true }
            }
            _ => Task {
                liveness: classification,
                owner: owner.map(str::to_string),
                armed: transition == Transition::Started,
            },
        };
        if self.tasks.len() < 1024 || known.is_some() {
            self.tasks.insert(id.to_string(), task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_serialization_drops_all_task_evidence() {
        use crate::board::{SessionKind, SessionRecord, SessionState};
        let mut session = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec![],
            "/tmp".into(),
            SessionState::Running,
        );
        session.background_tasks.record(
            "child",
            Some("subagent"),
            None,
            Transition::Started,
            Some("parent"),
        );
        let encoded = serde_json::to_string(&session).unwrap();
        assert!(!encoded.contains("background_tasks"));
        let restored: SessionRecord = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.background_tasks.liveness(), Liveness::None);
    }

    #[test]
    fn each_transition_reclassifies_and_progress_cannot_revive() {
        let mut r = Registry::default();
        // Listed, not Started: an armed task keeps its tool's classification.
        r.record("a", None, None, Transition::Listed, None);
        assert_eq!(r.liveness(), Liveness::Working);
        r.record("a", Some("shell"), None, Transition::Updated, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        r.record("a", Some("subagent"), Some("idle"), Transition::Updated, None);
        r.record("a", Some("subagent"), None, Transition::Updated, None);
        assert_eq!(r.liveness(), Liveness::None);
        r.record("a", Some("subagent"), Some("running"), Transition::Updated, None);
        assert_eq!(r.liveness(), Liveness::Working);
        for status in ["completed", "failed", "stopped", "cancelled", "interrupted", "idle"] {
            r.record("a", Some("subagent"), None, Transition::Started, None);
            r.record("a", None, Some(status), Transition::Updated, None);
            assert_eq!(r.liveness(), Liveness::None);
        }
    }

    #[test]
    fn nested_agents_outlive_parents_but_internal_shells_do_not_count() {
        let mut r = Registry::default();
        r.record("parent", Some("subagent"), None, Transition::Started, None);
        r.record("child", Some("subagent"), None, Transition::Started, Some("parent"));
        r.record("shell", Some("shell"), None, Transition::Started, Some("parent"));
        r.retain_snapshot(None, &[]);
        assert_eq!(r.liveness(), Liveness::Working);
        r.record("child", None, None, Transition::Completed, Some("parent"));
        assert_eq!(r.liveness(), Liveness::None);
    }

    /// The Bash tool's background command is work, and the payload's `shell`
    /// label for it — on a Stop row or a TaskOutput — cannot demote it to a
    /// watch (T-483). A status still ends it, and so does a snapshot that no
    /// longer lists it.
    #[test]
    fn an_armed_command_outranks_its_shell_label() {
        let mut r = Registry::default();
        r.record("cmd", Some(BACKGROUND_COMMAND), None, Transition::Started, None);
        assert_eq!(r.liveness(), Liveness::Working);
        r.record("cmd", Some("shell"), Some("running"), Transition::Listed, None);
        r.record("cmd", Some("local_bash"), Some("running"), Transition::Updated, None);
        assert_eq!(r.liveness(), Liveness::Working);
        // The same row with no tool behind it is a watch, as before.
        r.record("bare", Some("shell"), Some("running"), Transition::Listed, None);
        r.record("cmd", Some("local_bash"), Some("completed"), Transition::Updated, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        r.record("cmd", Some(BACKGROUND_COMMAND), None, Transition::Started, None);
        r.retain_snapshot(None, &["bare"]);
        assert_eq!(r.liveness(), Liveness::Monitoring);
    }

    /// A nested snapshot lists the whole session's tasks, so it never takes
    /// back one the registry holds — the lead's shell, armed or not — while a
    /// nested shell it introduces still does not count (T-483).
    #[test]
    fn a_nested_snapshot_keeps_the_leads_tasks() {
        let mut r = Registry::default();
        r.record("watch", Some("shell"), Some("running"), Transition::Listed, None);
        r.record("cmd", Some(BACKGROUND_COMMAND), None, Transition::Started, None);
        for id in ["watch", "cmd", "theirs"] {
            r.record(id, Some("shell"), Some("running"), Transition::Listed, Some("helper"));
        }
        assert_eq!(r.liveness(), Liveness::Working);
        r.record("cmd", None, None, Transition::Completed, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        // A terminal status is still the end, whoever's stop reports it.
        r.record("watch", Some("shell"), Some("completed"), Transition::Listed, Some("helper"));
        assert_eq!(r.liveness(), Liveness::None);
    }

    /// A `monitor` row a Stop lists is ambient unless the Monitor tool armed
    /// it (T-408); a listed shell is a start either way.
    #[test]
    fn a_listed_monitor_counts_only_with_tool_provenance() {
        let mut r = Registry::default();
        r.record("art", Some("monitor"), Some("running"), Transition::Listed, None);
        assert_eq!(r.liveness(), Liveness::None);
        r.record("watch", Some("shell"), Some("running"), Transition::Listed, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        r.retain_snapshot(None, &[]);
        assert_eq!(r.liveness(), Liveness::None);
        // The Monitor tool result names the task first; the snapshot then keeps it.
        r.record("armed", Some("monitor"), None, Transition::Started, None);
        r.record("armed", Some("monitor"), Some("running"), Transition::Listed, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        // An ambient row beside it changes nothing.
        r.record("art", Some("monitor"), Some("running"), Transition::Listed, None);
        assert_eq!(r.liveness(), Liveness::Monitoring);
        r.record("armed", None, None, Transition::Completed, None);
        assert_eq!(r.liveness(), Liveness::None);
    }
}
