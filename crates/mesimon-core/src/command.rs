//! The typed command envelope (D22/D32c): every mutation, from any client,
//! travels as one of these, carries its principal, and passes `authorize()`.
//! Wire: newline-delimited JSON over the daemon's unix socket.

use serde::{Deserialize, Serialize};

use crate::board::{Board, SessionKind};
use crate::Principal;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub principal: Principal,
    pub command: Command,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// First message on every connection. Reserved fields are the D32c/D33a
    /// seams: identity & caps travel even when both are trivial.
    Hello { version: u32, client: String },
    Snapshot,
    Subscribe,
    CreateTicket { column: String, title: String },
    RenameTicket { id: ulid::Ulid, title: String },
    /// Starts the grace band; the ticket vanishes from snapshots immediately
    /// and is destroyed when the band expires (D21).
    DeleteTicket { id: ulid::Ulid },
    /// Undo within the grace band.
    RestoreTicket { id: ulid::Ulid },
    MoveTicket { id: ulid::Ulid, column: String, before: Option<ulid::Ulid> },
    SpawnSession { ticket: ulid::Ulid, kind: SessionKind },
    KillSession { id: uuid::Uuid },
    /// Re-run the external-session census (19 §4 tier 1). Lazy by design:
    /// fired when the drawer opens, never on a timer.
    RescanExternal,
    /// Tier 2: import a discovered foreign session as an `external` record —
    /// observe-only, no process, no hooks. `ticket: None` mints a fresh
    /// ticket named after the session (the drawer's default gesture).
    AttachExternal { claude_session_id: uuid::Uuid, ticket: Option<ulid::Ulid> },
    /// Tier 3 in one step: import + take over via `claude --resume`.
    ResumeExternal { claude_session_id: uuid::Uuid, ticket: Option<ulid::Ulid>, confirm: bool },
    /// Take over (or re-take) an attached record. `confirm` overrides the
    /// running-elsewhere refusal (double-resume guard, 09 §9).
    ResumeSession { id: uuid::Uuid, confirm: bool },
    SleepSession { id: uuid::Uuid },
    WakeSession { id: uuid::Uuid },
    /// Sleep every eligible session (04's reclaim).
    ReclaimAll,
    PinAwake { id: uuid::Uuid, pinned: bool },
    /// Exclusive-focus token (D22). Grants the attach argv for the handover.
    FocusStart { session: uuid::Uuid },
    FocusEnd { session: uuid::Uuid },
    /// Has the GATE ceremony been passed on this machine?
    GateStatus,
    GatePassed,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "resp", rename_all = "snake_case")]
pub enum Response {
    Hello { version: u32, daemon_pid: u32 },
    Ok,
    Spawned { id: uuid::Uuid },
    /// ReclaimAll's receipt: how many actually slept, and why others did not.
    Reclaimed { slept: usize, skipped: usize },
    Board {
        board: Board,
        grace: Vec<GraceItem>,
        #[serde(default)]
        external: Vec<ExternalItem>,
        #[serde(default)]
        resources: Resources,
    },
    /// argv the client should exec for the focus handover.
    Attach { argv: Vec<String> },
    Gate { passed: bool, attach_argv: Option<Vec<String>> },
    Err { message: String },
}

/// A deleted ticket riding out its grace band (D21): shown as a ghost row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraceItem {
    pub id: ulid::Ulid,
    pub short_key: String,
    pub title: String,
    pub expires_in_secs: u64,
    pub live_sessions: usize,
}

/// A discovered foreign session (19 §4 tier 1) — daemon-computed on rescan,
/// never persisted; only attach/takeover mints a `SessionRecord` from one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalItem {
    pub claude_session_id: uuid::Uuid,
    pub cwd: String,
    pub transcript_path: String,
    pub mtime_ms: u64,
    /// Last assistant text, truncated by the daemon.
    pub preview: Option<String>,
    /// Display name from `sessions/<pid>.json`, when one matched (11 §11.3).
    pub name: Option<String>,
    /// A live pid claims this session right now — resuming it would
    /// interleave two writers into one transcript (09 §9).
    pub running_elsewhere: bool,
}

/// The header resource figures (D33e). Real measurements only — RSS is a
/// `ps` aggregate over tracked sessions, never a constant times a count
/// (spike ASK-23); the PTY count is the allocation high-water mark (B-D16).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resources {
    pub live: usize,
    pub asleep: usize,
    pub rss_bytes: u64,
    /// How many sessions the RSS aggregate actually saw.
    pub rss_measured: usize,
    pub pty_used: u32,
    pub pty_total: u32,
    /// Remaining spawn budget before the OS boundary (14 §5.1).
    pub pty_budget: u32,
}

/// Pushed to subscribed clients whenever board state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    BoardChanged,
}
