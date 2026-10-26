use serde::{Deserialize, Serialize};

/// The project default for new agent sessions. A session's `kind` retains
/// its provider when this setting changes; shells are not agent providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    #[default]
    ClaudeCode,
    Codex,
}

impl AgentProvider {
    pub fn session_kind(self) -> SessionKind {
        match self {
            Self::ClaudeCode => SessionKind::Claude,
            Self::Codex => SessionKind::Codex,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::ClaudeCode => Self::Codex,
            Self::Codex => Self::ClaudeCode,
        }
    }

    /// The CLI has a send-now key (T-601, T-685): Claude Code's Ctrl+X
    /// Ctrl+S, which puts the composer's words in front of a WORKING agent
    /// mid-turn. Codex has none, so `immediately` there is a plain paste at
    /// best and is refused instead.
    pub fn has_send_now(self) -> bool {
        match self {
            Self::ClaudeCode => true,
            Self::Codex => false,
        }
    }
}

/// Persisted session type and original provider. Notes are files, not sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Claude,
    Codex,
    Bash,
}

impl SessionKind {
    pub fn is_agent(self) -> bool {
        self.provider().is_some()
    }

    pub fn provider(self) -> Option<AgentProvider> {
        match self {
            Self::Claude => Some(AgentProvider::ClaudeCode),
            Self::Codex => Some(AgentProvider::Codex),
            Self::Bash => None,
        }
    }

    /// The kind's word, its serde spelling: a session row's name when the
    /// session set none.
    pub fn word(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Bash => "bash",
        }
    }
}

/// The full D15 state enum; the vocabulary is owned by 11 §11.7.1.
/// `Sleeping` has no producer until the sleep/reclaim milestone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SessionState {
    Spawning,
    Running,
    RequiresAction {
        reason: Reason,
    },
    Idle {
        stop_reason: StopReason,
    },
    Sleeping,
    Exited {
        reason: ExitReason,
    },
    Failed {
        reason: FailReason,
    },
    Throttled,
    Unknown {
        #[serde(default)]
        reason: UnknownReason,
    },
}

impl SessionState {
    /// The record belongs to the ticket's working set — a `Sleeping` session is
    /// live-but-parked (it can be woken), only `Exited` is out.
    pub fn is_live(&self) -> bool {
        !matches!(self, SessionState::Exited { .. })
    }

    /// A pane exists (or should) for this record. `Sleeping` records have no
    /// process and no pane; every pane operation and pane-derived count keys
    /// off this, not `is_live`.
    pub fn has_pane(&self) -> bool {
        !matches!(self, SessionState::Exited { .. } | SessionState::Sleeping)
    }

    pub fn unknown() -> Self {
        SessionState::Unknown { reason: UnknownReason::NoSignal }
    }

    /// Idle at its composer on tasks it left running, whichever park reads
    /// it (T-944): `Background`, or `Monitoring` — where a background
    /// command's park lands once `attention::PARK_STALE_MS` passes with no
    /// frame, since the command sends none while it runs (T-483). What the
    /// card's task count, `get_ticket`'s `background` and the crown's
    /// lingering wake read; never the train's, the queue's or the
    /// attention set's (`quiet::is_working` keeps `Background` alone).
    pub fn on_tasks(&self) -> bool {
        matches!(
            self,
            SessionState::Idle { stop_reason: StopReason::Background | StopReason::Monitoring }
        )
    }

    /// The turn ended on a limit (T-808): a usage limit's QUOTA (T-802), or
    /// `Throttled` — an overload, and every limit a build before T-802 kept.
    /// The agent is back at its composer and nothing runs, so it is no turn
    /// in progress: it sleeps, takes a tier switch and holds no checkout or
    /// train. It is still no idle for WORDS, which would only meet the limit
    /// again (`quiet::holds_against_words`), and no turn done (`automove`).
    /// The one place that says which states these are; `quiet` reads it.
    pub fn stopped_on_limit(&self) -> bool {
        matches!(
            self,
            SessionState::RequiresAction { reason: Reason::QuotaResume } | SessionState::Throttled
        )
    }

    /// The agent stopped on a QUESTION — one whose answer may change what a
    /// queued follow-up should say, so the queue holds every ask behind it
    /// for a person's send (T-420, T-565). A permission prompt is not one:
    /// allowing a tool changes nothing about the follow-up. Nor is a plan
    /// dialog, which holds only an unflagged ask, the way it always did.
    pub fn question_stop(&self) -> bool {
        matches!(
            self,
            SessionState::RequiresAction {
                reason: Reason::Question | Reason::Secret | Reason::Elicitation
            }
        )
    }

    /// A prompt has reached this session at least once: it is working, has
    /// worked, or is parked after working. `Spawning`, an `Idle{Unknown}`
    /// fresh off its `SessionStart`, `Unknown` and the dead states say
    /// nothing either way, and the `description unread` clause on the ticket
    /// page (T-224) reads this so it never accuses a session that has not
    /// had its first turn yet.
    pub fn has_prompted(&self) -> bool {
        matches!(
            self,
            SessionState::Running
                | SessionState::RequiresAction { .. }
                | SessionState::Sleeping
                | SessionState::Throttled
                | SessionState::Idle {
                    stop_reason: StopReason::EndTurn
                        | StopReason::Interrupted
                        | StopReason::Background
                        | StopReason::Monitoring
                }
        )
    }
}

/// Why a session needs a human. Rank order lives in `attention::rank`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Permission,
    Secret,
    Question,
    Plan,
    Elicitation,
    Auth,
    QuotaResume,
    Trust,
    StartupModal,
    /// The resume-from-summary blocking dialog (09 §9): the session was
    /// resumed but is not yet accepting input. Shares rank 8 with
    /// `StartupModal` — an amendment to 11 §11.7.1, recorded in STALE-MAP.
    ResumeDialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    Interrupted,
    /// The lead is parked while live agent tasks work. Quiet-pane probes must
    /// not interrupt it; it counts as working for cards and automation.
    Background,
    /// Only watches/background shells remain. No active agent work is held.
    Monitoring,
    Unknown,
}

/// The reason, never the code (11 §11.7.1). `Killed` and `Dismissed` are
/// deliberate non-normative extras: `Killed` means mesimon's own kill ladder
/// ended a live session (the conversation survives — its corpse stays
/// resumable), `Dismissed` means the user x-ed an already-dead corpse off
/// the ticket rail (the only exit the rail hides).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    UserQuit,
    Cleared,
    Resumed,
    LoggedOut,
    Crashed,
    Killed,
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    Server,
    InvalidRequest,
    ModelNotFound,
    MaxOutputTokens,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    SupervisorDead,
    AdapterGone,
    DaemonRestarted,
    ObservationLost,
    #[default]
    NoSignal,
}

/// How trustworthy the source of the current state is (11 §11.5.4).
/// `Low`/`Stale` never get the saturated colour and never enter the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    #[default]
    High,
    Medium,
    Low,
    Stale,
}

/// Where a session record came from. Not named `origin` — that is D32c's word
/// for ticket/text provenance and carries trust semantics this does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// mesimon minted the UUID and spawned the process (`--session-id`).
    #[default]
    Spawned,
    /// A foreign session attached from the External drawer (19 §4). Badge
    /// word is "external"; hooks exist only after takeover-by-resume.
    Adopted,
}

/// One short-lived human confirmation, bound to an exact shell pane and crown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellCloseRequest {
    pub id: uuid::Uuid,
    pub crown: ulid::Ulid,
    pub pane: String,
    pub expires_ms: u64,
}

/// A session record the daemon persists. Mesimon's UUID names its tmux
/// session and authorization principal. Provider conversation IDs are
/// separate: Claude initially accepts this UUID; Codex assigns its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: uuid::Uuid,
    pub kind: SessionKind,
    pub ticket: ulid::Ulid,
    /// Full argv, persisted and replayed on resume (D24) — with the identity
    /// flag (`--session-id`/`--resume`) rewritten to the current conversation.
    pub argv: Vec<String>,
    pub cwd: String,
    pub state: SessionState,
    /// Epoch ms when the session entered the attention set; None otherwise.
    /// Minted by the daemon only — orders the Tab queue.
    #[serde(default)]
    pub waiting_since: Option<u64>,
    #[serde(default)]
    pub state_changed_at: Option<u64>,
    /// From `SessionStart` — the only authoritative source (D24).
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// Short excerpt for the card (e.g. the rendered API error string). ≤200 chars.
    #[serde(default)]
    pub detail: Option<String>,
    /// The session's self-declared name (OSC-0 pane title — Claude keeps its
    /// conversation summary there). None until the agent sets one; latched so
    /// a parked/paneless session keeps the last name it had.
    #[serde(default)]
    pub title: Option<String>,
    /// The command running in a shell's pane (T-366): `#{pane_current_command}`
    /// when it is not the shell itself — `Some("cargo")` mid-build, `None` at
    /// the prompt. A shell has no hook stream, so this is the one fact that
    /// says whether its `Running` is work; `glyphs::is_working` reads it. The
    /// daemon fills it into the SNAPSHOT from a map it keeps in memory and
    /// never writes it to `sessions.json`: a foreground is a fact about a
    /// live pane, and a restart re-learns it on the first poll.
    ///
    /// On an idle Claude record it is the `!` command the person is running
    /// in Claude's own composer (T-707): bash mode fires no hook and writes
    /// the transcript only when the command ends, so the daemon reads it off
    /// the pane's child processes (`bash_mode_foreground`). The card spins
    /// and the rail row names the command; the state stays `idle`, so no
    /// automove sees a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    /// A person explicitly shared this shell's output with this ticket's agent and crown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shell_shared: bool,
    /// A crown's request, never authority to close. Only a local confirmation consumes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_close_request: Option<ShellCloseRequest>,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default)]
    pub provenance: Provenance,
    /// The crown's ticket when the crown's agent asked for this session
    /// (`start_agent`, T-412); `None` for every seat a person started. The
    /// spawn budget counts records that carry it while they hold a seat
    /// awake on a ticket the board shows (`Board::crown_started`), and a
    /// ticket whose agent carries it can never be crowned — so the graph of
    /// agents that start agents is one level deep by construction (D10).
    /// Persisted so a
    /// restart keeps the count honest; `#[serde(default)]` is the migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_by: Option<ulid::Ulid>,
    /// The claude-side session id when it differs from `id` — set for adopted
    /// sessions, and relearned from the SessionStart transcript filename when
    /// an in-app /resume hands the pane a different conversation. Stays None
    /// while the pane hosts the record's own minted conversation
    /// (mesimon-spawned ones pass `--session-id id`, so the two coincide).
    #[serde(default)]
    pub claude_session_id: Option<uuid::Uuid>,
    /// The tmux pane this record's process runs in, as `<server pid>:<pane
    /// id>` (the backend's `PANE_KEY`), set on every pane road (spawn,
    /// resume, wake, adopt). A wake reuses the session name; the pane key is
    /// what tells the old pane's death frames from the new pane's (T-245).
    /// `None` on a record from an older build or with no pane, and a death
    /// frame is then trusted as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_key: Option<String>,
    /// When the current pane was launched, epoch ms (T-961): written with
    /// `pane_key` at every launch and kept in `sessions.json`, so a restart
    /// reads a young pane as young. `None` on a record from an older build or
    /// an adopted pane, which is never young; nothing else stamps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_at: Option<u64>,
    /// The title typed into the current pane before its box drew (T-961): a
    /// start's prefill or a fresh wake's retype, exactly as typed; `None`
    /// when the launch typed nothing (a resume). Renewed with `launched_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_title: Option<String>,
    /// The road this record's process reports on (T-574): `mod` when its
    /// launch was handed the mod mesimon lays. Re-decided at every launch and
    /// wake, like the tier; absent means the hook set alone. Since T-577 a
    /// mod launch passes no hook set (`frames_by_mod`).
    #[serde(default, skip_serializing_if = "crate::road::Road::is_hooks")]
    pub road: crate::road::Road,
    /// The mod launch reports from Claude Code's native events
    /// (`MESIMON_MOD_NATIVE=1`, T-658): where Claude Code keeps the classic
    /// hook events from a person's plugins (a Team or Enterprise account),
    /// the mod relays from its own events and a one-entry hook set rides
    /// beside it for permissions alone. Stamped with the road at every
    /// launch and wake.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native: bool,
    /// Codex's exact resumable thread, independent of Mesimon's record UUID.
    /// Opaque: a provider may change its identifier format without changing
    /// board identity. Never replace this with a thread-tree session ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_thread_id: Option<String>,
    /// Identity of the owned Codex runtime process. A launch chooses a fresh
    /// generation, so an old supervisor cannot overwrite its replacement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_generation: Option<u64>,
    /// Last consumed snapshot within that generation. Handover must not
    /// replay an already-applied successful turn as a new automatic move.
    #[serde(default)]
    pub codex_observed_seq: u64,
    /// Observed evidence awaiting the common state machine settle.
    #[serde(default)]
    pub codex_pending_seq: Option<u64>,
    /// Last externally observed turn. Resuming a historical completed turn
    /// cannot acknowledge a newly queued prompt merely by showing its ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_turn_id: Option<String>,
    /// The last turn this seat reported failed (T-936): read again, after a
    /// restart too, it is no new failure for the crown to hear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_failed_turn: Option<String>,
    /// Mesimon's bounded normalized preview artifact, separate from native
    /// provider history in `transcript_path` and never parsed as that format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_preview_path: Option<String>,
    /// Last authoritative plan recorded as a ticket note.
    #[serde(default)]
    pub agent_plan_key: Option<String>,
    /// Codex observation has not yet proved the checkout quiet. Persisted
    /// independently of the display state, so stale completion after an
    /// observer gap cannot authorize queue delivery or an automatic merge.
    /// Missing means held; Claude retains its existing observation semantics.
    #[serde(default = "yes")]
    pub observation_hold: bool,
    /// The ticket title was typed into this session's box and is still
    /// waiting for its Enter. Spike T-5 arm C (2026-08-31): an Enter sent in
    /// the same breath as the text is swallowed by Claude's paste detection,
    /// so the submit is deferred to the `SessionStart` frame — the earliest
    /// point the pane provably accepts a keystroke as a keystroke. Cleared
    /// the moment it is delivered; a session that never starts just keeps the
    /// prefill, which is the ordinary spawn's behaviour anyway.
    #[serde(default)]
    pub pending_submit: bool,
    /// Fresh Codex input has not yet been proven ready for the ticket title.
    /// Persist separately from the owed Enter so startup modals and handover
    /// cannot cause Mesimon to type into a dialog or duplicate the title.
    #[serde(default)]
    pub pending_prefill: bool,
    /// The native Codex input already received its one Enter. The pending
    /// submission still holds the checkout until a new turn acknowledges it;
    /// daemon handover must not press Enter again and steer or queue a turn.
    #[serde(default)]
    pub codex_submit_sent: bool,
    /// Cleanup is still pending; even a sleeping record retains its checkout hold.
    #[serde(default)]
    pub codex_stopping: bool,
    #[serde(default)]
    pub codex_plan_dialog_seen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_plan_dismissed_turn: Option<String>,
    /// A Codex launch in plan mode (T-847) owes `/plan` in the composer
    /// before its words: Codex has no launch flag for the mode, so the
    /// title, the brief and the ask wait until the thread is seen planning.
    /// Persisted, so a daemon handover types it again (`/plan` is idempotent)
    /// rather than sending the words into the default mode.
    #[serde(default, skip_serializing_if = "is_false")]
    pub codex_plan_owed: bool,
    /// Named in-process teammates that reported idle and have not been
    /// messaged since (T-135). A Stop payload lists a teammate as `running`
    /// for its whole life, so this is what lets the attention machine tell a
    /// lead whose reviewers are still working from one whose reviewers are
    /// done; persisted because a daemon restart that forgot it would park the
    /// next finished turn for good. Sorted, deduplicated.
    #[serde(default)]
    pub idle_teammates: Vec<String>,
    /// Ephemeral liveness evidence: restart deliberately starts empty.
    #[serde(skip)]
    pub background_tasks: crate::background::Registry,
    /// The note this session's approved plan lives in (2026-09-03). Every
    /// `ExitPlanMode` the user approves writes its plan here, replacing the
    /// last, so a session has ONE plan note the way it has one plan file
    /// under `~/.claude/plans/` — a re-plan is a new revision, never a second
    /// note. Persisted so a daemon restart cannot turn the next re-plan into
    /// a second note; `None` until the first approval, and a note the user
    /// deleted since is not resurrected under its old id — the next approval
    /// mints a fresh one.
    #[serde(default)]
    pub plan_note: Option<ulid::Ulid>,
    /// The `tool_use` id of the plan or question dialog the hook stream
    /// projected for this seat (T-957 round 9): written at each projection,
    /// a replan on a record already held included, and cleared when the
    /// record leaves the dialog. Persisted, so a restart, which loses the
    /// projection, still knows which call the record is held on: a result
    /// the tail reads for another call never ends it, and a result for this
    /// one is read by identity, not against the daemon's clock. `None` from
    /// an older build, or for a dialog no hook projected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog_call: Option<String>,
    /// The interaction the last plan or question hook carried, beside its
    /// call (T-957 round 12): the tool and a hash of its whole input,
    /// bounded, whether or not the dialog could be drawn. An id-less
    /// `PermissionRequest` with the same print repeats that dialog and
    /// keeps its call; any other is a replacement. Set and let go with
    /// `dialog_call`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog_print: Option<String>,
    /// This session has read its ticket (T-224, 2026-09-05): `get_ticket`
    /// answered it, or the composed spawn pasted the description under the
    /// title as its first prompt. The ticket page reads it the other way
    /// round — a claude that has taken a turn on a ticket WITH a description
    /// and never read it earns a `description unread` clause — so the skip
    /// the user could not see before is on the page it matters on. Persisted
    /// so a restart does not accuse a session that did read it; a pre-field
    /// record reads false, which is the honest answer for a session nobody
    /// watched.
    #[serde(default)]
    pub ticket_read: bool,
    /// The tier this seat was last launched on (T-443): a tier id, stamped
    /// by every launch road — spawn, resume, wake. Empty on a record from
    /// before tiers, which reads as "unknown", never as a switch owed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tier: String,
    /// A person picked a different tier for this seat's ticket while it had
    /// a pane: at its next idle the daemon parks it and wakes it on the new
    /// one (`drain_tier_switches`). Only an explicit per-ticket pick sets
    /// it — a changed default or an edited tier applies at the next launch,
    /// the way column settings do. Cleared by every launch. Persisted, so a
    /// restart still owes the switch; a build that drops it just forgets a
    /// pending switch, so no schema bump.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tier_owed: bool,
    /// A Codex seat parked FOR a tier switch, waiting for its runtime to
    /// confirm it stopped before it may be woken on the new tier (a Codex
    /// wake cannot acknowledge unverified cleanup). Cleared by every launch.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tier_wake: bool,
    /// Words a launch road could not deliver (T-570): the composer never
    /// painted inside the wait, or the Enters ran out before the agent took
    /// them, or a startup modal stopped the pressing. The card wears the
    /// needs-you mark for it, the crown reads `unsent`, and the seat's
    /// Shift+Enter resends (`PromptSession.resend`). Cleared by the
    /// agent's next prompt, whoever typed it. Persisted, so a restart still
    /// says the brief never went; an older build drops it and goes back to
    /// saying nothing, which widens nothing, so no schema bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsent: Option<Unsent>,
    /// How many background tasks an agent idle at its composer has running
    /// (`SessionState::on_tasks`, T-599, T-944): the card says `idle ∙ 3
    /// tasks ∙ 2h`. The daemon fills it into the SNAPSHOT only, as it does
    /// `foreground`: the task registry is never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks_running: Option<u32>,
    /// When that stretch idle on tasks began (T-944): the turn's end, kept
    /// when the park moves between `Background` and `Monitoring`, which
    /// restamps `state_changed_at`. The card's age reads it. SNAPSHOT only,
    /// as `tasks_running` is; the daemon keeps it in memory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks_since: Option<u64>,
    /// A Claude seat whose conversation has never taken a prompt (T-603):
    /// a plain start (`c`, whose title is typed and never submitted), or a
    /// wake of one, which comes up on an empty composer. Claude Code writes
    /// no transcript before the first prompt, so it is the transcript's
    /// absence, read when the snapshot is built. There a blank ask sends
    /// the ticket's brief, as on an empty seat. SNAPSHOT only, as
    /// `tasks_running` is.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unprompted: bool,
    /// A compaction is in flight in this seat (T-894): Claude Code is
    /// summarizing the conversation, between its `PreCompact` and its
    /// `PostCompact`. The state is whatever it was — a compaction moves
    /// nothing — and the card paints the window: a slow glyph in place of
    /// the state's own, and `compacting the conversation` on its activity
    /// row, and the queue holds words against it (`quiet::holds_against_words`).
    /// Set by the daemon as the window opens and closes, from the machine;
    /// a restart's machine starts with none, and the record is reset with it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compacting: bool,
}

/// What a launch road meant to submit and did not (T-570). The ticket's
/// brief is read again at resend time, the way the first paste read it
/// (T-117), so only the person's own words are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsent {
    /// The words that rode with it: the ask under a brief, or the whole
    /// ask on a wake. Empty for the plain composed spawn.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The ticket's title and description were the prompt (the composed
    /// spawn, T-224).
    #[serde(default, skip_serializing_if = "is_false")]
    pub brief: bool,
}

impl Unsent {
    /// What the card, the ticket page and a banner say about it.
    pub fn word(&self) -> &'static str {
        if self.brief {
            "brief not sent"
        } else {
            "prompt not sent"
        }
    }
}

impl SessionRecord {
    /// Whether this record's pane reports through the mod (T-577): a mod
    /// launch from a build that passes no hook set, or a native one (T-658),
    /// whose `--settings` holds the permission hooks alone. A record launched
    /// on the mod beside the whole hook set (T-650, or an earlier build)
    /// reports through the hook set until its next wake, which re-decides.
    pub fn frames_by_mod(&self) -> bool {
        self.road == crate::road::Road::Mod
            && (self.native || !self.argv.iter().any(|a| a == "--settings"))
    }

    /// Words that never reached this seat's agent (T-570), while there is a
    /// pane to resend them into. A parked or dead seat has nothing to type
    /// at, and its card says so in its own words.
    pub fn unsent_words(&self) -> Option<&Unsent> {
        self.unsent.as_ref().filter(|_| self.state.has_pane())
    }

    /// The seat as ONE WORD for an agent reading another ticket (T-411):
    /// `agent_state_word`, except that a seat whose first prompt never went
    /// is `unsent` — an `idle` there reads as a turn that ended, and none
    /// began (T-570).
    pub fn state_word(&self) -> &'static str {
        if self.unsent_words().is_some() {
            "unsent"
        } else if self.kind.is_agent() && self.foreground.is_some() {
            // A `!` command running in the composer (T-707): the seat is
            // busy with the person's own command, whatever the hooks last said.
            "working"
        } else {
            agent_state_word(&self.state)
        }
    }

    /// Still owed the deferred Enter of a Shift+Enter spawn, in a state where
    /// pressing it is safe. A pane that died, or one showing a startup
    /// modal, is not a pane to keep pressing Enter into — the modal's Enter
    /// is an ANSWER, and mesimon does not answer dialogs on the user's behalf.
    pub fn pressable(&self) -> bool {
        self.pending_submit
            && !self.pending_prefill
            && !self.codex_submit_sent
            && self.state.has_pane()
            && matches!(
                self.state,
                SessionState::Spawning | SessionState::Idle { .. } | SessionState::Running
            )
    }

    pub fn new(
        id: uuid::Uuid,
        kind: SessionKind,
        ticket: ulid::Ulid,
        argv: Vec<String>,
        cwd: String,
        state: SessionState,
    ) -> Self {
        Self {
            id,
            kind,
            ticket,
            argv,
            cwd,
            state,
            waiting_since: None,
            state_changed_at: None,
            transcript_path: None,
            detail: None,
            title: None,
            started_by: None,
            foreground: None,
            shell_shared: false,
            shell_close_request: None,
            confidence: Confidence::default(),
            provenance: Provenance::default(),
            claude_session_id: None,
            pane_key: None,
            launched_at: None,
            typed_title: None,
            road: crate::road::Road::Hooks,
            native: false,
            codex_thread_id: None,
            codex_generation: None,
            codex_observed_seq: 0,
            codex_pending_seq: None,
            codex_turn_id: None,
            codex_failed_turn: None,
            agent_preview_path: None,
            agent_plan_key: None,
            observation_hold: kind == SessionKind::Codex,
            pending_submit: false,
            pending_prefill: false,
            codex_submit_sent: false,
            codex_stopping: false,
            codex_plan_dialog_seen: false,
            codex_plan_dismissed_turn: None,
            codex_plan_owed: false,
            idle_teammates: Vec::new(),
            background_tasks: Default::default(),
            plan_note: None,
            dialog_call: None,
            dialog_print: None,
            ticket_read: false,
            tier: String::new(),
            tier_owed: false,
            tier_wake: false,
            unsent: None,
            tasks_running: None,
            tasks_since: None,
            unprompted: false,
            compacting: false,
        }
    }

    /// The state written, and the held call (`dialog_call`, T-957 round 10)
    /// let go whenever the record is not on a plan or a question: every
    /// departure, a park, a death or a supervisor's loss as much as the
    /// machine's, goes through here.
    pub fn set_state(&mut self, state: SessionState) {
        if !matches!(
            state,
            SessionState::RequiresAction { reason: Reason::Plan | Reason::Question }
        ) {
            self.dialog_call = None;
            self.dialog_print = None;
        }
        self.state = state;
    }

    /// A column's opt-in (T-543): a confirmed agent idle at its prompt past
    /// the timeout, whether or not a turn finished there — just woken,
    /// interrupted, or done waiting on background work — because a column
    /// that asked for its agents to sleep meant every idle one (the author's
    /// T-534 was woken in DONE and never slept). Background and monitoring
    /// are still work. The daemon additionally checks pending work,
    /// ownership and resume history; a conversation with no history yet is
    /// its to refuse. A stop on a limit (T-808) is idle at its prompt too.
    pub fn autosleep_due(&self, now: u64, timeout_ms: u64) -> bool {
        (matches!(
            self.state,
            SessionState::Idle {
                stop_reason: StopReason::EndTurn | StopReason::Interrupted | StopReason::Unknown
            }
        ) || self.state.stopped_on_limit())
            && self.park_clock_due(now, timeout_ms)
    }

    fn park_clock_due(&self, now: u64, timeout_ms: u64) -> bool {
        timeout_ms > 0
            && self.kind.is_agent()
            && self.confidence == Confidence::High
            && (self.kind != SessionKind::Codex
                || (!self.observation_hold
                    && !self.codex_stopping
                    && !self.codex_submit_sent
                    && self.codex_pending_seq.is_none()))
            && self.state_changed_at.is_some_and(|at| now.saturating_sub(at) >= timeout_ms)
            && !self.argv.is_empty()
            && !self.pending_submit
            && !self.pending_prefill
    }

    /// A retiring runtime still owns its ticket and conversation until cleanup
    /// acknowledges that its processes stopped, even if the badge is Exited.
    pub fn holds_agent_seat(&self) -> bool {
        self.kind.is_agent() && (self.state.is_live() || self.codex_stopping)
    }

    /// The pane's death frame, read before the machine moves the record: a
    /// Codex pane that died with the record still on it leaves a runtime
    /// winding down, which holds the seat until cleanup confirms. A record
    /// the board already closed (`kill_session`'s `Exited`, a park's
    /// `Sleeping`) raised the flag itself when it closed, and its cleanup
    /// may have confirmed since: a death frame arriving after that names no
    /// new runtime, and raising it again put a dead seat beside the next
    /// agent on the ticket (T-1026). True when the record changed.
    pub fn pane_died(&mut self) -> bool {
        if self.kind != SessionKind::Codex || !self.state.has_pane() {
            return false;
        }
        self.codex_stopping = true;
        self.observation_hold = true;
        true
    }

    /// On the plan dialog (T-420): the card's `≡`.
    pub fn on_plan_dialog(&self) -> bool {
        self.state == (SessionState::RequiresAction { reason: Reason::Plan })
    }

    /// On the harness's folder trust dialog (T-844): TRUST on the card, a
    /// dialog that comes before the session and that the board may accept
    /// for a person who asked.
    pub fn on_trust_dialog(&self) -> bool {
        self.state == (SessionState::RequiresAction { reason: Reason::Trust })
    }

    /// Launched with `--permission-mode plan` — the one fact the board holds
    /// about a mode. A session that entered plan mode from inside its pane
    /// is not known to be planning.
    pub fn launched_in_plan(&self) -> bool {
        self.argv.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "plan")
    }

    /// A process mesimon must still account for: a pane, or a Codex runtime
    /// whose cleanup has not confirmed it stopped.
    pub fn holds_process(&self) -> bool {
        self.state.has_pane() || self.codex_stopping
    }

    /// A Codex session mesimon launched whose runtime may still be running:
    /// its snapshot is polled and its checkout held until cleanup says so.
    pub fn owns_codex_runtime(&self) -> bool {
        self.kind == SessionKind::Codex && !self.argv.is_empty() && self.holds_process()
    }

    /// Adopted to be watched, never launched: no argv, so mesimon may read
    /// its transcript but owns no process to resume, wake or kill.
    pub fn observe_only(&self) -> bool {
        self.provenance == Provenance::Adopted && self.argv.is_empty()
    }
}

/// The shells a pane is idle at. `#{pane_current_command}` is the process's
/// NAME, not `$SHELL`'s path, and the two disagree in the wild: macOS's
/// `/bin/sh` execs `bash` (or `zsh`, or `dash`) and reports THAT name, and a
/// user whose `$SHELL` is one shell may sit at another. So a known shell at
/// the prompt is idle whatever the pane was launched as; a nested shell at
/// its prompt is idle too, which is the truth.
const SHELL_NAMES: &[&str] =
    &["sh", "bash", "zsh", "fish", "dash", "ksh", "mksh", "tcsh", "csh", "nu", "xonsh", "elvish"];

/// What a shell pane is running, from tmux's `#{pane_current_command}` and
/// the shell the pane was born with (T-366). tmux reports the foreground
/// process's NAME — the shell's own at a prompt (`zsh`, `bash`, `fish`), the
/// command's while one runs (`cargo`, `claude`) — so the shell's name, or any
/// name in `SHELL_NAMES`, means idle and anything else is a foreground
/// command. `shell` is a path or a name (`/bin/zsh`, `zsh`); a login shell's
/// leading `-` is stripped on both sides in case a tmux ever reports argv[0].
pub fn foreground_of(current_command: &str, shell: &str) -> Option<String> {
    fn bare(s: &str) -> &str {
        let s = s.trim();
        let s = s.rsplit('/').next().unwrap_or(s);
        s.strip_prefix('-').unwrap_or(s)
    }
    let cmd = bare(current_command);
    if cmd.is_empty() || cmd == bare(shell) || SHELL_NAMES.contains(&cmd) {
        return None;
    }
    Some(cmd.to_string())
}

/// One row of the process table, as `ps -axo pid=,ppid=,etime=,comm=`
/// prints it: the pid, its parent, how long it has run and its command name
/// (a path on macOS, a bare name on Linux; `bash_mode_foreground` takes the
/// basename of either).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: i32,
    pub ppid: i32,
    pub age_secs: u64,
    pub comm: String,
}

/// `ps`'s `etime` — `[[dd-]hh:]mm:ss` on BSD and procps alike — as seconds.
/// `None` for anything else (a header, a `-` for a process with no start).
pub fn parse_etime(s: &str) -> Option<u64> {
    let s = s.trim();
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let mut secs = 0u64;
    let mut parts = 0;
    for p in rest.split(':') {
        secs = secs.checked_mul(60)?.checked_add(p.parse::<u64>().ok()?)?;
        parts += 1;
    }
    if !(2..=3).contains(&parts) {
        return None;
    }
    Some(days * 86_400 + secs)
}

/// `ps -axo pid=,ppid=,etime=,comm=` output into rows; a line that does not
/// parse is skipped. `comm` is the last field, taken whole: a macOS path may
/// hold a space.
pub fn parse_ps_rows(out: &str) -> Vec<ProcRow> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let ppid = f.next()?.parse().ok()?;
            let age_secs = parse_etime(f.next()?)?;
            let comm = f.collect::<Vec<_>>().join(" ");
            (!comm.is_empty()).then_some(ProcRow { pid, ppid, age_secs, comm })
        })
        .collect()
}

/// The `!` command a Claude pane is running (T-707), read off the process
/// table: Claude Code runs a bash-mode command as a shell it spawns
/// directly, with no controlling tty — so `#{pane_current_command}` keeps
/// naming `claude`, no hook fires, and the transcript gets both the command
/// and its output only when it ends. The one live fact is a shell among the
/// pane process's children.
///
/// Claude's other children are shells too — an MCP server launched through
/// `sh -c`, a Monitor watch, a background Bash task the last `Stop` reported —
/// and every one of them is older than the idle spell the record is in,
/// while a `!` command is typed into it. So only a shell younger than
/// `spell_secs` counts, the youngest when several, and the foreground is
/// named by that shell's own first child (`cargo` for `! cargo test`), or by
/// the shell while it has none.
pub fn bash_mode_foreground(pane_pid: i32, spell_secs: u64, rows: &[ProcRow]) -> Option<String> {
    fn name(comm: &str) -> &str {
        let s = comm.trim();
        let s = s.rsplit('/').next().unwrap_or(s);
        s.strip_prefix('-').unwrap_or(s)
    }
    let shell = rows
        .iter()
        .filter(|r| r.ppid == pane_pid && SHELL_NAMES.contains(&name(&r.comm)))
        .filter(|r| r.age_secs < spell_secs)
        .min_by_key(|r| (r.age_secs, r.pid))?;
    let child = rows.iter().filter(|r| r.ppid == shell.pid).min_by_key(|r| r.pid);
    Some(name(&child.unwrap_or(shell).comm).to_string())
}

#[cfg(test)]
mod bash_mode_tests {
    use super::{bash_mode_foreground, parse_etime, parse_ps_rows, ProcRow};

    fn row(pid: i32, ppid: i32, age_secs: u64, comm: &str) -> ProcRow {
        ProcRow { pid, ppid, age_secs, comm: comm.into() }
    }

    #[test]
    fn etime_is_seconds_minutes_hours_and_days() {
        assert_eq!(parse_etime("00:09"), Some(9));
        assert_eq!(parse_etime("   06:08"), Some(368));
        assert_eq!(parse_etime("01:02:03"), Some(3723));
        assert_eq!(parse_etime("01-08:46:18"), Some(86_400 + 8 * 3600 + 46 * 60 + 18));
        assert_eq!(parse_etime("20-01:29:44"), Some(20 * 86_400 + 3600 + 29 * 60 + 44));
        assert_eq!(parse_etime("-"), None);
        assert_eq!(parse_etime("ELAPSED"), None);
        assert_eq!(parse_etime("9"), None);
        assert_eq!(parse_etime("1:2:3:4"), None);
    }

    #[test]
    fn ps_rows_parse_and_keep_a_path_with_a_space() {
        let rows = parse_ps_rows(
            " 5279     1 01-08:46:18 /opt/homebrew/bin/tmux
             37473 33759       00:04 /bin/zsh
             37474 37473       00:04 sleep
             99 1 00:01 /Applications/My App/bin/thing
             garbage line
",
        );
        assert_eq!(
            rows,
            [
                row(5279, 1, 86_400 + 8 * 3600 + 46 * 60 + 18, "/opt/homebrew/bin/tmux"),
                row(37473, 33759, 4, "/bin/zsh"),
                row(37474, 37473, 4, "sleep"),
                row(99, 1, 1, "/Applications/My App/bin/thing"),
            ]
        );
    }

    #[test]
    fn a_young_shell_child_names_the_command_by_its_own_child() {
        // The pane process is claude (33759), idle for 60 s. Its MCP servers
        // are as old as the session; the `!` shell is 4 s old.
        let rows = [
            row(33759, 100, 600, "claude"),
            row(35716, 33759, 598, "/bin/sh"),
            row(35720, 33759, 598, "node"),
            row(37473, 33759, 4, "/bin/zsh"),
            row(37474, 37473, 4, "/usr/bin/sleep"),
        ];
        assert_eq!(bash_mode_foreground(33759, 60, &rows), Some("sleep".into()));
        // A shell still evaluating builtins, no child yet: the shell's name.
        assert_eq!(bash_mode_foreground(33759, 60, &rows[..4]), Some("zsh".into()));
        // Nothing younger than the spell: the MCP server's `sh -c` is not a
        // command the person typed.
        assert_eq!(bash_mode_foreground(33759, 3, &rows), None);
        assert_eq!(bash_mode_foreground(33759, 0, &rows), None);
        // Another pane's children are not this pane's.
        assert_eq!(bash_mode_foreground(1, 60, &rows), None);
        // A young child that is not a shell (a hook exec'd straight) is not
        // a command either.
        let hook = [row(33759, 100, 600, "claude"), row(40000, 33759, 1, "mesimon")];
        assert_eq!(bash_mode_foreground(33759, 60, &hook), None);
    }

    #[test]
    fn the_youngest_shell_wins_when_two_are_young() {
        // A Monitor watch born 30 s into a 60 s spell, and a `!` typed 2 s ago.
        let rows = [
            row(1, 0, 600, "claude"),
            row(2, 1, 30, "bash"),
            row(3, 2, 30, "tail"),
            row(4, 1, 2, "zsh"),
            row(5, 4, 2, "cargo"),
        ];
        assert_eq!(bash_mode_foreground(1, 60, &rows), Some("cargo".into()));
    }
}

#[cfg(test)]
mod foreground_tests {
    use super::foreground_of;

    #[test]
    fn a_shell_at_its_prompt_is_idle_and_anything_else_is_a_command() {
        assert_eq!(foreground_of("zsh", "/bin/zsh"), None);
        assert_eq!(foreground_of("-zsh", "zsh"), None);
        assert_eq!(foreground_of("", "/bin/zsh"), None);
        // macOS: `/bin/sh` execs bash and the pane names bash.
        assert_eq!(foreground_of("bash", "/bin/sh"), None);
        assert_eq!(foreground_of("fish", "/bin/zsh"), None);
        assert_eq!(foreground_of("cargo", "/bin/zsh"), Some("cargo".into()));
        assert_eq!(foreground_of("/usr/bin/claude", "/bin/zsh"), Some("claude".into()));
        assert_eq!(foreground_of("sleep", "/bin/sh"), Some("sleep".into()));
    }
}

impl SessionRecord {
    /// The tmux session name for this record: first 16 hex chars of the UUID.
    pub fn sid16(&self) -> String {
        self.id.simple().to_string()[..16].to_string()
    }
}

/// A column: its name — which IS its identity, `Ticket.column`'s foreign
/// key; there is no id, so a rename is a transaction over every ticket — its
/// order, and its settings (T-117). The settings are flattened, so on disk
/// their keys sit beside `name` and `order` inside the `[[columns]]` table
/// and every one of them defaults: a file from before T-117 parses as it did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// Fractional index; columns sort by it.
    pub order: String,
    #[serde(flatten)]
    pub settings: ColumnSettings,
}

impl Column {
    pub fn new(name: impl Into<String>, order: impl Into<String>) -> Self {
        Self { name: name.into(), order: order.into(), settings: ColumnSettings::default() }
    }
}

/// The `--permission-mode` a claude started on a ticket in this column runs
/// with (T-117). `Inherit` is the user's own `permissions.defaultMode`, the
/// pass-through every spawn made before columns had a say. The enum has no
/// `bypassPermissions` and no `dontAsk` on purpose: a column may narrow what
/// a person configured, never hand out a mode nobody asked for (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeMode {
    #[default]
    Inherit,
    Auto,
    Plan,
    Manual,
}

impl ClaudeMode {
    /// The flag's word, `None` for inherit. Claude Code 2.1.261 spells them
    /// `auto`, `plan`, `manual` — and `manual`, never `default`, which the
    /// flag refuses.
    pub fn flag_word(self) -> Option<&'static str> {
        match self {
            Self::Inherit => None,
            Self::Auto => Some("auto"),
            Self::Plan => Some("plan"),
            Self::Manual => Some("manual"),
        }
    }

    pub fn word(self) -> &'static str {
        self.flag_word().unwrap_or("inherit")
    }

    pub fn next(self) -> Self {
        match self {
            Self::Inherit => Self::Auto,
            Self::Auto => Self::Plan,
            Self::Plan => Self::Manual,
            Self::Manual => Self::Inherit,
        }
    }

    fn is_inherit(&self) -> bool {
        *self == Self::Inherit
    }
}

/// Which of mesimon's own MCP tools a claude on a ticket in this column may
/// call (T-117): board authority, tiered. Ordered — `Off < Read < Annotate <
/// Full` — and `mcp::AgentTools::needed_by` is the table that says which
/// tool needs which rung. Advertised at spawn (the shim lists only these)
/// and enforced by the daemon on every call against the ticket's CURRENT
/// column, so a REVIEW column can keep an agent from moving its own ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTools {
    /// No `--mcp-config` at all, like `Board::mcp_tools` off.
    Off,
    /// `get_ticket`, `list_board`, `read_note`.
    Read,
    /// Read, plus `write_note` and `tag_ticket`.
    Annotate,
    /// Everything: `move_ticket` and `create_ticket` too. Today's behaviour.
    #[default]
    Full,
}

impl AgentTools {
    /// The word on the shim's argv and in the file.
    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Read => "read",
            Self::Annotate => "annotate",
            Self::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => Self::Off,
            "read" => Self::Read,
            "annotate" => Self::Annotate,
            "full" => Self::Full,
            _ => return None,
        })
    }

    /// The dialog's cycle: narrowing first, so one press from `full` asks
    /// the smallest question.
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Annotate,
            Self::Annotate => Self::Read,
            Self::Read => Self::Off,
            Self::Off => Self::Full,
        }
    }

    fn is_full(&self) -> bool {
        *self == Self::Full
    }
}

/// What lands a ticket in this column on its own, read off the forge
/// (T-739): nothing, a pull request opened, its pipeline passed, every PR
/// merged, or the merge commit deployed to the environment the column
/// names (`arrives_env`). Edge-triggered on what a forge pass saw change,
/// through `place_ticket` under the automation principal, so the move gate
/// applies as to `on_done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrival {
    #[default]
    Nothing,
    PrOpened,
    PipelinePasses,
    Merged,
    Deployed,
}

impl Arrival {
    pub const ALL: [Arrival; 5] = [
        Arrival::Nothing,
        Arrival::PrOpened,
        Arrival::PipelinePasses,
        Arrival::Merged,
        Arrival::Deployed,
    ];

    /// The dialog's word.
    pub fn word(self) -> &'static str {
        match self {
            Arrival::Nothing => "nothing",
            Arrival::PrOpened => "PR opened",
            Arrival::PipelinePasses => "pipeline passes",
            Arrival::Merged => "merged",
            Arrival::Deployed => "deployed to",
        }
    }

    pub fn next(self) -> Self {
        let i = Arrival::ALL.iter().position(|a| *a == self).unwrap_or(0);
        Arrival::ALL[(i + 1) % Arrival::ALL.len()]
    }

    fn is_nothing(&self) -> bool {
        *self == Arrival::Nothing
    }
}

/// The column a forge pass's edge takes a ticket to (T-739): among the
/// columns whose rule the edge satisfies (`satisfied`), the furthest stage
/// — a deployment over a merge over a pass over an opening, `Arrival`'s
/// order — and among equals the right-most in board order, `columns` being
/// that order. A deploy column left of the ticket's own never takes it
/// (T-891): a ticket in production stays there when dev and staging read
/// deployed a pass later, so a later environment's column never yields to
/// an earlier one's. A merge or an opening may still take a ticket back, as
/// a follow-up PR on a shipped ticket does. `None` where nothing takes it.
pub fn arrival_dest<'a>(
    columns: &[&'a Column],
    from: &str,
    mut satisfied: impl FnMut(&Column) -> bool,
) -> Option<&'a str> {
    let at = columns.iter().position(|c| c.name == from);
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.name != from && !c.settings.arrives_on.is_nothing())
        .filter(|(i, c)| c.settings.arrives_on != Arrival::Deployed || at.is_none_or(|a| *i > a))
        .filter(|(_, c)| satisfied(c))
        .max_by_key(|(i, c)| (c.settings.arrives_on, *i))
        .map(|(_, c)| c.name.as_str())
}

/// How far the merge train reaches into this column (T-117): nowhere, rebase
/// asks only, or auto-merge candidates and rebase asks. The train's global
/// preference is still the consent switch; this says where it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainReach {
    #[default]
    Off,
    Rebase,
    Merge,
}

impl TrainReach {
    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Rebase => "rebase asks",
            Self::Merge => "auto-merge",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Rebase,
            Self::Rebase => Self::Merge,
            Self::Merge => Self::Off,
        }
    }

    fn is_off(&self) -> bool {
        *self == Self::Off
    }
}

/// A one-shot order for `Board::sort_column` (T-117). Not a setting: nothing
/// keeps a column sorted, and every gesture keeps working afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// Most recent arrival in the column first.
    NewestArrival,
    OldestArrival,
    /// `T-1`, `T-2`, … by number.
    Key,
    /// Needs-you first, each half keeping its order.
    NeedsYouFirst,
    /// Clumped by tag, in the order the PICKER's rows draw (T-283) — which
    /// is registry order, and `MoveTag` is what arranges it. Axis 1 decides,
    /// axis 2 breaks its ties, and so on; an axis a ticket wears nothing on
    /// sorts after every tag on it, so the untagged fall to the bottom.
    Tag,
}

impl SortBy {
    /// `Tag` is LAST on purpose: the dialog's row opens on `ALL[0]` and the
    /// column-settings goldens read `Sort once   newest first`.
    pub const ALL: [SortBy; 5] = [
        SortBy::NewestArrival,
        SortBy::OldestArrival,
        SortBy::Key,
        SortBy::NeedsYouFirst,
        SortBy::Tag,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Self::NewestArrival => "newest first",
            Self::OldestArrival => "oldest first",
            Self::Key => "by key",
            Self::NeedsYouFirst => "needs-you first",
            Self::Tag => "by tag",
        }
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Which bulk actions the column offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnOffers {
    #[default]
    Off,
    Sleep,
    Archive,
    Both,
}

impl ColumnOffers {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Sleep,
            Self::Sleep => Self::Archive,
            Self::Archive => Self::Both,
            Self::Both => Self::Off,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Sleep => "sleep",
            Self::Archive => "archive",
            Self::Both => "sleep + archive",
        }
    }

    pub fn sleep(self) -> bool {
        matches!(self, Self::Sleep | Self::Both)
    }

    pub fn archive(self) -> bool {
        matches!(self, Self::Archive | Self::Both)
    }
}

/// When a column folds to a one-cell spine (T-837), in the order Enter
/// cycles them. On disk it is two switches, `collapsed` (always) and
/// `collapsed_when_empty`, so a build that drops the second only shows an
/// empty column open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collapse {
    #[default]
    Off,
    Always,
    WhenEmpty,
}

impl Collapse {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Always,
            Self::Always => Self::WhenEmpty,
            Self::WhenEmpty => Self::Off,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Always => "always",
            Self::WhenEmpty => "when empty",
        }
    }
}

/// Native Codex launch policies. Inherit leaves the user's CLI configuration intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSandbox {
    #[default]
    Inherit,
    ReadOnly,
    WorkspaceWrite,
}
impl CodexSandbox {
    pub fn is_inherit(&self) -> bool {
        *self == Self::Inherit
    }
    pub fn next(self) -> Self {
        match self {
            Self::Inherit => Self::ReadOnly,
            Self::ReadOnly => Self::WorkspaceWrite,
            Self::WorkspaceWrite => Self::Inherit,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::ReadOnly => "read only",
            Self::WorkspaceWrite => "workspace write",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexApproval {
    #[default]
    Inherit,
    OnRequest,
    Never,
}
impl CodexApproval {
    pub fn is_inherit(&self) -> bool {
        *self == Self::Inherit
    }
    pub fn next(self) -> Self {
        match self {
            Self::Inherit => Self::OnRequest,
            Self::OnRequest => Self::Never,
            Self::Never => Self::Inherit,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::OnRequest => "on request",
            Self::Never => "never",
        }
    }
}

/// Everything a column decides (T-117). Every automation the daemon runs
/// on a ticket is a field here, read off the ticket's column through
/// `Board::column` and never off a column NAME: `on_working`/`on_done` ARE
/// automove, `on_merged` its merge edge (T-1037), `train` is the merge train's reach, `requires_merge` is the
/// DONE gate (`requires_ticked` its summary-box twin), `reclaim` is the
/// sleep/archive offer. The defaults are what a column with no template
/// settings did before: nothing moves, tools full, mode inherited, train
/// off. Each default is skipped on disk so a file says only what was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColumnSettings {
    /// What the column is for, in the user's words (T-467): one line,
    /// `sanitize_column_description`'s bound. Agents read it in `list_board`
    /// and `get_ticket` — a tool RESULT, the same road a note takes, never
    /// tool text — so "BACKLOG vs TODO" is the user's answer and not a
    /// guess. `None` = no description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Pinned as a one-cell spine unless the cursor is in it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// Pinned so only while no ticket is in it (T-837); `collapse()` reads
    /// the pair. No schema bump: a build that drops it shows the column open.
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed_when_empty: bool,
    /// The workspace a ticket CREATED here starts with, stamped onto the
    /// ticket at mint; the ticket field stays the truth and a change here is
    /// never retroactive. `None` = the board's (`Board::workspace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
    /// No ticket is minted here (T-764): a column a ticket only reaches by
    /// moving, as IN PROGRESS and REVIEW are. `o` on it files into
    /// `Board::filing_column`, and the daemon refuses a mint that names it.
    /// No schema bump: a build that drops it only lets a mint in again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub refuses_new: bool,
    #[serde(default, skip_serializing_if = "ClaudeMode::is_inherit")]
    pub claude_mode: ClaudeMode,
    #[serde(default, skip_serializing_if = "CodexSandbox::is_inherit")]
    pub codex_sandbox: CodexSandbox,
    #[serde(default, skip_serializing_if = "CodexApproval::is_inherit")]
    pub codex_approval: CodexApproval,
    #[serde(default, skip_serializing_if = "AgentTools::is_full")]
    pub agent_tools: AgentTools,
    /// The column a ticket moves to when its claude starts working
    /// (`Running` at Medium or better). `None` = stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_working: Option<String>,
    /// The column a ticket moves to when its claude ends a turn
    /// (`Idle{EndTurn}` at Medium or better). `None` = stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_done: Option<String>,
    /// The column a ticket moves to when a merge mesimon makes lands its
    /// branch while it sits here (T-1037): the train's, the crown's or a
    /// person's `m`. `None` = stay. No schema bump: a build that drops it
    /// only leaves a merged ticket where it was, as every build before did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_merged: Option<String>,
    /// Entry is refused while the ticket's worktree branch is unmerged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_merge: bool,
    /// Entry is refused unless the ticket's pipeline passed (T-739): a
    /// failing, running, absent or unread pipeline is refused alike, since
    /// unknown is never green.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_pipeline: bool,
    /// What lands a ticket here on its own, off the forge (T-739).
    #[serde(default, skip_serializing_if = "Arrival::is_nothing")]
    pub arrives_on: Arrival,
    /// The environment `Arrival::Deployed` waits for, as the forge names
    /// it (`staging`, `production`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrives_env: Option<String>,
    /// Entry, and an archive from here, are refused while a `Summary` box on
    /// the ticket is unticked (T-735; T-713 refused every archive, with no
    /// switch). Off on every column, the template's DONE too.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_ticked: bool,
    /// The header's sleep and archive offers, `X` and `Z` price this column.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reclaim: bool,
    /// Independent offer selection. Absent in older boards: `reclaim` still
    /// determines both offers there. An explicit choice overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offers: Option<ColumnOffers>,
    #[serde(default, skip_serializing_if = "TrainReach::is_off")]
    pub train: TrainReach,
    /// Park a Claude agent here once it has been idle at its prompt this
    /// many minutes (T-543, `SessionRecord::autosleep_due`); zero is off.
    /// The board's only idle timer since T-610 (`Board::idle_park_due`). No
    /// schema bump: a build that drops it only stops sleeping agents, which
    /// narrows nothing.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sleep_after_minutes: u32,
}

/// The column's auto-sleep choices, in the order Enter cycles them.
pub const COLUMN_SLEEP_MINUTES: [u32; 5] = [0, 1, 5, 15, 60];

impl ColumnSettings {
    pub fn offers(&self) -> ColumnOffers {
        self.offers.unwrap_or(if self.reclaim { ColumnOffers::Both } else { ColumnOffers::Off })
    }

    pub fn collapse(&self) -> Collapse {
        if self.collapsed {
            Collapse::Always
        } else if self.collapsed_when_empty {
            Collapse::WhenEmpty
        } else {
            Collapse::Off
        }
    }

    pub fn set_collapse(&mut self, c: Collapse) {
        self.collapsed = c == Collapse::Always;
        self.collapsed_when_empty = c == Collapse::WhenEmpty;
    }

    /// Whether any automation would act on a ticket here — the header's one
    /// optional mark.
    pub fn automated(&self) -> bool {
        self.arrives_on != Arrival::Nothing
            || self.on_working.is_some()
            || self.on_done.is_some()
            || self.on_merged.is_some()
            || self.train != TrainReach::Off
            || self.sleep_after_minutes > 0
    }

    /// The next auto-sleep choice after this column's: through
    /// `COLUMN_SLEEP_MINUTES`, a value from elsewhere landing on the next
    /// one above it.
    pub fn next_sleep_after(&self) -> u32 {
        COLUMN_SLEEP_MINUTES.iter().copied().find(|&m| m > self.sleep_after_minutes).unwrap_or(0)
    }

    /// The non-default settings in words, for `doctor` and the dialog.
    pub fn summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(d) = &self.description {
            out.push(format!("about: {d}"));
        }
        match self.collapse() {
            Collapse::Off => {}
            Collapse::Always => out.push("collapsed".into()),
            Collapse::WhenEmpty => out.push("collapsed when empty".into()),
        }
        if self.refuses_new {
            out.push("takes no new tickets".into());
        }
        if let Some(w) = self.workspace {
            out.push(format!("workspace: {}", w.word()));
        }
        if self.claude_mode != ClaudeMode::Inherit {
            out.push(format!("claude: {}", self.claude_mode.word()));
        }
        if self.codex_sandbox != CodexSandbox::Inherit {
            out.push(format!("codex sandbox: {}", self.codex_sandbox.word()));
        }
        if self.codex_approval != CodexApproval::Inherit {
            out.push(format!("codex approvals: {}", self.codex_approval.word()));
        }
        if self.agent_tools != AgentTools::Full {
            out.push(format!("agent tools: {}", self.agent_tools.word()));
        }
        if let Some(c) = &self.on_working {
            out.push(format!("working → {c}"));
        }
        if let Some(c) = &self.on_done {
            out.push(format!("done → {c}"));
        }
        if let Some(c) = &self.on_merged {
            out.push(format!("merged → {c}"));
        }
        if self.requires_merge {
            out.push("entry needs a merged branch".into());
        }
        if self.requires_pipeline {
            out.push("entry needs a passed pipeline".into());
        }
        match self.arrives_on {
            Arrival::Nothing => {}
            Arrival::Deployed => out.push(format!(
                "arrives when deployed to {}",
                self.arrives_env.as_deref().unwrap_or("?")
            )),
            a => out.push(format!("arrives when {}", a.word())),
        }
        if self.requires_ticked {
            out.push("entry and archive need every summary box ticked".into());
        }
        if self.offers() != ColumnOffers::Off {
            out.push(format!("offers {}", self.offers().word()));
        }
        if self.train != TrainReach::Off {
            out.push(format!("merge train: {}", self.train.word()));
        }
        if self.sleep_after_minutes > 0 {
            out.push(format!("sleeps idle agents after {} min", self.sleep_after_minutes));
        }
        out
    }
}

/// Durable execution restriction supplied by a trusted intake/import path.
/// Unlike `manual_merge`, this is not a user preference and has no toggle command.
/// It restricts Mesimon automation; it is not a sandbox for a local process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPolicy {
    #[default]
    LocalAutomation,
    OwnerOnly,
}

impl ExecutionPolicy {
    pub fn allows_automation(&self) -> bool {
        matches!(self, Self::LocalAutomation)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    /// Immutable identity; never appears in a path (D24).
    pub id: ulid::Ulid,
    /// Human-speakable display key, stable across moves, never encodes state (D24).
    pub short_key: String,
    pub title: String,
    pub column: String,
    /// Fractional index within the column.
    pub order: String,
    pub created_at: String,
    /// Who minted the ticket, in [`crate::Principal::note_author`]'s words —
    /// `local` for a person at the composer, `agent:<session-uuid>` for an
    /// agent's `create_ticket` — so a ticket and its notes name an author the
    /// same way (T-253, 2026-09-05). Empty on a ticket from before the field,
    /// which reads as UNKNOWN, never as a person. A scalar, with the scalars.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_by: String,
    /// The ticket the agent that filed this one was working on (T-253): the
    /// caller's binding at `create_ticket`, a fact of the FILE rather than a
    /// join against a session record a delete can take away. `None` on a
    /// person's ticket and on one from before the field. A ULID, never a key:
    /// the page resolves the key from the board and says nothing when that
    /// ticket is gone too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_from: Option<ulid::Ulid>,
    /// When the ticket entered its CURRENT column, same clock as `created_at`.
    /// Stamped at mint and by every column move — never by a reorder inside
    /// the column, a rename, a tag or a session — so the card's age is "time
    /// in column". `None` on a ticket from before the field; `column_since`
    /// falls back to `created_at` there, the only honest value left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_at: Option<String>,
    /// When a snooze woke this ticket with `needs_you` set, until the cursor
    /// has rested on the card (`SeenTicket`). Set means the card wears the
    /// needs-you mark with no session behind it — the one ticket-level
    /// attention producer (T-74). A scalar, so it sits with the scalars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub woke_at: Option<String>,
    /// The merge train leaves this ticket alone (T-227): no automatic merge
    /// and no rebase ask; `m` by hand still does both. The user's own
    /// opt-out, so it persists — a restart re-arms the train, and a ticket
    /// taken off it must not climb back on. Off by default (the train
    /// reaches every attached worktree ticket unless told otherwise), and
    /// omitted from the file while off. A scalar, with the scalars.
    #[serde(default, skip_serializing_if = "is_false")]
    pub manual_merge: bool,
    /// A durable floor on automatic execution and merge, independent of the
    /// toggleable `manual_merge` preference. Unknown policies fail decoding.
    #[serde(default, skip_serializing_if = "ExecutionPolicy::allows_automation")]
    pub execution_policy: ExecutionPolicy,
    /// The agent tier this ticket picked (T-443): a tier id, resolved against
    /// the machine's and the board's tiers (`tier::Book`). `None` follows the
    /// default. An id that no longer resolves reads as the default too. A
    /// scalar, with the scalars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    /// The Mesophon envelope this ticket was filed from (T-497): a paired
    /// browser's mailbox id, 32 hex characters, so an envelope the relay
    /// hands over twice files one ticket. Written by the mint, in the same
    /// file as the ticket; never edited, and a copy does not carry it. A
    /// scalar, with the scalars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<String>,
    /// Per-ticket workspace strategy (M4 layering: the ticket field is the truth;
    /// a column's `workspace` setting only defaults a NEW ticket, stamped here at
    /// mint — T-117). `None` = inherit the board default. Must stay after the
    /// scalar fields (TOML serialize order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
    /// Durable correlation for externally imported content; never an identity
    /// assertion or an authorization grant. No command edits this field. Copies
    /// retain it, and its presence imposes the OwnerOnly execution floor even
    /// when an incomplete import omitted the separate policy field.
    /// A TOML table, so it follows all scalar fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_origin: Option<crate::content::ImportOrigin>,
    /// An agent asked for a person and has not been answered (T-107): the
    /// SECOND ticket-level producer of the saturated colour, beside
    /// `woke_at`. A fact about the ticket rather than about the session, so
    /// it outlives the turn that raised it, the session being slept, and a
    /// daemon restart — which is the whole reason it is not a
    /// `RequiresAction` reason. Lowered by the person: leaving the ticket's
    /// page, or any prompt reaching its claude.
    ///
    /// A TOML table, so it sits with the tables — after `workspace` (a
    /// scalar) and before `[[tags]]`; a scalar serialized after it errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raised: Option<Raised>,
    /// Most recent completed column stay longer than one minute. Short visits
    /// leave it intact. Optional for old tickets; a table after all scalars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_column: Option<ColumnStay>,
    /// A ticket a paired browser filed, picked up at the desk (T-497): the
    /// first time its page was opened here, or an agent first started on it.
    /// The browser that filed it turns its ticks teal. Only a phone's ticket
    /// ([`Ticket::from_phone`]) gets one, once, and a copy does not carry it.
    /// A TOML table, with the tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<PickedUp>,
    /// Tags, at most one per group (a group is an axis: kind, environment…).
    /// Must stay after every scalar — this serializes as `[[tags]]`, an array
    /// of tables, and a scalar after a table errors. Tables may follow tables,
    /// so it sits between `workspace` (a scalar) and `[archived]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagRef>,
    /// The ticket's notes, creation order; `notes[0]` IS the description.
    /// Metadata only — the body is a file, `notes/<ULID>.md` beside
    /// `ticket.toml`, and never rides the snapshot. Another array of tables,
    /// so it sits after `tags` and before `[archived]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteMeta>,
    /// Archival is a field, not a directory move (13 §data-model) — the ticket
    /// keeps its column and order, so restore is exact. Must stay last: a TOML
    /// table; any scalar serialized after it errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<Archived>,
}

/// The `[picked]` table on a phone's ticket (T-497): when it was picked up,
/// and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickedUp {
    /// Same clock as `created_at` (`@<unix secs>`).
    pub at: String,
    /// [`PICKED_AT_DESK`] or [`PICKED_BY_AGENT`]. A word, never an enum: the
    /// ticket rides the TUI's snapshot, and a variant an older client does
    /// not know would fail the whole board.
    pub by: String,
}

/// Its page was opened in the TUI.
pub const PICKED_AT_DESK: &str = "desk";
/// An agent started on it.
pub const PICKED_BY_AGENT: &str = "agent";

/// A completed stay, frozen at departure rather than counting up in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnStay {
    pub column: String,
    pub seconds: u64,
}

/// What a TICKET wears: a pointer into the registry by (group, name).
///
/// Deliberately no colour here. Colour lives on the registry entry, so
/// recolouring a tag repaints every card at once instead of leaving 40
/// tickets holding a stale copy.
///
/// `group` is a plain `u8`, never an enum: an unknown enum variant from a
/// newer daemon fails the WHOLE `Response::Board` deserialize and the client
/// drops the line (the same reasoning that makes `Notice.kind` and
/// `WorktreeItem.status` `String`s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRef {
    pub name: String,
    /// The axis this tag belongs to — the digit that reaches it. 1–9, 0 = 10.
    pub group: u8,
}

/// One note on a ticket: who wrote it and when, and what to call it. The
/// body is the file; this is everything a rail row, an agent listing or a
/// status line needs WITHOUT reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteMeta {
    /// Minted by the daemon; the file is `notes/<id>.md`. Never chosen by
    /// the user or the agent (docs/15 §4.7: a name is a traversal primitive).
    pub id: ulid::Ulid,
    /// The body's first non-blank line, `#`s stripped, capped — computed by
    /// the daemon on every write ([`note_name`]).
    #[serde(default)]
    pub name: String,
    /// Bumped on every write. `edited_at` is `@<secs>`, and two writes in one
    /// second look identical; readers key their caches on `(id, rev)`.
    #[serde(default)]
    pub rev: u64,
    /// Same clock as `created_at` on the ticket (`@<unix secs>`).
    pub created_at: String,
    /// `local` for a person at the TUI, `agent:<session-uuid>` for an agent
    /// ([`crate::Principal::note_author`]).
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub edited_at: String,
    #[serde(default)]
    pub edited_by: String,
}

/// A REGISTRY entry: the vocabulary, and what each name looks like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub group: u8,
    /// Index into the tag tint ramp, chosen by the user (Tab cycles it).
    /// `None` means "never chosen" and falls back to a hash of the name, so a
    /// tag has a stable colour from the moment it exists without anyone
    /// having to pick one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
}

/// How many tints the tag ramp offers. Mirrors `mesimon-tui`'s `theme::PIPS`;
/// core cannot see the theme, and the colour index is stored here, so the
/// modulus has to live on both sides. `tag_tints_agree` (in `theme.rs`, the
/// side that CAN see both) pins them together.
///
/// Ten to match `MAX_TAGS_PER_GROUP`, so one axis can be entirely
/// colour-distinct. Raising it remaps every tag that never had a colour
/// picked — `default_tint` is a hash modulo this — which is the deliberate
/// cost of the change, paid once.
pub const TAG_TINTS: u8 = 10;

/// Most tags one axis may hold. A group is still meant to be a readable set
/// rather than a list, but five was too tight for a real vocabulary (author
/// 2026-09-01), so the cap is the same ten the groups themselves run to. The
/// picker row no longer has to fit the whole axis to stay usable: it windows
/// around the cursor cell (`ui/tagpicker.rs::window`).
pub const MAX_TAGS_PER_GROUP: usize = 10;

/// The colour a name falls back to when nobody has picked one: stable across
/// machines and screenshots, never the order it was created in.
pub fn default_tint(name: &str) -> u8 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h % TAG_TINTS as u64) as u8
}

impl Tag {
    /// The tint to paint this tag with.
    pub fn tint(&self) -> u8 {
        self.color.unwrap_or_else(|| default_tint(&self.name)) % TAG_TINTS
    }
}

/// The longest a tag name may be, in bytes (13 §data-model). Enforced at the
/// daemon boundary by `sanitize_tag`, not here.
pub const TAG_MAX_BYTES: usize = 24;

/// The longest a ticket title may be, in bytes. A title is one line on a
/// card, and past a few hundred cells nothing ever draws the rest — but a
/// paste can land a whole document in the field, and the slugger, the
/// activity feed and every card row would carry it forever. Large on purpose
/// (a long sentence in a four-byte script still fits), never infinite. The
/// composer's field mirrors it; the daemon's `sanitize_title` is the bound.
pub const TITLE_MAX_BYTES: usize = 2048;

/// The daemon-side boundary for a ticket title: user text on a card row,
/// scrubbed of what would break the row and capped at [`TITLE_MAX_BYTES`],
/// never split mid-character. Only ever removes.
pub fn sanitize_title(raw: &str) -> String {
    use crate::text::{cap_bytes, scrub_cells};
    cap_bytes(&scrub_cells(raw, false), TITLE_MAX_BYTES).to_string()
}

/// Whether a STORED title is one Claude Code's box would read as a shell
/// line (T-876): a start types the title, trimmed, into the empty box, and
/// a brief leads with it (`Daemon::launch_words`), where a first `!` is the
/// box's bash mode and the rest of the title runs as a command. Judged on
/// the form the board keeps, never the raw one a request carried:
/// `sanitize_title` removes a format hazard and turns a tab into a blank,
/// so `\u{200B}!rm` stored is `!rm`, and ` !rm` stored is typed as `!rm`.
pub fn shell_title(stored: &str) -> bool {
    stored.trim_start().starts_with('!')
}

/// The longest a note may be, in bytes. A note is a markdown file the
/// ticket page renders and an agent reads whole through one tool call; a
/// document past this belongs in the repo, not on a card. Bounded here and
/// enforced by `sanitize_note` at the daemon boundary so no client lifts it.
pub const NOTE_MAX_BYTES: usize = 32 * 1024;

/// The longest a note's NAME (its first line) may be, in bytes.
pub const NOTE_NAME_MAX_BYTES: usize = 80;

/// The daemon-side boundary for a note body: user or agent text headed for
/// cells (the ticket page) and for another process (an agent's `read_note`).
/// `scrub_cells` with newlines KEPT — block structure is nothing else — so
/// what survives is a subsequence of what was written: control characters
/// and the drawn-structure range go, `\t` becomes a space (fenced code with
/// tabs is the known cost), and the tail past [`NOTE_MAX_BYTES`] is cut on a
/// character boundary.
pub fn sanitize_note(raw: &str) -> String {
    use crate::text::{cap_bytes, scrub_cells};
    cap_bytes(&scrub_cells(raw, true), NOTE_MAX_BYTES).to_string()
}

/// Why a note write is refused for its size, or `None` when it fits. A write
/// past [`NOTE_MAX_BYTES`] is REFUSED, never cut (T-328): `sanitize_note`'s
/// cap is a floor under a client that lied about its length, not a policy a
/// caller may lean on, because a document whose tail was dropped on a success
/// receipt is a document nobody knows is incomplete. Measured on the bytes as
/// submitted — what the caller sent is what the message names — and a
/// multi-byte script counts in bytes, not characters, like the file it
/// becomes. The words carry both numbers so a client can split by them.
pub fn note_size_error(raw: &str) -> Option<String> {
    let bytes = raw.len();
    (bytes > NOTE_MAX_BYTES).then(|| {
        format!(
            "note is {bytes} bytes; the limit is {NOTE_MAX_BYTES} bytes ({} KiB). \
             Split it into parts, or keep a document this long in the repo and link it",
            NOTE_MAX_BYTES / 1024
        )
    })
}

/// What a note is called: its first non-blank line with any leading `#`
/// heading marks stripped, capped at [`NOTE_NAME_MAX_BYTES`]. A note has no
/// separate title field on purpose — the file is the whole record, and the
/// name is derived from it on every write so it cannot drift.
pub fn note_name(text: &str) -> String {
    use crate::text::cap_bytes;
    let line = text
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("(empty)");
    cap_bytes(line, NOTE_NAME_MAX_BYTES).trim_end().to_string()
}

/// Strip what a card row must never carry, then bound the length.
///
/// A tag name is user text rendered on a card row, so it runs the same
/// gauntlet as transcript peek text (`tui/src/peek.rs::sanitize`): control
/// chars, the drawn-structure range 0x2500–0x259F that the L1 law bans board-
/// wide, and the invisible width hazards — VS15/VS16 (U+FE0F turns a narrow
/// symbol into a two-cell emoji that `unicode-width` still counts as one),
/// ZWJ and the other zero-width format chars, and the combining keycap. A
/// terminal-vs-unicode-width disagreement on a card row shifts every later
/// cell one column right and strands a `selected_bg` cell past the card edge
/// that the diff never repaints.
///
/// One step along an axis: what a ticket wearing `current` should wear after
/// the next press of that group's digit. `names` is the registry's vocabulary
/// for the axis, in registry order ([`Board::group_tags`]).
///
/// The ladder is `none → first → … → last → none`, NOT a pure wrap. Off the
/// end is untagged on purpose: one finger has to be able to reach every value
/// the axis can hold, and "no tag" is one of them — a wrapping cycle can put
/// a tag on a card but never take the last one off, which would leave `^t`
/// the only way to undo a keystroke.
///
/// A `current` the registry does not know (a rename that raced the press, a
/// state file restored by hand) restarts the cycle rather than sticking on a
/// name nothing can reach. An empty `names` clears, which is the right answer
/// for a ticket wearing a tag from a vocabulary that no longer exists.
pub fn cycle_tag(names: &[&str], current: Option<&str>) -> Option<String> {
    let Some(cur) = current else {
        return names.first().map(|n| (*n).to_string());
    };
    match names.iter().position(|n| *n == cur) {
        Some(i) if i + 1 < names.len() => Some(names[i + 1].to_string()),
        Some(_) => None,
        None => names.first().map(|n| (*n).to_string()),
    }
}

/// Returns `None` for a name that is empty once sanitized — there is no such
/// thing as a blank tag.
pub fn sanitize_tag(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    nonblank(cap_bytes(&scrub_cells(raw, false), TAG_MAX_BYTES))
}

/// The longest a column name may be, in bytes (T-117). The header draws it
/// uppercased in a column no narrower than 26 cells, and a spine spells it
/// one letter per row; a tag's bound is the right size.
pub const COLUMN_NAME_MAX_BYTES: usize = 24;

/// The daemon-side boundary for a column name: `sanitize_tag`'s rule.
pub fn sanitize_column_name(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    nonblank(cap_bytes(&scrub_cells(raw, false), COLUMN_NAME_MAX_BYTES))
}

/// The longest a column's description may be, in bytes (T-467). A sentence
/// or two: it is one line in the column dialog, and every agent's
/// `get_ticket` carries every column's.
pub const COLUMN_DESCRIPTION_MAX_BYTES: usize = 300;

/// The daemon-side boundary for a column's description: drawn in the dialog
/// and read by agents, so `scrub_cells` on one line, which also removes
/// everything `scrub_text` does. `None` for one that is blank once scrubbed.
pub fn sanitize_column_description(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    nonblank(cap_bytes(&scrub_cells(raw, false), COLUMN_DESCRIPTION_MAX_BYTES))
}

/// An environment's name as a column waits for it (T-739): one line, as
/// the forge spells it, `forge::NAME_MAX_BYTES` at most.
pub fn sanitize_environment(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    let flat = scrub_cells(raw, false).split_whitespace().collect::<Vec<_>>().join(" ");
    nonblank(cap_bytes(&flat, crate::forge::NAME_MAX_BYTES))
}

/// The longest a raised hand's reason may be, in bytes (T-107). One line on
/// a card, cut to its column, and read whole on the board's hand row (T-888),
/// which wraps it: a short paragraph, since the mark is a POINTER and the
/// transcript is the record. 160 until T-888, which cut real asks mid-clause.
pub const RAISE_REASON_MAX_BYTES: usize = 400;

/// The daemon-side boundary for a raised hand's reason: agent text on a card
/// row, `sanitize_title`'s rule at a card row's size. Only ever removes.
/// `None` for a reason that is blank once scrubbed — a hand with nothing to
/// say is the bare `!` this tool exists to improve on.
pub fn sanitize_reason(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_cells};
    nonblank(cap_bytes(&scrub_cells(raw, false), RAISE_REASON_MAX_BYTES))
}

/// The `[raised]` table on a ticket (T-107): an agent asked for a person.
///
/// Presence IS the mark — the card lights, `needs_you_count` includes it,
/// the merge train leaves the ticket alone — and it comes off whole, because
/// the words are a pointer at a conversation that still holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raised {
    /// When the hand went up, same clock as `created_at` (`@<unix secs>`).
    pub at: String,
    /// Who asked, in [`crate::Principal::note_author`]'s words — always
    /// `agent:<session-uuid>` today, since no person's gesture raises one.
    #[serde(default)]
    pub by: String,
    /// One line, why. Sanitized and capped by [`sanitize_reason`], and never
    /// empty: a hand is raised WITH words or not at all.
    pub reason: String,
}

/// The `[archived]` table on a ticket. Presence = off the board.
///
/// A SNOOZE is an archive with a deadline (T-74): `until` set means the
/// daemon's tick wheel restores the ticket to the board when the clock
/// passes it, at the top of its column. Everything an archive already gets —
/// hidden by `Board::column_tickets`, refused by `place_ticket` and the
/// spawns, listed in the ARCHIVED dialog, restored by `a` — a snooze gets
/// for free, and a restore by hand simply cancels the snooze.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    /// Same clock as `created_at` (`@<unix secs>`).
    pub at: String,
    /// Actor. v0.1 has no user@host plumbing — always "local" (STALE-MAP).
    #[serde(default)]
    pub by: String,
    /// The wake deadline, same clock, when this archive is a snooze.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Raise `needs you` on the card when the snooze wakes (`Ticket::woke_at`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_you: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// `@<unix secs>` → seconds. The one parser for the ticket's own stamps.
pub fn stamp_secs(stamp: &str) -> Option<u64> {
    stamp.strip_prefix('@')?.parse().ok()
}

/// M4 (supersedes D25's column-only enum): how a ticket's sessions get a cwd.
/// The corpus's fourth value `none` is folded into "worktree, not yet provisioned" —
/// provisioning is lazy (first spawn), so an idea-card has no git identity anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStrategy {
    /// Own worktree at the state-dir root, branch `msmn/<KEY>-<slug>`.
    Worktree,
    /// cwd = the main checkout (pre-M4 behavior).
    SharedCheckout,
    /// Bound to a branch/worktree mesimon did not create; teardown keeps everything.
    AdoptExisting,
}

impl WorkspaceStrategy {
    pub fn word(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::SharedCheckout => "shared checkout",
            Self::AdoptExisting => "adopt",
        }
    }
}

/// What a ticket nothing stamped works in. A column's `workspace` setting
/// defaults a ticket CREATED in it by stamping the ticket field at mint
/// (T-117), and the board's (`Board::workspace`, T-816) stamps one the
/// column left alone, so this is the answer for a ticket minted before
/// either.
pub const DEFAULT_WORKSPACE: WorkspaceStrategy = WorkspaceStrategy::SharedCheckout;

impl Ticket {
    /// Import provenance can only narrow the stored execution policy. This also
    /// keeps a partial or hand-edited import from silently enabling automation.
    pub fn effective_execution_policy(&self) -> ExecutionPolicy {
        if self.import_origin.is_some() {
            ExecutionPolicy::OwnerOnly
        } else {
            self.execution_policy
        }
    }

    /// An agent minted this ticket (`created_by` is `agent:<uuid>`). A person's
    /// ticket and a pre-field one both answer no: the page says who filed a
    /// ticket only when it was not the person reading it.
    pub fn agent_created(&self) -> bool {
        self.created_by.starts_with("agent:")
    }

    /// The stamp the board's age slot counts from: when the ticket entered
    /// its current column, or its creation where no move has stamped it yet.
    pub fn column_since(&self) -> &str {
        self.entered_at.as_deref().unwrap_or(&self.created_at)
    }

    /// Remember a departure before changing the column or its arrival stamp.
    /// The caller supplies the clock; malformed or future stamps add no history.
    pub fn remember_column_stay(&mut self, departed_at: &str) {
        let seconds = stamp_secs(departed_at)
            .zip(stamp_secs(self.column_since()))
            .and_then(|(end, start)| end.checked_sub(start));
        if let Some(seconds) = seconds.filter(|seconds| *seconds > 60) {
            self.previous_column = Some(ColumnStay { column: self.column.clone(), seconds });
        }
    }

    /// Layered resolution: ticket field (a column's default is stamped here
    /// at mint, as is the board's own since T-816), else the shared checkout.
    pub fn workspace_strategy(&self) -> WorkspaceStrategy {
        self.workspace.unwrap_or(DEFAULT_WORKSPACE)
    }

    pub fn is_archived(&self) -> bool {
        self.archived.is_some()
    }

    /// The snooze deadline in unix seconds, when this archive is a snooze.
    pub fn snooze_until_secs(&self) -> Option<u64> {
        self.archived.as_ref()?.until.as_deref().and_then(stamp_secs)
    }

    /// Returned from a snooze and not yet seen: wears needs-you on its own.
    pub fn is_woke(&self) -> bool {
        self.woke_at.is_some()
    }

    /// An agent asked for a person and nobody has answered yet (T-107): the
    /// other ticket-level way to wear needs-you.
    pub fn hand_raised(&self) -> bool {
        self.raised.is_some()
    }

    /// Filed by a paired browser (T-497): its author is a device.
    pub fn from_phone(&self) -> bool {
        self.created_by.starts_with("device:")
    }

    /// A phone's ticket nobody has picked up yet: opening its page, or an
    /// agent starting on it, is news for the browser that filed it.
    pub fn awaits_pickup(&self) -> bool {
        self.from_phone() && self.picked.is_none()
    }

    /// This ticket's tag on axis `group`, if it wears one. At most one per
    /// group by construction — `set_tag` replaces rather than appends.
    pub fn tag_in(&self, group: u8) -> Option<&TagRef> {
        self.tags.iter().find(|t| t.group == group)
    }

    pub fn wears(&self, group: u8, name: &str) -> bool {
        self.tag_in(group).is_some_and(|t| t.name == name)
    }

    /// `notes[0]`: the description, when the ticket has one.
    pub fn description(&self) -> Option<&NoteMeta> {
        self.notes.first()
    }

    /// One note by id, on this ticket only.
    pub fn note(&self, id: ulid::Ulid) -> Option<&NoteMeta> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Set (or with `None`, clear) this ticket's tag on axis `group`.
    pub fn set_tag(&mut self, group: u8, name: Option<String>) {
        self.tags.retain(|t| t.group != group);
        if let Some(name) = name {
            self.tags.push(TagRef { name, group });
        }
        // Stable on disk and on the wire: a card's pips must not reorder
        // because an unrelated group changed.
        self.tags.sort_by_key(|t| t.group);
    }
}

/// How far the crown acts for the person on the agents it started (T-610):
/// one setting over what were two switches, `crown_sends` (T-550) and
/// `crown_answers` (T-569, T-582). `Autonomous` delivers its `ask_agent`
/// words by the queue without a person's `^y`, answers their questions and
/// accepts their plans; `Supervised` holds every ask for `^y` and leaves
/// every question and plan to the person. An agent a person started is the
/// person's under either mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrownMode {
    #[default]
    Autonomous,
    Supervised,
}

impl CrownMode {
    /// The crown's `ask_agent` words go by the queue (was `crown_sends`).
    pub fn sends(self) -> bool {
        self == Self::Autonomous
    }

    /// The crown answers questions and accepts plans (was `crown_answers`).
    pub fn answers(self) -> bool {
        self == Self::Autonomous
    }

    /// Is a session stopped in `state` the crown's to answer, under this
    /// mode, when the crown started it (T-569, T-582)? An agent's question
    /// or plan: a claude's, and since T-846 a codex's, whose request the
    /// runtime carries and whose dialog the board walks with the same keys.
    /// A secret, a form or a permission is never the crown's. The crown's
    /// wake and the person's banner (T-678) read this one rule.
    pub fn takes_stop(self, kind: SessionKind, state: &SessionState) -> bool {
        self.answers()
            && kind.is_agent()
            && matches!(
                state,
                SessionState::RequiresAction { reason: Reason::Question | Reason::Plan }
            )
    }

    /// The crown merges a worker's branch where the train will not
    /// (`merge_ticket`, T-613); supervised keeps the merge a person's.
    pub fn merges(self) -> bool {
        self == Self::Autonomous
    }

    /// The setting's value in the words the row and `doctor` show.
    pub fn word(self) -> &'static str {
        match self {
            Self::Autonomous => "autonomous",
            Self::Supervised => "supervised",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Autonomous => Self::Supervised,
            Self::Supervised => Self::Autonomous,
        }
    }
}

/// Where a board's worktree branches land (T-810): one workflow per board,
/// never both. `Main`, every board's default, is the board as before the
/// forge integration (T-739): the train and `m` fast-forward a branch into
/// the local base, and nothing reads a forge. `PullRequests`: a branch lands
/// through its PR, merged on the forge; the board reads the forge, never
/// fast-forwards a worktree branch, and the train asks only the rebase a PR
/// conflict needs. A shared-checkout ticket is unchanged under either: its
/// commits are on the base the moment they exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeInto {
    #[default]
    Main,
    PullRequests,
}

impl MergeInto {
    /// Branches land through pull requests: the forge is read, and merges.
    pub fn pull_requests(self) -> bool {
        self == Self::PullRequests
    }

    /// The setting's value in the words the row and `doctor` show.
    pub fn word(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::PullRequests => "pull requests",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Main => Self::PullRequests,
            Self::PullRequests => Self::Main,
        }
    }
}

/// Default timing for a person's follow-up composer, stored per board.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpMode {
    #[default]
    Queue,
    Steer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub columns: Vec<Column>,
    pub tickets: Vec<Ticket>,
    /// Marked on every `&mut` (T-839), so `sessions.json` is serialised
    /// only after something may have changed.
    pub sessions: crate::tracked::Tracked<Vec<SessionRecord>>,
    /// Provider captured when a new agent start is accepted. Existing
    /// sessions, including sleeping ones, retain their persisted kind.
    #[serde(default)]
    pub agent_provider: AgentProvider,
    /// The spawn budget (T-412): how many agent seats the crown's agent may
    /// have started at once, counted over `SessionRecord::started_by` while
    /// each seat is held awake (a sleeping record frees it, T-541) on a
    /// ticket that is not archived. Zero means the crown starts nothing. D10's money
    /// fire — an agent that starts agents — is bounded by this number and
    /// by the one-level rule on `started_by`, never by trust.
    #[serde(default = "default_crown_budget")]
    pub crown_budget: u8,
    /// The crown's mode (T-610): under `Autonomous`, the default, its
    /// `ask_agent` words for an agent it STARTED (`started_by`, any
    /// crown's) go by the queue when that agent is idle instead of waiting
    /// on a person's `^y` (T-550), `answer_agent` may answer the
    /// `AskUserQuestion` such an agent stopped on (T-569) and `accept_plan`
    /// its plan (T-582), and either stop wakes the crown. `Supervised`
    /// keeps all three for the person. An agent a person started is held
    /// for them whatever this says, and a wake on the crown's words is a
    /// seat in `crown_budget`. The two switches it replaced
    /// (`crown_sends`, `crown_answers`) are ignored on read: a board that
    /// had them comes up autonomous, the author's call for every board.
    #[serde(default)]
    pub crown_mode: CrownMode,
    /// Where the board's worktree branches land (T-810): into main by the
    /// train and `m`, or through pull requests merged on the forge. Main by
    /// default, and the forge is read only under pull requests. Nothing
    /// shipped mixed the two, so no migration and no schema bump.
    #[serde(default)]
    pub merge_into: MergeInto,
    /// The crown archives (T-590): `archive_ticket`, archive and restore
    /// alike, is refused while this is off; nothing else of the crown's
    /// changes. Taking a card off the board is
    /// a person's gesture unless the person says otherwise, per board (the
    /// author: "gate crown ability to archive with a setting opt in"). OFF
    /// by default. A build that drops it gives the crown its unconditional
    /// archive back, which is that build's own behaviour, not a misread.
    #[serde(default)]
    pub crown_archives: bool,
    /// The crown watches (T-712): `watch_ticket`, which has the board wake
    /// the crown for a ticket a person started as it does for one the crown
    /// started, is refused while this is off, and every watch ends when it
    /// is turned off. ON by default (T-732, the author: "crown can watch
    /// tickets on by default"): a watch is asked for by a person's words to
    /// the crown, it sends the watched agent nothing, and the person still
    /// turns it off per board. An absent key reads on, the `mcp_tools`
    /// shape; a board that already wrote `false` keeps it (the author's call:
    /// new boards only). A build that drops it has no watch to give back,
    /// and one that reads it (T-712's) reads only what the file says, so no
    /// schema bump.
    #[serde(default = "yes")]
    pub crown_watches: bool,
    /// The crown reads a session's context fill (T-969): `get_ticket`'s
    /// `context` on the crown's reads, absent while this is off. ON by
    /// default (the author, 2026-10-25): a fact the daemon already holds,
    /// read off a transcript, that sends nothing anywhere. An absent key
    /// reads on, so a person's off is saved at `COLUMNS_SCHEMA` 9, which a
    /// build before it refuses and bars rather than dropping the off for
    /// this build to read on again (round 1).
    #[serde(default = "yes")]
    pub crown_context: bool,
    /// The crown reads what a ticket has cost (T-969): `get_ticket`'s
    /// `cost`, the card's estimate, on the crown's reads. OFF by default
    /// (the author, 2026-10-25); an absent key reads off. It rides the v9
    /// bump `crown_context` needs, though an older build dropping it would
    /// only lose an opt-in.
    #[serde(default)]
    pub crown_cost: bool,
    /// The crown samples an agent it started (T-973): `sample_agent`, a
    /// bounded read of its turn, its transcript's tail and its pane's last
    /// rows, refused while this is off. ON by default (the person, gap row
    /// 55 on T-925): the agent is sent nothing. An absent key reads on, so a
    /// person's off is saved at `COLUMNS_SCHEMA` 9 with `crown_context`'s,
    /// which no released build carried.
    #[serde(default = "yes")]
    pub crown_samples: bool,
    /// Reviewed pull requests (T-859): a pull request someone else opened is
    /// a ticket's when its agent acted on it through the forge's CLI (a
    /// review, a comment, an approval, a checkout) or named it in a note's
    /// `Summary`, read by itself since no list of the account's own holds
    /// it. ON by default (the author: "gate in settings on by default"):
    /// shown and followed, never judged, so it costs the ticket nothing but
    /// a read a minute. OFF, a ticket's pull requests are the account's own
    /// alone, as before. An absent key reads on; a build that drops it
    /// reads every PR as the ticket's own, which only holds a merge where
    /// this build would not, so no schema bump.
    #[serde(default = "yes")]
    pub reviewed_prs: bool,
    /// Counter feeding short keys (T-1, T-2, …).
    pub next_key: u64,
    /// The tag registry: the vocabulary each axis offers, in the order it was
    /// created. Board-level and PERSISTED — a tag outlives the tickets that
    /// wear it, so untagging the last ticket does not silently retire the tag
    /// and a cycle keeps its shape.
    ///
    /// A board that never had a vocabulary is given `STARTER_TAGS` once, on
    /// its first load (`seed_starter_tags`); after that the registry grows
    /// only when a name is typed. "Create on the fly" is about not having to
    /// set the board up before using it, NOT about deriving the list from the
    /// tickets.
    #[serde(default)]
    pub tags: Vec<Tag>,
    /// Whether the starter offer has been made: true once `STARTER_TAGS` were
    /// written, or once the board was seen with a vocabulary of its own.
    /// Persisted, so forgetting every starter is not answered with the three
    /// coming back on the next daemon start.
    #[serde(default)]
    pub tags_seeded: bool,
    /// Whether sessions mesimon spawns carry the MCP tool surface at all
    /// (T-217). On by default, and per REPO rather than per machine: the
    /// question "may agents on this board see their ticket" is a property of
    /// the board. Off means `claude_argv` omits `--mcp-config` entirely and a
    /// wake drops it from the argv it replays, so the only way back in is a
    /// spawn or a wake — a live pane keeps what it was born with.
    ///
    /// Its default is `true`, which is why the file's field carries an
    /// explicit `#[serde(default = ..)]` rather than `bool`'s own `false`.
    #[serde(default = "yes")]
    pub mcp_tools: bool,
    /// The agent-brief offer was declined for good (T-217, re-aimed T-224). A
    /// stamp, in the grain of `tags_seeded`: the offer is a header chip, so
    /// without somewhere to record "never" it would stand in front of the menu
    /// forever. `mesimon doctor` still prints the brief and the Settings row
    /// still turns it on — that is the way back, and why "never" here can be
    /// total. The key keeps T-217's name on disk; renaming a persisted stamp
    /// would re-offer to everyone who had answered.
    #[serde(default)]
    pub claude_md_ignored: bool,
    /// Sessions mesimon starts carry `brief::TEXT` in their system prompt
    /// (`brief::FLAG`, T-224). ON by default since T-816 (the author: "it
    /// should be on by default"): the README shows the exact text (T-850),
    /// `doctor` prints it, and a person turns it off in Settings, which
    /// stamps `claude_md_ignored` so the store keeps it off (`store.rs`
    /// reads an off on a board nobody answered as on). README promise 3 names
    /// it as the one paragraph mesimon adds. Per REPO like `mcp_tools`, and
    /// honoured only while the tools are on (the sentence names a tool).
    #[serde(default = "yes")]
    pub system_prompt: bool,
    /// Where a ticket lands when nobody chose a column for it (T-279): an
    /// agent's `create_ticket` with `column` omitted. A column NAME, the
    /// foreign key every other cross-column reference is, so `rename_column`
    /// carries it and `delete_column` clears it; `None` — and a name the board
    /// no longer has — means the first column, which is what every board did
    /// before the field. Per repo, in `columns.toml`, chosen from the Settings
    /// submenu. A plain serde default with no schema bump: a build that drops
    /// it lands the agent's card in the first column, the old behaviour, and
    /// hands nobody anything wider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_column: Option<String>,
    #[serde(default)]
    pub follow_up_mode: FollowUpMode,
    /// The four sentences mesimon itself types into an agent's box, as this
    /// board would have them (T-353): the rebase ask, the merged notice and
    /// the note nudge. Empty — every board before the field, and every board
    /// nobody has edited — means mesimon's own words, which live in the
    /// BINARY (`prompts::AgentPrompt::default_text`) and never on disk. Per
    /// repo like `system_prompt` beside it, because what to say to an agent
    /// about a rebase is a property of the work, not of the machine.
    ///
    /// A plain serde default with no schema bump: a build that drops a
    /// custom template sends mesimon's own sentence instead, which is the
    /// text every board sent before the field existed.
    #[serde(default, skip_serializing_if = "crate::prompts::PromptSet::is_default")]
    pub prompts: crate::prompts::PromptSet,
    /// The one ticket whose agent may edit OTHER tickets (T-411, "the
    /// crown"). Every other agent's tools reach its own card only; the
    /// crowned ticket's agent may pass a ticket key to the same tools and
    /// three more. Granted by a person from the board (`CrownTicket`), never
    /// by a tool — an agent that could crown itself would be writing its own
    /// tier — and `Option` so exactly one holds it by type. A ULID, never a
    /// key. Per repo in `columns.toml` with a plain serde default and no
    /// schema bump: a build that drops it narrows what every agent gets,
    /// which is the safe direction.
    ///
    /// The seat is the mesimatron's: a later daemon rule mints, crowns and
    /// starts the ticket itself, and nothing about the authority changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown: Option<ulid::Ulid>,
    /// This board's agent tiers (T-443): its own, and its overrides of the
    /// machine's by id (`tier::Book`). The machine's live in `tiers.toml`
    /// and ride the snapshot separately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<crate::tier::Tier>,
    /// This board's default tier; `None` inherits the machine's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tier: Option<String>,
    /// The board was just made and its person has not been through Setup
    /// yet (T-816): the TUI opens Setup over it once. Set only by the store
    /// on a repository with no `columns.toml`, so every board before the
    /// field, and every board a test builds, reads as set up. Cleared by
    /// `Command::ApplySetup`, the skip included.
    #[serde(default, skip_serializing_if = "is_false")]
    pub setup_pending: bool,
    /// Where a ticket works when neither its composer nor its column chose
    /// (T-816): stamped onto the ticket at mint, like a column's. `None` is
    /// the shared checkout, every board before Setup; Setup writes a
    /// worktree unless the person chose otherwise or there is no repository
    /// to cut one from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
}

/// The crown's spawn budget when the file says nothing (T-412): three
/// seats, enough for a coordinator to fan out a small batch and few enough
/// that a runaway costs three sessions, not a fleet.
pub const DEFAULT_CROWN_BUDGET: u8 = 3;

fn default_crown_budget() -> u8 {
    DEFAULT_CROWN_BUDGET
}

/// `Board::mcp_tools` defaults ON: a serde default
/// has to be a function, and this is the whole of it.
fn yes() -> bool {
    true
}

/// A session's state as ONE WORD for an agent reading another ticket
/// (T-411): what the card shows, never what the pane holds. `needs-you` is
/// the attention set, `working` a turn in flight, `idle` a turn ended,
/// `background` a parked lead with tasks still live.
pub fn agent_state_word(state: &SessionState) -> &'static str {
    match state {
        SessionState::Spawning => "starting",
        SessionState::Running => "working",
        SessionState::RequiresAction { .. } => "needs-you",
        SessionState::Idle { stop_reason: StopReason::EndTurn } => "idle",
        SessionState::Idle { stop_reason: StopReason::Interrupted } => "interrupted",
        SessionState::Idle { stop_reason: StopReason::Background | StopReason::Monitoring } => {
            "background"
        }
        SessionState::Idle { stop_reason: StopReason::Unknown } => "idle",
        SessionState::Sleeping => "sleeping",
        SessionState::Exited { .. } => "exited",
        SessionState::Failed { .. } => "failed",
        SessionState::Throttled => "throttled",
        SessionState::Unknown { .. } => "unknown",
    }
}

/// What a `failed` agent's turn ended on (T-936), in plain words for an
/// agent reading the ticket and for the crown's wake line:
/// `agent_state_word` says `failed`, and this says why.
pub fn agent_fail_word(reason: FailReason) -> &'static str {
    match reason {
        FailReason::Server => "server error",
        FailReason::InvalidRequest => "invalid request",
        FailReason::ModelNotFound => "model not found",
        FailReason::MaxOutputTokens => "max output tokens",
        FailReason::Unknown => "unknown",
    }
}

/// Why a `needs-you` agent stopped, as ONE WORD for an agent reading the
/// ticket (T-566): `agent_state_word`'s companion, lower-case and plain
/// where the card's `reason_word` is a badge (`INPUT`, `SETUP`). Both
/// spawn-time modals read `startup`, as they share a rank.
pub fn agent_reason_word(reason: Reason) -> &'static str {
    match reason {
        Reason::Permission => "permission",
        Reason::Secret => "secret",
        Reason::Question => "question",
        Reason::Plan => "plan",
        Reason::Elicitation => "elicitation",
        Reason::Auth => "auth",
        Reason::QuotaResume => "quota",
        Reason::Trust => "trust",
        Reason::StartupModal | Reason::ResumeDialog => "startup",
    }
}

/// Hand-written rather than derived, for one field: `mcp_tools` starts ON, and
/// `#[derive(Default)]` would start it off. A default `Board` is what a repo
/// with no `columns.toml` gets and what `store::load` falls back to when the
/// file cannot be read — an unreadable file must not read as "the user turned
/// the agent tools off".
impl Default for Board {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            tickets: Vec::new(),
            sessions: Vec::new().into(),
            agent_provider: AgentProvider::default(),
            crown_budget: DEFAULT_CROWN_BUDGET,
            crown_mode: CrownMode::default(),
            merge_into: MergeInto::default(),
            crown_archives: false,
            crown_watches: true,
            crown_context: true,
            crown_cost: false,
            crown_samples: true,
            reviewed_prs: true,
            next_key: 0,
            tags: Vec::new(),
            tags_seeded: false,
            mcp_tools: true,
            claude_md_ignored: false,
            system_prompt: true,
            default_column: None,
            follow_up_mode: FollowUpMode::default(),
            prompts: crate::prompts::PromptSet::default(),
            crown: None,
            tiers: Vec::new(),
            default_tier: None,
            setup_pending: false,
            workspace: None,
        }
    }
}

/// The vocabulary a board starts with, on group 1: three names most work
/// sorts itself into, each with a colour picked by hand (indices into the
/// tag ring, tuned on the shipped graphite/chalk ring: rose, green, blue).
/// Offered ONCE to a board with no tags at all (user 2026-09-04: "creating
/// first tag gets people overwhelmed"); a board that already has a
/// vocabulary never sees them, and a user who forgets them is not re-seeded.
pub const STARTER_TAGS: [(&str, u8); 3] = [("BUG", 0), ("FEATURE", 2), ("CHANGE", 6)];

/// The axis `STARTER_TAGS` land on.
pub const STARTER_GROUP: u8 = 1;

/// D33i: the shipped default template.
pub const DEFAULT_COLUMNS: [&str; 4] = ["TODO", "IN PROGRESS", "REVIEW", "DONE"];

/// What each template column DID before T-117 made the rules explicit — and
/// the ONE place a column name is read as a literal. `with_default_columns`
/// seeds a fresh board with it and the store's v3→v4 migration seeds an
/// existing one; after that every automation reads the ticket's column
/// through `Board::column`, so renaming TODO breaks nothing.
pub fn template_settings(name: &str) -> Option<ColumnSettings> {
    let d = ColumnSettings::default;
    Some(match name {
        "TODO" => ColumnSettings { on_working: Some("IN PROGRESS".into()), ..d() },
        "IN PROGRESS" => ColumnSettings {
            on_done: Some("REVIEW".into()),
            train: TrainReach::Rebase,
            refuses_new: true,
            ..d()
        },
        "REVIEW" => ColumnSettings {
            on_working: Some("IN PROGRESS".into()),
            on_merged: Some("DONE".into()),
            train: TrainReach::Merge,
            refuses_new: true,
            ..d()
        },
        "DONE" => ColumnSettings { requires_merge: true, reclaim: true, ..d() },
        _ => return None,
    })
}

/// The boards Setup offers (T-816): each its columns and their rules, so a
/// template's moves never depend on what its columns are called. `Kanban`
/// is the board every repository got before Setup, `DEFAULT_COLUMNS` under
/// `template_settings`; the two pull-request boards move cards on what the
/// forge reports (`arrives_on`), and the last one adds a production column
/// a person widens to more environments from its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Template {
    #[default]
    Kanban,
    PullRequests,
    PullRequestsProduction,
}

impl Template {
    pub const ALL: [Template; 3] =
        [Template::Kanban, Template::PullRequests, Template::PullRequestsProduction];

    /// The board's columns, first to last, with their settings. `noun` is
    /// the forge's own word for one, `PR` or `MR` (`forge::Forge::noun`),
    /// which names the column a ticket waits in while it is open.
    pub fn columns(self, noun: &str) -> Vec<(String, ColumnSettings)> {
        let d = ColumnSettings::default;
        if self == Template::Kanban {
            return DEFAULT_COLUMNS
                .iter()
                .map(|n| (n.to_string(), template_settings(n).unwrap_or_default()))
                .collect();
        }
        let open = format!("{noun} OPEN");
        let prod = self == Template::PullRequestsProduction;
        let mut out = vec![
            ("TODO".to_string(), ColumnSettings { on_working: Some("IN PROGRESS".into()), ..d() }),
            (
                "IN PROGRESS".to_string(),
                ColumnSettings { train: TrainReach::Rebase, refuses_new: true, ..d() },
            ),
            (
                open,
                ColumnSettings {
                    arrives_on: Arrival::PrOpened,
                    train: TrainReach::Rebase,
                    refuses_new: true,
                    ..d()
                },
            ),
            (
                "MERGED".to_string(),
                ColumnSettings {
                    arrives_on: Arrival::Merged,
                    requires_merge: true,
                    reclaim: !prod,
                    refuses_new: true,
                    ..d()
                },
            ),
        ];
        if prod {
            out.push((
                "PRODUCTION".to_string(),
                ColumnSettings {
                    arrives_on: Arrival::Deployed,
                    arrives_env: Some("production".into()),
                    requires_merge: true,
                    reclaim: true,
                    refuses_new: true,
                    ..d()
                },
            ));
        }
        out
    }

    /// Where its branches land: a pull-request board merges through them.
    pub fn merge_into(self) -> MergeInto {
        match self {
            Template::Kanban => MergeInto::Main,
            _ => MergeInto::PullRequests,
        }
    }
}

/// What every short key starts with: `T-12`. The mint, the on-disk recovery
/// of the counter and the link recogniser (`links.rs`) all read it here.
pub const KEY_PREFIX: &str = "T-";

impl Board {
    pub fn with_default_columns() -> Self {
        let mut b = Board::default();
        let mut prev = String::new();
        for name in DEFAULT_COLUMNS {
            let order = crate::fracindex::between(&prev, "");
            b.columns.push(Column::new(name, order.clone()));
            prev = order;
        }
        b.seed_template_settings();
        b
    }

    /// Replace the columns with `template`'s (T-816), for Setup on a board
    /// with no ticket yet: refused once one exists, because a ticket in a
    /// column the template lacks would be stranded.
    pub fn apply_template(&mut self, template: Template, noun: &str) -> Result<(), String> {
        if !self.tickets.is_empty() {
            return Err("this board already has tickets, so its columns stay".into());
        }
        let mut prev = String::new();
        self.columns = template
            .columns(noun)
            .into_iter()
            .map(|(name, settings)| {
                let order = crate::fracindex::between(&prev, "");
                prev = order.clone();
                Column { name, order, settings }
            })
            .collect();
        self.default_column = None;
        Ok(())
    }

    /// Give every column named like the template its template settings, then
    /// drop any `on_working`/`on_done`/`on_merged` naming a column this board
    /// does not have (a quarantine-rebuilt board may lack one). The v3→v4 road and the
    /// fresh board's; returns whether anything changed.
    pub fn seed_template_settings(&mut self) -> bool {
        let mut changed = false;
        for c in self.columns.iter_mut() {
            if let Some(s) = template_settings(&c.name) {
                if c.settings != s {
                    c.settings = s;
                    changed = true;
                }
            }
        }
        changed | !self.prune_dangling_refs().is_empty()
    }

    /// Clear every `on_working`/`on_done`/`on_merged` that names a column the
    /// board does not have. Returns `(column, field)` for each one cleared, so the
    /// caller can say so.
    pub fn prune_dangling_refs(&mut self) -> Vec<(String, &'static str)> {
        let names: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let mut out = Vec::new();
        if let Some(d) = self.default_column.take_if(|n| !names.contains(n)) {
            out.push((d, "default_column"));
        }
        for c in self.columns.iter_mut() {
            if c.settings.on_working.as_ref().is_some_and(|n| !names.contains(n)) {
                c.settings.on_working = None;
                out.push((c.name.clone(), "on_working"));
            }
            if c.settings.on_done.as_ref().is_some_and(|n| !names.contains(n)) {
                c.settings.on_done = None;
                out.push((c.name.clone(), "on_done"));
            }
            if c.settings.on_merged.as_ref().is_some_and(|n| !names.contains(n)) {
                c.settings.on_merged = None;
                out.push((c.name.clone(), "on_merged"));
            }
        }
        out
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The idle park that is due for `rec` now (T-543), as the automation
    /// rule that names it in the feed and its timeout: the ticket's column's
    /// `sleep_after_minutes` under `autosleep_due`. The column is the only
    /// timer since T-610 removed the board's `park_after_minutes`.
    pub fn idle_park_due(
        &self,
        rec: &SessionRecord,
        now: u64,
        minute_ms: u64,
    ) -> Option<(&'static str, u64)> {
        let timeout = |minutes: u32| u64::from(minutes).saturating_mul(minute_ms);
        let column = self
            .ticket(rec.ticket)
            .and_then(|t| self.column(&t.column))
            .map_or(0, |c| timeout(c.settings.sleep_after_minutes));
        rec.autosleep_due(now, column).then_some(("autosleep", column))
    }

    /// The column a ticket lands in when nothing named one (T-279): the
    /// chosen default while the board still has it, else the first column,
    /// either one through `filing_column` (T-764). `None` only on a board
    /// where no column takes new tickets.
    pub fn landing_column(&self) -> Option<String> {
        let chosen = match self.default_column.as_deref().and_then(|n| self.column(n)) {
            Some(d) => d.name.clone(),
            None => self.sorted_columns().first()?.name.clone(),
        };
        self.filing_column(&chosen)
    }

    /// Where a ticket minted from `name` goes (T-764): `name` itself unless
    /// it refuses new tickets, then the nearest column to its left that
    /// takes them, then the nearest to its right. `None` when no column
    /// does, or `name` is not on the board.
    pub fn filing_column(&self, name: &str) -> Option<String> {
        let cols = self.sorted_columns();
        let at = cols.iter().position(|c| c.name == name)?;
        let open = |c: &&&Column| !c.settings.refuses_new;
        cols.get(at)
            .filter(open)
            .or_else(|| cols[..at].iter().rev().find(open))
            .or_else(|| cols[at + 1..].iter().find(open))
            .map(|c| c.name.clone())
    }

    /// Why a ticket may not be filed in `name` (T-764), naming where `o`
    /// would file it: `None` where the column takes new tickets.
    pub fn filing_refusal(&self, name: &str) -> Option<String> {
        if !self.column(name)?.settings.refuses_new {
            return None;
        }
        Some(match self.filing_column(name) {
            Some(open) => format!("{name} takes no new tickets: file it in {open}"),
            None => format!("{name} takes no new tickets"),
        })
    }

    /// Choose the landing column, or `None` to go back to the first column.
    /// Refused for a name the board does not have: a dangling default would
    /// read as the first column and the Settings row would say otherwise.
    pub fn set_default_column(&mut self, name: Option<&str>) -> Result<(), String> {
        match name {
            Some(n) if self.column(n).is_none() => Err(format!("no such column: {n}")),
            Some(n) => {
                self.default_column = Some(n.to_string());
                Ok(())
            }
            None => {
                self.default_column = None;
                Ok(())
            }
        }
    }

    pub fn column_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| c.name == name)
    }

    /// A name already on the board, letter case aside: the header uppercases
    /// every name, so `todo` beside `TODO` would draw as two of one column.
    fn column_name_taken(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Add a column after `after` (the sorted successor), else at the end.
    pub fn add_column(&mut self, name: String, after: Option<&str>) -> Result<(), String> {
        if self.column_name_taken(&name) {
            return Err(format!("a column named {name} already exists"));
        }
        let sorted = self.sorted_columns();
        let order = match after {
            Some(a) => {
                let Some(i) = sorted.iter().position(|c| c.name == a) else {
                    return Err(format!("no such column: {a}"));
                };
                let next = sorted.get(i + 1).map(|c| c.order.as_str()).unwrap_or("");
                crate::fracindex::between(&sorted[i].order, next)
            }
            None => {
                crate::fracindex::between(sorted.last().map(|c| c.order.as_str()).unwrap_or(""), "")
            }
        };
        self.columns.push(Column::new(name, order));
        Ok(())
    }

    /// Rename a column, carrying every ticket in it — ARCHIVED ONES INCLUDED,
    /// since an archive keeps its column for the restore — and every other
    /// column's `on_working`/`on_done`/`on_merged` naming it. Returns the ticket ids
    /// that changed so the caller writes only those files.
    pub fn rename_column(&mut self, from: &str, to: &str) -> Result<Vec<ulid::Ulid>, String> {
        if self.column(from).is_none() {
            return Err(format!("no such column: {from}"));
        }
        if from != to && !(from.eq_ignore_ascii_case(to)) && self.column_name_taken(to) {
            return Err(format!("a column named {to} already exists"));
        }
        if from == to {
            return Ok(Vec::new());
        }
        for c in self.columns.iter_mut() {
            if c.name == from {
                c.name = to.to_string();
            }
            if c.settings.on_working.as_deref() == Some(from) {
                c.settings.on_working = Some(to.to_string());
            }
            if c.settings.on_done.as_deref() == Some(from) {
                c.settings.on_done = Some(to.to_string());
            }
            if c.settings.on_merged.as_deref() == Some(from) {
                c.settings.on_merged = Some(to.to_string());
            }
        }
        if self.default_column.as_deref() == Some(from) {
            self.default_column = Some(to.to_string());
        }
        let mut touched = Vec::new();
        for t in self.tickets.iter_mut() {
            if t.column == from {
                t.column = to.to_string();
                touched.push(t.id);
            }
        }
        Ok(touched)
    }

    /// Delete a column. Refused while any live ticket is in it — an archived
    /// one keeps its column string and the restore falls back to the first
    /// column — and refused for the last column. Clears every
    /// `on_working`/`on_done`/`on_merged` pointing at it; returns those, as
    /// `prune_dangling_refs` does.
    pub fn delete_column(&mut self, name: &str) -> Result<Vec<(String, &'static str)>, String> {
        if self.column(name).is_none() {
            return Err(format!("no such column: {name}"));
        }
        if self.columns.len() == 1 {
            return Err("the last column stays".into());
        }
        let live = self.column_tickets(name).len();
        if live > 0 {
            return Err(format!(
                "move {} first",
                if live == 1 { "its ticket".to_string() } else { format!("its {live} tickets") }
            ));
        }
        self.columns.retain(|c| c.name != name);
        Ok(self.prune_dangling_refs())
    }

    /// Move a column before `before` in board order, else to the end.
    pub fn reorder_column(&mut self, name: &str, before: Option<&str>) -> Result<(), String> {
        if self.column(name).is_none() {
            return Err(format!("no such column: {name}"));
        }
        if before == Some(name) {
            return Ok(());
        }
        let others: Vec<(String, String)> = self
            .sorted_columns()
            .iter()
            .filter(|c| c.name != name)
            .map(|c| (c.name.clone(), c.order.clone()))
            .collect();
        let order = match before {
            Some(b) => {
                let Some(i) = others.iter().position(|(n, _)| n == b) else {
                    return Err(format!("no such column: {b}"));
                };
                let prev = if i == 0 { "" } else { others[i - 1].1.as_str() };
                crate::fracindex::between(prev, &others[i].1)
            }
            None => {
                crate::fracindex::between(others.last().map(|(_, o)| o.as_str()).unwrap_or(""), "")
            }
        };
        if let Some(c) = self.column_mut(name) {
            c.order = order;
        }
        Ok(())
    }

    /// Replace a column's settings whole. `on_working`/`on_done`/`on_merged` must name a
    /// column the board has, other than this one — a self-loop is refused,
    /// not resolved.
    pub fn set_column_settings(&mut self, name: &str, s: ColumnSettings) -> Result<(), String> {
        if self.column(name).is_none() {
            return Err(format!("no such column: {name}"));
        }
        for (field, target) in
            [("working", &s.on_working), ("done", &s.on_done), ("merged", &s.on_merged)]
        {
            if let Some(t) = target {
                if t == name {
                    return Err(format!("when {field}: a column cannot move a ticket to itself"));
                }
                if self.column(t).is_none() {
                    return Err(format!("when {field}: no such column: {t}"));
                }
            }
        }
        if s.arrives_on == Arrival::Deployed && s.arrives_env.is_none() {
            return Err("deployed to: name the environment".into());
        }
        // One column always takes new tickets (T-764), or `o` has nowhere
        // to file one.
        if s.refuses_new && self.columns.iter().all(|c| c.name == name || c.settings.refuses_new) {
            return Err("new tickets: the board's last column that takes them".into());
        }
        if let Some(c) = self.column_mut(name) {
            c.settings = s;
        }
        Ok(())
    }

    /// One-shot: fresh fractional indices for every live ticket in the
    /// column, in `by`'s order (ties keep their current order, then id).
    /// Returns the ids whose `order` changed. `needs_you` is the daemon's
    /// set — the attention queue's tickets and the woken ones.
    pub fn sort_column(
        &mut self,
        name: &str,
        by: SortBy,
        needs_you: &std::collections::HashSet<ulid::Ulid>,
    ) -> Vec<ulid::Ulid> {
        let mut ids: Vec<ulid::Ulid> = self.column_tickets(name).iter().map(|t| t.id).collect();
        let key_of = |id: ulid::Ulid| -> (u64, u64, bool) {
            let t = self.ticket(id).expect("id from column_tickets");
            let arrival = stamp_secs(t.column_since()).unwrap_or(0);
            let key =
                t.short_key.strip_prefix(KEY_PREFIX).and_then(|k| k.parse().ok()).unwrap_or(0);
            (arrival, key, needs_you.contains(&id))
        };
        match by {
            SortBy::NewestArrival => ids.sort_by_key(|&id| std::cmp::Reverse(key_of(id).0)),
            SortBy::OldestArrival => ids.sort_by_key(|&id| key_of(id).0),
            SortBy::Key => ids.sort_by_key(|&id| key_of(id).1),
            SortBy::NeedsYouFirst => ids.sort_by_key(|&id| !key_of(id).2),
            SortBy::Tag => {
                // A group's row is its registry entries in the order the flat
                // registry holds them (`group_entries`), and the picker's
                // `HJKL` is the only thing that arranges it — so this sorts by
                // the order the user put the picker in, never by the name. The
                // rank is the row index, taken once here rather than out of a
                // `group_entries` Vec per comparison.
                const GROUPS: usize = 10; // `move_tag`'s `1..=10`
                let mut rank: std::collections::HashMap<(u8, &str), u8> =
                    std::collections::HashMap::new();
                let mut filled = [0u8; GROUPS];
                for t in &self.tags {
                    let Some(seen) =
                        (t.group as usize).checked_sub(1).and_then(|i| filled.get_mut(i))
                    else {
                        continue;
                    };
                    let at = *seen;
                    *seen = seen.saturating_add(1);
                    rank.insert((t.group, t.name.as_str()), at);
                }
                // `[u8; GROUPS]` compares lexicographically, which IS "axis 1
                // decides, axis 2 breaks its ties". `u8::MAX` is "wears
                // nothing on this axis", so the untagged land at the bottom.
                ids.sort_by_key(|&id| {
                    let t = self.ticket(id).expect("id from column_tickets");
                    let mut k = [u8::MAX; GROUPS];
                    for r in &t.tags {
                        // A group outside 1-10 is skipped, never indexed: the
                        // field is a `u8` precisely so a value from a newer
                        // daemon cannot break the client.
                        let Some(cell) =
                            (r.group as usize).checked_sub(1).and_then(|i| k.get_mut(i))
                        else {
                            continue;
                        };
                        // A reference whose registry entry has gone still
                        // sorts as TAGGED — after every real tag, before the
                        // untagged. `tint_of`'s reasoning: it is not nothing.
                        *cell =
                            rank.get(&(r.group, r.name.as_str())).copied().unwrap_or(u8::MAX - 1);
                    }
                    k
                });
            }
        }
        let mut touched = Vec::new();
        let mut prev = String::new();
        for id in ids {
            let order = crate::fracindex::between(&prev, "");
            if let Some(t) = self.tickets.iter_mut().find(|t| t.id == id) {
                if t.order != order {
                    t.order = order.clone();
                    touched.push(id);
                }
            }
            prev = order;
        }
        touched
    }

    /// The columns whose tickets the bulk sleep offer prices.
    pub fn reclaim_columns(&self) -> std::collections::HashSet<&str> {
        self.columns
            .iter()
            .filter(|c| c.settings.offers().sleep())
            .map(|c| c.name.as_str())
            .collect()
    }

    pub fn archive_columns(&self) -> std::collections::HashSet<&str> {
        self.columns
            .iter()
            .filter(|c| c.settings.offers().archive())
            .map(|c| c.name.as_str())
            .collect()
    }

    pub fn ticket(&self, id: ulid::Ulid) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == id)
    }

    pub fn ticket_mut(&mut self, id: ulid::Ulid) -> Option<&mut Ticket> {
        self.tickets.iter_mut().find(|t| t.id == id)
    }

    /// The ticket a short key names (`T-12`), archived or not. A key in a
    /// note is the one way a ticket is referred to by hand, so this is the
    /// link recogniser's resolver (T-256); everything else looks up by id.
    pub fn ticket_by_key(&self, key: &str) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.short_key == key)
    }

    /// The ticket wearing the crown (T-411), if it is still on the board: a
    /// crown on a ticket that was deleted or archived under it reads as no
    /// crown, so every reader asks this and never `crown` itself.
    pub fn crown_holder(&self) -> Option<&Ticket> {
        self.crown.and_then(|id| self.ticket(id)).filter(|t| !t.is_archived())
    }

    /// Does `ticket` hold the crown right now?
    pub fn is_crowned(&self, ticket: ulid::Ulid) -> bool {
        self.crown_holder().is_some_and(|t| t.id == ticket)
    }

    /// Is `ticket`'s agent the worn crown's? The claim every crown wake makes
    /// on a worker (T-414): an agent a crown started, while a crown is worn,
    /// and never the crown's own ticket. The seat, not the ticket that wore
    /// it, owns what a crown started (T-927): a worker an earlier crown
    /// started is the crown's now, as `ask_agent`, `sleep_agent`,
    /// `answer_agent` and the spawn budget already took it, so a crown
    /// moved to another ticket hears its workers. A person's agent
    /// (`started_by: None`) never becomes one. The daemon's wakes and the
    /// person's banners (T-678) ask this one predicate, so a worker's news
    /// goes to exactly one of the two.
    pub fn started_by_crown(&self, ticket: ulid::Ulid) -> bool {
        let Some(crown) = self.crown_holder().map(|t| t.id) else { return false };
        ticket != crown
            && self
                .sessions
                .iter()
                .any(|s| s.ticket == ticket && s.holds_agent_seat() && s.started_by.is_some())
    }

    /// The vocabulary of axis `group`, in registry order — which is creation
    /// order, i.e. D31b's "stable order, config order, never by recency".
    ///
    /// Reading this off the registry rather than off the tickets is what
    /// makes the cycle well-behaved. An earlier cut derived it from whatever
    /// the tickets happened to wear, and that is a trap twice over: a tag
    /// vanished the moment its last wearer dropped it, and — worse — the
    /// ticket being cycled was itself part of the derivation, so taking a tag
    /// moved it in the list and the next press walked back to where it
    /// started. The cycle oscillated between two values and `none` was
    /// unreachable. A registry is independent of every ticket, so neither can
    /// happen.
    pub fn group_tags(&self, group: u8) -> Vec<&str> {
        self.tags.iter().filter(|t| t.group == group).map(|t| t.name.as_str()).collect()
    }

    /// The registry entries of axis `group`, in creation order.
    pub fn group_entries(&self, group: u8) -> Vec<&Tag> {
        self.tags.iter().filter(|t| t.group == group).collect()
    }

    /// Look a name up in the registry.
    pub fn tag_def(&self, group: u8, name: &str) -> Option<&Tag> {
        self.tags.iter().find(|t| t.group == group && t.name == name)
    }

    /// The tint a ticket's tag should be painted with. Falls back to the
    /// name's own hash for a reference whose registry entry has gone missing,
    /// so a card can never render a colourless band.
    pub fn tint_of(&self, t: &TagRef) -> u8 {
        self.tag_def(t.group, &t.name).map(|d| d.tint()).unwrap_or_else(|| default_tint(&t.name))
    }

    /// Make the starter offer: on a board that has never had a tag, write
    /// `STARTER_TAGS` onto `STARTER_GROUP`; on a board that already has a
    /// vocabulary, only remember that no offer is owed. Returns whether
    /// anything changed, so the caller knows to persist.
    pub fn seed_starter_tags(&mut self) -> bool {
        if self.tags_seeded {
            return false;
        }
        if self.tags.is_empty() {
            for (name, color) in STARTER_TAGS {
                self.tags.push(Tag {
                    name: name.to_string(),
                    group: STARTER_GROUP,
                    color: Some(color),
                });
            }
        }
        self.tags_seeded = true;
        true
    }

    /// Add a name to axis `group`. `Err` says why not, so the caller can show
    /// it rather than failing silently.
    pub fn register_tag(&mut self, group: u8, name: &str) -> Result<(), String> {
        if self.tags.iter().any(|t| t.group == group && t.name == name) {
            return Err(format!("{name} is already in group {group}"));
        }
        if self.group_entries(group).len() >= MAX_TAGS_PER_GROUP {
            return Err(format!("group {group} is full ({MAX_TAGS_PER_GROUP} tags)"));
        }
        self.tags.push(Tag { name: name.to_string(), group, color: None });
        Ok(())
    }

    /// Rename a tag, carrying every ticket wearing it along. Returns the ids
    /// that changed so the caller writes only those files.
    pub fn rename_tag(
        &mut self,
        group: u8,
        from: &str,
        to: &str,
    ) -> Result<Vec<ulid::Ulid>, String> {
        if self.tag_def(group, from).is_none() {
            return Err(format!("no tag {from:?} in group {group}"));
        }
        if from != to && self.tag_def(group, to).is_some() {
            return Err(format!("{to} is already in group {group}"));
        }
        for t in self.tags.iter_mut() {
            if t.group == group && t.name == from {
                t.name = to.to_string();
            }
        }
        let mut touched = Vec::new();
        for t in self.tickets.iter_mut() {
            let mut hit = false;
            for r in t.tags.iter_mut() {
                if r.group == group && r.name == from {
                    r.name = to.to_string();
                    hit = true;
                }
            }
            if hit {
                touched.push(t.id);
            }
        }
        Ok(touched)
    }

    /// Reposition a tag in the registry: along its own axis, or onto another
    /// one. Returns the ticket ids that changed, the way `rename_tag` does,
    /// so the caller writes only those files.
    ///
    /// Order is not decoration here. It is the order the picker's row draws
    /// in and the order a repeated digit cycles through, so which name an
    /// axis reaches first is worth being able to choose. The axis itself is
    /// the tag's meaning, and getting it wrong had no repair at all before
    /// this: `forget_tag` was the only other way off an axis, and it strips
    /// the tag from every ticket on the way out.
    ///
    /// A move onto ANOTHER axis is refused, never resolved, when a ticket
    /// wearing this tag already wears one there. One tag per group is what
    /// lets a digit address an axis, so the alternative is dropping somebody
    /// else's tag off a card nobody is looking at — not something one
    /// keypress may do quietly. The other two refusals are the ones
    /// `register_tag` already makes, for the same reasons: a full axis and a
    /// name that axis already holds.
    pub fn move_tag(
        &mut self,
        group: u8,
        name: &str,
        to_group: u8,
        to_index: usize,
    ) -> Result<Vec<ulid::Ulid>, String> {
        if !(1..=10).contains(&to_group) {
            return Err("tag group must be 1-10".into());
        }
        let Some(at) = self.tags.iter().position(|t| t.group == group && t.name == name) else {
            return Err(format!("no tag {name:?} in group {group}"));
        };
        if to_group != group {
            if self.group_entries(to_group).len() >= MAX_TAGS_PER_GROUP {
                return Err(format!("group {to_group} is full ({MAX_TAGS_PER_GROUP} tags)"));
            }
            if self.tag_def(to_group, name).is_some() {
                return Err(format!("{name} is already in group {to_group}"));
            }
            let blocked = self
                .tickets
                .iter()
                .filter(|t| t.wears(group, name) && t.tag_in(to_group).is_some());
            let blocked = blocked.count();
            if blocked > 0 {
                let noun = if blocked == 1 { "ticket" } else { "tickets" };
                return Err(format!("{blocked} {noun} already wear a tag on axis {to_group}"));
            }
        }
        let mut entry = self.tags.remove(at);
        entry.group = to_group;
        // A row is its group's entries in the order the flat registry holds
        // them, so the destination slot is a flat position — taken AFTER the
        // removal, or every index past the old one is off by one. Past the
        // end of the row means the end of the row: a tag arriving on a new
        // axis joins it, it does not push into the middle of it.
        let slots: Vec<usize> = self
            .tags
            .iter()
            .enumerate()
            .filter(|(_, t)| t.group == to_group)
            .map(|(i, _)| i)
            .collect();
        let at = match slots.get(to_index) {
            Some(i) => *i,
            None => slots.last().map(|i| i + 1).unwrap_or(self.tags.len()),
        };
        self.tags.insert(at, entry);

        let mut touched = Vec::new();
        if to_group != group {
            for t in self.tickets.iter_mut() {
                let mut hit = false;
                for r in t.tags.iter_mut() {
                    if r.group == group && r.name == name {
                        r.group = to_group;
                        hit = true;
                    }
                }
                if hit {
                    t.tags.sort_by_key(|r| r.group);
                    touched.push(t.id);
                }
            }
        }
        Ok(touched)
    }

    /// Set a tag's tint. `None` on the entry means "unchosen"; this always
    /// writes an explicit choice.
    pub fn set_tag_color(&mut self, group: u8, name: &str, color: u8) -> Result<(), String> {
        match self.tags.iter_mut().find(|t| t.group == group && t.name == name) {
            Some(t) => {
                t.color = Some(color % TAG_TINTS);
                Ok(())
            }
            None => Err(format!("no tag {name:?} in group {group}")),
        }
    }

    /// Remove a name from the registry AND from every ticket wearing it.
    /// Returns whether anything changed.
    ///
    /// Leaving a ticket wearing a retired tag would put a pip on the board
    /// that the cycle can never reach or clear, so the two must move together.
    pub fn forget_tag(&mut self, group: u8, name: &str) -> bool {
        let before = self.tags.len();
        self.tags.retain(|t| !(t.group == group && t.name == name));
        let mut changed = self.tags.len() != before;
        for t in &mut self.tickets {
            let n = t.tags.len();
            t.tags.retain(|tag| !(tag.group == group && tag.name == name));
            changed |= t.tags.len() != n;
        }
        changed
    }

    /// Tickets of one column, sorted by fractional order (ties by id for stability).
    /// Archived tickets stay in `tickets` (the ticket page needs them in the
    /// snapshot) but never surface here — this is the board's one chokepoint.
    pub fn column_tickets(&self, column: &str) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> =
            self.tickets.iter().filter(|t| t.column == column && !t.is_archived()).collect();
        v.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        v
    }

    /// Whether a column is pinned as a spine (T-117), which a `WhenEmpty`
    /// column is only while it holds no ticket (T-837).
    pub fn column_pinned(&self, column: &Column) -> bool {
        match column.settings.collapse() {
            Collapse::Off => false,
            Collapse::Always => true,
            Collapse::WhenEmpty => {
                !self.tickets.iter().any(|t| t.column == column.name && !t.is_archived())
            }
        }
    }

    /// Archived tickets, newest archive first (`at` stamps sort as strings).
    pub fn archived_tickets(&self) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> = self.tickets.iter().filter(|t| t.is_archived()).collect();
        v.sort_by(|a, b| {
            let ka = a.archived.as_ref().map(|x| x.at.as_str()).unwrap_or("");
            let kb = b.archived.as_ref().map(|x| x.at.as_str()).unwrap_or("");
            kb.cmp(ka).then(a.id.cmp(&b.id))
        });
        v
    }

    /// Tickets back from a snooze that nobody has looked at yet (T-74).
    pub fn woke_tickets(&self) -> Vec<&Ticket> {
        self.tickets.iter().filter(|t| t.is_woke() && !t.is_archived()).collect()
    }

    /// Tickets whose agent asked for a person and has not been answered
    /// (T-107). `woke_tickets`' rule: an archived one never counts.
    pub fn raised_tickets(&self) -> Vec<&Ticket> {
        self.tickets.iter().filter(|t| t.hand_raised() && !t.is_archived()).collect()
    }

    /// Tickets whose agent never got its first prompt (T-570): a seat with
    /// a pane and `unsent` words. `woke_tickets`' rule: an archived one
    /// never counts.
    pub fn unsent_tickets(&self) -> Vec<&Ticket> {
        // One pass over the sessions: the `!N` count asks this every frame.
        let unsent: std::collections::HashSet<ulid::Ulid> =
            self.sessions.iter().filter(|s| s.unsent_words().is_some()).map(|s| s.ticket).collect();
        if unsent.is_empty() {
            return Vec::new();
        }
        self.tickets.iter().filter(|t| !t.is_archived() && unsent.contains(&t.id)).collect()
    }

    /// Every ticket that needs the user, by any of the four roads: an
    /// attention-set session on it, a snooze that woke it (T-74), an
    /// agent's raised hand (T-107), or words a launch could not deliver
    /// (T-570). A SET, because the roads overlap — a woken ticket whose
    /// claude is also at a permission prompt is one ticket needing one
    /// person, and counting it twice made `!N` a number nothing on screen
    /// could be matched against.
    pub fn needs_you_tickets(&self, now: u64) -> std::collections::HashSet<ulid::Ulid> {
        crate::attention::attention_queue(self)
            .iter()
            .map(|s| s.ticket)
            .chain(self.woke_tickets().iter().map(|t| t.id))
            .chain(self.raised_tickets().iter().map(|t| t.id))
            .chain(self.unsent_tickets().iter().map(|t| t.id))
            .chain(
                self.sessions
                    .iter()
                    .filter(|s| {
                        s.kind == SessionKind::Bash
                            && s.state.has_pane()
                            && s.shell_close_request.as_ref().is_some_and(|q| {
                                q.expires_ms > now
                                    && self.crown_holder().map(|t| t.id) == Some(q.crown)
                            })
                    })
                    .map(|s| s.ticket),
            )
            .collect()
    }

    /// The `!N` count: tickets needing the user, counted once each. The
    /// header chip and the tmux status line both read this, so the number is
    /// one number.
    pub fn needs_you_count(&self, now: u64) -> usize {
        self.needs_you_tickets(now).len()
    }

    /// Sessions of the ticket that hold (or should hold) a pane. The archive
    /// gate, its TUI advisory, and the header suggestion all share this — the
    /// suggestion never offers what the keystroke would refuse.
    /// The one live agent a prompt from the board reaches: first in spawn
    /// order, the same session `board_enter` focuses, so the key that asks
    /// and the key that goes there land on one pane. `has_pane` and not
    /// `is_live` — a parked session is live and has no process to type at.
    /// The daemon picks by this and the TUI hints by it; one predicate.
    pub fn pane_target(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.sessions.iter().find(|s| s.ticket == ticket && s.kind.is_agent() && s.state.has_pane())
    }

    /// The pane seat that can take an `accept plan` (T-420): on the plan
    /// dialog, or launched planning. The daemon's column press and the
    /// TUI's ask field read this one answer.
    pub fn plan_seat(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.pane_target(ticket).filter(|s| s.on_plan_dialog() || s.launched_in_plan())
    }

    /// The pane seat Claude Code's send-now can reach (T-601, T-685): the
    /// prompt target where its provider has the key. A parked agent and an
    /// empty seat have no turn to cut into — a wake and a start take the
    /// words as the launch's first prompt — and a Codex pane has no key, so
    /// none of them is one. The TUI's `immediately` stop is offered by this
    /// and the daemon's `PromptSession`/`ask_agent` refusal reads it: one
    /// predicate, so the field never offers a stop the daemon refuses nor
    /// hides one it would take.
    pub fn send_now_seat(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.pane_target(ticket).filter(|s| s.kind.provider().is_some_and(|p| p.has_send_now()))
    }

    /// The agent a ticket already holds, parked or not — `is_live`, so a
    /// Sleeping record counts. A ticket holds ONE agent across providers: the
    /// daemon refuses a second spawn by this, and `c` wakes rather than
    /// starts by the same fact. A second seat on a ticket is a shell.
    pub fn live_agent(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.sessions.iter().find(|s| s.ticket == ticket && s.holds_agent_seat())
    }

    /// The seats the crown's agent started and still holds (T-412): the
    /// records that carry `started_by` and hold an agent seat AWAKE —
    /// whoever wears the crown now — on a ticket the board shows. The
    /// budget is counted over these, board-wide: a displaced crown does not
    /// free the seats the last one started, because the money is spent
    /// either way. A parked agent is out of the count (T-541): it runs no
    /// process and spends nothing, so `sleep_agent` (or a person's `x`, or
    /// the inactivity park) frees its seat, and its wake — a person's `c`,
    /// their send of the crown's held ask, or in autonomous mode the
    /// crown's own ask, which the daemon counts as a seat while it waits
    /// (T-550) — takes it back. An archived ticket's (a snooze's too) is out of the count
    /// as well (T-518), a belt now that an archive needs everything asleep.
    /// The record keeps its `started_by`, so a woken or restored seat counts
    /// again and its ticket still cannot be crowned.
    pub fn crown_started(&self) -> Vec<&SessionRecord> {
        self.sessions
            .iter()
            .filter(|s| s.started_by.is_some() && s.holds_agent_seat())
            .filter(|s| s.state != SessionState::Sleeping)
            .filter(|s| self.ticket(s.ticket).is_some_and(|t| !t.is_archived()))
            .collect()
    }

    /// Whether the snapshot carries this ticket (T-842): a live one, or an
    /// archived one with a pane still open on it, so a record that rides
    /// (`snapshot_record`) never names a ticket the board lacks. A parked
    /// agent on an archived ticket keeps its ticket home with its record.
    fn snapshot_ticket(&self, t: &Ticket) -> bool {
        !t.is_archived() || self.ticket_awake_sessions(t.id) > 0
    }

    /// Whether the snapshot carries this record (T-804): one with a pane
    /// always, a parked or gone one only on a ticket that is not archived.
    fn snapshot_record(
        rec: &SessionRecord,
        archived: &std::collections::HashSet<ulid::Ulid>,
    ) -> bool {
        rec.state.has_pane() || !archived.contains(&rec.ticket)
    }

    /// The board as the snapshot sends it, without an archived ticket
    /// (T-842) or a record with no pane on one (T-804). On a board months
    /// old the records were 614 of 629 and two thirds of every snapshot, and
    /// after that cut the tickets were 690 of 716 and three quarters of
    /// what remained, sent on every push and drawn by nothing on the board.
    /// `Command::ArchivedTickets` is how a client that lists them gets them
    /// and `Command::TicketSessions` how one that shows one gets its
    /// records. Only the kept tickets and records are cloned (T-838): the
    /// daemon took the copy whole — 641 parked records, over a megabyte of
    /// launch argv — and dropped them. Every other field is cloned whole.
    /// The count beside it is how many tickets are archived, kept or not.
    pub fn for_snapshot(&self) -> (Board, usize) {
        let archived: std::collections::HashSet<ulid::Ulid> =
            self.tickets.iter().filter(|t| t.is_archived()).map(|t| t.id).collect();
        let board = Board {
            columns: self.columns.clone(),
            tickets: self.tickets.iter().filter(|t| self.snapshot_ticket(t)).cloned().collect(),
            sessions: self
                .sessions
                .iter()
                .filter(|rec| Self::snapshot_record(rec, &archived))
                .cloned()
                .collect(),
            agent_provider: self.agent_provider,
            crown_budget: self.crown_budget,
            crown_mode: self.crown_mode,
            merge_into: self.merge_into,
            crown_archives: self.crown_archives,
            crown_watches: self.crown_watches,
            crown_context: self.crown_context,
            crown_cost: self.crown_cost,
            crown_samples: self.crown_samples,
            reviewed_prs: self.reviewed_prs,
            next_key: self.next_key,
            tags: self.tags.clone(),
            tags_seeded: self.tags_seeded,
            mcp_tools: self.mcp_tools,
            claude_md_ignored: self.claude_md_ignored,
            system_prompt: self.system_prompt,
            default_column: self.default_column.clone(),
            follow_up_mode: self.follow_up_mode,
            prompts: self.prompts.clone(),
            crown: self.crown,
            tiers: self.tiers.clone(),
            default_tier: self.default_tier.clone(),
            setup_pending: self.setup_pending,
            workspace: self.workspace,
        };
        (board, archived.len())
    }

    pub fn ticket_awake_sessions(&self, id: ulid::Ulid) -> usize {
        self.sessions.iter().filter(|s| s.ticket == id && s.state.has_pane()).count()
    }

    pub fn sorted_columns(&self) -> Vec<&Column> {
        let mut v: Vec<&Column> = self.columns.iter().collect();
        v.sort_by(|a, b| a.order.cmp(&b.order));
        v
    }
}

#[cfg(test)]
mod tests {

    /// T-577: a record reports through the mod alone when its launch took
    /// the mod and passed no hook set; one an earlier build launched on the
    /// mod still has `--settings` and its hook set until its next wake.
    #[test]
    fn frames_ride_the_mod_only_where_no_hook_set_was_passed() {
        let rec = |road, argv: &[&str]| {
            let mut r = SessionRecord::new(
                uuid::Uuid::from_u128(1),
                SessionKind::Claude,
                ulid::Ulid(1),
                argv.iter().map(|a| a.to_string()).collect(),
                "/".into(),
                SessionState::Running,
            );
            r.road = road;
            r
        };
        use crate::road::Road;
        assert!(rec(Road::Mod, &["claude", "--plugin-dir", "/m"]).frames_by_mod());
        assert!(!rec(Road::Mod, &["claude", "--settings", "/s.json", "--plugin-dir", "/m"])
            .frames_by_mod());
        assert!(!rec(Road::Hooks, &["claude", "--settings", "/s.json"]).frames_by_mod());
        // T-658: a native launch's `--settings` is the permission hooks
        // alone, and the mod reports the rest.
        let mut native = rec(Road::Mod, &["claude", "--settings", "/s.json", "--plugin-dir", "/m"]);
        native.native = true;
        assert!(native.frames_by_mod());
        native.road = Road::Hooks;
        assert!(!native.frames_by_mod(), "the hook set's road is never the mod's");
    }

    use super::*;

    /// T-570: words a launch never delivered make the seat `unsent` to the
    /// crown and needs-you to the header, while it has a pane to resend
    /// into — and ride `sessions.json` with no trace on a record without.
    #[test]
    fn an_unsent_seat_reads_unsent_and_needs_you_while_it_has_a_pane() {
        let ticket = ulid::Ulid::new();
        let mut board = Board::default();
        board.tickets.push(
            serde_json::from_value(serde_json::json!({
                "id": ticket.to_string(),
                "short_key": "T-1",
                "title": "t",
                "column": "TODO",
                "order": "1",
                "created_at": "@0",
            }))
            .unwrap(),
        );
        let mut rec = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ticket,
            Vec::new(),
            "/repo".into(),
            SessionState::Idle { stop_reason: StopReason::Unknown },
        );
        let bare = serde_json::to_value(&rec).unwrap();
        assert!(bare.get("unsent").is_none(), "absent until something went unsent");
        assert_eq!(rec.state_word(), "idle");
        rec.unsent = Some(Unsent { text: String::new(), brief: true });
        assert_eq!(rec.state_word(), "unsent");
        assert_eq!(rec.unsent_words().map(Unsent::word), Some("brief not sent"));
        let wire = serde_json::to_value(&rec).unwrap();
        assert_eq!(wire["unsent"], serde_json::json!({ "brief": true }));
        let back: SessionRecord = serde_json::from_value(wire).unwrap();
        assert_eq!(back.unsent, rec.unsent);
        board.sessions.push(rec);
        assert_eq!(board.needs_you_count(0), 1);
        assert_eq!(board.unsent_tickets().len(), 1);
        board.sessions[0].state = SessionState::Sleeping;
        assert_eq!(board.sessions[0].state_word(), "sleeping", "no pane, no resend");
        assert_eq!(board.needs_you_count(0), 0);
    }

    #[test]
    fn project_provider_defaults_without_reinterpreting_sessions() {
        let mut board = Board::default();
        assert_eq!(board.agent_provider, AgentProvider::ClaudeCode);
        for provider in [AgentProvider::ClaudeCode, AgentProvider::Codex] {
            board.agent_provider = provider;
            board.sessions.push(SessionRecord::new(
                uuid::Uuid::new_v4(),
                provider.session_kind(),
                ulid::Ulid::new(),
                Vec::new(),
                "/repo".into(),
                SessionState::Sleeping,
            ));
        }
        board.agent_provider = AgentProvider::ClaudeCode;
        let wire = serde_json::to_value(&board).unwrap();
        let restored: Board = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(restored.sessions[0].kind.provider(), Some(AgentProvider::ClaudeCode));
        assert_eq!(restored.sessions[1].kind.provider(), Some(AgentProvider::Codex));
        let mut legacy = wire;
        legacy.as_object_mut().unwrap().remove("agent_provider");
        assert_eq!(
            serde_json::from_value::<Board>(legacy).unwrap().agent_provider,
            AgentProvider::ClaudeCode
        );
    }

    /// T-610: a board from before the mode, whatever its two old switches
    /// said, lets the crown act; a supervised board stays supervised.
    #[test]
    fn crown_mode_defaults_autonomous_and_a_supervised_is_kept() {
        assert_eq!(Board::default().crown_mode, CrownMode::Autonomous);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        assert_eq!(wire["crown_mode"], "autonomous");
        wire.as_object_mut().unwrap().remove("crown_mode");
        wire["crown_sends"] = serde_json::json!(false);
        wire["crown_answers"] = serde_json::json!(false);
        assert_eq!(
            serde_json::from_value::<Board>(wire.clone()).unwrap().crown_mode,
            CrownMode::Autonomous,
            "the old switches are ignored"
        );
        wire["crown_mode"] = serde_json::json!("supervised");
        let back = serde_json::from_value::<Board>(wire).unwrap().crown_mode;
        assert_eq!(back, CrownMode::Supervised);
        assert!(!back.sends() && !back.answers());
        assert!(CrownMode::Autonomous.sends() && CrownMode::Autonomous.answers());
        assert_eq!(back.toggled(), CrownMode::Autonomous);
    }

    /// T-846: a codex's question or plan is the crown's as a claude's is;
    /// a permission, a secret or a form stays a person's for both, and a
    /// shell's stop is nobody's dialog.
    #[test]
    fn takes_stop_admits_a_claude_s_and_a_codex_s_question_or_plan_alone() {
        let stopped = |reason| SessionState::RequiresAction { reason };
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            assert!(CrownMode::Autonomous.takes_stop(kind, &stopped(Reason::Question)));
            assert!(CrownMode::Autonomous.takes_stop(kind, &stopped(Reason::Plan)));
            for reason in [Reason::Permission, Reason::Secret, Reason::Elicitation, Reason::Auth] {
                assert!(!CrownMode::Autonomous.takes_stop(kind, &stopped(reason)), "{reason:?}");
            }
            assert!(!CrownMode::Supervised.takes_stop(kind, &stopped(Reason::Question)));
            assert!(!CrownMode::Autonomous.takes_stop(kind, &SessionState::Running));
        }
        assert!(!CrownMode::Autonomous.takes_stop(SessionKind::Bash, &stopped(Reason::Question)));
    }

    /// T-810: every board merges into main unless it says pull requests.
    #[test]
    fn merge_into_defaults_main_and_pull_requests_is_kept() {
        assert_eq!(Board::default().merge_into, MergeInto::Main);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        assert_eq!(wire["merge_into"], "main");
        wire.as_object_mut().unwrap().remove("merge_into");
        assert_eq!(
            serde_json::from_value::<Board>(wire.clone()).unwrap().merge_into,
            MergeInto::Main
        );
        wire["merge_into"] = serde_json::json!("pull_requests");
        let back = serde_json::from_value::<Board>(wire).unwrap().merge_into;
        assert!(back.pull_requests() && !MergeInto::Main.pull_requests());
        assert_eq!(back.toggled(), MergeInto::Main);
        assert_eq!((MergeInto::Main.word(), back.word()), ("main", "pull requests"));
    }

    /// T-590: a board from before the field leaves archiving to a person,
    /// and one that turned it on keeps it on.
    #[test]
    fn crown_archives_defaults_off_and_an_on_is_kept() {
        assert!(!Board::default().crown_archives);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        wire.as_object_mut().unwrap().remove("crown_archives");
        assert!(!serde_json::from_value::<Board>(wire.clone()).unwrap().crown_archives);
        wire["crown_archives"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Board>(wire).unwrap().crown_archives);
    }

    /// T-732: a fresh board and one from before the field let the crown
    /// watch, and one that turned watching off keeps it off.
    #[test]
    fn crown_watches_defaults_on_and_an_off_is_kept() {
        assert!(Board::default().crown_watches);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        wire.as_object_mut().unwrap().remove("crown_watches");
        assert!(serde_json::from_value::<Board>(wire.clone()).unwrap().crown_watches);
        wire["crown_watches"] = serde_json::json!(false);
        assert!(!serde_json::from_value::<Board>(wire).unwrap().crown_watches);
    }

    /// T-969: a fresh board and one from before the fields let the crown
    /// read a session's context and not a ticket's cost, and a board that
    /// turned either the other way keeps it.
    #[test]
    fn crown_context_defaults_on_crown_cost_off_and_a_change_is_kept() {
        let b = Board::default();
        assert!(b.crown_context && !b.crown_cost);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        wire.as_object_mut().unwrap().remove("crown_context");
        wire.as_object_mut().unwrap().remove("crown_cost");
        let old: Board = serde_json::from_value(wire.clone()).unwrap();
        assert!(old.crown_context && !old.crown_cost);
        wire["crown_context"] = serde_json::json!(false);
        wire["crown_cost"] = serde_json::json!(true);
        let turned: Board = serde_json::from_value(wire).unwrap();
        assert!(!turned.crown_context && turned.crown_cost);
    }

    /// T-973: the crown samples on every board that never said otherwise,
    /// and an off survives.
    #[test]
    fn crown_samples_defaults_on_and_an_off_is_kept() {
        assert!(Board::default().crown_samples);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        wire.as_object_mut().unwrap().remove("crown_samples");
        let old: Board = serde_json::from_value(wire.clone()).unwrap();
        assert!(old.crown_samples);
        wire["crown_samples"] = serde_json::json!(false);
        let turned: Board = serde_json::from_value(wire).unwrap();
        assert!(!turned.crown_samples);
    }

    /// T-859: reviewed pull requests are a ticket's on every board that
    /// never said otherwise, and an off survives.
    #[test]
    fn reviewed_prs_default_on_and_an_off_is_kept() {
        assert!(Board::default().reviewed_prs);
        let mut wire = serde_json::to_value(Board::default()).unwrap();
        wire.as_object_mut().unwrap().remove("reviewed_prs");
        assert!(serde_json::from_value::<Board>(wire.clone()).unwrap().reviewed_prs);
        wire["reviewed_prs"] = serde_json::json!(false);
        assert!(!serde_json::from_value::<Board>(wire).unwrap().reviewed_prs);
    }

    /// T-543: a column opts its tickets into the idle park, taking an agent
    /// idle at its prompt with no finished turn (woken, interrupted) too; a
    /// ticket in a column that did not opt in never sleeps by time (T-610
    /// removed the board's timer).
    #[test]
    fn a_columns_sleep_timer_is_the_only_one() {
        let mut board = Board::default();
        let mut done = Column::new("DONE", "b");
        done.settings.sleep_after_minutes = 5;
        board.columns = vec![Column::new("TODO", "a"), done];
        board.tickets = vec![ticket(1, "TODO", "a"), ticket(2, "DONE", "a")];
        let rec = |ticket: u128, stop_reason| {
            let mut r = SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Claude,
                ulid::Ulid(ticket),
                vec!["claude".into()],
                "/repo".into(),
                SessionState::Idle { stop_reason },
            );
            r.confidence = Confidence::High;
            r.state_changed_at = Some(0);
            r
        };
        let minute = 1_000;
        let (woken, finished) = (rec(2, StopReason::Unknown), rec(2, StopReason::EndTurn));
        assert_eq!(board.idle_park_due(&woken, 4_999, minute), None, "not yet");
        assert_eq!(board.idle_park_due(&woken, 5_000, minute), Some(("autosleep", 5_000)));
        assert_eq!(board.idle_park_due(&finished, 5_000, minute), Some(("autosleep", 5_000)));
        let interrupted = rec(2, StopReason::Interrupted);
        assert_eq!(board.idle_park_due(&interrupted, 5_000, minute), Some(("autosleep", 5_000)));
        for working in [StopReason::Background, StopReason::Monitoring] {
            assert_eq!(board.idle_park_due(&rec(2, working), 99_000, minute), None, "{working:?}");
        }
        let elsewhere = rec(1, StopReason::EndTurn);
        assert_eq!(board.idle_park_due(&elsewhere, 99_000, minute), None, "nothing on there");
        let mut low = rec(2, StopReason::Unknown);
        low.confidence = Confidence::Low;
        assert_eq!(board.idle_park_due(&low, 99_000, minute), None, "a guess is not idle");
    }

    #[test]
    fn a_columns_sleep_choice_cycles_and_reads_in_words() {
        let mut s = ColumnSettings::default();
        let mut seen = vec![s.sleep_after_minutes];
        for _ in 0..5 {
            s.sleep_after_minutes = s.next_sleep_after();
            seen.push(s.sleep_after_minutes);
        }
        assert_eq!(seen, [0, 1, 5, 15, 60, 0]);
        s.sleep_after_minutes = 30;
        assert_eq!(s.next_sleep_after(), 60, "a hand-edited value joins the cycle");
        s.sleep_after_minutes = 120;
        assert_eq!(s.next_sleep_after(), 0);
        assert!(!ColumnSettings::default().automated());
        s.sleep_after_minutes = 5;
        assert!(s.automated(), "the header marks a column that sleeps agents");
        assert_eq!(s.summary(), ["sleeps idle agents after 5 min"]);
        let json = serde_json::to_string(&ColumnSettings::default()).unwrap();
        assert!(!json.contains("sleep_after_minutes"), "off is not written: {json}");
        let back: ColumnSettings = serde_json::from_str(r#"{"sleep_after_minutes":5}"#).unwrap();
        assert_eq!(back.sleep_after_minutes, 5);
    }

    #[test]
    fn autosleep_requires_a_confirmed_idle_and_ages_from_settlement() {
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            let mut rec = SessionRecord::new(
                uuid::Uuid::new_v4(),
                kind,
                ulid::Ulid::new(),
                vec!["claude".into()],
                "/repo".into(),
                SessionState::Idle { stop_reason: StopReason::EndTurn },
            );
            rec.confidence = Confidence::High;
            rec.observation_hold = false;
            assert!(!rec.autosleep_due(99_000, 60_000), "unknown idle age");
            rec.state_changed_at = Some(30_000);
            assert!(!rec.autosleep_due(89_999, 60_000));
            assert!(rec.autosleep_due(90_000, 60_000));
            assert!(!rec.autosleep_due(90_000, 0), "off");
            assert!(!rec.autosleep_due(29_999, 60_000), "clock moved backwards");
            for state in [
                SessionState::Running,
                SessionState::Spawning,
                SessionState::Sleeping,
                SessionState::RequiresAction { reason: Reason::Permission },
                SessionState::Failed { reason: FailReason::Unknown },
                SessionState::unknown(),
                SessionState::Idle { stop_reason: StopReason::Background },
                SessionState::Idle { stop_reason: StopReason::Monitoring },
            ] {
                let mut held = rec.clone();
                held.state = state;
                assert!(!held.autosleep_due(90_000, 60_000), "{:?}", held.state);
            }
            // A stop on a limit is at its prompt (T-808).
            for state in [
                SessionState::RequiresAction { reason: Reason::QuotaResume },
                SessionState::Throttled,
            ] {
                let mut limited = rec.clone();
                limited.state = state;
                assert!(limited.autosleep_due(90_000, 60_000), "{:?}", limited.state);
            }
            let mut shell = rec.clone();
            shell.kind = SessionKind::Bash;
            assert!(!shell.autosleep_due(90_000, 60_000));
            if kind == SessionKind::Codex {
                for flag in 0..4 {
                    let mut held = rec.clone();
                    match flag {
                        0 => held.observation_hold = true,
                        1 => held.codex_stopping = true,
                        2 => held.codex_submit_sent = true,
                        _ => held.codex_pending_seq = Some(1),
                    }
                    assert!(!held.autosleep_due(90_000, 60_000), "Codex hold {flag}");
                }
            }
            for confidence in [Confidence::Medium, Confidence::Low, Confidence::Stale] {
                let mut held = rec.clone();
                held.confidence = confidence;
                assert!(!held.autosleep_due(90_000, 60_000));
            }
            let mut held = rec.clone();
            held.pending_submit = true;
            assert!(!held.autosleep_due(90_000, 60_000));
            held = rec.clone();
            held.pending_prefill = true;
            assert!(!held.autosleep_due(90_000, 60_000));
            rec.argv.clear();
            assert!(!rec.autosleep_due(90_000, 60_000), "observe-only adoption");
        }
    }

    #[test]
    fn exited_codex_owns_the_agent_seat_until_server_cleanup_finishes() {
        let ticket = ulid::Ulid::new();
        let mut session = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Codex,
            ticket,
            Vec::new(),
            "/repo".into(),
            SessionState::Exited { reason: ExitReason::Killed },
        );
        session.codex_stopping = true;
        let mut board = Board {
            agent_provider: AgentProvider::ClaudeCode,
            sessions: vec![session].into(),
            ..Board::default()
        };
        assert!(board.live_agent(ticket).is_some());
        assert!(board.pane_target(ticket).is_none());
        board.sessions[0].codex_stopping = false;
        assert!(board.live_agent(ticket).is_none());
    }

    /// T-1026: a killed Codex seat's cleanup confirmed, the next agent
    /// started, and only then the old pane's death frame came. It must
    /// leave the closed record closed, so the ticket's one seat is the new
    /// agent's; a death with the record still on its pane holds the seat.
    #[test]
    fn a_pane_s_death_after_its_record_closed_holds_no_seat() {
        let ticket = ulid::Ulid::new();
        let record = |state: SessionState| {
            SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Codex,
                ticket,
                vec!["codex".into()],
                "/repo".into(),
                state,
            )
        };
        let next = record(SessionState::Idle { stop_reason: StopReason::EndTurn });
        for closed in [SessionState::Exited { reason: ExitReason::Killed }, SessionState::Sleeping]
        {
            let parked = closed == SessionState::Sleeping;
            let mut old = record(closed);
            let was = old.clone();
            assert!(!old.pane_died(), "{:?}", old.state);
            assert!(!old.codex_stopping, "{:?}", old.state);
            assert_eq!(old.observation_hold, was.observation_hold, "{:?}", old.state);
            let board = Board {
                sessions: if parked { vec![old] } else { vec![old, next.clone()] }.into(),
                ..Board::default()
            };
            if !parked {
                assert_eq!(board.live_agent(ticket).map(|s| s.id), Some(next.id));
            }
        }
        // Still on its pane: the runtime winds down and holds the seat.
        let mut dying = record(SessionState::Running);
        assert!(dying.pane_died());
        assert!(dying.codex_stopping && dying.observation_hold);
        dying.state = SessionState::Exited { reason: ExitReason::Crashed };
        assert!(dying.holds_agent_seat());
        // A shell or a Claude seat carries no Codex cleanup.
        let mut claude = record(SessionState::Running);
        claude.kind = SessionKind::Claude;
        assert!(!claude.pane_died() && !claude.codex_stopping);
    }

    #[test]
    fn sleeping_agents_reserve_the_same_seat_across_provider_switches() {
        let ticket = ulid::Ulid::new();
        for provider in [AgentProvider::ClaudeCode, AgentProvider::Codex] {
            let mut board = Board { agent_provider: provider.next(), ..Board::default() };
            board.sessions.push(SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Bash,
                ticket,
                Vec::new(),
                "/repo".into(),
                SessionState::Running,
            ));
            let id = uuid::Uuid::new_v4();
            board.sessions.push(SessionRecord::new(
                id,
                provider.session_kind(),
                ticket,
                Vec::new(),
                "/repo".into(),
                SessionState::Sleeping,
            ));
            assert_eq!(board.live_agent(ticket).unwrap().id, id);
            assert!(board.pane_target(ticket).is_none());
            board.sessions[1].state = SessionState::Running;
            assert_eq!(board.pane_target(ticket).unwrap().id, id);
        }
    }

    /// T-685: the one seat a send-now reaches. The TUI offers the
    /// `immediately` stop by this and the daemon refuses by it, so the
    /// four seats answer once: a paned Claude yes; a parked Claude, an
    /// empty seat and a Codex pane no.
    #[test]
    fn send_now_seat_is_a_claude_pane_alone() {
        let ticket = ulid::Ulid::new();
        let rec = |kind: SessionKind, state: SessionState| {
            SessionRecord::new(
                uuid::Uuid::new_v4(),
                kind,
                ticket,
                Vec::new(),
                "/repo".into(),
                state,
            )
        };
        let with =
            |sessions: Vec<SessionRecord>| Board { sessions: sessions.into(), ..Board::default() };
        let paned = with(vec![rec(SessionKind::Claude, SessionState::Running)]);
        assert_eq!(
            paned.send_now_seat(ticket).map(|s| s.id),
            paned.pane_target(ticket).map(|s| s.id)
        );
        assert!(paned.send_now_seat(ticket).is_some(), "a paned claude");
        let parked = with(vec![rec(SessionKind::Claude, SessionState::Sleeping)]);
        assert!(parked.live_agent(ticket).is_some(), "the seat is held");
        assert!(parked.send_now_seat(ticket).is_none(), "a parked claude: a wake takes the words");
        assert!(with(vec![]).send_now_seat(ticket).is_none(), "an empty seat: a start takes them");
        let codex = with(vec![rec(SessionKind::Codex, SessionState::Running)]);
        assert!(codex.pane_target(ticket).is_some());
        assert!(codex.send_now_seat(ticket).is_none(), "a codex pane has no key");
        // A shell beside a parked claude is a pane and not a seat.
        let shell = with(vec![
            rec(SessionKind::Bash, SessionState::Running),
            rec(SessionKind::Claude, SessionState::Sleeping),
        ]);
        assert!(shell.send_now_seat(ticket).is_none());
        assert!(AgentProvider::ClaudeCode.has_send_now());
        assert!(!AgentProvider::Codex.has_send_now());
    }

    /// T-541: a crown-started agent holds a budget seat only while it is
    /// awake. The author's crown parked its finished workers and was still
    /// refused until it archived their tickets; a parked agent spends
    /// nothing, so the park alone frees the seat and the wake takes it back.
    /// The record keeps `started_by` throughout.
    #[test]
    fn a_sleeping_crown_started_agent_frees_its_seat() {
        let crown = ulid::Ulid::new();
        let mut board = Board::default();
        let states = [
            SessionState::Running,
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Sleeping,
        ];
        for (n, state) in (1..=3).zip(states) {
            let t = ticket(n, "IN PROGRESS", "a");
            let mut rec = SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Claude,
                t.id,
                vec!["claude".into()],
                "/repo".into(),
                state,
            );
            rec.started_by = Some(crown);
            board.tickets.push(t);
            board.sessions.push(rec);
        }
        let held = |b: &Board| b.crown_started().iter().map(|s| s.ticket).collect::<Vec<_>>();
        assert_eq!(held(&board), vec![ulid::Ulid(1), ulid::Ulid(2)], "awake seats count");
        assert!(board.live_agent(ulid::Ulid(3)).is_some(), "the parked seat itself stands");
        assert!(board.sessions[2].started_by.is_some(), "and so does its provenance");
        board.sessions[2].state = SessionState::Spawning;
        assert_eq!(board.crown_started().len(), 3, "a wake counts it again");
    }

    /// T-927: the person moved the crown from one ticket to another, and
    /// the new crown heard nothing of four workers the old one had started.
    /// What a crown started is the seat's: the crown worn now hears it,
    /// whichever ticket wore the crown at its start. A person's agent is
    /// never the crown's, the crown's own ticket is never its own worker,
    /// and with no crown worn every worker is the person's.
    #[test]
    fn a_worker_an_earlier_crown_started_is_the_crowns_now() {
        let mut board = Board::default();
        for n in 1..=4 {
            board.tickets.push(ticket(n, "IN PROGRESS", "a"));
        }
        let seat = |n: u128, by: Option<u128>| {
            let mut rec = SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Claude,
                ulid::Ulid(n),
                vec!["claude".into()],
                "/repo".into(),
                SessionState::Running,
            );
            rec.started_by = by.map(ulid::Ulid);
            rec
        };
        // T-1 wore the crown and started T-3; T-4 is a person's agent.
        board.sessions.push(seat(2, None));
        board.sessions.push(seat(3, Some(1)));
        board.sessions.push(seat(4, None));
        board.crown = Some(ulid::Ulid(1));
        assert!(board.started_by_crown(ulid::Ulid(3)), "the crown that started it");
        board.crown = Some(ulid::Ulid(2));
        assert!(board.started_by_crown(ulid::Ulid(3)), "the crown worn now");
        assert!(!board.started_by_crown(ulid::Ulid(4)), "a person's agent stays the person's");
        assert!(!board.started_by_crown(ulid::Ulid(2)), "the crown's own ticket is no worker");
        board.crown = None;
        assert!(!board.started_by_crown(ulid::Ulid(3)), "no crown worn: the person's");
    }

    /// T-518: the author archived the three tickets the crown had started,
    /// and the crown was still refused on them. A seat on an archived
    /// ticket — a snooze is one too — is out of the budget's count, even
    /// held awake; the record keeps `started_by`, so a restore counts it
    /// again.
    #[test]
    fn an_archived_tickets_agent_frees_its_crown_seat() {
        let crown = ulid::Ulid::new();
        let mut board = Board::default();
        for n in 1..=3 {
            let t = ticket(n, "IN PROGRESS", "a");
            let mut rec = SessionRecord::new(
                uuid::Uuid::new_v4(),
                SessionKind::Claude,
                t.id,
                vec!["claude".into()],
                "/repo".into(),
                SessionState::Idle { stop_reason: StopReason::EndTurn },
            );
            rec.started_by = Some(crown);
            board.tickets.push(t);
            board.sessions.push(rec);
        }
        assert_eq!(board.crown_started().len(), 3, "an awake agent holds its seat");
        board.tickets[0].archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        board.tickets[1].archived = Some(Archived {
            at: "@1".into(),
            by: "local".into(),
            until: Some("@4102444800".into()),
            needs_you: false,
        });
        let held: Vec<ulid::Ulid> = board.crown_started().iter().map(|s| s.ticket).collect();
        assert_eq!(held, vec![ulid::Ulid(3)], "archived and snoozed seats are free");
        assert!(board.live_agent(ulid::Ulid(1)).is_some(), "the seat itself is untouched");
        assert!(board.sessions[0].started_by.is_some(), "and so is its provenance");
        board.tickets[0].archived = None;
        assert_eq!(board.crown_started().len(), 2, "a restore counts it again");
    }

    #[test]
    fn missing_codex_observation_defaults_to_a_checkout_hold() {
        let mut record = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Codex,
            ulid::Ulid::new(),
            Vec::new(),
            "/repo".into(),
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        record.codex_thread_id = Some("opaque:thread/with-non-uuid-format".into());
        let mut wire = serde_json::to_value(&record).unwrap();
        wire.as_object_mut().unwrap().remove("observation_hold");
        let restored: SessionRecord = serde_json::from_value(wire).unwrap();
        assert!(restored.observation_hold);
        assert_eq!(restored.codex_thread_id, record.codex_thread_id);
        assert!(crate::quiet::is_working(&restored));
    }

    #[test]
    fn owed_submit_cannot_precede_codex_prefill() {
        let mut record = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Codex,
            ulid::Ulid::new(),
            Vec::new(),
            "/repo".into(),
            SessionState::Idle { stop_reason: StopReason::Unknown },
        );
        record.pending_submit = true;
        record.pending_prefill = true;
        assert!(!record.pressable());
        record.pending_prefill = false;
        assert!(record.pressable());
        record.codex_submit_sent = true;
        assert!(!record.pressable(), "an unacknowledged Enter must not be repeated");
        assert!(record.pending_submit, "the checkout stays held while acknowledgement is pending");
    }

    #[test]
    fn column_offers_preserve_legacy_settings_and_round_trip_independently() {
        for legacy in [false, true] {
            let old: ColumnSettings =
                serde_json::from_value(serde_json::json!({"reclaim": legacy})).unwrap();
            assert_eq!(old.offers().sleep(), legacy);
            assert_eq!(old.offers().archive(), legacy);
            for offer in
                [ColumnOffers::Off, ColumnOffers::Sleep, ColumnOffers::Archive, ColumnOffers::Both]
            {
                let settings = ColumnSettings { offers: Some(offer), ..old.clone() };
                let wire: ColumnSettings =
                    serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
                assert_eq!(wire.offers(), offer);
                let board = Board {
                    columns: vec![Column { name: "chosen".into(), order: "a0".into(), settings }],
                    ..Default::default()
                };
                assert_eq!(board.reclaim_columns().contains("chosen"), offer.sleep());
                assert_eq!(board.archive_columns().contains("chosen"), offer.archive());
            }
        }
    }

    /// T-837: the collapse mode round-trips through its two switches, an old
    /// `collapsed = true` still reads `always`, and `when empty` pins a column
    /// only while no live ticket stands in it.
    #[test]
    fn collapse_modes_round_trip_and_when_empty_follows_the_tickets() {
        let old: ColumnSettings =
            serde_json::from_value(serde_json::json!({"collapsed": true})).unwrap();
        assert_eq!(old.collapse(), Collapse::Always);
        let mut c = Collapse::Off;
        for _ in 0..3 {
            let mut settings = ColumnSettings::default();
            settings.set_collapse(c);
            let wire: ColumnSettings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(wire.collapse(), c);
            c = c.next();
        }
        assert_eq!(c, Collapse::Off, "Enter cycles the three");
        assert_eq!(
            serde_json::to_string(&ColumnSettings::default()).unwrap(),
            "{}",
            "off writes nothing"
        );

        let mut settings = ColumnSettings::default();
        settings.set_collapse(Collapse::WhenEmpty);
        let mut board = Board {
            columns: vec![Column { name: "c".into(), order: "a0".into(), settings }],
            ..Default::default()
        };
        assert!(board.column_pinned(&board.columns[0]), "empty: a spine");
        let mut t = ticket(1, "c", "a0");
        t.archived =
            Some(Archived { at: "@1".into(), by: String::new(), until: None, needs_you: false });
        board.tickets.push(t);
        assert!(board.column_pinned(&board.columns[0]), "an archived ticket is not in it");
        board.tickets.push(ticket(2, "c", "a1"));
        assert!(!board.column_pinned(&board.columns[0]), "a ticket opens it");
    }

    /// An M1 sessions.json line must keep parsing after the M2 enum growth
    /// (store::load hard-fails on parse error — defaults are the migration).
    #[test]
    fn m1_session_record_parses() {
        let m1 = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "claude",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["claude"],
            "cwd": "/tmp",
            "state": { "state": "unknown" }
        }"#;
        let rec: SessionRecord = serde_json::from_str(m1).unwrap();
        assert_eq!(rec.state, SessionState::unknown());
        assert_eq!(rec.confidence, Confidence::High);
        assert!(rec.waiting_since.is_none());

        let m1_exited = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "bash",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["/bin/zsh"],
            "cwd": "/tmp",
            "state": { "state": "exited", "reason": "killed" }
        }"#;
        let rec: SessionRecord = serde_json::from_str(m1_exited).unwrap();
        assert_eq!(rec.state, SessionState::Exited { reason: ExitReason::Killed });
    }

    /// T-957 round 10: the held call is let go on every departure written
    /// through `set_state` (a park, a supervisor's loss, a death, a wake,
    /// the machine's own), and kept while the record is on a plan or a
    /// question.
    #[test]
    fn set_state_lets_the_held_call_go_off_a_dialog() {
        let mut rec = SessionRecord::new(
            uuid::Uuid::nil(),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/tmp".into(),
            SessionState::RequiresAction { reason: Reason::Plan },
        );
        for kept in [Reason::Plan, Reason::Question] {
            rec.dialog_call = Some("toolu_b".into());
            rec.set_state(SessionState::RequiresAction { reason: kept });
            assert_eq!(rec.dialog_call.as_deref(), Some("toolu_b"), "{kept:?}");
        }
        for gone in [
            SessionState::Sleeping,
            SessionState::Unknown { reason: UnknownReason::SupervisorDead },
            SessionState::Exited { reason: ExitReason::Killed },
            SessionState::Spawning,
            SessionState::Running,
            SessionState::RequiresAction { reason: Reason::Permission },
        ] {
            rec.dialog_call = Some("toolu_b".into());
            rec.dialog_print = Some("AskUserQuestion:00".into());
            rec.set_state(gone.clone());
            assert_eq!((rec.dialog_call.as_deref(), &rec.state), (None, &gone));
            assert!(rec.dialog_print.is_none());
        }
    }

    /// An M2 sessions.json line (pre-provenance) must keep parsing after the
    /// M3 field growth — defaults are the migration.
    #[test]
    fn m2_session_record_parses() {
        let m2 = r#"{
            "id": "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b",
            "kind": "claude",
            "ticket": "01J8ZQ7VJ00000000000000000",
            "argv": ["claude", "--settings", "/x.json", "--session-id", "3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b"],
            "cwd": "/tmp",
            "state": { "state": "requires_action", "reason": "permission" },
            "waiting_since": 1724900000000,
            "state_changed_at": 1724900000000,
            "transcript_path": "/tmp/t.jsonl",
            "detail": "Bash",
            "confidence": "high"
        }"#;
        let rec: SessionRecord = serde_json::from_str(m2).unwrap();
        assert_eq!(rec.provenance, Provenance::Spawned);
        assert!(rec.claude_session_id.is_none());
        assert!(rec.codex_thread_id.is_none());
        assert!(rec.codex_generation.is_none());
        assert!(rec.codex_turn_id.is_none());
        assert_eq!(rec.codex_observed_seq, 0);
        assert!(rec.agent_preview_path.is_none());
        assert!(!rec.pending_prefill);
        assert!(!rec.codex_submit_sent);
        assert!(rec.dialog_call.is_none(), "T-957: no call held from an older line");
        assert!(rec.dialog_print.is_none());
    }

    fn ticket(id: u128, column: &str, order: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid(id),
            short_key: format!("T-{id}"),
            title: "t".into(),
            column: column.into(),
            order: order.into(),
            created_at: "@0".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            previous_column: None,
            picked: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            envelope: None,
            raised: None,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        }
    }

    /// T-804: the snapshot leaves out a parked or gone record of an
    /// archived ticket, and nothing else — a record with a pane rides
    /// whatever its ticket, and a live ticket keeps its parked ones. T-842:
    /// it leaves out the archived ticket too, unless a pane is open on it,
    /// and counts every archived ticket either way.
    #[test]
    fn the_snapshot_drops_only_paneless_records_of_archived_tickets() {
        let mut board = Board {
            tickets: vec![ticket(1, "todo", "a"), ticket(2, "todo", "b"), ticket(3, "todo", "c")],
            ..Default::default()
        };
        let archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        board.tickets[1].archived = archived.clone();
        board.tickets[2].archived = archived;
        let rec = |n: u128, t: u128, state: SessionState| {
            SessionRecord::new(
                uuid::Uuid::from_u128(n),
                SessionKind::Claude,
                ulid::Ulid(t),
                Vec::new(),
                "/repo".into(),
                state,
            )
        };
        let gone = SessionState::Exited { reason: ExitReason::UserQuit };
        *board.sessions = vec![
            rec(1, 1, SessionState::Sleeping),
            rec(2, 2, SessionState::Sleeping),
            rec(3, 2, SessionState::Running),
            rec(4, 2, gone),
        ];
        let (copy, counted) = board.for_snapshot();
        let kept: Vec<u128> = copy.sessions.iter().map(|s| s.id.as_u128()).collect();
        assert_eq!(kept, vec![1, 3]);
        // Ticket 2 rides for its open pane, ticket 3 stays home; both count.
        let tickets: Vec<u128> = copy.tickets.iter().map(|t| t.id.0).collect();
        assert_eq!(tickets, vec![1, 2]);
        assert_eq!(counted, 2);
        // Every other field rides whole (T-838).
        board.tickets.retain(|t| tickets.contains(&t.id.0));
        board.sessions.retain(|s| kept.contains(&s.id.as_u128()));
        assert_eq!(
            serde_json::to_string(&copy).unwrap(),
            serde_json::to_string(&board).unwrap(),
            "the copy is the board less what it leaves out"
        );
    }

    #[test]
    fn column_stay_remembers_only_visits_over_one_minute() {
        let mut t = ticket(1, "IN PROGRESS", "a0");
        // Legacy tickets use creation until their first stamped move.
        for end in ["@0", "@59", "@60", "invalid"] {
            t.remember_column_stay(end);
            assert_eq!(t.previous_column, None);
        }
        t.remember_column_stay("@61");
        assert_eq!(
            t.previous_column,
            Some(ColumnStay { column: "IN PROGRESS".into(), seconds: 61 })
        );
        let remembered = t.previous_column.clone();
        t.column = "REVIEW".into();
        t.entered_at = Some("@1000".into());
        for end in ["@999", "@1060", "invalid"] {
            t.remember_column_stay(end);
            assert_eq!(t.previous_column, remembered);
        }
        t.remember_column_stay("@4661");
        assert_eq!(t.previous_column, Some(ColumnStay { column: "REVIEW".into(), seconds: 3661 }));
        let remembered = t.previous_column.clone();
        t.entered_at = Some("invalid".into());
        t.remember_column_stay("@9999");
        assert_eq!(t.previous_column, remembered);
        let wire = serde_json::to_string(&t).unwrap();
        let back: Ticket = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.previous_column, remembered);
    }

    /// A snooze is an archive with a deadline: the deadline and the wake
    /// flag ride inside `[archived]` (the TOML shape is pinned in the
    /// daemon's store tests), and the count that lights the header sees a
    /// woke ticket but never an archived one.
    #[test]
    fn snooze_fields_roundtrip_and_count() {
        let mut t = ticket(1, "TODO", "a");
        t.archived = Some(Archived {
            at: "@10".into(),
            by: "local".into(),
            until: Some("@4102444800".into()),
            needs_you: true,
        });
        let json = serde_json::to_string(&t).expect("serializes");
        let back: Ticket = serde_json::from_str(&json).expect("parses");
        assert_eq!(back.snooze_until_secs(), Some(4102444800));
        assert!(back.archived.as_ref().is_some_and(|a| a.needs_you));
        assert!(!back.is_woke());
        // A plain archive carries neither: absent, not null, so an older
        // build's file and this build's agree byte for byte.
        let plain = ticket(3, "TODO", "c");
        let json = serde_json::to_string(&plain).expect("serializes");
        assert!(!json.contains("woke_at") && !json.contains("needs_you"), "{json}");

        let mut w = ticket(2, "TODO", "b");
        w.woke_at = Some("@20".into());
        let json = serde_json::to_string(&w).expect("serializes");
        let back: Ticket = serde_json::from_str(&json).expect("parses");
        assert!(back.is_woke());

        let mut b = Board::default();
        b.tickets.push(t);
        b.tickets.push(w);
        assert_eq!(b.woke_tickets().len(), 1, "an archived ticket never counts, a woke one does");
        assert_eq!(b.needs_you_count(0), 1);
        assert_eq!(stamp_secs("@7"), Some(7));
    }

    /// A raised hand (T-107) is a table on the ticket, absent while no hand
    /// is up, and it lights the same count a snooze's wake does.
    #[test]
    fn a_raised_hand_roundtrips_and_counts() {
        let plain = ticket(1, "TODO", "a");
        let json = serde_json::to_string(&plain).expect("serializes");
        assert!(!json.contains("raised"), "absent, not empty: {json}");
        assert!(!plain.hand_raised());

        let mut t = ticket(2, "REVIEW", "b");
        t.raised = Some(Raised {
            at: "@20".into(),
            by: "agent:00000000-0000-0000-0000-000000000000".into(),
            reason: "which auth provider?".into(),
        });
        let json = serde_json::to_string(&t).expect("serializes");
        let back: Ticket = serde_json::from_str(&json).expect("parses");
        assert!(back.hand_raised());
        assert_eq!(back.raised.as_ref().map(|r| r.reason.as_str()), Some("which auth provider?"));

        // A ticket from before the field reads as no hand, never as a broken
        // one: the whole board must not fall over an older file. (The TOML
        // shape — `[raised]` with the tables, since a scalar after a table
        // errors — is pinned in the daemon's store tests, beside the snooze's.)
        let older = r#"{"id":"00000000000000000000000001","short_key":"T-1","title":"t",
                        "column":"TODO","order":"a","created_at":"@0"}"#;
        let old: Ticket = serde_json::from_str(older).expect("an older ticket still parses");
        assert!(!old.hand_raised());

        let mut b = Board::default();
        b.tickets.push(t);
        assert_eq!(b.raised_tickets().len(), 1);
        assert_eq!(b.needs_you_count(0), 1);

        // An archived ticket is off the board, so its hand is not counted —
        // `woke_tickets`' rule, and the reason `agent_raise_hand` refuses one.
        b.tickets[0].archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        assert!(b.raised_tickets().is_empty());
        assert_eq!(b.needs_you_count(0), 0);
    }

    /// `!N` counts TICKETS, not roads to them: a ticket that is both back
    /// from a snooze and holding a raised hand needs one person once.
    #[test]
    fn the_needs_you_count_never_counts_a_ticket_twice() {
        let mut t = ticket(1, "TODO", "a");
        t.woke_at = Some("@20".into());
        t.raised =
            Some(Raised { at: "@21".into(), by: "agent:x".into(), reason: "which one?".into() });
        let mut b = Board::default();
        b.tickets.push(t);
        assert_eq!(b.woke_tickets().len(), 1);
        assert_eq!(b.raised_tickets().len(), 1);
        assert_eq!(b.needs_you_count(0), 1, "one ticket, one person, one number");
        assert_eq!(stamp_secs("7"), None);
    }

    #[test]
    fn shell_close_attention_expires_at_the_injected_time() {
        let mut b = Board::default();
        b.tickets.push(ticket(1, "TODO", "crown"));
        b.crown = Some(ulid::Ulid(1));
        let mut shell = SessionRecord::new(
            uuid::Uuid::nil(),
            SessionKind::Bash,
            ulid::Ulid(1),
            vec!["sh".into()],
            "/".into(),
            SessionState::Running,
        );
        shell.shell_shared = true;
        b.sessions.push(shell.clone());
        assert_eq!(b.needs_you_count(9), 0, "sharing alone never asks for attention");
        b.sessions.clear();
        shell.shell_close_request = Some(ShellCloseRequest {
            id: uuid::Uuid::nil(),
            crown: ulid::Ulid(1),
            pane: "1:%1".into(),
            expires_ms: 10,
        });
        b.sessions.push(shell);
        assert_eq!(b.needs_you_count(9), 1);
        assert_eq!(b.needs_you_count(10), 0);
        b.crown = None;
        assert_eq!(b.needs_you_count(9), 0);
    }

    /// The registry is board-level and persisted: a group's vocabulary is
    /// what has been registered there, in creation order, independent of what
    /// any ticket currently wears. Nothing is seeded — a name enters by being
    /// used once — and nothing leaves except by an explicit retire.
    #[test]
    fn the_registry_outlives_its_wearers_and_holds_its_order() {
        let mut b = Board::with_default_columns();
        let mut id = 0u128;
        let mut mk = |tags: &[(u8, &str)]| {
            id += 1;
            let mut t = ticket(id, "TODO", "a");
            for (g, name) in tags {
                t.set_tag(*g, Some((*name).to_string()));
                let _ = b.register_tag(*g, name);
            }
            b.tickets.push(t);
        };
        mk(&[(1, "BUG"), (2, "DEV")]);
        mk(&[(1, "REGR")]);
        mk(&[(1, "BUG"), (2, "PRODUCTION")]);

        // Registry order is creation order (D31b's "config order").
        assert_eq!(b.group_tags(1), vec!["BUG", "REGR"]);
        assert_eq!(b.group_tags(2), vec!["DEV", "PRODUCTION"]);
        assert_eq!(b.group_tags(3), Vec::<&str>::new());
        assert!(b.register_tag(1, "BUG").is_err(), "registering twice is refused");

        // The vocabulary is independent of what the tickets wear. Derive it
        // from them instead and it breaks twice: a tag vanishes when its last
        // wearer drops it, and the ticket being cycled reorders its own list,
        // so the digit walks backwards and `none` becomes unreachable.
        let owned = |b: &Board| -> Vec<String> {
            b.group_tags(1).iter().map(|s| (*s).to_string()).collect()
        };
        let before = owned(&b);
        b.tickets[0].set_tag(1, Some("REGR".into()));
        assert_eq!(owned(&b), before, "wearing a tag must not reorder the cycle");
        b.tickets[0].set_tag(1, Some("BUG".into()));
        assert_eq!(owned(&b), before);

        // A tag OUTLIVES its wearers. This is the point of a registry: strip
        // it off every ticket and it is still in the cycle tomorrow.
        for t in b.tickets.iter_mut() {
            t.set_tag(1, None);
        }
        assert_eq!(owned(&b), before, "an unworn tag stays in the vocabulary");

        // Retiring is explicit, and takes the pips with it.
        b.tickets[0].set_tag(1, Some("BUG".into()));
        assert!(b.forget_tag(1, "BUG"));
        assert_eq!(b.group_tags(1), vec!["REGR"]);
        assert!(b.tickets[0].tag_in(1).is_none(), "a retired tag leaves no orphan pip");
        assert!(!b.forget_tag(1, "BUG"), "forgetting twice is a no-op");

        // Archiving must not renumber a cycle either.
        b.tickets[0].archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        assert_eq!(b.group_tags(1), vec!["REGR"]);
    }

    /// An axis is capped: ten names, the same ten the groups run to. Past it
    /// a group stops being an axis and becomes a list, and the picker row —
    /// windowed since the cap went up — spends more of itself scrolling than
    /// showing.
    #[test]
    fn a_group_fills_up() {
        let mut b = Board::with_default_columns();
        for i in 0..MAX_TAGS_PER_GROUP {
            assert!(b.register_tag(1, &format!("t{i}")).is_ok(), "{i}");
        }
        let err = b.register_tag(1, "one-too-many").expect_err("cap enforced");
        assert!(err.contains("full"), "{err}");
        // Other axes are unaffected, and a freed slot can be refilled.
        assert!(b.register_tag(2, "elsewhere").is_ok());
        assert!(b.forget_tag(1, "t0"));
        assert!(b.register_tag(1, "one-too-many").is_ok());
    }

    /// Along its own axis a move is pure order, and order is what the row
    /// draws and what a repeated digit walks. Nothing about any ticket
    /// changes, so nothing is reported as touched.
    #[test]
    fn moving_a_tag_along_its_axis_reorders_the_cycle() {
        let mut b = Board::with_default_columns();
        for n in ["A", "B", "C"] {
            b.register_tag(1, n).expect("registered");
        }
        b.register_tag(2, "OTHER").expect("registered");
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(1, Some("B".into()));
        b.tickets.push(t);

        assert_eq!(b.move_tag(1, "B", 1, 0), Ok(Vec::new()), "a reorder touches no ticket");
        assert_eq!(b.group_tags(1), vec!["B", "A", "C"]);
        assert_eq!(b.move_tag(1, "B", 1, 2), Ok(Vec::new()));
        assert_eq!(b.group_tags(1), vec!["A", "C", "B"]);
        // Past the end of the row is the end of the row, not a panic.
        assert_eq!(b.move_tag(1, "A", 1, 99), Ok(Vec::new()));
        assert_eq!(b.group_tags(1), vec!["C", "B", "A"]);
        // The interleaved axis is untouched, and so is the wearer.
        assert_eq!(b.group_tags(2), vec!["OTHER"]);
        assert_eq!(b.tickets[0].tag_in(1).map(|r| r.name.as_str()), Some("B"));
    }

    /// Onto another axis, the registry entry and every wearer move together
    /// — the same bargain `rename_tag` makes. A tag that changed axis without
    /// its wearers would leave pips on a row that no longer reaches them.
    #[test]
    fn moving_a_tag_to_another_axis_carries_its_wearers() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "STAGING").expect("registered");
        b.register_tag(2, "BUG").expect("registered");
        let mut wearing = ticket(1, "TODO", "a");
        wearing.set_tag(1, Some("STAGING".into()));
        let mut elsewhere = ticket(2, "TODO", "b");
        elsewhere.set_tag(2, Some("BUG".into()));
        b.tickets.push(wearing);
        b.tickets.push(elsewhere);

        assert_eq!(b.move_tag(1, "STAGING", 3, 0), Ok(vec![ulid::Ulid(1)]));
        assert_eq!(b.group_tags(1), Vec::<&str>::new());
        assert_eq!(b.group_tags(3), vec!["STAGING"]);
        assert!(b.tickets[0].tag_in(1).is_none(), "no pip left on the old axis");
        assert_eq!(b.tickets[0].tag_in(3).map(|r| r.name.as_str()), Some("STAGING"));
        // The ticket's own list stays sorted by group, which is what the card
        // reads first and second off.
        let groups: Vec<u8> = b.tickets[0].tags.iter().map(|r| r.group).collect();
        let mut sorted = groups.clone();
        sorted.sort_unstable();
        assert_eq!(groups, sorted);
        // A ticket that never wore it is not touched.
        assert_eq!(b.tickets[1].tag_in(2).map(|r| r.name.as_str()), Some("BUG"));
    }

    /// The three refusals, and the one that is this function's own: one tag
    /// per axis is what lets a digit address an axis, so a move that would
    /// make a ticket wear two there is refused rather than resolved. Each
    /// refusal leaves the registry exactly as it found it — a half-applied
    /// move is worse than none.
    #[test]
    fn a_tag_never_moves_onto_an_axis_a_wearer_already_uses() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "STAGING").expect("registered");
        b.register_tag(2, "PROD").expect("registered");
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(1, Some("STAGING".into()));
        t.set_tag(2, Some("PROD".into()));
        b.tickets.push(t);

        let before = b.tags.clone();
        let err = b.move_tag(1, "STAGING", 2, 0).expect_err("the wearer blocks it");
        assert!(err.contains("1 ticket") && err.contains("axis 2"), "{err}");
        assert_eq!(b.tags, before, "a refusal changes nothing");
        assert_eq!(b.tickets[0].tag_in(1).map(|r| r.name.as_str()), Some("STAGING"));

        // A name the destination already holds, and a full destination: the
        // same two refusals `register_tag` makes.
        b.register_tag(3, "STAGING").expect("a name may repeat across axes");
        let err = b.move_tag(1, "STAGING", 3, 0).expect_err("duplicate name");
        assert!(err.contains("already in group 3"), "{err}");
        for i in 0..MAX_TAGS_PER_GROUP {
            let _ = b.register_tag(4, &format!("f{i}"));
        }
        let err = b.move_tag(1, "STAGING", 4, 0).expect_err("full axis");
        assert!(err.contains("full"), "{err}");
        // And a tag that is not there at all.
        assert!(b.move_tag(9, "NOPE", 1, 0).is_err());
        assert!(b.move_tag(1, "STAGING", 0, 0).is_err(), "group 0 is not an axis");
    }

    /// Colour is a registry property, so recolouring repaints every card at
    /// once. Unchosen falls back to a hash of the name — stable across
    /// machines, so a tag has a colour from the moment it exists.
    #[test]
    fn colour_lives_on_the_registry() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "BUG").expect("registered");
        let def = b.tag_def(1, "BUG").expect("in registry");
        assert_eq!(def.color, None);
        assert_eq!(def.tint(), default_tint("BUG"));
        assert!(def.tint() < TAG_TINTS);

        b.set_tag_color(1, "BUG", 3).expect("recoloured");
        assert_eq!(b.tag_def(1, "BUG").expect("still there").tint(), 3);
        // Out of range wraps rather than being refused.
        b.set_tag_color(1, "BUG", TAG_TINTS + 1).expect("wrapped");
        assert_eq!(b.tag_def(1, "BUG").expect("still there").tint(), 1);
        assert!(b.set_tag_color(1, "NOPE", 0).is_err());
    }

    /// A rename carries every wearer along, and reports which files changed.
    #[test]
    fn renaming_carries_the_wearers() {
        let mut b = Board::with_default_columns();
        b.register_tag(1, "BUG").expect("registered");
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(1, Some("BUG".into()));
        b.tickets.push(t);
        b.tickets.push(ticket(2, "TODO", "b"));

        let touched = b.rename_tag(1, "BUG", "DEFECT").expect("renamed");
        assert_eq!(touched, vec![ulid::Ulid(1)], "only the wearer's file changed");
        assert_eq!(b.group_tags(1), vec!["DEFECT"]);
        assert!(b.tickets[0].wears(1, "DEFECT"));
        assert!(b.tickets[1].tag_in(1).is_none());
        // A name already in the group, and a name that is not there at all.
        b.register_tag(1, "OTHER").expect("registered");
        assert!(b.rename_tag(1, "DEFECT", "OTHER").is_err());
        assert!(b.rename_tag(1, "GHOST", "X").is_err());
    }

    /// One tag per group: setting replaces, `None` clears, and the set stays
    /// sorted so a card's pips never reorder because another axis changed.
    /// The ladder the digit walks: none → first → … → last → none. The turn
    /// off the end is the whole point — a wrapping cycle can put a tag on a
    /// card but never take the last one off, which would leave `^t` the only
    /// way to undo a keystroke.
    #[test]
    fn the_cycle_runs_off_the_end_into_untagged() {
        let names = ["BUG", "REGR", "FLAKE"];
        assert_eq!(cycle_tag(&names, None).as_deref(), Some("BUG"));
        assert_eq!(cycle_tag(&names, Some("BUG")).as_deref(), Some("REGR"));
        assert_eq!(cycle_tag(&names, Some("REGR")).as_deref(), Some("FLAKE"));
        assert_eq!(cycle_tag(&names, Some("FLAKE")), None, "off the end is untagged");
        assert_eq!(cycle_tag(&names, None).as_deref(), Some("BUG"), "and round again");
        // A name the registry no longer holds restarts the cycle rather than
        // sticking on a value no press can leave.
        assert_eq!(cycle_tag(&names, Some("GONE")).as_deref(), Some("BUG"));
        // An axis with no vocabulary clears whatever is stranded on it.
        assert_eq!(cycle_tag(&[], Some("GONE")), None);
        assert_eq!(cycle_tag(&[], None), None);
    }

    #[test]
    fn set_tag_replaces_within_a_group() {
        let mut t = ticket(1, "TODO", "a");
        t.set_tag(2, Some("DEV".into()));
        t.set_tag(1, Some("BUG".into()));
        assert_eq!(t.tags.iter().map(|t| t.group).collect::<Vec<_>>(), vec![1, 2]);

        t.set_tag(1, Some("REGR".into()));
        assert_eq!(t.tags.len(), 2);
        assert_eq!(t.tag_in(1).map(|t| t.name.as_str()), Some("REGR"));

        t.set_tag(1, None);
        assert_eq!(t.tag_in(1), None);
        assert_eq!(t.tag_in(2).map(|t| t.name.as_str()), Some("DEV"));
    }

    /// A tag name is user text on a card row, so it runs the same width
    /// gauntlet as peek text: a terminal-vs-unicode-width disagreement there
    /// strands a `selected_bg` cell past the card edge that the diff never
    /// repaints.
    /// T-876: the shell-title check reads the stored form, so what the
    /// scrub removes cannot hide a `!` from it.
    #[test]
    fn a_shell_title_is_judged_as_stored() {
        assert!(shell_title("!rm -rf ~"));
        assert!(shell_title("  ! rm -rf ~"));
        assert!(!shell_title("fix the build!"));
        assert!(!shell_title(""));
        // A zero-width space or a tab before the `!` goes in the scrub.
        assert!(shell_title(&sanitize_title("\u{200B}!rm -rf ~")));
        assert!(shell_title(&sanitize_title("\u{200B} !rm -rf ~")));
        assert!(shell_title(&sanitize_title("\t!rm -rf ~")));
    }

    #[test]
    fn sanitize_title_caps_without_splitting_a_character() {
        // A pasted document: bounded, and the cut never lands inside a
        // multi-byte character (Hebrew is two bytes a letter).
        let long = "ש".repeat(TITLE_MAX_BYTES);
        let out = sanitize_title(&long);
        assert!(out.len() <= TITLE_MAX_BYTES, "{} bytes", out.len());
        assert!(out.chars().all(|c| c == 'ש'));
        assert_eq!(out.chars().count(), TITLE_MAX_BYTES / 2);
        // Under the cap nothing moves.
        assert_eq!(sanitize_title("fix the auth bug"), "fix the auth bug");
        // Control characters go, as on every card row.
        assert_eq!(sanitize_title("fix\u{1b}[31m bug"), "fix[31m bug");
    }

    /// A note past the limit is refused with both numbers in the words, and
    /// the boundary is bytes: exactly the limit fits, one byte over does not,
    /// and a two-byte script hits it at half the characters.
    #[test]
    fn a_note_past_the_limit_is_refused_by_the_byte_and_named_in_bytes() {
        let exact = "x".repeat(NOTE_MAX_BYTES);
        assert_eq!(note_size_error(&exact), None);
        assert_eq!(sanitize_note(&exact), exact, "exactly the limit is stored whole");
        assert_eq!(note_size_error("short"), None);
        let over = "x".repeat(NOTE_MAX_BYTES + 1);
        let why = note_size_error(&over).expect("one byte over is refused");
        assert!(why.contains(&format!("{} bytes", NOTE_MAX_BYTES + 1)), "{why}");
        assert!(why.contains(&format!("limit is {NOTE_MAX_BYTES} bytes (32 KiB)")), "{why}");
        // Hebrew is two bytes a letter: the limit in characters is half.
        let heb_fits = "ש".repeat(NOTE_MAX_BYTES / 2);
        assert_eq!(note_size_error(&heb_fits), None);
        let heb_over = "ש".repeat(NOTE_MAX_BYTES / 2 + 1);
        let why = note_size_error(&heb_over).expect("one character over, two bytes over");
        assert!(why.contains(&format!("{} bytes", NOTE_MAX_BYTES + 2)), "{why}");
        // The floor under a lying client still cuts on a character boundary.
        let cut = sanitize_note(&heb_over);
        assert_eq!(cut.len(), NOTE_MAX_BYTES);
        assert!(cut.chars().all(|c| c == 'ש'));
    }

    /// The starters are lawful tags: names `sanitize_tag` would pass whole,
    /// on one axis, each with a distinct chosen colour on the ring. And the
    /// offer is made once: a board with a vocabulary is only stamped, and a
    /// stamped board is never re-seeded.
    #[test]
    fn the_starter_tags_are_lawful_and_offered_once() {
        for (name, color) in STARTER_TAGS {
            assert_eq!(sanitize_tag(name).as_deref(), Some(name));
            assert!(color < TAG_TINTS);
        }
        let mut colours: Vec<u8> = STARTER_TAGS.iter().map(|(_, c)| *c).collect();
        colours.dedup();
        assert_eq!(colours.len(), STARTER_TAGS.len(), "one tint each");

        let mut b = Board::default();
        assert!(b.seed_starter_tags());
        assert_eq!(b.group_tags(STARTER_GROUP), vec!["BUG", "FEATURE", "CHANGE"]);
        assert!(!b.seed_starter_tags(), "a second offer changes nothing");
        b.tags.clear();
        assert!(!b.seed_starter_tags(), "forgetting them is not answered with them");

        let mut own = Board::default();
        own.register_tag(1, "OWN").unwrap();
        assert!(own.seed_starter_tags(), "the stamp is a change");
        assert_eq!(own.group_tags(1), vec!["OWN"]);
    }

    #[test]
    fn sanitize_tag_strips_the_width_hazards() {
        assert_eq!(sanitize_tag("BUG"), Some("BUG".into()));
        assert_eq!(sanitize_tag("  spaced  "), Some("spaced".into()));
        // Nothing blank is a tag.
        assert_eq!(sanitize_tag(""), None);
        assert_eq!(sanitize_tag("   "), None);
        assert_eq!(sanitize_tag("\u{200b}"), None);
        // Control chars and the drawn-structure range the L1 law bans.
        assert_eq!(sanitize_tag("a\nb"), Some("ab".into()));
        assert_eq!(sanitize_tag("a\u{2500}b"), Some("ab".into()));
        assert_eq!(sanitize_tag("a\u{2588}b"), Some("ab".into()));
        // VS16 turns a narrow symbol into a two-cell emoji that
        // `unicode-width` still counts as one.
        assert_eq!(sanitize_tag("x\u{fe0f}"), Some("x".into()));
        assert_eq!(sanitize_tag("a\u{200d}b"), Some("ab".into()));
        // Bounded, and never split a char.
        let long = sanitize_tag(&"é".repeat(40)).expect("non-empty");
        assert!(long.len() <= TAG_MAX_BYTES, "{} bytes", long.len());
        assert!(long.chars().all(|c| c == 'é'));
    }

    /// Archived tickets stay in `tickets` but never surface on the board.
    #[test]
    fn column_tickets_excludes_archived() {
        let mut b = Board::with_default_columns();
        b.tickets.push(ticket(1, "DONE", "a"));
        let mut t = ticket(2, "DONE", "b");
        t.archived =
            Some(Archived { at: "@10".into(), by: "local".into(), until: None, needs_you: false });
        b.tickets.push(t);
        let done: Vec<_> = b.column_tickets("DONE").iter().map(|t| t.id.0).collect();
        assert_eq!(done, vec![1]);
        let arch: Vec<_> = b.archived_tickets().iter().map(|t| t.id.0).collect();
        assert_eq!(arch, vec![2]);
    }

    /// Newest archive first; `column` survives so restore is exact.
    #[test]
    fn archived_tickets_newest_first() {
        let mut b = Board::with_default_columns();
        for (id, at) in [(1u128, "@100"), (2, "@300"), (3, "@200")] {
            let mut t = ticket(id, "REVIEW", "a");
            t.archived =
                Some(Archived { at: at.into(), by: "local".into(), until: None, needs_you: false });
            b.tickets.push(t);
        }
        let ids: Vec<_> = b.archived_tickets().iter().map(|t| t.id.0).collect();
        assert_eq!(ids, vec![2, 3, 1]);
        assert_eq!(b.archived_tickets()[0].column, "REVIEW");
    }

    #[test]
    fn state_roundtrips() {
        for s in [
            SessionState::Spawning,
            SessionState::RequiresAction { reason: Reason::Permission },
            SessionState::RequiresAction { reason: Reason::ResumeDialog },
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Sleeping,
            SessionState::Failed { reason: FailReason::Server },
            SessionState::Throttled,
            SessionState::Unknown { reason: UnknownReason::SupervisorDead },
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: SessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    // ---- columns (T-117) ------------------------------------------------

    fn template_board() -> Board {
        Board::with_default_columns()
    }

    /// The seeding table covers exactly the template — the one place a name
    /// is read as a literal, and it reads no other.
    /// Every Setup template's rules name columns it has and pass the
    /// board's own column validation, one column takes new tickets, and
    /// Kanban is the board a repository got before Setup (T-816).
    #[test]
    fn every_template_is_a_valid_board() {
        for t in Template::ALL {
            for noun in ["PR", "MR"] {
                let mut b = Board::default();
                b.apply_template(t, noun).expect("an empty board takes a template");
                let names: Vec<String> =
                    b.sorted_columns().iter().map(|c| c.name.clone()).collect();
                assert_eq!(names, t.columns(noun).into_iter().map(|(n, _)| n).collect::<Vec<_>>());
                for c in b.columns.clone() {
                    b.set_column_settings(&c.name, c.settings.clone())
                        .unwrap_or_else(|e| panic!("{t:?} {}: {e}", c.name));
                }
                assert!(b.prune_dangling_refs().is_empty(), "{t:?}");
                assert!(b.columns.iter().any(|c| !c.settings.refuses_new), "{t:?}");
            }
        }
        let mut b = Board::default();
        b.apply_template(Template::Kanban, "PR").expect("template");
        let fresh = Board::with_default_columns();
        let shape = |b: &Board| -> Vec<(String, ColumnSettings)> {
            b.sorted_columns().iter().map(|c| (c.name.clone(), c.settings.clone())).collect()
        };
        assert_eq!(shape(&b), shape(&fresh));
        let names = |t: Template| t.columns("MR").into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(Template::PullRequests), ["TODO", "IN PROGRESS", "MR OPEN", "MERGED"]);
        assert_eq!(
            names(Template::PullRequestsProduction).last().map(String::as_str),
            Some("PRODUCTION")
        );
        assert_eq!(Template::Kanban.merge_into(), MergeInto::Main);
        assert!(Template::PullRequests.merge_into().pull_requests());
    }

    /// A board with a ticket keeps its columns: a template would strand it.
    #[test]
    fn a_template_is_refused_once_a_ticket_exists() {
        let mut b = Board::with_default_columns();
        b.tickets.push(ticket(1, "REVIEW", "a"));
        assert!(b.apply_template(Template::PullRequests, "PR").is_err());
        assert!(b.column("REVIEW").is_some());
    }

    #[test]
    fn template_settings_cover_exactly_the_default_columns() {
        for name in DEFAULT_COLUMNS {
            assert!(template_settings(name).is_some(), "{name}");
        }
        assert!(template_settings("Backlog").is_none());
        assert!(template_settings("todo").is_none(), "the literal, not a case-fold");
        let b = template_board();
        assert_eq!(b.column("TODO").unwrap().settings.on_working.as_deref(), Some("IN PROGRESS"));
        assert_eq!(b.column("IN PROGRESS").unwrap().settings.on_done.as_deref(), Some("REVIEW"));
        assert_eq!(b.column("IN PROGRESS").unwrap().settings.train, TrainReach::Rebase);
        assert_eq!(b.column("REVIEW").unwrap().settings.on_working.as_deref(), Some("IN PROGRESS"));
        assert_eq!(b.column("REVIEW").unwrap().settings.train, TrainReach::Merge);
        // A branch the train merges out of REVIEW closes there (T-1037).
        assert_eq!(b.column("REVIEW").unwrap().settings.on_merged.as_deref(), Some("DONE"));
        assert!(b.column("DONE").unwrap().settings.requires_merge);
        assert!(b.column("DONE").unwrap().settings.reclaim);
        assert_eq!(b.reclaim_columns().into_iter().collect::<Vec<_>>(), vec!["DONE"]);
        // Every template rule points at a column the board has.
        let mut b2 = b.clone();
        assert!(b2.prune_dangling_refs().is_empty());
        assert!(!b2.seed_template_settings(), "seeding twice changes nothing");
    }

    #[test]
    fn seeding_prunes_a_reference_to_a_missing_column() {
        let mut b = Board::default();
        b.columns.push(Column::new("TODO", "a"));
        b.columns.push(Column::new("DONE", "b"));
        assert!(b.seed_template_settings());
        assert_eq!(b.column("TODO").unwrap().settings.on_working, None);
        assert!(b.column("DONE").unwrap().settings.reclaim);
    }

    #[test]
    fn rename_column_carries_every_ticket_and_every_reference() {
        let mut b = template_board();
        b.tickets.push(ticket(1, "TODO", "a"));
        b.tickets.push(ticket(2, "TODO", "b"));
        b.tickets.push(ticket(3, "REVIEW", "a"));
        let mut archived = ticket(4, "TODO", "c");
        archived.archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        b.tickets.push(archived);
        let touched = b.rename_column("TODO", "INBOX").unwrap();
        assert_eq!(touched, vec![ulid::Ulid(1), ulid::Ulid(2), ulid::Ulid(4)]);
        assert!(b.column("TODO").is_none());
        assert!(b.column("INBOX").is_some());
        assert_eq!(b.ticket(ulid::Ulid(4)).unwrap().column, "INBOX", "archived too");
        assert_eq!(b.ticket(ulid::Ulid(3)).unwrap().column, "REVIEW");
        // A rule pointing at the renamed column follows it.
        let mut b = template_board();
        b.rename_column("IN PROGRESS", "DOING").unwrap();
        assert_eq!(b.column("TODO").unwrap().settings.on_working.as_deref(), Some("DOING"));
        assert_eq!(b.column("REVIEW").unwrap().settings.on_working.as_deref(), Some("DOING"));
        assert_eq!(b.column("DOING").unwrap().settings.on_done.as_deref(), Some("REVIEW"));
        b.rename_column("DONE", "SHIPPED").unwrap();
        assert_eq!(b.column("REVIEW").unwrap().settings.on_merged.as_deref(), Some("SHIPPED"));
        // A deleted target is cleared and said, as the automoves are.
        b.columns.retain(|c| c.name != "SHIPPED");
        assert!(b.prune_dangling_refs().contains(&("REVIEW".into(), "on_merged")));
        assert_eq!(b.column("REVIEW").unwrap().settings.on_merged, None);
    }

    #[test]
    fn default_column_is_the_landing_column_and_follows_a_rename() {
        let mut b = template_board();
        assert_eq!(b.landing_column().as_deref(), Some("TODO"), "unset means the first column");
        assert_eq!(b.set_default_column(Some("NOPE")).unwrap_err(), "no such column: NOPE");
        assert_eq!(b.default_column, None, "a refusal changes nothing");
        b.set_default_column(Some("DONE")).unwrap();
        assert_eq!(b.landing_column().as_deref(), Some("DONE"));
        b.rename_column("DONE", "SHIPPED").unwrap();
        assert_eq!(b.default_column.as_deref(), Some("SHIPPED"), "a rename carries it");
        assert_eq!(b.landing_column().as_deref(), Some("SHIPPED"));
        // A default that takes no new tickets lands where `o` on it would.
        b.set_default_column(Some("REVIEW")).unwrap();
        assert_eq!(b.landing_column().as_deref(), Some("TODO"));
        // A name the board lost reads as the first column, and the prune
        // clears it so the file agrees with what the board does.
        b.default_column = Some("GONE".into());
        assert_eq!(b.landing_column().as_deref(), Some("TODO"));
        assert_eq!(b.prune_dangling_refs(), vec![("GONE".to_string(), "default_column")]);
        assert_eq!(b.default_column, None);
        b.set_default_column(None).unwrap();
        assert_eq!(b.landing_column().as_deref(), Some("TODO"));
        assert_eq!(Board::default().landing_column(), None, "no columns, nowhere to land");
    }

    /// T-764: a column that takes no new tickets files `o` into its nearest
    /// open neighbour, the left one first; the board keeps one open column.
    #[test]
    fn a_closed_column_files_into_its_nearest_open_neighbour() {
        let mut b = template_board();
        assert!(b.column("IN PROGRESS").unwrap().settings.refuses_new);
        assert!(b.column("REVIEW").unwrap().settings.refuses_new);
        assert!(!b.column("TODO").unwrap().settings.refuses_new);
        assert!(!b.column("DONE").unwrap().settings.refuses_new);
        let filed = |b: &Board, n: &str| b.filing_column(n);
        assert_eq!(filed(&b, "TODO").as_deref(), Some("TODO"), "an open column is itself");
        assert_eq!(filed(&b, "IN PROGRESS").as_deref(), Some("TODO"));
        assert_eq!(filed(&b, "REVIEW").as_deref(), Some("TODO"), "left first, past a closed one");
        assert_eq!(filed(&b, "NOPE"), None);
        // Nothing open to the left: the right side answers.
        let close = |b: &mut Board, n: &str| {
            let mut s = b.column(n).unwrap().settings.clone();
            s.refuses_new = true;
            b.set_column_settings(n, s)
        };
        close(&mut b, "TODO").unwrap();
        assert_eq!(filed(&b, "IN PROGRESS").as_deref(), Some("DONE"));
        assert_eq!(filed(&b, "TODO").as_deref(), Some("DONE"));
        assert_eq!(b.landing_column().as_deref(), Some("DONE"));
        // The last open column cannot close.
        let err = close(&mut b, "DONE").unwrap_err();
        assert!(err.contains("last column"), "{err}");
        assert!(!b.column("DONE").unwrap().settings.refuses_new, "a refusal changes nothing");
        // A board whose every column refuses (an older file, a delete)
        // files nowhere: the caller says so.
        b.column_mut("DONE").unwrap().settings.refuses_new = true;
        assert_eq!(filed(&b, "REVIEW"), None);
        assert_eq!(b.landing_column(), None);
    }

    #[test]
    fn rename_refuses_a_taken_or_unknown_name_and_allows_a_recase() {
        let mut b = template_board();
        assert!(b.rename_column("NOPE", "X").is_err());
        assert!(b.rename_column("TODO", "REVIEW").is_err());
        assert!(b.rename_column("TODO", "review").is_err(), "the header uppercases");
        assert_eq!(b.rename_column("TODO", "TODO").unwrap(), Vec::new());
        b.tickets.push(ticket(1, "TODO", "a"));
        assert_eq!(b.rename_column("TODO", "Todo").unwrap().len(), 1, "a recase of itself");
        assert_eq!(b.column("Todo").unwrap().settings.on_working.as_deref(), Some("IN PROGRESS"));
    }

    #[test]
    fn delete_refuses_live_tickets_and_the_last_column_and_clears_refs() {
        let mut b = template_board();
        b.tickets.push(ticket(1, "IN PROGRESS", "a"));
        b.tickets.push(ticket(2, "IN PROGRESS", "b"));
        assert_eq!(b.delete_column("IN PROGRESS").unwrap_err(), "move its 2 tickets first");
        b.tickets.clear();
        b.tickets.push(ticket(3, "IN PROGRESS", "a"));
        assert_eq!(b.delete_column("IN PROGRESS").unwrap_err(), "move its ticket first");
        let mut archived = ticket(3, "IN PROGRESS", "a");
        archived.archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        b.tickets = vec![archived];
        b.set_default_column(Some("IN PROGRESS")).unwrap();
        let cleared = b.delete_column("IN PROGRESS").unwrap();
        assert_eq!(
            cleared,
            vec![
                ("IN PROGRESS".to_string(), "default_column"),
                ("TODO".to_string(), "on_working"),
                ("REVIEW".to_string(), "on_working")
            ]
        );
        assert_eq!(b.default_column, None, "the default goes with its column");
        assert!(b.column("IN PROGRESS").is_none());
        assert_eq!(b.column("TODO").unwrap().settings.on_working, None);
        assert_eq!(
            b.ticket(ulid::Ulid(3)).unwrap().column,
            "IN PROGRESS",
            "an archive keeps its string"
        );
        assert!(b.delete_column("NOPE").is_err());
        let mut one = Board::default();
        one.columns.push(Column::new("ONLY", "a"));
        assert_eq!(one.delete_column("ONLY").unwrap_err(), "the last column stays");
    }

    fn names(b: &Board) -> Vec<String> {
        b.sorted_columns().iter().map(|c| c.name.clone()).collect()
    }

    #[test]
    fn add_and_reorder_keep_a_strict_order() {
        let mut b = template_board();
        b.add_column("QA".into(), Some("REVIEW")).unwrap();
        assert_eq!(names(&b), ["TODO", "IN PROGRESS", "REVIEW", "QA", "DONE"]);
        b.add_column("LATER".into(), None).unwrap();
        assert_eq!(names(&b), ["TODO", "IN PROGRESS", "REVIEW", "QA", "DONE", "LATER"]);
        assert!(b.add_column("qa".into(), None).is_err(), "taken, case aside");
        assert!(b.add_column("X".into(), Some("NOPE")).is_err());
        b.reorder_column("LATER", Some("TODO")).unwrap();
        assert_eq!(names(&b), ["LATER", "TODO", "IN PROGRESS", "REVIEW", "QA", "DONE"]);
        b.reorder_column("LATER", None).unwrap();
        assert_eq!(names(&b), ["TODO", "IN PROGRESS", "REVIEW", "QA", "DONE", "LATER"]);
        b.reorder_column("QA", Some("IN PROGRESS")).unwrap();
        assert_eq!(names(&b), ["TODO", "QA", "IN PROGRESS", "REVIEW", "DONE", "LATER"]);
        b.reorder_column("QA", Some("QA")).unwrap();
        assert_eq!(names(&b), ["TODO", "QA", "IN PROGRESS", "REVIEW", "DONE", "LATER"]);
        let orders: Vec<&str> = b.sorted_columns().iter().map(|c| c.order.as_str()).collect();
        let mut sorted = orders.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(orders, sorted, "orders stay strictly increasing: {orders:?}");
    }

    #[test]
    fn set_column_settings_validates_the_references() {
        let mut b = template_board();
        let s = ColumnSettings { on_working: Some("TODO".into()), ..Default::default() };
        assert!(b.set_column_settings("TODO", s).unwrap_err().contains("itself"));
        let s = ColumnSettings { on_done: Some("NOPE".into()), ..Default::default() };
        assert!(b.set_column_settings("TODO", s).unwrap_err().contains("no such column"));
        let s = ColumnSettings { on_merged: Some("REVIEW".into()), ..Default::default() };
        assert_eq!(
            b.set_column_settings("REVIEW", s).unwrap_err(),
            "when merged: a column cannot move a ticket to itself"
        );
        let s = ColumnSettings { on_merged: Some("NOPE".into()), ..Default::default() };
        assert!(b.set_column_settings("REVIEW", s).unwrap_err().contains("no such column"));
        let s = ColumnSettings {
            on_done: Some("DONE".into()),
            agent_tools: AgentTools::Read,
            ..Default::default()
        };
        b.set_column_settings("TODO", s.clone()).unwrap();
        assert_eq!(b.column("TODO").unwrap().settings, s);
        assert!(b.set_column_settings("NOPE", ColumnSettings::default()).is_err());
    }

    #[test]
    fn sort_column_by_each_key() {
        let mut b = template_board();
        let mut t1 = ticket(1, "TODO", "c");
        t1.entered_at = Some("@30".into());
        let mut t2 = ticket(2, "TODO", "a");
        t2.entered_at = Some("@10".into());
        let mut t3 = ticket(3, "TODO", "b");
        t3.created_at = "@20".into(); // no entered_at: column_since falls back
        b.tickets.extend([t1, t2, t3]);
        b.tickets.push(ticket(9, "REVIEW", "a"));
        let order =
            |b: &Board| -> Vec<u128> { b.column_tickets("TODO").iter().map(|t| t.id.0).collect() };
        assert_eq!(order(&b), [2, 3, 1], "manual order to start");
        let none = std::collections::HashSet::new();
        let touched = b.sort_column("TODO", SortBy::Key, &none);
        assert_eq!(order(&b), [1, 2, 3]);
        assert!(!touched.is_empty());
        assert_eq!(b.ticket(ulid::Ulid(9)).unwrap().order, "a", "another column is untouched");
        b.sort_column("TODO", SortBy::NewestArrival, &none);
        assert_eq!(order(&b), [1, 3, 2]);
        b.sort_column("TODO", SortBy::OldestArrival, &none);
        assert_eq!(order(&b), [2, 3, 1]);
        let needs: std::collections::HashSet<_> = [ulid::Ulid(1)].into_iter().collect();
        b.sort_column("TODO", SortBy::NeedsYouFirst, &needs);
        assert_eq!(order(&b), [1, 2, 3], "needs-you first, the rest keeping their order");
        assert!(
            b.sort_column("TODO", SortBy::NeedsYouFirst, &needs).is_empty(),
            "a no-op touches nothing"
        );
        for t in b.column_tickets("TODO") {
            assert!(t.entered_at.is_some() || t.id == ulid::Ulid(3), "a sort is not a move");
        }
    }

    /// T-283. The three decisions in one test: the tags sort in the PICKER's
    /// row order and never by name, every axis counts with 1 deciding, and a
    /// ticket wearing nothing on an axis falls below every tag on it.
    #[test]
    fn sort_column_by_tag_follows_the_picker_row() {
        let mut b = template_board();
        // Registered out of alphabetical order on purpose: FEATURE is the row's
        // first cell, so its cards must come out first.
        b.register_tag(1, "FEATURE").unwrap();
        b.register_tag(1, "BUG").unwrap();
        b.register_tag(2, "RESEARCH").unwrap();
        b.register_tag(2, "QUESTION").unwrap();
        for id in 1..=5u128 {
            b.tickets.push(ticket(id, "TODO", &format!("{id}")));
        }
        fn tag(b: &mut Board, id: u128, group: u8, name: &str) {
            let t = b.tickets.iter_mut().find(|t| t.id == ulid::Ulid(id)).expect("ticket");
            t.set_tag(group, Some(name.to_string()));
        }
        tag(&mut b, 1, 1, "BUG");
        tag(&mut b, 2, 1, "FEATURE");
        tag(&mut b, 2, 2, "QUESTION");
        tag(&mut b, 3, 1, "FEATURE");
        tag(&mut b, 3, 2, "RESEARCH");
        tag(&mut b, 5, 2, "RESEARCH"); // nothing on axis 1, something on axis 2
        let order =
            |b: &Board| -> Vec<u128> { b.column_tickets("TODO").iter().map(|t| t.id.0).collect() };
        assert_eq!(order(&b), [1, 2, 3, 4, 5], "manual order to start");
        let none = std::collections::HashSet::new();

        b.sort_column("TODO", SortBy::Tag, &none);
        assert_eq!(
            order(&b),
            [3, 2, 1, 5, 4],
            "FEATURE's cards first because FEATURE is the row's first cell (not because \
             B sorts before F), RESEARCH ahead of QUESTION inside them for the same reason, \
             then BUG, then the axis-1 untagged — T-5 above T-4 because it at least wears \
             something on axis 2"
        );

        // The payoff: carry FEATURE right in the picker and its clump sinks.
        b.move_tag(1, "FEATURE", 1, 1).unwrap();
        assert_eq!(b.group_tags(1), ["BUG", "FEATURE"]);
        b.sort_column("TODO", SortBy::Tag, &none);
        assert_eq!(order(&b), [1, 3, 2, 5, 4], "the cards followed the row");

        // Ties keep the order they are in: two cards that wear exactly the
        // same tags stay in the order the column already had them.
        tag(&mut b, 2, 2, "RESEARCH");
        b.sort_column("TODO", SortBy::Tag, &none);
        assert_eq!(order(&b), [1, 3, 2, 5, 4], "same tags, previous order kept");
        assert!(b.sort_column("TODO", SortBy::Tag, &none).is_empty(), "a no-op touches nothing");
        for t in b.column_tickets("TODO") {
            assert!(t.entered_at.is_none(), "a sort is not a move");
        }

        // A board with no vocabulary at all has nothing to say: every card
        // ranks the same, and a stable sort leaves them where they were.
        let mut bare = template_board();
        bare.tickets.push(ticket(7, "TODO", "b"));
        bare.tickets.push(ticket(8, "TODO", "a"));
        bare.sort_column("TODO", SortBy::Tag, &none);
        assert_eq!(order(&bare), [8, 7]);
    }

    #[test]
    fn column_name_is_scrubbed_bounded_and_nonblank() {
        assert_eq!(sanitize_column_name("  QA \u{1b}[31m "), Some("QA [31m".into()));
        assert_eq!(sanitize_column_name("   "), None);
        let long = "x".repeat(COLUMN_NAME_MAX_BYTES + 5);
        assert_eq!(sanitize_column_name(&long).unwrap().len(), COLUMN_NAME_MAX_BYTES);
        assert_eq!(COLUMN_NAME_MAX_BYTES, TAG_MAX_BYTES);
    }

    /// T-467: a description is one line agents read and the dialog draws.
    #[test]
    fn column_description_is_one_scrubbed_bounded_line() {
        assert_eq!(
            sanitize_column_description(" ideas\n not yet\u{202e} planned \u{1b}[31m"),
            Some("ideas not yet planned [31m".into())
        );
        assert_eq!(sanitize_column_description(" \t "), None);
        let long = "é".repeat(COLUMN_DESCRIPTION_MAX_BYTES);
        let cut = sanitize_column_description(&long).unwrap();
        assert!(cut.len() <= COLUMN_DESCRIPTION_MAX_BYTES && cut.len().is_multiple_of(2));
        let s = ColumnSettings { description: Some("someday".into()), ..Default::default() };
        assert_eq!(s.summary(), ["about: someday"]);
        assert!(!s.automated());
    }

    /// D10: a column narrows what the user configured, never hands out a mode
    /// nobody asked for. The enum cannot spell the two escape hatches.
    #[test]
    fn codex_column_policy_defaults_and_roundtrip_do_not_translate_claude_mode() {
        let legacy: ColumnSettings = serde_json::from_str(r#"{"claude_mode":"plan"}"#).unwrap();
        assert_eq!(legacy.codex_sandbox, CodexSandbox::Inherit);
        assert_eq!(legacy.codex_approval, CodexApproval::Inherit);
        let configured = ColumnSettings {
            codex_sandbox: CodexSandbox::WorkspaceWrite,
            codex_approval: CodexApproval::OnRequest,
            ..legacy
        };
        let wire = serde_json::to_string(&configured).unwrap();
        assert_eq!(serde_json::from_str::<ColumnSettings>(&wire).unwrap(), configured);
        assert_eq!(configured.claude_mode, ClaudeMode::Plan);
        let empty = serde_json::to_value(ColumnSettings::default()).unwrap();
        assert!(empty.get("codex_sandbox").is_none());
        assert!(empty.get("codex_approval").is_none());
    }

    #[test]
    fn claude_mode_never_emits_bypass() {
        for m in [ClaudeMode::Inherit, ClaudeMode::Auto, ClaudeMode::Plan, ClaudeMode::Manual] {
            let w = m.flag_word().unwrap_or("");
            assert!(
                !["bypassPermissions", "dontAsk", "acceptEdits", "default"].contains(&w),
                "{w}"
            );
            assert_eq!(m.next().next().next().next(), m);
        }
        assert_eq!(
            ClaudeMode::Manual.flag_word(),
            Some("manual"),
            "never `default`, the flag refuses it"
        );
    }

    #[test]
    fn agent_tools_are_ordered_and_named() {
        assert!(AgentTools::Off < AgentTools::Read);
        assert!(AgentTools::Read < AgentTools::Annotate);
        assert!(AgentTools::Annotate < AgentTools::Full);
        for t in [AgentTools::Off, AgentTools::Read, AgentTools::Annotate, AgentTools::Full] {
            assert_eq!(AgentTools::parse(t.word()), Some(t));
        }
        assert_eq!(AgentTools::parse("all"), None);
        assert_eq!(AgentTools::default(), AgentTools::Full);
    }

    #[test]
    fn the_settings_summary_says_only_what_was_chosen() {
        assert!(ColumnSettings::default().summary().is_empty());
        let s = template_settings("REVIEW").unwrap();
        assert_eq!(
            s.summary(),
            [
                "takes no new tickets",
                "working → IN PROGRESS",
                "merged → DONE",
                "merge train: auto-merge"
            ]
        );
        assert!(s.automated());
        assert!(!template_settings("DONE").unwrap().automated());
    }

    /// T-891: the arrival rule's choice among columns. Among equal stages
    /// the right-most wins, a stage beats board order, a deploy column left
    /// of the ticket's own never takes it, and a merge column left of it
    /// still does.
    #[test]
    fn an_arrival_takes_the_right_most_of_equals_and_a_deploy_never_moves_left() {
        let col = |name: &str, order: &str, on: Arrival, env: Option<&str>| {
            let mut c = Column::new(name, order);
            c.settings.arrives_on = on;
            c.settings.arrives_env = env.map(str::to_string);
            c
        };
        let cols = [
            col("TODO", "a", Arrival::Nothing, None),
            col("OPEN", "b", Arrival::PrOpened, None),
            col("MERGED", "c", Arrival::Merged, None),
            col("DEV", "d", Arrival::Deployed, Some("dev")),
            col("STAGING", "e", Arrival::Deployed, Some("staging")),
            col("PROD", "f", Arrival::Deployed, Some("production")),
            col("DONE", "g", Arrival::Nothing, None),
        ];
        let cols: Vec<&Column> = cols.iter().collect();
        let deployed = |envs: &'static [&'static str]| {
            move |c: &Column| {
                c.settings.arrives_on == Arrival::Deployed
                    && c.settings.arrives_env.as_deref().is_some_and(|e| envs.contains(&e))
            }
        };
        // Three environments on one pass: the right-most.
        let three = deployed(&["dev", "staging", "production"]);
        assert_eq!(arrival_dest(&cols, "MERGED", three), Some("PROD"));
        // From production, dev and staging a pass later: no move back.
        let two = deployed(&["dev", "staging"]);
        assert_eq!(arrival_dest(&cols, "PROD", two), None);
        assert_eq!(arrival_dest(&cols, "STAGING", two), None);
        assert_eq!(arrival_dest(&cols, "DEV", two), Some("STAGING"));
        // Nor from DONE, right of every environment.
        assert_eq!(arrival_dest(&cols, "DONE", three), None);
        // A stage beats board order: deployed to dev over merged.
        let dev_and_merge =
            |c: &Column| c.settings.arrives_on == Arrival::Merged || deployed(&["dev"])(c);
        assert_eq!(arrival_dest(&cols, "OPEN", dev_and_merge), Some("DEV"));
        // A follow-up PR's merge takes a shipped ticket back to MERGED.
        let merged = |c: &Column| c.settings.arrives_on == Arrival::Merged;
        assert_eq!(arrival_dest(&cols, "PROD", merged), Some("MERGED"));
        // The column it is in never takes it, and nothing satisfied is None.
        assert_eq!(arrival_dest(&cols, "MERGED", merged), None);
        assert_eq!(arrival_dest(&cols, "TODO", |_| false), None);
    }
}
