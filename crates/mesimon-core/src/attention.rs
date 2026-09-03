//! The attention state machine (11 §11.7) — pure, time-injected, table-tested.
//! Precedence ranks are fixed forever (D28); ranks 0–8 are the attention set:
//! exactly these produce the `needs you` count and light a card. The daemon
//! owns one `Machine` per session and applies `Signal`s from the hook stream;
//! debounce rules are 11 §11.7.4.

use std::collections::BTreeSet;

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
    SessionStart {
        source: StartSource,
    },
    SessionEnd {
        kind: EndKind,
    },
    UserPromptSubmit,
    /// `blocking_tasks`: the Stop payload's `background_tasks[]` held an entry
    /// whose `.type` means the turn is PAUSED, not DONE (`task_blocks_end_turn`
    /// — emptiness alone is NOT the test). `teammates` is the number of
    /// `teammate` entries, which are neither: an in-process teammate reads
    /// `running` for as long as it exists, idle or not, so whether one holds
    /// the turn open is decided against the `TeammateIdle` frames the machine
    /// has seen (dogfood 2026-09-01, T-135: four idle reviewers parked a
    /// finished session for good).
    Stop {
        stop_hook_active: bool,
        has_agent_id: bool,
        blocking_tasks: bool,
        teammates: usize,
    },
    SubagentStop,
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
    /// remains the fallback for a record that never lands.
    PaneQuiet,
    /// Daemon-side probe while `Running`: Claude Code's own
    /// `~/.claude/sessions/<pid>.json` reads `status: idle`, stamped after
    /// this Running spell began. The recordless Esc — pressed before the
    /// first assistant output, the prompt handed back to the box — writes
    /// NOTHING to the transcript (spike S-E's case, seen live 2026-09-04),
    /// so PaneQuiet a minute later was its whole catch; the status file
    /// flips at the keypress. Same row as PaneQuiet, same confidence.
    StatusFileIdle,
    /// Observe tier: derived from an adopted session's transcript tail.
    TranscriptHint {
        kind: TailHint,
    },
}

/// Does one `background_tasks[]` entry mean the turn is PAUSED rather than
/// DONE? 11 §11.7.4 gates `Idle{EndTurn}` on the array being *empty*; dogfood
/// 2026-08-31 showed that is too coarse, and that it fails closed forever.
///
/// A `monitor` — the artifact-comment subscription an `Artifact` publish arms —
/// is not work the agent is doing. It is a dormant watch on an EXTERNAL human,
/// and it stays armed for the rest of the session. Under the emptiness test the
/// first publish therefore suppressed every later `Stop` in that session: the
/// arm targets `Running`, the state it is already in, so `apply` returns `None`
/// and nothing is recorded; the pane-quiet probe then demoted the finished turn
/// to `Idle{Interrupted}` 8 s later; and `automove` refuses to promote an
/// interrupt. The ticket never reached REVIEW (T-72 "shortcuts UX": activity
/// seq 114 `Stop`, no transition, seq 115 interrupted at Medium).
///
/// So classify, and let only genuinely in-flight work hold the turn open.
///
/// **The `.type` spellings are corpus claims, not spike-verified** — 11 §11.2.3
/// lists `shell|subagent|monitor|workflow|teammate|cloud session|MCP task`, but
/// S-A never captured a live `Stop` payload, and doc rule 4 says re-verify every
/// API claim at implementation time. Two guards against that: matching is on a
/// normalised token, so `"MCP task"`, `"mcp_task"` and `"mcpTask"` agree; and an
/// UNRECOGNISED type blocks, which keeps today's conservative behaviour for
/// anything new rather than ending a turn that is still running.
///
/// **A `teammate` is neither, and is answered elsewhere** (dogfood 2026-09-01,
/// T-135): an in-process teammate stays alive after it reports and is listed
/// as `running` in every later Stop payload for the rest of the session —
/// captured on the wire, idle teammate still `{type: "teammate", status:
/// "running"}` after the lead's final turn. Classing it blocking parked a
/// finished session for good; classing it dormant would end a turn whose
/// reviewers are still working. So it is `false` here and COUNTED by the
/// caller (`Signal::Stop::teammates`), and the machine weighs the count
/// against the `TeammateIdle` frames it has seen (`is_teammate_task`).
pub fn task_blocks_end_turn(kind: &str) -> bool {
    let norm = norm_task_kind(kind);
    // Anything bearing "monitor" is a watch by construction, whatever it ends
    // up being called (`monitor`, `artifact-comment-monitor`, …).
    !norm.contains("monitor") && !is_teammate_task(kind)
}

/// A `background_tasks[]` entry that is an in-process teammate (Claude Code
/// labels the `in_process_teammate` task `"teammate"`; the raw discriminant
/// is accepted too, the same way `task_blocks_end_turn` normalises).
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
    /// Named teammates that have reported idle and not been messaged since.
    /// A Stop payload lists a teammate as `running` for its whole life, so
    /// this set is the only way to tell "reviewers still working" from
    /// "reviewers done, lead done" — the daemon persists it on the record so
    /// a restart does not re-park a finished session.
    idle_teammates: BTreeSet<String>,
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
            pending: None,
            committed: Vec::new(),
            pinned_until: None,
            recent_left: None,
            idle_teammates: BTreeSet::new(),
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
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

        let (to, conf) = self.target(sig)?;

        // Flap pin: only terminal transitions get through while pinned.
        if let Some(until) = self.pinned_until {
            if now < until
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
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            return self.flush(now);
        }
        // Stale demotion: never latch red (11 §11.7.4).
        if is_attention(&self.state) && now.saturating_sub(self.entered_at) >= STALE_DEMOTE_MS {
            let to = SessionState::Unknown { reason: UnknownReason::NoSignal };
            return Some(self.commit(to, Confidence::Stale, now));
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
            // A session that just started sits at the prompt — that is idle,
            // not working (dogfood 2026-08-30: fresh spawns read "working"
            // forever). The one exception: a compact-restart fires
            // SessionStart mid-turn and the turn continues.
            Signal::SessionStart { source: StartSource::Compact } => t(S::Running),
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
            Signal::Stop { stop_hook_active: true, .. } => None, // re-entrancy guard
            Signal::Stop { has_agent_id: true, .. } => None,     // nested, never top-level
            // In-flight work (shell, subagent, …) holds the turn open; a
            // dormant watch does not — see `task_blocks_end_turn`. The turn is
            // PAUSED, so this is `Idle{Background}` and NOT a re-assertion of
            // `Running`: the pane stops painting the moment the agent parks, so
            // `Running` here is a claim the quiet probe refutes ~8 s later by
            // demoting to `Idle{Interrupted}` — a second, worse lie, and one
            // no signal corrects (compare `SubagentStop`, which at least has a
            // corrective). `Idle` is invisible to `probe_activity`, which only
            // scans `Running`, so the misread stops being possible rather than
            // being cleaned up afterwards.
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
            Signal::Stop { .. } => t(S::Idle { stop_reason: StopReason::EndTurn }),
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
                // A parked turn resumes when its OWN tool runs. The wake is a
                // prompt only when a task notification delivers it; a
                // teammate's report arrives as a teammate message and fires
                // no `UserPromptSubmit` at all (measured 2026-09-01, T-135:
                // twenty minutes of the lead's tool frames streamed past a
                // High `Idle{Background}` that only a prompt could leave).
                // The frame is a stated event, so High. A NESTED completion
                // is the teammate's work, not the lead's — the lead may well
                // still be parked — and says nothing here.
                S::Idle { stop_reason: StopReason::Background } if !nested => t(S::Running),
                // A tool completing is stated proof the turn is alive — it
                // outranks any INFERRED resting state (quiet-probe Medium,
                // tail-hint Low) and the post-restart Unknown, and is the
                // recovery probe_activity's "next real event corrects"
                // promise relies on (dogfood 2026-08-30: a quiet-probe
                // misfire, then a restart-tail StaleQuiet misread, each left
                // a working session glyph-less on "idle" while PostToolUse
                // frames streamed in). A hook-stated Idle stays inert — a
                // background task's completion must not flip a real
                // end_turn — and a straggler frame after a real Esc costs a
                // cosmetic "working" the quiet probe re-demotes.
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
            Signal::PaneQuiet | Signal::StatusFileIdle => {
                if self.state == S::Running && self.pending.is_none() {
                    Some((S::Idle { stop_reason: StopReason::Interrupted }, Confidence::Medium))
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
                    TailHint::AbortedMidStream => S::Idle { stop_reason: StopReason::Interrupted },
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
    fn stop_hook_active_and_agent_id_are_ignored() {
        let mut m = m(SessionState::Running);
        assert!(m
            .apply(
                &Signal::Stop {
                    stop_hook_active: true,
                    has_agent_id: false,
                    blocking_tasks: false,
                    teammates: 0
                },
                1000
            )
            .is_none());
        assert!(m
            .apply(
                &Signal::Stop {
                    stop_hook_active: false,
                    has_agent_id: true,
                    blocking_tasks: false,
                    teammates: 0
                },
                1000
            )
            .is_none());
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
        // The other hook-stated Idle stays inert to a completion, as before:
        // nothing is owed after a real end_turn.
        let mut done = Machine::new(SessionState::Idle { stop_reason: StopReason::EndTurn }, 0);
        assert!(done.apply(&Signal::ToolCompleted { nested: false }, 1000).is_none());
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

    /// In-flight work holds the turn open; a dormant watch must not. The
    /// spellings are unverified, so an unknown type keeps the safe behaviour.
    #[test]
    fn only_in_flight_task_types_block_end_turn() {
        for k in ["shell", "subagent", "workflow", "MCP task", "cloud session"] {
            assert!(task_blocks_end_turn(k), "{k} is work in flight");
        }
        for k in ["monitor", "artifact-comment-monitor", "Monitor", "artifact_monitor"] {
            assert!(!task_blocks_end_turn(k), "{k} is a dormant watch");
        }
        // A teammate is neither: counted by the caller, weighed by the machine
        // against the idle notices it has seen (T-135).
        for k in ["teammate", "in_process_teammate", "Teammate"] {
            assert!(!task_blocks_end_turn(k), "{k} is counted, not classed");
            assert!(is_teammate_task(k));
        }
        assert!(!is_teammate_task("shell"));
        // Forward-safe: an unseen type holds the turn open rather than ending
        // one that may still be running.
        assert!(task_blocks_end_turn("some_future_task"));
        assert!(task_blocks_end_turn(""));
    }

    /// The T-72 regression. An `Artifact` publish arms a comment monitor that
    /// stays live for the rest of the session, so under the old emptiness test
    /// EVERY later Stop was swallowed (`to == state`, no transition) and the
    /// pane-quiet probe mislabelled the finished turn `Interrupted` — which
    /// `automove` refuses to promote, stranding the ticket in IN PROGRESS.
    #[test]
    fn armed_monitor_does_not_suppress_end_turn() {
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            teammates: 0,
        };
        let mut m = m(SessionState::Running);
        // Two turns: the bug was that the monitor stayed armed and so ate the
        // SECOND one too.
        for turn in 0..2 {
            let base = turn * 100_000;
            assert!(m.apply(&stop, base + 1000).is_none(), "leave settles");
            let c = m.tick(base + 1000 + SETTLE_MS).expect("settles to end_turn");
            assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
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
        // mid-turn Running needs no re-assert.
        for state in [
            SessionState::Running,
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::RequiresAction { reason: Reason::Question },
            SessionState::RequiresAction { reason: Reason::Plan },
        ] {
            let mut m1 = m(state.clone());
            assert_eq!(m1.apply(&Signal::ToolCompleted { nested: false }, 1_000), None);
            assert!(m1.pending.is_none(), "no pending leave from {state:?}");
        }
    }

    #[test]
    fn status_file_idle_is_pane_quiet_sixty_seconds_early() {
        // The same row PaneQuiet owns: Running → Idle{Interrupted} at Medium
        // through the leave-settle, and nothing anywhere else.
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&Signal::StatusFileIdle, 10_000).is_none(), "leave settles");
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
            assert!(ma.apply(&Signal::StatusFileIdle, 1000).is_none(), "moved from {s:?}");
        }
    }

    #[test]
    fn a_probe_never_overrides_a_pending_stated_leave() {
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            teammates: 0,
        };
        // Stop first, probe inside its settle: the stated EndTurn commits.
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&stop, 1000).is_none(), "leave settles");
        assert!(mr.apply(&Signal::StatusFileIdle, 1500).is_none());
        assert!(mr.apply(&Signal::PaneQuiet, 1600).is_none());
        let c = mr.tick(1000 + SETTLE_MS).expect("commit");
        assert_eq!(c.to, SessionState::Idle { stop_reason: StopReason::EndTurn });
        assert_eq!(c.confidence, Confidence::High);
        // Probe first, Stop inside ITS settle: the stated word replaces it.
        let mut mr = m(SessionState::Running);
        assert!(mr.apply(&Signal::StatusFileIdle, 1000).is_none());
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
    fn tool_completed_recovers_tail_misreads_but_not_stated_idle() {
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

        // A hook-stated end_turn (High) stays inert — a background task's
        // completion must not flip a real turn end.
        let mut done = m(SessionState::Running);
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            teammates: 0,
        };
        assert!(done.apply(&stop, 1_000).is_none()); // leave settles
        done.tick(1_000 + SETTLE_MS).expect("settle to end_turn");
        assert_eq!(done.confidence(), Confidence::High);
        assert!(done.apply(&Signal::ToolCompleted { nested: false }, 10_000).is_none());
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
