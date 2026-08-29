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
    Board { board: Board, grace: Vec<GraceItem> },
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

/// Pushed to subscribed clients whenever board state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    BoardChanged,
}
