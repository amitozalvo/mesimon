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
    retired_turns: BTreeSet<String>,
    requests: BTreeMap<String, Reason>,
    work_requests: BTreeSet<String>,
    items: BTreeSet<String>,
    hooks: BTreeSet<String>,
    children: BTreeSet<String>,
    child_turns: BTreeMap<String, String>,
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
            retired_turns: BTreeSet::new(),
            requests: BTreeMap::new(),
            work_requests: BTreeSet::new(),
            items: BTreeSet::new(),
            hooks: BTreeSet::new(),
            children: BTreeSet::new(),
            child_turns: BTreeMap::new(),
            flags: Vec::new(),
            last_signal: None,
            state: SessionState::Unknown { reason: UnknownReason::ObservationLost },
        }
    }

    fn held(&self) -> bool {
        !self.continuous || self.capacity_lost || self.has_pending_work()
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

    /// Local half of a reconnect audit. The caller must additionally prove
    /// parent/descendant runtime idleness and uninterrupted capture during the
    /// audit. This intentionally ignores continuity itself, but never ignores
    /// unresolved requests, tools, hooks, children or a still-active turn.
    pub fn can_reconcile_idle(&self) -> bool {
        !self.capacity_lost
            && !self.has_pending_work()
            && !self.runtime_active
            && (self.active_turn.is_none() || self.terminal.is_some())
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
        let reason = self
            .requests
            .values()
            .chain(self.flags.iter())
            .copied()
            .min_by_key(|r| rank(&SessionState::RequiresAction { reason: *r }));
        if let Some(reason) = reason {
            self.emit(Signal::Attention { reason }, update);
        } else if !self.continuous || self.capacity_lost {
            self.emit(Signal::ObservationLost, update);
        } else if let Some(outcome) = self.terminal {
            if !self.has_pending_work() {
                if !self.terminal_emitted {
                    self.emit(Signal::TurnEnded { outcome }, update);
                    self.terminal_emitted = true;
                }
            } else {
                self.emit(Signal::TurnStarted, update);
            }
        } else if self.runtime_active || self.active_turn.is_some() || self.has_pending_work() {
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
            + self.children.len();
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
            self.child_turns.clear();
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
            // A known child's explicit terminal turn can release that child's
            // hold, but can never finish a parent lacking its own completion.
            if self.children.contains(thread) {
                if method == "turn/started" {
                    if let Some(id) = string(&params["turn"], "id") {
                        self.child_turns.insert(thread.into(), id.into());
                    }
                } else if method == "turn/completed"
                    && self.child_turns.get(thread).map(String::as_str)
                        == string(&params["turn"], "id")
                    && self.child_turns.contains_key(thread)
                    && outcome(&params["turn"]).is_some()
                {
                    self.children.remove(thread);
                    self.child_turns.remove(thread);
                    self.project(&mut update);
                }
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
            }
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
                    }
                    self.read_flags(&params["status"]);
                    match string(&params["status"], "type") {
                        Some("idle" | "active") => {}
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
        let terminal = match string(item, "status") {
            Some("inProgress" | "running" | "pendingInit") => false,
            Some("completed" | "failed" | "interrupted") => true,
            _ => completed,
        };
        match kind {
            "agentMessage" | "plan" => {
                if terminal {
                    if let Some(text) = string(item, "text") {
                        let text = text.chars().take(MAX_PREVIEW_CHARS).collect();
                        if kind == "plan" {
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
                    self.items.insert(id.into());
                }
                update.activity = Some(kind.chars().take(80).collect());
            }
        }
        if kind == "collabAgentToolCall" {
            if let Some(receivers) = item["receiverThreadIds"].as_array() {
                for child in receivers.iter().filter_map(Value::as_str) {
                    let state = string(&item["agentsStates"][child], "status");
                    if matches!(state, Some("completed" | "interrupted" | "errored" | "shutdown")) {
                        self.children.remove(child);
                        self.child_turns.remove(child);
                    } else {
                        // Unknown/notFound does not prove a formerly active
                        // child stopped; a caller must reconcile it explicitly.
                        self.children.insert(child.into());
                    }
                }
            }
        }
    }
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
    fn known_child_completion_requires_matching_child_turn_and_parent_completion() {
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
        assert_eq!(ledger.state(), &SessionState::Running);
        assert_eq!(
            complete(&mut ledger, "turn-1", "completed").signals,
            vec![Signal::TurnEnded { outcome: TurnOutcome::Completed }]
        );
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
}
