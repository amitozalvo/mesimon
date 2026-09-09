use serde::{Deserialize, Serialize};

/// Session kinds v0.1 actually spawns. Notes are files, not sessions (D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Claude,
    Bash,
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
    /// The turn ended but the agent is PARKED, not done: the Stop payload
    /// carried a `background_tasks[]` entry that means work is still in
    /// flight (`attention::task_blocks_end_turn`). Nothing is painting the
    /// pane and nobody is waiting on the user — the task's completion
    /// notification arrives as a `UserPromptSubmit` and the turn resumes.
    ///
    /// It exists because the two states either side of it are both lies.
    /// Re-asserting `Running` (what shipped first) is contradicted within
    /// seconds by the quiet probe, which then demotes to `Interrupted` — and
    /// nothing interrupted it (dogfood 2026-09-01: a backgrounded build-poll
    /// left T-128 with no card glyph at all for two minutes). Only `EndTurn`
    /// promotes a ticket, so parking here also keeps the card in IN PROGRESS,
    /// which is the truthful place for it.
    Background,
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

/// A session record the daemon persists. The UUID is minted by mesimon and
/// passed to the agent (`--session-id`) and to tmux (session name = sid16),
/// so identity is never discovered (D24).
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
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default)]
    pub provenance: Provenance,
    /// The claude-side session id when it differs from `id` — set for adopted
    /// sessions, and relearned from the SessionStart transcript filename when
    /// an in-app /resume hands the pane a different conversation. Stays None
    /// while the pane hosts the record's own minted conversation
    /// (mesimon-spawned ones pass `--session-id id`, so the two coincide).
    #[serde(default)]
    pub claude_session_id: Option<uuid::Uuid>,
    /// The ticket title was typed into this session's box and is still
    /// waiting for its Enter. Spike T-5 arm C (2026-08-31): an Enter sent in
    /// the same breath as the text is swallowed by Claude's paste detection,
    /// so the submit is deferred to the `SessionStart` frame — the earliest
    /// point the pane provably accepts a keystroke as a keystroke. Cleared
    /// the moment it is delivered; a session that never starts just keeps the
    /// prefill, which is the ordinary spawn's behaviour anyway.
    #[serde(default)]
    pub pending_submit: bool,
    /// Named in-process teammates that reported idle and have not been
    /// messaged since (T-135). A Stop payload lists a teammate as `running`
    /// for its whole life, so this is what lets the attention machine tell a
    /// lead whose reviewers are still working from one whose reviewers are
    /// done; persisted because a daemon restart that forgot it would park the
    /// next finished turn for good. Sorted, deduplicated.
    #[serde(default)]
    pub idle_teammates: Vec<String>,
    /// IDs returned by this conversation's top-level Monitor tool. Claude can
    /// label these dormant watches as `shell` in Stop; identity preserves their
    /// meaning across daemon restarts. No commands or prompt text are stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitor_task_ids: Vec<String>,
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
}

impl SessionRecord {
    /// Still owed the deferred Enter of a Shift+Enter spawn, in a state where
    /// pressing it is safe. A pane that died, or one showing a startup
    /// modal, is not a pane to keep pressing Enter into — the modal's Enter
    /// is an ANSWER, and mesimon does not answer dialogs on the user's behalf.
    pub fn pressable(&self) -> bool {
        self.pending_submit
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
            confidence: Confidence::default(),
            provenance: Provenance::default(),
            claude_session_id: None,
            pending_submit: false,
            idle_teammates: Vec::new(),
            monitor_task_ids: Vec::new(),
            plan_note: None,
            ticket_read: false,
        }
    }

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
    /// column-settings goldens read `Sort now: newest first`.
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

/// Everything a column decides (T-117). Every automation the daemon runs
/// on a ticket is a field here, read off the ticket's column through
/// `Board::column` and never off a column NAME: `on_working`/`on_done` ARE
/// automove, `train` is the merge train's reach, `requires_merge` is the
/// DONE gate, `reclaim` is the sleep/archive offer. The defaults are what a
/// column with no template settings did before: nothing moves, tools full,
/// mode inherited, train off. Each default is skipped on disk so a file
/// says only what was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ColumnSettings {
    /// Pinned as a one-cell spine unless the cursor is in it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
    /// The workspace a ticket CREATED here starts with, stamped onto the
    /// ticket at mint; the ticket field stays the truth and a change here is
    /// never retroactive. `None` = the board default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
    #[serde(default, skip_serializing_if = "ClaudeMode::is_inherit")]
    pub claude_mode: ClaudeMode,
    #[serde(default, skip_serializing_if = "AgentTools::is_full")]
    pub agent_tools: AgentTools,
    /// A ticket a PERSON creates here gets claude started on its title, the
    /// brief pasted and submitted — the composer's Shift+Enter, fired by the
    /// daemon. Creation only: never a move, an agent's `create_ticket`, a
    /// wake, an unarchive or an undo.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_run: bool,
    /// The column a ticket moves to when its claude starts working
    /// (`Running` at Medium or better). `None` = stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_working: Option<String>,
    /// The column a ticket moves to when its claude ends a turn
    /// (`Idle{EndTurn}` at Medium or better). `None` = stay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_done: Option<String>,
    /// Entry is refused while the ticket's worktree branch is unmerged.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_merge: bool,
    /// The header's sleep and archive offers, `X` and `Z` price this column.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reclaim: bool,
    #[serde(default, skip_serializing_if = "TrainReach::is_off")]
    pub train: TrainReach,
}

impl ColumnSettings {
    /// Whether any automation would act on a ticket here — the header's one
    /// optional mark.
    pub fn automated(&self) -> bool {
        self.auto_run
            || self.on_working.is_some()
            || self.on_done.is_some()
            || self.train != TrainReach::Off
    }

    /// The non-default settings in words, for `doctor` and the dialog.
    pub fn summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.collapsed {
            out.push("collapsed".into());
        }
        if let Some(w) = self.workspace {
            out.push(format!("workspace: {}", w.word()));
        }
        if self.claude_mode != ClaudeMode::Inherit {
            out.push(format!("claude: {}", self.claude_mode.word()));
        }
        if self.agent_tools != AgentTools::Full {
            out.push(format!("agent tools: {}", self.agent_tools.word()));
        }
        if self.auto_run {
            out.push("starts claude on creation".into());
        }
        if let Some(c) = &self.on_working {
            out.push(format!("working → {c}"));
        }
        if let Some(c) = &self.on_done {
            out.push(format!("done → {c}"));
        }
        if self.requires_merge {
            out.push("entry needs a merged branch".into());
        }
        if self.reclaim {
            out.push("offers sleep + archive".into());
        }
        if self.train != TrainReach::Off {
            out.push(format!("train: {}", self.train.word()));
        }
        out
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
    /// Per-ticket workspace strategy (M4 layering: the ticket field is the truth;
    /// a column's `workspace` setting only defaults a NEW ticket, stamped here at
    /// mint — T-117). `None` = inherit the board default. Must stay after the
    /// scalar fields (TOML serialize order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
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

/// The longest a raised hand's reason may be, in bytes (T-107). One line on
/// the cursor card and one clause on the ticket page's state row — the mark
/// is a POINTER and the transcript is the record, so the bound is a card
/// row's worth of words rather than a note's.
pub const RAISE_REASON_MAX_BYTES: usize = 160;

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

/// Layer-0 board default. A column's `workspace` setting defaults a ticket
/// CREATED in it by stamping the ticket field at mint (T-117), so this stays
/// the answer for a ticket nothing stamped.
pub const DEFAULT_WORKSPACE: WorkspaceStrategy = WorkspaceStrategy::SharedCheckout;

impl Ticket {
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

    /// Layered resolution: ticket field (a column's default is stamped here
    /// at mint), else the board default.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub columns: Vec<Column>,
    pub tickets: Vec<Ticket>,
    pub sessions: Vec<SessionRecord>,
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
    /// (`brief::FLAG`, T-224). OFF by default — README promise 3 says mesimon
    /// adds no token to a conversation, and this is the one exception a person
    /// turns on, after a dialog has shown them the exact text. Per REPO like
    /// `mcp_tools`, and honoured only while the tools are on (the sentence
    /// names a tool). A plain serde default, no schema bump: a downgrade that
    /// drops `system_prompt = true` sends LESS to the model, which is the safe
    /// direction — the `mcp_tools` bump exists because dropping `false` sends
    /// more.
    #[serde(default)]
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
}

/// `Board::mcp_tools` defaults ON: a serde default has to be a function, and
/// this is the whole of it.
fn yes() -> bool {
    true
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
            sessions: Vec::new(),
            next_key: 0,
            tags: Vec::new(),
            tags_seeded: false,
            mcp_tools: true,
            claude_md_ignored: false,
            system_prompt: false,
            default_column: None,
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
        "IN PROGRESS" => {
            ColumnSettings { on_done: Some("REVIEW".into()), train: TrainReach::Rebase, ..d() }
        }
        "REVIEW" => ColumnSettings {
            on_working: Some("IN PROGRESS".into()),
            train: TrainReach::Merge,
            ..d()
        },
        "DONE" => ColumnSettings { requires_merge: true, reclaim: true, ..d() },
        _ => return None,
    })
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

    /// Give every column named like the template its template settings, then
    /// drop any `on_working`/`on_done` naming a column this board does not
    /// have (a quarantine-rebuilt board may lack one). The v3→v4 road and the
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

    /// Clear every `on_working`/`on_done` that names a column the board does
    /// not have. Returns `(column, field)` for each one cleared, so the
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
        }
        out
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The column a ticket lands in when nothing named one (T-279): the
    /// chosen default while the board still has it, else the first column.
    /// `None` only on a board with no columns at all.
    pub fn landing_column(&self) -> Option<String> {
        if let Some(d) = self.default_column.as_deref().and_then(|n| self.column(n)) {
            return Some(d.name.clone());
        }
        self.sorted_columns().first().map(|c| c.name.clone())
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
    /// column's `on_working`/`on_done` naming it. Returns the ticket ids
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
    /// `on_working`/`on_done` pointing at it; returns those, as
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

    /// Replace a column's settings whole. `on_working`/`on_done` must name a
    /// column the board has, other than this one — a self-loop is refused,
    /// not resolved.
    pub fn set_column_settings(&mut self, name: &str, s: ColumnSettings) -> Result<(), String> {
        if self.column(name).is_none() {
            return Err(format!("no such column: {name}"));
        }
        for (field, target) in [("working", &s.on_working), ("done", &s.on_done)] {
            if let Some(t) = target {
                if t == name {
                    return Err(format!("when {field}: a column cannot move a ticket to itself"));
                }
                if self.column(t).is_none() {
                    return Err(format!("when {field}: no such column: {t}"));
                }
            }
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

    /// The columns whose tickets the sleep and archive offers, `X` and `Z`
    /// price (`reclaim`).
    pub fn reclaim_columns(&self) -> std::collections::HashSet<&str> {
        self.columns.iter().filter(|c| c.settings.reclaim).map(|c| c.name.as_str()).collect()
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

    /// Every ticket that needs the user, by any of the three roads: an
    /// attention-set session on it, a snooze that woke it (T-74), or an
    /// agent's raised hand (T-107). A SET, because the roads overlap — a
    /// woken ticket whose claude is also at a permission prompt is one
    /// ticket needing one person, and counting it twice made `!N` a number
    /// nothing on screen could be matched against.
    pub fn needs_you_tickets(&self) -> std::collections::HashSet<ulid::Ulid> {
        crate::attention::attention_queue(self)
            .iter()
            .map(|s| s.ticket)
            .chain(self.woke_tickets().iter().map(|t| t.id))
            .chain(self.raised_tickets().iter().map(|t| t.id))
            .collect()
    }

    /// The `!N` count: tickets needing the user, counted once each. The
    /// header chip and the tmux status line both read this, so the number is
    /// one number.
    pub fn needs_you_count(&self) -> usize {
        self.needs_you_tickets().len()
    }

    /// Sessions of the ticket that hold (or should hold) a pane. The archive
    /// gate, its TUI advisory, and the header suggestion all share this — the
    /// suggestion never offers what the keystroke would refuse.
    /// The one live claude a prompt from the board reaches: first in spawn
    /// order, the same session `board_enter` focuses, so the key that asks
    /// and the key that goes there land on one pane. `has_pane` and not
    /// `is_live` — a parked session is live and has no process to type at.
    /// The daemon picks by this and the TUI hints by it; one predicate.
    pub fn pane_target(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.sessions
            .iter()
            .find(|s| s.ticket == ticket && s.kind == SessionKind::Claude && s.state.has_pane())
    }

    /// The claude a ticket already holds, parked or not — `is_live`, so a
    /// Sleeping record counts. A ticket holds ONE claude (2026-09-02): the
    /// daemon refuses a second spawn by this, and `c` wakes rather than
    /// starts by the same fact. A second seat on a ticket is a shell.
    pub fn live_claude(&self, ticket: ulid::Ulid) -> Option<&SessionRecord> {
        self.sessions
            .iter()
            .find(|s| s.ticket == ticket && s.kind == SessionKind::Claude && s.state.is_live())
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
    use super::*;

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
            woke_at: None,
            manual_merge: false,
            raised: None,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        }
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
        assert_eq!(b.needs_you_count(), 1);
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
        assert_eq!(b.needs_you_count(), 1);

        // An archived ticket is off the board, so its hand is not counted —
        // `woke_tickets`' rule, and the reason `agent_raise_hand` refuses one.
        b.tickets[0].archived =
            Some(Archived { at: "@1".into(), by: "local".into(), until: None, needs_you: false });
        assert!(b.raised_tickets().is_empty());
        assert_eq!(b.needs_you_count(), 0);
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
        assert_eq!(b.needs_you_count(), 1, "one ticket, one person, one number");
        assert_eq!(stamp_secs("7"), None);
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
    }

    #[test]
    fn default_column_is_the_landing_column_and_follows_a_rename() {
        let mut b = template_board();
        assert_eq!(b.landing_column().as_deref(), Some("TODO"), "unset means the first column");
        assert_eq!(b.set_default_column(Some("NOPE")).unwrap_err(), "no such column: NOPE");
        assert_eq!(b.default_column, None, "a refusal changes nothing");
        b.set_default_column(Some("REVIEW")).unwrap();
        assert_eq!(b.landing_column().as_deref(), Some("REVIEW"));
        b.rename_column("REVIEW", "QA").unwrap();
        assert_eq!(b.default_column.as_deref(), Some("QA"), "a rename carries it");
        assert_eq!(b.landing_column().as_deref(), Some("QA"));
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
        let s =
            ColumnSettings { on_done: Some("DONE".into()), auto_run: true, ..Default::default() };
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

    /// D10: a column narrows what the user configured, never hands out a mode
    /// nobody asked for. The enum cannot spell the two escape hatches.
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
        assert_eq!(s.summary(), ["working → IN PROGRESS", "train: auto-merge"]);
        assert!(s.automated());
        assert!(!template_settings("DONE").unwrap().automated());
    }
}
