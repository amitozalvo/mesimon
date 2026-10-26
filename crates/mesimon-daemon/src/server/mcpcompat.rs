//! MCP reader compatibility, before dispatch and at the response boundary.
use super::*;
use mesimon_core::command::MCP_PROTOCOL;

const REFRESH: &str = "this session's Mesimon tools need refreshing after an update; \
    this request was not applied. Sleeping and waking this agent from its card reloads \
    its tools. raise_hand remains available to tell the person that this session needs \
    sleep and wake";

/// A reader contract is not a permission: the ordinary agent allowlist,
/// session binding and authorization still judge every admitted command.
fn prepare(protocol: u32, command: &mut Command) -> Result<(), &'static str> {
    if !mcp::agent_allows(command)
        || matches!(command, Command::ModNext { .. } | Command::ModPing { .. })
    {
        return Ok(());
    }
    // A mismatched reader must still be able to reach the person. Its
    // receipt has used the same shape since raise_hand was introduced.
    if matches!(command, Command::AgentRaiseHand { .. }) {
        return Ok(());
    }
    match protocol {
        MCP_PROTOCOL => Ok(()),
        0 => match command {
            Command::AgentGetTicket { full } => {
                *full = true;
                Ok(())
            }
            Command::AgentReadTicket { full, plan, .. } => {
                *full = true;
                // The old tool has no `plan` argument with which to ask
                // for the remainder of a plan's new bounded head.
                *plan = true;
                Ok(())
            }
            // Frozen beta.13 command set. A new tool is refused by default
            // before it does anything, until its legacy receipt is reviewed.
            Command::AgentListBoard
            | Command::AgentMoveTicket { .. }
            | Command::AgentRenameTicket { .. }
            | Command::AgentSetWorkspace { .. }
            | Command::AgentArchiveTicket { .. }
            | Command::AgentStartTicket { .. }
            | Command::AgentSleepTicket { .. }
            | Command::AgentMergeTicket { .. }
            | Command::AgentWatchTicket { .. }
            | Command::AgentAskTicket { .. }
            | Command::AgentAskBoard { .. }
            | Command::AgentAnswerTicket { .. }
            | Command::AgentAcceptPlan { .. }
            | Command::AgentReadAttachment { .. }
            | Command::AgentReadShell { .. }
            | Command::AgentRequestCloseShell { .. }
            | Command::AgentReadNote { .. }
            | Command::AgentWriteNote { .. }
            | Command::AgentCreateTicket { .. }
            | Command::AgentTagTicket { .. } => Ok(()),
            _ => Err(REFRESH),
        },
        _ => Err(REFRESH),
    }
}

impl Daemon {
    pub(super) fn handle_compatible(
        &mut self,
        mut env: Envelope,
        protocol: u32,
        stream: &Arc<Mutex<UnixStream>>,
    ) -> Response {
        if !matches!(env.principal, Principal::Agent { .. }) {
            return self.handle(env, stream);
        }
        if let Err(message) = prepare(protocol, &mut env.command) {
            return Response::Err { message: message.into() };
        }
        let response = self.handle(env, stream);
        if protocol == MCP_PROTOCOL {
            return response;
        }
        match response {
            Response::AgentNoteWritten { note, .. } => Response::NoteWritten { note },
            Response::AgentTicketUpdated { key, .. } => self
                .board
                .ticket_by_key(&key)
                .and_then(|t| self.agent_ticket_view(t.id))
                .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket }),
            other => other,
        }
    }
}

pub(super) fn wire_reply(
    response: Response,
    told: Option<(uuid::Uuid, u64)>,
    legacy: bool,
) -> (Response, Option<(uuid::Uuid, u64)>) {
    if legacy {
        if let Response::CrownRules { inner, .. } = response {
            return (*inner, None);
        }
    }
    (response, told)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_reader_gets_full_reads_and_new_commands_stop_before_dispatch() {
        let mut read = Command::AgentReadTicket { key: "T-1".into(), full: false, plan: false };
        assert!(prepare(0, &mut read).is_ok());
        assert!(matches!(read, Command::AgentReadTicket { full: true, plan: true, .. }));
        let mut sample = Command::AgentSampleTicket { key: "T-1".into() };
        assert_eq!(prepare(0, &mut sample), Err(REFRESH));
        assert!(prepare(MCP_PROTOCOL, &mut sample).is_ok());
    }

    #[test]
    fn unknown_contract_keeps_the_way_to_reach_the_person() {
        let mut read = Command::AgentGetTicket { full: false };
        assert_eq!(prepare(MCP_PROTOCOL + 1, &mut read), Err(REFRESH));
        let mut hand = Command::AgentRaiseHand { reason: "tools need refresh".into() };
        assert!(prepare(MCP_PROTOCOL + 1, &mut hand).is_ok());
        assert!(REFRESH.contains("not applied"));
        assert!(REFRESH.contains("raise_hand"));
        assert!(REFRESH.contains("Sleeping and waking"));
    }

    #[test]
    fn stripping_rules_from_a_deferred_reply_never_counts_them_as_delivered() {
        let told = Some((uuid::Uuid::nil(), 3));
        let wrapped = || Response::CrownRules {
            rules: "rules".into(),
            inner: Box::new(Response::Err { message: "a refusal".into() }),
        };
        let (old, old_told) = wire_reply(wrapped(), told, true);
        assert!(matches!(old, Response::Err { .. }));
        assert_eq!(old_told, None);
        let (new, new_told) = wire_reply(wrapped(), told, false);
        assert!(matches!(new, Response::CrownRules { .. }));
        assert_eq!(new_told, told);
    }
}
