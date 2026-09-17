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
    Started,
    Updated,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Task {
    liveness: Liveness,
    owner: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    tasks: BTreeMap<String, Task>,
}

pub fn is_live_status(status: &str) -> bool {
    !matches!(status, "idle" | "completed" | "failed" | "stopped" | "cancelled" | "interrupted")
}

pub fn classify(kind: Option<&str>) -> Liveness {
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

impl Registry {
    pub fn clear(&mut self) {
        self.tasks.clear();
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
    /// agent: children may outlive their parent. Status-free snapshot entries
    /// are starts because Stop explicitly lists in-flight tasks.
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
        if transition == Transition::Completed
            || status.is_some_and(|s| !is_live_status(s))
            || (owner.is_some() && (kind.is_none() || classification == Liveness::Monitoring))
        {
            self.tasks.remove(id);
            return;
        }
        if transition == Transition::Updated && status.is_none() && !self.tasks.contains_key(id) {
            return;
        }
        if self.tasks.len() < 1024 || self.tasks.contains_key(id) {
            self.tasks.insert(
                id.to_string(),
                Task { liveness: classification, owner: owner.map(str::to_string) },
            );
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
        r.record("a", None, None, Transition::Started, None);
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
}
