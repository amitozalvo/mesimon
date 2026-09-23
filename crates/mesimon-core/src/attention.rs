//! The attention state machine (11 §11.7) — pure, time-injected, table-tested.
//! Precedence ranks are fixed forever (D28); ranks 0–8 are the attention set:
//! exactly these produce the `needs you` count and light a card. The daemon
//! owns one `Machine` per session and applies `Signal`s from the hook stream;
//! debounce rules are 11 §11.7.4.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::board::{
    Board, Confidence, ExitReason, FailReason, Reason, SessionRecord, SessionState, StopReason,
    UnknownReason,
};

/// Leave-settle for `RequiresAction` and `Running -> Idle` (11 §11.7.4).
pub const SETTLE_MS: u64 = 1500;
/// Leaving `Throttled` settles longer (11 §11.7.4).
pub const THROTTLE_LEAVE_MS: u64 = 5000;
/// A `RequiresAction` with no clearing event demotes — never latches red.
/// Measured from the last time the wait was AFFIRMED, not from entry: a
/// dialog the transcript still shows open is a wait that has not lost its
/// clearing event (T-363).
pub const STALE_DEMOTE_MS: u64 = 15 * 60 * 1000;
/// A background park is a CLOCK, not a latch. `Idle{Background}` says the
/// lead's turn is paused on work it started and resumes on its own — and
/// nothing in the hook stream is obliged to say that work ended. Claude Code
/// lists a teammate `running` for its whole life and reports only the
/// transitions, so a teammate that dies on a usage limit, or an idle notice
/// that never lands, holds the count above the idle set for good (dogfood
/// 2026-09-18, T-403: eight teammates, four idle notices, a session that
/// finished its turn at 19:44 still spelling "working" hours later). Measured
/// from the last frame that PROVED background work alive, not from entry, so
/// a slow task that still reports keeps its park (cf. `STALE_DEMOTE_MS`).
pub const PARK_STALE_MS: u64 = 10 * 60 * 1000;
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
    /// Total by construction, but `Clear` and `Resume` no longer reach it:
    /// both are in-app conversation handoffs that `target` refuses to treat
    /// as exits at all, so `Cleared`/`Resumed` are now only ever read back
    /// out of a state file an older build wrote.
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
    /// A tool call with no result yet — the session is working, in a tool.
    ToolInFlight,
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

/// A provider's stated outcome after its adapter has accounted for pending
/// interactions and background work. Interruption and failure never imply
/// successful completion, regardless of a provider's "idle" status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed(FailReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Native input is ready, with no turn or unresolved attention request.
    /// An empty prompt is not evidence of a successfully completed turn.
    Ready,
    /// An adapter has established a current active top-level turn. It emits
    /// this again when all attention requests resolve and that turn continues.
    TurnStarted,
    TurnEnded {
        outcome: TurnOutcome,
    },
    /// The adapter's request ledger supplies the highest-priority unresolved
    /// attention reason. A sibling's resolution must not clear another ask.
    Attention {
        reason: Reason,
    },
    /// The observer lost continuity. Cancels a pending completion immediately;
    /// the daemon independently holds checkout operations until reconciliation.
    ObservationLost,
    SessionStart {
        source: StartSource,
    },
    SessionEnd {
        kind: EndKind,
    },
    UserPromptSubmit,
    /// A stopped lead with live agent tasks is working; watch-only work is
    /// monitoring. Teammates are weighed against their explicit idle notices.
    Stop {
        stop_hook_active: bool,
        has_agent_id: bool,
        blocking_tasks: bool,
        monitoring_tasks: bool,
        teammates: usize,
    },
    SubagentStop,
    /// Task evidence can reclassify a parked turn, but never clear attention.
    BackgroundChanged {
        liveness: crate::background::Liveness,
    },
    /// A named in-process teammate is about to go idle. It stays alive (and
    /// listed as `running` in every later Stop payload), so this frame is the
    /// only thing that says its work is done.
    TeammateIdle {
        name: Option<String>,
    },
    /// The lead (or another teammate) sent a teammate a message, which wakes
    /// it: whatever it reported before, it is working again.
    TeammateMessaged {
        name: String,
    },
    StopFailure {
        class: StopFailureClass,
    },
    PermissionRequest,
    PermissionDenied,
    PreToolUse {
        tool: AttentionTool,
    },
    /// The narrow post-tool pair only: fires when the user has ANSWERED the
    /// question / resolved the plan dialog, which is the only mid-turn moment
    /// the `RequiresAction` can truthfully drop back to `Running`.
    PostToolUse {
        tool: AttentionTool,
    },
    /// Broad PostToolUse (any other tool). A tool only completes after its
    /// dialog was allowed, so this is the accept path for a held generic
    /// permission — there is no "permission answered" event (11 §11.7.3).
    /// `nested` marks a frame carrying an `agent_id`: a subagent's or
    /// teammate's tool ran, not the session's own (measured 2026-09-01 — the
    /// parent's frames carry no `agent_id`, a subagent's do). Only the
    /// session's OWN completion proves ITS turn resumed.
    ToolCompleted {
        nested: bool,
    },
    /// Broad PreToolUse (any tool but the interaction pair): the model has
    /// issued a tool call. Mid-turn that says nothing new — but while a
    /// plan or question dialog is held it is the only frame that says the
    /// dialog was REFUSED. A human's "No, keep planning" (or an Esc out of
    /// a question) fires no hook at all — no `PostToolUse`, no
    /// `PostToolUseFailure`, no `PermissionDenied` (that one is auto mode's
    /// classifier) — and the model's next call is the first sign it has the
    /// answer (T-447, 2026-09-23: a refused plan wore "plan" through four
    /// tool completions until the agent's next question relabelled it).
    /// `nested` as on `ToolCompleted`.
    ToolStarted {
        nested: bool,
    },
    Notification {
        kind: NotificationKind,
    },
    Elicitation,
    ElicitationResult,
    /// tmux pane-died — authoritative for exit (spike T-7).
    PaneDied {
        status: Option<i32>,
    },
    /// Daemon-side probe while `Spawning` (11 §11.5.3 approximation).
    /// `resume` marks a `--resume` spawn: a modal there is the resume-from-
    /// summary dialog (09 §9), not first-run setup.
    SpawnProbe {
        bytes: bool,
        osc0: bool,
        resume: bool,
    },
    /// Daemon-side probe while `Running`: the pane stopped painting past the
    /// quiet threshold (60 s — a working pane goes quiet for 6–10 s routinely
    /// and ~50 s while a large tool input streams, dogfood 2026-09-02, so
    /// only a long silence says the turn is over). An Esc interrupt still fires
    /// no hook (spike S-E), but current Claude Code DOES write an interrupt
    /// record to the transcript, and that is the primary catch (poll_tails'
    /// abort-only class → `TranscriptHint{AbortedMidStream}`, dogfood
    /// 2026-08-30: post-turn painting kept panes "active" for 60–80 s, so
    /// this probe alone left interrupted cards on "working"). PaneQuiet
    /// remains the fallback for a record that never lands — and the daemon
    /// holds it while the transcript shows a tool in flight or a fresh reply
    /// (`tail::turn_in_flight`, T-439): a live Claude Code pane can write no
    /// byte for over a minute while a tool runs.
    PaneQuiet,
    /// Daemon-side probe while `Running`: Claude Code's own
    /// `~/.claude/sessions/<pid>.json` reads `status: idle`, stamped after
    /// this Running spell began. The recordless Esc — pressed before the
    /// first assistant output, the prompt handed back to the box — writes
    /// NOTHING to the transcript (spike S-E's case, seen live 2026-09-04),
    /// so PaneQuiet a minute later was its whole catch; the status file
    /// flips at the keypress. Same row as PaneQuiet, same confidence — but
    /// the file flips `idle` at the end of EVERY turn, milliseconds before
    /// the Stop hook fires, so the probe races the hook on every turn and a
    /// late or lost Stop read as an Esc (simbly T-11, 2026-09-05: a relinked
    /// hook binary stalled 41 s in exec). `turn_done` is the transcript's
    /// word (`tail::turn_done_since`): the turn that began this spell has
    /// closed, so the row is `Idle{EndTurn}` rather than `Interrupted`.
    StatusFileIdle {
        turn_done: bool,
    },
    /// The same live session was observed waiting while Permission was held,
    /// then busy at a strictly newer timestamp. Qualified by the daemon.
    StatusFilePermissionResumed,
    PreCompact {
        manual: bool,
    },
    PostCompact {
        manual: bool,
    },
    /// Observe tier: derived from an adopted session's transcript tail.
    TranscriptHint {
        kind: TailHint,
    },
}

/// A `background_tasks[]` entry that is an in-process teammate (Claude Code
/// labels the `in_process_teammate` task `"teammate"`; the raw discriminant
/// is accepted too).
pub fn is_teammate_task(kind: &str) -> bool {
    norm_task_kind(kind).contains("teammate")
}

fn norm_task_kind(kind: &str) -> String {
    kind.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
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

/// A bounded diagnostic projection: no prompt, tool input, or teammate names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineView {
    pub state: SessionState,
    pub confidence: Confidence,
    pub entered_at: u64,
    pub pending: Option<PendingView>,
    pub pinned_until: Option<u64>,
    pub idle_teammates: usize,
    pub manual_compaction_prior: Option<SessionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingView {
    pub state: SessionState,
    pub confidence: Confidence,
    pub deadline: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub signal: String,
    pub outcome: &'static str,
    pub before: MachineView,
    pub after: MachineView,
}

/// Per-session machine. `apply` handles a signal (enters are immediate, leaves
/// are scheduled); `tick` fires scheduled work. All times are epoch ms,
/// injected by the caller.
#[derive(Debug, Clone)]
pub struct Machine {
    state: SessionState,
    confidence: Confidence,
    entered_at: u64,
    /// When the current state was last stated or restated — entry, or a
    /// later signal re-affirming it. The stale clock runs from here, so an
    /// attention state the evidence keeps confirming never demotes (T-363:
    /// a plan left open over lunch went `?` at fifteen minutes while the
    /// agent still waited on it).
    affirmed_at: u64,
    pending: Option<Pending>,
    /// Timestamps of committed (debounced) changes, for the flap guard.
    committed: Vec<u64>,
    pinned_until: Option<u64>,
    /// Last attention reason left, for the 30 s re-emit suppression.
    recent_left: Option<(Reason, u64)>,
    /// Named teammates that have reported idle and not been messaged since.
    /// A Stop payload lists a teammate as `running` for its whole life, so
    /// this set is the only way to tell "reviewers still working" from
    /// "reviewers done, lead done" — the daemon persists it on the record so
    /// a restart does not re-park a finished session.
    idle_teammates: BTreeSet<String>,
    /// When work the lead is parked on was last PROVED alive — entry into
    /// the park, or a subagent/teammate/nested-tool frame since.
    /// `PARK_STALE_MS` runs from here.
    background_at: u64,
    manual_compact_prior: Option<(SessionState, Confidence)>,
}

impl Machine {
    pub fn new(state: SessionState, now: u64) -> Self {
        Self::restore(state, Confidence::High, now)
    }

    /// Re-mint with the persisted idle-teammate set (see `idle_teammates`).
    pub fn restore_with_teammates(
        state: SessionState,
        confidence: Confidence,
        idle_teammates: impl IntoIterator<Item = String>,
        now: u64,
    ) -> Self {
        let mut m = Self::restore(state, confidence, now);
        m.idle_teammates = idle_teammates.into_iter().collect();
        m
    }

    /// The teammates currently known idle, sorted — for persistence.
    pub fn idle_teammates(&self) -> impl Iterator<Item = &str> {
        self.idle_teammates.iter().map(String::as_str)
    }

    /// Re-mint from a persisted record, carrying its confidence — a restart
    /// must not launder a tail-derived Low into High (dogfood 2026-08-30:
    /// that laundering blinded ToolCompleted's inferred-idle recovery, so a
    /// misread session stayed glyph-less across restarts).
    pub fn restore(state: SessionState, confidence: Confidence, now: u64) -> Self {
        Self {
            state,
            confidence,
            entered_at: now,
            affirmed_at: now,
            pending: None,
            committed: Vec::new(),
            pinned_until: None,
            recent_left: None,
            idle_teammates: BTreeSet::new(),
            background_at: now,
            manual_compact_prior: None,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn view(&self) -> MachineView {
        MachineView {
            state: self.state.clone(),
            confidence: self.confidence,
            entered_at: self.entered_at,
            pinned_until: self.pinned_until,
            idle_teammates: self.idle_teammates.len(),
            manual_compaction_prior: self
                .manual_compact_prior
                .as_ref()
                .map(|(state, _)| state.clone()),
            pending: self.pending.as_ref().map(|p| PendingView {
                state: p.to.clone(),
                confidence: p.confidence,
                deadline: p.deadline,
            }),
        }
    }

    /// Use the same reducer as production and expose no-op decisions too.
    pub fn apply_explained(&mut self, sig: &Signal, now: u64) -> (Option<Change>, Decision) {
        let before = self.view();
        let target = self.target(sig);
        let blocked = if self.state == SessionState::Sleeping {
            "sleeping_latch"
        } else if matches!(self.state, SessionState::Exited { .. }) {
            "exited_latch"
        } else if target.is_none() {
            "no_transition_rule"
        } else if self.pinned_until.is_some_and(|until| now < until)
            && target.as_ref().is_some_and(|(state, confidence)| {
                *confidence != Confidence::High
                    && !matches!(state, SessionState::Exited { .. } | SessionState::Failed { .. })
            })
        {
            "inference_flap_guard"
        } else {
            "unchanged"
        };
        let change = self.apply(sig, now);
        let after = self.view();
        let outcome = if before.state == after.state && before.confidence != after.confidence {
            "confidence_raised"
        } else if change.is_some() {
            "committed"
        } else if before.pending != after.pending {
            if after.pending.is_some() {
                "settling"
            } else {
                "settle_cancelled"
            }
        } else if before.manual_compaction_prior != after.manual_compaction_prior {
            "compaction_updated"
        } else if before.idle_teammates != after.idle_teammates {
            "background_updated"
        } else {
            blocked
        };
        // Only the enum variant name, never a Debug payload containing names.
        let signal = format!("{sig:?}")
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("Unknown")
            .to_string();
        (change, Decision { signal, outcome, before, after })
    }

    pub fn apply(&mut self, sig: &Signal, now: u64) -> Option<Change> {
        // Teammate bookkeeping happens in every state, including the latched
        // ones: a teammate going idle while the lead sleeps is still a fact
        // the next Stop has to weigh.
        match sig {
            Signal::TeammateIdle { name: Some(name) } => {
                self.idle_teammates.insert(name.clone());
            }
            Signal::TeammateMessaged { name } => {
                self.idle_teammates.remove(name);
            }
            _ => {}
        }
        // ...and so does the park clock, for the same reason: the frames that
        // prove a park is still earned are exactly the ones `target` answers
        // `None` to, so nothing else in this machine ever sees them.
        if proves_background(sig) {
            self.background_at = now;
        }
        // Sleeping latches: the daemon's own SIGTERM produces SessionEnd and
        // pane-died, and neither those nor any straggler frame may flip a
        // parked session to Exited. Only wake leaves — by re-minting the
        // machine as Spawning (11 §11.7.3).
        if self.state == SessionState::Sleeping {
            return None;
        }
        // Exited is terminal: publish once (02 §7.3). A late SessionEnd may
        // refine a pane-derived reason in place, silently — except the two
        // in-app kinds, `resume` and `clear`, which end a CONVERSATION inside
        // a living pane and must never relabel a real death.
        if let SessionState::Exited { reason } = &self.state {
            if let Signal::SessionEnd { kind } = sig {
                if !matches!(kind, EndKind::Resume | EndKind::Clear)
                    && matches!(reason, ExitReason::UserQuit | ExitReason::Crashed)
                {
                    self.state = SessionState::Exited { reason: kind.exit_reason() };
                }
            }
            return None;
        }

        match sig {
            Signal::PreCompact { manual: true } => {
                if self.manual_compact_prior.is_none() {
                    self.manual_compact_prior = Some((self.state.clone(), self.confidence));
                }
            }
            Signal::PreCompact { manual: false }
            | Signal::UserPromptSubmit
            | Signal::TurnStarted
            | Signal::Ready
            | Signal::ObservationLost => {
                self.manual_compact_prior = None;
            }
            Signal::SessionStart { source } if *source != StartSource::Compact => {
                self.manual_compact_prior = None;
            }
            _ => {}
        }
        let (to, conf) = self.target(sig)?;

        // Flap pin: while pinned, only a STATED transition or a terminal one
        // gets through. The guard exists for evidence that argues with
        // itself — a probe, a transcript tail, a silence — not for Claude's
        // own hooks: a `Stop` is the turn ending whatever came before it, and
        // dropping it (2026-09-04, four asks in twenty seconds) left a
        // finished session `Running` until the probe called it interrupted.
        if let Some(until) = self.pinned_until {
            if now < until
                && conf != Confidence::High
                && !matches!(to, SessionState::Exited { .. } | SessionState::Failed { .. })
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
            // ...and re-arms the stale clock: that clock catches a wait whose
            // clearing event was lost, and a wait restated is one that has
            // not cleared. The transcript tail is the source that repeats —
            // the recovery adapter re-reads a pending `ExitPlanMode` or
            // `AskUserQuestion` off the tail once a minute for as long as the
            // dialog is open (T-363). Any confidence: the clock is not a
            // claim about the state, only about its age.
            self.affirmed_at = now;
            if (conf as u8) < (self.confidence as u8) {
                self.confidence = conf;
                // The daemon must persist/publish the stronger evidence and
                // re-evaluate movement even when the label is unchanged.
                return Some(Change {
                    from: self.state.clone(),
                    to: self.state.clone(),
                    attention_added: false,
                    confidence: conf,
                });
            }
            return None;
        }

        let delay =
            if matches!(sig, Signal::ObservationLost) { 0 } else { delay_for(&self.state, &to) };
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
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            return self.flush(now);
        }
        // Stale demotion: never latch red (11 §11.7.4) — but never drop a
        // wait the evidence keeps affirming either (T-363).
        if is_attention(&self.state) && now.saturating_sub(self.affirmed_at) >= STALE_DEMOTE_MS {
            let to = SessionState::Unknown { reason: UnknownReason::NoSignal };
            return Some(self.commit(to, Confidence::Stale, now));
        }
        // Park demotion: a park nothing has proved in PARK_STALE_MS falls back
        // to what the `Stop` that made it plainly said — the turn ended. Only
        // the background half of that Stop was ever inference, so only the
        // confidence drops; any later frame promotes straight back to Running.
        // `Idle{Monitoring}` is left alone: a watch is MEANT to be silent, and
        // it already spells itself "monitoring" and counts as quiet.
        if self.state == (SessionState::Idle { stop_reason: StopReason::Background })
            && now.saturating_sub(self.background_at) >= PARK_STALE_MS
        {
            let to = SessionState::Idle { stop_reason: StopReason::EndTurn };
            return Some(self.commit(to, Confidence::Medium, now));
        }
        None
    }

    /// Commit the pending transition NOW, settle or no settle. The daemon's
    /// shutdown road: a leave still inside its 1500 ms window is a frame the
    /// session already sent, and a restart re-derives it from the transcript
    /// at Low confidence, which automove refuses — so a `Stop` one second
    /// before a `U` reload left its ticket in IN PROGRESS (dogfood 2026-09-01,
    /// T-140). The settle exists to absorb a re-trigger; at exit there is
    /// nothing left to absorb. Stale demotion is not flushed: it is a clock,
    /// not a signal.
    pub fn flush(&mut self, now: u64) -> Option<Change> {
        let p = self.pending.take()?;
        Some(self.commit(p.to, p.confidence, now))
    }

    fn commit(&mut self, to: SessionState, confidence: Confidence, now: u64) -> Change {
        let from = std::mem::replace(&mut self.state, to.clone());
        if let SessionState::RequiresAction { reason } = &from {
            self.recent_left = Some((*reason, now));
        }
        self.entered_at = now;
        self.affirmed_at = now;
        self.background_at = now;

        // Flap guard: >FLAP_MAX committed changes in the window pins the
        // machine at the state just committed. (Deviation from 11 §11.7.4's
        // "lowest rank seen": we pin in place — simpler, and the pinned state
        // is at most one transition away from that.) A stated, High
        // transition keeps its confidence through the pin: it is Claude
        // saying so, and marking it Low made `automove` refuse a move the
        // user's own prompt had just asked for. The pin still arms, so the
        // inferred signals stay out while the hooks are this busy.
        self.committed.retain(|t| now.saturating_sub(*t) <= FLAP_WINDOW_MS);
        self.committed.push(now);
        let mut confidence = confidence;
        if self.committed.len() > FLAP_MAX {
            self.pinned_until = Some(now + FLAP_PIN_MS);
            if confidence != Confidence::High {
                confidence = Confidence::Low;
            }
        }
        self.confidence = confidence;

        let suppressed = match (&to, &self.recent_left) {
            (SessionState::RequiresAction { reason }, Some((left, at))) => {
                reason == left && now.saturating_sub(*at) < REEMIT_SUPPRESS_MS
            }
            _ => false,
        };
        let newly = match (&from, &to) {
            (
                SessionState::RequiresAction { reason: a },
                SessionState::RequiresAction { reason: b },
            ) => a != b,
            (_, SessionState::RequiresAction { .. }) => true,
            _ => false,
        };
        let attention_added =
            newly && !suppressed && matches!(confidence, Confidence::High | Confidence::Medium);

        Change { from, to, attention_added, confidence }
    }

    /// The 11 §11.7.3 transition table, restricted to the M2 registered set.
    /// `None` = the signal produces no top-level transition from this state.
    fn target(&self, sig: &Signal) -> Option<(SessionState, Confidence)> {
        use SessionState as S;
        let t = |s: S| Some((s, Confidence::High));
        match sig {
            Signal::Ready => t(S::Idle { stop_reason: StopReason::Unknown }),
            Signal::TurnStarted => t(S::Running),
            Signal::TurnEnded { outcome } => match outcome {
                TurnOutcome::Completed => t(S::Idle { stop_reason: StopReason::EndTurn }),
                TurnOutcome::Interrupted => t(S::Idle { stop_reason: StopReason::Interrupted }),
                TurnOutcome::Failed(reason) => t(S::Failed { reason: *reason }),
            },
            Signal::Attention { reason } => t(S::RequiresAction { reason: *reason }),
            Signal::ObservationLost => t(S::Unknown { reason: UnknownReason::ObservationLost }),
            // A session that just started sits at the prompt — that is idle,
            // not working (dogfood 2026-08-30: fresh spawns read "working"
            // forever). The one exception: a compact-restart fires
            // SessionStart mid-turn and the turn continues.
            // PreCompact makes maintenance visible. Keep the saved state
            // through PostCompact: an async SessionStart may arrive afterward.
            Signal::PreCompact { .. } => t(S::Running),
            Signal::PostCompact { manual: true } => Some(
                self.manual_compact_prior
                    .clone()
                    .unwrap_or((S::Idle { stop_reason: StopReason::Unknown }, Confidence::Low)),
            ),
            Signal::PostCompact { manual: false } => t(S::Running),
            Signal::SessionStart { source: StartSource::Compact } => {
                if self.manual_compact_prior.is_some() {
                    None
                } else {
                    t(S::Running)
                }
            }
            Signal::SessionStart { .. } => t(S::Idle { stop_reason: StopReason::Unknown }),
            // In-app `/resume` and `/clear` end the CONVERSATION, not the
            // process: Claude Code fires SessionEnd{reason:X} then
            // SessionStart{source:X} in the same live pane (dogfood
            // 2026-08-30: honoring it as an exit stranded a live session as a
            // corpse and wedged every later resume). `clear` was the same bug
            // wearing a different word and kept it for a year of the corpus —
            // it is the harsher one, because the SessionStart that follows is
            // then swallowed by the terminal latch and the session reads dead
            // for as long as the pane goes on living. Identity moves via the
            // SessionStart frame's transcript_path; real death still arrives
            // as PaneDied.
            Signal::SessionEnd { kind: EndKind::Resume | EndKind::Clear } => None,
            Signal::SessionEnd { kind } => t(S::Exited { reason: kind.exit_reason() }),
            Signal::UserPromptSubmit => t(S::Running),
            // stop_hook_active describes a PREVIOUS continuation. It does not
            // say this stop is blocked (real 2.1.266 stop-continuation capture).
            Signal::Stop { has_agent_id: true, .. } => None, // nested, never top-level
            // Keep a parked lead out of Running: its quiet pane is expected.
            Signal::Stop { blocking_tasks: true, .. } => {
                t(S::Idle { stop_reason: StopReason::Background })
            }
            // Teammates are listed `running` idle or busy (they live until
            // the session ends), so the payload cannot say whether they hold
            // the turn open — the `TeammateIdle` frames can. More teammates
            // than idle notices means at least one is still working: parked.
            // Every one accounted for means the lead's turn ending IS the
            // end (dogfood 2026-09-01, T-135: four finished reviewers held a
            // finished session at `Idle{Background}` through three Stops).
            Signal::Stop { teammates, .. } if *teammates > self.idle_teammates.len() => {
                t(S::Idle { stop_reason: StopReason::Background })
            }
            Signal::Stop { monitoring_tasks: true, .. } => {
                t(S::Idle { stop_reason: StopReason::Monitoring })
            }
            Signal::Stop { .. } => t(S::Idle { stop_reason: StopReason::EndTurn }),
            Signal::BackgroundChanged { liveness } => {
                let parked = |s: &S| {
                    matches!(
                        s,
                        S::Idle { stop_reason: StopReason::Background | StopReason::Monitoring }
                    )
                };
                if parked(&self.state)
                    || self.pending.as_ref().is_some_and(|p| parked(&p.to))
                    || (*liveness != crate::background::Liveness::None
                        && (matches!(self.state, S::Idle { .. })
                            || self
                                .pending
                                .as_ref()
                                .is_some_and(|p| matches!(p.to, S::Idle { .. }))))
                {
                    t(S::Idle {
                        stop_reason: match liveness {
                            crate::background::Liveness::Working => StopReason::Background,
                            crate::background::Liveness::Monitoring => StopReason::Monitoring,
                            // Task completion is not proof the lead has delivered its final response.
                            crate::background::Liveness::None => StopReason::Unknown,
                        },
                    })
                } else {
                    None
                }
            }

            // A subagent finishing proves the parent is still orchestrating.
            // The restart-window tail re-derive reads "waiting on background
            // subagents" as done — the parent's turn genuinely ends in the
            // transcript while they run — so a sub-High Idle/Unknown here is
            // a misread: promote back to Running at Medium (inference, not a
            // stated event). A High-confidence Idle came from a real Stop
            // (which reports in-flight work via background_tasks) and stands.
            Signal::SubagentStop => match &self.state {
                S::Idle { .. } | S::Unknown { .. } if self.confidence != Confidence::High => {
                    Some((S::Running, Confidence::Medium))
                }
                _ => None,
            },
            Signal::TeammateIdle { .. } | Signal::TeammateMessaged { .. } => None,
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
            // Generic tool completion clears ONLY a held permission dialog
            // (the accept path — dogfood 2026-08-30: an accepted tool stayed
            // needs-you until end of turn). From anywhere else it says
            // nothing: a completion from a parallel sibling must not clear a
            // Question/Plan, and mid-turn Running needs no re-assert. A
            // sibling completing while a DIFFERENT dialog is held can clear
            // early (no key to join on — PermissionRequest carries no
            // tool_use_id); the idle permission Notification re-asserts at
            // Medium, so the miss self-heals.
            Signal::ToolCompleted { nested } => match &self.state {
                S::RequiresAction { reason: Reason::Permission } => t(S::Running),
                // The lead's OWN tool completing is stated proof its turn is
                // alive, from ANY idle. A parked turn resumes on it: the wake
                // is a prompt only when a task notification delivers it; a
                // teammate's report arrives as a teammate message and fires
                // no `UserPromptSubmit` at all (measured 2026-09-01, T-135:
                // twenty minutes of the lead's tool frames streamed past a
                // High `Idle{Background}` that only a prompt could leave).
                // And so does a FINISHED one (T-228, 2026-09-05): a `!` bash
                // command in Claude Code puts its output into the conversation
                // and the model takes a turn on it, and no `UserPromptSubmit`
                // fires — the first frame of that turn is a `PostToolUse`,
                // and holding a hook-stated `EndTurn` against it left the
                // ticket in REVIEW with no working mark for three minutes,
                // until a `PermissionDenied` happened to promote it. The rule
                // this replaces — "a background task's completion must not
                // flip a real end_turn" — guarded a frame that does not
                // exist: a backgrounded shell's completion emits no
                // `PostToolUse` (captured, T-135); its single frame is at
                // launch. The frame is a stated event, so High. A NESTED
                // completion is a subagent's or teammate's work, not the
                // lead's — the lead may well still be parked, or done — and
                // says nothing here. A straggler after a real end would cost
                // a "working" the quiet probe re-demotes; none has been seen.
                S::Idle { .. } if !nested => t(S::Running),
                // Any completion, nested too, outranks an INFERRED resting
                // state (quiet-probe Medium, tail-hint Low) and the
                // post-restart Unknown — the recovery probe_activity's "next
                // real event corrects" promise relies on (dogfood
                // 2026-08-30: a quiet-probe misfire, then a restart-tail
                // StaleQuiet misread, each left a working session glyph-less
                // on "idle" while PostToolUse frames streamed in).
                S::Idle { .. } if self.confidence != Confidence::High => t(S::Running),
                S::Unknown { .. } => t(S::Running),
                _ => None,
            },
            // The model cannot call a tool while its own dialog is open: the
            // interaction tools run serially, and a new call is emitted only
            // once the dialog's result is back. So the session's OWN next
            // call while a Plan/Question is held is the refusal road — the
            // accept road is the pair's `PostToolUse` above, and it arrives
            // first when it arrives. A nested call is a subagent's and says
            // nothing about the lead's dialog. From a held Permission the
            // sibling argument is the same one `ToolCompleted` already
            // accepts (a parallel sibling may clear early; the idle
            // permission Notification re-asserts at Medium). Elsewhere the
            // frame is mirrored on `ToolCompleted`, which follows it by the
            // tool's duration: a turn resumed without a prompt shows on its
            // first frame, which is this one.
            Signal::ToolStarted { nested } => match &self.state {
                S::RequiresAction {
                    reason: Reason::Plan | Reason::Question | Reason::Permission,
                } if !nested => t(S::Running),
                S::Idle { .. } if !nested => t(S::Running),
                S::Idle { .. } if self.confidence != Confidence::High => t(S::Running),
                S::Unknown { .. } => t(S::Running),
                _ => None,
            },
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
                        Some((S::RequiresAction { reason: Reason::Permission }, Confidence::Medium))
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
                    let reason = if *resume { Reason::ResumeDialog } else { Reason::StartupModal };
                    Some((S::RequiresAction { reason }, Confidence::Medium))
                } else if !*bytes {
                    Some((S::unknown(), Confidence::Low))
                } else {
                    None
                }
            }
            // The 11 §11.7.3 interrupt row, activity-approximated: only ever a
            // demotion out of Running, never a promotion — Medium because byte
            // silence is inference, not a stated event. And never over a leave
            // already pending: a `Stop` settles for 1500 ms, the status file
            // goes idle in the same second, and a probe on a 2 s cadence that
            // replaced the pending EndTurn turned a finished turn into
            // "interrupted" — which automove does not promote (2026-09-04).
            Signal::StatusFilePermissionResumed => {
                if self.state == (S::RequiresAction { reason: Reason::Permission }) {
                    Some((S::Running, Confidence::Medium))
                } else {
                    None
                }
            }
            Signal::PaneQuiet | Signal::StatusFileIdle { .. } => {
                if self.state == S::Running && self.pending.is_none() {
                    let stop_reason = match sig {
                        Signal::StatusFileIdle { turn_done: true } => StopReason::EndTurn,
                        _ => StopReason::Interrupted,
                    };
                    Some((S::Idle { stop_reason }, Confidence::Medium))
                } else {
                    None
                }
            }
            Signal::TranscriptHint { kind } => {
                let s = match kind {
                    TailHint::AssistantText | TailHint::ToolInFlight => S::Running,
                    TailHint::AskUserQuestion => S::RequiresAction { reason: Reason::Question },
                    TailHint::ExitPlanMode => S::RequiresAction { reason: Reason::Plan },
                    TailHint::TurnComplete => S::Idle { stop_reason: StopReason::EndTurn },
                    TailHint::AbortedMidStream => S::Idle { stop_reason: StopReason::Interrupted },
                    TailHint::StaleQuiet => S::Idle { stop_reason: StopReason::Unknown },
                };
                Some((s, Confidence::Low))
            }
        }
    }
}

/// Frames that prove work the lead is parked on is still alive. Every one of
/// them is deliberately `None` in `target` — a subagent's or teammate's step
/// says nothing about the LEAD's state — which is why the park clock has to
/// read them here: they are the only evidence a park is still earned, and
/// without them the clock would time out a team that is plainly working.
fn proves_background(sig: &Signal) -> bool {
    match sig {
        Signal::SubagentStop | Signal::TeammateIdle { .. } | Signal::TeammateMessaged { .. } => {
            true
        }
        // A nested stop is a subagent's turn ending, not the lead's.
        Signal::Stop { has_agent_id, .. } => *has_agent_id,
        Signal::ToolCompleted { nested } | Signal::ToolStarted { nested } => *nested,
        Signal::BackgroundChanged { liveness } => *liveness != crate::background::Liveness::None,
        _ => false,
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
    fn normalized_turns_preserve_settle_and_only_success_moves_to_done() {
        let settings = crate::board::template_settings("IN PROGRESS").unwrap();
        for outcome in [
            TurnOutcome::Completed,
            TurnOutcome::Interrupted,
            TurnOutcome::Failed(FailReason::Server),
        ] {
            let mut machine = m(SessionState::Spawning);
            let ready = machine.apply(&Signal::Ready, 0).expect("ready");
            assert_eq!(
                crate::automove::automove(&settings, &ready.to, ready.confidence),
                None,
                "a ready prompt must not count as success"
            );
            machine.apply(&Signal::TurnStarted, 1).expect("turn starts immediately");
            let ended = machine.apply(&Signal::TurnEnded { outcome }, 2);
            let change = match outcome {
                TurnOutcome::Failed(_) => ended.expect("failure is immediate"),
                _ => {
                    assert!(ended.is_none());
                    assert!(machine.tick(SETTLE_MS + 1).is_none());
                    machine.tick(SETTLE_MS + 2).expect("completion settles")
                }
            };
            assert_eq!(
                crate::automove::automove(&settings, &change.to, change.confidence),
                (outcome == TurnOutcome::Completed).then_some("REVIEW")
            );
        }
    }

    #[test]
    fn normalized_observation_loss_cancels_pending_completion_immediately() {
        for state in [SessionState::Running, RA_PERM, SessionState::Throttled] {
            let mut machine = m(state);
            assert!(machine
                .apply(&Signal::TurnEnded { outcome: TurnOutcome::Completed }, 10)
                .is_none());
            assert!(machine.view().pending.is_some());
            let change = machine.apply(&Signal::ObservationLost, 11).expect("gap is immediate");
            assert_eq!(change.to, SessionState::Unknown { reason: UnknownReason::ObservationLost });
            assert!(machine.view().pending.is_none());
            assert!(machine.tick(THROTTLE_LEAVE_MS + 20).is_none());
            assert!(!matches!(
                machine.state(),
                SessionState::Idle { stop_reason: StopReason::EndTurn }
            ));
        }
    }

    #[test]
    fn normalized_attention_retrigger_cancels_its_pending_leave() {
        let mut machine = m(SessionState::Running);
        let question = Signal::Attention { reason: Reason::Question };
        assert!(machine.apply(&question, 0).unwrap().attention_added);
        assert!(machine.apply(&Signal::TurnStarted, 10).is_none());
        assert!(machine.view().pending.is_some());
        assert!(machine.apply(&question, 20).is_none());
        assert!(machine.view().pending.is_none());
        assert!(machine.tick(SETTLE_MS + 20).is_none());
        assert_eq!(machine.state(), &SessionState::RequiresAction { reason: Reason::Question });
        assert!(machine.apply(&Signal::TurnStarted, 2000).is_none());
        assert_eq!(machine.tick(2000 + SETTLE_MS).unwrap().to, SessionState::Running);
    }

    #[test]
    fn normalized_signals_do_not_unpark_or_revive_sessions() {
        for state in [SessionState::Sleeping, SessionState::Exited { reason: ExitReason::Killed }] {
            let mut machine = m(state.clone());
            for signal in [
                Signal::Ready,
                Signal::TurnStarted,
                Signal::TurnEnded { outcome: TurnOutcome::Completed },
                Signal::TurnEnded { outcome: TurnOutcome::Interrupted },
                Signal::TurnEnded { outcome: TurnOutcome::Failed(FailReason::Server) },
                Signal::Attention { reason: Reason::Permission },
                Signal::ObservationLost,
            ] {
                assert!(machine.apply(&signal, 10).is_none());
                assert_eq!(machine.state(), &state);
                assert!(machine.view().pending.is_none());
            }
        }
    }

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

    /// A pending leave commits on `flush` at once, keeps the confidence the
    /// signal carried, and leaves nothing behind for the next tick. With
    /// nothing pending, flush is a no-op — an idle machine is not disturbed
    /// by a shutdown.
    #[test]
    fn flush_commits_a_pending_leave_immediately() {
        let mut m = m(SessionState::Running);
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(m.apply(&stop, 1000).is_none(), "leaving Running settles");
        let c = m.flush(1100).expect("flush commits inside the settle window");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
        assert!(m.tick(1100 + SETTLE_MS).is_none(), "nothing left to settle");
        assert!(m.flush(5000).is_none(), "nothing pending, nothing flushed");
    }

    #[test]
    fn stop_needs_quiet_before_idle() {
        let mut m = m(SessionState::Running);
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
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
    fn continued_turn_can_finish_but_nested_stop_cannot_finish_parent() {
        let mut m = m(SessionState::Running);
        let stop = Signal::Stop {
            stop_hook_active: true,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(m.apply(&stop, 1000).is_none());
        assert!(m.pending.is_some());
        let change = m.tick(2500).expect("continued turn settles");
        assert_eq!(change.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        let mut m = Machine::new(SessionState::Running, 0);
        m.apply(
            &Signal::Stop {
                stop_hook_active: true,
                has_agent_id: true,
                blocking_tasks: false,
                monitoring_tasks: false,
                teammates: 0,
            },
            1000,
        );
        assert!(m.pending.is_none());
    }

    /// In-flight background work parks the turn — it does not hold it
    /// `Running`, and it does not end it. The distinction is the whole point:
    /// `EndTurn` would move the ticket to REVIEW, `Running` would be caught
    /// and mangled by the quiet probe (see the test below).
    #[test]
    fn stop_with_blocking_tasks_parks_the_turn() {
        let mut m = m(SessionState::Running);
        // Leaving `Running` settles for 1500 ms, so the park commits on tick.
        assert!(m
            .apply(
                &Signal::Stop {
                    stop_hook_active: false,
                    has_agent_id: false,
                    blocking_tasks: true,
                    monitoring_tasks: false,
                    teammates: 0,
                },
                1000,
            )
            .is_none());
        let c = m.tick(3000).expect("parks");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Background });
        assert_eq!(c.confidence, Confidence::High);
        assert!(!c.attention_added, "a parked turn asks nothing of the user");
    }

    /// The regression this state exists for (dogfood 2026-09-01, T-128): a
    /// backgrounded build-poll parked the turn, the pane went quiet, and the
    /// interrupt probe demoted a session nobody had interrupted. `Idle` is
    /// not `Running`, so the probe never reaches it.
    #[test]
    fn a_parked_turn_is_not_demoted_to_interrupted() {
        let mut m = m(SessionState::Running);
        m.apply(
            &Signal::Stop {
                stop_hook_active: false,
                has_agent_id: false,
                blocking_tasks: true,
                monitoring_tasks: false,
                teammates: 0,
            },
            1000,
        );
        m.tick(3000).expect("parks");
        assert!(m.apply(&Signal::PaneQuiet, 20_000).is_none(), "PaneQuiet only demotes Running");
        assert!(m.tick(40_000).is_none());
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
    }

    /// The wake path, as observed on the wire: the task's completion arrives
    /// as a `UserPromptSubmit` and the turn simply resumes.
    #[test]
    fn a_parked_turn_resumes_on_the_task_notification() {
        let mut m = m(SessionState::Idle { stop_reason: StopReason::Background });
        let c = m.apply(&Signal::UserPromptSubmit, 1000).expect("resumes");
        assert_eq!(c.to, SessionState::Running);
    }

    fn stop_with_teammates(teammates: usize) -> Signal {
        Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates,
        }
    }

    fn idle(name: &str) -> Signal {
        Signal::TeammateIdle { name: Some(name.to_string()) }
    }

    /// The T-135 regression, first half. Four named reviewers were spawned
    /// and the lead stopped to wait: that Stop listed four `teammate` entries
    /// and no idle notice had arrived, so the turn is parked, not done.
    #[test]
    fn busy_teammates_park_the_turn() {
        let mut m = m(SessionState::Running);
        assert!(m.apply(&stop_with_teammates(4), 1000).is_none(), "leave settles");
        let c = m.tick(1000 + SETTLE_MS).expect("parks");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Background });
        // Two of four have reported: still parked.
        m.apply(&idle("reuse"), 2000);
        m.apply(&idle("altitude"), 2100);
        assert!(m.apply(&stop_with_teammates(4), 3000).is_none());
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
    }

    /// The T-135 regression, second half. An idle teammate stays alive and
    /// is listed `running` in every later Stop payload, so the lead's final
    /// turn re-parked itself three times over. Once every listed teammate has
    /// reported idle, the lead's Stop is the end of the turn.
    #[test]
    fn idle_teammates_do_not_hold_the_turn() {
        let mut m = m(SessionState::Idle { stop_reason: StopReason::Background });
        for name in ["reuse", "simplify", "efficiency", "altitude"] {
            assert!(m.apply(&idle(name), 1000).is_none(), "an idle notice moves nothing by itself");
        }
        // A repeat notice is not a fifth teammate.
        m.apply(&idle("reuse"), 1500);
        let c = m.apply(&stop_with_teammates(4), 2000).expect("ends the turn");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
        // A teammate that was shut down drops out of the list; fewer listed
        // than idle is still "all accounted for".
        let mut m2 = Machine::new(SessionState::Idle { stop_reason: StopReason::Background }, 0);
        m2.apply(&idle("a"), 1000);
        m2.apply(&idle("b"), 1000);
        assert!(m2.apply(&stop_with_teammates(1), 2000).is_some());
    }

    /// T-403. The count is not enough on its own: a teammate reports its
    /// transitions, and a teammate that dies on a usage limit reports nothing
    /// ever again. Eight teammates, four idle notices, a lead whose last turn
    /// ended at 19:44 — the park never lifted and the card spelled "working"
    /// for hours. The clock is the floor under every such hole.
    #[test]
    fn a_park_nothing_proves_falls_back_to_the_turn_that_ended() {
        let mut m = m(SessionState::Running);
        for name in ["reuse", "simplification", "efficiency", "altitude"] {
            m.apply(&idle(name), 1000);
        }
        // Four of the eight are alive but will never report again.
        assert!(m.apply(&stop_with_teammates(8), 2000).is_none(), "leave settles");
        let c = m.tick(2000 + SETTLE_MS).expect("parks");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Background });
        let parked = 2000 + SETTLE_MS;
        assert!(m.tick(parked + PARK_STALE_MS - 1).is_none());
        let c = m.tick(parked + PARK_STALE_MS).expect("the park times out");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        // Inference, so Medium — and Medium is what automove asks for, which
        // is the point: the finished ticket reaches REVIEW the way a clean
        // Stop would have taken it there.
        assert_eq!(c.confidence, Confidence::Medium);
        // The demote is not a latch either: the next real frame resumes.
        let c = m.apply(&Signal::UserPromptSubmit, parked + PARK_STALE_MS + 1).expect("resumes");
        assert_eq!(c.to, SessionState::Running);
    }

    /// The other half: a team that is plainly working keeps its park, however
    /// long it takes. Every frame here is one `target` answers `None` to —
    /// that is exactly why the clock reads them itself.
    #[test]
    fn background_frames_keep_a_park_alive() {
        for sig in [
            Signal::SubagentStop,
            Signal::TeammateIdle { name: Some("reuse".into()) },
            Signal::TeammateMessaged { name: "reuse".into() },
            Signal::ToolCompleted { nested: true },
            Signal::BackgroundChanged { liveness: crate::background::Liveness::Working },
        ] {
            let mut m = m(SessionState::Idle { stop_reason: StopReason::Background });
            let mut at = 0;
            // Five spells of nine minutes, each ended by one proving frame.
            for _ in 0..5 {
                at += PARK_STALE_MS - 60_000;
                assert!(m.tick(at).is_none(), "{sig:?} left the park to time out");
                m.apply(&sig, at);
            }
            assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
            let c = m.tick(at + PARK_STALE_MS).expect("silence still times out");
            assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        }
    }

    /// A watch is MEANT to be silent — `tail -f` proves nothing by saying
    /// nothing — and it already spells itself "monitoring" and counts as
    /// quiet, so the clock does not touch it.
    #[test]
    fn a_monitoring_park_has_no_clock() {
        let mut m = m(SessionState::Idle { stop_reason: StopReason::Monitoring });
        assert!(m.tick(10 * PARK_STALE_MS).is_none());
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Monitoring });
    }

    /// Messaging an idle teammate wakes it: it is working again, whatever it
    /// last reported, and the count must say so.
    #[test]
    fn a_messaged_teammate_is_working_again() {
        let mut m = m(SessionState::Running);
        m.apply(&idle("reuse"), 1000);
        assert!(m.apply(&Signal::TeammateMessaged { name: "reuse".into() }, 2000).is_none());
        assert!(m.apply(&stop_with_teammates(1), 3000).is_none(), "leave settles");
        let c = m.tick(3000 + SETTLE_MS).expect("parks");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Background });
        // A name that never reported idle is a no-op to forget.
        m.apply(&Signal::TeammateMessaged { name: "nobody".into() }, 4000);
        assert_eq!(m.idle_teammates().count(), 0);
    }

    /// A teammate's report arrives as a teammate message and fires no
    /// `UserPromptSubmit` (measured 2026-09-01), so the lead's OWN tool frames
    /// are what say the parked turn resumed — twenty minutes of them streamed
    /// past a High `Idle{Background}` on T-135. A nested frame (a subagent's
    /// tool, `agent_id` set) is the teammate's work and moves nothing.
    #[test]
    fn a_parked_turn_resumes_on_its_own_tool_completion() {
        let mut m = m(SessionState::Idle { stop_reason: StopReason::Background });
        assert!(m.apply(&Signal::ToolCompleted { nested: true }, 1000).is_none());
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
        let c = m.apply(&Signal::ToolCompleted { nested: false }, 2000).expect("resumes");
        assert_eq!(c.to, SessionState::Running);
        assert_eq!(c.confidence, Confidence::High);
    }

    /// A turn that starts with no prompt at all: a `!` bash command in Claude
    /// Code puts its output into the conversation and the model takes a turn
    /// on it, and no `UserPromptSubmit` fires (captured 2026-09-05, T-228:
    /// Stop 14:02:21, `<bash-input>` 14:04:08, then PostToolUse frames from
    /// 14:04:34 that a High `Idle{EndTurn}` held inert — REVIEW, no working
    /// mark, until a `PermissionDenied` three minutes on). The first frame of
    /// that turn is the lead's own tool completing, and it is the turn.
    #[test]
    fn a_turn_resumed_without_a_prompt_shows_on_its_first_tool_frame() {
        let mut m = m(SessionState::Running);
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        assert!(m.apply(&stop, 1_000).is_none()); // leave settles
        let c = m.tick(1_000 + SETTLE_MS).expect("settle to end_turn");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(m.confidence(), Confidence::High);
        // A subagent's or teammate's tool says nothing about the lead.
        assert!(m.apply(&Signal::ToolCompleted { nested: true }, 5_000).is_none());
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::EndTurn });
        // The lead's own does, at once and stated.
        let c =
            m.apply(&Signal::ToolCompleted { nested: false }, 6_000).expect("the turn is alive");
        assert_eq!(c.to, SessionState::Running);
        assert_eq!(c.confidence, Confidence::High);
        // ...and the next Stop ends it as any turn ends.
        assert!(m.apply(&stop, 9_000).is_none());
        let c = m.tick(9_000 + SETTLE_MS).expect("settle to end_turn again");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
    }

    /// The set survives a daemon restart through the record, or the next
    /// finished turn would park for good again.
    #[test]
    fn restore_carries_idle_teammates() {
        let m = Machine::restore_with_teammates(
            SessionState::Idle { stop_reason: StopReason::Background },
            Confidence::High,
            ["b".to_string(), "a".to_string(), "a".to_string()],
            0,
        );
        assert_eq!(m.idle_teammates().collect::<Vec<_>>(), vec!["a", "b"]);
        let mut m = m;
        assert!(m.apply(&stop_with_teammates(2), 1000).is_some(), "all accounted for");
    }

    #[test]
    fn a_late_background_start_cancels_pending_completion() {
        let mut m = m(SessionState::Running);
        m.apply(
            &Signal::Stop {
                stop_hook_active: false,
                has_agent_id: false,
                blocking_tasks: false,
                monitoring_tasks: false,
                teammates: 0,
            },
            100,
        );
        m.apply(&Signal::BackgroundChanged { liveness: crate::background::Liveness::Working }, 200);
        m.tick(200 + SETTLE_MS);
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
    }

    #[test]
    fn background_changes_reclassify_pending_parks_without_clearing_attention() {
        use crate::background::Liveness;
        let mut m = m(SessionState::Running);
        m.apply(
            &Signal::Stop {
                stop_hook_active: false,
                has_agent_id: false,
                blocking_tasks: true,
                monitoring_tasks: true,
                teammates: 0,
            },
            100,
        );
        m.apply(&Signal::BackgroundChanged { liveness: Liveness::Monitoring }, 200);
        m.tick(200 + SETTLE_MS);
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Monitoring });
        assert!(m.apply(&Signal::PaneQuiet, 100_000).is_none());
        m.apply(&Signal::BackgroundChanged { liveness: Liveness::Working }, 101_000);
        assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::Background });
        m.apply(&Signal::PermissionRequest, 102_000);
        m.apply(&Signal::BackgroundChanged { liveness: Liveness::Monitoring }, 103_000);
        assert_eq!(m.state(), &SessionState::RequiresAction { reason: Reason::Permission });
    }

    /// The T-72 regression. An `Artifact` publish arms a comment monitor that
    /// stays live for the rest of the session, so under the old emptiness test
    /// EVERY later Stop was swallowed (`to == state`, no transition) and the
    /// pane-quiet probe mislabelled the finished turn `Interrupted` — which
    /// `automove` refuses to promote, stranding the ticket in IN PROGRESS.
    #[test]
    fn armed_monitor_settles_to_monitoring_on_each_turn() {
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: true,
            teammates: 0,
        };
        let mut m = m(SessionState::Running);
        // Two turns: the bug was that the monitor stayed armed and so ate the
        // SECOND one too.
        for turn in 0..2 {
            let base = turn * 100_000;
            assert!(m.apply(&stop, base + 1000).is_none(), "leave settles");
            let c = m.tick(base + 1000 + SETTLE_MS).expect("settles to end_turn");
            assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Monitoring });
            assert_eq!(c.confidence, Confidence::High);
            assert!(m.apply(&Signal::UserPromptSubmit, base + 50_000).is_some());
        }
    }

    #[test]
    fn subagent_stop_corrects_tail_derived_done() {
        // Restart window: the tail re-derive reads "parent waiting on
        // background subagents" as done (the parent's turn genuinely ends in
        // the transcript while they run). A SubagentStop proves the parent is
        // still orchestrating.
        let mut ma = m(SessionState::unknown());
        let c = ma
            .apply(&Signal::TranscriptHint { kind: TailHint::TurnComplete }, 1000)
            .expect("tail derive");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::Low);
        let c = ma.apply(&Signal::SubagentStop, 2000).expect("promote");
        assert_eq!(c.to, SessionState::Running);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn subagent_stop_corrects_pane_quiet_interrupt_misread() {
        // Waiting on background subagents stops the pane repainting; the
        // interrupt probe demotes. The next SubagentStop undoes the misread.
        let mut ma = m(SessionState::Running);
        assert!(ma.apply(&Signal::PaneQuiet, 1000).is_none());
        let c = ma.tick(3000).expect("demoted");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
        let c = ma.apply(&Signal::SubagentStop, 4000).expect("promote");
        assert_eq!(c.to, SessionState::Running);
        assert_eq!(c.confidence, Confidence::Medium);
    }

    #[test]
    fn subagent_stop_inert_from_high_confidence_states() {
        // A High Idle came from a real Stop (background_tasks reports
        // in-flight work), and Running needs no promotion.
        let mut idle = m(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert!(idle.apply(&Signal::SubagentStop, 1000).is_none());
        assert_eq!(idle.state(), &SessionState::Idle { stop_reason: StopReason::EndTurn });
        let mut run = m(SessionState::Running);
        assert!(run.apply(&Signal::SubagentStop, 1000).is_none());
        assert!(run.pending.is_none());
    }

    #[test]
    fn stopfailure_classes_route() {
        let mut m1 = m(SessionState::Running);
        let c = m1
            .apply(&Signal::StopFailure { class: StopFailureClass::AuthenticationFailed }, 1)
            .unwrap();
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
        assert!(m
            .apply(&Signal::Notification { kind: NotificationKind::QuotaFired }, 1000)
            .is_none());
        assert!(m.tick(4000).is_none());
        let c = m.tick(6100).expect("left throttled");
        assert_eq!(c.to, SessionState::Running);
    }

    #[test]
    fn quota_stale_needs_human() {
        let mut m = m(SessionState::Throttled);
        let c =
            m.apply(&Signal::Notification { kind: NotificationKind::QuotaStale }, 1000).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::QuotaResume });
        assert!(c.attention_added);
    }

    #[test]
    fn sandboxed_permission_prompt_is_medium_and_confirmation_is_silent() {
        let mut m = m(SessionState::Running);
        let c = m
            .apply(&Signal::Notification { kind: NotificationKind::PermissionPrompt }, 1000)
            .unwrap();
        assert_eq!(c.to, RA_PERM);
        assert_eq!(c.confidence, Confidence::Medium);
        // The 6 s-late confirmation raises nothing and never re-transitions.
        assert!(m
            .apply(&Signal::Notification { kind: NotificationKind::PermissionPrompt }, 2000)
            .is_none());
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
    fn accepted_permission_settles_back_to_running() {
        // The accept path: no "permission answered" event exists — the
        // tool's own completion is the clear (dogfood 2026-08-30: an
        // accepted tool stayed needs-you until end of turn).
        let mut m = m(SessionState::Running);
        m.apply(&Signal::PermissionRequest, 1_000).unwrap();
        assert!(m.apply(&Signal::ToolCompleted { nested: false }, 5_000).is_none()); // leave settles
        assert!(m.tick(5_000 + SETTLE_MS - 1).is_none());
        let c = m.tick(5_000 + SETTLE_MS).unwrap();
        assert_eq!(c.to, SessionState::Running);
        assert!(!c.attention_added);
    }

    #[test]
    fn tool_completed_is_inert_outside_a_held_permission() {
        // A sibling's completion must not clear a Question/Plan, and
        // mid-turn Running needs no re-assert. (A hook-stated end_turn sat
        // in this list until T-228: the lead's own completion after it IS
        // the next turn — `a_turn_resumed_without_a_prompt_shows_on_its_
        // first_tool_frame`; a nested one still says nothing there.)
        for state in [
            SessionState::Running,
            SessionState::RequiresAction { reason: Reason::Question },
            SessionState::RequiresAction { reason: Reason::Plan },
        ] {
            let mut m1 = m(state.clone());
            assert_eq!(m1.apply(&Signal::ToolCompleted { nested: false }, 1_000), None);
            assert!(m1.pending.is_none(), "no pending leave from {state:?}");
        }
        let mut done = m(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(done.apply(&Signal::ToolCompleted { nested: true }, 1_000), None);
        assert!(done.pending.is_none());
    }

    #[test]
    fn a_refused_plan_or_question_clears_on_the_sessions_next_tool_call() {
        // T-447 (2026-09-23): "No, keep planning" fires no hook of its own —
        // the model's next tool call is the first frame that says the dialog
        // is gone. It is a leave, so it settles like the answered pair does.
        for reason in [Reason::Plan, Reason::Question, Reason::Permission] {
            let mut m1 = m(SessionState::RequiresAction { reason });
            // A subagent's call is not the lead's answer.
            assert_eq!(m1.apply(&Signal::ToolStarted { nested: true }, 1_000), None);
            assert!(m1.pending.is_none(), "a nested call leaves {reason:?} held");
            assert!(m1.apply(&Signal::ToolStarted { nested: false }, 5_000).is_none());
            assert!(m1.tick(5_000 + SETTLE_MS - 1).is_none());
            let c = m1.tick(5_000 + SETTLE_MS).unwrap();
            assert_eq!(c.to, SessionState::Running);
            assert!(!c.attention_added);
        }
        // The accept road is unchanged and arrives first: the pair's
        // PostToolUse settles the leave, and the call that follows re-affirms.
        let mut m2 = m(SessionState::Running);
        m2.apply(&Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }, 1_000).unwrap();
        assert!(m2
            .apply(&Signal::PostToolUse { tool: AttentionTool::ExitPlanMode }, 5_000)
            .is_none());
        assert!(m2.apply(&Signal::ToolStarted { nested: false }, 5_100).is_none());
        assert_eq!(m2.tick(5_000 + SETTLE_MS).unwrap().to, SessionState::Running);
        // Mid-turn it is inert; from a rest it is the turn resuming, as its
        // completion is.
        let mut m3 = m(SessionState::Running);
        assert_eq!(m3.apply(&Signal::ToolStarted { nested: false }, 1_000), None);
        let mut m4 = m(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(
            m4.apply(&Signal::ToolStarted { nested: false }, 1_000).unwrap().to,
            SessionState::Running
        );
        let mut m5 = m(SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(m5.apply(&Signal::ToolStarted { nested: true }, 1_000), None);
    }

    #[test]
    fn status_file_idle_is_pane_quiet_sixty_seconds_early() {
        // The same row PaneQuiet owns: Running → Idle{Interrupted} at Medium
        // through the leave-settle, and nothing anywhere else.
        let mut mr = m(SessionState::Running);
        assert!(
            mr.apply(&Signal::StatusFileIdle { turn_done: false }, 10_000).is_none(),
            "leave settles"
        );
        let c = mr.tick(10_000 + SETTLE_MS).expect("demote");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
        assert_eq!(c.confidence, Confidence::Medium);
        for s in [
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Idle { stop_reason: StopReason::Background },
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::Spawning,
            SessionState::Sleeping,
        ] {
            let mut ma = m(s.clone());
            assert!(
                ma.apply(&Signal::StatusFileIdle { turn_done: false }, 1000).is_none(),
                "moved from {s:?}"
            );
        }
    }

    #[test]
    fn status_file_idle_over_a_finished_turn_is_end_turn() {
        // The transcript says the turn closed: the same row, the same
        // settle, the same Medium — and EndTurn, which automove promotes
        // (a late Stop then commits High over it, a lost one never needs to).
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&Signal::StatusFileIdle { turn_done: true }, 10_000).is_none());
        let c = mr.tick(10_000 + SETTLE_MS).expect("demote");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::Medium);
        for s in [
            SessionState::Idle { stop_reason: StopReason::Interrupted },
            SessionState::Idle { stop_reason: StopReason::Background },
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::Spawning,
        ] {
            let mut ma = m(s.clone());
            assert!(ma.apply(&Signal::StatusFileIdle { turn_done: true }, 1000).is_none(), "{s:?}");
        }
    }

    #[test]
    fn a_probe_never_overrides_a_pending_stated_leave() {
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        // Stop first, probe inside its settle: the stated EndTurn commits.
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&stop, 1000).is_none(), "leave settles");
        assert!(mr.apply(&Signal::StatusFileIdle { turn_done: false }, 1500).is_none());
        assert!(mr.apply(&Signal::PaneQuiet, 1600).is_none());
        let c = mr.tick(1000 + SETTLE_MS).expect("commit");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
        // Probe first, Stop inside ITS settle: the stated word replaces it.
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&Signal::StatusFileIdle { turn_done: false }, 1000).is_none());
        assert!(mr.apply(&stop, 1500).is_none());
        let c = mr.tick(1500 + SETTLE_MS).expect("commit");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
    }

    #[test]
    fn tool_completed_recovers_a_quiet_probe_misfire() {
        // Running → PaneQuiet misfire → Idle{Interrupted}; the next tool
        // completion is stated proof the turn is alive and flips it back
        // immediately (Idle→Running has no settle). The real-Esc straggler
        // costs a cosmetic Running the quiet probe re-demotes.
        let mut m = m(SessionState::Running);
        assert!(m.apply(&Signal::PaneQuiet, 10_000).is_none()); // leave settles
        let c = m.tick(10_000 + SETTLE_MS).expect("demote");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
        let c = m.apply(&Signal::ToolCompleted { nested: false }, 15_000).expect("recover");
        assert_eq!(c.to, SessionState::Running);
        assert_eq!(c.confidence, Confidence::High);
        assert!(!c.attention_added);
        // And the probe still owns the demotion if the pane goes quiet again.
        assert!(m.apply(&Signal::PaneQuiet, 25_000).is_none());
        assert!(m.tick(25_000 + SETTLE_MS).is_some(), "probe still demotes");
    }

    #[test]
    fn tool_completed_recovers_tail_misreads() {
        // The restart path: Unknown{DaemonRestarted}, then the tail misreads
        // a long quiet tool run as StaleQuiet → Idle{Unknown} at Low. Either
        // rung recovers on the next completion frame.
        let mut lost = m(SessionState::unknown());
        let c = lost
            .apply(&Signal::ToolCompleted { nested: false }, 1_000)
            .expect("recover from unknown");
        assert_eq!(c.to, SessionState::Running);

        let mut tailed = m(SessionState::unknown());
        tailed.apply(&Signal::TranscriptHint { kind: TailHint::StaleQuiet }, 1_000).unwrap();
        assert_eq!(tailed.confidence(), Confidence::Low);
        let c = tailed
            .apply(&Signal::ToolCompleted { nested: false }, 2_000)
            .expect("recover from tail idle");
        assert_eq!(c.to, SessionState::Running);
        // (A hook-stated end_turn used to stay inert here, against a
        // background task's completion frame that turned out not to exist;
        // `a_turn_resumed_without_a_prompt_shows_on_its_first_tool_frame`
        // holds the rule that replaced it.)
    }

    #[test]
    fn restore_carries_persisted_confidence() {
        // A daemon restart re-mints machines from the store; a tail-derived
        // Low idle must stay recoverable after the restart (dogfood
        // 2026-08-30: Machine::new laundered it to High and the session
        // stayed glyph-less across the reload meant to fix it).
        let mut restored = Machine::restore(
            SessionState::Idle { stop_reason: StopReason::Unknown },
            Confidence::Low,
            0,
        );
        assert_eq!(restored.confidence(), Confidence::Low);
        let c = restored
            .apply(&Signal::ToolCompleted { nested: false }, 1_000)
            .expect("recover after restore");
        assert_eq!(c.to, SessionState::Running);
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
            (EndKind::Logout, ExitReason::LoggedOut),
            (EndKind::PromptInputExit, ExitReason::UserQuit),
            (EndKind::Other, ExitReason::Crashed),
        ] {
            let mut ma = m(SessionState::Running);
            let c = ma.apply(&Signal::SessionEnd { kind }, 1).unwrap();
            assert_eq!(c.to, SessionState::Exited { reason });
        }
    }

    /// `/clear` is `/resume`'s twin and was the worse of the two: the pane
    /// lives on, so honoring the end as an exit left the record dead for the
    /// rest of the session — the SessionStart that follows arrives into a
    /// terminal state and the latch drops it.
    #[test]
    fn in_app_clear_is_not_an_exit() {
        for state in
            [SessionState::Running, SessionState::Idle { stop_reason: StopReason::EndTurn }]
        {
            let mut ma = m(state.clone());
            assert!(
                ma.apply(&Signal::SessionEnd { kind: EndKind::Clear }, 1).is_none(),
                "clear end must be inert from {state:?}"
            );
            assert_eq!(ma.state(), &state);
        }
        // ...and the whole round trip from where a user actually types it:
        // mid-turn, so the fresh conversation settles to idle rather than
        // snapping there, and the pane is still ours the whole way through.
        let mut ma = m(SessionState::Running);
        assert!(ma.apply(&Signal::SessionEnd { kind: EndKind::Clear }, 1).is_none());
        assert!(ma.apply(&Signal::SessionStart { source: StartSource::Clear }, 2).is_none());
        let c = ma.tick(2 + SETTLE_MS).expect("the cleared session is alive and idle");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Unknown });
        // A real death still relabels nothing: a late clear over a corpse is
        // not a reason to call that corpse "cleared".
        let mut dead = m(SessionState::Exited { reason: ExitReason::Crashed });
        assert!(dead.apply(&Signal::SessionEnd { kind: EndKind::Clear }, 3).is_none());
        assert_eq!(dead.state(), &SessionState::Exited { reason: ExitReason::Crashed });
    }

    #[test]
    fn in_app_resume_is_not_an_exit() {
        // SessionEnd{resume} = conversation handoff inside a live pane:
        // no transition from any live state, and no relabeling of a death.
        for state in
            [SessionState::Running, SessionState::Idle { stop_reason: StopReason::EndTurn }]
        {
            let mut ma = m(state.clone());
            assert!(
                ma.apply(&Signal::SessionEnd { kind: EndKind::Resume }, 1).is_none(),
                "resume end must be inert from {state:?}"
            );
            assert_eq!(ma.state(), &state);
        }
        let mut ma = m(SessionState::Exited { reason: ExitReason::Crashed });
        assert!(ma.apply(&Signal::SessionEnd { kind: EndKind::Resume }, 1).is_none());
        assert_eq!(ma.state(), &SessionState::Exited { reason: ExitReason::Crashed });
    }

    #[test]
    fn spawn_probe_paths() {
        // Bytes but no Claude title → startup modal, medium confidence.
        let mut m1 = m(SessionState::Spawning);
        let c =
            m1.apply(&Signal::SpawnProbe { bytes: true, osc0: false, resume: false }, 1).unwrap();
        assert_eq!(c.to, SessionState::RequiresAction { reason: Reason::StartupModal });
        assert_eq!(c.confidence, Confidence::Medium);
        // No bytes at all → unknown, never failed, low (out of the queue).
        let mut m2 = m(SessionState::Spawning);
        let c =
            m2.apply(&Signal::SpawnProbe { bytes: false, osc0: false, resume: false }, 1).unwrap();
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
        let c =
            m1.apply(&Signal::SpawnProbe { bytes: true, osc0: false, resume: true }, 1).unwrap();
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
            (TailHint::ToolInFlight, SessionState::Running),
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
    fn aborted_hint_demotes_running_after_settle() {
        // The Esc-interrupt catch: no hook fires, the transcript's interrupt
        // record (via poll_tails' abort-only class) must take a
        // High-confidence Running machine to Idle{Interrupted}, debounced
        // like any other leave.
        let mut ma = m(SessionState::Running);
        assert!(ma
            .apply(&Signal::TranscriptHint { kind: TailHint::AbortedMidStream }, 1000)
            .is_none());
        assert!(ma.tick(1000 + SETTLE_MS - 1).is_none());
        let c = ma.tick(1000 + SETTLE_MS).expect("settled demote");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::Interrupted });
        assert_eq!(c.confidence, Confidence::Low);
        assert!(!c.attention_added);
    }

    #[test]
    fn sleeping_latches_against_all_signals() {
        let mut ma = m(SessionState::Sleeping);
        // The daemon's own SIGTERM emits these — none may wake or exit the record.
        assert!(ma.apply(&Signal::SessionEnd { kind: EndKind::Other }, 1000).is_none());
        assert!(ma.apply(&Signal::PaneDied { status: Some(1) }, 1000).is_none());
        assert!(ma
            .apply(
                &Signal::Stop {
                    stop_hook_active: false,
                    has_agent_id: false,
                    blocking_tasks: false,
                    monitoring_tasks: false,
                    teammates: 0
                },
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

    /// T-363: a plan dialog left open for an hour is still a plan dialog.
    /// The recovery adapter restates the wait off the transcript tail while
    /// the dialog is open, and every restatement re-arms the clock; the
    /// demote fires fifteen minutes after the LAST affirmation, so a wait
    /// whose clearing event really was lost still never latches red.
    #[test]
    fn a_restated_wait_re_arms_the_stale_clock() {
        let mut m = m(SessionState::Running);
        m.apply(&Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }, 1000).unwrap();
        let affirmed = 1000 + 10 * 60 * 1000;
        assert!(
            m.apply(&Signal::TranscriptHint { kind: TailHint::ExitPlanMode }, affirmed).is_none(),
            "a restatement is not a transition"
        );
        assert_eq!(m.confidence(), Confidence::High, "a Low restatement never lowers a High state");
        assert!(m.tick(1000 + STALE_DEMOTE_MS).is_none(), "the clock runs from the affirmation");
        assert!(m.tick(affirmed + STALE_DEMOTE_MS - 1).is_none());
        let c = m.tick(affirmed + STALE_DEMOTE_MS).expect("unaffirmed for fifteen minutes");
        assert_eq!(c.to, SessionState::unknown());
        assert_eq!(c.confidence, Confidence::Stale);
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
    fn flap_guard_pins_the_inferred_signals() {
        let mut m = m(SessionState::Running);
        // 5 immediate transitions inside 20 s: alternate reasons (all enter at 0ms).
        m.apply(&Signal::PermissionRequest, 1000).unwrap();
        m.apply(&Signal::Elicitation, 2000).unwrap();
        let _ = m.apply(&Signal::PermissionRequest, 3000);
        let _ = m.apply(&Signal::Elicitation, 4000);
        let c = m.apply(&Signal::PreToolUse { tool: AttentionTool::AskUserQuestion }, 5000);
        // The commit that crossed the threshold armed the pin — at its own
        // confidence, since it was stated…
        assert!(c.is_some());
        assert_eq!(m.confidence(), Confidence::High);
        assert!(m.pinned_until.is_some());
        // …and while pinned, inference is ignored: the transcript tail, the
        // quiet probe, the status file. Nothing is pending for the tick.
        let stop = stop_with_teammates(0);
        // Leaving needs-you settles, pinned or not; the stated leave commits.
        assert!(m.apply(&Signal::UserPromptSubmit, 5500).is_none(), "leave settles");
        let c = m.tick(5500 + SETTLE_MS).expect("a stated signal passes the pin");
        assert_eq!(c.to, SessionState::Running);
        assert!(m.apply(&stop, 8000).is_none(), "leave settles");
        m.tick(8000 + SETTLE_MS).expect("settles to end of turn");
        let mut tailed = m.clone();
        assert!(tailed
            .apply(&Signal::TranscriptHint { kind: TailHint::AssistantText }, 10_000)
            .is_none());
        m.apply(&Signal::UserPromptSubmit, 10_000).expect("stated, and Idle → Running is instant");
        assert!(m.apply(&Signal::PaneQuiet, 11_000).is_none());
        assert!(m.apply(&Signal::StatusFileIdle { turn_done: false }, 11_000).is_none());
        assert!(m.tick(14_000).is_none());
        assert_eq!(*m.state(), SessionState::Running);
        // Terminal still passes.
        let c =
            m.apply(&Signal::PaneDied { status: Some(1) }, 15_000).expect("terminal passes pin");
        assert_eq!(c.to, SessionState::Exited { reason: ExitReason::Crashed });
    }

    /// Dogfood 2026-09-04: four Shift+Enter asks at one agent inside twenty
    /// seconds, each a two-second turn. The fifth commit armed the pin, the
    /// `Stop` that followed was DROPPED by it, the card kept the spinner on a
    /// finished turn, and when the pin lifted the status-file probe called
    /// the silence "interrupted". A `Stop` is stated: it commits through the
    /// pin at High, the probe then has no `Running` to demote, and the
    /// `Running` that armed the pin kept High too, so the move the prompt
    /// asked for is not refused.
    #[test]
    fn a_stated_stop_commits_through_the_flap_pin() {
        let stop = stop_with_teammates(0);
        let mut m = m(SessionState::Idle { stop_reason: StopReason::EndTurn });
        let mut t = 0;
        for _ in 0..2 {
            m.apply(&Signal::UserPromptSubmit, t).expect("running");
            assert!(m.apply(&stop, t + 3000).is_none(), "leave settles");
            m.tick(t + 3000 + SETTLE_MS).expect("end of turn");
            t += 8000;
        }
        // The fifth commit in the window.
        let c = m.apply(&Signal::UserPromptSubmit, t).expect("running");
        assert_eq!(c.confidence, Confidence::High, "a stated edge is not a flap");
        assert!(m.pinned_until.is_some(), "the guard still arms");
        // The Stop inside the pin.
        assert!(m.apply(&stop, t + 2000).is_none(), "leave settles");
        let c = m.tick(t + 2000 + SETTLE_MS).expect("the stated end of turn commits");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
        // The probes after the pin have nothing to demote.
        let later = t + FLAP_PIN_MS + 1000;
        assert!(m.apply(&Signal::StatusFileIdle { turn_done: false }, later).is_none());
        assert!(m.apply(&Signal::PaneQuiet, later).is_none());
        assert!(m.tick(later + SETTLE_MS).is_none());
        assert_eq!(*m.state(), SessionState::Idle { stop_reason: StopReason::EndTurn });
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
        let plan =
            mk(SessionState::RequiresAction { reason: Reason::Plan }, Some(10), Confidence::High);
        let low = mk(RA_PERM, Some(1), Confidence::Low);
        let running = mk(SessionState::Running, None, Confidence::High);
        board.sessions = vec![running, plan.clone(), low, perm_late.clone(), perm_early.clone()];
        let q = attention_queue(&board);
        let ids: Vec<_> = q.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![perm_early.id, perm_late.id, plan.id]);
    }
}
