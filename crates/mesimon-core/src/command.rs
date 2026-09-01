//! The typed command envelope (D22/D32c): every mutation, from any client,
//! travels as one of these, carries its principal, and passes `authorize()`.
//! Wire: newline-delimited JSON over the daemon's unix socket.

use serde::{Deserialize, Serialize};

use crate::board::{Board, SessionKind, WorkspaceStrategy};
use crate::Principal;

pub const PROTOCOL_VERSION: u32 = 1;

/// A build fingerprint for the executable a process runs from: the mtime (ms
/// since epoch) and length of that file, read when the process started. Not a
/// content hash — hashing a 30 MB debug binary on every connect is not free,
/// and (mtime, len) is precisely the signal `tui/src/update.rs` already trusts
/// for the `update ready` offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExeStamp {
    pub mtime_ms: u64,
    pub len: u64,
}

/// One non-fatal thing the user should know about persisted state (13 §13.10.3's
/// quarantine banner, 16 §6.2's schema banner). Standing, not transient: it
/// describes a condition still true on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Machine tag — a WORD, not an enum, exactly as `WorktreeItem.status` is.
    /// An unknown enum variant from a newer daemon would fail the whole
    /// `Response::Board` deserialize, and the client DROPS a line it cannot
    /// parse (tui/src/client.rs) — one new notice kind would blank the board
    /// on an older client. One of:
    /// `quarantined` | `future_version` | `worktrees_barred` | `build_skew` |
    /// `shell_env`.
    pub kind: String,
    /// The headline, in mesimon's voice, ready to render. Never raw serde text.
    pub text: String,
    /// The file this is about (absolute), when there is one.
    #[serde(default)]
    pub path: Option<String>,
    /// The parser's own words — file:line:column. For the log and the detail
    /// line, never the headline (13 §13.10.3).
    #[serde(default)]
    pub detail: Option<String>,
}

impl Notice {
    pub fn new(kind: &str, text: impl Into<String>) -> Self {
        Self { kind: kind.into(), text: text.into(), path: None, detail: None }
    }

    pub fn with_path(mut self, p: impl std::fmt::Display) -> Self {
        self.path = Some(p.to_string());
        self
    }

    pub fn with_detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

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
    Hello {
        version: u32,
        client: String,
    },
    Snapshot,
    Subscribe,
    CreateTicket {
        column: String,
        title: String,
    },
    RenameTicket {
        id: ulid::Ulid,
        title: String,
    },
    /// Starts the grace band; the ticket vanishes from snapshots immediately
    /// and is destroyed when the band expires (D21). `discard_worktree` is the
    /// delete-gate's red "remove": the user confirmed losing unmerged work, so
    /// teardown may delete the branch with `-D` (M4).
    DeleteTicket {
        id: ulid::Ulid,
        #[serde(default)]
        discard_worktree: bool,
    },
    /// M4 layering: set the per-ticket workspace strategy. Refused once the
    /// ticket has any session or a worktree binding (the choice is locked).
    SetWorkspace {
        id: ulid::Ulid,
        workspace: Option<WorkspaceStrategy>,
    },
    /// Set (or with `name: None`, clear) the ticket's tag on one axis.
    /// One tag per group, so this REPLACES rather than appends — the digit
    /// that names the group is the same digit that cycles it.
    ///
    /// `group` is a `u8` and `name` a `String` on purpose: neither may be an
    /// enum, or an unknown value from a newer daemon would fail the whole
    /// `Response::Board` deserialize and the client would drop the line.
    /// The daemon sanitizes and length-caps the name (`board::sanitize_tag`).
    SetTag {
        id: ulid::Ulid,
        group: u8,
        name: Option<String>,
    },
    /// Retire a tag: remove it from the registry and from every ticket
    /// wearing it. The counterpart to create-on-the-fly — a vocabulary that
    /// only ever grows collects every typo forever.
    ForgetTag {
        group: u8,
        name: String,
    },
    /// Put a new name in the registry without putting it on any ticket.
    /// Creating and wearing are separate gestures in the picker.
    RegisterTag {
        group: u8,
        name: String,
    },
    /// Rename a tag, carrying every ticket wearing it along.
    RenameTag {
        group: u8,
        from: String,
        to: String,
    },
    /// Pick a tag's tint (Tab in the picker). `color` indexes the tag ramp
    /// and is taken modulo its size, so an out-of-range value from a newer
    /// client wraps rather than being refused.
    SetTagColor {
        group: u8,
        name: String,
        color: u8,
    },
    /// Merge the ticket's branch into the default branch — fast-forward ONLY.
    /// A branch the default moved past answers `NeedsRebase`: the agent
    /// rebases + tests in its worktree first (`MergeToAgent`), so mesimon
    /// never mints merge commits and tests ran on the merged state.
    MergeTicket {
        id: ulid::Ulid,
    },
    /// Paste one of the merge-flow requests into the ticket's live claude
    /// session (explicit user gesture — the m key's staged progression).
    MergeToAgent {
        id: ulid::Ulid,
        request: MergeRequest,
    },
    /// Undo within the grace band.
    RestoreTicket {
        id: ulid::Ulid,
    },
    /// Off the board, kept on disk (13: a field, not a directory move).
    /// Refused while any session holds a pane — archive means everything
    /// is asleep. Reversible: `UnarchiveTicket` restores to the same column.
    ArchiveTicket {
        id: ulid::Ulid,
    },
    UnarchiveTicket {
        id: ulid::Ulid,
    },
    /// Re-read the user's shell environment (the Esc menu's shell-env row).
    ///
    /// Deliberately explicit rather than automatic on an rc-file change: the
    /// capture runs the user's rc files, and doing that unbidden every time an
    /// editor writes `~/.zshrc` would fork a shell on every keystroke-save.
    /// The daemon notices the change and OFFERS; the person decides.
    ReloadShellEnv,
    /// Take the header's archive offer: archive exactly the tickets the
    /// suggestion prices (the offer's own candidate set, nothing broader).
    ArchiveAll,
    MoveTicket {
        id: ulid::Ulid,
        column: String,
        before: Option<ulid::Ulid>,
    },
    SpawnSession {
        ticket: ulid::Ulid,
        kind: SessionKind,
        /// Submit the ticket title as the session's first prompt instead of
        /// only typing it into the box. The board's Shift+Enter compose sets
        /// it; nothing else does. Serde-additive: a frame from an older
        /// client parses as `false`, which is the prefill-only behaviour that
        /// has always been the default (README: zero token injection by
        /// default — this is the user asking, explicitly, per session).
        #[serde(default)]
        submit_prompt: bool,
    },
    KillSession {
        id: uuid::Uuid,
    },
    /// Re-run the external-session census (19 §4 tier 1). Lazy by design:
    /// fired when the drawer opens, never on a timer.
    RescanExternal,
    /// Tier 2: import a discovered foreign session as an `external` record —
    /// observe-only, no process, no hooks. `ticket: None` mints a fresh
    /// ticket named after the session (the drawer's default gesture).
    AttachExternal {
        claude_session_id: uuid::Uuid,
        ticket: Option<ulid::Ulid>,
    },
    /// Tier 3 in one step: import + take over via `claude --resume`.
    ResumeExternal {
        claude_session_id: uuid::Uuid,
        ticket: Option<ulid::Ulid>,
        confirm: bool,
    },
    /// Take over (or re-take) an attached record. `confirm` overrides the
    /// running-elsewhere refusal (double-resume guard, 09 §9).
    ResumeSession {
        id: uuid::Uuid,
        confirm: bool,
    },
    SleepSession {
        id: uuid::Uuid,
    },
    WakeSession {
        id: uuid::Uuid,
    },
    /// Take the header's sleep offer: sleep every eligible session on
    /// sleep-safe tickets (2026-08-30 rescope — was board-wide; the key must
    /// sleep exactly what the suggestion names, nothing broader).
    ReclaimAll,
    PinAwake {
        id: uuid::Uuid,
        pinned: bool,
    },
    /// Exclusive-focus token (D22). Grants the attach argv for the handover.
    FocusStart {
        session: uuid::Uuid,
    },
    FocusEnd {
        session: uuid::Uuid,
    },
    /// Has the GATE ceremony been passed on this machine?
    GateStatus,
    GatePassed,
    Shutdown,
    /// Read-only diff viewer (M4b): the ticket's stable file list,
    /// BASE...BRANCH. Served on the connection thread, never the writer.
    DiffList {
        ticket: ulid::Ulid,
    },
    /// One file's hunks on demand. `context` is the -U density (1 | 3 | 8).
    DiffFile {
        ticket: ulid::Ulid,
        path: String,
        #[serde(default = "default_diff_context")]
        context: u32,
    },

    /// The ticket page's preview zone (M4b's read-only spirit, shells): the
    /// last non-empty lines a pane has on screen. Sessions are the daemon's
    /// to read — the backend is its alone — and `mcp::agent_allows` denies
    /// this outright, which is D10's "no session read at any tier".
    PaneTail {
        session: uuid::Uuid,
        /// How many non-empty lines to take from the bottom; clamped daemon-side.
        lines: u16,
    },

    // ------------------------------------------------------------------
    // The agent tier (T-84). Three commands, reachable only by
    // `Principal::Agent`, and gated by `mcp::agent_allows` — which is an
    // exhaustive match, so a command added below this line will not compile
    // until someone decides whether an agent may send it.
    //
    // No agent command takes a ticket id. The ticket comes from the session
    // the connection is bound to, so an agent cannot address another ticket
    // even by guessing an id, and there is no ownership check to get wrong.
    // ------------------------------------------------------------------
    /// The caller's own ticket, as `get_ticket` renders it.
    AgentGetTicket,
    /// Board metadata only. Deliberately NOT `Snapshot`: no session, argv,
    /// transcript path, cwd or cost ever reaches an agent, at any tier.
    AgentListBoard,
    /// Move the caller's own ticket. `to_column` is validated server-side
    /// against the board's real columns and the tier's permitted set.
    AgentMoveTicket {
        to_column: String,
        /// The client's `_meta["claudecode/toolUseId"]` when it has one.
        /// A mid-call transport drop hands the model the literal string
        /// `Connection closed` AFTER the move has already been persisted —
        /// the mutation happened and the agent believes it failed. Replaying
        /// the stored result is what stops the retry moving the card twice.
        #[serde(default)]
        idempotency_key: Option<String>,
    },
}

fn default_diff_context() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "resp", rename_all = "snake_case")]
pub enum Response {
    /// Every field added here after `daemon_pid` MUST be `#[serde(default)]`.
    /// The TUI's reader thread drops a line it cannot deserialize, so a
    /// required field would turn "an older daemon answered" into a silent
    /// 10 s response timeout — a refusal to launch on exactly the version
    /// skew these fields exist to detect.
    Hello {
        version: u32,
        daemon_pid: u32,
        /// The daemon's own CARGO_PKG_VERSION (docs/02 §5.2's `server_build`).
        /// Empty = a daemon predating this field.
        #[serde(default)]
        build: String,
        /// (mtime_ms, len) of the daemon's executable, captured at startup.
        /// `None` = unknown; callers must read that as "unknown", never as
        /// "changed" (D26 fails closed).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exe_stamp: Option<ExeStamp>,
        /// True only for a daemon mesimon spawned detached. A human's
        /// foreground `mesimon daemon --repo` is never restarted under them.
        #[serde(default)]
        detached: bool,
    },
    Ok,
    /// CreateTicket's receipt: the minted id, so the client can select it.
    Created {
        id: ulid::Ulid,
    },
    Spawned {
        id: uuid::Uuid,
    },
    /// ReclaimAll's receipt: how many actually slept, and why others did not.
    Reclaimed {
        slept: usize,
        skipped: usize,
    },
    /// ArchiveAll's receipt: the honest split (skipped = woke since pricing).
    Archived {
        archived: usize,
        skipped: usize,
    },
    Board {
        board: Board,
        grace: Vec<GraceItem>,
        #[serde(default)]
        external: Vec<ExternalItem>,
        #[serde(default)]
        resources: Resources,
        /// Per-ticket worktree bindings (M4). Serde-additive: absent from an
        /// older daemon parses as empty.
        #[serde(default)]
        worktrees: Vec<WorktreeItem>,
        /// Standing advisories about persisted state — a quarantined file, a
        /// file newer than this build. Serde-additive: absent from an older
        /// daemon parses as empty.
        #[serde(default)]
        notices: Vec<Notice>,
    },
    /// SpawnSession on a worktree ticket that is not provisioned yet: the
    /// worktree is being created off-thread; a BoardChanged follows when the
    /// session actually spawns.
    Provisioning,
    /// MergeTicket's receipt.
    Merge {
        /// What the user's shell environment is doing (see [`ShellEnvStatus`]).
        /// Serde-additive: absent from an older daemon parses as "fresh and
        /// empty", which shows no offer — the right way to fail.
        #[serde(default)]
        shell_env: ShellEnvStatus,
        outcome: MergeOutcome,
        detail: String,
    },
    /// argv the client should exec for the focus handover.
    Attach {
        argv: Vec<String>,
    },
    Gate {
        passed: bool,
        attach_argv: Option<Vec<String>>,
    },
    Err {
        message: String,
    },
    /// DiffList's answer: the stable file list plus display-only in-flight
    /// flags. `branch_oid` is the live tip at serve time.
    DiffList {
        branch: String,
        base_oid: String,
        branch_oid: String,
        files: Vec<crate::diff::FileEntry>,
        /// false = evicted: no worktree directory, so no dirty/untracked
        /// flags and no `!` shell — the diff itself still renders from the
        /// object store.
        #[serde(default)]
        worktree_present: bool,
    },
    DiffFile {
        file: crate::diff::FileDiff,
    },
    /// PaneTail's answer: oldest line first, ready to draw in that order.
    /// Bounded daemon-side; the client still sanitizes, because a pane holds
    /// whatever a command decided to print.
    PaneTail {
        lines: Vec<String>,
    },
    /// AgentGetTicket's answer.
    AgentTicket {
        ticket: AgentTicketView,
    },
    /// AgentListBoard's answer.
    AgentBoard {
        board: AgentBoardView,
    },
    /// AgentMoveTicket's receipt: where the ticket actually ended up.
    AgentMoved {
        column: String,
        #[serde(default)]
        board_version: u64,
        /// True when the key had already been used and the stored result was
        /// replayed instead of moving again.
        #[serde(default)]
        replayed: bool,
    },
}

/// The caller's own ticket, as an agent sees it.
///
/// A hand-written projection, not `Ticket` with fields skipped: a projection
/// that is a separate type cannot silently grow a field when the board model
/// does. Everything here is board data the agent could read off disk anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTicketView {
    pub key: String,
    pub title: String,
    pub column: String,
    /// `worktree` | `shared_checkout` | `adopt_existing`.
    pub workspace: String,
    /// The ticket's branch, when it has a worktree.
    #[serde(default)]
    pub branch: Option<String>,
    /// Merge state as a word: `merged` | `ahead` | `needs_rebase` | `clean`,
    /// or absent when the ticket has no worktree. A word, not an enum, for
    /// the same reason `WorktreeItem.status` is one.
    #[serde(default)]
    pub merge_state: Option<String>,
    /// Where `move_ticket` will accept a move to, right now. This is why
    /// `to_column` needs no schema enum: the valid set travels as transient
    /// result data instead of permanent context.
    #[serde(default)]
    pub allowed_columns: Vec<String>,
    #[serde(default)]
    pub board_version: u64,
}

/// One row of `list_board`. Three fields, and no fourth is coming: a ticket's
/// session is not an agent's business.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTicketRow {
    pub key: String,
    pub title: String,
    pub column: String,
}

/// The board as an agent sees it: columns in board order, tickets, nothing
/// else. `agent_board_view_leaks_no_session_data` in the daemon asserts the
/// serialized form carries no session key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBoardView {
    pub columns: Vec<String>,
    pub tickets: Vec<AgentTicketRow>,
    #[serde(default)]
    pub board_version: u64,
}

/// A ticket's worktree binding, as the board renders it (M4). Oids stay
/// daemon-side; the client gets words, flags, and (M4b) the worktree path —
/// carried solely so `!` on the diff screen can open a shell there.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeItem {
    pub ticket: ulid::Ulid,
    pub branch: String,
    /// Status word: queued | provisioning | attached | evicted | error.
    pub status: String,
    /// Branch tip is an ancestor of the default branch.
    pub merged: bool,
    /// Duplicate-branch blocker (12 §12.6.7): another worktree holds this
    /// branch — commits will delete each other.
    pub conflict: bool,
    /// Commits on the branch not yet in the default branch — merge available
    /// when > 0.
    #[serde(default)]
    pub ahead: u32,
    /// The default branch moved past this branch: no fast-forward — the m
    /// flow's rebase stage comes first.
    #[serde(default)]
    pub needs_rebase: bool,
    /// Error detail when status == "error" (names the failing stage).
    #[serde(default)]
    pub detail: Option<String>,
    /// Worktree directory — Some only while attached (M4b, `!` handover).
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeOutcome {
    Merged,
    AlreadyMerged,
    /// The default branch moved past this branch — no fast-forward. The next
    /// stage asks the agent to rebase + test (`MergeToAgent`).
    NeedsRebase,
    /// Not performed; `detail` says why (sessions active, dirty checkout, …).
    Refused,
}

/// What `MergeToAgent` pastes into the agent's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeRequest {
    /// "Rebase onto <base>, resolve conflicts, run the tests" — the
    /// pre-merge stage when the default branch moved.
    Rebase,
    /// "Your branch was merged into <base>" — the post-merge notice.
    MergedNotice,
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
    /// The header's sleep suggestion (suggestions over shortcuts, dogfood
    /// 2026-08-29): sessions on sleep-safe tickets passing the D23 floors
    /// right now, and the RSS they hold. Zero sessions = no suggestion.
    #[serde(default)]
    pub reclaim_bytes: u64,
    #[serde(default)]
    pub reclaim_sessions: usize,
    /// The header's archive suggestion: tickets whose sessions are all asleep
    /// and untouched past the hour threshold. Zero tickets = no suggestion.
    #[serde(default)]
    pub archive_tickets: usize,
}

/// Pushed to subscribed clients whenever board state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    BoardChanged,
}

/// The state of the environment mesimon hands to new panes.
///
/// A Claude pane is exec'd directly by tmux, so it reads no shell startup file
/// of its own and gets exactly what the daemon passes it. This is how the board
/// says whether that is still current.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellEnvStatus {
    /// A shell startup file has changed since the environment new panes are
    /// getting was captured. What the header offers to act on.
    #[serde(default)]
    pub stale: bool,
    /// A capture is running right now.
    #[serde(default)]
    pub reloading: bool,
    /// The last capture failed and the previous environment is still in force.
    /// Offered the same way staleness is — a capture that failed once (a slow
    /// rc file, a shell that was mid-edit) otherwise leaves the user with the
    /// fallback environment and no way to ask again.
    #[serde(default)]
    pub failed: bool,
    /// How many variables the current environment carries — the menu row spends
    /// it as the concrete thing the reload would change.
    #[serde(default)]
    pub vars: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An older daemon's Hello — no `build`, no `exe_stamp`, no `detached` —
    /// must still parse. The client's reader thread DROPS a line it cannot
    /// deserialize, so a required field here would surface as a 10 s response
    /// timeout and a TUI that refuses to launch, on exactly the skew these
    /// fields exist to detect. This is the test that catches a missing default.
    #[test]
    fn old_hello_json_parses() {
        let old = r#"{"resp":"hello","version":1,"daemon_pid":7}"#;
        let r: Response = serde_json::from_str(old).unwrap();
        match r {
            Response::Hello { version, daemon_pid, build, exe_stamp, detached } => {
                assert_eq!(version, 1);
                assert_eq!(daemon_pid, 7);
                assert!(build.is_empty(), "absent build reads as empty");
                assert!(exe_stamp.is_none(), "absent stamp is unknown, not changed");
                assert!(!detached, "absent detached reads as not-ours-to-restart");
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }

    /// A `Response::Board` from a daemon with no `notices` key parses as empty
    /// — same drop-on-parse hazard, same rule as `worktrees` before it.
    #[test]
    fn old_board_json_parses() {
        let old = r#"{"resp":"board","board":{"columns":[],"tickets":[],"sessions":[],
            "next_key":0},"grace":[]}"#;
        let r: Response = serde_json::from_str(old).unwrap();
        match r {
            Response::Board { notices, worktrees, external, .. } => {
                assert!(notices.is_empty());
                assert!(worktrees.is_empty());
                assert!(external.is_empty());
            }
            other => panic!("expected board, got {other:?}"),
        }
    }

    /// `Notice.kind` is a String precisely so a kind this build has never heard
    /// of still parses. An enum would fail the whole Board deserialize and the
    /// client would drop the line — blanking the board on an older client.
    #[test]
    fn unknown_notice_kind_parses() {
        let n: Notice = serde_json::from_str(r#"{"kind":"something_new","text":"hello"}"#).unwrap();
        assert_eq!(n.kind, "something_new");
        assert!(n.path.is_none());
        assert!(n.detail.is_none());
    }

    /// The new Hello round-trips with every field populated.
    #[test]
    fn hello_roundtrips_with_identity() {
        let h = Response::Hello {
            version: PROTOCOL_VERSION,
            daemon_pid: 4711,
            build: "0.1.0-alpha.1".into(),
            exe_stamp: Some(ExeStamp { mtime_ms: 1_788_046_350_000, len: 30_000_000 }),
            detached: true,
        };
        let s = serde_json::to_string(&h).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        match back {
            Response::Hello { build, exe_stamp, detached, .. } => {
                assert_eq!(build, "0.1.0-alpha.1");
                assert_eq!(exe_stamp.unwrap().len, 30_000_000);
                assert!(detached);
            }
            other => panic!("expected hello, got {other:?}"),
        }
    }
}
