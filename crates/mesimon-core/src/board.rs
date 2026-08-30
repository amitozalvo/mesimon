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
    RequiresAction { reason: Reason },
    Idle { stop_reason: StopReason },
    Sleeping,
    Exited { reason: ExitReason },
    Failed { reason: FailReason },
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
    /// Manual override: never sleep this session (D23 guard, third part).
    #[serde(default)]
    pub pinned_awake: bool,
}

impl SessionRecord {
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
            pinned_awake: false,
        }
    }

    /// The tmux session name for this record: first 16 hex chars of the UUID.
    pub fn sid16(&self) -> String {
        self.id.simple().to_string()[..16].to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// Fractional index; columns sort by it.
    pub order: String,
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
    /// Per-ticket workspace strategy (M4 layering: the ticket field is the truth;
    /// a column policy only defaults NEW tickets, M5). `None` = inherit the board
    /// default. Must stay after the scalar fields (TOML serialize order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceStrategy>,
    /// Archival is a field, not a directory move (13 §data-model) — the ticket
    /// keeps its column and order, so restore is exact. Must stay last: a TOML
    /// table; any scalar serialized after it errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<Archived>,
}

/// The `[archived]` table on a ticket. Presence = off the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    /// Same clock as `created_at` (`@<unix secs>`).
    pub at: String,
    /// Actor. v0.1 has no user@host plumbing — always "local" (STALE-MAP).
    #[serde(default)]
    pub by: String,
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

/// Layer-0 board default until column policies land (M5).
pub const DEFAULT_WORKSPACE: WorkspaceStrategy = WorkspaceStrategy::SharedCheckout;

impl Ticket {
    /// Layered resolution: ticket field, else the board default (column default is M5).
    pub fn workspace_strategy(&self) -> WorkspaceStrategy {
        self.workspace.unwrap_or(DEFAULT_WORKSPACE)
    }

    pub fn is_archived(&self) -> bool {
        self.archived.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Board {
    pub columns: Vec<Column>,
    pub tickets: Vec<Ticket>,
    pub sessions: Vec<SessionRecord>,
    /// Counter feeding short keys (T-1, T-2, …).
    pub next_key: u64,
}

/// D33i: the shipped default template.
pub const DEFAULT_COLUMNS: [&str; 4] = ["TODO", "IN PROGRESS", "REVIEW", "DONE"];

impl Board {
    pub fn with_default_columns() -> Self {
        let mut b = Board::default();
        let mut prev = String::new();
        for name in DEFAULT_COLUMNS {
            let order = crate::fracindex::between(&prev, "");
            b.columns.push(Column { name: name.into(), order: order.clone() });
            prev = order;
        }
        b
    }

    pub fn ticket(&self, id: ulid::Ulid) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == id)
    }

    pub fn ticket_mut(&mut self, id: ulid::Ulid) -> Option<&mut Ticket> {
        self.tickets.iter_mut().find(|t| t.id == id)
    }

    /// Tickets of one column, sorted by fractional order (ties by id for stability).
    /// Archived tickets stay in `tickets` (the ticket page needs them in the
    /// snapshot) but never surface here — this is the board's one chokepoint.
    pub fn column_tickets(&self, column: &str) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| t.column == column && !t.is_archived())
            .collect();
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

    /// Sessions of the ticket that hold (or should hold) a pane. The archive
    /// gate, its TUI advisory, and the header suggestion all share this — the
    /// suggestion never offers what the keystroke would refuse.
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
        assert!(!rec.pinned_awake);
    }

    fn ticket(id: u128, column: &str, order: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid(id),
            short_key: format!("T-{id}"),
            title: "t".into(),
            column: column.into(),
            order: order.into(),
            created_at: "@0".into(),
            workspace: None,
            archived: None,
        }
    }

    /// Archived tickets stay in `tickets` but never surface on the board.
    #[test]
    fn column_tickets_excludes_archived() {
        let mut b = Board::with_default_columns();
        b.tickets.push(ticket(1, "DONE", "a"));
        let mut t = ticket(2, "DONE", "b");
        t.archived = Some(Archived { at: "@10".into(), by: "local".into() });
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
            t.archived = Some(Archived { at: at.into(), by: "local".into() });
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
}
