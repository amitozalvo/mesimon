use crate::Principal;

/// What is being attempted. Grows with the command set; every daemon mutation
/// path constructs one of these before acting (D32c invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Decision {
    pub fn denied(&self) -> bool {
        matches!(self, Decision::Deny { .. })
    }
}

/// The single authorization chokepoint (D32c invariant 2).
///
/// `Local` is the human at the keyboard and is allowed everything. `Automation`
/// is the daemon's own rules acting without anyone asking — also allowed
/// everything, because what restrains an automation is not authority but the
/// ping-pong guard, the flap fuse and the cascade depth limit in
/// `Daemon::place_ticket`. `Agent` is an agent session asking over MCP, and it
/// is the one principal this function actually restricts.
///
/// Two rules for `Agent`, and they are the permanent floor rather than a
/// default someone can widen in a config file:
///
/// * **No agent reads or changes a session, ever.** Not its own, not another's.
///   There is no `get_session`, no transcript, no scrollback, no cost — and
///   when a second principal exists (D32h), "teammates see tickets and
///   outcomes, never sessions" is already enforced here.
/// * **No agent mutates the board as a whole.** It may move its own ticket
///   between columns; it may not create, delete or reorder columns, and there
///   is no command that would let it try (`mcp::agent_allows`).
///
/// Ticket ownership is enforced by construction, not here: no agent command
/// carries a ticket id, so the daemon can only ever pass the agent's own.
pub fn authorize(principal: &Principal, action: &Action, resource: &Resource) -> Decision {
    let deny = |reason: &str| Decision::Deny { reason: reason.to_string() };
    match principal {
        Principal::Local | Principal::Automation { .. } => Decision::Allow,
        Principal::Agent { .. } => match (action, resource) {
            (_, Resource::Session { .. }) => {
                deny("an agent cannot read or change a session, at any tier")
            }
            (Action::Mutate, Resource::Board) => {
                deny("an agent cannot change the board itself, only its own ticket's column")
            }
            (Action::Read, _) => Decision::Allow,
            (Action::Mutate, Resource::Ticket { .. } | Resource::Column { .. }) => Decision::Allow,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> Principal {
        Principal::Agent { session: uuid::Uuid::nil() }
    }
    fn automation() -> Principal {
        Principal::Automation { rule: "automove".into() }
    }

    #[test]
    fn v01_allows_local() {
        assert_eq!(
            authorize(&Principal::Local, &Action::Mutate, &Resource::Board),
            Decision::Allow
        );
        assert_eq!(
            authorize(
                &Principal::Local,
                &Action::Mutate,
                &Resource::Session { id: uuid::Uuid::nil() }
            ),
            Decision::Allow
        );
    }

    /// Automation is what `automove` and hook ingestion run as. Restricting it
    /// here would break the board moving itself, which is a feature; what keeps
    /// it honest is the guard set in `place_ticket`, not this function.
    #[test]
    fn automation_is_allowed_and_restrained_elsewhere() {
        assert_eq!(authorize(&automation(), &Action::Mutate, &Resource::Board), Decision::Allow);
        assert_eq!(
            authorize(&automation(), &Action::Mutate, &Resource::Session { id: uuid::Uuid::nil() }),
            Decision::Allow
        );
    }

    /// The permanent floor. If this test is ever edited to pass, the change is
    /// to the product's security posture.
    #[test]
    fn an_agent_never_touches_a_session() {
        let s = Resource::Session { id: uuid::Uuid::nil() };
        assert!(authorize(&agent(), &Action::Read, &s).denied());
        assert!(authorize(&agent(), &Action::Mutate, &s).denied());
    }

    #[test]
    fn an_agent_cannot_mutate_the_board_itself() {
        assert!(authorize(&agent(), &Action::Mutate, &Resource::Board).denied());
        assert_eq!(authorize(&agent(), &Action::Read, &Resource::Board), Decision::Allow);
    }

    #[test]
    fn an_agent_may_move_a_ticket_between_columns() {
        let t = Resource::Ticket { id: ulid::Ulid::nil() };
        let c = Resource::Column { name: "REVIEW".into() };
        assert_eq!(authorize(&agent(), &Action::Mutate, &t), Decision::Allow);
        assert_eq!(authorize(&agent(), &Action::Mutate, &c), Decision::Allow);
    }

    #[test]
    fn denials_say_why() {
        let Decision::Deny { reason } =
            authorize(&agent(), &Action::Read, &Resource::Session { id: uuid::Uuid::nil() })
        else {
            panic!("expected a denial");
        };
        assert!(reason.contains("session"), "the reason is rendered to the agent: {reason}");
    }
}
