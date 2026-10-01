//! Agent tiers in the daemon (T-443): the machine layer's cache, the five
//! commands, and the switch a running seat is owed.
//!
//! A tier reaches a session one way — `LaunchContext.tier`, read at every
//! spawn, resume and wake — so a SWITCH is a relaunch: the seat is parked at
//! its next idle and woken on the new tier, the road T-434's plan mode
//! already takes (`relaunch`). Only a person's explicit pick for a ticket
//! owes one; a changed default or an edited tier waits for the next launch,
//! the way a column setting does.

use super::*;
use mesimon_core::tier::{self, Book, MachineTiers, Tier, TierScope};

/// How long a ticket's pick must stand still before its running seat is
/// relaunched: `^n` pressed three times on the way to a tier relaunches
/// once, on the last.
pub(super) const TIER_SETTLE_MS: u64 = 2_000;

/// `tiers.toml` as this daemon last read it. Every board's daemon shares
/// the file; one `stat` per look tells whether another board changed it.
pub(super) struct MachineTierCache {
    path: Option<std::path::PathBuf>,
    /// (mtime, length) at the last read; `None` for a file that was absent.
    stamp: Option<(std::time::SystemTime, u64)>,
    pub(super) tiers: MachineTiers,
    pub(super) notice: Option<Notice>,
    pub(super) barred: bool,
}

impl MachineTierCache {
    pub(super) fn new(path: Option<std::path::PathBuf>) -> Self {
        let mut cache =
            Self { path, stamp: None, tiers: MachineTiers::default(), notice: None, barred: false };
        cache.reload();
        cache
    }

    fn current_stamp(&self) -> Option<(std::time::SystemTime, u64)> {
        let meta = std::fs::metadata(self.path.as_ref()?).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    fn reload(&mut self) {
        let Some(path) = self.path.clone() else { return };
        let loaded = store::load_machine_tiers(&path);
        self.tiers = loaded.tiers;
        self.notice = loaded.notice;
        self.barred = loaded.barred;
        self.stamp = self.current_stamp();
    }

    /// Re-read when the file changed since the last look. True when it did.
    pub(super) fn refresh(&mut self) -> bool {
        if self.path.is_none() || self.current_stamp() == self.stamp {
            return false;
        }
        let before = self.tiers.clone();
        self.reload();
        self.tiers != before
    }

    fn save(&mut self, tiers: MachineTiers) -> Result<(), String> {
        if self.barred {
            return Err("tiers.toml could not be read and is being preserved — nothing was \
                        changed"
                .into());
        }
        let Some(path) = self.path.clone() else {
            return Err("no state directory for the machine's tiers".into());
        };
        store::save_machine_tiers(&path, &tiers).map_err(|e| format!("could not save: {e:#}"))?;
        self.tiers = tiers;
        self.stamp = self.current_stamp();
        Ok(())
    }
}

impl Daemon {
    /// The two layers, for one question.
    pub(super) fn tier_book(&self) -> Book<'_> {
        Book::new(&self.machine_tiers.tiers, &self.board)
    }

    /// Pick up another board's edit of `tiers.toml`: one `stat`, on every
    /// command and every second, so a spawn never reads a stale list for
    /// longer than that.
    pub(super) fn refresh_machine_tiers(&mut self) {
        if self.machine_tiers.refresh() {
            self.broadcast();
        }
    }

    /// Whether this seat owes a switch that has not happened yet.
    pub(super) fn tier_owed(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().any(|s| s.id == id && s.tier_owed && s.state.has_pane())
    }

    /// The tier id a record launched on; a record from before tiers ran its
    /// provider's built-in.
    fn launched_tier(rec: &SessionRecord) -> String {
        match (rec.tier.is_empty(), rec.kind.provider()) {
            (true, Some(p)) => tier::builtin_id(p).to_string(),
            _ => rec.tier.clone(),
        }
    }

    /// What every launch stamps on the record it launches (T-443).
    pub(super) fn stamp_tier(&mut self, id: uuid::Uuid) {
        let Some((ticket, kind)) =
            self.board.sessions.iter().find(|s| s.id == id).map(|s| (s.ticket, s.kind))
        else {
            return;
        };
        let launched = self.tier_book().launch(ticket, kind).id;
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.tier = launched;
            rec.tier_owed = false;
            rec.tier_wake = false;
        }
    }

    /// `Command::SetTicketTier`.
    pub(super) fn set_ticket_tier(&mut self, id: ulid::Ulid, pick: Option<String>) -> Response {
        match self.apply_ticket_tier(id, pick) {
            Ok(()) => {
                self.persist_sessions();
                self.broadcast();
                Response::Ok
            }
            Err(message) => Response::Err { message },
        }
    }

    /// Set a ticket's pick (the command, the ask field's `^n`, the
    /// composer's mint): validated, stored as inherit when it is the
    /// default, and — on a seat with a pane — owed as a switch when it is
    /// not what the seat runs. The caller persists sessions and broadcasts.
    pub(super) fn apply_ticket_tier(
        &mut self,
        id: ulid::Ulid,
        pick: Option<String>,
    ) -> Result<(), String> {
        let Some(current) = self.board.ticket(id).map(|t| t.tier.clone()) else {
            return Err("no such ticket".into());
        };
        let book = self.tier_book();
        let want = match pick.as_deref() {
            None => book.default_tier(),
            Some(t) => book.get(t).ok_or_else(|| format!("no tier {t} on this machine"))?,
        };
        let seat = self.board.live_agent(id).map(|s| (s.id, s.kind, s.state.has_pane()));
        if let Some(provider) = seat.and_then(|(_, kind, _)| kind.provider()) {
            if provider != want.provider {
                return Err(format!(
                    "this ticket's {} runs {} ∙ a {} tier needs a fresh one",
                    mesimon_core::keymap::AGENT_WORD,
                    provider.label(),
                    want.provider.label()
                ));
            }
        }
        let stored = book.stored_pick(&want.id);
        if current != stored {
            if let Some(t) = self.board.ticket_mut(id) {
                t.tier = stored;
                let t = t.clone();
                let _ = store::save_ticket(&self.paths, &t);
            }
        }
        if let Some((sid, _, true)) = seat {
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == sid) {
                rec.tier_owed = Self::launched_tier(rec) != want.id;
            }
            self.tier_set_at.insert(id, now_ms());
        }
        Ok(())
    }

    /// `Command::SaveTier`: create or edit a tier on one layer.
    pub(super) fn save_tier(&mut self, scope: TierScope, mut t: Tier) -> Response {
        t.name = t.name.trim().to_string();
        t.model = t.model.trim().to_string();
        if t.id.is_empty() || t.is_builtin() {
            return Response::Err { message: "the built-in tiers cannot be edited".into() };
        }
        let taken: Vec<Tier> = match scope {
            TierScope::Machine => {
                let mut v = self.machine_tiers.tiers.tiers.clone();
                v.extend(self.board.tiers.iter().cloned());
                v
            }
            TierScope::Board => self.tier_book().all(),
        };
        if let Err(message) = tier::check_name(&t.name, &taken, Some(&t.id)) {
            return Response::Err { message };
        }
        if let Err(message) = tier::check_model(&t.model) {
            return Response::Err { message };
        }
        if !tier::Effort::ring(t.provider).contains(&t.effort) {
            return Response::Err {
                message: format!("{} has no {} effort", t.provider.label(), t.effort.word()),
            };
        }
        match scope {
            TierScope::Machine => {
                let mut next = self.machine_tiers.tiers.clone();
                match next.tiers.iter_mut().find(|x| x.id == t.id) {
                    Some(slot) => *slot = t,
                    None => next.tiers.push(t),
                }
                if let Err(message) = self.machine_tiers.save(next) {
                    return Response::Err { message };
                }
            }
            TierScope::Board => {
                if self.columns_barred {
                    return Response::Err { message: self.barred_message("columns") };
                }
                match self.board.tiers.iter_mut().find(|x| x.id == t.id) {
                    Some(slot) => *slot = t,
                    None => self.board.tiers.push(t),
                }
                self.persist_columns();
            }
        }
        self.broadcast();
        Response::Ok
    }

    /// `Command::DeleteTier`. At board scope an override reverts to the
    /// machine's tier. This board's tickets whose pick stops resolving go
    /// back to the default — and their seats owe nothing, since nobody
    /// picked what they now resolve to. Other boards' tickets dangle to
    /// their own default, which the resolver already reads that way.
    pub(super) fn delete_tier(&mut self, scope: TierScope, id: String) -> Response {
        match scope {
            TierScope::Machine => {
                let mut next = self.machine_tiers.tiers.clone();
                next.tiers.retain(|t| t.id != id);
                if next.default_tier.as_deref() == Some(id.as_str()) {
                    next.default_tier = None;
                }
                if next == self.machine_tiers.tiers {
                    return Response::Ok;
                }
                if let Err(message) = self.machine_tiers.save(next) {
                    return Response::Err { message };
                }
            }
            TierScope::Board => {
                if self.columns_barred {
                    return Response::Err { message: self.barred_message("columns") };
                }
                self.board.tiers.retain(|t| t.id != id);
                if self.board.default_tier.as_deref() == Some(id.as_str())
                    && self.tier_book().get(&id).is_none()
                {
                    self.board.default_tier = None;
                }
                self.persist_columns();
            }
        }
        if self.tier_book().get(&id).is_none() {
            let orphans: Vec<ulid::Ulid> = self
                .board
                .tickets
                .iter()
                .filter(|t| t.tier.as_deref() == Some(id.as_str()))
                .map(|t| t.id)
                .collect();
            for ticket in orphans {
                let _ = self.with_ticket(ticket, |t| t.tier = None);
                for rec in self.board.sessions.iter_mut().filter(|s| s.ticket == ticket) {
                    rec.tier_owed = false;
                }
            }
            self.persist_sessions();
        }
        self.broadcast();
        Response::Ok
    }

    /// `Command::MoveTier` (T-562): one layer's order, which the tiers list
    /// draws and `^n` cycles. A board moves only its own tiers.
    pub(super) fn move_tier(&mut self, scope: TierScope, id: String, to: usize) -> Response {
        let ordered = self.tier_book().ordered(scope);
        let refused = || Response::Err {
            message: match scope {
                TierScope::Machine => format!("no tier {id} on this machine"),
                TierScope::Board => {
                    "the machine orders its tiers ∙ this board orders its own".to_string()
                }
            },
        };
        match scope {
            TierScope::Machine => {
                let mut next = self.machine_tiers.tiers.clone();
                match tier::move_tier(&mut next.tiers, &ordered, &id, to) {
                    None => return refused(),
                    Some(false) => return Response::Ok,
                    Some(true) => {}
                }
                if let Err(message) = self.machine_tiers.save(next) {
                    return Response::Err { message };
                }
            }
            TierScope::Board => {
                if self.columns_barred {
                    return Response::Err { message: self.barred_message("columns") };
                }
                match tier::move_tier(&mut self.board.tiers, &ordered, &id, to) {
                    None => return refused(),
                    Some(false) => return Response::Ok,
                    Some(true) => {}
                }
                self.persist_columns();
            }
        }
        self.broadcast();
        Response::Ok
    }

    /// `Command::SetDefaultTier`. The board's choice keeps
    /// `Board::agent_provider` in step, so the pre-tier reading of that field
    /// (`Book::board_default`) never contradicts it.
    pub(super) fn set_default_tier(&mut self, scope: TierScope, id: Option<String>) -> Response {
        let provider = match id.as_deref() {
            None => AgentProvider::ClaudeCode,
            Some(t) => match self.tier_book().get(t) {
                Some(t) => t.provider,
                None => return Response::Err { message: format!("no tier {t} on this machine") },
            },
        };
        match scope {
            TierScope::Machine => {
                let mut next = self.machine_tiers.tiers.clone();
                next.default_tier = id.filter(|t| t != tier::CLAUDE);
                if next == self.machine_tiers.tiers {
                    return Response::Ok;
                }
                if let Err(message) = self.machine_tiers.save(next) {
                    return Response::Err { message };
                }
            }
            TierScope::Board => {
                if self.columns_barred {
                    return Response::Err { message: self.barred_message("columns") };
                }
                if self.board.default_tier == id && self.board.agent_provider == provider {
                    return Response::Ok;
                }
                self.board.default_tier = id;
                self.board.agent_provider = provider;
                self.persist_columns();
            }
        }
        self.broadcast();
        Response::Ok
    }

    /// Relaunch the seats that owe a tier switch (T-443). Two phases:
    ///
    /// A. A paned seat whose ticket's pick has stood still for
    ///    `TIER_SETTLE_MS`, idle between turns, not the pane a person is
    ///    inside, with a conversation to resume and no ask queued (a queued
    ///    ask carries the switch itself — `deliver` relaunches instead of
    ///    pasting — and keeps the checkout rules the queue has). Claude is
    ///    parked and woken on the spot; Codex is parked and marked, because
    ///    its wake has to wait for the runtime to confirm the stop.
    /// B. A Codex seat parked for a switch whose stop is confirmed: woken.
    pub(super) fn drain_tier_switches(&mut self) -> bool {
        let now = now_ms();
        let mut changed = false;
        let wakes: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.tier_wake
                    && s.state == SessionState::Sleeping
                    && !s.codex_stopping
                    && !self.queued.iter().any(|q| q.ticket == s.ticket)
            })
            .map(|s| s.id)
            .collect();
        for id in wakes {
            let ticket = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket);
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.tier_wake = false;
            }
            changed = true;
            match self.resume_session_in(id, false, false) {
                Response::Err { message } => {
                    eprintln!("mesimon: tier switch wake failed: {message}");
                    self.feed.board("automation", "tier_switch_failed", ticket);
                }
                _ => self.feed.board("automation", "tier_switch_woke", ticket),
            }
        }
        let focused = match self.focus_held() {
            Some(Focus::Session(id)) => Some(id),
            _ => None,
        };
        let due: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.tier_owed
                    && s.kind.is_agent()
                    && s.state.has_pane()
                    && focused != Some(s.id)
                    && self
                        .tier_set_at
                        .get(&s.ticket)
                        .is_none_or(|at| now.saturating_sub(*at) >= TIER_SETTLE_MS)
                    && self.session_idle(s.id)
                    && !self.queued.iter().any(|q| q.ticket == s.ticket)
                    && !self.owed.contains_key(&s.id)
                    && !self.pending_resumes.iter().any(|p| p.session == s.id)
                    && self.machines.get(&s.id).is_some_and(|m| m.view().pending.is_none())
            })
            .map(|s| s.id)
            .collect();
        let by = Principal::Automation { rule: "tier_switch".into() };
        for id in due {
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { continue };
            if !matches!(
                authorize(&by, &Action::Mutate, &Resource::Session { id }),
                Decision::Allow
            ) || self.resume_board_guard(rec).is_some()
                || self.resume_guard(rec, false).is_some()
            {
                continue;
            }
            let Some(adapter) = crate::agents::adapter(rec.kind) else { continue };
            // The switch continues THIS conversation or it does not happen:
            // a relaunch that could only start fresh waits for the person's
            // own wake, which says so.
            if adapter.history_missing(rec) || adapter.conversation_key(rec).is_none() {
                continue;
            }
            let ticket = rec.ticket;
            changed = true;
            if let Response::Err { message } = self.relaunch(ticket, id, None, false, "automation")
            {
                eprintln!("mesimon: tier switch failed: {message}");
                self.feed.board("automation", "tier_switch_failed", Some(ticket));
            }
        }
        if changed {
            self.persist_and_notify();
        }
        changed
    }

    /// Park an idle pane and wake it on what the ticket asks for now — plan
    /// mode for one launch (T-434), a new tier (T-443), or both — with the
    /// words, if any, held for its first tick. The one road, because
    /// neither CLI changes its launch flags from inside a turn the daemon
    /// can see. `sleep_one` is the gate: only an idle agent sleeps, so a
    /// turn is never cut.
    ///
    /// Codex cannot be woken until its runtime confirms the stop, so a
    /// Codex seat is marked (`tier_wake`) and any words wait in the queue
    /// as a wake of the same record, which `queued_target_ready` holds
    /// until the stop is confirmed.
    pub(super) fn relaunch(
        &mut self,
        ticket: ulid::Ulid,
        id: uuid::Uuid,
        text: Option<String>,
        plan: bool,
        actor: &str,
    ) -> Response {
        let why = if plan { "plan mode" } else { "a tier switch" };
        if !self.session_idle(id) {
            return Response::Err {
                message: format!(
                    "{} is mid-turn ∙ {why} waits for idle",
                    mesimon_core::keymap::AGENT_WORD
                ),
            };
        }
        let codex = self.board.sessions.iter().any(|s| s.id == id && s.kind == SessionKind::Codex);
        if let Err(message) = self.sleep_one(id, false) {
            return Response::Err { message: format!("could not park for {why}: {message}") };
        }
        self.feed.board(actor, if plan { "plan_relaunch" } else { "tier_switch" }, Some(ticket));
        if codex {
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.tier_wake = true;
            }
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                if let Err(message) =
                    self.park_ask(ticket, QueuedSeat::Wake(id), text, None, false, plan)
                {
                    eprintln!("mesimon: the words for the switched codex were lost: {message}");
                }
                self.persist_queue();
            }
            self.persist_and_notify();
            return Response::Ok;
        }
        match text {
            Some(text) => self.prompt_sleeping(ticket, text, plan),
            None => self.resume_session_in(id, false, plan),
        }
    }
}
