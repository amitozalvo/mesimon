use serde::{Deserialize, Serialize};

/// Who is asking. D32c invariant 1: every mutation carries a principal.
/// The enum stays open for v0.2 (`member:<id>`, `guest`).
///
/// The three inhabitants are three *different* answers to "why did the board
/// change", and keeping them apart is load-bearing (T-84). Before MCP,
/// `automove` borrowed `Agent`, which was the only principal that could stand
/// for "not the human". Once an agent can ask for a move itself, the two must
/// be told apart — otherwise `authorize()` cannot restrict one without
/// breaking the other, the activity feed cannot say who moved a card, and the
/// ping-pong guard has nothing to compare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    /// The local user, on the machine the daemon runs on.
    Local,
    /// An agent session asking over MCP, identified by the mesimon-minted
    /// session UUID. Never used for work the daemon does on a session's
    /// behalf — that is `Automation`.
    Agent { session: uuid::Uuid },
    /// The daemon's own rules acting without anyone asking: `automove` today,
    /// column on-enter actions in M5. `rule` names the rule for the feed.
    Automation { rule: String },
}

impl Principal {
    /// Did a human ask for this? Only `Local` is a person pressing a key.
    /// The ping-pong guard and the flap fuse restrain everything else.
    pub fn is_human(&self) -> bool {
        matches!(self, Principal::Local)
    }

    /// The feed's actor word. Stable strings — the activity log is read by
    /// eye and grepped in tests.
    pub fn actor(&self) -> &str {
        match self {
            Principal::Local => "local",
            Principal::Agent { .. } => "agent",
            Principal::Automation { .. } => "automation",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_is_human() {
        assert!(Principal::Local.is_human());
        assert!(!Principal::Agent { session: uuid::Uuid::nil() }.is_human());
        assert!(!Principal::Automation { rule: "automove".into() }.is_human());
    }

    #[test]
    fn actors_are_stable_words() {
        assert_eq!(Principal::Local.actor(), "local");
        assert_eq!(Principal::Agent { session: uuid::Uuid::nil() }.actor(), "agent");
        assert_eq!(Principal::Automation { rule: "automove".into() }.actor(), "automation");
    }

    #[test]
    fn round_trips_on_the_wire() {
        for p in [
            Principal::Local,
            Principal::Agent { session: uuid::Uuid::nil() },
            Principal::Automation { rule: "automove".into() },
        ] {
            let s = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Principal>(&s).unwrap(), p);
        }
    }
}
