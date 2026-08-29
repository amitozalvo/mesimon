use serde::{Deserialize, Serialize};

/// Who is asking. D32c invariant 1: every mutation carries a principal.
/// v0.1 has exactly two inhabitants; the enum is open for v0.2 (`member:<id>`, `guest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    /// The local user, on the machine the daemon runs on.
    Local,
    /// An agent session, identified by the mesimon-minted session UUID.
    Agent { session: uuid::Uuid },
}
