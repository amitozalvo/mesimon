//! The typed command envelope (D22/D32c): every mutation, from any client,
//! travels as one of these, carries its principal, and passes `authorize()`.
//! Wire: newline-delimited JSON over the daemon's unix socket.

use crate::authorize::Action;
use serde::{Deserialize, Serialize};

use crate::board::{AgentProvider, Board, ColumnSettings, SessionKind, SortBy, WorkspaceStrategy};
use crate::Principal;

// Codex session variants and provider settings must not be sent to a v1
// client, which cannot deserialize their enum values. Storage has separate
// forward-version barriers so a downgraded writer cannot erase provider data.
pub const PROTOCOL_VERSION: u32 = 2;

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
    /// `shell_env` | `merge_train_suspended` | `merge_train_blocked` |
    /// `automation_suspended`.
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
    Mesophon {
        action: crate::mesophon::LocalAction,
    },
    Snapshot,
    Subscribe,
    CreateTicket {
        column: String,
        title: String,
        /// The composer's workspace choice, riding the mint (T-117) so the
        /// column's auto-run can spawn before a later `SetWorkspace` would
        /// hit the lock. Absent means the column's own default.
        #[serde(default)]
        workspace: Option<WorkspaceStrategy>,
    },
    /// A trusted local adapter materializes owner-delegated external content.
    /// Same-UID local trust only: origin is correlation, never remote authority.
    /// No agent MCP tool may call this; it never starts a session or workspace.
    ImportTicket {
        column: String,
        content: crate::content::TicketContent,
        origin: crate::content::ImportOrigin,
    },
    /// Board sharing (T-215). Every one of these is a person's gesture on
    /// the board and is answered `Ok` at once; the work happens on the
    /// daemon's relay thread and its outcome is read from `Snapshot.team`.
    /// `relay` is `host[:port] [pin]`; `display_name` is what teammates see.
    TeamSignIn {
        relay: String,
        display_name: String,
    },
    TeamSignOut,
    /// Publish this board: mint a key, create the board on the relay, send
    /// every ticket — and every note, unless `notes` is off, in which case
    /// members see titles, columns and order and the notes stay on this
    /// machine. The daemon becomes the board's owner.
    ShareBoard {
        #[serde(default = "default_true")]
        notes: bool,
    },
    /// Stop sharing: the board vanishes for every member.
    UnshareBoard,
    /// Mint a one-time invite code for `role` (`contributor` or `viewer`);
    /// the code lands in `Snapshot.team.invite`.
    MintInvite {
        role: String,
    },
    /// Remove a member (by device id, hex) and rotate the board key.
    RevokeMember {
        device: String,
    },
    /// Redeem an invite code. The daemon creates a board root for the joined
    /// board and lists it in `Snapshot.team.boards` with its path.
    JoinBoard {
        code: String,
    },
    LeaveBoard,
    /// Ask the relay for the list of boards this device belongs to.
    TeamRefresh,
    /// Copy a ticket's content into a fresh, sessionless card immediately below it.
    DuplicateTicket {
        id: ulid::Ulid,
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
    /// Reposition a tag in the registry — along its own axis, or onto
    /// another one, carrying every wearer with it (`board::move_tag`).
    ///
    /// `to_index` is a slot in the DESTINATION row and is clamped there, so a
    /// client that has just watched a row change under it lands the tag at
    /// the end rather than being refused.
    MoveTag {
        group: u8,
        name: String,
        to_group: u8,
        to_index: usize,
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
    /// Put a prompt the USER typed in front of the ticket's live claude and
    /// submit it, without the user going to the pane (the board's
    /// Shift+Enter). `MergeToAgent` is the same delivery with mesimon's own
    /// words; this one carries only the user's, which is why it may be
    /// reached from an ordinary key while that one is a staged confirmation.
    ///
    /// README promise 3 survives literally: `sanitize_prompt` only ever
    /// REMOVES characters, and nothing — no prefix, no marker, no trailing
    /// note — is appended on the way to the pane. The text Claude reads is a
    /// subsequence of the text the user typed.
    PromptSession {
        ticket: ulid::Ulid,
        text: String,
        /// Wait for idle and a quiet checkout, including approvals and questions.
        /// One in-memory entry per ticket, drained in board order.
        /// Absent from an older client = send now.
        #[serde(default)]
        queued: bool,
    },
    /// Atomically remove a waiting prompt and return its words for editing.
    TakeQueuedAsk {
        ticket: ulid::Ulid,
    },
    /// Deliver the waiting words now, bypassing idle and checkout waits.
    SendQueuedAsk {
        ticket: ulid::Ulid,
    },
    /// Discard a waiting prompt without delivering it.
    DropQueuedAsk {
        ticket: ulid::Ulid,
    },
    /// The board's Shift+Enter on a COLUMN HEADER (T-378): the user's words,
    /// once, in front of every SEAT in that column — a pane is pasted into,
    /// a parked agent is woken with the words held for its first tick, and
    /// a ticket with no agent starts one on them (T-405, the same three
    /// roads `PromptSession` takes, with one receipt). Blank words reach
    /// the empty seats alone, where the Enter lands on the ticket title the
    /// spawn types; every seated agent is skipped, because an empty paste
    /// would press Enter on a turn nobody wrote.
    ///
    /// Promise 3 holds as it does there: `sanitize_prompt` runs once, only
    /// ever removes, and nothing is appended per ticket.
    PromptColumn {
        column: String,
        text: String,
        /// Wait for each session to become idle and its checkout to be quiet.
        /// Shared-checkout prompts are serialized in board order.
        #[serde(default)]
        queued: bool,
    },
    /// Release uncommitted uploads owned by this connection.
    DiscardAttachmentUploads {
        uploads: Vec<ulid::Ulid>,
    },
    /// A base64 PNG chunk. The first chunk mints a connection-owned handle.
    UploadAttachment {
        upload: Option<ulid::Ulid>,
        offset: usize,
        data: String,
        complete: bool,
    },
    /// Read a ticket-local PNG on demand; image bytes never ride snapshots.
    ReadAttachment {
        ticket: ulid::Ulid,
        attachment: ulid::Ulid,
    },
    /// Commit completed upload handles before publishing the note references.
    SaveNoteWithAttachments {
        ticket: ulid::Ulid,
        note: Option<ulid::Ulid>,
        text: String,
        uploads: Vec<ulid::Ulid>,
    },
    /// Publish pictures and description before starting any column automation.
    CreateTicketWithNote {
        column: String,
        title: String,
        workspace: Option<crate::board::WorkspaceStrategy>,
        text: String,
        uploads: Vec<ulid::Ulid>,
    },
    /// One note's body, read whole. Bodies never ride the snapshot (a note
    /// can be 32 KiB and the board is cloned on every event), so the ticket
    /// page asks for the one it is showing.
    ReadNote {
        ticket: ulid::Ulid,
        note: ulid::Ulid,
    },
    /// Create (`note: None`) or replace (`Some`) a note on a ticket — the
    /// whole file, never a section (docs/13 §13.3: prose is an opaque blob).
    /// Blank text on an existing note deletes it. The daemon mints the id,
    /// derives the name and stamps who and when; `sanitize_note` only ever
    /// removes.
    WriteNote {
        ticket: ulid::Ulid,
        #[serde(default)]
        note: Option<ulid::Ulid>,
        text: String,
    },
    /// Tell the ticket's live claude that a note changed: mesimon's own
    /// sentence, pasted and submitted like `MergeToAgent`. A human gesture
    /// (the editor's second `^s`); `mcp::agent_allows` denies it.
    NoteToAgent {
        ticket: ulid::Ulid,
        note: ulid::Ulid,
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
    /// Archive with a deadline (T-74): the daemon's tick wheel restores the
    /// ticket at `until` (unix seconds), at the top of its column, and with
    /// `needs_you` lights it until the cursor rests on it. Same gate as
    /// `ArchiveTicket`; refused when `until` is already past. Reversible the
    /// same way — `UnarchiveTicket` is also how a snooze is cancelled.
    SnoozeTicket {
        id: ulid::Ulid,
        until: u64,
        needs_you: bool,
    },
    /// The cursor rested on a ticket a snooze woke: clear its needs-you
    /// mark (`Ticket::woke_at`). A no-op on any other ticket.
    SeenTicket {
        id: ulid::Ulid,
    },
    /// The person is done with a raised hand (`Ticket::raised`, T-107): the
    /// mark comes off. Sent when the ticket's page is LEFT — the page is the
    /// look, and lowering on the way out rather than on the way in is what
    /// lets the reason be read. A no-op on a ticket holding no hand, so the
    /// TUI may send it whenever it likes.
    ///
    /// Deliberately not folded into `SeenTicket`: that one fires on every
    /// keypress that lands the board cursor on a card, and a hand discharged
    /// by `j`-ing past it would be a hand nobody read.
    LowerHand {
        id: ulid::Ulid,
    },
    /// Take a ticket off the merge train, or put it back (T-227, the `t`
    /// key): `on` sets `Ticket::manual_merge`, and the train then neither
    /// merges the branch nor asks its agent to rebase — `m` by hand still
    /// does both. Persisted on the ticket, so a restart cannot re-arm it.
    /// A person's gesture: an agent may not decide whether its own branch
    /// lands on its own, so the tier never gets it.
    SetManualMerge {
        id: ulid::Ulid,
        on: bool,
    },
    /// Crown a ticket (T-411): its agent may then edit every other ticket
    /// through the keyed forms of its tools. One crown per board — crowning
    /// a second ticket displaces the first. A person's gesture (`^o`), and
    /// the never-tier holds both this and `Uncrown`: an agent that could
    /// crown itself would be deciding its own tier.
    CrownTicket {
        id: ulid::Ulid,
    },
    /// Take the crown off whichever ticket wears it.
    Uncrown,
    /// Re-read the user's shell environment (the Esc menu's shell-env row).
    ///
    /// Deliberately explicit rather than automatic on an rc-file change: the
    /// capture runs the user's rc files, and doing that unbidden every time an
    /// editor writes `~/.zshrc` would fork a shell on every keystroke-save.
    /// The daemon notices the change and OFFERS; the person decides.
    ReloadShellEnv,
    /// Fetch the checkout's upstream remote now (the Esc menu's `Fetch origin`
    /// row), whether or not the periodic opt-in is armed. A person's gesture:
    /// it reaches the network and writes remote-tracking refs, which is why
    /// the agent tier never gets it. A press while a sample is in flight
    /// queues rather than being refused.
    GitFetch,
    /// Arm or disarm the merge train (2026-09-04): while every claude on
    /// the board is idle, mesimon fast-forwards finished REVIEW branches and
    /// asks ONE idle agent whose branch fell behind to rebase + test. Held in
    /// daemon memory and tied to the CONNECTION that armed it — a closed board
    /// is a stopped train. Local only: it makes mesimon prompt an agent with
    /// no per-press gesture, which is why the Settings row is the opt-in.
    SetAutomation {
        merge_train: bool,
        /// After a train merge, paste the merged notice into that agent.
        #[serde(default)]
        merge_notice: bool,
    },
    /// Move the private tmux server's status line to the top or the bottom
    /// of every pane (T-264). A per-machine preference (`prefs.json`) the
    /// TUI pushes on every toggle and whenever a snapshot reads the daemon
    /// holding the other side — a daemon restart, an older daemon; the
    /// daemon reads no preference file. Held in memory, applied to the live
    /// server and to the conf the next one starts from. Local only: chrome
    /// over the user's own panes is nothing an agent has a say in.
    SetStatusLine {
        top: bool,
    },
    /// Turn the agent tool surface on or off for this board (T-217).
    ///
    /// Per repo, persisted in `columns.toml`, and read at every spawn: off
    /// means `claude_argv` omits `--mcp-config` and a wake drops it from the
    /// argv it replays. Local only — an agent that could switch its own tools
    /// off, or back on, would be deciding its own tier.
    SetMcpTools {
        on: bool,
    },
    /// Board inactivity timeout for Claude sessions, in minutes. Zero disables it.
    SetParkAfterMinutes {
        minutes: u32,
    },
    /// Project default for newly accepted agent starts. Existing sessions
    /// retain their provider. Local only: agents cannot choose who runs
    /// subsequent sessions on the board.
    SetAgentProvider {
        provider: AgentProvider,
    },
    /// Turn the agent brief on or off for this board (T-224): `brief::TEXT`
    /// in the system prompt of every claude mesimon starts here, through
    /// `--append-system-prompt`. Per repo, persisted in `columns.toml`, read
    /// at every spawn and wake; off by default, and the dialog that offers it
    /// shows the text verbatim first. Local only — a tier that could write
    /// its own system prompt is not one.
    SetSystemPrompt {
        on: bool,
    },
    /// Choose the board's DEFAULT column (T-279): where an agent's
    /// `create_ticket` lands a card that names no column. `None` is the first
    /// column, the old behaviour; a name is validated against the board's
    /// real columns. Per repo, persisted in `columns.toml`, a Settings row.
    /// Local only — an agent that could choose where its own cards land
    /// would be choosing what the user sees first.
    SetDefaultColumn {
        #[serde(default)]
        column: Option<String>,
    },
    /// Per-board default for the person's follow-up composer.
    SetFollowUpMode {
        mode: crate::board::FollowUpMode,
    },
    /// Rewrite one of the three sentences mesimon types into an agent's box
    /// (T-353): the rebase ask, the merged notice, the note nudge. `None`
    /// puts mesimon's own words back — so does text that sanitizes to
    /// nothing, which is what an emptied field sends. The daemon sanitizes
    /// what it stores (`sanitize_prompt`, the same boundary a typed ask
    /// crosses), so a template is one line and the stored bytes are the
    /// bytes the tty will receive. Per repo, persisted in `columns.toml`.
    /// Local only — an agent that could rewrite the sentence that starts its
    /// own next turn would be writing its own instructions.
    SetAgentPrompt {
        which: crate::prompts::AgentPrompt,
        #[serde(default)]
        text: Option<String>,
    },
    /// The column lifecycle (T-117). All local only: a tier that could add a
    /// column, rename the one it is in, or rewrite its own column's rules
    /// would be writing its own tier. A column's NAME is its identity —
    /// `Ticket.column`'s foreign key, there is no id — so a rename is a daemon
    /// transaction over every ticket file, archived ones included.
    AddColumn {
        name: String,
        /// The column it lands after, in board order; absent = at the end.
        #[serde(default)]
        after: Option<String>,
    },
    RenameColumn {
        name: String,
        to: String,
    },
    /// Refused while any live ticket is in it (`move its N tickets first`)
    /// and for the last column; an archived ticket keeps its string and the
    /// restore falls back to the first column.
    DeleteColumn {
        name: String,
    },
    ReorderColumn {
        name: String,
        /// The column it lands before, in board order; absent = at the end.
        #[serde(default)]
        before: Option<String>,
    },
    /// The whole struct, every time: one validation site for the
    /// `on_working`/`on_done` pair. Equal to what stands = no write.
    SetColumnSettings {
        name: String,
        settings: ColumnSettings,
    },
    /// One-shot: fresh `order`s for every live ticket in the column, once.
    /// Not a setting — nothing keeps a column sorted afterwards.
    SortColumn {
        column: String,
        by: SortBy,
    },
    /// Answer the agent-brief offer with "never": stamp the board so the chip
    /// and its menu row stop. Declining for now and copying send nothing.
    IgnoreBriefOffer,
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
    /// Exclusive-focus token (D22). Grants the attach argv for the handover.
    FocusStart {
        session: uuid::Uuid,
    },
    FocusEnd {
        session: uuid::Uuid,
    },
    /// The project's TERMINAL (T-273): a persistent shell on the private
    /// tmux server — the checkout root, or a ticket's attached worktree —
    /// that is no session of any ticket. Takes the focus token like
    /// `FocusStart` and answers with the attach argv; `TerminalEnd` gives
    /// the token back.
    OpenTerminal {
        ticket: Option<ulid::Ulid>,
    },
    TerminalEnd,
    /// Has the GATE ceremony been passed on this machine?
    GateStatus,
    GatePassed,
    Shutdown,
    /// Read-only diff viewer (M4b): the target's stable file list. Served on
    /// the connection thread, never the writer.
    DiffList {
        target: DiffTarget,
    },
    /// One file's hunks on demand. `context` is the -U density (1 | 3 | 8).
    DiffFile {
        target: DiffTarget,
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

    /// `PaneTail` for the `!` terminal (T-366): the ticket page previews the
    /// ticket's terminal before it is adopted, and the terminal has no
    /// session id to ask by — the ticket names it, exactly as `OpenTerminal`
    /// finds it. Denied to agents by the same rule as `PaneTail`.
    TerminalTail {
        ticket: Option<ulid::Ulid>,
        lines: u16,
    },

    /// Adopt the ticket's `!` terminal as a shell session of the ticket
    /// (T-366): the daemon mints a Bash record and RENAMES the pane to the
    /// record's `sid16`, so the shell keeps its history and every session
    /// road (preview, sleep, wake, the reaper) reaches it from then on.
    /// Answers `Response::Spawned`.
    AdoptTerminal {
        ticket: ulid::Ulid,
    },

    /// How long the person inside the FOCUSED pane has been quiet (T-299).
    ///
    /// The one question the board cannot answer about itself. While it is
    /// handed over, focus reporting is off and every keystroke reaches tmux
    /// instead of us, so "is anybody still there" has no source in the
    /// client at all — and the notification thread needs it, because the
    /// suppression it drives takes the sound as well as the banner. tmux
    /// knows: it is the program reading that terminal.
    ///
    /// Takes no argument. The subject is whatever the daemon holds the focus
    /// token on, which is the pane on the user's terminal by construction —
    /// asking about a session named by the client would let a stale id
    /// answer for a pane nobody is in. A read, and denied to agents like
    /// every other session read.
    FocusQuiet,

    // ------------------------------------------------------------------
    // The agent tier (T-84). Seven commands now, reachable only by
    // `Principal::Agent`, and gated by `mcp::agent_allows` — which is an
    // exhaustive match, so a command added below this line will not compile
    // until someone decides whether an agent may send it.
    //
    // No agent command takes a ticket id. The ticket comes from the session
    // the connection is bound to, so an agent cannot address another ticket
    // even by guessing an id, and there is no ownership check to get wrong.
    // (`AgentCreateTicket` mints one and returns its KEY, never an id, and
    // nothing here accepts a key back.)
    // ------------------------------------------------------------------
    /// The caller's own ticket, as `get_ticket` renders it.
    AgentGetTicket,
    /// ANOTHER ticket, by key, as `get_ticket` renders it with a `key`
    /// (T-411). The one read the crown admits and nothing else does: the
    /// daemon refuses it unless the caller's ticket wears the crown, and the
    /// refusal says how a person grants one. A separate command rather than
    /// a field on `AgentGetTicket` so a shim from before the crown still
    /// parses on the wire.
    AgentReadTicket {
        key: String,
    },
    /// Board metadata only. Deliberately NOT `Snapshot`: no session, argv,
    /// transcript path, cwd or cost ever reaches an agent, at any tier.
    AgentListBoard,
    /// Move the caller's own ticket — or, with `key`, another ticket, which
    /// only the crown may (T-411). `to_column` is validated server-side
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
        /// The ticket to move, by key; absent means the caller's own. Any
        /// key that is not the caller's own needs the crown.
        #[serde(default)]
        key: Option<String>,
        /// Land ABOVE this ticket (a key) in the destination — position is
        /// priority. Absent lands at the top, as every agent move did.
        #[serde(default)]
        before: Option<String>,
        /// The `seen` stamp `get_ticket` returned for the target: the move
        /// is refused when the ticket changed since it was read. Required
        /// with `key`, ignored without.
        #[serde(default)]
        seen: Option<String>,
    },
    /// One of the caller's own ticket's notes, whole. A `note` id off the
    /// ticket reads as "no such note" — the binding, not the id, is the
    /// authority.
    AgentReadAttachment {
        attachment: ulid::Ulid,
    },
    AgentReadNote {
        note: ulid::Ulid,
        /// Another ticket's note, by key (T-411): the crown's road.
        #[serde(default)]
        key: Option<String>,
    },
    /// Create or replace a note on the caller's own ticket (D10 T1 ANNOTATE,
    /// the tier tags never had a home in). Same shape as `WriteNote` minus
    /// the ticket.
    AgentWriteNote {
        #[serde(default)]
        note: Option<ulid::Ulid>,
        text: String,
        /// A note on another ticket, by key (T-411): the crown's road.
        #[serde(default)]
        key: Option<String>,
    },
    /// Retitle another ticket, by key (T-411). Crown only; `seen` is the
    /// stamp `get_ticket` returned for it. `sanitize_title` at the boundary
    /// as for the human's `RenameTicket`.
    AgentRenameTicket {
        key: String,
        title: String,
        #[serde(default)]
        seen: Option<String>,
    },
    /// Choose another ticket's workspace, by key (T-411): `worktree` or
    /// `shared_checkout`, a WORD validated server-side. Crown only, and the
    /// human's own lock applies — refused once a pane or a worktree exists
    /// ("only if pending").
    AgentSetWorkspace {
        key: String,
        workspace: String,
        #[serde(default)]
        seen: Option<String>,
    },
    /// Archive another ticket, by key, or with `restore` bring it back
    /// (T-411). Crown only. The reversible spelling of delete, which stays
    /// in the never-tier: nothing an agent does to a card is final.
    AgentArchiveTicket {
        key: String,
        #[serde(default)]
        restore: bool,
        #[serde(default)]
        seen: Option<String>,
    },
    /// Mint a NEW ticket (`create_ticket`). The one agent command that is
    /// not about the caller's own ticket, and the one place the tier makes a
    /// second card: an agent that finds work outside its ticket's scope
    /// files it instead of doing it or losing it. `column` is a plain string
    /// validated against the board's real columns; absent means the board's
    /// default column (`Board::landing_column`, T-279) — the first column
    /// unless the user chose another in Settings. The
    /// caller's session stays bound to ITS ticket — a created ticket has no
    /// session, and no tool can give it one.
    AgentCreateTicket {
        title: String,
        #[serde(default)]
        column: Option<String>,
        /// Becomes the ticket's first note, its description.
        #[serde(default)]
        description: Option<String>,
        /// Tag NAMES, resolved against the board's registry server-side and
        /// worn by the new ticket. A name the registry does not hold is
        /// refused, never registered: the vocabulary stays the human's
        /// (`RegisterTag` is never-tier), and the tool only puts a card
        /// under a word somebody already chose.
        #[serde(default)]
        tags: Vec<String>,
        /// Same role as `AgentMoveTicket`'s: a retry after a dropped
        /// connection replays the first receipt instead of minting twice.
        #[serde(default)]
        idempotency_key: Option<String>,
    },
    /// Put one of the board's EXISTING tags on the caller's own ticket, or
    /// take it off (`tag_ticket`). The registry is the human's vocabulary —
    /// which names exist, what each axis means, what colour each wears — and
    /// this command never touches it: a name the registry does not hold is
    /// refused, never registered. That is the difference from the human's
    /// `SetTag`, where using a name is what creates it. `group` is only
    /// needed when the same name sits on two axes. Idempotent by nature
    /// (wearing what is worn, removing what is absent, both succeed), so no
    /// idempotency key.
    AgentTagTicket {
        name: String,
        #[serde(default)]
        group: Option<u8>,
        #[serde(default)]
        remove: bool,
        /// Another ticket, by key (T-411): the crown's road.
        #[serde(default)]
        key: Option<String>,
    },
    /// Ask for a person on the caller's own ticket (`raise_hand`, T-107):
    /// the card wears the needs-you mark and `!N` counts it until somebody
    /// deals with it. The one channel an agent has into the loud register,
    /// and it reaches ONE card — its own.
    ///
    /// Deliberately not a `SessionState::RequiresAction` reason: the `Stop`
    /// that follows the tool call moments later would wipe it, the 15-minute
    /// stale demote would drop it silently, and a daemon restart re-derives
    /// every session as `Unknown{DaemonRestarted}`. The turn ending is the
    /// one thing that must NOT clear a raised hand, so it is the ticket's.
    AgentRaiseHand {
        /// One line, why. Required: a bare mark makes the user open the
        /// ticket to learn anything at all.
        reason: String,
    },
}

fn default_true() -> bool {
    true
}

fn default_diff_context() -> u32 {
    3
}

/// Which diff a `DiffList`/`DiffFile` is about (T-221).
///
/// An `Option<ulid::Ulid>` whose `None` silently meant "the checkout" would be
/// exactly the implicit classification [`Command::meta`] and
/// [`crate::mcp::agent_allows`] exist to refuse: the two targets read
/// different things through different git plumbing, and one of them has no
/// ticket at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffTarget {
    /// The ticket's worktree branch, BASE...BRANCH — what this ticket changed,
    /// still right after base moved.
    Ticket { id: ulid::Ulid },
    /// The board's own checkout, HEAD vs the working tree — what is
    /// uncommitted here, right now. No ticket, no worktree binding.
    Checkout,
}

impl DiffTarget {
    /// The ticket this diff belongs to, if any. The checkout belongs to none,
    /// which is what every screen-to-ticket map in the TUI has to say.
    pub fn ticket(&self) -> Option<ulid::Ulid> {
        match self {
            DiffTarget::Ticket { id } => Some(*id),
            DiffTarget::Checkout => None,
        }
    }
}

/// The ceiling on one `PromptSession`. A prompt is a sentence or a paragraph
/// typed into a card-width field, not a document: what the board's Shift+Enter
/// is for is "ask the agent a thing while looking at the board", and anything
/// longer belongs in the pane where it can be edited. The bound is here rather
/// than on the field so a client cannot lift it.
pub const PROMPT_MAX_BYTES: usize = 4096;

/// The daemon-side boundary for a user-typed prompt, and the twin of
/// [`crate::board::sanitize_tag`]: user text about to leave mesimon for
/// somebody else's process.
///
/// It only ever REMOVES — that is what keeps the README's third promise
/// literally true for a command whose whole job is to put words in an agent's
/// box. Control characters go because a prompt rides a bracketed paste into a
/// live tty: a bare CR would submit the text early (splitting one prompt into
/// two turns), and an ESC would be read as a key, not as content. `\t` is not
/// spared — inside Claude's input box Tab is a completion, not whitespace.
/// A line break STAYS (T-380, the ask editor): inside a bracketed paste a
/// `\n` is a line of the same prompt, in claude's box and codex's alike, and
/// the Enter that submits is sent separately. The text is bounded, never
/// split mid-character, and blank input is `None` so an empty paste can never
/// press Enter on a turn the user did not write.
pub fn sanitize_prompt(raw: &str) -> Option<String> {
    use crate::text::{cap_bytes, nonblank, scrub_lines};
    nonblank(cap_bytes(&scrub_lines(raw), PROMPT_MAX_BYTES))
}

/// What the daemon must know about a command before running it: the D32c
/// action the chokepoint authorizes, whether the activity feed records it,
/// and the ticket it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub action: Action,
    pub logged: bool,
    pub subject: Option<ulid::Ulid>,
}

impl Command {
    /// The name serde puts on the wire (`create_ticket`), which is also the
    /// name the activity feed records — one spelling, derived, never typed.
    pub fn wire_name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("cmd")?.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    /// Exhaustive on purpose, like [`crate::mcp::agent_allows`]: a command
    /// added to the wire does not compile until it is classified here, in
    /// one place, instead of in parallel tables that drift apart.
    pub fn meta(&self) -> Meta {
        use Action::{Mutate, Read};
        use Command::*;
        let m = |action, logged, subject| Meta { action, logged, subject };
        match self {
            Hello { .. }
            | Snapshot
            | Subscribe
            | GateStatus
            // Mutates only the daemon's discovery cache, never board state.
            | RescanExternal
            | DiffList { .. }
            | DiffFile { .. }
            | PaneTail { .. }
            | TerminalTail { .. }
            | FocusQuiet
            | ReadNote { .. }
            | AgentGetTicket
            | AgentReadTicket { .. }
            | AgentReadAttachment { .. }
            | ReadAttachment { .. }
            | AgentReadNote { .. }
            | AgentListBoard => m(Read, false, None),
            // The crown (T-411): a person's gesture the feed answers "who
            // crowned T-12" with.
            CrownTicket { id } => m(Mutate, true, Some(*id)),
            Uncrown => m(Mutate, true, None),
            CreateTicketWithNote { .. } | CreateTicket { .. } => m(Mutate, true, None),
            DiscardAttachmentUploads { .. } | UploadAttachment { .. } => m(Mutate, false, None),
            ImportTicket { .. } => m(Action::ImportContent, true, None),
            Mesophon { action: crate::mesophon::LocalAction::Status } => m(Read, false, None),
            Mesophon { .. } => m(Mutate, true, None),
            TeamRefresh => m(Read, false, None),
            TeamSignIn { .. }
            | TeamSignOut
            | ShareBoard { .. }
            | UnshareBoard
            | MintInvite { .. }
            | RevokeMember { .. }
            | JoinBoard { .. }
            | LeaveBoard => m(Mutate, true, None),
            DuplicateTicket { id }
            | RenameTicket { id, .. }
            | DeleteTicket { id, .. }
            | SetWorkspace { id, .. }
            | SetTag { id, .. }
            | MergeTicket { id }
            | MergeToAgent { id, .. }
            | RestoreTicket { id }
            | ArchiveTicket { id }
            | UnarchiveTicket { id }
            | SnoozeTicket { id, .. }
            | SetManualMerge { id, .. } => m(Mutate, true, Some(*id)),
            // A cursor landing is not news for the feed, and neither is
            // walking off the page a raised hand was read on.
            SeenTicket { id } | LowerHand { id } => m(Mutate, false, Some(*id)),
            // Chrome over the panes, not the board's history: the feed says
            // what the board did, and where the status line sits is neither.
            SetStatusLine { .. } => m(Mutate, false, None),
            // The ticket, never the text: the feed records that the user
            // asked, not what they asked.
            PromptSession { ticket, .. }
            | TakeQueuedAsk { ticket }
            | SendQueuedAsk { ticket }
            | DropQueuedAsk { ticket }
            | SpawnSession { ticket, .. }
            | AdoptTerminal { ticket }
            | SaveNoteWithAttachments { ticket, .. }
            | WriteNote { ticket, .. }
            | NoteToAgent { ticket, .. } => m(Mutate, true, Some(*ticket)),
            // A column's worth of tickets: `subject` names one, so the
            // handler writes the feed itself — a line per ticket delivered,
            // the way the ask queue does.
            PromptColumn { .. } => m(Mutate, false, None),
            AttachExternal { ticket, .. } | ResumeExternal { ticket, .. } => {
                m(Mutate, true, *ticket)
            }
            ForgetTag { .. }
            | RegisterTag { .. }
            | RenameTag { .. }
            | SetTagColor { .. }
            | MoveTag { .. }
            | ArchiveAll
            | ReloadShellEnv
            | GitFetch
            | SetAutomation { .. }
            // Board-wide settings a person took a gesture to change — the
            // feed is where "who turned the agent tools off" and "who turned
            // the brief on" get answered later.
            | SetMcpTools { .. }
            | SetAgentProvider { .. }
            | SetParkAfterMinutes { .. }
            | SetSystemPrompt { .. }
            | SetFollowUpMode { .. }
            | SetDefaultColumn { .. }
            | SetAgentPrompt { .. }
            | IgnoreBriefOffer
            // The column lifecycle (T-117): a person's gesture, and the feed
            // is where "who renamed TODO" gets answered.
            | AddColumn { .. }
            | RenameColumn { .. }
            | DeleteColumn { .. }
            | ReorderColumn { .. }
            | SetColumnSettings { .. }
            | SortColumn { .. }
            | KillSession { .. }
            | ResumeSession { .. }
            | SleepSession { .. }
            | WakeSession { .. }
            | ReclaimAll => m(Mutate, true, None),
            // Moves are recorded by `place_ticket` itself, with the mover;
            // the rest are session plumbing the feed does not narrate.
            MoveTicket { .. }
            | FocusStart { .. }
            | FocusEnd { .. }
            | OpenTerminal { .. }
            | TerminalEnd
            | GatePassed
            | Shutdown
            | AgentMoveTicket { .. }
            // Logged by `handle_agent` with the agent as actor.
            | AgentWriteNote { .. }
            | AgentCreateTicket { .. }
            | AgentTagTicket { .. }
            | AgentRenameTicket { .. }
            | AgentSetWorkspace { .. }
            | AgentArchiveTicket { .. }
            | AgentRaiseHand { .. } => m(Mutate, false, None),
        }
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn the_feed_name_is_the_wire_name() {
        let c = Command::CreateTicket { column: "a".into(), title: "b".into(), workspace: None };
        assert_eq!(c.wire_name(), "create_ticket");
        assert_eq!(Command::ReloadShellEnv.wire_name(), "reload_shell_env");
    }

    #[test]
    fn reads_are_never_logged_and_subjects_ride_along() {
        assert_eq!(
            Command::Snapshot.meta(),
            Meta { action: Action::Read, logged: false, subject: None }
        );
        let id = ulid::Ulid::new();
        let m = Command::PromptSession { ticket: id, text: "x".into(), queued: false }.meta();
        assert_eq!(m, Meta { action: Action::Mutate, logged: true, subject: Some(id) });
        // A column's ask names no single ticket, so the handler logs per
        // ticket itself (T-378) and the chokepoint logs nothing.
        let m =
            Command::PromptColumn { column: "TODO".into(), text: "x".into(), queued: true }.meta();
        assert_eq!(m, Meta { action: Action::Mutate, logged: false, subject: None });
    }

    /// The column ask's receipt from a daemon that knows fewer of its
    /// fields still parses: every count defaults to zero (T-378).
    #[test]
    fn a_bare_asked_receipt_reads_as_all_zero() {
        let r: Response = serde_json::from_str(r#"{"resp":"asked"}"#).unwrap();
        let Response::Asked { sent, woke, started, queued, skipped, failed } = r else {
            panic!("not asked: {r:?}");
        };
        assert_eq!((sent, woke, started, queued, skipped, failed), (0, 0, 0, 0, 0, 0));
    }
}

// `Board` is the snapshot — the whole board, its bindings and what the
// daemon owes — and it is the one variant a client holds; every other reply
// is a receipt. Boxing it to please the lint would put a heap hop on the road
// every event takes and change nothing on the wire.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "resp", rename_all = "snake_case")]
pub enum Response {
    Mesophon {
        info: crate::mesophon::Info,
    },
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
    /// CreateTicket's receipt: the minted id, so the client can select it,
    /// and whether the column's `auto_run` started a claude on it (T-117) —
    /// so the composer does not start a second.
    Created {
        id: ulid::Ulid,
        #[serde(default)]
        started: bool,
    },
    /// Durable import acceptance. Exact retries retain the original identity,
    /// including after the ticket was deleted; `created` is false on replay.
    Imported {
        id: ulid::Ulid,
        key: String,
        created: bool,
    },
    Spawned {
        id: uuid::Uuid,
        /// The spawn started a NEW conversation where a resume was asked for,
        /// because the record had none to come back to. Defaulted rather than
        /// required: an added field is the only safe way to grow this enum —
        /// a client that cannot parse a `Response` line DROPS it and then
        /// waits forever for a reply that already came (see `Notice::kind`).
        #[serde(default)]
        fresh: bool,
    },
    /// The words removed by TakeQueuedAsk.
    PromptTakenBack {
        text: String,
    },
    /// PromptSession's receipt when the words wait for idle or the checkout.
    /// Only a client that sent `queued: true` can receive it.
    Queued {
        #[serde(default)]
        behind: Vec<String>,
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
    /// PromptColumn's receipt (T-378), per seat: pasted into a pane now,
    /// a parked agent woken with the words held, an empty seat started on
    /// them (T-405), parked in the ask queue, skipped, or refused on the way
    /// (a paste that failed, a spawn the PTY budget turned down). `skipped`
    /// is an adopted external session, or — under a BLANK ask, which only an
    /// empty seat can take — a seat that already has its agent. Every field
    /// defaulted so a client one build behind still reads the line.
    Asked {
        #[serde(default)]
        sent: usize,
        #[serde(default)]
        woke: usize,
        #[serde(default)]
        started: usize,
        #[serde(default)]
        queued: usize,
        #[serde(default)]
        skipped: usize,
        #[serde(default)]
        failed: usize,
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
        /// What the user's shell environment is doing (see [`ShellEnvStatus`]).
        /// Serde-additive: absent from an older daemon parses as "fresh and
        /// empty", which shows no offer — the right way to fail.
        #[serde(default)]
        shell_env: ShellEnvStatus,
        /// Where the checkout the board sits in stands against its upstream
        /// (see [`RepoGit`]). Serde-additive: absent from an older daemon parses
        /// as "not sampled", which draws nothing.
        #[serde(default)]
        git: RepoGit,
        /// What mesimon will do next, per ticket (see [`Pending`]): a queued
        /// ask, a merge the train will make, a rebase it will ask for. Absent
        /// from an older daemon parses as nothing owed.
        #[serde(default)]
        pending: Vec<Pending>,
        /// Whether the merge train is ARMED (a connection holds it), and what
        /// it has asked. Absent from an older daemon parses as off.
        #[serde(default)]
        automation: AutomationStatus,
        /// Whether the repo's `CLAUDE.md` already tells a session to read its
        /// ticket (see [`ClaudeMdStatus`]). Absent from an older daemon parses
        /// as "not sampled", where `present: false` would offer — so the
        /// TUI reads `path` being empty as "no answer yet" and offers nothing.
        #[serde(default)]
        claude_md: ClaudeMdStatus,
        /// Board sharing (T-215): who this device is, whether this board is
        /// shared and how the sync stands. Absent from an older daemon parses
        /// as "not signed in", which offers sign-in and nothing else.
        #[serde(default)]
        team: crate::team::TeamInfo,
        #[serde(default)]
        mesophon: crate::mesophon::Info,
        /// The user's own `permissions.defaultMode`, what a column's
        /// `claude_mode: inherit` resolves to (T-117) — so the dialog can
        /// say `inherit (auto)`. Absent: unknown or unset.
        #[serde(default)]
        claude_default_mode: Option<String>,
        /// Where the daemon holds the tmux status line (T-264, see
        /// [`Command::SetStatusLine`]): the TUI pushes its preference when
        /// this disagrees. Absent from an older daemon parses as bottom.
        #[serde(default)]
        status_top: bool,
        /// The `!` terminals alive on the private server (T-366): one per
        /// directory, named by ticket. What the ticket page's ghost row and
        /// the card's spinner read; none of it is persisted, the panes are
        /// the record. Absent from an older daemon parses as none.
        #[serde(default)]
        terminals: Vec<TerminalItem>,
        /// The crown's edits of the last ten seconds (T-411), for the board
        /// to light the touched cards as they happen. Absent from an older
        /// daemon parses as none.
        #[serde(default)]
        crown_touches: Vec<CrownTouch>,
    },
    /// SpawnSession on a worktree ticket that is not provisioned yet: the
    /// worktree is being created off-thread; a BoardChanged follows when the
    /// session actually spawns.
    Provisioning,
    /// MergeTicket's receipt.
    Merge {
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
    /// flags. `branch_oid` is the live tip at serve time; on a checkout
    /// target it is empty and `base_oid` is the HEAD the diff was taken
    /// against.
    DiffList {
        branch: String,
        base_oid: String,
        branch_oid: String,
        files: Vec<crate::diff::FileEntry>,
        /// false = evicted: no worktree directory, so no dirty/untracked
        /// flags and no `!` shell — the diff itself still renders from the
        /// object store. Always true on a checkout target, which IS the
        /// working tree; whether `!` is offered there is the TUI's to say.
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
    /// FocusQuiet's answer: how long the client attached to the focused
    /// pane has been silent.
    ///
    /// `None` is every way of not knowing — nothing focused, the session
    /// gone, nobody attached, tmux unable to say — and it is deliberately
    /// the same answer as "away". No evidence reads as absent everywhere in
    /// this feature (`core::notify::Presence`), because the failure that
    /// makes it look broken is the silent one.
    FocusQuiet {
        #[serde(default)]
        quiet_ms: Option<u64>,
    },
    /// AgentGetTicket's answer.
    AgentTicket {
        ticket: AgentTicketView,
    },
    /// AgentListBoard's answer.
    AgentBoard {
        board: AgentBoardView,
    },
    /// ReadNote / AgentReadNote's answer: the body and its metadata.
    Note {
        text: String,
        meta: crate::board::NoteMeta,
    },
    /// Receipt for one accepted upload chunk, including the first minted ID.
    AttachmentUploaded {
        upload: ulid::Ulid,
    },
    Attachment {
        meta: crate::attachment::Attachment,
        data: String,
    },
    /// WriteNote / AgentWriteNote's receipt: the note's id (minted on a
    /// create), or `None` when blank text deleted it.
    NoteWritten {
        #[serde(default)]
        note: Option<ulid::Ulid>,
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
        /// The target's fresh `seen` stamp after a keyed move (T-411), so the
        /// next edit needs no second read. Absent on an own-ticket move.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seen: Option<String>,
    },
    /// AgentCreateTicket's receipt: the new ticket's key (what an agent
    /// addresses a ticket by) and where it landed.
    AgentCreated {
        key: String,
        column: String,
        #[serde(default)]
        board_version: u64,
        /// True when the key had already been used and the first receipt
        /// was replayed instead of minting a second ticket.
        #[serde(default)]
        replayed: bool,
    },
    /// AgentTagTicket's receipt: what the ticket wears now, and the tag on
    /// the same axis that was taken off to make room, when there was one.
    AgentTagged {
        tags: Vec<AgentTagView>,
        #[serde(default)]
        replaced: Option<String>,
        #[serde(default)]
        board_version: u64,
        /// The target's fresh `seen` stamp after a keyed tag (T-411).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seen: Option<String>,
    },
    /// AgentRaiseHand's receipt: the words as the board kept them (scrubbed
    /// and capped, so a long line comes back short) and when they go away.
    AgentRaised {
        reason: String,
        #[serde(default)]
        board_version: u64,
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
    /// The tags this ticket wears, one per group at most, in group order.
    #[serde(default)]
    pub tags: Vec<AgentTagView>,
    /// The board's whole tag vocabulary — the names `create_ticket` and
    /// `tag_ticket` accept. Same reasoning as `allowed_columns`: transient
    /// result data, not a schema enum that would put the user's words into
    /// every request.
    #[serde(default)]
    pub allowed_tags: Vec<AgentTagView>,
    #[serde(default)]
    pub board_version: u64,
    /// The first note's body — the ticket's description — capped; the whole
    /// of it comes from `read_note`.
    #[serde(default)]
    pub description: Option<String>,
    /// Every note, in order, so `read_note`/`write_note` have an id to name.
    #[serde(default)]
    pub notes: Vec<AgentNoteView>,
    /// Whether THIS ticket wears the crown (T-411) — whether its agent may
    /// pass a `key` to the tools. False on every other ticket, and said
    /// explicitly: an absent key would leave the model guessing.
    #[serde(default)]
    pub crowned: bool,
    /// The card's words about the ticket's agent (T-411): the state the
    /// board shows, never the pane. Absent when the ticket has no agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AgentStateView>,
    /// An opaque stamp over everything a keyed edit may assume — column,
    /// order, title, tags, notes, workspace, the agent's state — returned
    /// to the daemon by every keyed mutation, which refuses when the ticket
    /// changed since it was read. Read-before-write as a check, not a claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<String>,
}

/// A ticket's agent as its card shows it (T-411). Words, never a session:
/// `agent_state_word`'s vocabulary, how long it has been so, and the raised
/// hand's own sentence when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStateView {
    pub state: String,
    /// Seconds in that state, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_secs: Option<u64>,
    /// The agent's `raise_hand` reason, while the hand is up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raised: Option<String>,
}

/// One of the crown's recent edits (T-411), for the board to light the
/// touched card: which ticket, what was done (a WORD — `moved`, `renamed`,
/// `tagged`, `note`, `workspace`, `archived`, `restored` — so an older
/// client drops what it cannot read), and when. In memory only, pruned
/// after ten seconds; the feed is the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrownTouch {
    pub ticket: ulid::Ulid,
    pub action: String,
    pub at_ms: u64,
}

/// One tag as an agent sees it: the name, and the axis it lives on (the
/// digit that reaches it in the picker; 1–9, 0 = 10). No colour — a tint is
/// how a card paints it, not what it means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTagView {
    pub name: String,
    pub group: u8,
}

/// One note as an agent lists it. No body: that is `read_note`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentNoteView {
    pub id: ulid::Ulid,
    pub name: String,
    /// `local` | `agent:<session-uuid>`.
    pub by: String,
    pub edited_at: String,
}

/// One row of `list_board`. The card's words only: a ticket's session is
/// not an agent's business, and `state` is the same one word the card
/// shows (T-411), never anything off the pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTicketRow {
    pub key: String,
    pub title: String,
    pub column: String,
    /// `person` or `agent`: who filed the card, for a coordinator triaging
    /// what agents left behind. Absent on a ticket from before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The ticket's agent, as one word; absent when it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
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
    /// The key of the ticket wearing the crown (T-411), when one does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown: Option<String>,
}

/// A ticket's worktree binding, as the board renders it (M4). Oids stay
/// daemon-side; the client gets words, flags, and (M4b) the worktree path —
/// carried solely so `!` on the diff screen can open a shell there.
/// One thing mesimon owes a ticket and will do on its own clock — the
/// card's slow mark and the cursor card's `queued ∙ after T-12` row read this.
/// `action` is a WORD (`ask` | `merge` | `rebase`), `Notice::kind`'s rule: a
/// client that cannot parse a snapshot line drops it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub ticket: ulid::Ulid,
    pub action: String,
    /// Short keys of the tickets whose claudes still hold the checkout (or
    /// the board, for the train); may include this ticket's own key.
    #[serde(default)]
    pub waits_on: Vec<String>,
    /// The ask's words, so a second Shift+Enter reopens the field on them —
    /// and, on a `merge` row, the reason the checkout REFUSED it (T-289),
    /// which is what makes the row say `blocked` instead of promising a
    /// merge that will not happen. The local socket only — never the feed
    /// (D11), never a file.
    #[serde(default)]
    pub text: Option<String>,
    /// Pasted, waiting on the agent's `UserPromptSubmit` ack.
    #[serde(default)]
    pub in_flight: bool,
}

impl Pending {
    /// Is this row a queued ASK — words parked for the ticket's claude,
    /// whatever seat it is in? `ask` is a live pane, `wake` a parked claude
    /// the delivery wakes, `start` an empty seat where the delivery starts
    /// one (T-294). The three words live here so no screen spells the
    /// vocabulary itself; the train's `merge` and `rebase` rows are not asks.
    pub fn is_queued_ask(&self) -> bool {
        matches!(self.action.as_str(), "ask" | "start" | "wake")
    }
}

/// The merge train as the board sees it (see `Command::SetAutomation`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationStatus {
    /// ARMED — a connection holds it — not the preference.
    #[serde(default)]
    pub merge_train: bool,
    #[serde(default)]
    pub merge_notice: bool,
    /// Rebase asks the train (or a hand `m`) delivered, so the ticket page
    /// can say `rebase requested` without a TUI-local memory.
    #[serde(default)]
    pub train_asked: Vec<TrainAsk>,
    /// Tickets the train's fuse suspended it for.
    #[serde(default)]
    pub train_suspended: Vec<ulid::Ulid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainAsk {
    pub ticket: ulid::Ulid,
    /// Asked at the CURRENT base tip — the agent has not caught up yet.
    pub current: bool,
    pub at_ms: u64,
    /// `train` or `local` — a word, `Principal::actor`'s vocabulary.
    pub by: String,
}

/// One alive `!` terminal (T-366). `ticket: None` is the checkout's.
/// `foreground` is the name of the command running in it — `None` at a
/// prompt, `Some("cargo")` mid-build — read off `#{pane_current_command}`
/// on the daemon's poll bucket; it is what makes a busy shell spin the card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalItem {
    pub ticket: Option<ulid::Ulid>,
    #[serde(default)]
    pub foreground: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeItem {
    pub ticket: ulid::Ulid,
    pub branch: String,
    /// Status word: queued | provisioning | attached | evicted | error.
    pub status: String,
    /// The branch's work is on the default branch — its tip an ancestor of
    /// it, or its patch already up there under a squash or a rebase-merge
    /// somebody made on a forge (T-267).
    pub merged: bool,
    /// Where it landed, when that is worth saying: `origin/main`, or `main`
    /// where a squash and not a fast-forward is what put it there. Empty for
    /// the ordinary ancestor merge — the word for that is just `merged` —
    /// and empty while unmerged.
    #[serde(default)]
    pub merged_in: String,
    /// The commit on that ref carrying the branch's patch, when patch
    /// equality is what found it. Empty for a plain ancestor merge — there
    /// is no one commit to name.
    #[serde(default)]
    pub merged_oid: String,
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

impl MergeRequest {
    /// Which of the board's templates writes this one (T-353). The two
    /// enums stay apart because `MergeRequest` is the stage of the merge
    /// flow and `AgentPrompt` is a row in Settings; this is the one seam.
    pub fn prompt(self) -> crate::prompts::AgentPrompt {
        match self {
            MergeRequest::Rebase => crate::prompts::AgentPrompt::Rebase,
            MergeRequest::MergedNotice => crate::prompts::AgentPrompt::Merged,
        }
    }
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
    /// Mesimon drawer selector. The legacy wire name remains accepted; this
    /// is not necessarily the provider's conversation identifier.
    #[serde(rename = "claude_session_id")]
    pub id: uuid::Uuid,
    #[serde(default)]
    pub provider: AgentProvider,
    #[serde(default)]
    pub conversation_id: String,
    pub cwd: String,
    pub transcript_path: String,
    pub mtime_ms: u64,
    /// Last assistant text, truncated by the daemon.
    pub preview: Option<String>,
    /// Display name from `sessions/<pid>.json`, when one matched (11 §11.3).
    pub name: Option<String>,
    /// A possible live owner requires takeover confirmation. Positive
    /// ownership or an unavailable provider inventory cannot establish that
    /// another writer is absent (09 §9).
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

/// Whether the repo's `CLAUDE.md` already tells a session to read its ticket
/// (T-217). A user who wrote the instruction into their own file has done the
/// thing the agent-brief offer (T-224) is for, and is not offered it.
///
/// A FILESYSTEM fact, which is why it rides the snapshot rather than the board:
/// the ignore stamp is board state (`Board::claude_md_ignored`) and travels with
/// it, but whether the file says the words is something only a `stat` and a read
/// can answer. The daemon samples it behind an mtime+len gate — a repo's
/// CLAUDE.md can be a hundred kilobytes and a snapshot happens on every change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeMdStatus {
    /// `<repo>/CLAUDE.md`, whether or not it exists — the file `doctor` names.
    /// Empty before the first sample, which offers nothing: an unknown must
    /// never read as "missing".
    #[serde(default)]
    pub path: String,
    /// `claudemd::MARKER` was found — here, or in `.claude/CLAUDE.md`. True
    /// withdraws the offer, however the words got there.
    #[serde(default)]
    pub present: bool,
}

/// A commit in one direction of the checkout's upstream comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommit {
    pub oid: String,
    pub subject: String,
}

/// The git state of the checkout the board sits in — the REPO's, not a
/// ticket's worktree (that is [`WorktreeItem`]). Sampled by the daemon off its
/// writer thread from one `git status --porcelain=v2 --branch`; the header
/// draws the branch, the arrows and the change count from it, and the Esc
/// menu's `Fetch origin` row spells the same facts in words.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoGit {
    /// A sample has landed and the directory is a git checkout. False draws
    /// nothing at all — no sample yet, or not a repository.
    #[serde(default)]
    pub sampled: bool,
    /// The branch name, or the short oid while HEAD is detached.
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub detached: bool,
    /// `origin/main` — the tracking ref. None means no arrows and nothing to
    /// fetch.
    #[serde(default)]
    pub upstream: Option<String>,
    /// Commits on the branch the upstream lacks: a push is due.
    #[serde(default)]
    pub ahead: u32,
    /// Commits on the upstream the branch lacks: a pull is due. Only ever
    /// moves after a fetch, mesimon's (opt-in) or the user's own.
    #[serde(default)]
    pub behind: u32,
    /// Newest commits first, capped at 100 per direction. None means the
    /// list is unavailable (including snapshots from older daemons).
    #[serde(default)]
    pub to_push: Option<Vec<GitCommit>>,
    #[serde(default)]
    pub to_pull: Option<Vec<GitCommit>>,
    /// Entries `git status` lists: modified, staged, unmerged and untracked.
    /// On a workspace (`repos` non-empty) it is the SUM over the root and
    /// every nested repo — the number the board's checkout diff then lists.
    #[serde(default)]
    pub changed: u32,
    /// The root's immediate children that are repositories of their own
    /// (T-225, `workspace::nested_repos`), sorted. Empty on an ordinary
    /// checkout. Non-empty means the board sits on a WORKSPACE — a meta repo
    /// over its children, or a plain folder of them (then `branch` is empty:
    /// the root itself has no HEAD) — and the header names the count where
    /// it would name a branch; the branch, arrows and upstream here are the
    /// root's own and speak for nothing under it.
    #[serde(default)]
    pub repos: Vec<String>,
    /// A fetch is running right now.
    #[serde(default)]
    pub fetching: bool,
    /// The periodic fetch cadence (`MESIMON_GIT_FETCH`, minutes → seconds);
    /// 0 = the opt-in is off and only the menu row fetches.
    #[serde(default)]
    pub fetch_every_secs: u64,
    /// When the last fetch succeeded (unix ms); 0 = never, this daemon.
    #[serde(default)]
    pub fetched_at_ms: u64,
    /// The last fetch failed — its first stderr line. The previous
    /// remote-tracking refs stand, so `behind` is simply older than it looks.
    #[serde(default)]
    pub fetch_error: Option<String>,
}

/// Pushed to subscribed clients whenever board state changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    BoardChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_git_sample_has_unavailable_commit_lists() {
        let g: RepoGit = serde_json::from_str(
            r#"{"sampled":true,"branch":"main","upstream":"origin/main","ahead":2}"#,
        )
        .unwrap();
        assert_eq!(g.ahead, 2);
        assert!(g.to_push.is_none());
        assert!(g.to_pull.is_none());
    }

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
            Response::Board { notices, worktrees, external, git, .. } => {
                assert!(notices.is_empty());
                assert!(worktrees.is_empty());
                assert!(external.is_empty());
                assert_eq!(git, RepoGit::default(), "absent git state reads as unsampled");
                assert!(!git.sampled);
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

    /// The sanitizer subtracts and never adds — the mechanical half of the
    /// README's zero-prompt-injection promise for a command that types into
    /// an agent's box.
    #[test]
    fn sanitize_prompt_only_ever_removes() {
        for raw in [
            "rebase onto main",
            "  padded  ",
            "run \u{1b}[31mthe\u{1b} tests\r\nnow",
            "tabs\there",
            &"é".repeat(9000),
        ] {
            let Some(out) = sanitize_prompt(raw) else { continue };
            let mut src = raw.chars();
            for c in out.chars() {
                assert!(
                    src.any(|s| s == c),
                    "sanitize_prompt introduced {c:?} that {raw:?} did not have"
                );
            }
            assert!(out.len() <= PROMPT_MAX_BYTES, "{} bytes", out.len());
        }
    }

    /// The three characters that would turn one prompt into a different
    /// event: CR submits early, ESC is read as a key, Tab completes. A line
    /// break is content (T-380): it rides the bracketed paste as a line of
    /// the same prompt, and `"\r\n"` is one of them.
    #[test]
    fn sanitize_prompt_drops_what_a_tty_would_act_on() {
        assert_eq!(sanitize_prompt("a\rb"), Some("ab".into()));
        assert_eq!(sanitize_prompt("a\nb"), Some("a\nb".into()));
        assert_eq!(sanitize_prompt("a\r\nb\n\nc"), Some("a\nb\n\nc".into()));
        assert_eq!(sanitize_prompt("a\u{1b}b"), Some("ab".into()));
        assert_eq!(sanitize_prompt("a\tb"), Some("ab".into()));
        // Nothing blank ever presses Enter.
        assert_eq!(sanitize_prompt(""), None);
        assert_eq!(sanitize_prompt("   "), None);
        assert_eq!(sanitize_prompt("\r\n\t"), None);
        // Bounded, and never split a char.
        let long = sanitize_prompt(&"é".repeat(9000)).expect("non-empty");
        assert!(long.len() <= PROMPT_MAX_BYTES, "{} bytes", long.len());
        assert!(long.chars().all(|c| c == 'é'));
    }
}
