//! Pure Codex app-server observation. This module never replies to a request.
//!
//! Transport owners must reconnect to the same thread, reject obsolete worker
//! generations, and audit outstanding requests and child work before passing
//! `work_reconciled = true`. An idle thread snapshot alone is NOT that audit:
//! paginated history may omit active descendants. Snapshots never manufacture
//! a successful completion from historical turns.

use std::collections::{BTreeMap, BTreeSet};

use mesimon_core::attention::{rank, Signal, TurnOutcome};
use mesimon_core::board::{FailReason, Reason, SessionState, StopReason, UnknownReason};
use serde_json::Value;

const MAX_IDENTITIES: usize = 1024;
const MAX_PREVIEW_CHARS: usize = 32_768;

#[derive(Debug)]
enum PendingItem {
    Independent,
    Compaction { turn_id: Option<String> },
}

impl PendingItem {
    fn new(kind: &str, turn_id: Option<&str>) -> Self {
        if kind == "contextCompaction" {
            Self::Compaction { turn_id: turn_id.map(str::to_owned) }
        } else {
            Self::Independent
        }
    }
}

fn retire_cancelled_compaction(items: &mut BTreeMap<String, PendingItem>, turn: &Value) {
    if !matches!(outcome(turn), Some(TurnOutcome::Interrupted | TurnOutcome::Failed(_))) {
        return;
    }
    let Some(id) = string(turn, "id") else { return };
    // Codex cancels the turn's compaction future without item/completed.
    // Unlike tools, hooks, requests and descendants, that work cannot outlive
    // its terminal turn. Require its identity; an idle snapshot is not proof.
    items.retain(|_, item| {
        !matches!(item, PendingItem::Compaction { turn_id: Some(turn_id) } if turn_id == id)
    });
}

/// Normalized work captured before or after a parent identifies a descendant.
/// Never retain raw frames or model text, and never interpret child completion
/// as proof that its approvals, tools or hook continuations have stopped.
#[derive(Debug, Default)]
struct ChildWork {
    known: bool,
    requests: BTreeMap<String, Reason>,
    work_requests: BTreeSet<String>,
    items: BTreeMap<String, PendingItem>,
    hooks: BTreeSet<String>,
    descendants: BTreeSet<String>,
    flags: Vec<Reason>,
    uncertain: bool,
}

/// Native active flags summarize the same thread's requests coarsely (a
/// genuine MCP form also sets waitingOnApproval). Prefer the observed request
/// kind for display, retaining flags as fallback and independent safety work.
fn attention_reasons<'a>(
    requests: &'a BTreeMap<String, Reason>,
    flags: &'a [Reason],
) -> impl Iterator<Item = &'a Reason> {
    requests.values().chain(flags.iter().filter(move |_| requests.is_empty()))
}

impl ChildWork {
    fn pending(&self) -> bool {
        self.uncertain
            || !self.requests.is_empty()
            || !self.work_requests.is_empty()
            || !self.items.is_empty()
            || !self.hooks.is_empty()
            || !self.flags.is_empty()
    }

    fn identities(&self) -> usize {
        1 + self.requests.len()
            + self.work_requests.len()
            + self.items.len()
            + self.hooks.len()
            + self.descendants.len()
    }

    fn item(&mut self, item: &Value, completed: bool, turn_id: Option<&str>) {
        let (Some(id), Some(kind)) = (string(item, "id"), string(item, "type")) else { return };
        if !matches!(kind, "agentMessage" | "plan" | "userMessage" | "reasoning" | "hookPrompt") {
            if item_terminal(item, completed) {
                self.items.remove(id);
            } else {
                self.items.insert(id.into(), PendingItem::new(kind, turn_id));
            }
        }
        if kind == "collabAgentToolCall" {
            self.descendants.extend(receiver_ids(item));
        }
    }

    fn observe(&mut self, method: &str, frame: &Value) -> bool {
        let params = &frame["params"];
        if let Some(reason) = request_reason(method, params) {
            if let Some(id) = request_id(&frame["id"]) {
                self.requests.insert(id, reason);
            } else {
                self.uncertain = true;
            }
            return true;
        }
        match method {
            "turn/started" => {}
            "turn/completed" => {
                // Item terminal status is useful; the turn's terminal status
                // alone never resolves independently outstanding local work.
                if let Some(items) = params["turn"]["items"].as_array() {
                    for item in items {
                        self.item(item, true, string(&params["turn"], "id"));
                    }
                }
                retire_cancelled_compaction(&mut self.items, &params["turn"]);
            }
            "item/started" | "item/completed" => {
                self.item(&params["item"], method == "item/completed", string(params, "turnId"))
            }
            "serverRequest/resolved" => {
                if let Some(id) = request_id(&params["requestId"]) {
                    self.requests.remove(&id);
                    self.work_requests.remove(&id);
                }
            }
            "hook/started" | "hook/completed" => {
                if let Some(id) = string(&params["run"], "id") {
                    if method == "hook/started" {
                        self.hooks.insert(id.into());
                    } else {
                        self.hooks.remove(id);
                    }
                } else {
                    self.uncertain = true;
                }
            }
            "thread/status/changed" => {
                self.flags.clear();
                match string(&params["status"], "type") {
                    // A closed native child is eligible for independent audit,
                    // but closing it does not resolve any recorded request.
                    Some("idle" | "notLoaded") => {}
                    Some("active") => {
                        if let Some(flags) = params["status"]["activeFlags"].as_array() {
                            for flag in flags {
                                match flag.as_str() {
                                    Some("waitingOnApproval") => {
                                        self.flags.push(Reason::Permission)
                                    }
                                    Some("waitingOnUserInput") => self.flags.push(Reason::Question),
                                    _ => self.uncertain = true,
                                }
                            }
                        } else {
                            self.uncertain = true;
                        }
                    }
                    _ => self.uncertain = true,
                }
            }
            "item/tool/requestUserInput" if params["isBlocking"].as_bool() == Some(false) => {
                return false
            }
            _ => {
                if let Some(id) = request_id(&frame["id"]) {
                    self.work_requests.insert(id);
                } else {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Debug)]
pub struct Update {
    pub signals: Vec<Signal>,
    /// Current safety hold, not a request to change display state. Applies
    /// even when this update contains no signal (for example, a wrong thread).
    pub observation_hold: bool,
    /// Desired state before the common reducer's leave-settle. A runtime
    /// snapshot can serialize this without serializing raw provider payloads.
    pub state: SessionState,
    pub turn_id: Option<String>,
    pub reply: Option<String>,
    pub reply_key: Option<String>,
    /// Only a completed `plan` item supplies authoritative plan text.
    pub plan: Option<String>,
    pub activity: Option<String>,
}

#[derive(Debug)]
pub struct Ledger {
    thread_id: String,
    generation: u64,
    continuous: bool,
    capacity_lost: bool,
    runtime_active: bool,
    active_turn: Option<String>,
    terminal: Option<TurnOutcome>,
    terminal_emitted: bool,
    completed_plan: bool,
    saw_compaction: bool,
    saw_task_item: bool,
    retired_turns: BTreeSet<String>,
    requests: BTreeMap<String, Reason>,
    work_requests: BTreeSet<String>,
    items: BTreeMap<String, PendingItem>,
    hooks: BTreeSet<String>,
    children: BTreeSet<String>,
    child_work: BTreeMap<String, ChildWork>,
    late_work: bool,
    flags: Vec<Reason>,
    last_signal: Option<Signal>,
    state: SessionState,
}

impl Ledger {
    pub fn new(thread_id: String, generation: u64) -> Self {
        Self {
            thread_id,
            generation,
            continuous: false,
            capacity_lost: false,
            runtime_active: false,
            active_turn: None,
            terminal: None,
            terminal_emitted: false,
            completed_plan: false,
            saw_compaction: false,
            saw_task_item: false,
            retired_turns: BTreeSet::new(),
            requests: BTreeMap::new(),
            work_requests: BTreeSet::new(),
            items: BTreeMap::new(),
            hooks: BTreeSet::new(),
            children: BTreeSet::new(),
            child_work: BTreeMap::new(),
            late_work: false,
            flags: Vec::new(),
            last_signal: None,
            state: SessionState::Unknown { reason: UnknownReason::ObservationLost },
        }
    }

    fn held(&self) -> bool {
        !self.continuous
            || self.capacity_lost
            || self.has_pending_work()
            || self.late_work
            || self.plan_dialog_pending()
    }

    fn plan_dialog_pending(&self) -> bool {
        self.completed_plan && self.terminal == Some(TurnOutcome::Completed)
    }

    fn has_pending_work(&self) -> bool {
        !self.requests.is_empty()
            || !self.work_requests.is_empty()
            || !self.items.is_empty()
            || !self.hooks.is_empty()
            || !self.children.is_empty()
            || !self.flags.is_empty()
    }

    fn update(&self) -> Update {
        Update {
            signals: Vec::new(),
            observation_hold: self.held(),
            state: self.state.clone(),
            turn_id: self.active_turn.clone(),
            reply: None,
            reply_key: None,
            plan: None,
            activity: None,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn turn_id(&self) -> Option<&str> {
        self.active_turn.as_deref()
    }

    /// The transport withheld this successful projection behind independent
    /// owned work. Its caller must first prove that this exact turn has never
    /// been published; an already visible completion must not be replayed.
    pub fn defer_terminal_projection(&mut self) {
        if self.terminal == Some(TurnOutcome::Completed) {
            self.terminal_emitted = false;
        }
    }

    /// Local half of a reconnect audit. The caller must additionally prove
    /// parent/descendant runtime idleness and uninterrupted capture during the
    /// audit. This intentionally ignores continuity itself, but never ignores
    /// unresolved requests, tools, hooks, children or a still-active turn.
    pub fn can_reconcile_idle(&self) -> bool {
        !self.capacity_lost
            && !self.has_pending_work()
            && !self.late_work
            && !self.plan_dialog_pending()
            && !self.runtime_active
            && (self.active_turn.is_none() || self.terminal.is_some())
    }

    /// All discovered descendants, including those previously audited idle.
    /// Keeping their identities lets late activity reassert the safety hold.
    pub fn known_children(&self) -> Vec<String> {
        self.child_work.iter().filter(|(_, work)| work.known).map(|(id, _)| id.clone()).collect()
    }

    /// Local half of a complete metadata audit. Child identity/activity hints
    /// and a plan dialog do not prevent an audit, but unresolved work does.
    pub fn can_reconcile_work(&self) -> bool {
        !self.capacity_lost
            && self.requests.is_empty()
            && self.work_requests.is_empty()
            && self.items.is_empty()
            && self.hooks.is_empty()
            && self.flags.is_empty()
            && self.child_work.values().filter(|work| work.known).all(|work| !work.pending())
    }

    pub fn needs_work_audit(&self) -> bool {
        !self.continuous || !self.children.is_empty() || self.late_work
    }

    /// Caller proves the parent and every known/loaded descendant quiescent
    /// with uninterrupted capture. Preserve the observed parent result/plan;
    /// metadata alone can never synthesize a new successful completion.
    pub fn reconcile_work(&mut self, generation: u64, thread: &Value) -> Update {
        let mut update = self.update();
        if generation != self.generation
            || string(thread, "id") != Some(&self.thread_id)
            || string(&thread["status"], "type") != Some("idle")
            || !self.can_reconcile_work()
        {
            return update;
        }
        self.continuous = true;
        self.runtime_active = false;
        self.children.clear();
        self.late_work = false;
        if self.terminal.is_none() {
            self.active_turn = None;
        }
        self.project(&mut update);
        update
    }

    fn emit(&mut self, signal: Signal, update: &mut Update) {
        self.state = match &signal {
            Signal::Ready => SessionState::Idle { stop_reason: StopReason::Unknown },
            Signal::TurnStarted => SessionState::Running,
            Signal::TurnEnded { outcome: TurnOutcome::Completed } => {
                SessionState::Idle { stop_reason: StopReason::EndTurn }
            }
            Signal::TurnEnded { outcome: TurnOutcome::Interrupted } => {
                SessionState::Idle { stop_reason: StopReason::Interrupted }
            }
            Signal::TurnEnded { outcome: TurnOutcome::Failed(reason) } => {
                SessionState::Failed { reason: *reason }
            }
            Signal::Attention { reason } => SessionState::RequiresAction { reason: *reason },
            Signal::ObservationLost => {
                SessionState::Unknown { reason: UnknownReason::ObservationLost }
            }
            _ => unreachable!("the Codex adapter emits only normalized signals"),
        };
        update.state = self.state.clone();
        if self.last_signal.as_ref() != Some(&signal) {
            self.last_signal = Some(signal.clone());
            update.signals.push(signal);
        }
    }

    fn project(&mut self, update: &mut Update) {
        let reason = attention_reasons(&self.requests, &self.flags)
            .chain(
                self.child_work
                    .values()
                    .filter(|work| work.known)
                    .flat_map(|work| attention_reasons(&work.requests, &work.flags)),
            )
            .copied()
            .min_by_key(|r| rank(&SessionState::RequiresAction { reason: *r }));
        if let Some(reason) = reason {
            self.emit(Signal::Attention { reason }, update);
        } else if !self.continuous || self.capacity_lost {
            self.emit(Signal::ObservationLost, update);
        } else if self.plan_dialog_pending() && !self.has_pending_work() && !self.late_work {
            // Native Codex opens a local implementation dialog after the plan
            // turn; no server approval request accompanies it. The daemon may
            // release this hold only after observing that dialog close onto
            // the native composer, or a subsequent structured turn starts.
            self.emit(Signal::Attention { reason: Reason::Plan }, update);
        } else if let Some(outcome) = self.terminal {
            if !self.has_pending_work() && !self.late_work {
                if !self.terminal_emitted {
                    if outcome == TurnOutcome::Completed
                        && self.saw_compaction
                        && !self.saw_task_item
                    {
                        // Native /compact is a real turn with a completed
                        // contextCompaction item, but completes maintenance,
                        // not the user's ticket. Automatic compaction inside
                        // a task keeps its user/output/tool evidence below.
                        self.emit(Signal::Ready, update);
                    } else {
                        self.emit(Signal::TurnEnded { outcome }, update);
                    }
                    self.terminal_emitted = true;
                } else if self.state == SessionState::Running {
                    // Late work has now drained and been audited. The prior
                    // success was already emitted; do not repeat automation.
                    self.emit(Signal::Ready, update);
                }
            } else {
                self.emit(Signal::TurnStarted, update);
            }
        } else if self.runtime_active
            || self.active_turn.is_some()
            || self.has_pending_work()
            || self.late_work
        {
            self.emit(Signal::TurnStarted, update);
        } else {
            self.emit(Signal::Ready, update);
        }
        update.observation_hold = self.held();
        update.turn_id = self.active_turn.clone();
    }

    fn enforce_bounds(&mut self) {
        let count = self.retired_turns.len()
            + self.requests.len()
            + self.work_requests.len()
            + self.items.len()
            + self.hooks.len()
            + self.children.len()
            + self.child_work.values().map(ChildWork::identities).sum::<usize>();
        if count > MAX_IDENTITIES {
            // Stop accumulating IDs and fail closed. A fresh audited snapshot
            // is the only safe way to re-establish the baseline.
            self.capacity_lost = true;
            self.continuous = false;
        }
    }

    /// A gap invalidates a completion still settling in the shared reducer.
    /// It does not forget known work or let an idle snapshot erase it.
    pub fn lost(&mut self, generation: u64) -> Update {
        let mut update = self.update();
        if generation != self.generation {
            return update;
        }
        self.continuous = false;
        self.terminal = None;
        self.terminal_emitted = false;
        self.emit(Signal::ObservationLost, &mut update);
        update.observation_hold = true;
        update
    }

    /// `thread` is the actual Thread object from a read/resume response.
    /// `work_reconciled` asserts an independent complete request/descendant
    /// audit, not merely `thread.status == idle`. False never releases a hold.
    pub fn reconcile(&mut self, generation: u64, thread: &Value, work_reconciled: bool) -> Update {
        let mut update = self.update();
        if generation != self.generation || string(thread, "id") != Some(&self.thread_id) {
            return update;
        }
        let status = &thread["status"];
        let kind = string(status, "type");
        self.continuous = work_reconciled && matches!(kind, Some("idle" | "active"));
        self.completed_plan = false;
        self.saw_compaction = false;
        self.saw_task_item = false;
        // Historical completion must not trigger a new automatic column move.
        self.terminal = None;
        self.terminal_emitted = false;
        if self.continuous && kind == Some("idle") {
            self.active_turn = None;
            self.runtime_active = false;
            self.requests.clear();
            self.work_requests.clear();
            self.items.clear();
            self.hooks.clear();
            self.children.clear();
            self.child_work.clear();
            self.late_work = false;
            self.flags.clear();
            self.capacity_lost = false;
            self.retired_turns.clear();
        }
        if let Some(turns) = thread["turns"].as_array() {
            for turn in turns {
                if let Some(id) = string(turn, "id") {
                    if string(turn, "status") == Some("inProgress") {
                        self.retired_turns.remove(id);
                        self.active_turn = Some(id.into());
                        if let Some(items) = turn["items"].as_array() {
                            for item in items {
                                self.observe_item(item, false, &mut update);
                            }
                        }
                    } else if self.retired_turns.len() < MAX_IDENTITIES {
                        self.retired_turns.insert(id.into());
                    }
                }
            }
        }
        self.read_flags(status);
        self.enforce_bounds();
        self.project(&mut update);
        update
    }

    /// A notification or server request from the already initialized observer
    /// connection. Responses are handled by the transport and never guessed at
    /// here. Request IDs remain opaque and are never answered by Mesimon.
    pub fn observe(&mut self, generation: u64, frame: &Value) -> Update {
        let mut update = self.update();
        if generation != self.generation || self.capacity_lost {
            return update;
        }
        let Some(method) = string(frame, "method") else { return update };
        let params = &frame["params"];
        let Some(thread) = string(params, "threadId") else { return update };
        if thread != self.thread_id {
            let newly_seen = !self.child_work.contains_key(thread);
            let work = self.child_work.entry(thread.into()).or_default();
            let changed = work.observe(method, frame);
            let known = work.known;
            if newly_seen && !changed {
                self.child_work.remove(thread);
            }
            if known && changed {
                self.children.insert(thread.into());
                self.discover_descendants(thread);
            }
            self.enforce_bounds();
            if known || self.capacity_lost {
                self.project(&mut update);
            }
            return update;
        }
        if let Some(turn) = string(params, "turnId") {
            if self.retired_turns.contains(turn)
                || self.active_turn.as_deref().is_some_and(|active| active != turn)
            {
                return update;
            }
            if self.active_turn.is_none()
                && matches!(method, "item/started" | "item/completed" | "item/agentMessage/delta")
            {
                // The native runtime can emit scoped items before any
                // turn/started notification reaches this connection.
                self.active_turn = Some(turn.into());
                self.late_work = false;
            }
        }
        if self.terminal_emitted
            && (frame.get("id").is_some()
                || matches!(method, "item/started" | "hook/started")
                || (method == "thread/status/changed"
                    && string(&params["status"], "type") == Some("active")))
        {
            self.late_work = true;
        }
        if let Some(reason) = request_reason(method, params) {
            if let Some(id) = request_id(&frame["id"]) {
                self.requests.insert(id, reason);
                if self.terminal_emitted {
                    self.terminal = None;
                    self.terminal_emitted = false;
                }
                if self.active_turn.is_none() {
                    self.active_turn = string(params, "turnId").map(str::to_owned);
                }
            } else {
                self.continuous = false;
            }
        } else {
            match method {
                "turn/started" => {
                    let Some(id) = string(&params["turn"], "id") else {
                        return self.lost(generation);
                    };
                    if self.retired_turns.contains(id) {
                        return update;
                    }
                    if self.active_turn.as_deref() != Some(id) {
                        if let Some(old) = self.active_turn.replace(id.into()) {
                            self.retired_turns.insert(old);
                        }
                        self.terminal = None;
                        self.terminal_emitted = false;
                        self.completed_plan = false;
                        self.saw_compaction = false;
                        self.saw_task_item = false;
                        self.late_work = false;
                    }
                    self.runtime_active = true;
                }
                "turn/completed" => {
                    let turn = &params["turn"];
                    if string(turn, "id") != self.active_turn.as_deref() || self.terminal.is_some()
                    {
                        return update;
                    }
                    let Some(result) = outcome(turn) else { return self.lost(generation) };
                    if let Some(items) = turn["items"].as_array() {
                        for item in items {
                            self.observe_item(item, true, &mut update);
                        }
                    }
                    retire_cancelled_compaction(&mut self.items, turn);
                    self.terminal = Some(result);
                    self.runtime_active = false;
                }
                "serverRequest/resolved" => {
                    if let Some(id) = request_id(&params["requestId"]) {
                        self.requests.remove(&id);
                        self.work_requests.remove(&id);
                    }
                }
                "thread/status/changed" => {
                    if string(&params["status"], "type") == Some("active")
                        && !self.runtime_active
                        && self.terminal.is_some()
                    {
                        if let Some(old) = self.active_turn.take() {
                            self.retired_turns.insert(old);
                        }
                        self.terminal = None;
                        self.terminal_emitted = false;
                        self.completed_plan = false;
                        self.saw_compaction = false;
                        self.saw_task_item = false;
                    }
                    self.read_flags(&params["status"]);
                    match string(&params["status"], "type") {
                        // Native API failures report systemError around their
                        // failed turn. This is known runtime status, not a
                        // missing observation: keep the authoritative turn
                        // outcome and all outstanding work/interaction holds.
                        Some("idle" | "active" | "systemError") => {}
                        _ => return self.lost(generation),
                    }
                }
                "item/started" | "item/completed" => {
                    self.observe_item(&params["item"], method == "item/completed", &mut update);
                }
                "hook/started" | "hook/completed" => {
                    let run = &params["run"];
                    let Some(id) = string(run, "id") else { return self.lost(generation) };
                    if method == "hook/started" {
                        self.hooks.insert(id.into());
                        // A Stop hook after turn/completed can continue it.
                        // Reassert work so a pending leave is cancelled.
                        if self.terminal_emitted {
                            self.terminal = None;
                            self.terminal_emitted = false;
                        }
                    } else {
                        self.hooks.remove(id);
                        if matches!(string(run, "status"), Some("blocked" | "stopped")) {
                            self.terminal = None;
                            self.terminal_emitted = false;
                        }
                    }
                }
                // Text deltas are not authoritative final messages or plans.
                // Activity is intentionally descriptive, never model text.
                "item/agentMessage/delta" => update.activity = Some("replying".into()),
                "item/plan/delta" | "turn/plan/updated" => {
                    update.activity = Some("planning".into());
                }
                "error" if params["willRetry"].as_bool() == Some(true) => {
                    update.activity = Some("retrying".into());
                }
                "item/tool/requestUserInput" if params["isBlocking"].as_bool() == Some(false) => {
                    return update;
                }
                _ => {
                    // Unrecognized scoped server requests still represent
                    // work. Do not guess that an unknown method is harmless,
                    // nor mislabel provider/tool callbacks as human dialogs.
                    if let Some(id) = request_id(&frame["id"]) {
                        self.work_requests.insert(id);
                    } else {
                        return update;
                    }
                }
            }
        }
        self.enforce_bounds();
        self.project(&mut update);
        update
    }

    fn read_flags(&mut self, status: &Value) {
        self.flags.clear();
        self.runtime_active = string(status, "type") == Some("active");
        if string(status, "type") != Some("active") {
            return;
        }
        let Some(flags) = status["activeFlags"].as_array() else {
            self.continuous = false;
            return;
        };
        for flag in flags {
            match flag.as_str() {
                Some("waitingOnApproval") => self.flags.push(Reason::Permission),
                Some("waitingOnUserInput") => self.flags.push(Reason::Question),
                _ => self.continuous = false,
            }
        }
    }

    fn observe_item(&mut self, item: &Value, completed: bool, update: &mut Update) {
        let Some(id) = string(item, "id") else { return };
        let Some(kind) = string(item, "type") else { return };
        let terminal = item_terminal(item, completed);
        if kind == "contextCompaction" {
            self.saw_compaction = true;
        } else if !matches!(kind, "reasoning" | "hookPrompt") {
            self.saw_task_item = true;
        }
        match kind {
            "agentMessage" | "plan" => {
                if terminal {
                    if let Some(text) = string(item, "text") {
                        let text = text.chars().take(MAX_PREVIEW_CHARS).collect();
                        if kind == "plan" {
                            self.completed_plan = true;
                            update.plan = Some(text);
                        } else {
                            update.reply = Some(text);
                            update.reply_key = Some(id.into());
                        }
                    }
                }
            }
            "userMessage" | "reasoning" | "hookPrompt" => {}
            _ => {
                if terminal {
                    self.items.remove(id);
                } else {
                    self.items
                        .insert(id.into(), PendingItem::new(kind, self.active_turn.as_deref()));
                }
                update.activity = Some(kind.chars().take(80).collect());
            }
        }
        if kind == "collabAgentToolCall" {
            for child in receiver_ids(item) {
                if child == self.thread_id {
                    continue;
                }
                self.child_work.entry(child.clone()).or_default().known = true;
                // Even agentsStates=completed is only a hint. Independent
                // runtime metadata plus an empty local ledger releases work.
                self.children.insert(child.clone());
                self.discover_descendants(&child);
            }
        }
    }

    fn discover_descendants(&mut self, child: &str) {
        let mut queue = vec![child.to_owned()];
        let mut visited = BTreeSet::new();
        while let Some(id) = queue.pop() {
            if id == self.thread_id || !visited.insert(id.clone()) {
                continue;
            }
            if visited.len() > MAX_IDENTITIES {
                self.capacity_lost = true;
                self.continuous = false;
                break;
            }
            let work = self.child_work.entry(id.clone()).or_default();
            if !work.known {
                work.known = true;
                self.children.insert(id);
            }
            queue.extend(work.descendants.iter().cloned());
        }
    }
}

fn item_terminal(item: &Value, completed: bool) -> bool {
    match string(item, "status") {
        Some("inProgress" | "running" | "pendingInit") => false,
        Some("completed" | "failed" | "interrupted") => true,
        _ => completed,
    }
}

fn receiver_ids(item: &Value) -> impl Iterator<Item = String> + '_ {
    item["receiverThreadIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(MAX_IDENTITIES + 1)
        .map(str::to_owned)
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn request_id(value: &Value) -> Option<String> {
    (value.is_string() || value.is_i64() || value.is_u64()).then(|| value.to_string())
}

fn outcome(turn: &Value) -> Option<TurnOutcome> {
    Some(match string(turn, "status")? {
        "completed" => TurnOutcome::Completed,
        "interrupted" => TurnOutcome::Interrupted,
        "failed" => TurnOutcome::Failed(FailReason::Unknown),
        _ => return None,
    })
}

fn request_reason(method: &str, params: &Value) -> Option<Reason> {
    match method {
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval" => Some(Reason::Permission),
        "item/tool/requestUserInput" if params["isBlocking"].as_bool() != Some(false) => {
            let secret = params["questions"].as_array().is_some_and(|questions| {
                questions.iter().any(|q| q["isSecret"].as_bool() == Some(true))
            });
            Some(if secret { Reason::Secret } else { Reason::Question })
        }
        "mcpServer/elicitation/request" => {
            // Codex uses the MCP form transport for native one-tool approval
            // as well as actual server elicitation. Captured 0.153.4 metadata
            // distinguishes them; the form's title/text is not authority.
            Some(if params["_meta"]["codex_approval_kind"] == "mcp_tool_call" {
                Reason::Permission
            } else {
                Reason::Elicitation
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(status: Value) -> Value {
        json!({"id":"thread-a", "status":status, "turns":[]})
    }

    fn ready() -> Ledger {
        let mut ledger = Ledger::new("thread-a".into(), 7);
        let update = ledger.reconcile(7, &snapshot(json!({"type":"idle"})), true);
        assert_eq!(update.signals, vec![Signal::Ready]);
        assert!(!update.observation_hold);
        ledger
    }

    fn frame(method: &str, params: Value) -> Value {
        let mut params = params;
        params["threadId"] = json!("thread-a");
        json!({"method":method, "params":params})
    }

    fn start(ledger: &mut Ledger, id: &str) -> Update {
        ledger.observe(
            7,
            &frame("turn/started", json!({"turn":{"id":id,"status":"inProgress","items":[]}})),
        )
    }

    fn complete(ledger: &mut Ledger, id: &str, status: &str) -> Update {
        ledger.observe(
            7,
            &frame("turn/completed", json!({"turn":{"id":id,"status":status,"items":[]}})),
        )
    }

    fn request(ledger: &mut Ledger, id: Value, method: &str) -> Update {
        let mut f = frame(method, json!({"turnId":"turn-1","isBlocking":true}));
        f["id"] = id;
        ledger.observe(7, &f)
    }

    fn resolve(ledger: &mut Ledger, id: Value) -> Update {
        ledger.observe(7, &frame("serverRequest/resolved", json!({"requestId":id})))
    }

    #[test]
    fn successful_turn_is_once_and_wrong_identity_never_changes_it() {
        let mut ledger = ready();
        assert_eq!(start(&mut ledger, "turn-1").signals, vec![Signal::TurnStarted]);
        let mut wrong = frame(
            "turn/completed",
            json!({"turn":{"id":"turn-1","status":"completed","items":[]}}),
        );
        wrong["params"]["threadId"] = json!("other-thread");
        assert!(ledger.observe(7, &wrong).signals.is_empty());
        wrong["params"]["threadId"] = json!("thread-a");
        assert!(ledger.observe(6, &wrong).signals.is_empty());
        assert!(complete(&mut ledger, "old-turn", "completed").signals.is_empty());
        let update = complete(&mut ledger, "turn-1", "completed");
        assert_eq!(update.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]);
        assert!(!update.observation_hold);
        assert_eq!(update.turn_id.as_deref(), Some("turn-1"));
        assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
        start(&mut ledger, "turn-2");
        assert!(start(&mut ledger, "turn-1").signals.is_empty());
        assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
        assert_eq!(ledger.turn_id(), Some("turn-2"));
    }

    #[test]
    fn parallel_requests_keep_the_highest_priority_until_its_own_resolution() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        request(&mut ledger, json!(1), "item/commandExecution/requestApproval");
        request(&mut ledger, json!("1"), "item/tool/requestUserInput");
        assert_eq!(ledger.state(), &SessionState::RequiresAction { reason: Reason::Permission });
        assert!(complete(&mut ledger, "turn-1", "completed").observation_hold);
        let resolved = resolve(&mut ledger, json!(1));
        assert_eq!(resolved.signals, vec![Signal::Attention { reason: Reason::Question }]);
        assert!(resolved.observation_hold);
        assert!(resolve(&mut ledger, json!(1)).signals.is_empty());
        let last = resolve(&mut ledger, json!("1"));
        assert_eq!(last.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]);
        assert!(!last.observation_hold);
    }

    #[test]
    fn failed_and_interrupted_outcomes_cannot_be_relabelled_successful() {
        for (status, expected) in [
            ("failed", TurnOutcome::Failed(FailReason::Unknown)),
            ("interrupted", TurnOutcome::Interrupted),
        ] {
            let mut ledger = ready();
            start(&mut ledger, "turn-1");
            assert_eq!(
                complete(&mut ledger, "turn-1", status).signals,
                vec![Signal::TurnEnded { outcome: expected }]
            );
            assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
            assert!(!matches!(
                ledger.state(),
                SessionState::Idle { stop_reason: StopReason::EndTurn }
            ));
        }
    }

    #[test]
    fn loss_requires_work_audit_and_snapshots_never_reemit_historical_success() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        complete(&mut ledger, "turn-1", "completed");
        assert_eq!(ledger.lost(7).signals, vec![Signal::ObservationLost]);
        let mut idle = snapshot(json!({"type":"idle"}));
        idle["turns"] = json!([{"id":"turn-1","status":"completed","items":[]}]);
        assert!(ledger.reconcile(7, &idle, false).observation_hold);
        assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
        let audited = ledger.reconcile(7, &idle, true);
        assert_eq!(audited.signals, vec![Signal::Ready]);
        assert!(!audited.observation_hold);
        assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
    }

    #[test]
    fn active_snapshot_flags_are_attention_not_success() {
        let mut ledger = Ledger::new("thread-a".into(), 7);
        let update = ledger.reconcile(
            7,
            &snapshot(
                json!({"type":"active","activeFlags":["waitingOnUserInput","waitingOnApproval"]}),
            ),
            true,
        );
        assert_eq!(update.signals, vec![Signal::Attention { reason: Reason::Permission }]);
        assert!(update.observation_hold);
        let next = ledger.observe(
            7,
            &frame(
                "thread/status/changed",
                json!({"status":{"type":"active","activeFlags":["waitingOnUserInput"]}}),
            ),
        );
        assert_eq!(next.signals, vec![Signal::Attention { reason: Reason::Question }]);
        let next = ledger.observe(
            7,
            &frame("thread/status/changed", json!({"status":{"type":"active","activeFlags":[]}})),
        );
        assert_eq!(next.signals, vec![Signal::TurnStarted]);
    }

    #[test]
    fn known_child_completion_requires_independent_audit_and_parent_completion() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let child = json!({"type":"collabAgentToolCall","id":"spawn","status":"completed","receiverThreadIds":["child"],"agentsStates":{"child":{"status":"running"}}});
        let started =
            ledger.observe(7, &frame("item/completed", json!({"turnId":"turn-1","item":child})));
        assert!(started.observation_hold);
        let finish = json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"child-turn","status":"completed","items":[]}}});
        assert!(ledger.observe(7, &finish).signals.is_empty());
        assert!(ledger.observe(7, &json!({"method":"turn/started","params":{"threadId":"child","turn":{"id":"child-turn","status":"inProgress"}}})).observation_hold);
        let child_done = ledger.observe(7, &finish);
        assert!(child_done.signals.is_empty(), "a child cannot complete the parent");
        assert!(child_done.observation_hold, "child terminal is not a work audit");
        assert_eq!(ledger.state(), &SessionState::Running);
        assert!(complete(&mut ledger, "turn-1", "completed").observation_hold);
        assert_eq!(
            ledger.reconcile_work(7, &snapshot(json!({"type":"idle"}))).signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    fn child_frame(method: &str, params: Value) -> Value {
        let mut frame = frame(method, params);
        frame["params"]["threadId"] = json!("child");
        frame
    }

    fn discover_child(ledger: &mut Ledger) -> Update {
        // A terminal summary is deliberately not sufficient to clear work.
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"turnId":"turn-1","item":{
                    "id":"spawn", "type":"collabAgentToolCall", "status":"completed",
                    "receiverThreadIds":["child"], "agentsStates":{"child":{"status":"completed"}}
                }}),
            ),
        )
    }

    #[test]
    fn child_terminal_keeps_pre_discovery_approval_tool_and_hook_until_resolved_and_audited() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let mut approval =
            child_frame("item/commandExecution/requestApproval", json!({"turnId":"child-turn"}));
        approval["id"] = json!(5);
        let early = ledger.observe(7, &approval);
        assert!(!early.observation_hold, "unrelated thread is not yet a known descendant");
        ledger.observe(7, &child_frame("item/started", json!({"turnId":"child-turn","item":{"id":"tool","type":"commandExecution","status":"inProgress"}})));
        ledger.observe(7, &child_frame("hook/started", json!({"run":{"id":"hook"}})));
        let discovered = discover_child(&mut ledger);
        assert_eq!(discovered.signals, vec![Signal::Attention { reason: Reason::Permission }]);
        assert_eq!(ledger.known_children(), vec!["child"]);
        let terminal = child_frame(
            "turn/completed",
            json!({"turn":{"id":"child-turn","status":"completed","items":[]}}),
        );
        assert!(ledger.observe(7, &terminal).observation_hold);
        complete(&mut ledger, "turn-1", "completed");
        assert!(!ledger.can_reconcile_work());
        assert!(ledger.reconcile_work(7, &snapshot(json!({"type":"idle"}))).observation_hold);
        ledger.observe(7, &child_frame("serverRequest/resolved", json!({"requestId":5})));
        assert!(!ledger.can_reconcile_work(), "the tool and hook remain");
        ledger.observe(
            7,
            &child_frame(
                "item/completed",
                json!({"item":{"id":"tool","type":"commandExecution","status":"completed"}}),
            ),
        );
        assert!(!ledger.can_reconcile_work(), "the hook remains");
        let drained = ledger.observe(
            7,
            &child_frame("hook/completed", json!({"run":{"id":"hook","status":"completed"}})),
        );
        assert!(drained.observation_hold);
        assert!(ledger.can_reconcile_work());
        assert!(ledger.needs_work_audit());
        let audited = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
        assert_eq!(audited.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]);
        assert!(!audited.observation_hold);
        assert!(!ledger.needs_work_audit());
        assert_eq!(ledger.known_children(), vec!["child"], "retain identity for late work");
        assert!(
            ledger
                .observe(7, &child_frame("hook/started", json!({"run":{"id":"late"}})))
                .observation_hold
        );
    }

    #[test]
    fn closed_child_still_requires_pending_request_resolution_and_work_audit() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        discover_child(&mut ledger);
        let mut approval = child_frame("item/commandExecution/requestApproval", json!({}));
        approval["id"] = json!(5);
        ledger.observe(7, &approval);
        complete(&mut ledger, "turn-1", "completed");
        let closed = ledger.observe(
            7,
            &child_frame("thread/status/changed", json!({"status":{"type":"notLoaded"}})),
        );
        assert!(closed.observation_hold);
        assert!(!ledger.can_reconcile_work());
        ledger.observe(7, &child_frame("serverRequest/resolved", json!({"requestId":5})));
        assert!(ledger.can_reconcile_work(), "normal unload does not poison later audit");
        assert!(ledger.needs_work_audit());
        let audited = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
        assert!(!audited.observation_hold);
        assert_eq!(audited.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]);
    }

    #[test]
    fn early_child_start_and_completion_need_no_repeated_start_to_be_auditable() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        ledger.observe(
            7,
            &child_frame("turn/started", json!({"turn":{"id":"child-turn","status":"inProgress"}})),
        );
        discover_child(&mut ledger);
        ledger.observe(
            7,
            &child_frame(
                "turn/completed",
                json!({"turn":{"id":"child-turn","status":"completed","items":[]}}),
            ),
        );
        assert!(complete(&mut ledger, "turn-1", "completed").observation_hold);
        assert!(ledger.can_reconcile_work());
        assert!(ledger.reconcile_work(6, &snapshot(json!({"type":"idle"}))).observation_hold);
        assert!(
            ledger
                .reconcile_work(7, &json!({"id":"foreign","status":{"type":"idle"}}))
                .observation_hold
        );
        assert!(
            ledger
                .reconcile_work(7, &snapshot(json!({"type":"active","activeFlags":[]})))
                .observation_hold
        );
        assert!(!ledger.reconcile_work(7, &snapshot(json!({"type":"idle"}))).observation_hold);
    }

    #[test]
    fn pre_discovery_grandchild_requests_are_promoted_transitively_and_bounded() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let mut request = child_frame("item/futureTool/request", json!({}));
        request["params"]["threadId"] = json!("grandchild");
        request["id"] = json!(7);
        ledger.observe(7, &request);
        ledger.observe(7, &child_frame("item/completed", json!({"item":{
            "id":"spawn-grandchild","type":"collabAgentToolCall","status":"completed","receiverThreadIds":["grandchild"]}})));
        discover_child(&mut ledger);
        assert_eq!(ledger.known_children(), vec!["child", "grandchild"]);
        assert!(!ledger.can_reconcile_work());
        for id in 0..=MAX_IDENTITIES {
            let mut frame =
                child_frame("turn/started", json!({"turn":{"id":"t","status":"inProgress"}}));
            frame["params"]["threadId"] = json!(format!("unrelated-{id}"));
            ledger.observe(7, &frame);
        }
        assert!(ledger.capacity_lost);
        let count = ledger.child_work.len();
        let mut extra = child_frame("turn/started", json!({"turn":{"id":"t"}}));
        extra["params"]["threadId"] = json!("exhausted");
        assert!(ledger.observe(7, &extra).observation_hold);
        assert_eq!(ledger.child_work.len(), count);
        assert!(!ledger.can_reconcile_work());
    }

    #[test]
    fn work_audit_preserves_plan_and_never_repeats_a_published_completion() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"item":{"id":"plan","type":"plan","text":"Synthetic plan"}}),
            ),
        );
        discover_child(&mut ledger);
        complete(&mut ledger, "turn-1", "completed");
        assert!(ledger.can_reconcile_work(), "a plan does not bar a work audit");
        let plan = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
        assert!(plan.observation_hold);
        assert_eq!(plan.state, SessionState::RequiresAction { reason: Reason::Plan });
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        assert_eq!(
            complete(&mut ledger, "turn-1", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
        let working = ledger.observe(7, &frame("item/started", json!({"turnId":"turn-1","item":{"id":"late","type":"commandExecution","status":"inProgress"}})));
        assert_eq!(working.signals, vec![Signal::TurnStarted]);
        let done = ledger.observe(7, &frame("item/completed", json!({"turnId":"turn-1","item":{"id":"late","type":"commandExecution","status":"completed"}})));
        assert!(done.observation_hold, "late work needs independent idle proof");
        assert!(done.signals.is_empty());
        let idle = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
        assert!(!idle.observation_hold);
        assert_eq!(idle.signals, vec![Signal::Ready]);
        assert!(ledger.reconcile_work(7, &snapshot(json!({"type":"idle"}))).signals.is_empty());
    }

    #[test]
    fn outstanding_item_and_stop_continuation_prevent_early_completion() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let running = json!({"type":"commandExecution","id":"command","status":"inProgress"});
        ledger.observe(7, &frame("item/started", json!({"turnId":"turn-1","item":running})));
        let completed = ledger.observe(
            7,
            &frame(
                "turn/completed",
                json!({"turn":{"id":"turn-1","status":"completed","items":[running]}}),
            ),
        );
        assert!(completed.signals.is_empty());
        assert!(completed.observation_hold);
        ledger.observe(7, &frame("hook/started", json!({"turnId":"turn-1","run":{"id":"stop","eventName":"stop","status":"running"}})));
        ledger.observe(7, &frame("hook/completed", json!({"turnId":"turn-1","run":{"id":"stop","eventName":"stop","status":"blocked"}})));
        let command_done = ledger.observe(7, &frame("item/completed", json!({"turnId":"turn-1","item":{"type":"commandExecution","id":"command","status":"completed"}})));
        assert!(command_done.signals.is_empty());
        assert_eq!(ledger.state(), &SessionState::Running);
        assert_eq!(
            complete(&mut ledger, "turn-1", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    #[test]
    fn completed_plan_and_reply_are_bounded_and_keep_opaque_item_identity() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let delta = ledger.observe(
            7,
            &frame(
                "item/plan/delta",
                json!({"turnId":"turn-1","itemId":"plan","delta":"proposed"}),
            ),
        );
        assert!(delta.plan.is_none());
        let plan = ledger.observe(7, &frame("item/completed", json!({"turnId":"turn-1","item":{"type":"plan","id":"plan","text":"authoritative plan"}})));
        assert_eq!(plan.plan.as_deref(), Some("authoritative plan"));
        assert_eq!(ledger.state(), &SessionState::Running, "plan text is not a blocking approval");
        let reply = ledger.observe(7, &frame("item/completed", json!({"turnId":"turn-1","item":{"type":"agentMessage","id":"msg-opaque","text":"א".repeat(MAX_PREVIEW_CHARS + 1)}})));
        assert_eq!(reply.reply_key.as_deref(), Some("msg-opaque"));
        assert_eq!(reply.reply.unwrap().chars().count(), MAX_PREVIEW_CHARS);
    }

    #[test]
    fn plan_dialog_dismissal_is_not_eligible_until_child_work_is_audited() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"item":{"id":"plan","type":"plan","text":"Synthetic plan"}}),
            ),
        );
        discover_child(&mut ledger);
        let done = complete(&mut ledger, "turn-1", "completed");
        assert_eq!(done.state, SessionState::Running);
        assert!(done.observation_hold);
        let plan = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
        assert_eq!(plan.state, SessionState::RequiresAction { reason: Reason::Plan });
        assert!(plan.observation_hold);
        let late = ledger.observe(7, &child_frame("hook/started", json!({"run":{"id":"late"}})));
        assert_eq!(late.state, SessionState::Running);
        assert!(late.observation_hold);
    }

    #[test]
    fn completed_plan_holds_until_a_new_native_turn_without_successful_completion() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"turnId":"turn-1","item":{"type":"plan","id":"plan","text":"the plan"}}),
            ),
        );
        let done = complete(&mut ledger, "turn-1", "completed");
        assert_eq!(done.signals, vec![Signal::Attention { reason: Reason::Plan }]);
        assert!(done.observation_hold);
        assert!(!ledger.can_reconcile_idle());
        let idle =
            ledger.observe(7, &frame("thread/status/changed", json!({"status":{"type":"idle"}})));
        assert!(idle.observation_hold, "server idleness does not dismiss a local dialog");
        assert!(idle.signals.is_empty());
        let accepted = start(&mut ledger, "turn-2");
        assert_eq!(accepted.signals, vec![Signal::TurnStarted]);
        assert!(!accepted.observation_hold);
        assert_eq!(
            complete(&mut ledger, "turn-2", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    #[test]
    fn native_item_before_turn_started_establishes_scoped_activity() {
        // Shape/order observed in connected capture d06076d5 (Codex 0.153.4):
        // active status, userMessage item, final answer, turn/completed; the
        // second observer received no turn/started notification.
        let mut ledger = ready();
        let active = ledger.observe(
            7,
            &frame("thread/status/changed", json!({"status":{"type":"active","activeFlags":[]}})),
        );
        assert_eq!(active.signals, vec![Signal::TurnStarted]);
        ledger.observe(
            7,
            &frame(
                "item/started",
                json!({"turnId":"turn-1","item":{"type":"userMessage","id":"user","content":[]}}),
            ),
        );
        let done = complete(&mut ledger, "turn-1", "completed");
        assert_eq!(done.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]);
        ledger.observe(
            7,
            &frame("thread/status/changed", json!({"status":{"type":"active","activeFlags":[]}})),
        );
        ledger.observe(
            7,
            &frame(
                "item/started",
                json!({"turnId":"turn-2","item":{"type":"userMessage","id":"user2","content":[]}}),
            ),
        );
        assert_eq!(ledger.turn_id(), Some("turn-2"));
        assert_eq!(
            complete(&mut ledger, "turn-2", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    #[test]
    fn nonblocking_question_and_unrelated_system_threads_do_not_hold_checkout() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let mut question =
            frame("item/tool/requestUserInput", json!({"turnId":"turn-1","isBlocking":false}));
        question["id"] = json!(1);
        assert!(!ledger.observe(7, &question).observation_hold);
        let foreign = json!({"method":"thread/started","params":{"thread":{"id":"system","parentThreadId":null,"ephemeral":true,"threadSource":"memoryConsolidation"}}});
        assert!(ledger.observe(7, &foreign).signals.is_empty());
        assert_eq!(
            complete(&mut ledger, "turn-1", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    #[test]
    fn unknown_scoped_requests_hold_without_inventing_human_attention() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let mut request = frame("item/futureTool/request", json!({"turnId":"turn-1"}));
        request["id"] = json!(23);
        let update = ledger.observe(7, &request);
        assert!(update.observation_hold);
        assert_eq!(ledger.state(), &SessionState::Running);
        assert!(complete(&mut ledger, "turn-1", "completed").signals.is_empty());
        assert_eq!(
            resolve(&mut ledger, json!(23)).signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
        let unknown_status = ledger.observe(
            7,
            &frame("thread/status/changed", json!({"status":{"type":"future_status"}})),
        );
        assert!(unknown_status.observation_hold);
        assert_eq!(unknown_status.signals, vec![Signal::ObservationLost]);
    }

    #[test]
    fn native_mcp_tool_approval_is_distinct_from_server_elicitation() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let mut approval = frame(
            "mcpServer/elicitation/request",
            json!({"turnId":"turn-1","mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call"}}),
        );
        approval["id"] = json!(101);
        assert_eq!(
            ledger.observe(7, &approval).signals,
            vec![Signal::Attention { reason: Reason::Permission }]
        );
        assert!(ledger.observe(7, &approval).signals.is_empty(), "duplicate request is idempotent");
        resolve(&mut ledger, json!(101));
        let mut elicitation =
            frame("mcpServer/elicitation/request", json!({"turnId":"turn-1","mode":"form"}));
        elicitation["id"] = json!(102);
        assert_eq!(
            ledger.observe(7, &elicitation).signals,
            vec![Signal::Attention { reason: Reason::Elicitation }]
        );
    }

    #[test]
    fn precise_native_requests_override_coarse_flags_without_releasing_unresolved_holds() {
        // The native 0.153.4 genuine MCP form capture ac872b39 set
        // waitingOnApproval without codex_approval_kind=mcp_tool_call.
        for (method, params, expected) in [
            (
                "mcpServer/elicitation/request",
                json!({
                    "turnId":"turn-1", "mode":"form", "serverName":"elicitation_fixture",
                    "message":"Disposable fixture confirmation.",
                    "requestedSchema":{"type":"object","properties":{
                        "confirmation":{"type":"string","enum":["ACCEPT_FIXTURE"]}
                    },"required":["confirmation"]}
                }),
                Reason::Elicitation,
            ),
            (
                "item/tool/requestUserInput",
                json!({
                    "turnId":"turn-1", "isBlocking":true,
                    "questions":[{"id":"choice","question":"Fixture choice?","isSecret":false}]
                }),
                Reason::Question,
            ),
            (
                "item/tool/requestUserInput",
                json!({
                    "turnId":"turn-1", "isBlocking":true,
                    "questions":[{"id":"secret","question":"Synthetic secret?","isSecret":true}]
                }),
                Reason::Secret,
            ),
        ] {
            let mut ledger = ready();
            start(&mut ledger, "turn-1");
            let flags = frame(
                "thread/status/changed",
                json!({"status":{
                    "type":"active","activeFlags":["waitingOnApproval","waitingOnUserInput"]
                }}),
            );
            ledger.observe(7, &flags);
            let mut request = frame(method, params);
            request["id"] = json!("native-form");
            let precise = ledger.observe(7, &request);
            assert_eq!(precise.state, SessionState::RequiresAction { reason: expected });
            assert!(precise.observation_hold);
            // A repeated coarse status after the precise request also cannot
            // relabel it, and a terminal turn cannot hide an unresolved form.
            assert_eq!(ledger.observe(7, &flags).state, precise.state);
            assert_eq!(complete(&mut ledger, "turn-1", "completed").state, precise.state);
            let resolved = resolve(&mut ledger, json!("native-form"));
            assert_eq!(resolved.state, SessionState::RequiresAction { reason: Reason::Permission });
            assert!(resolved.observation_hold, "request resolution does not clear status flags");
            assert!(!ledger.can_reconcile_work());
            let cleared = ledger
                .observe(7, &frame("thread/status/changed", json!({"status":{"type":"idle"}})));
            assert_eq!(
                cleared.signals,
                vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
            );
            assert!(!cleared.observation_hold);
        }
    }

    #[test]
    fn child_request_specificity_does_not_mask_another_threads_permission() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        let flags = json!({"status":{"type":"active","activeFlags":["waitingOnApproval"]}});
        ledger.observe(7, &frame("thread/status/changed", flags.clone()));
        let mut form = frame("mcpServer/elicitation/request", json!({"mode":"form"}));
        form["id"] = json!("parent-form");
        ledger.observe(7, &form);
        discover_child(&mut ledger);
        let child_wait = ledger.observe(7, &child_frame("thread/status/changed", flags));
        assert_eq!(child_wait.state, SessionState::RequiresAction { reason: Reason::Permission });
        form["id"] = json!("child-form");
        form["params"]["threadId"] = json!("child");
        let child_form = ledger.observe(7, &form);
        assert_eq!(child_form.state, SessionState::RequiresAction { reason: Reason::Elicitation });
        let mut permission =
            child_frame("item/fileChange/requestApproval", json!({"itemId":"edit"}));
        permission["id"] = json!("child-permission");
        assert_eq!(
            ledger.observe(7, &permission).state,
            SessionState::RequiresAction { reason: Reason::Permission }
        );
        let specific_resolved = ledger.observe(
            7,
            &child_frame("serverRequest/resolved", json!({"requestId":"child-permission"})),
        );
        assert_eq!(
            specific_resolved.state,
            SessionState::RequiresAction { reason: Reason::Elicitation }
        );
        let form_resolved = ledger
            .observe(7, &child_frame("serverRequest/resolved", json!({"requestId":"child-form"})));
        assert_eq!(
            form_resolved.state,
            SessionState::RequiresAction { reason: Reason::Permission }
        );
        assert!(form_resolved.observation_hold);
        let child_clear = ledger
            .observe(7, &child_frame("thread/status/changed", json!({"status":{"type":"idle"}})));
        assert_eq!(child_clear.state, SessionState::RequiresAction { reason: Reason::Elicitation });
        // The parent retains its own unresolved coarse flag independently.
        assert_eq!(
            resolve(&mut ledger, json!("parent-form")).state,
            SessionState::RequiresAction { reason: Reason::Permission }
        );
        assert!(!ledger.can_reconcile_work());
    }

    #[test]
    fn idle_audit_requires_no_known_work_and_memory_exhaustion_fails_closed() {
        let mut ledger = ready();
        assert!(ledger.can_reconcile_idle());
        start(&mut ledger, "turn-1");
        assert!(!ledger.can_reconcile_idle());
        complete(&mut ledger, "turn-1", "completed");
        assert!(ledger.can_reconcile_idle());
        ledger.lost(7);
        assert!(!ledger.can_reconcile_idle(), "loss forgot the outcome, not the outstanding turn");
        ledger.reconcile(7, &snapshot(json!({"type":"idle"})), true);
        assert!(ledger.can_reconcile_idle());
        start(&mut ledger, "turn-2");
        for id in 0..=MAX_IDENTITIES {
            let mut frame = frame("item/futureTool/request", json!({"turnId":"turn-2"}));
            frame["id"] = json!(id);
            ledger.observe(7, &frame);
        }
        assert!(ledger.capacity_lost);
        assert!(ledger.held());
        assert!(!ledger.can_reconcile_idle());
        let count = ledger.work_requests.len();
        let extra = json!({"method":"item/futureTool/request","id":"extra","params":{"threadId":"thread-a","turnId":"turn-2"}});
        assert!(ledger.observe(7, &extra).observation_hold);
        assert_eq!(ledger.work_requests.len(), count, "bounded after exhaustion");
    }
    #[test]
    fn known_system_error_status_preserves_failed_turn_in_either_event_order() {
        for status_first in [false, true] {
            let mut ledger = ready();
            start(&mut ledger, "failed-api");
            let status = frame("thread/status/changed", json!({"status":{"type":"systemError"}}));
            if status_first {
                let pending = ledger.observe(7, &status);
                assert_eq!(pending.state, SessionState::Running);
            }
            let failed = complete(&mut ledger, "failed-api", "failed");
            assert_eq!(failed.state, SessionState::Failed { reason: FailReason::Unknown });
            assert_eq!(
                failed.signals,
                vec![Signal::TurnEnded { outcome: TurnOutcome::Failed(FailReason::Unknown) }]
            );
            let after = ledger.observe(7, &status);
            assert_eq!(after.state, failed.state);
            assert!(after.signals.is_empty());
            start(&mut ledger, "retry");
            assert_eq!(
                complete(&mut ledger, "retry", "completed").state,
                SessionState::Idle { stop_reason: StopReason::EndTurn }
            );
        }
    }

    #[test]
    fn interrupted_compaction_without_item_completion_settles_the_matching_turn() {
        for automatic in [false, true] {
            let mut ledger = ready();
            start(&mut ledger, "compact");
            if automatic {
                ledger.observe(
                    7,
                    &frame(
                        "item/completed",
                        json!({"turnId":"compact",
                    "item":{"type":"userMessage","id":"user"}}),
                    ),
                );
            }
            ledger.observe(
                7,
                &frame(
                    "item/started",
                    json!({"turnId":"compact",
                "item":{"type":"contextCompaction","id":"compact-item"}}),
                ),
            );
            assert!(complete(&mut ledger, "other", "interrupted").observation_hold);
            let done = complete(&mut ledger, "compact", "interrupted");
            assert_eq!(done.state, SessionState::Idle { stop_reason: StopReason::Interrupted });
            assert_eq!(done.signals, vec![Signal::TurnEnded { outcome: TurnOutcome::Interrupted }]);
            assert!(!done.observation_hold);
            assert!(ledger.can_reconcile_work());
            assert!(complete(&mut ledger, "compact", "interrupted").signals.is_empty());
            start(&mut ledger, "next-task");
            assert_eq!(
                complete(&mut ledger, "next-task", "completed").signals,
                vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
            );
        }
    }

    #[test]
    fn cancelled_compaction_preserves_independent_work_on_parent_and_child() {
        for child in [false, true] {
            for status in ["interrupted", "failed"] {
                let mut ledger = ready();
                start(&mut ledger, "turn-1");
                if child {
                    discover_child(&mut ledger);
                }
                let scoped = if child { child_frame } else { frame };
                for (id, kind) in [("compact", "contextCompaction"), ("tool", "commandExecution")] {
                    ledger.observe(
                        7,
                        &scoped(
                            "item/started",
                            json!({"turnId":"turn-1",
                        "item":{"id":id,"type":kind}}),
                        ),
                    );
                }
                ledger.observe(7, &scoped("hook/started", json!({"run":{"id":"hook"}})));
                let mut approval =
                    scoped("item/commandExecution/requestApproval", json!({"turnId":"turn-1"}));
                approval["id"] = json!(1);
                ledger.observe(7, &approval);
                let mut callback = scoped("unknown/callback", json!({"turnId":"turn-1"}));
                callback["id"] = json!(2);
                ledger.observe(7, &callback);
                let terminal = scoped(
                    "turn/completed",
                    json!({"turn":{
                    "id":"turn-1","status":status,"items":[]}}),
                );
                assert!(ledger.observe(7, &terminal).observation_hold);
                assert!(!ledger.can_reconcile_work());
                ledger.observe(7, &scoped("serverRequest/resolved", json!({"requestId":1})));
                assert!(!ledger.can_reconcile_work(), "callback, tool and hook remain");
                ledger.observe(7, &scoped("serverRequest/resolved", json!({"requestId":2})));
                assert!(!ledger.can_reconcile_work(), "tool and hook remain");
                ledger.observe(
                    7,
                    &scoped(
                        "item/completed",
                        json!({"turnId":"turn-1",
                    "item":{"id":"tool","type":"commandExecution","status":"completed"}}),
                    ),
                );
                assert!(!ledger.can_reconcile_work(), "hook remains");
                let drained = ledger.observe(
                    7,
                    &scoped(
                        "hook/completed",
                        json!({
                    "run":{"id":"hook","status":"completed"}}),
                    ),
                );
                assert!(
                    ledger.can_reconcile_work(),
                    "cancelled compaction must no longer block audit"
                );
                if child {
                    assert!(drained.observation_hold, "child still needs independent audit");
                    assert_eq!(drained.state, SessionState::Running, "child cannot end the parent");
                    complete(&mut ledger, "turn-1", "interrupted");
                }
                let audited = ledger.reconcile_work(7, &snapshot(json!({"type":"idle"})));
                assert!(!audited.observation_hold);
                assert_eq!(
                    audited.state,
                    if child || status == "interrupted" {
                        SessionState::Idle { stop_reason: StopReason::Interrupted }
                    } else {
                        SessionState::Failed { reason: FailReason::Unknown }
                    }
                );
            }
        }
    }

    #[test]
    fn child_compaction_requires_its_own_terminal_turn() {
        let mut ledger = ready();
        start(&mut ledger, "turn-1");
        discover_child(&mut ledger);
        ledger.observe(
            7,
            &child_frame(
                "item/started",
                json!({"turnId":"child-turn",
            "item":{"id":"compact","type":"contextCompaction"}}),
            ),
        );
        complete(&mut ledger, "turn-1", "interrupted");
        for status in ["interrupted", "failed", "completed"] {
            ledger.observe(
                7,
                &child_frame(
                    "turn/completed",
                    json!({"turn":{
                "id":"wrong-turn","status":status,"items":[]}}),
                ),
            );
            assert!(!ledger.can_reconcile_work());
        }
        ledger.observe(7, &child_frame("thread/status/changed", json!({"status":{"type":"idle"}})));
        assert!(!ledger.can_reconcile_work(), "idle alone cannot retire compaction");
        ledger.observe(
            7,
            &child_frame(
                "turn/completed",
                json!({"turn":{
            "id":"child-turn","status":"interrupted","items":[]}}),
            ),
        );
        assert!(ledger.can_reconcile_work());
        assert!(!ledger.reconcile_work(7, &snapshot(json!({"type":"idle"}))).observation_hold);
    }

    #[test]
    fn manual_compaction_is_ready_unknown_after_maintenance_and_hook_finish() {
        let mut ledger = ready();
        start(&mut ledger, "task");
        assert_eq!(
            complete(&mut ledger, "task", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
        start(&mut ledger, "compact");
        ledger.observe(
            7,
            &frame("hook/started", json!({"run":{"id":"precompact","eventName":"preCompact"}})),
        );
        ledger.observe(
            7,
            &frame(
                "item/started",
                json!({"turnId":"compact","item":{"type":"contextCompaction","id":"compact-item"}}),
            ),
        );
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"turnId":"compact","item":{"type":"contextCompaction","id":"compact-item"}}),
            ),
        );
        let held = complete(&mut ledger, "compact", "completed");
        assert!(held.observation_hold);
        assert_eq!(held.state, SessionState::Running);
        let done = ledger.observe(
            7,
            &frame("hook/completed", json!({"run":{"id":"precompact","status":"completed"}})),
        );
        assert_eq!(done.signals, vec![Signal::Ready]);
        assert_eq!(done.state, SessionState::Idle { stop_reason: StopReason::Unknown });
        assert!(!done.observation_hold);
        assert!(complete(&mut ledger, "compact", "completed").signals.is_empty());
        // Maintenance classification belongs to the old turn only.
        start(&mut ledger, "next-task");
        assert_eq!(
            complete(&mut ledger, "next-task", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
    }

    #[test]
    fn in_task_compaction_keeps_success_and_missing_start_still_classifies_maintenance() {
        for kind in ["userMessage", "agentMessage", "commandExecution"] {
            let mut ledger = ready();
            start(&mut ledger, "task");
            ledger.observe(
                7,
                &frame(
                    "item/completed",
                    json!({"turnId":"task","item":{"type":kind,"id":"task-item","text":"fixture"}}),
                ),
            );
            ledger.observe(7,&frame("item/completed",json!({"turnId":"task","item":{"type":"contextCompaction","id":"compact-item"}})));
            assert_eq!(
                complete(&mut ledger, "task", "completed").signals,
                vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }],
                "{kind}"
            );
        }
        let mut ledger = ready();
        start(&mut ledger, "task");
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"turnId":"task","item":{"type":"agentMessage","id":"reply","text":"done"}}),
            ),
        );
        complete(&mut ledger, "task", "completed");
        ledger.observe(
            7,
            &frame("thread/status/changed", json!({"status":{"type":"active","activeFlags":[]}})),
        );
        ledger.observe(
            7,
            &frame(
                "item/completed",
                json!({"turnId":"compact","item":{"type":"contextCompaction","id":"compact-item"}}),
            ),
        );
        assert_eq!(complete(&mut ledger, "compact", "completed").signals, vec![Signal::Ready]);
    }

    #[test]
    fn completed_turn_items_classify_compaction_without_hiding_failure_or_interruption() {
        for status in ["completed", "interrupted", "failed"] {
            let mut ledger = ready();
            start(&mut ledger, "compact");
            let done=ledger.observe(7,&frame("turn/completed",json!({"turn":{"id":"compact","status":status,"items":[{"type":"contextCompaction","id":"compact-item"}]}})));
            if status == "completed" {
                assert_eq!(done.signals, vec![Signal::Ready]);
            } else {
                assert!(matches!(
                    done.signals.as_slice(),
                    [Signal::TurnEnded {
                        outcome: TurnOutcome::Interrupted | TurnOutcome::Failed(_)
                    }]
                ));
            }
        }
    }
}
