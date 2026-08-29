use crate::Principal;

/// What is being attempted. Grows with the command set; every daemon mutation
/// path constructs one of these before acting (D32c invariant 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Read,
    Mutate,
}

/// What it is being attempted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    Board,
    Ticket { id: ulid::Ulid },
    Column { name: String },
    Session { id: uuid::Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

/// The single authorization chokepoint (D32c invariant 2).
///
/// v0.1: returns `Allow` for every principal — enforcement lives upstream in the
/// MCP tier restrictions (D10). The point of this function existing now is that
/// every call site already routes through it, so v0.2 changes one function.
pub fn authorize(_principal: &Principal, _action: &Action, _resource: &Resource) -> Decision {
    Decision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v01_allows_local() {
        let d = authorize(&Principal::Local, &Action::Mutate, &Resource::Board);
        assert_eq!(d, Decision::Allow);
    }

    #[test]
    fn v01_allows_agent() {
        let d = authorize(
            &Principal::Agent {
                session: uuid::Uuid::new_v4(),
            },
            &Action::Read,
            &Resource::Board,
        );
        assert_eq!(d, Decision::Allow);
    }
}
