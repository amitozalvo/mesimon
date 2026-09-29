use crate::Principal;

/// What is being attempted. Grows with the command set; every daemon mutation
/// path constructs one of these before acting (D32c invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Mutate,
    /// Materialize external content as a fresh, inert local ticket. There is
    /// no wire command for this seam. Existing agents and daemon automation
    /// do not acquire import authority from ordinary content-write access.
    ImportContent,
    /// Submit user input to an existing session; does not confer lifecycle authority.
    PromptExisting,
    /// Answer one pending native permission; no policy or lifecycle authority.
    ApproveExisting,
    /// Append one new ticket to a column: a title, a description and tags
    /// the board already has (T-497). Starts, moves and edits nothing, so
    /// the owner's paired phone may file a ticket without gaining `Mutate`
    /// on a column, which would also reach `PromptColumn`.
    FileTicket,
    /// Start an agent in one ticket's empty seat (T-498): the owner's paired
    /// phone gains the start without `Mutate` on the ticket, which would also
    /// move, rename and merge it. The crown's `start_agent` stays a `Mutate`.
    StartAgent,
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
/// is the daemon's own rules acting without anyone asking — allowed ordinary
/// reads and mutations, because what restrains an automation there is the
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
///   between columns and append a new ticket to a column (`create_ticket`,
///   a `Mutate` on `Resource::Column`); it may not create, delete or reorder
///   columns, and there is no command that would let it try
///   (`mcp::agent_allows`).
///
/// Ticket ownership is enforced by construction, not here: no agent command
/// carries a ticket id, so the daemon can only ever pass the agent's own.
/// `ImportContent` is a separate local-owner-only action, scoped to a destination
/// column; ordinary write access does not grant authority to materialize imports.
/// `FileTicket` is the one board write a `Paired` phone makes: the owner at
/// the keyboard or on their paired device, into a column, and nothing else.
/// `StartAgent` is its one start, on a ticket; `authorize_execution` is the
/// floor under it.
pub fn authorize(principal: &Principal, action: &Action, resource: &Resource) -> Decision {
    let deny = |reason: &str| Decision::Deny { reason: reason.to_string() };
    if matches!(action, Action::PromptExisting | Action::ApproveExisting) {
        return match (principal, resource) {
            (Principal::Local | Principal::Paired { .. }, Resource::Session { .. }) => {
                Decision::Allow
            }
            _ => deny("prompting requires an authenticated owner and an existing session"),
        };
    }
    if *action == Action::ImportContent {
        return match (principal, resource) {
            (Principal::Local, Resource::Column { .. }) => Decision::Allow,
            _ => deny("content import requires a local owner and a destination column"),
        };
    }
    if *action == Action::FileTicket {
        return match (principal, resource) {
            (Principal::Local | Principal::Paired { .. }, Resource::Column { .. }) => {
                Decision::Allow
            }
            _ => deny("filing a ticket requires an authenticated owner and a destination column"),
        };
    }
    if *action == Action::StartAgent {
        return match (principal, resource) {
            (Principal::Local | Principal::Paired { .. }, Resource::Ticket { .. }) => {
                Decision::Allow
            }
            _ => deny("starting an agent requires an authenticated owner and a ticket"),
        };
    }
    match principal {
        Principal::Local | Principal::Automation { .. } => Decision::Allow,
        Principal::Paired { .. } => match action {
            Action::Read => Decision::Allow,
            _ => deny(
                "paired devices only read, prompt, answer existing permissions, file tickets \
                 and start agents",
            ),
        },
        Principal::Agent { .. } => match (action, resource) {
            (_, Resource::Session { .. }) => {
                deny("an agent cannot read or change a session, at any tier")
            }
            (Action::Mutate, Resource::Board) => {
                deny("an agent cannot change the board itself, only its own ticket's column")
            }
            (Action::Read, _) => Decision::Allow,
            (Action::Mutate, Resource::Ticket { .. } | Resource::Column { .. }) => Decision::Allow,
            (
                Action::ImportContent
                | Action::PromptExisting
                | Action::ApproveExisting
                | Action::FileTicket
                | Action::StartAgent,
                _,
            ) => deny("an agent cannot import external content"),
        },
        // A teammate on a shared board (T-215): tickets and notes per the
        // role the relay enforced, never a session, never the board's own
        // shape. The role check happened before the record was accepted;
        // this is the floor under it.
        Principal::Remote { .. } => match (action, resource) {
            (_, Resource::Session { .. }) => deny("a teammate cannot read or change a session"),
            (Action::Mutate, Resource::Board) => {
                deny("a teammate cannot change the board itself, only its tickets")
            }
            (Action::Read, _) => Decision::Allow,
            (Action::Mutate, Resource::Ticket { .. } | Resource::Column { .. }) => Decision::Allow,
            (
                Action::ImportContent
                | Action::PromptExisting
                | Action::ApproveExisting
                | Action::FileTicket
                | Action::StartAgent,
                _,
            ) => deny("a teammate cannot import external content"),
        },
    }
}

/// Additional execution floor for a ticket with durable intake restrictions.
/// Ordinary resource authorization must also pass. Remote principals are never
/// converted to Local by an adapter; Local retains the existing same-UID boundary.
/// The owner's paired phone (T-498) passes where automation does: a ticket
/// whose words came from outside (an import, a teammate) waits for the
/// keyboard, whoever asks.
pub fn authorize_execution(
    principal: &Principal,
    policy: crate::board::ExecutionPolicy,
) -> Decision {
    match principal {
        Principal::Local => Decision::Allow,
        Principal::Automation { .. } | Principal::Paired { .. } if policy.allows_automation() => {
            Decision::Allow
        }
        Principal::Automation { .. }
        | Principal::Agent { .. }
        | Principal::Remote { .. }
        | Principal::Paired { .. } => {
            Decision::Deny { reason: "execution requires the owner at the keyboard".into() }
        }
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
    fn remote() -> Principal {
        Principal::Remote { member: "Dana".into() }
    }

    #[test]
    fn paired_authority_is_limited_to_reads_and_existing_prompts() {
        let paired = Principal::Paired { device: "device".into(), grant: "grant".into() };
        let session = Resource::Session { id: uuid::Uuid::nil() };
        assert_eq!(authorize(&paired, &Action::PromptExisting, &session), Decision::Allow);
        assert_eq!(authorize(&paired, &Action::ApproveExisting, &session), Decision::Allow);
        for by in [agent(), remote(), automation()] {
            assert!(authorize(&by, &Action::ApproveExisting, &session).denied());
        }
        assert!(authorize(&paired, &Action::ApproveExisting, &Resource::Board).denied());
        assert_eq!(authorize(&paired, &Action::Read, &session), Decision::Allow);
        for by in [agent(), remote(), automation()] {
            assert!(authorize(&by, &Action::PromptExisting, &session).denied());
        }
        for resource in [
            session,
            Resource::Board,
            Resource::Ticket { id: ulid::Ulid::nil() },
            Resource::Column { name: "TODO".into() },
        ] {
            assert!(authorize(&paired, &Action::Mutate, &resource).denied());
            assert!(authorize(&paired, &Action::ImportContent, &resource).denied());
        }
        assert!(authorize(&paired, &Action::PromptExisting, &Resource::Board).denied());
        assert!(authorize_execution(&paired, crate::board::ExecutionPolicy::OwnerOnly).denied());
    }

    /// The phone starts an agent on one ticket (T-498) and gains nothing a
    /// ticket `Mutate` would carry: no move, no rename, no merge. A ticket
    /// whose words came from outside still starts only at the keyboard.
    #[test]
    fn starting_an_agent_is_the_owners_and_only_on_a_ticket() {
        use crate::board::ExecutionPolicy::{LocalAutomation, OwnerOnly};
        let paired = Principal::Paired { device: "device".into(), grant: "grant".into() };
        let ticket = Resource::Ticket { id: ulid::Ulid::nil() };
        for owner in [&Principal::Local, &paired] {
            assert_eq!(authorize(owner, &Action::StartAgent, &ticket), Decision::Allow);
            for elsewhere in [
                Resource::Board,
                Resource::Column { name: "TODO".into() },
                Resource::Session { id: uuid::Uuid::nil() },
            ] {
                assert!(authorize(owner, &Action::StartAgent, &elsewhere).denied());
            }
        }
        for by in [agent(), remote(), automation()] {
            assert!(authorize(&by, &Action::StartAgent, &ticket).denied());
        }
        assert!(authorize(&paired, &Action::Mutate, &ticket).denied());
        assert_eq!(authorize_execution(&paired, LocalAutomation), Decision::Allow);
        assert!(authorize_execution(&paired, OwnerOnly).denied());
    }

    /// The phone files a ticket into a column (T-497) and gains nothing a
    /// column `Mutate` would carry: no move, no column prompt, no edit.
    #[test]
    fn filing_a_ticket_is_the_owners_and_only_into_a_column() {
        let paired = Principal::Paired { device: "device".into(), grant: "grant".into() };
        let column = Resource::Column { name: "TODO".into() };
        for owner in [&Principal::Local, &paired] {
            assert_eq!(authorize(owner, &Action::FileTicket, &column), Decision::Allow);
            for elsewhere in [
                Resource::Board,
                Resource::Ticket { id: ulid::Ulid::nil() },
                Resource::Session { id: uuid::Uuid::nil() },
            ] {
                assert!(authorize(owner, &Action::FileTicket, &elsewhere).denied());
            }
        }
        for by in [agent(), remote(), automation()] {
            assert!(authorize(&by, &Action::FileTicket, &column).denied());
        }
        assert!(authorize(&paired, &Action::Mutate, &column).denied());
    }

    /// The teammate floor (T-215): like an agent on sessions and the board's
    /// shape, like a person on tickets. Never an importer, never a starter.
    #[test]
    fn a_teammate_edits_tickets_and_nothing_else() {
        let session = Resource::Session { id: uuid::Uuid::nil() };
        assert!(authorize(&remote(), &Action::Read, &session).denied());
        assert!(authorize(&remote(), &Action::Mutate, &session).denied());
        assert!(authorize(&remote(), &Action::Mutate, &Resource::Board).denied());
        assert_eq!(authorize(&remote(), &Action::Read, &Resource::Board), Decision::Allow);
        assert_eq!(
            authorize(&remote(), &Action::Mutate, &Resource::Ticket { id: ulid::Ulid::nil() }),
            Decision::Allow
        );
        assert_eq!(
            authorize(&remote(), &Action::Mutate, &Resource::Column { name: "TODO".into() }),
            Decision::Allow
        );
        assert!(authorize(
            &remote(),
            &Action::ImportContent,
            &Resource::Column { name: "TODO".into() }
        )
        .denied());
        use crate::board::ExecutionPolicy::{LocalAutomation, OwnerOnly};
        assert!(authorize_execution(&remote(), OwnerOnly).denied());
        assert!(authorize_execution(&remote(), LocalAutomation).denied());
    }

    #[test]
    fn owner_only_execution_never_inherits_automation_authority() {
        use crate::board::ExecutionPolicy::{LocalAutomation, OwnerOnly};
        assert_eq!(authorize_execution(&Principal::Local, OwnerOnly), Decision::Allow);
        assert_eq!(authorize_execution(&automation(), LocalAutomation), Decision::Allow);
        assert!(authorize_execution(&automation(), OwnerOnly).denied());
        assert!(authorize_execution(&agent(), OwnerOnly).denied());
        assert!(authorize_execution(&agent(), LocalAutomation).denied());
    }

    #[test]
    fn import_requires_local_owner_and_destination_column() {
        let destination = Resource::Column { name: "TODO".into() };
        assert_eq!(
            authorize(&Principal::Local, &Action::ImportContent, &destination),
            Decision::Allow
        );
        for by in [agent(), automation()] {
            assert!(authorize(&by, &Action::ImportContent, &destination).denied());
        }
        for resource in [
            Resource::Board,
            Resource::Ticket { id: ulid::Ulid::nil() },
            Resource::Session { id: uuid::Uuid::nil() },
        ] {
            assert!(authorize(&Principal::Local, &Action::ImportContent, &resource).denied());
        }
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
