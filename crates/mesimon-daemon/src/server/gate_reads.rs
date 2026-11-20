//! Fresh git and transcript evidence for reads that can finish on a later turn.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BranchQuery {
    pub branch: String,
    legs: Vec<worktree::Leg>,
    resolve_base: bool,
}

pub(super) struct LegRead {
    pub leg: worktree::Leg,
    pub tip: String,
    merged: bool,
}

impl BranchQuery {
    pub(super) fn read(&self, ancestry: bool) -> Vec<LegRead> {
        self.legs
            .iter()
            .cloned()
            .map(|mut leg| {
                if self.resolve_base {
                    leg.base = worktree::default_branch(&leg.repo).unwrap_or_default();
                }
                let tip = worktree::branch_tip(&leg.repo, &self.branch);
                let merged = ancestry
                    && tip != leg.base_oid
                    && !tip.is_empty()
                    && !leg.base.is_empty()
                    && worktree::is_merged(&leg.repo, &tip, &format!("refs/heads/{}", leg.base));
                LegRead { leg, tip, merged }
            })
            .collect()
    }
}

pub(super) struct Facts {
    pub legs: Vec<LegRead>,
    pub urls: Option<Vec<String>>,
}

pub(super) struct ViewRead {
    ticket: ulid::Ulid,
    plan: ViewPlan,
    facts: Facts,
    view: AgentTicketView,
    caller: Option<uuid::Uuid>,
    reply: Sender<ClientReply>,
}

#[derive(PartialEq, Eq)]
struct ViewPlan {
    query: Option<BranchQuery>,
    ancestry: bool,
    tails: bool,
}

impl Daemon {
    pub(super) fn branch_query(&self, ticket: ulid::Ulid) -> Option<BranchQuery> {
        let b = self.worktrees.get(&ticket)?;
        Some(BranchQuery {
            branch: b.branch.clone(),
            legs: self.binding_legs(b),
            resolve_base: !b.is_workspace() && self.worktree_base_cached().is_none(),
        })
    }

    pub(super) fn reads_landed(
        &self,
        ticket: ulid::Ulid,
        legs: &[LegRead],
        given_up: bool,
    ) -> bool {
        legs.iter().all(|r| self.leg_lands(ticket, &r.leg, &r.tip, || r.merged, given_up))
    }

    fn view_plan(&self, view: &AgentTicketView, ticket: ulid::Ulid) -> Option<ViewPlan> {
        let settings = || {
            view.allowed_columns
                .iter()
                .flatten()
                .filter_map(|c| self.board.column(c))
                .map(|c| &c.settings)
        };
        let merge = settings().any(|s| s.requires_merge);
        let pipeline = self.through_prs() && settings().any(|s| s.requires_pipeline);
        if !(merge || pipeline) {
            return None;
        }
        let set = self.forge.sets.get(&ticket).map(Vec::as_slice);
        let live = self.forge.at_ms.contains_key(&ticket);
        let pipelined = set.is_some_and(mesimon_core::forge::pipelined);
        let ancestry = merge && self.open_pr(ticket).is_none();
        let query = (ancestry || (pipeline && pipelined && live))
            .then(|| self.branch_query(ticket))
            .flatten()
            .filter(|q| !q.branch.is_empty());
        let tails = pipeline && !pipelined && !self.tearing_down.contains(&ticket);
        Some(ViewPlan { query, ancestry, tails })
    }

    /// Only a view that survived authorization and actually advertises gated
    /// columns needs I/O. A compact crown read has already shed those columns.
    pub(super) fn reply_ticket_view(
        &mut self,
        view: AgentTicketView,
        reply: Sender<ClientReply>,
        caller: Option<uuid::Uuid>,
        wait: bool,
    ) {
        let Some((ticket, plan)) = self
            .board
            .ticket_by_key(&view.key)
            .and_then(|t| self.view_plan(&view, t.id).map(|plan| (t.id, plan)))
        else {
            self.send_ticket_view(view, caller, reply, wait);
            return;
        };
        let tails = plan.tails.then(|| self.costs.tails(ticket, &self.live_transcripts(ticket)));
        self.after_session_save(wait, move |d, result| {
            if let Err(message) = result {
                d.session_reply(reply, Response::Err { message }, None, false);
                return;
            }
            d.off_writer(move || {
                let facts = Facts {
                    legs: plan.query.as_ref().map(|q| q.read(plan.ancestry)).unwrap_or_default(),
                    urls: tails.as_deref().and_then(crate::cost::urls_in_tails),
                };
                let read = ViewRead { ticket, plan, view, caller, reply, facts };
                Msg::TicketGates(Box::new(read))
            });
        });
    }

    pub(super) fn ticket_gates_read(&mut self, read: ViewRead) {
        let ViewRead { ticket, plan, facts, mut view, caller, reply } = read;
        if self.view_plan(&view, ticket).as_ref() != Some(&plan) {
            // A new binding or forge result can require different evidence.
            self.reply_ticket_view(view, reply, caller, false);
            return;
        }
        let merged = self.reads_landed(ticket, &facts.legs, true);
        if let Some(cols) = &mut view.allowed_columns {
            cols.retain(|c| {
                self.merge_pipeline_gate(
                    ticket,
                    c,
                    || (!merged).then(String::new),
                    Reading::Observed(&facts),
                )
                .is_none()
            });
        }
        self.send_ticket_view(view, caller, reply, false);
    }

    fn send_ticket_view(
        &mut self,
        view: AgentTicketView,
        caller: Option<uuid::Uuid>,
        reply: Sender<ClientReply>,
        wait: bool,
    ) {
        let (response, told) = self.caller_rules(caller, Response::AgentTicket { ticket: view });
        self.session_reply(reply, response, told, wait);
    }
}
