//! The attention state machine (11 §11.7) — pure, time-injected, table-tested.
//! Precedence ranks are fixed forever (D28); ranks 0–8 are the attention set:
//! exactly these produce the `needs you` count and light a card. The daemon
//! owns one `Machine` per session and applies `Signal`s from the hook stream;
//! debounce rules are 11 §11.7.4.

use crate::board::{
    Board, Confidence, ExitReason, FailReason, Reason, SessionRecord, SessionState, StopReason,
    UnknownReason,
};

/// Leave-settle for `RequiresAction` and `Running -> Idle` (11 §11.7.4).
pub const SETTLE_MS: u64 = 1500;
/// Leaving `Throttled` settles longer (11 §11.7.4).
pub const THROTTLE_LEAVE_MS: u64 = 5000;
/// A `RequiresAction` with no clearing event demotes — never latches red.
pub const STALE_DEMOTE_MS: u64 = 15 * 60 * 1000;
/// Flap guard: more than this many debounced changes inside the window pins.
pub const FLAP_MAX: usize = 4;
pub const FLAP_WINDOW_MS: u64 = 20_000;
pub const FLAP_PIN_MS: u64 = 20_000;
/// Do not re-announce a reason the session left this recently.
pub const REEMIT_SUPPRESS_MS: u64 = 30_000;

/// 11 §11.7.2 — fixed precedence. Lower wins; a ticket's state is the minimum
/// rank across its sessions.
pub fn rank(s: &SessionState) -> u8 {
    match s {
        SessionState::RequiresAction { reason } => match reason {
            Reason::Permission => 0,
            Reason::Secret => 1,
            Reason::Question => 2,
            Reason::Plan => 3,
            Reason::Elicitation => 4,
            Reason::Auth => 5,
            Reason::QuotaResume => 6,
            Reason::Trust => 7,
            // Shared rank: both are "session stuck at a spawn-time modal".
            // D28 forbids moving existing ranks; ties order by waiting_since.
            Reason::StartupModal | Reason::ResumeDialog => 8,
        },
        SessionState::Failed { .. } => 9,
        SessionState::Throttled => 10,
        SessionState::Spawning => 11,
        SessionState::Running => 12,
        SessionState::Idle { .. } => 13,
        SessionState::Unknown { .. } => 14,
        SessionState::Sleeping => 15,
        SessionState::Exited { .. } => 16,
    }
}

pub fn is_attention(s: &SessionState) -> bool {
    rank(s) <= 8
}

/// The card's state word: the reason replaces it, uppercase (06 §3).
pub fn reason_word(r: Reason) -> &'static str {
    match r {
        Reason::Permission => "PERMISSION",
        Reason::Secret => "SECRET",
        Reason::Question => "QUESTION",
        Reason::Plan => "PLAN",
        Reason::Elicitation => "INPUT",
        Reason::Auth => "AUTH",
        Reason::QuotaResume => "QUOTA",
        Reason::Trust => "TRUST",
        Reason::StartupModal => "SETUP",
        Reason::ResumeDialog => "RESUME",
    }
}

/// The Tab queue: attention-set sessions at usable confidence, cheapest first.
/// Low/stale confidence never enters (11 §11.5.4).
pub fn attention_queue(board: &Board) -> Vec<&SessionRecord> {
    let mut v: Vec<&SessionRecord> = board
        .sessions
        .iter()
        .filter(|s| {
            is_attention(&s.state) && matches!(s.confidence, Confidence::High | Confidence::Medium)
        })
        .collect();
    v.sort_by(|a, b| {
        rank(&a.state)
            .cmp(&rank(&b.state))
            .then(a.waiting_since.unwrap_or(u64::MAX).cmp(&b.waiting_since.unwrap_or(u64::MAX)))
            .then(a.id.cmp(&b.id))
    });
    v
}

// ---------------------------------------------------------------------------
// Signals — what the ingest layer distills a hook frame into.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSource {
    Startup,
    Resume,
    Clear,
    Compact,
    Fork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    Clear,
    Resume,
    Logout,
    PromptInputExit,
    Other,
}

impl EndKind {
    fn exit_reason(self) -> ExitReason {
        match self {
            EndKind::Clear => ExitReason::Cleared,
            EndKind::Resume => ExitReason::Resumed,
            EndKind::Logout => ExitReason::LoggedOut,
            EndKind::PromptInputExit => ExitReason::UserQuit,
            // `other` is how a crash reports itself (11 §11.2.3).
            EndKind::Other => ExitReason::Crashed,
        }
    }
}

/// The ten `StopFailure` matchers — the matcher IS the error class (spike S-A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopFailureClass {
    RateLimit,
    Overloaded,
    AuthenticationFailed,
    OauthOrgNotAllowed,
    BillingError,
    InvalidRequest,
    ModelNotFound,
    MaxOutputTokens,
    ServerError,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionTool {
    AskUserQuestion,
    ExitPlanMode,
}

/// What the observe-tier transcript tail saw (09 §4.4). Always applied at
/// `Confidence::Low` — Tier-0 evidence never enters the attention queue and
/// never lights the saturated colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailHint {
    /// A new assistant text block — the session is producing output.
    AssistantText,
    AskUserQuestion,
    ExitPlanMode,
    TurnComplete,
    AbortedMidStream,
    /// Transcript mtime quiet past the threshold while we thought it ran.
    StaleQuiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    PermissionPrompt,
    QuotaFired,
    QuotaStale,
    QuotaDisabled,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    SessionStart { source: StartSource },
    SessionEnd { kind: EndKind },
    UserPromptSubmit,
    Stop { stop_hook_active: bool, has_agent_id: bool, background_tasks: bool },
    SubagentStop,
    TeammateIdle,
    StopFailure { class: StopFailureClass },
    PermissionRequest,
    PermissionDenied,
    PreToolUse { tool: AttentionTool },
    /// The narrow post-tool pair only: fires when the user has ANSWERED the
    /// question / resolved the plan dialog, which is the only mid-turn moment
    /// the `RequiresAction` can truthfully drop back to `Running`.
    PostToolUse { tool: AttentionTool },
    Notification { kind: NotificationKind },
    Elicitation,
    ElicitationResult,
    /// tmux pane-died — authoritative for exit (spike T-7).
    PaneDied { status: Option<i32> },
    /// Daemon-side probe while `Spawning` (11 §11.5.3 approximation).
    /// `resume` marks a `--resume` spawn: a modal there is the resume-from-
    /// summary dialog (09 §9), not first-run setup.
    SpawnProbe { bytes: bool, osc0: bool, resume: bool },
    /// Daemon-side probe while `Running`: the pane stopped painting past the
    /// quiet threshold. A turn in flight repaints continuously (spinner), so
    /// sustained silence means the turn is over — this is the ONLY signal an
    /// Esc interrupt leaves (spike S-E: no hook fires, the transcript may get
    /// no record, and the corpus's OSC 9;4 / title-glyph Tier A− channels no
    /// longer exist).
    PaneQuiet,
    /// Observe tier: derived from an adopted session's transcript tail.
    TranscriptHint { kind: TailHint },
}

/// One debounced, publishable transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub from: SessionState,
    pub to: SessionState,
    /// True only when this transition adds a NEW attention item (dedup'd per
    /// 11 §11.7.4's 30 s suppression). Never true at Low/Stale confidence.
    pub attention_added: bool,
    pub confidence: Confidence,
}

#[derive(Debug, Clone)]
struct Pending {
    to: SessionState,
    confidence: Confidence,
    deadline: u64,
}

/// Per-session machine. `apply` handles a signal (enters are immediate, leaves
/// are scheduled); `tick` fires scheduled work. All times are epoch ms,
/// injected by the caller.
#[derive(Debug, Clone)]
pub struct Machine {
    state: SessionState,
    confidence: Confidence,
    entered_at: u64,
    pending: Option<Pending>,
    /// Timestamps of committed (debounced) changes, for the flap guard.
    committed: Vec<u64>,
    pinned_until: Option<u64>,
    /// Last attention reason left, for the 30 s re-emit suppression.
    recent_left: Option<(Reason, u64)>,
}

impl Machine {
    pub fn new(state: SessionState, now: u64) -> Self {
        Self {
            state,
            confidence: Confidence::High,
            entered_at: now,
            pending: None,
            committed: Vec::new(),
            pinned_until: None,
            recent_left: None,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn apply(&mut self, sig: &Signal, now: u64) -> Option<Change> {
        // Sleeping latches: the daemon's own SIGTERM produces SessionEnd and
        // pane-died, and neither those nor any straggler frame may flip a
        // parked session to Exited. Only wake leaves — by re-minting the
        // machine as Spawning (11 §11.7.3).
        if self.state == SessionState::Sleeping {
            return None;
        }
        // Exited is terminal: publish once (02 §7.3). A late SessionEnd may
        // refine a pane-derived reason in place, silently.
        if let SessionState::Exited { reason } = &self.state {
            if let Signal::SessionEnd { kind } = sig {
                if matches!(reason, ExitReason::UserQuit | ExitReason::Crashed) {
                    self.state = SessionState::Exited { reason: kind.exit_reason() };
                }
            }
            return None;
        }

        let (to, conf) = self.target(sig)?;

        // Flap pin: only terminal transitions get through while pinned.
        if let Some(until) = self.pinned_until {
            if now < until && !matches!(to, SessionState::Exited { .. } | SessionState::Failed { .. })
            {
                return None;
            }
        }

        if to == self.state {
            // Re-affirmation cancels any scheduled leave (11 §11.7.4: the
            // clearing event must survive 1500 ms without a re-trigger). A
            // confirming signal may raise confidence, never state; High is
            // discriminant 0, so "raise" is the lower value.
            self.pending = None;
            if (conf as u8) < (self.confidence as u8) {
                self.confidence = conf;
            }
            return None;
        }

        let delay = delay_for(&self.state, &to);
        if delay == 0 {
            self.pending = None;
            Some(self.commit(to, conf, now))
        } else {
            // Re-asserting the target already pending keeps the ORIGINAL
            // deadline — a repeat cadence faster than the settle would
            // otherwise push the deadline forever and never commit. Only a
            // signal re-affirming the CURRENT state cancels a leave.
            if self.pending.as_ref().is_none_or(|p| p.to != to) {
                self.pending = Some(Pending { to, confidence: conf, deadline: now + delay });
            }
            None
        }
    }

    pub fn tick(&mut self, now: u64) -> Option<Change> {
        if let Some(p) = &self.pending {
            if now >= p.deadline {
                let (to, conf) = (p.to.clone(), p.confidence);
                self.pending = None;
                return Some(self.commit(to, conf, now));
            }
        }
        // Stale demotion: never latch red (11 §11.7.4).
        if is_attention(&self.state) && now.saturating_sub(self.entered_at) >= STALE_DEMOTE_MS {
            let to = SessionState::Unknown { reason: UnknownReason::NoSignal };
            return Some(self.commit(to, Confidence::Stale, now));
        }
        None
    }

    fn commit(&mut self, to: SessionState, confidence: Confidence, now: u64) -> Change {
        let from = std::mem::replace(&mut self.state, to.clone());
        if let SessionState::RequiresAction { reason } = &from {
            self.recent_left = Some((*reason, now));
        }
        self.entered_at = now;

        // Flap guard: >FLAP_MAX committed changes in the window pins the
        // machine at the state just committed, confidence low. (Deviation from
        // 11 §11.7.4's "lowest rank seen": we pin in place — simpler, and the
        // pinned state is at most one transition away from that.)
        self.committed.retain(|t| now.saturating_sub(*t) <= FLAP_WINDOW_MS);
        self.committed.push(now);
        let mut confidence = confidence;
        if self.committed.len() > FLAP_MAX {
            self.pinned_until = Some(now + FLAP_PIN_MS);
            confidence = Confidence::Low;
        }
        self.confidence = confidence;

        let suppressed = match (&to, &self.recent_left) {
            (SessionState::RequiresAction { reason }, Some((left, at))) => {
                reason == left && now.saturating_sub(*at) < REEMIT_SUPPRESS_MS
            }
            _ => false,
        };
        let newly = match (&from, &to) {
            (SessionState::RequiresAction { reason: a }, SessionState::RequiresAction { reason: b }) => a != b,
            (_, SessionState::RequiresAction { .. }) => true,
            _ => false,
        };
        let attention_added = newly
            && !suppressed
            && matches!(confidence, Confidence::High | Confidence::Medium);

        Change { from, to, attention_added, confidence }
    }

    /// The 11 §11.7.3 transition table, restricted to the M2 registered set.
    /// `None` = the signal produces no top-level transition from this state.
    fn target(&self, sig: &Signal) -> Option<(SessionState, Confidence)> {
        use SessionState as S;
        let t = |s: S| Some((s, Confidence::High));
        match sig {
            // A session that just started sits at the prompt — that is idle,
            // not working (dogfood 2026-08-30: fresh spawns read "working"
            // forever). The one exception: a compact-restart fires
            // SessionStart mid-turn and the turn continues.
            Signal::SessionStart { source: StartSource::Compact } => t(S::Running),
            Signal::SessionStart { .. } => t(S::Idle { stop_reason: StopReason::Unknown }),
            Signal::SessionEnd { kind } => t(S::Exited { reason: kind.exit_reason() }),
            Signal::UserPromptSubmit => t(S::Running),
            Signal::Stop { stop_hook_active: true, .. } => None, // re-entrancy guard
            Signal::Stop { has_agent_id: true, .. } => None,     // nested, never top-level
            Signal::Stop { background_tasks: true, .. } => t(S::Running),
            Signal::Stop { .. } => t(S::Idle { stop_reason: StopReason::EndTurn }),
            Signal::SubagentStop | Signal::TeammateIdle => None,
            Signal::StopFailure { class } => match class {
                StopFailureClass::RateLimit | StopFailureClass::Overloaded => t(S::Throttled),
                StopFailureClass::AuthenticationFailed
                | StopFailureClass::OauthOrgNotAllowed
                | StopFailureClass::BillingError => t(S::RequiresAction { reason: Reason::Auth }),
                StopFailureClass::InvalidRequest => {
                    t(S::Failed { reason: FailReason::InvalidRequest })
                }
                StopFailureClass::ModelNotFound => {
                    t(S::Failed { reason: FailReason::ModelNotFound })
                }
                StopFailureClass::MaxOutputTokens => {
                    t(S::Failed { reason: FailReason::MaxOutputTokens })
                }
                StopFailureClass::ServerError => t(S::Failed { reason: FailReason::Server }),
                StopFailureClass::Unknown => t(S::Failed { reason: FailReason::Unknown }),
            },
            Signal::PermissionRequest => t(S::RequiresAction { reason: Reason::Permission }),
            // The auto-mode deny path — the human-deny path is "next event wins".
            Signal::PermissionDenied => t(S::Running),
            Signal::PreToolUse { tool } => match tool {
                AttentionTool::AskUserQuestion => t(S::RequiresAction { reason: Reason::Question }),
                AttentionTool::ExitPlanMode => t(S::RequiresAction { reason: Reason::Plan }),
            },
            // Tool completed = the user answered; the turn resumes.
            Signal::PostToolUse { .. } => t(S::Running),
            Signal::Notification { kind } => match kind {
                NotificationKind::QuotaStale | NotificationKind::QuotaDisabled => {
                    t(S::RequiresAction { reason: Reason::QuotaResume })
                }
                NotificationKind::QuotaFired => t(S::Running),
                // The sandboxed-network case: the only path that exists when no
                // PermissionRequest fired (11 §11.2.3). Confirmation-only when
                // the state is already held.
                NotificationKind::PermissionPrompt => {
                    // Confirmation-only when a dialog is already held — and a
                    // held Plan/Question IS this prompt (the interaction tools
                    // surface as permission dialogs); never blur it to the
                    // generic reason.
                    if matches!(
                        self.state,
                        S::RequiresAction {
                            reason: Reason::Permission | Reason::Plan | Reason::Question
                        }
                    ) {
                        None
                    } else {
                        Some((
                            S::RequiresAction { reason: Reason::Permission },
                            Confidence::Medium,
                        ))
                    }
                }
                NotificationKind::Other => None,
            },
            Signal::Elicitation => t(S::RequiresAction { reason: Reason::Elicitation }),
            Signal::ElicitationResult => t(S::Running),
            Signal::PaneDied { status } => t(S::Exited {
                reason: if status.unwrap_or(-1) == 0 {
                    ExitReason::UserQuit
                } else {
                    ExitReason::Crashed
                },
            }),
            Signal::SpawnProbe { bytes, osc0, resume } => {
                if self.state != S::Spawning {
                    return None;
                }
                if *bytes && !*osc0 {
                    let reason =
                        if *resume { Reason::ResumeDialog } else { Reason::StartupModal };
                    Some((S::RequiresAction { reason }, Confidence::Medium))
                } else if !*bytes {
                    Some((S::unknown(), Confidence::Low))
                } else {
                    None
                }
            }
            // The 11 §11.7.3 interrupt row, activity-approximated: only ever a
            // demotion out of Running, never a promotion — Medium because byte
            // silence is inference, not a stated event.
            Signal::PaneQuiet => {
                if self.state == S::Running {
                    Some((
                        S::Idle { stop_reason: StopReason::Interrupted },
                        Confidence::Medium,
                    ))
                } else {
                    None
                }
            }
            Signal::TranscriptHint { kind } => {
                let s = match kind {
                    TailHint::AssistantText => S::Running,
                    TailHint::AskUserQuestion => S::RequiresAction { reason: Reason::Question },
                    TailHint::ExitPlanMode => S::RequiresAction { reason: Reason::Plan },
                    TailHint::TurnComplete => S::Idle { stop_reason: StopReason::EndTurn },
                    TailHint::AbortedMidStream => {
                        S::Idle { stop_reason: StopReason::Interrupted }
                    }
                    TailHint::StaleQuiet => S::Idle { stop_reason: StopReason::Unknown },
                };
                Some((s, Confidence::Low))
            }
        }
    }
}

/// 11 §11.7.4: enters into the attention set, errors, throttle and terminal
/// states are immediate; leaves settle.
fn delay_for(from: &SessionState, to: &SessionState) -> u64 {
    use SessionState as S;
    if matches!(to, S::Exited { .. } | S::Failed { .. } | S::Throttled) || is_attention(to) {
        return 0;
    }
    match from {
        S::RequiresAction { .. } => SETTLE_MS,
        S::Throttled => THROTTLE_LEAVE_MS,
        S::Running if matches!(to, S::Idle { .. }) => SETTLE_MS,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::SessionKind;

    fn m(state: SessionState) -> Machine {
        Machine::new(state, 0)
    }

    const RA_PERM: SessionState = SessionState::RequiresAction { reason: Reason::Permission };

    #[test]
    fn ranks_are_the_fixed_table() {
        assert_eq!(rank(&RA_PERM), 0);
        assert_eq!(rank(&SessionState::RequiresAction { reason: Reason::StartupModal }), 8);
        assert_eq!(rank(&SessionState::Failed { reason: FailReason::Server }), 9);
        assert_eq!(rank(&SessionState::Throttled), 10);
        assert_eq!(rank(&SessionState::Spawning), 11);
        assert_eq!(rank(&SessionState::Running), 12);
        assert_eq!(rank(&SessionState::Idle { stop_reason: StopReason::EndTurn }), 13);
        assert_eq!(rank(&SessionState::unknown()), 14);
        assert_eq!(rank(&SessionState::Sleeping), 15);
        assert_eq!(rank(&SessionState::Exited { reason: ExitReason::Crashed }), 16);
        assert!(is_attention(&RA_PERM));
        assert!(!is_attention(&SessionState::Failed { reason: FailReason::Unknown }));
    }

    #[test]
    fn permission_enters_immediately() {
        let mut m = m(SessionState::Running);
        let c = m.apply(&Signal::PermissionRequest, 1000).expect("immediate");
        assert_eq!(c.to, RA_PERM);
        assert!(c.attention_added);
    }

    #[test]
    fn leave_settles_1500ms_and_retrigger_cancels() {
        let mut m = m(RA_PERM);
        // Human answered; the next prompt clears it — but only after settle.
        assert!(m.apply(&Signal::UserPromptSubmit, 1000).is_none());
        assert!(m.tick(2000).is_none()); // not yet
        // A re-trigger inside the settle window cancels the leave.
        assert!(m.apply(&Signal::PermissionRequest, 2100).is_none());
        assert!(m.tick(3000).is_none());
        assert_eq!(m.state(), &RA_PERM);
        // Clear again, let it settle.
        assert!(m.apply(&Signal::UserPromptSubmit, 4000).is_none());
        let c = m.tick(5500).expect("settled");
        assert_eq!(c.to, SessionState::Running);
    }

    #[test]
    fn stop_needs_quiet_before_idle() {
        let mut m = m(SessionState::Running);
        let stop = Signal::Stop { stop_hook_active: false, has_agent_id: false, background_tasks: false };
        assert!(m.apply(&stop, 1000).is_none());
        // A prompt inside the window keeps it running.
        assert!(m.apply(&Signal::UserPromptSubmit, 1200).is_none());
        assert!(m.tick(3000).is_none());
        assert_eq!(m.state(), &SessionState::Running);
        // Quiet stop goes idle.
        assert!(m.apply(&stop, 4000).is_none());
        let c = m.tick(5500).expect("idle");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
    }

    #[test]
    fn stop_hook_active_and_agent_id_are_ignored() {
        let mut m = m(SessionState::Running);
        assert!(m
            .apply(&Signal::Stop { stop_hook_active: true, has_agent_id: false, background_tasks: false }, 1000)
            .is_none());
        assert!(m
            .apply(&Signal::Stop { stop_hook_active: false, has_agent_id: true, background_tasks: false }, 1000)
            .is_none());
        assert!(m.pending.is_none());
    }

    #[test]
    fn stop_with_background_tasks_stays_running() {
        let mut m = m(SessionState::Running);
        assert!(m
            .apply(&Signal::Stop { stop_hook_active: false, has_agent_id: false, background_tasks: true }, 1000)
            .is_none());
        assert!(m.tick(9000).is_none());
        assert_eq!(m.state(), &SessionState::Running);
    }

    #[test]
    fn stopfailure_classes_route() {
        let mut m1 = m(SessionState::Running);
        let c = m1.apply(&Signal::StopFailure { class: StopFailureClass::AuthenticationFailed }, 1).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::Auth });
        assert!(c.attention_added);

        let mut m2 = m(SessionState::Running);
        let c = m2.apply(&Signal::StopFailure { class: StopFailureClass::RateLimit }, 1).unwrap();
        assert_eq!(c.to, SessionState::Throttled);
        assert!(!c.attention_added);

        let mut m3 = m(SessionState::Running);
        let c = m3.apply(&Signal::StopFailure { class: StopFailureClass::ServerError }, 1).unwrap();
        assert_eq!(c.to, SessionState::Failed { reason: FailReason::Server });
    }

    #[test]
    fn throttled_leave_settles_5s() {
        let mut m = m(SessionState::Throttled);
        assert!(m.apply(&Signal::Notification { kind: NotificationKind::QuotaFired }, 1000).is_none());
        assert!(m.tick(4000).is_none());
        let c = m.tick(6100).expect("left throttled");
        assert_eq!(c.to, SessionState::Running);
    }

    #[test]
    fn quota_stale_needs_human() {
        let mut m = m(SessionState::Throttled);
        let c = m.apply(&Signal::Notification { kind: NotificationKind::QuotaStale }, 1000).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::QuotaResume });
        assert!(c.attention_added);
    }

    #[test]
    fn sandboxed_permission_prompt_is_medium_and_confirmation_is_silent() {
        let mut m = m(SessionState::Running);
        let c = m.apply(&Signal::Notification { kind: NotificationKind::PermissionPrompt }, 1000).unwrap();
        assert_eq!(c.to, RA_PERM);
        assert_eq!(c.confidence, Confidence::Medium);
        // The 6 s-late confirmation raises nothing and never re-transitions.
        assert!(m.apply(&Signal::Notification { kind: NotificationKind::PermissionPrompt }, 2000).is_none());
    }

    #[test]
    fn question_and_plan_from_pretooluse() {
        let mut m1 = m(SessionState::Running);
        let c = m1.apply(&Signal::PreToolUse { tool: AttentionTool::AskUserQuestion }, 1).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::Question });
        let mut m2 = m(SessionState::Running);
        let c = m2.apply(&Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }, 1).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::Plan });
    }

    #[test]
    fn permission_prompt_never_blurs_a_held_plan_or_question() {
        // The plan dialog is a permission dialog for ExitPlanMode; the
        // straggler permission_prompt notification must not demote the
        // sharper reason to generic Permission (dogfood 2026-08-30).
        for reason in [Reason::Plan, Reason::Question] {
            let mut m1 = m(SessionState::RequiresAction { reason });
            let sig = Signal::Notification { kind: NotificationKind::PermissionPrompt };
            assert_eq!(m1.apply(&sig, 1), None);
            assert_eq!(m1.state(), &SessionState::RequiresAction { reason });
        }
    }

    #[test]
    fn answered_question_settles_back_to_running() {
        let mut m = m(SessionState::Running);
        m.apply(&Signal::PreToolUse { tool: AttentionTool::AskUserQuestion }, 1_000).unwrap();
        // The answer lands mid-turn: a leave, so it settles, never instant.
        assert!(m
            .apply(&Signal::PostToolUse { tool: AttentionTool::AskUserQuestion }, 5_000)
            .is_none());
        assert!(m.tick(5_000 + SETTLE_MS - 1).is_none());
        let c = m.tick(5_000 + SETTLE_MS).unwrap();
        assert_eq!(c.to, SessionState::Running);
        assert!(!c.attention_added);
    }

    #[test]
    fn pane_died_is_terminal_and_publish_once() {
        let mut m = m(RA_PERM);
        let c = m.apply(&Signal::PaneDied { status: Some(7) }, 1000).expect("immediate");
        assert_eq!(c.to, SessionState::Exited { reason: ExitReason::Crashed });
        // Nothing moves an exited session; SessionEnd only refines the reason.
        assert!(m.apply(&Signal::PaneDied { status: Some(0) }, 2000).is_none());
        assert!(m.apply(&Signal::SessionEnd { kind: EndKind::Logout }, 2000).is_none());
        assert_eq!(m.state(), &SessionState::Exited { reason: ExitReason::LoggedOut });
        assert!(m.apply(&Signal::PermissionRequest, 3000).is_none());
    }

    #[test]
    fn session_end_kinds_map() {
        for (kind, reason) in [
            (EndKind::Clear, ExitReason::Cleared),
            (EndKind::Resume, ExitReason::Resumed),
            (EndKind::Logout, ExitReason::LoggedOut),
            (EndKind::PromptInputExit, ExitReason::UserQuit),
            (EndKind::Other, ExitReason::Crashed),
        ] {
            let mut ma = m(SessionState::Running);
            let c = ma.apply(&Signal::SessionEnd { kind }, 1).unwrap();
            assert_eq!(c.to, SessionState::Exited { reason });
        }
    }

    #[test]
    fn spawn_probe_paths() {
        // Bytes but no Claude title → startup modal, medium confidence.
        let mut m1 = m(SessionState::Spawning);
        let c = m1
            .apply(&Signal::SpawnProbe { bytes: true, osc0: false, resume: false }, 1)
            .unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::StartupModal });
        assert_eq!(c.confidence, Confidence::Medium);
        // No bytes at all → unknown, never failed, low (out of the queue).
        let mut m2 = m(SessionState::Spawning);
        let c = m2
            .apply(&Signal::SpawnProbe { bytes: false, osc0: false, resume: false }, 1)
            .unwrap();
        assert_eq!(c.to, SessionState::unknown());
        assert_eq!(c.confidence, Confidence::Low);
        assert!(!c.attention_added);
        // Healthy title → hold Spawning.
        let mut m3 = m(SessionState::Spawning);
        assert!(m3
            .apply(&Signal::SpawnProbe { bytes: true, osc0: true, resume: false }, 1)
            .is_none());
        // Ignored once no longer spawning.
        let mut m4 = m(SessionState::Running);
        assert!(m4
            .apply(&Signal::SpawnProbe { bytes: true, osc0: false, resume: false }, 1)
            .is_none());
    }

    #[test]
    fn resume_spawn_probe_is_resume_dialog() {
        assert_eq!(rank(&SessionState::RequiresAction { reason: Reason::ResumeDialog }), 8);
        assert_eq!(reason_word(Reason::ResumeDialog), "RESUME");
        let mut m1 = m(SessionState::Spawning);
        let c = m1
            .apply(&Signal::SpawnProbe { bytes: true, osc0: false, resume: true }, 1)
            .unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::ResumeDialog });
        assert_eq!(c.confidence, Confidence::Medium);
        assert!(c.attention_added);
    }

    #[test]
    fn pane_quiet_settles_running_to_idle_interrupted() {
        let mut m = m(SessionState::Running);
        // Quiet enters via the same settle as any Running -> Idle leave.
        assert!(m.apply(&Signal::PaneQuiet, 1000).is_none());
        assert!(m.tick(2000).is_none());
        let c = m.tick(2600).expect("settled");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
        assert_eq!(c.confidence, Confidence::Medium);
        assert!(!c.attention_added);
    }

    #[test]
    fn repeated_pane_quiet_keeps_the_original_deadline() {
        // The probe fires every second; the settle is 1.5 s. Re-assertion must
        // not push the deadline out forever.
        let mut m = m(SessionState::Running);
        assert!(m.apply(&Signal::PaneQuiet, 1000).is_none());
        assert!(m.apply(&Signal::PaneQuiet, 2000).is_none());
        let c = m.tick(2500).expect("committed at the 1000+1500 deadline");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
    }

    #[test]
    fn pane_quiet_is_cancelled_by_a_prompt_inside_settle() {
        let mut m = m(SessionState::Running);
        assert!(m.apply(&Signal::PaneQuiet, 1000).is_none());
        // The turn was alive after all — re-affirmation cancels the leave.
        assert!(m.apply(&Signal::UserPromptSubmit, 1500).is_none());
        assert!(m.tick(3000).is_none());
        assert_eq!(m.state(), &SessionState::Running);
    }

    #[test]
    fn pane_quiet_only_ever_demotes_running() {
        // A permission wait is legitimately quiet — never clear attention on
        // byte silence. Same for every other state: quiet is not evidence.
        for s in [
            RA_PERM,
            SessionState::Spawning,
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Throttled,
            SessionState::unknown(),
        ] {
            let mut ma = m(s.clone());
            assert!(ma.apply(&Signal::PaneQuiet, 1000).is_none(), "moved from {s:?}");
            assert!(ma.tick(60_000).is_none());
        }
    }

    #[test]
    fn transcript_hints_are_always_low_and_silent() {
        for (kind, want) in [
            (TailHint::AssistantText, SessionState::Running),
            (TailHint::AskUserQuestion, SessionState::RequiresAction { reason: Reason::Question }),
            (TailHint::ExitPlanMode, SessionState::RequiresAction { reason: Reason::Plan }),
            (TailHint::TurnComplete, SessionState::Idle { stop_reason: StopReason::EndTurn }),
            (
                TailHint::AbortedMidStream,
                SessionState::Idle { stop_reason: StopReason::Interrupted },
            ),
            (TailHint::StaleQuiet, SessionState::Idle { stop_reason: StopReason::Unknown }),
        ] {
            let mut ma = m(SessionState::unknown());
            let c = ma.apply(&Signal::TranscriptHint { kind }, 1).expect("transition");
            assert_eq!(c.to, want);
            assert_eq!(c.confidence, Confidence::Low);
            // Tier-0 evidence never announces — Low is out of the queue.
            assert!(!c.attention_added);
        }
    }

    #[test]
    fn sleeping_latches_against_all_signals() {
        let mut ma = m(SessionState::Sleeping);
        // The daemon's own SIGTERM emits these — none may wake or exit the record.
        assert!(ma.apply(&Signal::SessionEnd { kind: EndKind::Other }, 1000).is_none());
        assert!(ma.apply(&Signal::PaneDied { status: Some(1) }, 1000).is_none());
        assert!(ma
            .apply(
                &Signal::Stop { stop_hook_active: false, has_agent_id: false, background_tasks: false },
                1000
            )
            .is_none());
        assert!(ma.apply(&Signal::PermissionRequest, 1000).is_none());
        assert!(ma.tick(10_000_000).is_none());
        assert_eq!(ma.state(), &SessionState::Sleeping);
    }

    #[test]
    fn stale_demotes_after_15min_never_latches() {
        let mut m = m(SessionState::Running);
        m.apply(&Signal::PermissionRequest, 1000).unwrap();
        assert!(m.tick(1000 + STALE_DEMOTE_MS - 1).is_none());
        let c = m.tick(1000 + STALE_DEMOTE_MS).expect("demoted");
        assert_eq!(c.to, SessionState::unknown());
        assert_eq!(c.confidence, Confidence::Stale);
        assert!(!c.attention_added);
    }

    #[test]
    fn reemit_suppressed_within_30s() {
        let mut m = m(RA_PERM);
        let _ = m.apply(&Signal::UserPromptSubmit, 1000);
        m.tick(2500).expect("left");
        // Same reason returns 3 s later: state transitions, but no re-announce.
        let c = m.apply(&Signal::PermissionRequest, 5500).expect("re-entered");
        assert_eq!(c.to, RA_PERM);
        assert!(!c.attention_added);
        // A DIFFERENT reason announces normally.
        let mut m2 = m_clone_left();
        let c = m2.apply(&Signal::Elicitation, 5500).expect("entered");
        assert!(c.attention_added);
    }

    fn m_clone_left() -> Machine {
        let mut m = Machine::new(RA_PERM, 0);
        let _ = m.apply(&Signal::UserPromptSubmit, 1000);
        m.tick(2500).expect("left");
        m
    }

    #[test]
    fn flap_guard_pins_low() {
        let mut m = m(SessionState::Running);
        // 5 immediate transitions inside 20 s: alternate reasons (all enter at 0ms).
        m.apply(&Signal::PermissionRequest, 1000).unwrap();
        m.apply(&Signal::Elicitation, 2000).unwrap();
        let _ = m.apply(&Signal::PermissionRequest, 3000);
        let _ = m.apply(&Signal::Elicitation, 4000);
        let c = m.apply(&Signal::PreToolUse { tool: AttentionTool::AskUserQuestion }, 5000);
        // Whichever commit crossed the threshold went Low…
        assert_eq!(m.confidence(), Confidence::Low);
        let _ = c;
        // …and further non-terminal transitions are ignored while pinned.
        assert!(m.apply(&Signal::UserPromptSubmit, 6000).is_none());
        assert!(m.tick(9000).is_none());
        // Terminal still passes.
        let c = m.apply(&Signal::PaneDied { status: Some(1) }, 7000).expect("terminal passes pin");
        assert_eq!(c.to, SessionState::Exited { reason: ExitReason::Crashed });
    }

    #[test]
    fn queue_orders_by_rank_then_wait() {
        let mut board = Board::default();
        let t = ulid::Ulid::new();
        let mk = |state: SessionState, waiting: Option<u64>, conf: Confidence| {
            let mut r = SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Claude,
                t,
                vec![],
                "/tmp".into(),
                state,
            );
            r.waiting_since = waiting;
            r.confidence = conf;
            r
        };
        let perm_late = mk(RA_PERM, Some(2000), Confidence::High);
        let perm_early = mk(RA_PERM, Some(1000), Confidence::High);
        let plan = mk(
            SessionState::RequiresAction { reason: Reason::Plan },
            Some(10),
            Confidence::High,
        );
        let low = mk(RA_PERM, Some(1), Confidence::Low);
        let running = mk(SessionState::Running, None, Confidence::High);
        board.sessions =
            vec![running, plan.clone(), low, perm_late.clone(), perm_early.clone()];
        let q = attention_queue(&board);
        let ids: Vec<_> = q.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![perm_early.id, perm_late.id, plan.id]);
    }
}
