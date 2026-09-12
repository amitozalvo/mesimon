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
    /// A person on another machine, editing a shared board (T-215). Minted
    /// only by the daemon's own sync when it applies a record the relay
    /// delivered and the record's signature verified against the member
    /// list; a client that sends it over the wire is refused. `member` is
    /// the display name the relay holds for the signing device.
    Remote { member: String },
}

impl Principal {
    /// Did a human ask for this? `Local` is a person pressing a key here and
    /// `Remote` is a person pressing one elsewhere. The ping-pong guard and
    /// the flap fuse restrain everything else.
    pub fn is_human(&self) -> bool {
        matches!(self, Principal::Local | Principal::Remote { .. })
    }

    /// The feed's actor word. Stable strings — the activity log is read by
    /// eye and grepped in tests.
    pub fn actor(&self) -> &str {
        match self {
            Principal::Local => "local",
            Principal::Agent { .. } => "agent",
            Principal::Automation { .. } => "automation",
            Principal::Remote { .. } => "remote",
        }
    }

    /// Who wrote a note (`NoteMeta::created_by` / `edited_by`): docs/13's
    /// `origin` vocabulary — `local` for a person, `agent:<session-uuid>` for
    /// an agent. The session id travels INSIDE the word so the file outlives
    /// the session record it names, and a reader that only wants "a person
    /// or an agent" still gets it from the prefix.
    pub fn note_author(&self) -> String {
        match self {
            Principal::Local => "local".into(),
            Principal::Agent { session } => format!("agent:{session}"),
            Principal::Automation { rule } => format!("automation:{rule}"),
            Principal::Remote { member } => format!("member:{member}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_is_human() {
        assert!(Principal::Local.is_human());
        assert!(Principal::Remote { member: "Dana".into() }.is_human());
        assert!(!Principal::Agent { session: uuid::Uuid::nil() }.is_human());
        assert!(!Principal::Automation { rule: "automove".into() }.is_human());
    }

    #[test]
    fn actors_are_stable_words() {
        assert_eq!(Principal::Local.actor(), "local");
        assert_eq!(Principal::Agent { session: uuid::Uuid::nil() }.actor(), "agent");
        assert_eq!(Principal::Automation { rule: "automove".into() }.actor(), "automation");
        assert_eq!(Principal::Remote { member: "Dana".into() }.actor(), "remote");
        assert_eq!(Principal::Remote { member: "Dana".into() }.note_author(), "member:Dana");
    }

    #[test]
    fn note_authors_carry_the_session() {
        assert_eq!(Principal::Local.note_author(), "local");
        assert_eq!(
            Principal::Agent { session: uuid::Uuid::nil() }.note_author(),
            "agent:00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn round_trips_on_the_wire() {
        for p in [
            Principal::Local,
            Principal::Agent { session: uuid::Uuid::nil() },
            Principal::Automation { rule: "automove".into() },
            Principal::Remote { member: "Dana".into() },
        ] {
            let s = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<Principal>(&s).unwrap(), p);
        }
    }
}
