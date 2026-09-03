//! Board app: state, keymap (M1 subset — the full 04 keymap lands in M2/M6),
//! MOVE mode with client-side ghost (07 §7 core rules), focus flow with GATE,
//! and the M3.5 ticket screen (Enter opens it; the old session picker is its
//! SESSIONS rail now).

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use mesimon_core::board::{
    Board, ExitReason, NoteMeta, Provenance, SessionKind, SessionState, TagRef, Ticket,
    WorkspaceStrategy,
};
use mesimon_core::command::{
    Command, ExternalItem, GraceItem, MergeOutcome, Resources, Response, WorktreeItem,
};
use mesimon_core::keymap::{self, Ctx, Key, Scope, Verb};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::client::Transport;
use crate::text::{EditBuffer, TextArea};
use crate::theme::{Flavor, Ground, Theme};

/// Which screen owns the keymap and the frame (07 §1). `Mode` remains the
/// board's sub-state; the ticket screen has no modes yet.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Board,
    Ticket {
        ticket: ulid::Ulid,
        rail_idx: usize,
    },
    /// Read-only diff viewer (M4b): ticket `v`. State lives in `App::diff`,
    /// not here — Screen is cloned on every keypress.
    Diff {
        ticket: ulid::Ulid,
    },
}

/// One shell pane's last lines, as last fetched (`Command::PaneTail`).
pub struct ShellTail {
    pub session: uuid::Uuid,
    /// Oldest line first — draw order.
    pub lines: Vec<String>,
    /// Last ATTEMPT, not last success: a daemon that cannot answer must be
    /// asked on the same slow beat as one that can.
    fetched: Instant,
}

impl ShellTail {
    pub(crate) fn new(session: uuid::Uuid, lines: Vec<String>) -> Self {
        Self { session, lines, fetched: Instant::now() }
    }
}

/// What the last draw of the ticket page's preview zone measured: which
/// document it showed, where it was scrolled to, and how far it could go.
/// Draw-side state, written by `ui/ticket.rs` and read by the `{ }` press
/// and the footer — the zone's height is a fact of the frame, so the page
/// size and the overflow can only be known there.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PreviewView {
    /// The document on screen: the selected session and, for a reply, the
    /// reply itself. `None` while the zone shows nothing.
    pub key: Option<u64>,
    /// Rows hidden above the window, after the clamp.
    pub offset: usize,
    /// The largest offset that still fills the window (0 = it all fits).
    pub max: usize,
    /// One press's worth of rows: the window less one row of overlap.
    pub page: usize,
    /// A shell tail sits at its bottom unless told otherwise — the newest
    /// line is what it is for — so scrolling back down to `max` hands it
    /// back to the pane rather than pinning it to today's last row.
    pub follows_tail: bool,
}

/// Everything the diff screen holds (M4b). Per-view and in-memory only —
/// no persistent caches; R and the density cycle recompute.
pub struct DiffState {
    pub ticket: ulid::Ulid,
    /// Restore `Screen::Ticket` on q/esc.
    pub rail_idx: usize,
    pub branch: String,
    pub base_oid: String,
    pub branch_oid: String,
    pub files: Vec<mesimon_core::diff::FileEntry>,
    pub file_idx: usize,
    /// Hunk-pane top row; draw clamps against the rendered height.
    pub scroll: Cell<usize>,
    /// Marquee clock for the selected file row's overflowing path — same
    /// behaviour as the board card title and the ticket rail (draw-side).
    pub marquee: Cell<Option<(usize, std::time::Instant)>>,
    /// -U context: 1 | 3 | 8 (`z z` cycles).
    pub density: u32,
    /// Fetched files, keyed by path — valid for the current density only.
    pub cache: std::collections::HashMap<String, mesimon_core::diff::FileDiff>,
    /// A first `z` arms the view chord (`z z` density, `z p` pane swap).
    pub z_armed: bool,
    /// Below the two-pane breakpoint: false shows the file list, true the diff.
    pub swap: bool,
    /// false = evicted: no dirty/untracked flags, `!` refused.
    pub worktree_present: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    /// MOVE: ghost position tracked client-side; nothing is sent until the
    /// drop. `grab` is the key that started it (`>`, `<`, or `m`): the same
    /// key again (or Enter) commits; for `>`/`<` the opposite key cancels,
    /// while an `m` grab has no opposite — `>`/`<` shift columns instead.
    /// `home` is where the grab happened (col, idx): a foreign column is
    /// always entered at the top, the home column at the ticket's own
    /// position (author 2026-08-30) — which is why `m` grabs in place.
    Move {
        ticket: ulid::Ulid,
        col: usize,
        idx: usize,
        grab: char,
        home: (usize, usize),
    },
    Input {
        purpose: InputPurpose,
        buffer: EditBuffer,
    },
    /// External drawer: discovered foreign sessions (19 §4 tier 1).
    External {
        idx: usize,
    },
    /// Archived-tickets dialog: restore or open from here.
    Archived {
        idx: usize,
    },
    /// The Esc menu (07 §16): everything that acts on the board as a whole,
    /// plus the two lists that are not the board.
    Menu {
        idx: usize,
    },
    /// The theme picker: `idx` is the cursor over `Flavor::ALL`, and the
    /// live `theme` IS the preview — nothing else is kept in step.
    Theme {
        idx: usize,
    },
    /// The note editor. A mode and not a second slot: it REPLACES the
    /// one-line composer (Tab carries the title over) and never coexists
    /// with a move, a menu or a picker, so `Mode` is where it belongs.
    /// `Screen` is untouched underneath, so Esc returns wherever the editor
    /// was opened from. Composing, it is a panel OVER the board (the header,
    /// the column headers and the footer stay in view, and it grows out of
    /// the phantom card it replaced — `Editor::grow`); on a note it takes
    /// the whole screen.
    Editor(Editor),
}

/// The note editor's state: a one-line title over a multi-line body.
#[derive(Debug, Clone, PartialEq)]
pub struct Editor {
    pub purpose: EditorPurpose,
    /// Composing: the new ticket's title, editable. On a note: the ticket's
    /// title, shown read-only on the same row so both purposes share a shape.
    pub title: EditBuffer,
    pub body: TextArea,
    pub focus: Field,
    /// (title, body) as opened or last saved; `dirty()` compares against it.
    baseline: (String, String),
    /// A first Esc on a dirty editor arms this; any other key clears it.
    pub esc_armed: bool,
    /// `^s` on an EMPTIED existing note is a delete, and takes two presses.
    pub delete_armed: bool,
    /// First visible body line; the draw follows the cursor and writes back.
    pub top: Cell<usize>,
    /// Where the composer panel is growing FROM — the phantom card's own
    /// rectangle on the board, and when Tab was pressed. The panel draws
    /// itself between that rectangle and its resting one for `GROW`, so the
    /// eye is carried from the one-line composer to the bigger room instead
    /// of being dropped into it. `None` on a note, and once settled.
    pub grow: Option<(ratatui::layout::Rect, Instant)>,
}

/// How long the composer panel takes to grow out of its card. One gesture,
/// one short motion, never a loop — the author asked for the transition
/// (2026-09-03) so the editor reads as the composer opened up, not as a
/// different place.
pub const GROW: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, PartialEq)]
pub enum EditorPurpose {
    /// A new ticket: title + description, with the mini composer's picks
    /// riding along so Shift+Tab and `^t` keep working in the bigger room.
    Compose { workspace: Option<WorkspaceStrategy>, tags: Vec<TagRef> },
    /// A note on a ticket that exists. `note: None` until the first save
    /// mints it.
    Note { ticket: ulid::Ulid, note: Option<ulid::Ulid> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Body,
}

impl Editor {
    pub(crate) fn new(
        purpose: EditorPurpose,
        title: EditBuffer,
        body: TextArea,
        focus: Field,
    ) -> Self {
        let baseline = (title.as_str().to_string(), body.as_str().to_string());
        Self {
            purpose,
            title,
            body,
            focus,
            baseline,
            esc_armed: false,
            delete_armed: false,
            top: Cell::new(0),
            grow: None,
        }
    }

    /// How far the panel has grown, 0.0 at the card and 1.0 at rest; `None`
    /// once the motion is over (or never started). Eased out, so the panel
    /// leaves the card fast and settles gently.
    pub fn grow_progress(&self) -> Option<f32> {
        let (_, at) = self.grow?;
        let t = at.elapsed().as_secs_f32() / GROW.as_secs_f32();
        if t >= 1.0 {
            return None;
        }
        Some(1.0 - (1.0 - t) * (1.0 - t))
    }

    pub fn dirty(&self) -> bool {
        self.title.as_str() != self.baseline.0 || self.body.as_str() != self.baseline.1
    }

    pub fn composing(&self) -> bool {
        matches!(self.purpose, EditorPurpose::Compose { .. })
    }

    fn saved(&mut self) {
        self.baseline = (self.title.as_str().to_string(), self.body.as_str().to_string());
    }
}

/// One row of the ticket page's rail: the sessions first, in spawn order,
/// then every note of the ticket. Sessions-first is an invariant
/// `board_enter` and the focus return lean on — a position in
/// `rail_sessions` IS a `rail_idx`.
#[derive(Debug, Clone, Copy)]
pub enum RailRow<'a> {
    Session(&'a mesimon_core::board::SessionRecord),
    Note(&'a NoteMeta),
}

/// One note's body as last fetched (`Command::ReadNote`). Keyed on the
/// note's `rev`, so a snapshot carrying a newer one reads as stale.
pub struct NoteText {
    pub rev: u64,
    /// `None` = the last attempt failed; retried after `NOTE_RETRY`.
    pub text: Option<String>,
    tried: Instant,
}

/// A prompt field's position in the ask history while `↑`/`↓` walk it.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryWalk {
    /// Index into `App::prompt_history` currently shown in the field.
    pub idx: usize,
    /// What was in the field when the walk began — restored by `↓` past
    /// the newest entry, so browsing never eats a half-typed prompt.
    pub draft: String,
}

/// How many asks the prompt field remembers. In memory only, per TUI run:
/// a recall aid, not a record — the transcript is the record.
const PROMPT_HISTORY_MAX: usize = 50;

/// The m key's staged progression (author 2026-08-30): each press shows what
/// the next press does. Stage is derived from git state, never stored.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MergeStage {
    /// ff possible — next m merges.
    Merge,
    /// default branch moved — next m asks the agent to rebase + test.
    Rebase,
    /// merged — next m tells the agent.
    Notify,
}

/// The most recent undoable action, for `u`. A delete carries its own
/// daemon-side grace band (the countdown in the advisory row); an archive is
/// remembered here until it is undone, superseded, or undone by someone else.
#[derive(Debug, Clone, Copy, PartialEq)]
enum LastUndo {
    Delete,
    Archive(ulid::Ulid),
}

/// The most recent action `.` can do again. A move is the only one so far —
/// it is the gesture that costs the most aiming and gets repeated the most
/// (triage is "these four go to done"). The column is remembered by NAME and
/// re-resolved against the live board every frame: an index would follow a
/// column that was renamed or removed into meaning something else.
#[derive(Debug, Clone, PartialEq)]
enum LastAction {
    Move { column: String },
}

/// Where a focus handover started — unfocus returns exactly there (author
/// 2026-08-30): board Enter comes back to the board, ticket-screen focus
/// comes back to the ticket screen.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FocusOrigin {
    Board,
    Ticket,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputPurpose {
    /// New-ticket composer. `workspace` is the Shift+Tab selector below the
    /// name (M4 layering): None = the board default (shared checkout).
    Create {
        workspace: Option<WorkspaceStrategy>,
        /// Tags picked with `^t` before the ticket exists. Sent as
        /// `SetTag` commands once `Response::Created` gives us an id — the
        /// same shape the workspace selector uses.
        tags: Vec<TagRef>,
    },
    Rename {
        id: ulid::Ulid,
    },
    /// The board's Shift+Enter: a one-line field on the selected card whose
    /// text goes to that ticket's live claude, submitted, with the board
    /// still up. Nothing here is saved and no ticket is minted — which is why
    /// `Ctx::composing` stays false and the input scope's `save` word
    /// becomes `send`.
    Prompt {
        ticket: ulid::Ulid,
        /// `Some` while `↑`/`↓` are walking [`App::prompt_history`]: where
        /// the walk is and the draft it stepped off, so `↓` past the newest
        /// ask puts the user's own words back. `None` is the ordinary field.
        walk: Option<HistoryWalk>,
    },
}

/// The `^t` tail's state. Unlike the delete and archive chords this is a
/// place you stay: an axis stays picked so the next digit can pick another
/// without a second `^t`.
///
/// The name field lives HERE rather than in a second `Mode::Input`, because
/// `Mode` is a single slot and the composer may already be holding it. Keeping
/// the buffer on the arm makes the gesture identical from the board, the
/// ticket screen and the composer.
#[derive(Debug, Clone, PartialEq)]
pub struct TagArm {
    /// The ticket being tagged. `None` while composing — the ticket does not
    /// exist yet and the picks buffer on `InputPurpose::Create`.
    pub ticket: Option<ulid::Ulid>,
    /// Cursor into the picker grid: which visible group row…
    pub row: usize,
    /// …and which cell along it. The last cell is `+ new` unless the group
    /// is full.
    pub col: usize,
    /// Typing a name, and whether it will create or rename.
    pub naming: Option<(Naming, EditBuffer)>,
    /// `d` has been pressed once: the next one deletes the tag board-wide.
    pub forget_armed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Naming {
    New,
    Rename,
}

/// `{`/`}` (and PgUp/PgDn) hunk-pane page step. The key handler cannot see
/// the rendered height, so this approximates a screenful; draw clamps.
const DIFF_PAGE: usize = 20;

/// How often the ticket page re-captures the selected shell's pane. Slow on
/// purpose: it is a fork per beat, and a terminal a person is reading rather
/// than driving does not need to be a live mirror.
const SHELL_TAIL_EVERY: Duration = Duration::from_millis(1000);
/// One `pgup`/`pgdn` in the editor body, in lines. The handler cannot see
/// the rendered height; a screenful is approximated.
const EDITOR_PAGE: usize = 20;
/// A note read that failed is asked again after this, not every tick.
const NOTE_RETRY: Duration = Duration::from_secs(2);
/// How many note bodies the TUI keeps; past it the oldest attempt goes.
const NOTE_CACHE_MAX: usize = 64;
/// How many lines to ask for — more than the zone can hold at any sane
/// height, so the draw does the trimming and a resize needs no refetch.
const SHELL_TAIL_LINES: u16 = 60;

/// How long a quick-tag digit holds the cursor card open. Long enough to
/// read a word off the chip row, short enough that it is a confirmation and
/// not a mode — and every repeat of the digit re-arms it, so cycling an axis
/// keeps the card open for the whole walk instead of blinking once per press.
const TAG_FLASH: Duration = Duration::from_millis(1500);

/// A field's byte limit as the status line says it: the round ones in KB
/// (`2 KB`, `4 KB`), a tag's `24 bytes`. Bytes, honestly — the cap is a byte
/// cap at the daemon, and a character count would be wrong in Hebrew.
fn limit_words(bytes: usize) -> String {
    if bytes > 0 && bytes % 1024 == 0 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

pub struct App {
    pub client: Box<dyn Transport>,
    pub repo_root: PathBuf,
    pub board: Board,
    pub grace: Vec<GraceItem>,
    pub external: Vec<ExternalItem>,
    pub resources: Resources,
    /// Per-ticket worktree bindings (M4): branch, status word, merged/conflict.
    pub worktrees: Vec<WorktreeItem>,
    /// Standing advisories from the daemon — a quarantined state file, a file
    /// a newer mesimon wrote. Refreshed with every snapshot. NOT `status`:
    /// that is cleared by the next keypress, and these stay true until fixed.
    pub notices: Vec<mesimon_core::command::Notice>,
    /// What the environment new panes get is doing — whether a shell startup
    /// file has moved since it was captured, and whether a reload is running.
    pub shell_env: mesimon_core::command::ShellEnvStatus,
    pub theme: Theme,
    /// The drawer row whose resume was refused as running-elsewhere — a
    /// second R on the same row sends the confirm override.
    resume_refused: Option<uuid::Uuid>,
    /// The m flow's armed stage: a first `m` names what the next `m` does;
    /// the second performs it. Any other key disarms.
    merge_armed: Option<(ulid::Ulid, MergeStage)>,
    /// The m flow's reply — rendered on the ticket screen's identity line
    /// (next to the branch state it acts on), never the footer. Cleared with
    /// `status` on the next keypress.
    pub merge_note: String,
    pub screen: Screen,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub mode: Mode,
    pub status: String,
    pub quit: bool,
    /// Board-column viewport (layout::board_geometry slides it); Cell because
    /// the draw pass owns it and draw takes &App.
    pub col_window: Cell<usize>,
    /// Marquee clock for the selected card's truncated title: which ticket is
    /// scrolling and since when (draw-side state).
    pub marquee: Cell<Option<(ulid::Ulid, std::time::Instant)>>,
    /// Same clock for the ticket rail's selected session name.
    pub rail_marquee: Cell<Option<(uuid::Uuid, std::time::Instant)>>,
    /// First visible card row of the cursor column (draw-side scroll state).
    pub scroll_row: Cell<usize>,
    /// Transcript peek (`p`): the cursor card also shows its latest assistant
    /// reply, read from the transcript at draw time (peek.rs).
    pub peek: bool,
    pub peek_cache: crate::peek::PeekCache,
    /// A quick-tag digit holds the card it tagged open for a moment (the
    /// ticket it landed on, and when). The stripe is one cell at rest and
    /// carries no words — it can say "two tags, these hues" and nothing
    /// more — so the press that changes it opens the card that names it,
    /// then lets go. Keyed to the TICKET: moving the cursor ends the reveal,
    /// because a flash is about the card you just tagged and no other.
    pub tag_flash: Option<(ulid::Ulid, Instant)>,
    /// The ticket page's preview zone: the selected shell's pane tail, and
    /// when it was fetched. Per-view and in memory only — a shell has no
    /// transcript file to read the way `peek_cache` reads an agent's, so
    /// this comes over the wire, and only while a shell is being looked at.
    pub shell_tail: Option<ShellTail>,
    /// Note bodies the ticket page has asked for, by note id. Bodies never
    /// ride the snapshot; `poll_notes` fetches the ones on screen, once per
    /// `(id, rev)`, and a save seeds it from our own text.
    pub notes: std::collections::HashMap<ulid::Ulid, NoteText>,
    /// Where `{ }` asked the preview zone to be: rows hidden above, and the
    /// document (`PreviewView::key`) that was asked for. Another document
    /// under the cursor — the rail moved, or a new reply landed — reads it
    /// as zero, so a page into one reply never opens the next one halfway.
    /// Draw clamps it and writes the clamp back, as the diff pane does.
    pub preview_scroll: Cell<Option<(u64, usize)>>,
    /// What the last draw of that zone measured (see `PreviewView`).
    pub preview_view: Cell<PreviewView>,
    /// Where the board last drew the composer's phantom card — the
    /// rectangle Tab's panel grows out of. Draw-side, like `preview_view`:
    /// the card's place on screen is a fact of the frame, not of the board.
    pub compose_card: Cell<Option<ratatui::layout::Rect>>,
    /// Working-spinner clock: epoch of the first draw (draw-side state, so
    /// the first rendered frame is always frame 0 — goldens stay stable).
    pub spin_epoch: Cell<Option<std::time::Instant>>,
    /// Diff-viewer state, Some while `Screen::Diff` is (or was just) open.
    pub diff: Option<DiffState>,
    /// Set when the user asked to focus: the main loop performs the handover
    /// outside the render loop.
    pub pending_attach: Option<Vec<String>>,
    /// cwd for the pending handover child (`!` shell in the worktree).
    pub pending_attach_cwd: Option<PathBuf>,
    pending_gate_then: Option<(uuid::Uuid, FocusOrigin)>,
    /// M4: `c` on an unprovisioned worktree ticket parks the spawn daemon-side
    /// (`Response::Provisioning`); this parks the focus half of that keypress.
    /// The first refresh that shows the replayed session finishes it; any
    /// other keypress abandons it (the user moved on — never yank focus).
    pending_spawn_focus: Option<(ulid::Ulid, SessionKind)>,
    /// The session a running handover holds focus on (and where the focus
    /// started) — released on return.
    focused_session_hint: Option<(uuid::Uuid, FocusOrigin)>,
    /// The ticket the composer just minted: its next plain Enter spawns claude
    /// straight away (the fresh-ticket fast path). Any other key closes the
    /// window — browsing away means the moment passed.
    just_created: Option<ulid::Ulid>,
    /// Every prompt sent from the board this run, oldest first, one copy of
    /// each (a repeat moves to the end), at most `PROMPT_HISTORY_MAX`. `↑`
    /// in a prompt field walks it — see `HistoryWalk`.
    prompt_history: Vec<String>,
    /// New-binary watch (dev rebuild or prod upgrade — same signal).
    update_watch: crate::update::UpdateWatch,
    /// Is a newer RELEASE published? Inert in every build `ci/release.sh`
    /// did not cut, so a dev board never asks and never downloads.
    release: crate::release::ReleaseWatch,
    /// U on a ready update: the main loop execs the new binary in place.
    pub pending_reexec: bool,
    /// The `d` chord is armed on this ticket: the next `d` deletes it, `D`
    /// deletes and discards the branch, anything else cancels.
    delete_armed: Option<ulid::Ulid>,
    /// The `a` chord, same shape: the next `a` archives, anything else
    /// cancels. Only ever armed when `a` would archive — restoring is one
    /// press, because undoing a mistake must not be harder than making it.
    archive_armed: Option<ulid::Ulid>,
    /// The `^t` tail is open. Checked BEFORE `Mode::Input` in `scope()` and
    /// in `handle_key`, so arming from the composer leaves the half-typed
    /// title untouched underneath and Esc returns to it.
    pub(crate) tag_armed: Option<TagArm>,
    /// What `u` would undo. Archiving is fully reversible and leaves the
    /// ticket in the snapshot, so it needs no daemon-side grace band — it
    /// just needs to be reachable, which is what this is.
    last_undo: Option<LastUndo>,
    /// What `.` would do again. Set only by an action the USER took here —
    /// an automove or another client's move is not something this hand did,
    /// so it never arms the key.
    last_action: Option<LastAction>,
    /// The `?` overlay is up. The next key — any key — puts it away.
    pub help: bool,
    /// `^L`: the main loop clears and redraws from scratch.
    pub force_redraw: bool,
    /// `^Z`: the main loop restores the terminal and raises SIGTSTP.
    pub pending_suspend: bool,
    /// Whether the diff viewer last drew above its two-pane breakpoint. Draw
    /// owns it (Cell), and the keymap reads it so `z s` hides itself when
    /// there is nothing to swap.
    pub diff_two_pane: Cell<bool>,
    /// The terminal answered the kitty-protocol probe (`lib.rs::init_terminal`
    /// sets it), so Shift+Enter arrives as its own atom. False by default —
    /// on the legacy floor every ShiftEnter binding stays unavailable, which
    /// is what keeps the composer from hinting a key that would land as a
    /// plain Enter.
    pub rich_keys: bool,
    /// Light/dark watch (`lib.rs::run` arms it, and only when the terminal
    /// answered the startup query). None means the ground is settled for the
    /// process: forced by `MESIMON_THEME`, mono, or a terminal that cannot
    /// be asked.
    pub flavor_watch: Option<crate::detect::GroundWatch>,
    /// The ground the terminal last reported — the slot a pick sets.
    pub ground: Ground,
    /// `MESIMON_THEME`, while it holds: cleared by a pick, which is the more
    /// recent explicit choice.
    pub forced: Option<Flavor>,
    /// The two slots (`prefs.rs`). `prefs_path` None means never write —
    /// every test app, and a machine with no HOME. This is the per-machine
    /// preference store the peek toggle (`p`) never had; carrying `p` here
    /// is a follow-up.
    pub prefs: crate::prefs::Prefs,
    pub prefs_path: Option<PathBuf>,
    /// A newer build wrote the file: picks last the session, nothing is
    /// written back.
    pub prefs_write_barred: bool,
    /// The flavor query's reply, when it comes back after the query gave up
    /// on it, arrives as keystrokes; this recognises and discards it ahead
    /// of everything else (`osc.rs`).
    reply_swallow: crate::osc::ReplySwallow,
    /// Daemon connection lost: keep the last board, re-dial on a slow cadence.
    daemon_down: bool,
    last_reconnect: Option<Instant>,
}

impl App {
    pub fn new(mut client: Box<dyn Transport>, repo_root: PathBuf, theme: Theme) -> Result<Self> {
        let (board, grace, external, resources, worktrees, notices) = fetch(client.as_mut())?;
        // Before the move: the checker resolves the state root and the
        // staging dir off the same repo path everything else keys on.
        let release = crate::release::ReleaseWatch::new(&repo_root);
        Ok(Self {
            client,
            repo_root,
            board,
            grace,
            external,
            resources,
            worktrees,
            notices,
            shell_env: Default::default(),
            theme,
            resume_refused: None,
            merge_armed: None,
            merge_note: String::new(),
            screen: Screen::Board,
            cursor_col: 0,
            cursor_row: 0,
            mode: Mode::Normal,
            status: String::new(),
            quit: false,
            col_window: Cell::new(0),
            marquee: Cell::new(None),
            rail_marquee: Cell::new(None),
            scroll_row: Cell::new(0),
            peek: false,
            peek_cache: crate::peek::PeekCache::default(),
            tag_flash: None,
            shell_tail: None,
            notes: std::collections::HashMap::new(),
            preview_scroll: Cell::new(None),
            preview_view: Cell::new(PreviewView::default()),
            compose_card: Cell::new(None),
            spin_epoch: Cell::new(None),
            diff: None,
            pending_attach: None,
            pending_attach_cwd: None,
            pending_gate_then: None,
            pending_spawn_focus: None,
            focused_session_hint: None,
            just_created: None,
            prompt_history: Vec::new(),
            rich_keys: false,
            flavor_watch: None,
            ground: Ground::Dark,
            forced: None,
            prefs: Default::default(),
            prefs_path: None,
            prefs_write_barred: false,
            reply_swallow: crate::osc::ReplySwallow::default(),
            update_watch: crate::update::UpdateWatch::new(),
            release,
            pending_reexec: false,
            delete_armed: None,
            tag_armed: None,
            archive_armed: None,
            last_undo: None,
            last_action: None,
            help: false,
            force_redraw: false,
            pending_suspend: false,
            diff_two_pane: Cell::new(true),
            daemon_down: false,
            last_reconnect: None,
        })
    }

    /// Whether something on screen is mid-motion and wants the next frame
    /// sooner than the spinner's cadence: the composer panel growing.
    pub fn animating(&self) -> bool {
        matches!(&self.mode, Mode::Editor(ed) if ed.grow_progress().is_some())
    }

    /// The working-spinner frame for this draw. The event loop redraws at
    /// least every 100 ms (`tick`'s poll timeout), which is what actually
    /// paces the animation; this just makes the frame a function of time so
    /// event bursts don't fast-forward it.
    pub fn spin_frame(&self) -> usize {
        let epoch = self.spin_epoch.get().unwrap_or_else(|| {
            let e = std::time::Instant::now();
            self.spin_epoch.set(Some(e));
            e
        });
        (epoch.elapsed().as_millis() as u64 / crate::glyphs::SPIN_STEP_MS) as usize
    }

    /// Soft on transport failure: a dead daemon keeps the last board on
    /// screen and flags the reconnect cadence instead of exiting the TUI.
    pub fn refresh(&mut self) -> Result<()> {
        match fetch(self.client.as_mut()) {
            Ok((board, grace, external, resources, worktrees, notices)) => {
                self.board = board;
                self.grace = grace;
                self.external = external;
                self.resources = resources;
                self.worktrees = worktrees;
                self.notices = notices;
                self.clamp_cursor();
                self.clamp_screen();
                if self.daemon_down {
                    self.daemon_down = false;
                    self.status = "daemon back ∙ board refreshed".into();
                }
                self.settle_pending_spawn_focus()?;
            }
            Err(_) => self.note_daemon_down(),
        }
        Ok(())
    }

    /// Finish a `c` whose spawn was parked behind worktree provisioning: once
    /// the replayed session shows up in a snapshot, focus it — that is what
    /// the keypress meant. A provisioning failure (or a replay that died
    /// silently after the binding attached) surfaces in the status line
    /// instead of leaving "session starts when ready" quietly unfulfilled.
    fn settle_pending_spawn_focus(&mut self) -> Result<()> {
        let Some((ticket, kind)) = self.pending_spawn_focus else {
            return Ok(());
        };
        if self.board.ticket(ticket).is_none() {
            self.pending_spawn_focus = None;
            return Ok(());
        }
        if let Some(sid) = self
            .rail_sessions(ticket)
            .iter()
            .find(|s| s.kind == kind && s.state.is_live())
            .map(|s| s.id)
        {
            self.pending_spawn_focus = None;
            return self.focus_session(sid);
        }
        match self.wt_item(ticket).map(|w| (w.status.clone(), w.detail.clone())) {
            Some((s, _)) if s == "queued" || s == "provisioning" => {} // still in flight
            // Binding landed in this snapshot with no session: the daemon's
            // replayed spawn errored (binding + session land in the same
            // writer turn, so "attached, no session" is never a race).
            Some((s, _)) if s == "attached" => {
                self.pending_spawn_focus = None;
                self.status = "worktree ready ∙ session spawn failed — c retries".into();
            }
            Some((s, detail)) => {
                self.pending_spawn_focus = None;
                self.status = format!(
                    "worktree {s} ∙ {}",
                    detail.unwrap_or_else(|| "provisioning failed".into())
                );
            }
            None => self.pending_spawn_focus = None, // binding vanished
        }
        Ok(())
    }

    fn note_daemon_down(&mut self) {
        self.daemon_down = true;
        self.status = "daemon unreachable ∙ reconnecting".into();
    }

    /// Request through the reconnect-tolerant seam: a transport failure
    /// becomes an ordinary Err response (every call site already surfaces
    /// those) and flags the reconnect cadence.
    fn req(&mut self, command: Command) -> Response {
        match self.client.request(command) {
            Ok(r) => r,
            Err(_) => {
                self.note_daemon_down();
                Response::Err { message: "daemon unreachable ∙ reconnecting".into() }
            }
        }
    }

    pub fn update_ready(&self) -> bool {
        self.update_watch.ready()
    }

    /// Render tests need the update offer without a real rebuild on disk.
    #[cfg(test)]
    pub(crate) fn force_update_ready(&mut self) {
        self.update_watch.force_ready();
    }

    /// And the release offer without a network or a release build to make it
    /// in — a test binary is stamped `dev`, so the checker is inert there.
    #[cfg(test)]
    pub(crate) fn force_release_available(&mut self, tag: &str) {
        self.release.force_available(tag);
    }

    /// Take whatever board a command replied with (RescanExternal does this).
    fn absorb_board(&mut self, resp: Response) {
        if let Response::Board {
            board,
            grace,
            external,
            resources,
            worktrees,
            notices,
            shell_env,
        } = resp
        {
            self.board = board;
            self.grace = grace;
            self.external = external;
            self.resources = resources;
            self.worktrees = worktrees;
            self.notices = notices;
            self.shell_env = shell_env;
            self.clamp_cursor();
            self.clamp_screen();
        }
    }

    pub fn columns(&self) -> Vec<String> {
        self.board.sorted_columns().iter().map(|c| c.name.clone()).collect()
    }

    pub fn selected_ticket(&self) -> Option<&Ticket> {
        let cols = self.columns();
        let col = cols.get(self.cursor_col)?;
        self.board.column_tickets(col).get(self.cursor_row).copied()
    }

    fn clamp_cursor(&mut self) {
        let cols = self.columns();
        if cols.is_empty() {
            return;
        }
        self.cursor_col = self.cursor_col.min(cols.len() - 1);
        let n = self.board.column_tickets(&cols[self.cursor_col]).len();
        self.cursor_row = self.cursor_row.min(n.saturating_sub(1));
    }

    /// Return to the board. Restarts the marquee clock so the selected
    /// card's title replays its reveal on re-landing.
    // A navigation verb ("go to the board"), not a conversion — the
    // wrong_self_convention lint reads the `to_` prefix as the latter.
    #[allow(clippy::wrong_self_convention)]
    fn to_board(&mut self) {
        self.marquee.set(None);
        self.rail_marquee.set(None);
        self.screen = Screen::Board;
    }

    /// A refresh can delete the ticket the ticket screen shows.
    fn clamp_screen(&mut self) {
        // Same for the archived dialog: a restore (here or from another
        // client) can empty the list under it.
        if matches!(self.mode, Mode::Archived { .. }) && self.board.archived_tickets().is_empty() {
            self.mode = Mode::Normal;
        }
        // A refresh can retire the menu row the cursor was on (the last thing
        // in done got slept elsewhere); clamp rather than point past the end.
        if let Mode::Menu { idx } = self.mode {
            let n = keymap::menu_items(&self.ctx()).len();
            if n == 0 {
                self.mode = Mode::Normal;
            } else if idx >= n {
                self.mode = Mode::Menu { idx: n - 1 };
            }
        }
        // A note editor on a ticket that vanished has nowhere to save to.
        if let Mode::Editor(Editor { purpose: EditorPurpose::Note { ticket, .. }, .. }) = &self.mode
        {
            if self.board.ticket(*ticket).is_none() {
                self.mode = Mode::Normal;
                self.status = "ticket gone ∙ note discarded".into();
            }
        }
        match &self.screen {
            Screen::Ticket { ticket, rail_idx } => {
                if self.board.ticket(*ticket).is_none() {
                    self.to_board();
                } else {
                    let n = self.rail_rows(*ticket).len();
                    let idx = (*rail_idx).min(n.saturating_sub(1));
                    self.screen = Screen::Ticket { ticket: *ticket, rail_idx: idx };
                }
            }
            // Ticket-vanish only. A binding going away must NOT exit: evicted
            // worktrees still render from the object store.
            Screen::Diff { ticket } => {
                if self.board.ticket(*ticket).is_none() {
                    self.diff = None;
                    self.to_board();
                }
            }
            Screen::Board => {}
        }
    }

    /// Poll one terminal event; returns whether a redraw is needed.
    pub fn tick(&mut self) -> Result<bool> {
        let mut dirty = false;
        // A transport-level advisory (build skew that would not settle) joins
        // the daemon's own notices in the advisory row.
        if let Some(n) = self.client.take_notice() {
            self.notices.insert(0, n);
            dirty = true;
        }
        // The header chip is the whole offer (`◦ update ready (U ∙ esc)`);
        // the footer would only say it twice, so this just asks for the
        // redraw that paints the chip.
        if self.update_watch.tick() {
            dirty = true;
        }
        // The release checker, on its own long clock: at most one question
        // every half hour, and none at all from a build that was not cut by
        // `ci/release.sh`. What it has to say is a chip, plus one status line
        // for the moments a download starts, lands or falls over.
        if self.release.tick() {
            if let Some(note) = self.release.take_note() {
                self.status = note;
            }
            dirty = true;
        }
        self.watch_flavor()?;
        // Async board-changed events from the daemon.
        while self.client.poll_event() {
            dirty = true;
        }
        if dirty {
            self.refresh()?;
        }
        // Reconnect cadence: the daemon went away (update restart, crash).
        // The reopen inside `refresh` respawns it when it is truly gone.
        if !self.client.healthy() && !self.daemon_down {
            self.note_daemon_down();
            dirty = true;
        }
        if self.daemon_down
            && self.last_reconnect.is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            self.last_reconnect = Some(Instant::now());
            self.refresh()?;
            dirty = true;
        }
        // The ticket page's preview zone, on its own slow cadence — it is
        // the one thing on screen the daemon does not push.
        dirty |= self.poll_shell_tail();
        dirty |= self.poll_notes();
        // The loop redraws once per tick, so the poll timeout is the frame
        // rate: 100 ms paces the spinner, and a panel in motion gets a
        // shorter one for the few frames it takes to settle.
        let frame = if self.animating() { 16 } else { 100 };
        if !event::poll(Duration::from_millis(frame))? {
            return Ok(dirty);
        }
        let key = match event::read()? {
            TermEvent::Key(key) => key,
            // Bracketed paste (armed by `lib.rs::init_terminal`): the whole
            // clipboard as one event, never as keystrokes.
            TermEvent::Paste(text) => return Ok(self.on_paste(&text)? || dirty),
            _ => return Ok(dirty),
        };
        if key.kind != KeyEventKind::Press {
            return Ok(dirty);
        }
        Ok(self.on_key(key.code, key.modifiers)? || dirty)
    }

    /// One raw key event from the terminal. The reply swallow sees it first —
    /// before the text-field barrier, before the keymap — because a late
    /// answer to our own colour query is the one thing on stdin that is not
    /// the user, and it must never reach either. Returns whether anything
    /// was handled.
    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<bool> {
        let keys = match self.reply_swallow.feed(code, mods) {
            crate::osc::Feed::Swallowed => return Ok(false),
            crate::osc::Feed::Pass(keys) => keys,
        };
        // Any keypress abandons a parked spawn-focus: the user moved on, and
        // yanking them into a session mid-thought is worse than not focusing.
        // (A new `c` re-arms it below; the session itself still spawns.)
        self.pending_spawn_focus = None;
        self.status.clear();
        self.merge_note.clear();
        for (code, mods) in keys {
            self.handle_key(code, mods)?;
        }
        Ok(true)
    }

    /// One bracketed paste from the terminal, as one event. Only a text
    /// field takes it — the composer, a rename, the ask field, a tag name —
    /// and everywhere else it is inert. Before bracketed paste was armed a
    /// paste arrived as keystrokes: a multi-line paste into the composer
    /// saved the first line on its newline and typed the rest onto the
    /// board as verbs, and a paste with no field open walked the keymap.
    /// The field flattens it to one line (see `EditBuffer::paste`) and the
    /// status line says when the field's limit cut it. Returns whether the
    /// screen changed.
    pub fn on_paste(&mut self, text: &str) -> Result<bool> {
        self.pending_spawn_focus = None;
        self.status.clear();
        self.merge_note.clear();
        let (what, pasted, limit) = if let Some(arm) = self.tag_armed.as_mut() {
            // The picker owns the keys while it is open; a paste with no
            // name field under it goes nowhere, not into the composer behind.
            let Some((_, buf)) = arm.naming.as_mut() else { return Ok(false) };
            ("a tag name", buf.paste(text), buf.limit())
        } else if let Mode::Input { purpose, buffer } = &mut self.mode {
            let what = match purpose {
                InputPurpose::Create { .. } | InputPurpose::Rename { .. } => "a title",
                InputPurpose::Prompt { .. } => "an ask",
            };
            (what, buffer.paste(text), buffer.limit())
        } else if let Mode::Editor(ed) = &mut self.mode {
            ed.esc_armed = false;
            ed.delete_armed = false;
            match ed.focus {
                // The body keeps the paste's newlines: a note is the one
                // field that reads block structure.
                Field::Body => ("a note", ed.body.paste(text), ed.body.limit()),
                Field::Title => ("a title", ed.title.paste(text), ed.title.limit()),
            }
        } else {
            return Ok(false);
        };
        if pasted.trimmed {
            self.status = format!("paste trimmed ∙ {what} holds at most {}", limit_words(limit));
        }
        Ok(true)
    }

    /// The ticket page's preview zone (shells only): re-ask the daemon for
    /// the pane's last lines. A shell has no transcript to read off disk —
    /// tmux is the only record it keeps — so this is the one view that pulls
    /// on a clock instead of on a push.
    ///
    /// It is deliberately narrow: one fork a second, and only while a ticket
    /// page has a live shell selected. The board never asks, a parked shell
    /// never asks, and moving off the row drops the state.
    fn poll_shell_tail(&mut self) -> bool {
        let Some(session) = self.selected_shell() else {
            return self.shell_tail.take().is_some();
        };
        if self
            .shell_tail
            .as_ref()
            .is_some_and(|t| t.session == session && t.fetched.elapsed() < SHELL_TAIL_EVERY)
        {
            return false;
        }
        let lines = match self.req(Command::PaneTail { session, lines: SHELL_TAIL_LINES }) {
            Response::PaneTail { lines } => lines,
            // A pane that just died, or a daemon mid-reconnect: keep the last
            // good capture rather than blinking the zone empty — the rail row
            // beside it is what says the session is gone — but still stamp
            // the attempt, or a refusal becomes a fork every 100 ms.
            _ => match self.shell_tail.as_mut() {
                Some(t) if t.session == session => {
                    t.fetched = Instant::now();
                    return false;
                }
                _ => Vec::new(),
            },
        };
        let same =
            self.shell_tail.as_ref().is_some_and(|t| t.session == session && t.lines == lines);
        self.shell_tail = Some(ShellTail::new(session, lines));
        !same
    }

    /// The note bodies the ticket page is showing — the description and the
    /// selected note row — fetched once per `(id, rev)`. Edge-triggered,
    /// never a clock: the steady state sends nothing. A refusal is stamped
    /// so it is retried on `NOTE_RETRY`, not every 100 ms.
    fn poll_notes(&mut self) -> bool {
        let Screen::Ticket { ticket, .. } = self.screen else {
            return false;
        };
        let mut wanted: Vec<(ulid::Ulid, u64)> = Vec::new();
        if let Some(t) = self.board.ticket(ticket) {
            if let Some(d) = t.description() {
                wanted.push((d.id, d.rev));
            }
        }
        if let Some(RailRow::Note(n)) = self.rail_row() {
            if !wanted.iter().any(|(id, _)| *id == n.id) {
                wanted.push((n.id, n.rev));
            }
        }
        let mut changed = false;
        for (id, rev) in wanted {
            let fresh = self.notes.get(&id).is_some_and(|n| {
                n.rev == rev && (n.text.is_some() || n.tried.elapsed() < NOTE_RETRY)
            });
            if fresh {
                continue;
            }
            let text = match self.req(Command::ReadNote { ticket, note: id }) {
                Response::Note { text, .. } => Some(crate::peek::sanitize(&text)),
                _ => None,
            };
            self.remember_note(id, rev, text);
            changed = true;
        }
        changed
    }

    pub(crate) fn remember_note(&mut self, id: ulid::Ulid, rev: u64, text: Option<String>) {
        if self.notes.len() >= NOTE_CACHE_MAX && !self.notes.contains_key(&id) {
            if let Some(oldest) = self.notes.iter().min_by_key(|(_, n)| n.tried).map(|(k, _)| *k) {
                self.notes.remove(&oldest);
            }
        }
        self.notes.insert(id, NoteText { rev, text, tried: Instant::now() });
    }

    /// A note's body, if it has been fetched and is not stale.
    pub fn note_text(&self, meta: &NoteMeta) -> Option<&str> {
        self.notes.get(&meta.id).filter(|n| n.rev == meta.rev).and_then(|n| n.text.as_deref())
    }

    /// The ticket page's selected rail session, when it is a shell with a
    /// live pane — the only session kind whose story is on a pane and not in
    /// a transcript.
    fn selected_shell(&self) -> Option<uuid::Uuid> {
        match self.rail_row()? {
            RailRow::Session(s) => {
                (s.kind == SessionKind::Bash && s.state.has_pane()).then_some(s.id)
            }
            RailRow::Note(_) => None,
        }
    }

    /// The rail row under the ticket page's cursor.
    pub fn rail_row(&self) -> Option<RailRow<'_>> {
        let Screen::Ticket { ticket, rail_idx } = &self.screen else {
            return None;
        };
        let rows = self.rail_rows(*ticket);
        rows.get((*rail_idx).min(rows.len().saturating_sub(1))).copied()
    }

    /// The selected rail note, if the cursor is on one.
    fn selected_note(&self) -> Option<ulid::Ulid> {
        match self.rail_row()? {
            RailRow::Note(n) => Some(n.id),
            RailRow::Session(_) => None,
        }
    }

    /// The OS flipped appearance (or the user flipped the terminal's theme)
    /// and the terminal followed: re-ask on the watch's cadence and rebuild
    /// the theme in the other flavor. No refresh — the daemon owns none of
    /// this, and the next frame is drawn unconditionally, so a swapped theme
    /// IS the repaint.
    ///
    /// The query reads the tty and discards whatever sits ahead of the reply,
    /// so it never runs with a keypress already waiting, and never under a
    /// text field where the cost of eating one would be a character lost from
    /// a title. Both cases just wait: `due` stays true and the next frame
    /// tries again.
    fn watch_flavor(&mut self) -> Result<()> {
        if !self.flavor_watch.as_ref().is_some_and(|w| w.due()) {
            return Ok(());
        }
        if self.typing() || event::poll(Duration::ZERO)? {
            return Ok(());
        }
        if let Some(ground) =
            self.flavor_watch.as_mut().and_then(|w| w.poll(crate::detect::query_ground))
        {
            self.ground = ground;
            // Under the picker the preview stays: the popup's header starts
            // naming the new slot and Enter writes that one. A watch and a
            // pin never coexist, so the slot's pick is the resting theme.
            if !matches!(self.mode, Mode::Theme { .. }) {
                self.preview(self.prefs.for_ground(ground));
            }
        }
        Ok(())
    }

    /// Wear a flavor now. The one road every retheme takes — the watch, the
    /// picker's cursor, Esc's put-back — so they cannot disagree about it.
    fn preview(&mut self, flavor: Flavor) {
        if self.theme.flavor == flavor {
            return;
        }
        self.theme = Theme::new(flavor, self.theme.profile);
        // Same recovery as ^L: repaint from nothing rather than trust a
        // cell-level diff to have touched every cell whose colour moved.
        self.force_redraw = true;
    }

    /// The theme the board rests on when nobody is previewing: the pin, or
    /// the current ground's slot.
    fn resting_flavor(&self) -> Flavor {
        self.forced.unwrap_or(self.prefs.for_ground(self.ground))
    }

    /// Enter in the picker: the slot the terminal is on takes the flavor,
    /// and the file follows where it may. A pick outranks `MESIMON_THEME`
    /// for the rest of the session — it is the more recent explicit choice —
    /// but the env var still pins the next launch, and the status says so.
    fn commit_theme(&mut self, flavor: Flavor) {
        let slot = match self.ground {
            Ground::Dark => "dark",
            Ground::Light => "light",
        };
        self.prefs.set(self.ground, flavor);
        let pinned = self.forced.take().is_some();
        self.mode = Mode::Normal;
        self.preview(flavor);
        let name = flavor.name();
        self.status = match self.prefs_path.as_ref() {
            _ if self.prefs_write_barred => format!(
                "{name} for this session ∙ prefs.json was written by a newer mesimon, not touched"
            ),
            None => format!("{name} for this session"),
            Some(path) => match crate::prefs::save(path, &self.prefs) {
                Ok(()) => format!("{name} saved for {slot} terminals"),
                Err(e) => {
                    format!("{name} for this session ∙ could not write {}: {e}", path.display())
                }
            },
        };
        if pinned {
            let var = std::env::var("MESIMON_THEME").unwrap_or_default();
            self.status.push_str(&format!(" ∙ MESIMON_THEME={var} pins the next launch"));
        }
    }

    /// A text field owns the keyboard: the composer, a rename, or a tag name.
    fn typing(&self) -> bool {
        matches!(self.mode, Mode::Input { .. } | Mode::Editor(_))
            || self.tag_armed.as_ref().is_some_and(|a| a.naming.is_some())
    }

    /// Which keymap owns this keypress. Derived, never stored — the chord
    /// tails (`d`, `z`) are scopes too, which is what makes a stray key inside
    /// a chord resolve to nothing and cancel instead of acting.
    pub fn scope(&self) -> Scope {
        // Before the input barrier on purpose: `^t` works while a title is
        // being typed, and the tail must own the digits until it closes.
        if let Some(arm) = self.tag_armed.as_ref() {
            // Naming a tag IS a text field, and saying so is what puts
            // enter/esc back in the footer: every tag binding stands down
            // while `tag_naming`, so `TagChord` would hint nothing at all.
            return if arm.naming.is_some() { Scope::Input } else { Scope::TagChord };
        }
        if matches!(self.mode, Mode::Input { .. }) {
            return Scope::Input;
        }
        if matches!(self.mode, Mode::Editor(_)) {
            return Scope::Editor;
        }
        if self.delete_armed.is_some() {
            return Scope::DeleteChord;
        }
        if self.archive_armed.is_some() {
            return Scope::ArchiveChord;
        }
        if self.diff.as_ref().is_some_and(|d| d.z_armed)
            && matches!(self.screen, Screen::Diff { .. })
        {
            return Scope::DiffView;
        }
        match &self.mode {
            Mode::Move { .. } => Scope::Move,
            Mode::Menu { .. } => Scope::Menu,
            Mode::External { .. } => Scope::Drawer,
            Mode::Archived { .. } => Scope::Archived,
            Mode::Theme { .. } => Scope::Theme,
            _ => match self.screen {
                Screen::Diff { .. } => Scope::Diff,
                Screen::Ticket { .. } => Scope::Ticket,
                Screen::Board => Scope::Board,
            },
        }
    }

    /// What this screen can do right now. The single input to every
    /// availability predicate and every state-dependent hint word, so the
    /// footer, the `?` overlay and the key dispatch cannot disagree.
    pub fn ctx(&self) -> Ctx {
        let sel = self.selected_ticket().map(|t| t.id);
        // On the ticket screen the "selected ticket" is the one being shown,
        // not whatever the board cursor happens to sit on.
        let subject = match &self.screen {
            Screen::Ticket { ticket, .. } | Screen::Diff { ticket } => Some(*ticket),
            Screen::Board => sel,
        };
        let sessions: Vec<&mesimon_core::board::SessionRecord> =
            subject.map(|t| self.rail_sessions(t)).unwrap_or_default();
        // The rail row under the cursor: a session, a note, or nothing. Every
        // `sel_*` session fact reads off `selected`, so on a note row they
        // are all false without a second thought.
        let row = self.rail_row();
        let selected = match row {
            Some(RailRow::Session(s)) => Some(s),
            _ => None,
        };
        let editor = match &self.mode {
            Mode::Editor(e) => Some(e),
            _ => None,
        };
        let wt = subject.and_then(|t| self.wt_item(t));
        let merge = subject.map(|t| self.merge_stage_word(t)).unwrap_or(None);
        Ctx {
            has_ticket: subject.is_some(),
            multi_column: self.columns().len() > 1,
            ticket_has_sessions: !sessions.is_empty(),
            ticket_has_claude: sessions
                .iter()
                .any(|s| s.kind == SessionKind::Claude && s.state.is_live()),
            // A pane, not merely a session: `is_live()` counts a parked one,
            // and a prompt needs somewhere to be typed. Mirrors the daemon's
            // `prompt_target`, which is what actually picks the session.
            ticket_promptable: subject.is_some_and(|t| self.board.pane_target(t).is_some()),
            ticket_awake: subject.map(|t| self.board.ticket_awake_sessions(t) > 0).unwrap_or(false),
            ticket_archived: subject
                .and_then(|t| self.board.ticket(t))
                .is_some_and(|t| t.is_archived()),
            ticket_hot: sessions.iter().any(|s| {
                s.kind == SessionKind::Claude
                    && matches!(
                        s.state,
                        SessionState::Running | SessionState::RequiresAction { .. }
                    )
            }),
            can_undo: self.undo_target().is_some(),
            undo_word: match self.undo_target() {
                Some(LastUndo::Archive(_)) => "undo archive",
                _ => "undo delete",
            },
            can_nudge: self.can_nudge(),
            can_repeat: self.repeat_target().is_some(),
            repeat_word: match self.last_action {
                Some(LastAction::Move { .. }) => "move again",
                None => "again",
            },
            bulk_sleep: self.resources.reclaim_sessions,
            bulk_sleep_bytes: self.resources.reclaim_bytes,
            bulk_archive: self.resources.archive_tickets,
            has_archived: !self.board.archived_tickets().is_empty(),
            peek_on: self.peek,
            theme_name: self.theme.flavor.name(),
            theme_blurb: self.theme.flavor.blurb(),
            theme_slot_word: match self.ground {
                Ground::Dark => "dark",
                Ground::Light => "light",
            },
            theme_pinned: self.forced.is_some(),
            preview_scrolls: self.preview_view.get().max > 0,
            update_ready: self.update_ready(),
            // A binary already waiting on disk outranks a download: reload
            // what you have before fetching it again. This is also what keeps
            // the two rows from standing in the menu together after an
            // `install.sh` run in another terminal.
            release_available: self.release.available() && !self.update_ready(),
            release_tag: self.release.tag(),
            // `reloading` takes the offer down the instant the press lands, so
            // a slow rc file does not leave the chip standing as if it missed.
            shell_env_stale: self.shell_env.stale && !self.shell_env.reloading,
            shell_env_failed: self.shell_env.failed && !self.shell_env.reloading,
            any_attention: !mesimon_core::attention::attention_queue(&self.board).is_empty(),
            sel_session: selected.is_some(),
            sel_note: matches!(row, Some(RailRow::Note(_))),
            ticket_described: subject
                .and_then(|t| self.board.ticket(t))
                .is_some_and(|t| t.description().is_some()),
            sel_sleeping: selected.is_some_and(|s| matches!(s.state, SessionState::Sleeping)),
            sel_dead: selected.is_some_and(|s| !s.state.is_live()),
            sel_pinned: selected.is_some_and(|s| s.pinned_awake),
            has_worktree: wt.is_some_and(|w| !w.branch.is_empty()),
            merge_actionable: merge.is_some(),
            merge_word: merge.unwrap_or("merge"),
            two_pane: self.diff_two_pane.get(),
            worktree_present: self.diff.as_ref().is_some_and(|d| d.worktree_present),
            density_word: self
                .diff
                .as_ref()
                .map(|d| crate::ui::diff::density_word(d.density))
                .unwrap_or("context lines"),
            composing: matches!(
                self.mode,
                Mode::Input { purpose: InputPurpose::Create { .. }, .. }
            ),
            prompting: matches!(
                self.mode,
                Mode::Input { purpose: InputPurpose::Prompt { .. }, .. }
            ),
            prompt_history: !self.prompt_history.is_empty(),
            tag_naming: self.tag_armed.as_ref().is_some_and(|a| a.naming.is_some()),
            tag_on_entry: self.tag_cell().is_some(),
            tag_worn: self.tag_cell().is_some_and(|(g, n, _)| {
                self.tag_subject().is_some_and(|t| t.iter().any(|t| t.group == g && t.name == n))
            }),
            tag_forget_armed: self.tag_armed.as_ref().is_some_and(|a| a.forget_armed),
            // Board-wide, because the ten digits share one binding and `avail`
            // never sees which one was pressed. A digit whose own group is
            // empty says so in the status line instead.
            tags_exist: !self.board.tags.is_empty(),
            editing: editor.is_some(),
            editor_composing: editor.is_some_and(|e| e.composing()),
            editor_body: editor.is_some_and(|e| e.focus == Field::Body),
            editor_dirty: editor.is_some_and(|e| e.dirty()),
            editor_can_tell: editor.is_some_and(|e| {
                !e.dirty()
                    && matches!(
                        e.purpose,
                        EditorPurpose::Note { ticket, note: Some(_) }
                            if self.board.pane_target(ticket).is_some()
                    )
            }),
            rich_keys: self.rich_keys,
        }
    }

    /// What the next `m` would do, or `None` when `m` is inert — the
    /// availability predicate and the hint word behind the `m` binding. It
    /// mirrors `merge_key`'s stage derivation; `merge_stage_matches_key` holds
    /// the two together.
    fn merge_stage_word(&self, ticket: ulid::Ulid) -> Option<&'static str> {
        let w = self.wt_item(ticket)?;
        if w.branch.is_empty() || w.status != "attached" {
            return None;
        }
        if w.merged {
            Some("tell the agent it merged")
        } else if w.needs_rebase {
            Some("ask the agent to rebase")
        } else if w.ahead > 0 && !self.ticket_busy(ticket) {
            Some("merge")
        } else {
            None
        }
    }

    /// The full key dispatch, seam for the TestBackend harness. Every path
    /// from a keypress to an action runs through `keymap::resolve`, and the
    /// verb it returns is matched exhaustively below — so a binding with no
    /// handler is a compile error, not a dead key.
    pub fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        // The tag tail outranks the input barrier: `^t` is reachable from the
        // composer, so the keys that follow it must not land in the title.
        if self.tag_armed.is_some() {
            return self.key_tag(code, mods);
        }
        // Text input is a scope barrier: it owns every key, including Tab, and
        // an atom it does not bind is a character to type.
        if let Mode::Input { .. } = self.mode {
            return self.key_input(code, mods);
        }
        if let Mode::Editor(_) = self.mode {
            return self.key_editor(code, mods);
        }
        let Some(key) = crate::keys::to_key(code, mods) else {
            return Ok(());
        };
        // The help overlay swallows the next key, whatever it is: it is a
        // reference card, and any key is "I'm done reading".
        if self.help {
            self.help = false;
            if key == Key::Char('?') || key == Key::Esc || key == Key::Char('q') {
                return Ok(());
            }
        }
        let scope = self.scope();
        let ctx = self.ctx();
        // Disarm the two things a keypress can be standing in the middle of.
        // A chord tail keeps its own arming (the scope carries it) until the
        // dispatch below either consumes it or cancels it.
        let was = (self.delete_armed.take(), self.archive_armed.take());
        if scope != Scope::DeleteChord {
            self.delete_armed = None;
        }
        if scope != Scope::ArchiveChord {
            self.archive_armed = None;
        }
        if !matches!(key, Key::Char('m')) {
            self.merge_armed = None;
        }
        // The fresh-ticket window lives exactly one Enter long.
        if !matches!(key, Key::Enter) {
            self.just_created = None;
        }
        let Some(verb) = keymap::resolve(scope, key, &ctx) else {
            // Unbound here, or bound but unavailable. Inside a chord tail that
            // means "never mind" — and the status says so, because a chord
            // that silently evaporates is worse than one that reports.
            if scope == Scope::DeleteChord {
                self.status = "delete cancelled".into();
            } else if scope == Scope::ArchiveChord {
                self.status = "archive cancelled".into();
            } else if scope == Scope::DiffView {
                if let Some(d) = self.diff.as_mut() {
                    d.z_armed = false;
                }
                return self.handle_key(code, mods);
            }
            return Ok(());
        };
        self.delete_armed = was.0;
        self.archive_armed = was.1;
        self.dispatch(verb, key, scope, &ctx)
    }

    /// One exhaustive match on [`Verb`]. Adding a binding to the table without
    /// handling it here does not compile.
    fn dispatch(&mut self, verb: Verb, key: Key, scope: Scope, ctx: &Ctx) -> Result<()> {
        match verb {
            // ---- global ----------------------------------------------------
            Verb::Help => self.help = true,
            Verb::NextAttention => self.cycle_attention(false),
            Verb::PrevAttention => self.cycle_attention(true),
            Verb::Reload => {
                let _ = self.client.request(Command::Shutdown);
                self.pending_reexec = true;
            }
            Verb::InstallUpdate => {
                if let Some(tag) = self.release.begin_install() {
                    // The offer comes down with the press (the shell-env
                    // reload's discipline), so while the fetch runs this line
                    // is the only place it is said — and it says where this
                    // stops, because nothing here restarts anything.
                    self.status = format!("downloading {tag} ∙ nothing restarts until you say so");
                }
            }
            Verb::Redraw => self.force_redraw = true,
            Verb::Suspend => self.pending_suspend = true,
            Verb::Undo => match self.undo_target() {
                Some(LastUndo::Archive(id)) => {
                    self.last_undo = None;
                    self.unarchive(id)?;
                }
                Some(LastUndo::Delete) => {
                    if let Some(g) = self.grace.last() {
                        let id = g.id;
                        self.last_undo = None;
                        self.send(Command::RestoreTicket { id })?;
                    }
                }
                None => {}
            },
            // ---- navigation ------------------------------------------------
            Verb::CursorLeft | Verb::CursorRight | Verb::CursorUp | Verb::CursorDown => {
                self.nav(verb, scope)
            }
            Verb::First => {
                self.cursor_row = 0;
            }
            Verb::Last => {
                self.cursor_row = usize::MAX;
                self.clamp_cursor();
            }
            Verb::Act => self.act(scope)?,
            Verb::Back => self.back(scope),
            Verb::Quit => self.quit = true,
            Verb::Menu => self.mode = Mode::Menu { idx: 0 },
            // ---- tickets ---------------------------------------------------
            Verb::OpenTicket => {
                self.mode = Mode::Input {
                    purpose: InputPurpose::Create { workspace: None, tags: Vec::new() },
                    buffer: EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
                };
            }
            Verb::TicketScreen => {
                if let Some(t) = self.selected_ticket() {
                    self.screen = Screen::Ticket { ticket: t.id, rail_idx: 0 };
                }
            }
            Verb::Rename => {
                if let Some(t) = self.subject().and_then(|id| self.board.ticket(id)) {
                    self.mode = Mode::Input {
                        purpose: InputPurpose::Rename { id: t.id },
                        buffer: EditBuffer::from_text(
                            t.title.clone(),
                            mesimon_core::board::TITLE_MAX_BYTES,
                        ),
                    };
                }
            }
            Verb::NoteEdit => {
                if let Some(ticket) = self.subject() {
                    let note = self.selected_note().or_else(|| {
                        self.board.ticket(ticket).and_then(|t| t.description()).map(|n| n.id)
                    });
                    self.open_note_editor(ticket, note)?;
                }
            }
            Verb::NoteNew => {
                if let Some(ticket) = self.subject() {
                    self.open_note_editor(ticket, None)?;
                }
            }
            // `d` only arms. The second press is what deletes.
            Verb::DeletePrefix => {
                if let Some(id) = self.subject() {
                    self.delete_armed = Some(id);
                    // Only what the next press does. That anything else
                    // cancels is learned once, on the first stray key.
                    self.status = if ctx.has_worktree {
                        "d again deletes ∙ D also discards the branch".into()
                    } else {
                        "d again deletes".into()
                    };
                }
            }
            Verb::Delete | Verb::DeleteDiscard => {
                if let Some(id) = self.delete_armed.take() {
                    self.delete_gated(id, verb == Verb::DeleteDiscard)?;
                }
            }
            Verb::Grab => self.grab(key, scope, ctx)?,
            Verb::Nudge => self.nudge(key)?,
            Verb::Repeat => self.repeat_last()?,
            // `a` on a ticket that is already archived restores it right
            // away; otherwise it arms, and the second `a` archives.
            Verb::TagPrefix => {
                // While composing the ticket does not exist yet; the picks
                // buffer on the composer and travel with it on save.
                let ticket = if ctx.composing { None } else { self.subject() };
                if ticket.is_none() && !ctx.composing {
                    return Ok(());
                }
                self.tag_armed =
                    Some(TagArm { ticket, row: 0, col: 0, naming: None, forget_armed: false });
                self.status.clear();
            }
            Verb::TagLeft | Verb::TagRight | Verb::TagUp | Verb::TagDown => {
                self.tag_move(verb);
            }
            Verb::TagCarryLeft | Verb::TagCarryRight | Verb::TagCarryUp | Verb::TagCarryDown => {
                self.tag_carry(verb)?;
            }
            Verb::TagGroup => {
                let Key::Char(c) = key else { return Ok(()) };
                let Some(d) = c.to_digit(10) else { return Ok(()) };
                // `0` is group 10 — the row it addresses, not the number it
                // spells.
                let group = if d == 0 { 10u8 } else { d as u8 };
                let rows = crate::ui::tag_rows(self);
                let Some(row) = rows.iter().position(|g| *g == group) else { return Ok(()) };
                let len = crate::ui::tag_row_len(self, group).max(1);
                if let Some(arm) = self.tag_armed.as_mut() {
                    // Pressing the same digit again steps along that row and
                    // wraps at its end, so one finger reaches every tag on an
                    // axis and keeps reaching them — a key that goes dead on
                    // the last cell asks for a second key to get back.
                    if arm.row == row {
                        arm.col = (arm.col + 1) % len;
                    } else {
                        arm.row = row;
                        arm.col = 0;
                    }
                    arm.forget_armed = false;
                }
                self.tag_clamp();
            }
            Verb::TagColor => {
                let Some((group, name, tint)) = self.tag_cell() else { return Ok(()) };
                let next = (tint + 1) % mesimon_core::board::TAG_TINTS;
                if let Response::Err { message } =
                    self.req(Command::SetTagColor { group, name, color: next })
                {
                    self.status = message;
                }
                self.refresh()?;
            }
            Verb::TagRename => {
                let Some((_, name, _)) = self.tag_cell() else { return Ok(()) };
                if let Some(arm) = self.tag_armed.as_mut() {
                    arm.naming = Some((
                        Naming::Rename,
                        EditBuffer::from_text(name, mesimon_core::board::TAG_MAX_BYTES),
                    ));
                }
            }
            Verb::TagToggle => {
                // Naming borrows Enter; the hint says so.
                if self.tag_armed.as_ref().is_some_and(|a| a.naming.is_some()) {
                    return self.commit_tag_name();
                }
                match self.tag_cell() {
                    // A real tag: wear it, or take it off if already worn.
                    Some((group, name, _)) => {
                        let worn = self
                            .tag_subject()
                            .is_some_and(|t| t.iter().any(|t| t.group == group && t.name == name));
                        let next = if worn { None } else { Some(name) };
                        self.apply_tag(group, next)?;
                        self.refresh()?;
                    }
                    // The `+ new` cell.
                    None => {
                        if let Some(arm) = self.tag_armed.as_mut() {
                            arm.naming = Some((
                                Naming::New,
                                EditBuffer::new(mesimon_core::board::TAG_MAX_BYTES),
                            ));
                        }
                    }
                }
            }
            Verb::TagForget => {
                let Some((group, name, _)) = self.tag_cell() else { return Ok(()) };
                let armed = self.tag_armed.as_ref().is_some_and(|a| a.forget_armed);
                if !armed {
                    let wearers =
                        self.board.tickets.iter().filter(|t| t.wears(group, &name)).count();
                    if let Some(arm) = self.tag_armed.as_mut() {
                        arm.forget_armed = true;
                    }
                    // Name the blast radius before asking for the second
                    // press, not after it.
                    let noun = if wearers == 1 { "ticket" } else { "tickets" };
                    self.status = format!(
                        "d again deletes {name} from the board ∙ {wearers} {noun} wearing it"
                    );
                    return Ok(());
                }
                if let Some(arm) = self.tag_armed.as_mut() {
                    arm.forget_armed = false;
                }
                match self.req(Command::ForgetTag { group, name: name.clone() }) {
                    Response::Err { message } => self.status = message,
                    _ => self.status = format!("{name} deleted"),
                }
                self.refresh()?;
                self.tag_clamp();
            }
            Verb::TagCycle => {
                let Key::Char(c) = key else { return Ok(()) };
                let Some(d) = c.to_digit(10) else { return Ok(()) };
                // `0` is group 10 — the row it addresses, not the number it
                // spells, exactly as inside the picker.
                let group = if d == 0 { 10u8 } else { d as u8 };
                self.cycle_tag(group)?;
            }
            Verb::TagDone => {
                // Esc backs out of the name field first, and only closes the
                // picker on a second press — losing a half-typed name AND the
                // panel to one key is a gesture nobody means.
                let naming = self.tag_armed.as_ref().is_some_and(|a| a.naming.is_some());
                if naming {
                    if let Some(arm) = self.tag_armed.as_mut() {
                        arm.naming = None;
                    }
                } else {
                    self.tag_armed = None;
                }
                self.status.clear();
            }
            Verb::ArchivePrefix => {
                if let Some(id) = self.subject() {
                    if ctx.ticket_archived {
                        self.unarchive(id)?;
                    } else if self.board.ticket_awake_sessions(id) > 0 {
                        // Say why before asking for a second press we would
                        // only refuse.
                        self.archive_gated(id)?;
                    } else {
                        self.archive_armed = Some(id);
                        self.status = "a again archives".into();
                    }
                }
            }
            Verb::Archive => {
                if let Mode::Archived { idx } = self.mode {
                    let list: Vec<ulid::Ulid> =
                        self.board.archived_tickets().iter().map(|t| t.id).collect();
                    if let Some(id) = list.get(idx.min(list.len().saturating_sub(1))).copied() {
                        self.unarchive(id)?;
                    }
                } else if let Some(id) = self.archive_armed.take() {
                    self.archive_gated(id)?;
                }
            }
            Verb::ReloadShellEnv => {
                self.send(Command::ReloadShellEnv)?;
                // Names the boundary in the same breath as the confirmation: a
                // running process's environment cannot be changed, so a live
                // pane keeps what it was born with and sleep/wake is the way
                // an existing session picks the new one up.
                self.status = "re-reading your shell environment ∙                                new and woken sessions get it"
                    .into();
            }
            Verb::ArchiveAllDone => {
                match self.req(Command::ArchiveAll) {
                    Response::Archived { archived, skipped } => {
                        self.status = match (archived, skipped) {
                            (0, 0) => "nothing was ready to archive".into(),
                            (n, 0) => format!("archived {n} ∙ esc menu lists them"),
                            (n, k) => format!("archived {n} ∙ {k} not ready"),
                        };
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.mode = Mode::Normal;
                self.refresh()?;
            }
            // ---- sessions --------------------------------------------------
            Verb::Claude | Verb::Shell => {
                let kind =
                    if verb == Verb::Claude { SessionKind::Claude } else { SessionKind::Bash };
                if let Some(id) = self.subject() {
                    self.focus_kind_or_spawn(id, kind)?;
                }
            }
            Verb::ShellNew => {
                if let Some(id) = self.subject() {
                    self.spawn_and_focus(id, SessionKind::Bash)?;
                }
            }
            // Open the field on the card and get out of the way. Nothing is
            // sent here — the press that opens a prompt must not also be the
            // press that delivers one. The exception is an EMPTY seat, where
            // the title IS the prompt: that is the composer's Shift+Enter a
            // press late, and it starts claude on the title without a field.
            Verb::Prompt => {
                if let Some(id) = self.subject() {
                    if ctx.ticket_has_claude {
                        self.mode = Mode::Input {
                            purpose: InputPurpose::Prompt { ticket: id, walk: None },
                            buffer: EditBuffer::new(mesimon_core::command::PROMPT_MAX_BYTES),
                        };
                    } else {
                        self.start_composed(id);
                    }
                }
            }
            Verb::Sleep => self.sleep_verb(scope, ctx)?,
            Verb::SleepAllDone => {
                match self.req(Command::ReclaimAll) {
                    Response::Reclaimed { slept, skipped } => {
                        self.status = match (slept, skipped) {
                            (0, 0) => "nothing in done to sleep".into(),
                            (n, 0) => format!("slept {n}"),
                            (n, k) => format!("slept {n} ∙ {k} not ready"),
                        };
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.mode = Mode::Normal;
                self.refresh()?;
            }
            Verb::Pin => {
                if let Some(sid) = self.selected_session() {
                    let pinned = !ctx.sel_pinned;
                    self.send(Command::PinAwake { id: sid, pinned })?;
                    self.status = if pinned { "kept awake".into() } else { "free to sleep".into() };
                }
            }
            // ---- worktree --------------------------------------------------
            Verb::Merge => {
                if let Some(id) = self.subject() {
                    self.merge_key(id)?;
                }
            }
            Verb::OpenDiff => {
                if let Screen::Ticket { ticket, rail_idx } = self.screen {
                    self.open_diff(ticket, rail_idx)?;
                }
            }
            Verb::WorktreeShell => self.worktree_shell(),
            // ---- diff ------------------------------------------------------
            Verb::ScrollDown => self.diff_scroll(1),
            Verb::ScrollUp => self.diff_scroll(-1),
            // One pair of keys, two read-only zones: the diff's hunk pane
            // and the ticket page's preview. Which one is the screen's to say.
            Verb::PageDown | Verb::PageUp => {
                let dir: isize = if verb == Verb::PageDown { 1 } else { -1 };
                match self.screen {
                    Screen::Diff { .. } => self.diff_scroll(dir * DIFF_PAGE as isize),
                    Screen::Ticket { .. } => self.preview_page(dir),
                    Screen::Board => {}
                }
            }
            Verb::NextFile => self.diff_nav(1),
            Verb::PrevFile => self.diff_nav(-1),
            Verb::Refresh => {
                if let Screen::Diff { ticket } = self.screen {
                    self.diff_refresh(ticket);
                }
            }
            Verb::ViewPrefix => {
                if let Some(d) = self.diff.as_mut() {
                    d.z_armed = true;
                }
            }
            Verb::Density => {
                let idx = {
                    let Some(d) = self.diff.as_mut() else {
                        return Ok(());
                    };
                    d.z_armed = false;
                    d.density = match d.density {
                        1 => 3,
                        3 => 8,
                        _ => 1,
                    };
                    d.cache.clear();
                    d.scroll.set(0);
                    let noun = if d.density == 1 { "line" } else { "lines" };
                    self.status = format!(
                        "{} ∙ {} context {noun} around each change",
                        crate::ui::diff::density_word(d.density),
                        d.density
                    );
                    d.file_idx
                };
                self.diff_fetch(idx);
            }
            Verb::SwapPanes => {
                if let Some(d) = self.diff.as_mut() {
                    d.z_armed = false;
                    d.swap = !d.swap;
                }
            }
            // ---- move ------------------------------------------------------
            Verb::Drop => {
                if let Mode::Move { ticket, col, idx, .. } = self.mode {
                    let cols = self.columns();
                    self.drop_ghost(&cols, ticket, col, idx)?;
                }
            }
            Verb::DropColumn => {
                if let (Mode::Move { ticket, col, idx, grab, home }, Key::Char(c)) =
                    (self.mode.clone(), key)
                {
                    let cols = self.columns();
                    let want = c.to_digit(10).unwrap_or(1).saturating_sub(1) as usize;
                    if want < cols.len() {
                        let idx = if want == col {
                            idx
                        } else {
                            self.ghost_entry_idx(&cols, want, home, ticket)
                        };
                        self.mode = Mode::Move { ticket, col: want, idx, grab, home };
                    }
                }
            }
            Verb::Cancel => self.mode = Mode::Normal,
            // ---- view / lists ----------------------------------------------
            Verb::Peek => {
                self.peek = !self.peek;
                self.status = if self.peek {
                    "showing the latest reply under the selected card".into()
                } else {
                    "replies hidden".into()
                };
                self.mode = Mode::Normal;
            }
            Verb::ExternalDrawer => self.open_drawer()?,
            Verb::ArchivedList => {
                if self.board.archived_tickets().is_empty() {
                    self.status = "nothing archived".into();
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::Archived { idx: 0 };
                }
            }
            Verb::ThemePick => {
                let idx = Flavor::ALL.iter().position(|f| *f == self.theme.flavor).unwrap_or(0);
                self.mode = Mode::Theme { idx };
            }
            Verb::AdoptObserve => self.adopt_external(false)?,
            // ---- input (handled in key_input; unreachable here) -------------
            Verb::Save
            | Verb::SaveStart
            | Verb::CycleWorkspace
            | Verb::EditLeft
            | Verb::EditRight
            | Verb::EditWordLeft
            | Verb::EditWordRight
            | Verb::EditHome
            | Verb::EditEnd
            | Verb::EditBackspace
            | Verb::EditDelete
            | Verb::EditDeleteWord
            | Verb::EditKillToStart
            | Verb::HistoryPrev
            | Verb::HistoryNext
            | Verb::Describe
            // ---- editor (handled in key_editor; unreachable here) ----------
            | Verb::EditorSave
            | Verb::EditorNewline
            | Verb::EditorUp
            | Verb::EditorDown => {}
        }
        Ok(())
    }

    /// What `u` would undo, or `None`. Re-derived from the board rather than
    /// trusted: a grace band expires on its own, and the ticket we remember
    /// archiving may have been restored from another client.
    fn undo_target(&self) -> Option<LastUndo> {
        match self.last_undo {
            Some(LastUndo::Archive(id)) => self
                .board
                .ticket(id)
                .is_some_and(|t| t.is_archived())
                .then_some(LastUndo::Archive(id)),
            // The grace band is the daemon's, and it is the thing that
            // actually holds the deleted ticket — an empty band means the
            // window closed, whatever we last remembered.
            _ => (!self.grace.is_empty()).then_some(LastUndo::Delete),
        }
    }

    /// The ticket a verb acts on: the shown ticket on the ticket and diff
    /// screens, the cursor card on the board.
    fn subject(&self) -> Option<ulid::Ulid> {
        match &self.screen {
            Screen::Ticket { ticket, .. } | Screen::Diff { ticket } => Some(*ticket),
            Screen::Board => self.selected_ticket().map(|t| t.id),
        }
    }

    fn selected_session(&self) -> Option<uuid::Uuid> {
        match self.rail_row()? {
            RailRow::Session(s) => Some(s.id),
            RailRow::Note(_) => None,
        }
    }

    /// One motion verb, four surfaces. The target is resolved here; the verb
    /// stayed the same in every scope, which is the whole point.
    fn nav(&mut self, verb: Verb, scope: Scope) {
        let down = verb == Verb::CursorDown;
        let up = verb == Verb::CursorUp;
        match scope {
            Scope::Board => match verb {
                Verb::CursorLeft => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                    self.clamp_cursor();
                }
                Verb::CursorRight => {
                    self.cursor_col += 1;
                    self.clamp_cursor();
                }
                Verb::CursorUp => self.cursor_row = self.cursor_row.saturating_sub(1),
                _ => {
                    self.cursor_row += 1;
                    self.clamp_cursor();
                }
            },
            Scope::Ticket => {
                let Screen::Ticket { ticket, rail_idx } = self.screen else {
                    return;
                };
                let n = self.rail_rows(ticket).len();
                let idx = if down {
                    (rail_idx + 1).min(n.saturating_sub(1))
                } else if up {
                    rail_idx.saturating_sub(1)
                } else {
                    rail_idx
                };
                self.screen = Screen::Ticket { ticket, rail_idx: idx };
            }
            Scope::Move => {
                let Mode::Move { ticket, col, idx, grab, home } = self.mode.clone() else {
                    return;
                };
                let cols = self.columns();
                match verb {
                    Verb::CursorLeft | Verb::CursorRight => {
                        let to = if verb == Verb::CursorRight {
                            (col + 1).min(cols.len().saturating_sub(1))
                        } else {
                            col.saturating_sub(1)
                        };
                        // A saturated edge press stays put — no entry, no reset.
                        let idx = if to == col {
                            idx
                        } else {
                            self.ghost_entry_idx(&cols, to, home, ticket)
                        };
                        self.mode = Mode::Move { ticket, col: to, idx, grab, home };
                    }
                    Verb::CursorUp => {
                        self.mode =
                            Mode::Move { ticket, col, idx: idx.saturating_sub(1), grab, home };
                    }
                    _ => {
                        let n = self.ghost_len(&cols, col, ticket);
                        self.mode = Mode::Move { ticket, col, idx: (idx + 1).min(n), grab, home };
                    }
                }
            }
            Scope::Menu => {
                let Mode::Menu { idx } = self.mode else {
                    return;
                };
                let n = keymap::menu_items(&self.ctx()).len();
                let idx =
                    if down { (idx + 1).min(n.saturating_sub(1)) } else { idx.saturating_sub(1) };
                self.mode = Mode::Menu { idx };
            }
            Scope::Drawer => {
                let Mode::External { idx } = self.mode else {
                    return;
                };
                let n = self.external.len();
                let idx =
                    if down { (idx + 1).min(n.saturating_sub(1)) } else { idx.saturating_sub(1) };
                self.mode = Mode::External { idx };
            }
            Scope::Archived => {
                let Mode::Archived { idx } = self.mode else {
                    return;
                };
                let n = self.board.archived_tickets().len();
                let idx =
                    if down { (idx + 1).min(n.saturating_sub(1)) } else { idx.saturating_sub(1) };
                self.mode = Mode::Archived { idx };
            }
            Scope::Theme => {
                let Mode::Theme { idx } = self.mode else {
                    return;
                };
                let n = Flavor::ALL.len();
                let idx = if down { (idx + 1).min(n - 1) } else { idx.saturating_sub(1) };
                self.mode = Mode::Theme { idx };
                // The cursor is the preview.
                self.preview(Flavor::ALL[idx]);
            }
            _ => {}
        }
    }

    /// Enter: act on the selection, whatever the selection is here.
    fn act(&mut self, scope: Scope) -> Result<()> {
        match scope {
            Scope::Board => self.board_enter(),
            Scope::Ticket => {
                if let Screen::Ticket { ticket, .. } = self.screen {
                    if let Some(note) = self.selected_note() {
                        return self.open_note_editor(ticket, Some(note));
                    }
                }
                if let Some(sid) = self.selected_session() {
                    self.focus_session(sid)?;
                }
                Ok(())
            }
            Scope::Menu => {
                let Mode::Menu { idx } = self.mode else {
                    return Ok(());
                };
                let ctx = self.ctx();
                let items = keymap::menu_items(&ctx);
                let Some(item) = items.get(idx.min(items.len().saturating_sub(1))) else {
                    return Ok(());
                };
                let verb = item.verb;
                self.mode = Mode::Normal;
                let ctx = self.ctx();
                self.dispatch(verb, Key::Enter, Scope::Board, &ctx)
            }
            Scope::Drawer => self.adopt_external(true),
            Scope::Archived => {
                let Mode::Archived { idx } = self.mode else {
                    return Ok(());
                };
                let list: Vec<ulid::Ulid> =
                    self.board.archived_tickets().iter().map(|t| t.id).collect();
                if let Some(id) = list.get(idx.min(list.len().saturating_sub(1))).copied() {
                    self.mode = Mode::Normal;
                    self.screen = Screen::Ticket { ticket: id, rail_idx: 0 };
                }
                Ok(())
            }
            Scope::Theme => {
                if let Mode::Theme { idx } = self.mode {
                    self.commit_theme(Flavor::ALL[idx.min(Flavor::ALL.len() - 1)]);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// `q` / `esc`: pop exactly one level.
    fn back(&mut self, scope: Scope) {
        match scope {
            Scope::Ticket => self.to_board(),
            Scope::Diff => {
                let rail_idx = self.diff.as_ref().map(|d| d.rail_idx).unwrap_or(0);
                let ticket = match self.screen {
                    Screen::Diff { ticket } => ticket,
                    _ => return,
                };
                self.diff = None;
                self.screen = Screen::Ticket { ticket, rail_idx };
            }
            Scope::Theme => {
                // Put it back: whatever was previewed, the board returns to
                // the theme it rests on. No entry flavor is stored, which is
                // also what makes a ground flip under the picker right.
                self.mode = Mode::Normal;
                self.preview(self.resting_flavor());
            }
            _ => self.mode = Mode::Normal,
        }
    }

    /// Board Enter is "get me working": a live agent focuses directly, a
    /// just-composed ticket starts one, and only then does Enter mean the
    /// ticket page. The hint says which BEFORE the press (`ticket_hot`).
    fn board_enter(&mut self) -> Result<()> {
        let Some(t) = self.selected_ticket() else {
            return Ok(());
        };
        let ticket = t.id;
        let fresh = self.just_created.take() == Some(ticket);
        let hot = self.rail_sessions(ticket).iter().position(|s| {
            s.kind == SessionKind::Claude
                && matches!(s.state, SessionState::Running | SessionState::RequiresAction { .. })
        });
        if let Some(rail_idx) = hot {
            let sid = self.rail_sessions(ticket)[rail_idx].id;
            self.focus_session(sid)?;
            if self.pending_attach.is_none() {
                // Focus refused: fall open to the ticket page, where the
                // status explains itself.
                self.screen = Screen::Ticket { ticket, rail_idx };
            }
        } else if fresh {
            self.spawn_and_focus(ticket, SessionKind::Claude)?;
        } else {
            self.screen = Screen::Ticket { ticket, rail_idx: 0 };
        }
        Ok(())
    }

    /// `x`: sleep or wake. On the board the target is every session of the
    /// ticket; on the ticket page it is the selected one. One verb, one key,
    /// the target resolved by where you are.
    fn sleep_verb(&mut self, scope: Scope, ctx: &Ctx) -> Result<()> {
        if scope == Scope::Ticket {
            let Some(sid) = self.selected_session() else {
                return Ok(());
            };
            // A corpse cannot be slept, so `x` there is the rail's dismissal
            // gesture instead — the record stays on the board and re-imports
            // through the drawer; only the rail stops showing it. Before
            // this, the press reached `SleepSession` and came back "only idle
            // sessions sleep", which is true and useless.
            if ctx.sel_dead {
                let resp = self.req(Command::KillSession { id: sid });
                self.status = match resp {
                    // NOT "still in the drawer": the drawer is a transcript
                    // census, so a dismissed record only reappears there if
                    // it has one — which the no-transcript corpse, now the
                    // commonest kind reaching this key, does not. What IS
                    // unconditionally true is that dismissing touches the
                    // record and never Claude's own store.
                    Response::Ok => "dismissed ∙ the conversation is untouched".into(),
                    Response::Err { message } => message,
                    _ => String::new(),
                };
                return self.refresh();
            }
            let cmd = if ctx.sel_sleeping {
                Command::WakeSession { id: sid }
            } else {
                Command::SleepSession { id: sid }
            };
            return self.send(cmd);
        }
        let Some(id) = self.subject() else {
            return Ok(());
        };
        let wake = !ctx.ticket_awake;
        let ids: Vec<uuid::Uuid> = self
            .rail_sessions(id)
            .iter()
            .filter(|s| matches!(s.state, SessionState::Sleeping) == wake)
            .map(|s| s.id)
            .collect();
        if ids.is_empty() {
            self.status = if wake { "nothing to wake".into() } else { "already asleep".into() };
            return Ok(());
        }
        let n = ids.len();
        for sid in ids {
            let cmd = if wake {
                Command::WakeSession { id: sid }
            } else {
                Command::SleepSession { id: sid }
            };
            if let Response::Err { message } = self.req(cmd) {
                self.status = message;
                return self.refresh();
            }
        }
        self.status = if wake { format!("woke {n}") } else { format!("slept {n}") };
        self.refresh()
    }

    /// `>` / `<`: grab the card and shift its ghost one column that way, or —
    /// already holding one — shift it again. The same key commits where the
    /// ghost stands; the opposite key cancels the whole move.
    fn grab(&mut self, key: Key, scope: Scope, _ctx: &Ctx) -> Result<()> {
        let Key::Char(c) = key else { return Ok(()) };
        let cols = self.columns();
        if cols.is_empty() {
            return Ok(());
        }
        if scope != Scope::Move {
            let Some(t) = self.selected_ticket() else {
                return Ok(());
            };
            let id = t.id;
            let col = if c == '>' {
                (self.cursor_col + 1) % cols.len()
            } else {
                (self.cursor_col + cols.len() - 1) % cols.len()
            };
            let home = (self.cursor_col, self.cursor_row);
            let idx = self.ghost_entry_idx(&cols, col, home, id);
            self.mode = Mode::Move { ticket: id, col, idx, grab: c, home };
            return Ok(());
        }
        let Mode::Move { ticket, col, idx, grab, .. } = self.mode.clone() else {
            return Ok(());
        };
        if c == grab {
            // The grab key again: commit where the ghost stands, so `>>` is
            // one column in one gesture.
            return self.drop_ghost(&cols, ticket, col, idx);
        }
        // The opposite key cancels the whole move.
        self.mode = Mode::Normal;
        Ok(())
    }

    /// Is there anywhere for `alt+<direction>` to send the selected card —
    /// another column, or another row in this one? One column holding one
    /// card is the board where the gesture means nothing, and the hint goes
    /// down with the key.
    fn can_nudge(&self) -> bool {
        let Some(col) = self.selected_ticket().map(|t| t.column.clone()) else {
            return false;
        };
        self.columns().len() > 1 || self.board.column_tickets(&col).len() > 1
    }

    /// `alt+<direction>`: the move `> <` makes, without the ghost. One press
    /// carries the card one column over or one row along and the cursor rides
    /// with it — the card is the thing being aimed, so the eye should not
    /// have to go back for it. Sideways it enters the foreign column at the
    /// top, which is exactly where a grabbed ghost enters one; an edge press
    /// stays put rather than wrapping, because a wrap is a fine thing to do
    /// to a ghost you can still cancel and a poor thing to do to a card that
    /// has already moved.
    fn nudge(&mut self, key: Key) -> Result<()> {
        let cols = self.columns();
        let Some(id) = self.selected_ticket().map(|t| t.id) else {
            return Ok(());
        };
        if cols.is_empty() {
            return Ok(());
        }
        let (col, idx) = match key {
            Key::AltLeft | Key::AltRight => {
                let to = if key == Key::AltRight {
                    (self.cursor_col + 1).min(cols.len() - 1)
                } else {
                    self.cursor_col.saturating_sub(1)
                };
                if to == self.cursor_col {
                    return Ok(());
                }
                (to, 0)
            }
            Key::AltUp | Key::AltDown => {
                // `ghost_len` counts the column without this card in it, which
                // is the same count an insertion index is measured against.
                let n = self.ghost_len(&cols, self.cursor_col, id);
                let to = if key == Key::AltDown {
                    self.cursor_row + 1
                } else {
                    self.cursor_row.saturating_sub(1)
                };
                if to == self.cursor_row || to > n {
                    return Ok(());
                }
                (self.cursor_col, to)
            }
            _ => return Ok(()),
        };
        self.drop_ghost(&cols, id, col, idx)
    }

    /// `!` in the diff viewer: a shell in the worktree, at its root.
    fn worktree_shell(&mut self) {
        let Screen::Diff { ticket } = self.screen else {
            return;
        };
        let present = self.diff.as_ref().is_some_and(|d| d.worktree_present);
        let path = self.wt_item(ticket).and_then(|w| w.path.clone());
        match (present, path) {
            (true, Some(p)) => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                // Plain argv, no sh -c. Gate/focus hints stay None so
                // after_handover leaves the diff screen alone.
                self.pending_attach = Some(vec![shell]);
                self.pending_attach_cwd = Some(PathBuf::from(p));
            }
            _ => self.status = "the worktree is gone — no directory to open".into(),
        }
    }

    fn diff_scroll(&mut self, delta: isize) {
        let Some(d) = self.diff.as_ref() else { return };
        let now = d.scroll.get() as isize;
        d.scroll.set(now.saturating_add(delta).max(0) as usize);
    }

    /// `{ }` on the ticket page: move the preview zone one page, by what the
    /// last draw measured. Clamped here AND at draw, so a press past the end
    /// sits on the last full window rather than a blank one; a shell tail
    /// scrolled back to its bottom is released to follow the pane again.
    fn preview_page(&mut self, dir: isize) {
        let v = self.preview_view.get();
        let Some(key) = v.key else { return };
        let next = (v.offset as isize + dir * v.page as isize).clamp(0, v.max as isize) as usize;
        if v.follows_tail && next >= v.max {
            self.preview_scroll.set(None);
        } else {
            self.preview_scroll.set(Some((key, next)));
        }
        // The next press may land before the next frame (a held key queues
        // several), so the measurement moves with the request instead of
        // waiting for the draw to say so.
        self.preview_view.set(PreviewView { offset: next, ..v });
    }

    /// The external drawer's two verbs. `resume` adopts and takes the
    /// conversation over here; otherwise it is imported observe-only.
    fn adopt_external(&mut self, resume: bool) -> Result<()> {
        let Mode::External { idx } = self.mode else {
            return Ok(());
        };
        if self.external.is_empty() {
            self.mode = Mode::Normal;
            return Ok(());
        }
        let idx = idx.min(self.external.len() - 1);
        let claude_session_id = self.external[idx].claude_session_id;
        if !resume {
            match self.req(Command::AttachExternal { claude_session_id, ticket: None }) {
                Response::Spawned { id, .. } => {
                    self.refresh()?;
                    self.status = self
                        .board
                        .sessions
                        .iter()
                        .find(|s| s.id == id)
                        .and_then(|s| self.board.ticket(s.ticket))
                        .map(|t| format!("imported \"{}\" — watching only", t.title))
                        .unwrap_or_else(|| "imported — watching only".into());
                    self.mode = Mode::Normal;
                    return Ok(());
                }
                Response::Err { message } => self.status = message,
                _ => {}
            }
            return self.refresh();
        }
        let confirm = self.resume_refused == Some(claude_session_id);
        match self.req(Command::ResumeExternal { claude_session_id, ticket: None, confirm }) {
            Response::Spawned { .. } => {
                self.resume_refused = None;
                self.status = "resumed here".into();
                self.mode = Mode::Normal;
            }
            Response::Err { message } => {
                if message.contains("running elsewhere") {
                    self.resume_refused = Some(claude_session_id);
                }
                self.status = message;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        self.refresh()
    }

    /// The INPUT scope: a barrier that owns every key. The keymap resolves the
    /// editing verbs; anything it does not bind is a character to type.
    fn key_input(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        let Mode::Input { mut purpose, mut buffer } = self.mode.clone() else {
            return Ok(());
        };
        let word = crate::keys::word_wise(mods);
        let key = crate::keys::to_key_text(code, mods);
        let ctx = self.ctx();
        let verb = key.and_then(|k| keymap::resolve(Scope::Input, k, &ctx));
        match verb {
            Some(Verb::Cancel) => {
                self.mode = Mode::Normal;
                return Ok(());
            }
            Some(Verb::Save) => {
                self.mode = Mode::Normal;
                self.commit_input(purpose, buffer.into_text(), false)?;
                return Ok(());
            }
            Some(Verb::SaveStart) => {
                self.mode = Mode::Normal;
                self.commit_input(purpose, buffer.into_text(), true)?;
                return Ok(());
            }
            // The composer grows into the editor: title carried over, cursor
            // in the description, the picks riding along.
            Some(Verb::Describe) => {
                if let InputPurpose::Create { workspace, tags } = purpose {
                    let mut ed = Editor::new(
                        EditorPurpose::Compose { workspace, tags },
                        buffer,
                        TextArea::new(mesimon_core::board::NOTE_MAX_BYTES),
                        Field::Body,
                    );
                    // The panel grows out of the card the last frame drew.
                    ed.grow = self.compose_card.get().map(|r| (r, Instant::now()));
                    self.mode = Mode::Editor(ed);
                } else {
                    self.mode = Mode::Input { purpose, buffer };
                }
                return Ok(());
            }
            Some(Verb::TagPrefix) => {
                // Arm the tail and hand the following keys to it. The mode
                // stays `Input`, so the half-typed title is untouched
                // underneath and Esc comes back to it.
                self.mode = Mode::Input { purpose, buffer };
                self.tag_armed = Some(TagArm {
                    ticket: None,
                    row: 0,
                    col: 0,
                    naming: None,
                    forget_armed: false,
                });
                self.status = "1-9 pick a group ∙ esc done".into();
                return Ok(());
            }
            Some(Verb::CycleWorkspace) => {
                if let InputPurpose::Create { workspace, .. } = &mut purpose {
                    *workspace = match workspace {
                        None => Some(WorkspaceStrategy::Worktree),
                        Some(_) => None,
                    };
                }
            }
            // The ask history, shell-style. `↑` from the ordinary field keeps
            // the draft and shows the newest ask; each further `↑` goes one
            // older and stops at the oldest. `↓` comes back the same way, and
            // the step past the newest is the draft again, walk over. The
            // keymap only binds these while prompting with history, so the
            // `Prompt` arm is the only one that can be reached.
            Some(Verb::HistoryPrev) => {
                if let InputPurpose::Prompt { walk, .. } = &mut purpose {
                    let newest = self.prompt_history.len().checked_sub(1);
                    let next = match (&*walk, newest) {
                        (None, Some(newest)) => Some((newest, buffer.clone().into_text())),
                        (Some(w), _) if w.idx > 0 => Some((w.idx - 1, w.draft.clone())),
                        _ => None,
                    };
                    if let Some((idx, draft)) = next {
                        buffer = EditBuffer::from_text(
                            self.prompt_history[idx].clone(),
                            mesimon_core::command::PROMPT_MAX_BYTES,
                        );
                        *walk = Some(HistoryWalk { idx, draft });
                    }
                }
            }
            Some(Verb::HistoryNext) => {
                if let InputPurpose::Prompt { walk, .. } = &mut purpose {
                    if let Some(w) = walk.take() {
                        match self.prompt_history.get(w.idx + 1) {
                            Some(newer) => {
                                buffer = EditBuffer::from_text(
                                    newer.clone(),
                                    mesimon_core::command::PROMPT_MAX_BYTES,
                                );
                                *walk = Some(HistoryWalk { idx: w.idx + 1, draft: w.draft });
                            }
                            None => {
                                buffer = EditBuffer::from_text(
                                    w.draft,
                                    mesimon_core::command::PROMPT_MAX_BYTES,
                                )
                            }
                        }
                    }
                }
            }
            // Backspace and the arrows widen to a word under ctrl/alt; which
            // modifier that is depends on the terminal, so both count.
            Some(Verb::EditBackspace) if word => buffer.delete_word_back(),
            Some(Verb::EditBackspace) => buffer.backspace(),
            Some(Verb::EditDeleteWord) => buffer.delete_word_back(),
            Some(Verb::EditKillToStart) => buffer.kill_to_start(),
            Some(Verb::EditDelete) => buffer.delete(),
            Some(Verb::EditLeft) if word => buffer.word_left(),
            Some(Verb::EditLeft) => buffer.left(),
            Some(Verb::EditRight) if word => buffer.word_right(),
            Some(Verb::EditRight) => buffer.right(),
            Some(Verb::EditHome) => buffer.home(),
            Some(Verb::EditEnd) => buffer.end(),
            _ => match code {
                // An unhandled chord must never type its letter.
                KeyCode::Char(_) if word => {}
                KeyCode::Char(c) => buffer.insert(c),
                _ => {}
            },
        }
        self.mode = Mode::Input { purpose, buffer };
        Ok(())
    }

    /// The editor. A barrier like `key_input`; the mode is TAKEN rather than
    /// cloned, because a 32 KiB body copied per keystroke is silly.
    fn key_editor(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        let Mode::Editor(ed) = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return Ok(());
        };
        let word = crate::keys::word_wise(mods);
        let key = crate::keys::to_key_text(code, mods);
        // The ctx reads the editor off `self.mode`; put it back for the
        // duration of the lookup.
        self.mode = Mode::Editor(ed);
        let ctx = self.ctx();
        let verb = key.and_then(|k| keymap::resolve(Scope::Editor, k, &ctx));
        let Mode::Editor(mut ed) = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return Ok(());
        };
        // The two armed second-presses survive only their own key.
        if verb != Some(Verb::Cancel) {
            ed.esc_armed = false;
        }
        if verb != Some(Verb::EditorSave) {
            ed.delete_armed = false;
        }
        match verb {
            Some(Verb::Cancel) => return self.editor_cancel(ed),
            Some(Verb::EditorSave) => return self.editor_save(ed, false),
            Some(Verb::SaveStart) => return self.editor_save(ed, true),
            Some(Verb::TagPrefix) => {
                self.mode = Mode::Editor(ed);
                self.tag_armed = Some(TagArm {
                    ticket: None,
                    row: 0,
                    col: 0,
                    naming: None,
                    forget_armed: false,
                });
                self.status = "1-9 pick a group ∙ esc done".into();
                return Ok(());
            }
            Some(Verb::CycleWorkspace) => {
                if let EditorPurpose::Compose { workspace, .. } = &mut ed.purpose {
                    *workspace = match workspace {
                        None => Some(WorkspaceStrategy::Worktree),
                        Some(_) => None,
                    };
                }
            }
            Some(Verb::EditorNewline) => match ed.focus {
                Field::Title => ed.focus = Field::Body,
                Field::Body => ed.body.newline(),
            },
            Some(Verb::EditorUp) => match ed.focus {
                // Off the top of the body is the title — when it is ours to
                // edit. A note's title belongs to the ticket.
                Field::Body if ed.body.cursor_line() == 0 && ed.composing() => {
                    ed.focus = Field::Title
                }
                Field::Body => ed.body.up(),
                Field::Title => {}
            },
            Some(Verb::EditorDown) => match ed.focus {
                Field::Title => ed.focus = Field::Body,
                Field::Body => ed.body.down(),
            },
            Some(Verb::PageUp) => ed.body.page(-(EDITOR_PAGE as isize)),
            Some(Verb::PageDown) => ed.body.page(EDITOR_PAGE as isize),
            Some(Verb::EditBackspace) if word => match ed.focus {
                Field::Title => ed.title.delete_word_back(),
                Field::Body => ed.body.delete_word_back(),
            },
            Some(Verb::EditBackspace) => match ed.focus {
                Field::Title => ed.title.backspace(),
                Field::Body => ed.body.backspace(),
            },
            Some(Verb::EditDeleteWord) => match ed.focus {
                Field::Title => ed.title.delete_word_back(),
                Field::Body => ed.body.delete_word_back(),
            },
            Some(Verb::EditKillToStart) => match ed.focus {
                Field::Title => ed.title.kill_to_start(),
                Field::Body => ed.body.kill_to_start(),
            },
            Some(Verb::EditDelete) => match ed.focus {
                Field::Title => ed.title.delete(),
                Field::Body => ed.body.delete(),
            },
            Some(Verb::EditLeft) => match (ed.focus, word) {
                (Field::Title, true) => ed.title.word_left(),
                (Field::Title, false) => ed.title.left(),
                (Field::Body, true) => ed.body.word_left(),
                (Field::Body, false) => ed.body.left(),
            },
            Some(Verb::EditRight) => match (ed.focus, word) {
                (Field::Title, true) => ed.title.word_right(),
                (Field::Title, false) => ed.title.right(),
                (Field::Body, true) => ed.body.word_right(),
                (Field::Body, false) => ed.body.right(),
            },
            Some(Verb::EditHome) => match ed.focus {
                Field::Title => ed.title.home(),
                Field::Body => ed.body.home(),
            },
            Some(Verb::EditEnd) => match ed.focus {
                Field::Title => ed.title.end(),
                Field::Body => ed.body.end(),
            },
            _ => match code {
                // An unhandled chord must never type its letter.
                KeyCode::Char(_) if word => {}
                // A note's title is the ticket's, not the editor's to type in.
                KeyCode::Char(c) if ed.focus == Field::Title && ed.composing() => {
                    ed.title.insert(c)
                }
                KeyCode::Char(c) if ed.focus == Field::Body => ed.body.insert(c),
                _ => {}
            },
        }
        self.mode = Mode::Editor(ed);
        Ok(())
    }

    /// Esc: two presses when there is something to lose. A clean composer
    /// goes back to the one-line field it grew out of, title and all —
    /// symmetric with Tab.
    fn editor_cancel(&mut self, mut ed: Editor) -> Result<()> {
        if ed.dirty() && !ed.esc_armed {
            ed.esc_armed = true;
            self.status = "unsaved ∙ esc again discards".into();
            self.mode = Mode::Editor(ed);
            return Ok(());
        }
        if !ed.dirty() {
            if let EditorPurpose::Compose { workspace, tags } = ed.purpose {
                self.mode = Mode::Input {
                    purpose: InputPurpose::Create { workspace, tags },
                    buffer: ed.title,
                };
                return Ok(());
            }
        }
        self.mode = Mode::Normal;
        Ok(())
    }

    /// `^s` (and, composing, Shift+Enter with `start`). Composing mints the
    /// ticket and closes. A note is written and the editor STAYS — it is a
    /// document being kept, not a field being submitted — and the same key
    /// on a saved note tells the ticket's claude to go read it.
    fn editor_save(&mut self, mut ed: Editor, start: bool) -> Result<()> {
        match ed.purpose.clone() {
            EditorPurpose::Compose { workspace, tags } => {
                let title = ed.title.as_str().trim().to_string();
                if title.is_empty() {
                    self.status = "a ticket needs a title".into();
                    self.mode = Mode::Editor(ed);
                    return Ok(());
                }
                let body = if ed.body.as_str().trim().is_empty() {
                    None
                } else {
                    Some(ed.body.as_str().to_string())
                };
                self.mode = Mode::Normal;
                self.mint_ticket(title, workspace, tags, body, start)
            }
            EditorPurpose::Note { ticket, note } => {
                let body = ed.body.as_str().to_string();
                let blank = body.trim().is_empty();
                if !ed.dirty() {
                    // Nothing to save: the second press tells claude, if
                    // there is one to tell.
                    if let Some(note) = note {
                        if self.board.pane_target(ticket).is_some() {
                            self.status = match self.req(Command::NoteToAgent { ticket, note }) {
                                Response::Ok => "asked".into(),
                                Response::Err { message } => message,
                                _ => String::new(),
                            };
                        }
                    }
                    self.mode = Mode::Editor(ed);
                    return Ok(());
                }
                if blank {
                    let Some(note) = note else {
                        self.status = "nothing to save".into();
                        self.mode = Mode::Editor(ed);
                        return Ok(());
                    };
                    if !ed.delete_armed {
                        ed.delete_armed = true;
                        self.status = "empty ∙ ^s again deletes the note".into();
                        self.mode = Mode::Editor(ed);
                        return Ok(());
                    }
                    match self.req(Command::WriteNote {
                        ticket,
                        note: Some(note),
                        text: String::new(),
                    }) {
                        Response::NoteWritten { .. } => {
                            self.notes.remove(&note);
                            self.status = "note deleted".into();
                            self.mode = Mode::Normal;
                        }
                        Response::Err { message } => {
                            self.status = message;
                            self.mode = Mode::Editor(ed);
                        }
                        _ => self.mode = Mode::Editor(ed),
                    }
                    return self.refresh();
                }
                let created = note.is_none();
                match self.req(Command::WriteNote { ticket, note, text: body.clone() }) {
                    Response::NoteWritten { note: Some(id) } => {
                        ed.purpose = EditorPurpose::Note { ticket, note: Some(id) };
                        ed.saved();
                        self.refresh()?;
                        // A fresh note that landed first IS the description
                        // now, and the status says so once.
                        let (rev, first) = self
                            .board
                            .ticket(ticket)
                            .and_then(|t| {
                                t.note(id).map(|n| (n.rev, created && t.notes[0].id == id))
                            })
                            .unwrap_or((0, false));
                        self.remember_note(id, rev, Some(crate::peek::sanitize(&body)));
                        let what = if first { "saved as the description" } else { "saved" };
                        self.status = if self.board.pane_target(ticket).is_some() {
                            format!("{what} ∙ ^s again tells claude")
                        } else {
                            what.into()
                        };
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.mode = Mode::Editor(ed);
                Ok(())
            }
        }
    }

    /// Open the editor on a ticket's note — an existing one re-read from the
    /// daemon first (never the cache: an edit must start from the truth), or
    /// a fresh one that the first save mints.
    fn open_note_editor(&mut self, ticket: ulid::Ulid, note: Option<ulid::Ulid>) -> Result<()> {
        let Some(t) = self.board.ticket(ticket) else {
            return Ok(());
        };
        let title = EditBuffer::from_text(t.title.clone(), mesimon_core::board::TITLE_MAX_BYTES);
        let body = match note {
            Some(id) => match self.req(Command::ReadNote { ticket, note: id }) {
                Response::Note { text, .. } => {
                    TextArea::from_text(&text, mesimon_core::board::NOTE_MAX_BYTES)
                }
                Response::Err { message } => {
                    self.status = message;
                    return Ok(());
                }
                _ => return Ok(()),
            },
            None => TextArea::new(mesimon_core::board::NOTE_MAX_BYTES),
        };
        self.mode = Mode::Editor(Editor::new(
            EditorPurpose::Note { ticket, note },
            title,
            body,
            Field::Body,
        ));
        Ok(())
    }

    /// The picker. Owns every key while it is open, exactly as the input
    /// barrier does — including the digits and `hjkl`, which is why the whole
    /// table stands down while a name is being typed.
    fn key_tag(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        let Some(mut arm) = self.tag_armed.take() else { return Ok(()) };
        let naming = arm.naming.is_some();
        // Alt is the "by word" modifier inside the name field and an atom of
        // its own everywhere else — the grid nudges with `alt+hjkl`, and a
        // text field never sees an Alt atom. The picker IS a text field only
        // while `naming`, so that is exactly where the reading switches.
        let key = if naming {
            crate::keys::to_key_text(code, mods)
        } else {
            crate::keys::to_key(code, mods)
        };
        let Some(key) = key else {
            self.tag_armed = Some(arm);
            return Ok(());
        };
        let word = crate::keys::word_wise(mods);

        // Editing keys inside the name field. Resolved against the TAG scope,
        // NOT `Scope::Input`: borrowing that scope is what used to put the
        // composer's own hints ("shift+enter save + ask claude", "shift+tab
        // workspace") under a field that does none of those things.
        if let Some((_, buf)) = arm.naming.as_mut() {
            match code {
                KeyCode::Backspace if word => buf.delete_word_back(),
                KeyCode::Backspace => buf.backspace(),
                KeyCode::Delete => buf.delete(),
                KeyCode::Left if word => buf.word_left(),
                KeyCode::Left => buf.left(),
                KeyCode::Right if word => buf.word_right(),
                KeyCode::Right => buf.right(),
                KeyCode::Home => buf.home(),
                KeyCode::End => buf.end(),
                KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
                    buf.delete_word_back()
                }
                KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => buf.kill_to_start(),
                KeyCode::Char(_) if word => {}
                KeyCode::Char(c) => buf.insert(c),
                _ => {}
            }
        }

        // Any key but `d` cancels a half-pressed delete — the grace band is
        // one keypress wide, exactly like the delete and archive chords.
        if !matches!(key, Key::Char('d')) {
            arm.forget_armed = false;
        }
        self.tag_armed = Some(arm);

        // While naming, only Enter and Esc still resolve; everything else the
        // field already consumed above.
        let ctx = self.ctx();
        let Some(verb) = keymap::resolve(Scope::TagChord, key, &ctx) else {
            // A stray key inside the picker closes it rather than acting on
            // the board behind it. A nudge is never a stray key, though —
            // neither one that had nowhere to go, nor the composed character
            // a terminal that ate the modifier sends instead (macOS Terminal
            // turns `alt+h` into `˙`). Either way the accelerator has to be
            // inert, not a panel that vanishes on one terminal and not the
            // other.
            let nudge = matches!(key, Key::AltLeft | Key::AltRight | Key::AltUp | Key::AltDown)
                || matches!(key, Key::Char(c) if !c.is_ascii());
            if !naming && !nudge {
                self.tag_armed = None;
                self.status.clear();
            }
            return Ok(());
        };
        self.dispatch(verb, key, Scope::TagChord, &ctx)
    }

    /// The tags the picker is acting on. While composing the ticket does not
    /// exist yet, so the picks live on `InputPurpose::Create` until save.
    pub(crate) fn tag_subject(&self) -> Option<&[TagRef]> {
        let arm = self.tag_armed.as_ref()?;
        match arm.ticket {
            Some(id) => self.board.ticket(id).map(|t| t.tags.as_slice()),
            None => self.compose_tags().map(Vec::as_slice),
        }
    }

    /// The composer's buffered picks, whichever composer is open — the
    /// one-line field or the editor it grew into.
    fn compose_tags(&self) -> Option<&Vec<TagRef>> {
        match &self.mode {
            Mode::Input { purpose: InputPurpose::Create { tags, .. }, .. } => Some(tags),
            Mode::Editor(Editor { purpose: EditorPurpose::Compose { tags, .. }, .. }) => Some(tags),
            _ => None,
        }
    }

    fn compose_tags_mut(&mut self) -> Option<&mut Vec<TagRef>> {
        match &mut self.mode {
            Mode::Input { purpose: InputPurpose::Create { tags, .. }, .. } => Some(tags),
            Mode::Editor(Editor { purpose: EditorPurpose::Compose { tags, .. }, .. }) => Some(tags),
            _ => None,
        }
    }

    /// The registry entry under the picker cursor: `(group, name, tint)`.
    /// `None` means the cursor is on the `+ new` cell.
    pub(crate) fn tag_cell(&self) -> Option<(u8, String, u8)> {
        let arm = self.tag_armed.as_ref()?;
        let group = *crate::ui::tag_rows(self).get(arm.row)?;
        let def = self.board.group_entries(group).get(arm.col).copied()?;
        Some((group, def.name.clone(), def.tint()))
    }

    /// Keep the cursor on a cell that exists. Called after anything that can
    /// shrink the grid under it — a delete, a group change, a refresh.
    fn tag_clamp(&mut self) {
        let rows = crate::ui::tag_rows(self);
        let (row, len) = {
            let Some(arm) = self.tag_armed.as_ref() else { return };
            let row = arm.row.min(rows.len().saturating_sub(1));
            (row, rows.get(row).map(|g| crate::ui::tag_row_len(self, *g)).unwrap_or(1))
        };
        if let Some(arm) = self.tag_armed.as_mut() {
            arm.row = row;
            arm.col = arm.col.min(len.saturating_sub(1));
        }
    }

    fn tag_move(&mut self, verb: Verb) {
        let rows = crate::ui::tag_rows(self);
        if rows.is_empty() {
            return;
        }
        let Some(arm) = self.tag_armed.as_ref() else { return };
        let (mut row, mut col) = (arm.row, arm.col);
        match verb {
            Verb::TagUp => row = row.saturating_sub(1),
            Verb::TagDown => row = (row + 1).min(rows.len() - 1),
            Verb::TagLeft => col = col.saturating_sub(1),
            _ => col += 1,
        }
        if let Some(arm) = self.tag_armed.as_mut() {
            arm.row = row;
            arm.col = col;
            // A vertical move keeps the column only as far as the new row
            // reaches; `tag_clamp` does the trimming.
            arm.forget_armed = false;
        }
        self.tag_clamp();
    }

    /// Take the tag under the cursor with you — the picker's grid nudged the
    /// way the board's is. Along the row it is order, which is what the row
    /// draws and what a repeated digit walks. Across rows it is the axis
    /// itself, and the wearers travel too; `board::move_tag` refuses rather
    /// than resolving when the move would leave a ticket wearing two tags on
    /// one axis, so the status line says what is in the way and nothing
    /// changes.
    fn tag_carry(&mut self, verb: Verb) -> Result<()> {
        let Some((group, name, _)) = self.tag_cell() else { return Ok(()) };
        let rows = crate::ui::tag_rows(self);
        let Some((row, col)) = self.tag_armed.as_ref().map(|a| (a.row, a.col)) else {
            return Ok(());
        };
        let (to_group, to_index) = match verb {
            Verb::TagCarryLeft if col > 0 => (group, col - 1),
            Verb::TagCarryRight if col + 1 < self.board.group_entries(group).len() => {
                (group, col + 1)
            }
            Verb::TagCarryUp | Verb::TagCarryDown => {
                let to = if verb == Verb::TagCarryUp { row.checked_sub(1) } else { Some(row + 1) };
                let Some(g) = to.and_then(|r| rows.get(r).copied()) else { return Ok(()) };
                // A tag arriving on an axis JOINS it, at the end: pushing
                // into the middle would reorder a row the eye already knows
                // to make room for one nobody aimed at a slot.
                (g, self.board.group_entries(g).len())
            }
            // The ends of the grid. Nowhere to go, and nothing to say.
            _ => return Ok(()),
        };
        // The composer's picks are on no ticket yet, so the daemon's refusal
        // cannot see them. Mirror it here, and carry them on the way back.
        let composing_wearer = self
            .compose_tags()
            .is_some_and(|tags| tags.iter().any(|t| t.group == group && t.name == name));
        if to_group != group && composing_wearer {
            if let Some(tags) = self.compose_tags() {
                if tags.iter().any(|t| t.group == to_group) {
                    self.status = format!("this ticket already wears a tag on axis {to_group}");
                    return Ok(());
                }
            }
        }
        if let Response::Err { message } =
            self.req(Command::MoveTag { group, name: name.clone(), to_group, to_index })
        {
            self.status = message;
            return Ok(());
        }
        if to_group != group && composing_wearer {
            if let Some(tags) = self.compose_tags_mut() {
                for t in tags.iter_mut() {
                    if t.group == group && t.name == name {
                        t.group = to_group;
                    }
                }
                tags.sort_by_key(|t| t.group);
            }
        }
        self.status.clear();
        self.refresh()?;
        // Ride with the tag: where it landed, not where the cursor was.
        let rows = crate::ui::tag_rows(self);
        let landed = (
            rows.iter().position(|g| *g == to_group),
            self.board.group_entries(to_group).iter().position(|d| d.name == name),
        );
        if let Some(arm) = self.tag_armed.as_mut() {
            if let (Some(r), Some(c)) = landed {
                arm.row = r;
                arm.col = c;
            }
            arm.forget_armed = false;
        }
        self.tag_clamp();
        Ok(())
    }

    /// Apply one axis change, wherever the subject lives.
    fn apply_tag(&mut self, group: u8, name: Option<String>) -> Result<()> {
        let ticket = self.tag_armed.as_ref().and_then(|a| a.ticket);
        match ticket {
            Some(id) => {
                if let Response::Err { message } = self.req(Command::SetTag { id, group, name }) {
                    self.status = message;
                }
            }
            None => {
                if let Some(tags) = self.compose_tags_mut() {
                    tags.retain(|t| t.group != group);
                    if let Some(name) = name {
                        tags.push(TagRef { name, group });
                    }
                    tags.sort_by_key(|t| t.group);
                }
            }
        }
        Ok(())
    }

    /// `1`-`0` without the picker: step the selected ticket one place along
    /// that group's vocabulary, and off the end back to untagged.
    ///
    /// This goes straight to `SetTag` rather than through `apply_tag`, which
    /// reads its subject out of the open picker's arm — there is no arm here,
    /// and the composer (whose tags buffer `apply_tag` also feeds) never
    /// reaches this verb: a text field owns its digits.
    fn cycle_tag(&mut self, group: u8) -> Result<()> {
        let Some(id) = self.subject() else { return Ok(()) };
        let current = self.board.ticket(id).and_then(|t| t.tag_in(group)).map(|t| t.name.clone());
        let (empty, next) = {
            let names = self.board.group_tags(group);
            (names.is_empty(), mesimon_core::board::cycle_tag(&names, current.as_deref()))
        };
        // An axis with no vocabulary is not a broken key, it is an axis
        // nobody has named yet — so say where names come from.
        if empty {
            self.status = format!("no tags in group {group} ∙ ^t makes one");
            return Ok(());
        }
        let word = next.clone().unwrap_or_else(|| "none".into());
        if let Response::Err { message } = self.req(Command::SetTag { id, group, name: next }) {
            self.status = message;
        } else {
            self.status = format!("group {group} ∙ {word}");
            // Show the change where it will be read from now on. The status
            // line says WHICH tag in words; the card is where the stripe and
            // the chip live, and a refusal has nothing to show.
            self.tag_flash = Some((id, Instant::now()));
        }
        self.refresh()
    }

    /// Is the cursor card open for `ticket` — the `p` preference, or a
    /// quick-tag digit still inside its reveal? Only the board asks, and only
    /// of the card under the cursor: the flash is keyed to the ticket, so the
    /// first `j` closes it and no second card ever opens behind it.
    pub(crate) fn peek_showing(&self, ticket: ulid::Ulid) -> bool {
        self.peek || self.tag_flash.is_some_and(|(id, at)| id == ticket && at.elapsed() < TAG_FLASH)
    }

    /// Finish naming: create a new tag, or rename the one under the cursor.
    /// Sanitizing here as well as at the daemon keeps the status line and the
    /// composer buffer agreeing with what actually got stored.
    fn commit_tag_name(&mut self) -> Result<()> {
        let Some(arm) = self.tag_armed.as_ref() else { return Ok(()) };
        let Some((purpose, buf)) = arm.naming.as_ref() else { return Ok(()) };
        let (purpose, raw) = (*purpose, buf.as_str().to_string());
        let rows = crate::ui::tag_rows(self);
        let Some(group) = rows.get(arm.row).copied() else { return Ok(()) };
        let Some(name) = mesimon_core::board::sanitize_tag(&raw) else {
            self.status = "a tag needs a name".into();
            return Ok(());
        };
        let cmd = match purpose {
            Naming::New => Command::RegisterTag { group, name: name.clone() },
            Naming::Rename => match self.tag_cell() {
                Some((_, from, _)) => Command::RenameTag { group, from, to: name.clone() },
                None => return Ok(()),
            },
        };
        match self.req(cmd) {
            Response::Err { message } => {
                // Keep the field open on a refusal — a full group or a
                // duplicate name is worth another try, not a lost keystroke.
                self.status = message;
                return Ok(());
            }
            _ => self.status.clear(),
        }
        if let Some(arm) = self.tag_armed.as_mut() {
            arm.naming = None;
        }
        self.refresh()?;
        // Land the cursor on what was just made, so Enter wears it.
        if purpose == Naming::New {
            let col =
                self.board.group_entries(group).iter().position(|d| d.name == name).unwrap_or(0);
            if let Some(arm) = self.tag_armed.as_mut() {
                arm.col = col;
            }
        }
        self.tag_clamp();
        Ok(())
    }

    /// The ticket's worktree binding, as the last snapshot reported it.
    pub fn wt_item(&self, ticket: ulid::Ulid) -> Option<&WorktreeItem> {
        self.worktrees.iter().find(|w| w.ticket == ticket)
    }

    /// A session on this ticket is mid-turn — the daemon's quiet-tickets
    /// predicate (server.rs merge_ticket), mirrored so the m flow can refuse
    /// before arming rather than after the confirm press.
    pub(crate) fn ticket_busy(&self, ticket: ulid::Ulid) -> bool {
        self.board.sessions.iter().any(|s| {
            s.ticket == ticket
                && matches!(
                    s.state,
                    SessionState::Spawning
                        | SessionState::Running
                        | SessionState::RequiresAction { .. }
                )
        })
    }

    /// Ticket `v` (M4b): enter the read-only diff viewer. Column-agnostic
    /// (D34.7); attached or evicted both work — evicted renders from the
    /// object store.
    fn open_diff(&mut self, ticket: ulid::Ulid, rail_idx: usize) -> Result<()> {
        let viewable = self
            .wt_item(ticket)
            .map(|w| matches!(w.status.as_str(), "attached" | "evicted"))
            .unwrap_or(false);
        if !viewable {
            self.status = "no worktree to diff — review is per-branch".into();
            return Ok(());
        }
        match self.req(Command::DiffList { ticket }) {
            Response::DiffList { branch, base_oid, branch_oid, files, worktree_present } => {
                self.diff = Some(DiffState {
                    ticket,
                    rail_idx,
                    branch,
                    base_oid,
                    branch_oid,
                    files,
                    file_idx: 0,
                    scroll: Cell::new(0),
                    marquee: Cell::new(None),
                    density: 3,
                    cache: std::collections::HashMap::new(),
                    z_armed: false,
                    swap: false,
                    worktree_present,
                });
                self.screen = Screen::Diff { ticket };
                self.diff_fetch(0);
            }
            Response::Err { message } => self.status = message,
            _ => {}
        }
        Ok(())
    }

    /// Re-run DiffList in place (R, and the density cycle's cache flush).
    /// Keeps the cursor on the same path when it survives the recompute.
    fn diff_refresh(&mut self, ticket: ulid::Ulid) {
        let Some(d) = self.diff.as_ref() else { return };
        let keep = d.files.get(d.file_idx).map(|f| f.path.clone());
        match self.req(Command::DiffList { ticket }) {
            Response::DiffList { branch, base_oid, branch_oid, files, worktree_present } => {
                let Some(d) = self.diff.as_mut() else { return };
                d.file_idx = keep.and_then(|p| files.iter().position(|f| f.path == p)).unwrap_or(0);
                d.branch = branch;
                d.base_oid = base_oid;
                d.branch_oid = branch_oid;
                d.files = files;
                d.worktree_present = worktree_present;
                d.cache.clear();
                d.scroll.set(0);
                let idx = d.file_idx;
                self.diff_fetch(idx);
            }
            Response::Err { message } => self.status = message,
            _ => {}
        }
    }

    /// Fetch the cursor file plus one prefetch each side (~10 ms/file [M]),
    /// skipping cached entries and untracked-only rows (nothing to fetch —
    /// git diff cannot see them).
    fn diff_fetch(&mut self, idx: usize) {
        for (i, cursor) in
            [(idx as isize, true), (idx as isize + 1, false), (idx as isize - 1, false)]
        {
            let Some(d) = self.diff.as_ref() else { return };
            if i < 0 {
                continue;
            }
            let Some(f) = d.files.get(i as usize) else {
                continue;
            };
            if f.status.is_empty() || d.cache.contains_key(&f.path) {
                continue;
            }
            let (ticket, path, context) = (d.ticket, f.path.clone(), d.density);
            match self.req(Command::DiffFile { ticket, path: path.clone(), context }) {
                Response::DiffFile { file } => {
                    if let Some(d) = self.diff.as_mut() {
                        d.cache.insert(path, file);
                    }
                }
                Response::Err { message } if cursor => self.status = message,
                _ => {}
            }
        }
    }

    /// Move the diff cursor by whole files (h/l, J/K).
    fn diff_nav(&mut self, delta: isize) {
        let Some(d) = self.diff.as_mut() else { return };
        if d.files.is_empty() {
            return;
        }
        let max = d.files.len() as isize - 1;
        let idx = (d.file_idx as isize + delta).clamp(0, max) as usize;
        if idx == d.file_idx {
            return;
        }
        d.file_idx = idx;
        d.scroll.set(0);
        self.diff_fetch(idx);
    }

    /// Delete with the M4 worktree gate (author rule 2): an unmerged worktree
    /// must be dealt with first — `m` merges, `D` discards worktree + branch.
    fn delete_gated(&mut self, id: ulid::Ulid, discard: bool) -> Result<()> {
        if !discard {
            if let Some(w) = self.wt_item(id) {
                if !w.branch.is_empty() && !w.merged {
                    // Name the two ways out in the keymap's own words, so this
                    // refusal cannot outlive the keys it teaches.
                    self.status = "the branch is not merged ∙ m merges it ∙ d D deletes \
                                   the ticket and discards the branch"
                        .into();
                    return Ok(());
                }
            }
        }
        if matches!(self.screen, Screen::Ticket { .. }) {
            self.to_board();
        }
        self.last_undo = Some(LastUndo::Delete);
        self.send(Command::DeleteTicket { id, discard_worktree: discard })
    }

    /// The m state machine: stage derives from git state; the first press
    /// names what the next press does, the second performs it.
    ///
    ///   ahead + ff-able   m → "merge N? m"        → m → ff merge
    ///   default moved     m → "m asks rebase"      → m → inject rebase+test
    ///   merged            m → "m notifies agent"   → m → inject notice
    fn merge_key(&mut self, ticket: ulid::Ulid) -> Result<()> {
        // Every reply of this flow goes to `merge_note` — the ticket screen's
        // identity line, right where the branch state already announces `m`.
        // The footer never talks about the merge (author 2026-08-30).
        let Some(w) = self.wt_item(ticket) else {
            self.merge_note = "no worktree on this ticket".into();
            return Ok(());
        };
        let (branch, ahead) = (w.branch.clone(), w.ahead);
        let stage = if w.merged {
            MergeStage::Notify
        } else if w.needs_rebase {
            MergeStage::Rebase
        } else if w.ahead > 0 {
            MergeStage::Merge
        } else {
            self.merge_note = "no commits on the branch yet — nothing to merge".into();
            return Ok(());
        };
        // Quiet-tickets rule, surfaced up front: the daemon refuses a merge
        // under a working agent, so the first press says so instead of arming
        // a confirm the second press can only lose.
        if stage == MergeStage::Merge && self.ticket_busy(ticket) {
            self.merge_armed = None;
            self.merge_note = "agent still working — wait for it to finish".into();
            return Ok(());
        }
        if self.merge_armed != Some((ticket, stage)) {
            self.merge_armed = Some((ticket, stage));
            self.merge_note = match stage {
                MergeStage::Merge => format!("merge {ahead} commit(s) of {branch}? m confirms"),
                MergeStage::Rebase => "main moved — m asks the agent to rebase + test".into(),
                MergeStage::Notify => "merged ∙ m tells the agent".into(),
            };
            return Ok(());
        }
        self.merge_armed = None;
        match stage {
            MergeStage::Merge => match self.req(Command::MergeTicket { id: ticket }) {
                Response::Merge { outcome, detail } => {
                    self.merge_note = match outcome {
                        // The note promises the next press notifies, so arm
                        // that stage now — same as the NeedsRebase race below.
                        MergeOutcome::Merged => {
                            self.merge_armed = Some((ticket, MergeStage::Notify));
                            format!("{detail} ∙ m tells the agent")
                        }
                        MergeOutcome::AlreadyMerged => detail,
                        // Raced: main moved between snapshot and keypress.
                        MergeOutcome::NeedsRebase => {
                            self.merge_armed = Some((ticket, MergeStage::Rebase));
                            format!("{detail} ∙ m asks the agent to rebase + test")
                        }
                        MergeOutcome::Refused => detail,
                    };
                }
                Response::Err { message } => self.merge_note = message,
                _ => {}
            },
            MergeStage::Rebase => {
                match self.req(Command::MergeToAgent {
                    id: ticket,
                    request: mesimon_core::command::MergeRequest::Rebase,
                }) {
                    Response::Ok => {
                        self.merge_note = "rebase request sent — m merges once it lands".into()
                    }
                    Response::Err { message } => self.merge_note = message,
                    _ => {}
                }
            }
            MergeStage::Notify => {
                match self.req(Command::MergeToAgent {
                    id: ticket,
                    request: mesimon_core::command::MergeRequest::MergedNotice,
                }) {
                    Response::Ok => self.merge_note = "agent notified".into(),
                    Response::Err { message } => self.merge_note = message,
                    _ => {}
                }
            }
        }
        self.refresh()
    }

    fn focus_kind_or_spawn(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Result<()> {
        // Live sessions only: the rail's resumable corpse is Enter's business —
        // `c` on a ticket whose claude died spawns fresh, as it always has.
        // A Sleeping one is live and lands in `focus_session`, which resumes
        // before it attaches — so `c` on a parked claude wakes it, and the
        // daemon's one-claude gate never has to refuse this key.
        let existing = self
            .rail_sessions(ticket)
            .iter()
            .find(|s| s.kind == kind && s.state.is_live())
            .map(|s| s.id);
        match existing {
            Some(sid) => self.focus_session(sid),
            None => self.spawn_and_focus(ticket, kind),
        }
    }

    /// `e`: rescan (lazy census — this is the only trigger) and open the drawer.
    fn open_drawer(&mut self) -> Result<()> {
        match self.req(Command::RescanExternal) {
            Response::Err { message } => {
                self.status = message;
                return Ok(());
            }
            resp => self.absorb_board(resp),
        }
        if self.external.is_empty() {
            self.status = "no external sessions found for this repo".into();
        } else {
            self.mode = Mode::External { idx: 0 };
        }
        Ok(())
    }

    /// Where the ghost lands when it enters `col`: the top of any foreign
    /// column (height is adjusted by hand afterwards), its own original
    /// position when coming back home while still moving.
    fn ghost_entry_idx(
        &self,
        cols: &[String],
        col: usize,
        home: (usize, usize),
        ticket: ulid::Ulid,
    ) -> usize {
        if col == home.0 {
            home.1.min(self.ghost_len(cols, col, ticket))
        } else {
            0
        }
    }

    /// Archive with the advisory pre-check (the daemon gates again): archive
    /// means everything is already asleep — the same predicate the header
    /// suggestion prices.
    fn archive_gated(&mut self, id: ulid::Ulid) -> Result<()> {
        if self.board.ticket_awake_sessions(id) > 0 {
            let how = keymap::hint_for(Scope::Board, Verb::Sleep, &self.ctx())
                .map(|(show, _)| format!(" ({show})"))
                .unwrap_or_default();
            self.status = format!("its sessions are awake — sleep them first{how}");
            return Ok(());
        }
        let key = self.board.ticket(id).map(|t| t.short_key.clone()).unwrap_or_default();
        match self.req(Command::ArchiveTicket { id }) {
            Response::Err { message } => self.status = message,
            _ => {
                self.last_undo = Some(LastUndo::Archive(id));
                self.status = format!("archived {key} ∙ u undoes it");
            }
        }
        self.refresh()
    }

    fn unarchive(&mut self, id: ulid::Ulid) -> Result<()> {
        if self.last_undo == Some(LastUndo::Archive(id)) {
            self.last_undo = None;
        }
        let col = self.board.ticket(id).map(|t| t.column.clone()).unwrap_or_default();
        match self.req(Command::UnarchiveTicket { id }) {
            Response::Err { message } => self.status = message,
            _ => self.status = format!("restored to {col}"),
        }
        self.refresh()
    }

    /// Commit the MOVE ghost: reinsert `ticket` at (`col`, `idx`) and land the
    /// cursor on it.
    fn drop_ghost(
        &mut self,
        cols: &[String],
        ticket: ulid::Ulid,
        col: usize,
        idx: usize,
    ) -> Result<()> {
        let target_col = cols[col.min(cols.len() - 1)].clone();
        let others: Vec<ulid::Ulid> = self
            .board
            .column_tickets(&target_col)
            .iter()
            .filter(|t| t.id != ticket)
            .map(|t| t.id)
            .collect();
        let before = others.get(idx).copied();
        self.mode = Mode::Normal;
        // A move that stays inside its column is a reorder, not a filing, and
        // `repeat_target` refuses to repeat one anyway (the card is already
        // there). Arming on it would leave `.` aimed at whichever column the
        // cursor happened to be standing in.
        if self.board.ticket(ticket).is_some_and(|t| t.column != target_col) {
            self.last_action = Some(LastAction::Move { column: target_col.clone() });
        }
        self.send(Command::MoveTicket { id: ticket, column: target_col, before })?;
        self.cursor_col = col;
        self.cursor_row = idx;
        Ok(())
    }

    /// Which column `.` would move the selected card into, or `None` when the
    /// key is inert. Re-derived from the live board rather than trusted, the
    /// way `undo_target` is: the column may have been renamed away, and a card
    /// already sitting in the target has nothing to repeat — moving it would
    /// be a shuffle, not the same action again.
    fn repeat_target(&self) -> Option<usize> {
        // No `_` arm: the second repeatable action must be decided here.
        match self.last_action.as_ref() {
            Some(LastAction::Move { column }) => {
                if !matches!(self.screen, Screen::Board) {
                    return None;
                }
                let id = self.selected_ticket().map(|t| t.id)?;
                if self.board.ticket(id)?.column == *column {
                    return None;
                }
                self.columns().iter().position(|c| c == column)
            }
            None => None,
        }
    }

    /// `.` — the last move again, on the card under the cursor, without the
    /// aiming. The cursor deliberately does NOT follow the card the way a drop
    /// makes it: the whole point of the key is that the next card slides up
    /// under the cursor, so `. . .` files three of them without a keystroke
    /// spent travelling back. It lands at the top of the target column, which
    /// is exactly where a fresh grab's ghost enters a foreign column.
    fn repeat_last(&mut self) -> Result<()> {
        let Some(col) = self.repeat_target() else { return Ok(()) };
        let Some(id) = self.selected_ticket().map(|t| t.id) else { return Ok(()) };
        let cols = self.columns();
        let (home_col, home_row) = (self.cursor_col, self.cursor_row);
        self.drop_ghost(&cols, id, col, 0)?;
        self.cursor_col = home_col;
        self.cursor_row = home_row;
        self.clamp_cursor();
        // Only claim it if it happened: `drop_ghost` puts a refusal (the DONE
        // gate on an unmerged worktree is the live one) into the status, and
        // saying "moved to done" over it would be the report contradicting
        // the board. The card is proof either way.
        if self.board.ticket(id).is_some_and(|t| t.column == cols[col]) {
            self.status = format!("moved to {}", cols[col]);
        }
        Ok(())
    }

    /// Rows in `col` excluding the ghost's own ticket (a drop can land after last).
    fn ghost_len(&self, cols: &[String], col: usize, ticket: ulid::Ulid) -> usize {
        cols.get(col)
            .map(|c| self.board.column_tickets(c).iter().filter(|t| t.id != ticket).count())
            .unwrap_or(0)
    }

    /// `start` is the composer's Shift+Enter: mint the ticket AND put claude
    /// on it with the title as a prompt it has already been asked — without
    /// handing the terminal over. The board stays up, so the next ticket is
    /// the next keystroke.
    fn commit_input(&mut self, purpose: InputPurpose, buffer: String, start: bool) -> Result<()> {
        let title = buffer.trim().to_string();
        if title.is_empty() {
            return Ok(());
        }
        match purpose {
            InputPurpose::Create { workspace, tags } => {
                self.mint_ticket(title, workspace, tags, None, start)?;
            }
            InputPurpose::Rename { id } => {
                self.send(Command::RenameTicket { id, title })?;
            }
            // The board's Shift+Enter, second half. `start` is not consulted:
            // sending IS the whole act here, so both Enters do it — see the
            // input scope's ShiftEnter binding for why the harder one stays
            // bound rather than dying under the finger that opened the field.
            InputPurpose::Prompt { ticket, .. } => {
                self.remember_prompt(&title);
                self.status = match self.req(Command::PromptSession { ticket, text: title }) {
                    // Deliberately not "sent to claude": what is provably
                    // true is that it went into the box and Enter was
                    // pressed. Whether the agent took it is the card's to
                    // say, seconds from now, in the only vocabulary that has
                    // ever been trusted for it — the hooks.
                    Response::Ok => "asked".into(),
                    Response::Err { message } => message,
                    _ => String::new(),
                };
                self.refresh()?;
            }
        }
        Ok(())
    }

    /// Mint the composed ticket in the cursor column and replay everything
    /// picked before it had an id: the workspace, the tags, and — from the
    /// editor — the description as its first note. `start` is Shift+Enter's
    /// half: claude on the title, submitted.
    fn mint_ticket(
        &mut self,
        title: String,
        workspace: Option<WorkspaceStrategy>,
        tags: Vec<TagRef>,
        description: Option<String>,
        start: bool,
    ) -> Result<()> {
        let cols = self.columns();
        let column = cols.get(self.cursor_col).cloned().unwrap_or_default();
        match self.req(Command::CreateTicket { column, title }) {
            Response::Created { id } => {
                if workspace.is_some() {
                    let _ = self.req(Command::SetWorkspace { id, workspace });
                }
                // Tags picked with `^t` while the ticket was still
                // being named, replayed now that it has an id.
                for tag in tags {
                    let _ =
                        self.req(Command::SetTag { id, group: tag.group, name: Some(tag.name) });
                }
                if let Some(text) = description {
                    if let Response::Err { message } =
                        self.req(Command::WriteNote { ticket: id, note: None, text })
                    {
                        self.status = message;
                    }
                }
                self.refresh()?;
                self.select_ticket(id);
                if start {
                    self.start_composed(id);
                    return Ok(());
                }
                // Enter-Enter: the next plain Enter starts claude on
                // the fresh ticket (board_enter's fast path).
                self.just_created = Some(id);
                self.status = "enter starts claude ∙ space opens the ticket".into();
            }
            Response::Err { message } => {
                self.status = message;
                self.refresh()?;
            }
            // Pre-Created daemon (rebuild trap): plain Ok, no id to select.
            _ => self.refresh()?,
        }
        Ok(())
    }

    /// One copy of each ask, newest last: a repeat moves to the end rather
    /// than appearing twice on the walk, and the oldest falls off at the cap.
    fn remember_prompt(&mut self, text: &str) {
        self.prompt_history.retain(|p| p != text);
        self.prompt_history.push(text.to_string());
        let over = self.prompt_history.len().saturating_sub(PROMPT_HISTORY_MAX);
        self.prompt_history.drain(..over);
    }

    /// Shift+Enter's second half: start claude on the ticket the composer just
    /// minted — or, from the board, on a ticket whose claude seat is empty —
    /// with its title submitted as the first prompt, and STAY on the board.
    /// No `focus_session` — the whole point of the key is to queue work
    /// without leaving; the card's own state is how the user watches it land.
    /// The fresh-ticket Enter window is not armed either: the agent is already
    /// running, so the next Enter should mean what it always means.
    fn start_composed(&mut self, ticket: ulid::Ulid) {
        let cmd = Command::SpawnSession { ticket, kind: SessionKind::Claude, submit_prompt: true };
        self.status = match self.req(cmd) {
            Response::Spawned { .. } => "claude started on the title".into(),
            // M4: the worktree is still being cut. The daemon replays the
            // parked spawn — submit flag and all — when it lands.
            Response::Provisioning => "provisioning worktree ∙ claude starts when ready".into(),
            Response::Err { message } => message,
            _ => String::new(),
        };
        let _ = self.refresh();
    }

    /// Live sessions of a ticket, in spawn order. `Sleeping` is live-but-parked.
    /// This is the ticket screen's SESSIONS rail — fixed creation order, never
    /// resorted by activity (06 §7 R4). `Exited` drops out, with one exception:
    /// the latest exited claude conversation stays on the rail (Enter resumes
    /// it — the transcript survives the process, a deliberate `x` kill
    /// included) unless dismissed. Since park-on-exit a corpse here is the
    /// UN-wakeable kind: a crash, a logout, or a session that never wrote a
    /// transcript; a clean exit is `Sleeping` and never reaches this branch.
    /// Older corpses re-import through the drawer.
    ///
    /// `Dismissed` is the one exit the rail hides, and it is currently
    /// unreachable from the TUI: `Daemon::kill_session` mints it, but nothing
    /// here sends `Command::KillSession` — `x` on a corpse goes to
    /// `sleep_verb`, which refuses with "only idle sessions sleep".
    pub fn rail_sessions(&self, ticket: ulid::Ulid) -> Vec<&mesimon_core::board::SessionRecord> {
        let corpse = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.ticket == ticket
                    && s.kind == SessionKind::Claude
                    && matches!(s.state, SessionState::Exited { reason } if reason != ExitReason::Dismissed)
            })
            .max_by_key(|s| s.state_changed_at.unwrap_or(0))
            .map(|s| s.id);
        self.board
            .sessions
            .iter()
            .filter(|s| s.ticket == ticket && (s.state.is_live() || Some(s.id) == corpse))
            .collect()
    }

    /// The ticket page's rail: `rail_sessions` first, then every note of the
    /// ticket in creation order — the description included, so a long one
    /// can be paged in the preview zone. Sessions-first is what keeps a
    /// position in `rail_sessions` a valid `rail_idx` (`board_enter`, the
    /// focus return).
    pub fn rail_rows(&self, ticket: ulid::Ulid) -> Vec<RailRow<'_>> {
        let mut rows: Vec<RailRow<'_>> =
            self.rail_sessions(ticket).into_iter().map(RailRow::Session).collect();
        if let Some(t) = self.board.ticket(ticket) {
            rows.extend(t.notes.iter().map(RailRow::Note));
        }
        rows
    }

    fn spawn_and_focus(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Result<()> {
        match self.req(Command::SpawnSession { ticket, kind, submit_prompt: false }) {
            Response::Spawned { id, .. } => {
                self.refresh()?;
                self.focus_session(id)?;
            }
            // M4: the worktree is being created off-thread; the session spawns
            // when it is ready (a BoardChanged follows) and the parked focus
            // intent finishes this keypress then.
            Response::Provisioning => {
                self.status = "provisioning worktree ∙ session starts when ready".into();
                self.pending_spawn_focus = Some((ticket, kind));
                self.refresh()?;
            }
            Response::Err { message } => self.status = message,
            _ => self.refresh()?,
        }
        Ok(())
    }

    fn focus_session(&mut self, sid: uuid::Uuid) -> Result<()> {
        // Remember where the focus started — unfocus lands back there.
        let origin = match self.screen {
            Screen::Ticket { .. } => FocusOrigin::Ticket,
            _ => FocusOrigin::Board,
        };
        // Enter means "get me into this session": paneless records (imported
        // observe-only, sleeping, exited claude) resume first, then the focus
        // flow runs. An exited claude is a conversation, not a process — the
        // daemon replays its argv (`--resume`) into a fresh pane.
        if let Some(rec) = self.board.sessions.iter().find(|s| s.id == sid) {
            let observe_only = rec.provenance == Provenance::Adopted && rec.argv.is_empty();
            let sleeping = matches!(rec.state, SessionState::Sleeping);
            let exited_claude =
                rec.kind == SessionKind::Claude && matches!(rec.state, SessionState::Exited { .. });
            if observe_only || sleeping || exited_claude {
                let cmd = if sleeping && rec.kind == SessionKind::Bash {
                    Command::WakeSession { id: sid }
                } else {
                    Command::ResumeSession { id: sid, confirm: self.resume_refused == Some(sid) }
                };
                match self.req(cmd) {
                    Response::Spawned { fresh, .. } => {
                        self.resume_refused = None;
                        // The row said "resume" and this is not one: the
                        // record had no conversation left, so the daemon
                        // started a new one rather than refusing forever.
                        // Someone who thought they were picking up work has
                        // to be told they are not.
                        if fresh {
                            self.status = "nothing to resume ∙ started a fresh conversation".into();
                        }
                        self.refresh()?;
                        // fall through to the focus flow below
                    }
                    Response::Err { message } => {
                        if message.contains("running elsewhere") {
                            // The daemon's message says "resume again to
                            // override" — the next Enter carries the confirm.
                            self.resume_refused = Some(sid);
                        }
                        self.status = message;
                        self.refresh()?;
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
            }
        }
        // GATE (D20): prove the unfocus key once before the first real focus.
        match self.req(Command::GateStatus) {
            Response::Gate { passed: true, .. } => {
                match self.req(Command::FocusStart { session: sid }) {
                    Response::Attach { argv } => {
                        self.pending_attach = Some(argv);
                        self.focused_session_hint = Some((sid, origin));
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
            }
            Response::Gate { passed: false, attach_argv: Some(argv) } => {
                self.pending_attach = Some(argv);
                self.pending_gate_then = Some((sid, origin));
            }
            Response::Err { message } => self.status = message,
            _ => {}
        }
        Ok(())
    }

    /// Called by the main loop after a handover returns.
    pub fn after_handover(&mut self) -> Result<()> {
        if let Some((sid, origin)) = self.pending_gate_then.take() {
            // Detaching from the gate session IS the proof (D20).
            self.send(Command::GatePassed)?;
            match self.req(Command::FocusStart { session: sid }) {
                Response::Attach { argv } => {
                    self.pending_attach = Some(argv);
                    self.focused_session_hint = Some((sid, origin));
                    return Ok(());
                }
                Response::Err { message } => self.status = message,
                _ => {}
            }
        } else if let Some((sid, origin)) = self.focused_session_hint.take() {
            self.send(Command::FocusEnd { session: sid })?;
            self.refresh()?;
            // Unfocus returns exactly where the focus started: board Enter
            // comes back to the board (ticket selected), ticket-screen focus
            // comes back to the ticket screen (session selected).
            if let Some(rec) = self.board.sessions.iter().find(|s| s.id == sid) {
                let ticket = rec.ticket;
                self.select_ticket(ticket);
                match origin {
                    FocusOrigin::Board => self.to_board(),
                    FocusOrigin::Ticket => {
                        let idx = self
                            .rail_sessions(ticket)
                            .iter()
                            .position(|s| s.id == sid)
                            .unwrap_or(0);
                        self.screen = Screen::Ticket { ticket, rail_idx: idx };
                    }
                }
            }
            return Ok(());
        }
        self.refresh()
    }

    /// Jump the cursor to the next (or previous) card needing attention.
    /// Queue order: precedence rank, then longest-waiting (daemon-minted
    /// `waiting_since`), wrapping. Inert when nothing waits. Pops back to the
    /// board — attention is a board-level gesture.
    fn cycle_attention(&mut self, reverse: bool) {
        let queue = mesimon_core::attention::attention_queue(&self.board);
        let mut tickets: Vec<ulid::Ulid> = Vec::new();
        for s in queue {
            if !tickets.contains(&s.ticket) {
                tickets.push(s.ticket);
            }
        }
        if tickets.is_empty() {
            self.status = "nothing needs you".into();
            return;
        }
        self.to_board();
        self.mode = Mode::Normal;
        let current = self.selected_ticket().map(|t| t.id);
        let pos = current.and_then(|id| tickets.iter().position(|t| *t == id));
        let next = match (pos, reverse) {
            (Some(i), false) => tickets[(i + 1) % tickets.len()],
            (Some(i), true) => tickets[(i + tickets.len() - 1) % tickets.len()],
            (None, _) => tickets[0],
        };
        self.select_ticket(next);
    }

    /// Point the board cursor at a ticket (so Esc from the ticket screen lands on it).
    fn select_ticket(&mut self, ticket: ulid::Ulid) {
        let cols = self.columns();
        for (ci, col) in cols.iter().enumerate() {
            if let Some(ri) = self.board.column_tickets(col).iter().position(|t| t.id == ticket) {
                self.cursor_col = ci;
                self.cursor_row = ri;
                return;
            }
        }
    }

    fn send(&mut self, command: Command) -> Result<()> {
        if let Response::Err { message } = self.req(command) {
            self.status = message;
        }
        self.refresh()
    }
}

type Snapshot6 = (
    Board,
    Vec<GraceItem>,
    Vec<ExternalItem>,
    Resources,
    Vec<WorktreeItem>,
    Vec<mesimon_core::command::Notice>,
);

fn fetch(client: &mut dyn Transport) -> Result<Snapshot6> {
    match client.request(Command::Snapshot)? {
        // `shell_env` is deliberately not carried through here: `App::apply`
        // owns that field and runs on the first tick, so the startup snapshot
        // would only be setting it a quarter-second early.
        Response::Board { board, grace, external, resources, worktrees, notices, .. } => {
            Ok((board, grace, external, resources, worktrees, notices))
        }
        other => anyhow::bail!("unexpected snapshot response: {other:?}"),
    }
}

/// Test-only transport + constructor: canned snapshots, no daemon, no tmux.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub struct FakeTransport {
        pub board: Board,
        pub grace: Vec<GraceItem>,
        pub external: Vec<ExternalItem>,
        pub resources: Resources,
        pub shell_env: mesimon_core::command::ShellEnvStatus,
        /// Debug-formatted log of every request, for behavior assertions.
        pub sent: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        /// Make FocusStart answer Err (the daemon refusing a focus).
        pub refuse_focus: bool,
        /// Note bodies by id, the daemon's files stood in for.
        pub notes: std::collections::HashMap<ulid::Ulid, String>,
    }

    impl Transport for FakeTransport {
        fn request(&mut self, command: Command) -> Result<Response> {
            self.sent.borrow_mut().push(format!("{command:?}"));
            match command {
                Command::CreateTicket { column, title } => {
                    let id = ulid::Ulid(999);
                    self.board.tickets.push(Ticket {
                        id,
                        short_key: "T-999".into(),
                        title,
                        column,
                        order: "zzzz".into(),
                        created_at: "1970-01-01T00:00:00Z".into(),
                        entered_at: None,
                        workspace: None,
                        tags: Vec::new(),
                        notes: Vec::new(),
                        archived: None,
                    });
                    return Ok(Response::Created { id });
                }
                Command::GateStatus => {
                    return Ok(Response::Gate { passed: true, attach_argv: None });
                }
                Command::FocusStart { .. } => {
                    return Ok(if self.refuse_focus {
                        Response::Err { message: "no pane".into() }
                    } else {
                        Response::Attach { argv: vec!["tmux".into()] }
                    });
                }
                Command::MergeTicket { .. } => {
                    return Ok(Response::Merge {
                        outcome: MergeOutcome::Merged,
                        detail: "merged 2 commit(s)".into(),
                    });
                }
                Command::ReadNote { ticket, note } => {
                    let known = self.board.ticket(ticket).and_then(|t| t.note(note)).cloned();
                    return Ok(match (known, self.notes.get(&note)) {
                        (Some(meta), Some(text)) => Response::Note { text: text.clone(), meta },
                        _ => Response::Err { message: "no such note".into() },
                    });
                }
                Command::WriteNote { ticket, note, text } => {
                    let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == ticket) else {
                        return Ok(Response::Err { message: "no such ticket".into() });
                    };
                    if text.trim().is_empty() {
                        let Some(id) = note else {
                            return Ok(Response::Err { message: "nothing to save".into() });
                        };
                        t.notes.retain(|n| n.id != id);
                        self.notes.remove(&id);
                        return Ok(Response::NoteWritten { note: None });
                    }
                    let name = mesimon_core::board::note_name(&text);
                    let id = match note {
                        Some(id) => {
                            let Some(n) = t.notes.iter_mut().find(|n| n.id == id) else {
                                return Ok(Response::Err { message: "no such note".into() });
                            };
                            n.rev += 1;
                            n.name = name;
                            id
                        }
                        None => {
                            let id = ulid::Ulid(900 + t.notes.len() as u128);
                            t.notes.push(mesimon_core::board::NoteMeta {
                                id,
                                name,
                                rev: 1,
                                created_at: "@1000".into(),
                                created_by: "local".into(),
                                edited_at: "@1000".into(),
                                edited_by: "local".into(),
                            });
                            id
                        }
                    };
                    self.notes.insert(id, text);
                    return Ok(Response::NoteWritten { note: Some(id) });
                }
                Command::SpawnSession { ticket, kind, submit_prompt } => {
                    let mut rec = mesimon_core::board::SessionRecord::new(
                        uuid::Uuid::from_u128(4242),
                        kind,
                        ticket,
                        vec!["claude".into()],
                        "/repo".into(),
                        SessionState::Running,
                    );
                    rec.pending_submit = submit_prompt;
                    let id = rec.id;
                    self.board.sessions.push(rec);
                    return Ok(Response::Spawned { id, fresh: false });
                }
                _ => {}
            }
            match command {
                Command::Snapshot => Ok(Response::Board {
                    board: self.board.clone(),
                    grace: self.grace.clone(),
                    external: self.external.clone(),
                    resources: self.resources.clone(),
                    worktrees: Vec::new(),
                    notices: Vec::new(),
                    shell_env: self.shell_env.clone(),
                }),
                Command::MoveTicket { id, column, before } => {
                    // The daemon's DONE gate, as close as the fake can stand
                    // in for it: it holds no worktree bindings, so a ticket
                    // that WANTS a worktree plays the part of one whose branch
                    // has not merged. The point under test is the refusal
                    // reaching the client, not which git fact caused it.
                    if column == "DONE"
                        && self.board.ticket(id).and_then(|t| t.workspace)
                            == Some(mesimon_core::board::WorkspaceStrategy::Worktree)
                    {
                        return Ok(Response::Err {
                            message: "worktree unmerged — merge before DONE".into(),
                        });
                    }
                    let mut order: Vec<ulid::Ulid> = self
                        .board
                        .column_tickets(&column)
                        .iter()
                        .map(|t| t.id)
                        .filter(|t| *t != id)
                        .collect();
                    let pos = before
                        .and_then(|b| order.iter().position(|t| *t == b))
                        .unwrap_or(order.len());
                    order.insert(pos, id);
                    if let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == id) {
                        t.column = column;
                    }
                    // Renumber the whole target column: orders only compare
                    // within a column, so clobbering them is safe here.
                    for (i, tid) in order.iter().enumerate() {
                        if let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == *tid) {
                            t.order = format!("{i:04}");
                        }
                    }
                    Ok(Response::Ok)
                }
                // Mirror the daemon's gate so the refusal path is testable.
                Command::ArchiveTicket { id } => {
                    if self.board.sessions.iter().any(|s| s.ticket == id && s.state.has_pane()) {
                        return Ok(Response::Err {
                            message: "sessions still awake — sleep them first".into(),
                        });
                    }
                    match self.board.tickets.iter_mut().find(|t| t.id == id) {
                        Some(t) => {
                            t.archived = Some(mesimon_core::board::Archived {
                                at: "@1000".into(),
                                by: "local".into(),
                            });
                            Ok(Response::Ok)
                        }
                        None => Ok(Response::Err { message: "no such ticket".into() }),
                    }
                }
                Command::UnarchiveTicket { id } => {
                    match self.board.tickets.iter_mut().find(|t| t.id == id) {
                        Some(t) => {
                            t.archived = None;
                            Ok(Response::Ok)
                        }
                        None => Ok(Response::Err { message: "no such ticket".into() }),
                    }
                }
                _ => Ok(Response::Ok),
            }
        }

        fn poll_event(&mut self) -> bool {
            false
        }
    }

    impl App {
        pub(crate) fn for_test(board: Board, theme: Theme) -> App {
            Self::for_test_logged(board, theme, false).0
        }

        /// Like `for_test`, but hands back the request log (and optionally a
        /// focus-refusing daemon) for behavior assertions.
        pub(crate) fn for_test_logged(
            board: Board,
            theme: Theme,
            refuse_focus: bool,
        ) -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
            let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            let fake = FakeTransport {
                board,
                grace: vec![],
                external: vec![],
                resources: Resources::default(),
                shell_env: Default::default(),
                sent: sent.clone(),
                refuse_focus,
                notes: std::collections::HashMap::new(),
            };
            let app = App::new(Box::new(fake), PathBuf::from("/repo/kanban-tui"), theme)
                .expect("fake transport snapshot");
            (app, sent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};
    use mesimon_core::board::{Column, Ticket};

    fn ticket(n: u128, column: &str, order: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid(n),
            short_key: format!("T-{n}"),
            title: format!("ticket {n}"),
            column: column.into(),
            order: order.into(),
            created_at: "1970-01-01T00:00:00Z".into(),
            entered_at: None,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        }
    }

    fn board_three_columns() -> Board {
        let mut b = Board::default();
        for (i, name) in ["todo", "doing", "done"].iter().enumerate() {
            b.columns.push(Column { name: (*name).into(), order: format!("{i}") });
        }
        b.tickets.push(ticket(1, "todo", "a"));
        b.tickets.push(ticket(2, "todo", "b"));
        b.tickets.push(ticket(3, "done", "a"));
        b
    }

    fn theme() -> Theme {
        Theme::new(Flavor::Graphite, Profile::TrueColor)
    }

    fn app_three_columns() -> App {
        App::for_test(board_three_columns(), theme())
    }

    /// Three-column board plus one claude session on ticket 1, request log out.
    fn app_with_claude(
        state: SessionState,
        refuse_focus: bool,
    ) -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>, uuid::Uuid) {
        app_with_session(SessionKind::Claude, state, refuse_focus)
    }

    fn app_with_shell(
        state: SessionState,
    ) -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>, uuid::Uuid) {
        app_with_session(SessionKind::Bash, state, false)
    }

    fn app_with_session(
        kind: SessionKind,
        state: SessionState,
        refuse_focus: bool,
    ) -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>, uuid::Uuid) {
        let mut b = board_three_columns();
        let sid = uuid::Uuid::from_u128(7);
        b.sessions.push(mesimon_core::board::SessionRecord::new(
            sid,
            kind,
            ulid::Ulid(1),
            vec![if kind == SessionKind::Claude { "claude" } else { "zsh" }.into()],
            "/repo".into(),
            state,
        ));
        let (app, sent) = App::for_test_logged(b, theme(), refuse_focus);
        (app, sent, sid)
    }

    fn note_meta(n: u128, rev: u64, by: &str) -> mesimon_core::board::NoteMeta {
        mesimon_core::board::NoteMeta {
            id: ulid::Ulid(n),
            name: format!("note {n}"),
            rev,
            created_at: "@1000".into(),
            created_by: by.into(),
            edited_at: "@1000".into(),
            edited_by: by.into(),
        }
    }

    /// Three columns, ticket 1 with one note (id 90, body in the fake's
    /// files), request log out.
    fn app_with_note() -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        app_with_note_and(false)
    }

    /// …and, when asked, a running claude on ticket 1 — in the FAKE's board,
    /// since every save refreshes from it.
    fn app_with_note_and(claude: bool) -> (App, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
        let mut b = board_three_columns();
        b.tickets[0].notes.push(note_meta(90, 1, "local"));
        if claude {
            b.sessions.push(mesimon_core::board::SessionRecord::new(
                uuid::Uuid::from_u128(7),
                SessionKind::Claude,
                ulid::Ulid(1),
                vec!["claude".into()],
                "/repo".into(),
                SessionState::Running,
            ));
        }
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut notes = std::collections::HashMap::new();
        notes.insert(ulid::Ulid(90), "# Why\n\nbecause".to_string());
        let fake = super::test_support::FakeTransport {
            board: b,
            grace: vec![],
            external: vec![],
            resources: Resources::default(),
            shell_env: Default::default(),
            sent: sent.clone(),
            refuse_focus: false,
            notes,
        };
        let app = App::new(Box::new(fake), PathBuf::from("/repo/kanban-tui"), theme())
            .expect("fake transport snapshot");
        (app, sent)
    }

    fn editor(app: &App) -> &Editor {
        match &app.mode {
            Mode::Editor(e) => e,
            other => panic!("not in the editor: {other:?}"),
        }
    }

    #[test]
    fn tab_carries_the_title_into_the_editor() {
        let mut app = app_three_columns();
        press(&mut app, 'o');
        for c in "Ship it".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::BackTab, KeyModifiers::SHIFT).unwrap();
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE).unwrap();
        let ed = editor(&app);
        assert_eq!(ed.title.as_str(), "Ship it");
        assert_eq!(ed.focus, Field::Body);
        assert!(matches!(
            ed.purpose,
            EditorPurpose::Compose { workspace: Some(WorkspaceStrategy::Worktree), .. }
        ));
        assert!(!ed.dirty(), "opening is not a change");
        // Esc on a clean editor goes back to the one-line field, title intact.
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        match &app.mode {
            Mode::Input { purpose: InputPurpose::Create { workspace, .. }, buffer } => {
                assert_eq!(buffer.as_str(), "Ship it");
                assert_eq!(*workspace, Some(WorkspaceStrategy::Worktree));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn editor_save_mints_then_writes_the_description() {
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        press(&mut app, 'o');
        for c in "Ship it".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE).unwrap();
        for c in "why".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        for c in "and how".chars() {
            press(&mut app, c);
        }
        assert!(editor(&app).dirty());
        ctrl(&mut app, 's');
        assert_eq!(app.mode, Mode::Normal);
        let log = sent.borrow().join("\n");
        let create = log.find("CreateTicket").expect("minted");
        let note = log.find("WriteNote").expect("described");
        assert!(create < note, "the ticket before its note: {log}");
        assert!(log.contains("why\\nand how"), "newlines survive: {log}");
        assert!(app.status.contains("enter starts claude"), "{}", app.status);
    }

    #[test]
    fn shift_enter_in_the_editor_mints_and_starts() {
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = true;
        press(&mut app, 'o');
        for c in "Ship it".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE).unwrap();
        for c in "why".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert_eq!(app.mode, Mode::Normal);
        assert!(sent_contains(&sent, "submit_prompt: true"), "{:?}", sent.borrow());
        assert!(sent_contains(&sent, "WriteNote"), "{:?}", sent.borrow());
    }

    #[test]
    fn a_blank_title_cannot_mint() {
        let mut app = app_three_columns();
        press(&mut app, 'o');
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE).unwrap();
        for c in "why".chars() {
            press(&mut app, c);
        }
        ctrl(&mut app, 's');
        assert!(matches!(app.mode, Mode::Editor(_)));
        assert_eq!(app.status, "a ticket needs a title");
    }

    #[test]
    fn n_rereads_the_note_before_opening_and_edits_it() {
        let (mut app, sent) = app_with_note();
        app.remember_note(ulid::Ulid(90), 1, Some("stale".into()));
        press(&mut app, 'n');
        assert!(sent_contains(&sent, "ReadNote"), "{:?}", sent.borrow());
        let ed = editor(&app);
        assert_eq!(ed.body.as_str(), "# Why\n\nbecause", "the daemon's text, not the cache");
        assert_eq!(ed.title.as_str(), "ticket 1");
        assert!(matches!(ed.purpose, EditorPurpose::Note { note: Some(_), .. }));
        // The title is the ticket's: typing goes nowhere there.
        assert_eq!(ed.focus, Field::Body);
    }

    #[test]
    fn a_note_save_stays_open_and_the_second_ctrl_s_tells_claude() {
        let (mut app, sent) = app_with_note();
        // No claude: the save says so and the second press is inert.
        press(&mut app, 'n');
        app.handle_key(KeyCode::End, KeyModifiers::NONE).unwrap();
        press(&mut app, '!');
        ctrl(&mut app, 's');
        assert!(matches!(app.mode, Mode::Editor(_)), "stays open");
        assert_eq!(app.status, "saved");
        assert!(!editor(&app).dirty());
        let before = sent.borrow().len();
        ctrl(&mut app, 's');
        assert_eq!(sent.borrow().len(), before, "nothing to tell");
        // With a live claude the save offers the second press, and it asks.
        let (mut app, sent) = app_with_note_and(true);
        press(&mut app, 'n');
        app.handle_key(KeyCode::End, KeyModifiers::NONE).unwrap();
        press(&mut app, '?');
        ctrl(&mut app, 's');
        assert_eq!(app.status, "saved ∙ ^s again tells claude");
        ctrl(&mut app, 's');
        assert_eq!(app.status, "asked");
        assert!(sent_contains(&sent, "NoteToAgent"), "{:?}", sent.borrow());
    }

    #[test]
    fn esc_is_two_press_only_when_dirty() {
        let (mut app, _) = app_with_note();
        press(&mut app, 'n');
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Normal, "clean: one press");
        press(&mut app, 'n');
        press(&mut app, 'x');
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Editor(_)));
        assert_eq!(app.status, "unsaved ∙ esc again discards");
        // Any other key disarms it.
        press(&mut app, 'y');
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Editor(_)), "disarmed by the letter");
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn emptying_a_note_takes_two_presses_and_deletes() {
        let (mut app, sent) = app_with_note();
        press(&mut app, 'n');
        // Wipe the body: to the end of each line, kill it, eat the break.
        for _ in 0..8 {
            app.handle_key(KeyCode::End, KeyModifiers::NONE).unwrap();
            ctrl(&mut app, 'u');
            app.handle_key(KeyCode::Delete, KeyModifiers::NONE).unwrap();
        }
        assert!(editor(&app).body.as_str().is_empty(), "{:?}", editor(&app).body.as_str());
        ctrl(&mut app, 's');
        assert!(matches!(app.mode, Mode::Editor(_)));
        assert_eq!(app.status, "empty ∙ ^s again deletes the note");
        ctrl(&mut app, 's');
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.status, "note deleted");
        assert!(sent_contains(&sent, "text: \"\""), "{:?}", sent.borrow());
        assert!(app.board.tickets[0].notes.is_empty());
    }

    #[test]
    fn big_n_always_opens_a_fresh_note() {
        let (mut app, sent) = app_with_note();
        press(&mut app, 'N');
        assert!(!sent_contains(&sent, "ReadNote"), "{:?}", sent.borrow());
        assert!(matches!(editor(&app).purpose, EditorPurpose::Note { note: None, .. }));
        for c in "second".chars() {
            press(&mut app, c);
        }
        ctrl(&mut app, 's');
        assert_eq!(app.board.tickets[0].notes.len(), 2);
        assert!(matches!(editor(&app).purpose, EditorPurpose::Note { note: Some(_), .. }));
        assert_eq!(app.status, "saved");
    }

    #[test]
    fn poll_notes_fetches_once_per_rev() {
        let (mut app, sent) = app_with_note();
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        assert!(app.poll_notes());
        let n = sent.borrow().iter().filter(|c| c.contains("ReadNote")).count();
        assert_eq!(n, 1);
        assert!(!app.poll_notes(), "steady state asks nothing");
        assert_eq!(sent.borrow().iter().filter(|c| c.contains("ReadNote")).count(), 1);
        // A new revision on the snapshot is the edge.
        app.board.tickets[0].notes[0].rev = 2;
        assert!(app.poll_notes());
        assert_eq!(sent.borrow().iter().filter(|c| c.contains("ReadNote")).count(), 2);
    }

    #[test]
    fn a_note_row_is_not_a_session() {
        let (mut app, _) = app_with_note();
        app.board.sessions.push(mesimon_core::board::SessionRecord::new(
            uuid::Uuid::from_u128(7),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            SessionState::Sleeping,
        ));
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 1 };
        let ctx = app.ctx();
        assert!(ctx.sel_note);
        assert!(!ctx.sel_session);
        assert!(!ctx.sel_sleeping);
        assert!(!ctx.sel_dead);
        assert!(ctx.ticket_described);
        // `x` has nothing to sleep here; Enter opens the note.
        press(&mut app, 'x');
        assert_eq!(app.mode, Mode::Normal);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(matches!(editor(&app).purpose, EditorPurpose::Note { note: Some(_), .. }));
    }

    #[test]
    fn paste_into_the_body_keeps_newlines_and_is_trimmed_at_the_limit() {
        let (mut app, _) = app_with_note();
        press(&mut app, 'N');
        app.on_paste("one\r\ntwo\n").unwrap();
        assert_eq!(editor(&app).body.as_str(), "one\ntwo\n");
        let big = "x".repeat(mesimon_core::board::NOTE_MAX_BYTES + 10);
        app.on_paste(&big).unwrap();
        assert!(app.status.starts_with("paste trimmed ∙ a note holds at most"), "{}", app.status);
        assert_eq!(editor(&app).body.as_str().len(), mesimon_core::board::NOTE_MAX_BYTES);
    }

    fn press(app: &mut App, c: char) {
        app.handle_key(KeyCode::Char(c), KeyModifiers::NONE).unwrap();
    }

    fn sent_contains(sent: &std::cell::RefCell<Vec<String>>, needle: &str) -> bool {
        sent.borrow().iter().any(|c| c.contains(needle))
    }

    /// Archiving is `a` then `a` — the chord the author asked for.
    fn archive(app: &mut App) {
        press(app, 'a');
        press(app, 'a');
    }

    fn field_text(app: &App) -> String {
        match &app.mode {
            Mode::Input { buffer, .. } => buffer.as_str().to_string(),
            _ => panic!("no text field open"),
        }
    }

    /// The bug: a multi-line paste into the composer arrived as keystrokes,
    /// so the first newline saved a one-line ticket and the rest of the
    /// clipboard walked the board as verbs. As one event it is one title.
    #[test]
    fn paste_into_the_composer_is_one_title() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        app.mode = Mode::Input {
            purpose: InputPurpose::Create { workspace: None, tags: Vec::new() },
            buffer: EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
        };
        assert!(app.on_paste("fix the\nauth bug\n").unwrap());
        assert_eq!(field_text(&app), "fix the auth bug");
        assert!(!sent_contains(&sent, "create_ticket"), "a paste never saves");
        assert!(app.status.is_empty(), "{}", app.status);
        // Enter saves the whole line, once.
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert!(sent_contains(&sent, "fix the auth bug"));
    }

    /// With no field open a paste is nothing — not a walk through the keymap.
    #[test]
    fn paste_on_the_board_is_inert() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        let before = sent.borrow().len();
        assert!(!app.on_paste("j\nd\nd\nn\n").unwrap());
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.cursor_row, 0);
        assert_eq!(sent.borrow().len(), before);
    }

    /// A paste past the field's limit is cut at the daemon's own cap and the
    /// status says so — the one thing about a paste the user cannot see.
    #[test]
    fn oversized_paste_is_trimmed_and_announced() {
        use mesimon_core::command::PROMPT_MAX_BYTES;
        let mut app = app_three_columns();
        app.mode = Mode::Input {
            purpose: InputPurpose::Prompt { ticket: ulid::Ulid(1), walk: None },
            buffer: EditBuffer::new(PROMPT_MAX_BYTES),
        };
        app.on_paste(&"x".repeat(PROMPT_MAX_BYTES + 100)).unwrap();
        assert_eq!(field_text(&app).len(), PROMPT_MAX_BYTES);
        assert!(app.status.contains("paste trimmed"), "{}", app.status);
        assert!(app.status.contains("an ask holds at most 4 KB"), "{}", app.status);
        // The composer's limit is the title's, and the words say so.
        app.mode = Mode::Input {
            purpose: InputPurpose::Create { workspace: None, tags: Vec::new() },
            buffer: EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
        };
        app.on_paste(&"y".repeat(mesimon_core::board::TITLE_MAX_BYTES + 1)).unwrap();
        assert!(app.status.contains("a title holds at most 2 KB"), "{}", app.status);
        // The next keypress clears it, like every status.
        app.handle_key(KeyCode::Backspace, KeyModifiers::NONE).unwrap();
        app.on_key(KeyCode::Char('z'), KeyModifiers::NONE).unwrap();
        assert!(app.status.is_empty());
    }

    /// The tag picker: a name field takes the paste under its own cap, and
    /// an open picker with no field takes nothing (never the composer
    /// underneath).
    #[test]
    fn paste_into_the_picker_goes_to_the_name_field_only() {
        use mesimon_core::board::TAG_MAX_BYTES;
        let mut app = app_three_columns();
        app.mode = Mode::Input {
            purpose: InputPurpose::Create { workspace: None, tags: Vec::new() },
            buffer: EditBuffer::new(mesimon_core::board::TITLE_MAX_BYTES),
        };
        app.tag_armed =
            Some(TagArm { ticket: None, row: 0, col: 0, naming: None, forget_armed: false });
        assert!(!app.on_paste("HOTFIX\n").unwrap());
        assert_eq!(field_text(&app), "");
        app.tag_armed.as_mut().unwrap().naming =
            Some((Naming::New, EditBuffer::new(TAG_MAX_BYTES)));
        assert!(app.on_paste("HOT\nFIX ".repeat(6).as_str()).unwrap());
        let name = app.tag_armed.as_ref().unwrap().naming.as_ref().unwrap().1.as_str().to_string();
        assert!(name.starts_with("HOT FIX HOT FIX"), "{name}");
        assert!(name.len() <= TAG_MAX_BYTES);
        assert!(app.status.contains("a tag name holds at most 24 bytes"), "{}", app.status);
        assert_eq!(field_text(&app), "", "the composer under the picker took nothing");
    }

    /// The archived list has no key of its own any more — it is a menu row.
    /// Every test that used to press `V` walks the menu instead, which is
    /// also a small proof that the menu reaches what the keys gave up.
    fn open_archived(app: &mut App) {
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        let items = keymap::menu_items(&app.ctx());
        let idx = items
            .iter()
            .position(|m| m.verb == Verb::ArchivedList)
            .expect("the archived row is offered when something is archived");
        for _ in 0..idx {
            press(app, 'j');
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
    }

    /// The theme picker is a menu row too.
    fn open_theme_picker(app: &mut App) {
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        let items = keymap::menu_items(&app.ctx());
        let idx = items.iter().position(|m| m.verb == Verb::ThemePick).expect("the theme row");
        for _ in 0..idx {
            press(app, 'j');
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
    }

    #[test]
    fn theme_row_opens_the_picker_on_the_current_theme() {
        let mut app = app_three_columns();
        app.theme = Theme::new(Flavor::Blue, Profile::TrueColor);
        open_theme_picker(&mut app);
        assert_eq!(app.mode, Mode::Theme { idx: 2 }, "the cursor starts on blue");
        assert_eq!(app.scope(), Scope::Theme);
        assert_eq!(app.theme.flavor, Flavor::Blue, "opening previews nothing");
    }

    #[test]
    fn moving_the_cursor_previews_and_esc_puts_it_back() {
        let mut app = app_three_columns();
        open_theme_picker(&mut app);
        assert_eq!(app.mode, Mode::Theme { idx: 0 });
        press(&mut app, 'j');
        assert_eq!(app.theme.flavor, Flavor::Chalk, "the cursor is the preview");
        press(&mut app, 'j');
        assert_eq!(app.theme.flavor, Flavor::Blue);
        assert!(app.force_redraw, "a retheme repaints from nothing");
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.theme.flavor, Flavor::Graphite, "esc puts the resting theme back");
        assert_eq!(app.prefs.dark, Flavor::Graphite, "nothing was saved");
    }

    #[test]
    fn enter_saves_the_slot_the_terminal_reports() {
        let dir = std::env::temp_dir().join(format!("msmn-app-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("prefs.json");
        let mut app = app_three_columns();
        app.prefs_path = Some(path.clone());
        app.ground = Ground::Light;
        open_theme_picker(&mut app);
        press(&mut app, 'j');
        press(&mut app, 'j');
        press(&mut app, 'j');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.theme.flavor, Flavor::Amber);
        assert_eq!(app.status, "amber saved for light terminals");
        let back = crate::prefs::load(&path).prefs;
        assert_eq!(back.light, Flavor::Amber);
        assert_eq!(back.dark, Flavor::Graphite, "the other slot is untouched");
        // A ground flip now lands on the other slot's pick.
        app.ground = Ground::Dark;
        assert_eq!(app.resting_flavor(), Flavor::Graphite);
    }

    #[test]
    fn a_pin_is_cleared_by_a_pick_and_the_status_says_so() {
        let mut app = app_three_columns();
        app.forced = Some(Flavor::Green);
        app.theme = Theme::new(Flavor::Green, Profile::TrueColor);
        let ctx = app.ctx();
        assert!(ctx.theme_pinned);
        open_theme_picker(&mut app);
        press(&mut app, 'k');
        press(&mut app, 'k');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.theme.flavor, Flavor::Blue);
        assert!(app.forced.is_none(), "the pick is the more recent choice");
        assert!(app.status.contains("pins the next launch"), "{}", app.status);
        assert_eq!(app.prefs.dark, Flavor::Blue);
    }

    #[test]
    fn enter_on_plain_ticket_opens_the_ticket_screen() {
        let mut app = app_three_columns();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.screen, Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 });
    }

    #[test]
    fn enter_right_after_compose_spawns_claude() {
        // o, title, Enter mints the ticket; the next Enter starts the work.
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        press(&mut app, 'o');
        press(&mut app, 'n');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.just_created, Some(ulid::Ulid(999)));
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "SpawnSession"));
        assert!(app.pending_attach.is_some(), "spawned session focuses");
        assert_eq!(app.screen, Screen::Board, "focus starts from the board");
        // Consumed: a third Enter (post-unfocus) must not spawn again — the
        // awake-claude fast path owns it now.
    }

    /// The same key one stage later. The ticket exists and an agent is on
    /// it, so Shift+Enter has nothing to mint — it opens a field on the card
    /// instead, and Enter puts what was typed in front of that agent. Two
    /// presses and a sentence, and the board never went away.
    #[test]
    fn shift_enter_prompts_a_live_agent_from_the_board() {
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = true;
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert!(
            matches!(app.mode, Mode::Input { purpose: InputPurpose::Prompt { .. }, .. }),
            "the press opens a field, it does not send anything: {:?}",
            app.mode
        );
        assert!(
            !sent_contains(&sent, "PromptSession"),
            "the press that OPENS a prompt must never also deliver one"
        );
        // …and it is the ticket under the cursor that gets asked.
        assert!(matches!(
            app.mode,
            Mode::Input { purpose: InputPurpose::Prompt { ticket, .. }, .. } if ticket == ulid::Ulid(1)
        ));
        for c in "run the tests".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(
            sent_contains(&sent, "run the tests"),
            "the typed prompt travels verbatim: {:?}",
            sent.borrow()
        );
        assert!(sent_contains(&sent, "PromptSession"));
        assert_eq!(app.screen, Screen::Board, "the board never leaves");
        assert!(app.pending_attach.is_none(), "no handover — that is the point");
        assert_eq!(app.mode, Mode::Normal, "the field closed");
    }

    /// `↑` in the prompt field recalls what was asked before, newest first;
    /// `↓` walks back toward the present, and the step past the newest ask
    /// is the draft that was under the cursor when the walk began. A recalled
    /// ask is sent as-is by Enter, and sending it again moves it to the end
    /// of the walk rather than listing it twice.
    #[test]
    fn prompt_field_walks_its_history_and_comes_back_to_the_draft() {
        fn field(app: &App) -> String {
            match &app.mode {
                Mode::Input { buffer, .. } => buffer.as_str().to_string(),
                m => panic!("not in a field: {m:?}"),
            }
        }
        fn open(app: &mut App) {
            app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
            assert!(matches!(app.mode, Mode::Input { purpose: InputPurpose::Prompt { .. }, .. }));
        }
        fn up(app: &mut App) {
            app.handle_key(KeyCode::Up, KeyModifiers::NONE).unwrap();
        }
        fn down(app: &mut App) {
            app.handle_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
        }
        fn ask(app: &mut App, text: &str) {
            open(app);
            for c in text.chars() {
                press(app, c);
            }
            app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        }
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = true;

        // No past: the arrows are inert and the draft is untouched.
        open(&mut app);
        press(&mut app, 'x');
        up(&mut app);
        down(&mut app);
        assert_eq!(field(&app), "x", "nothing to recall yet");
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();

        ask(&mut app, "run the tests");
        ask(&mut app, "rebase");

        open(&mut app);
        for c in "dra".chars() {
            press(&mut app, c);
        }
        up(&mut app);
        assert_eq!(field(&app), "rebase", "first ↑ is the newest ask");
        up(&mut app);
        assert_eq!(field(&app), "run the tests", "second ↑ is one older");
        up(&mut app);
        assert_eq!(field(&app), "run the tests", "the oldest is a wall, not a wrap");
        down(&mut app);
        assert_eq!(field(&app), "rebase");
        down(&mut app);
        assert_eq!(field(&app), "dra", "↓ past the newest ask is the draft again");
        assert!(
            matches!(
                &app.mode,
                Mode::Input { purpose: InputPurpose::Prompt { walk: None, .. }, .. }
            ),
            "back on the draft, the walk is over"
        );
        down(&mut app);
        assert_eq!(field(&app), "dra", "and stays the draft");

        // Recall the oldest and send it: it travels verbatim and moves to the
        // end of the walk — one copy, newest position.
        up(&mut app);
        up(&mut app);
        assert_eq!(field(&app), "run the tests");
        sent.borrow_mut().clear();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "run the tests"));
        assert_eq!(app.prompt_history, vec!["rebase".to_string(), "run the tests".to_string()]);
        open(&mut app);
        up(&mut app);
        assert_eq!(field(&app), "run the tests", "the repeat is now the newest");
    }

    /// The finger is still holding shift from the press that opened the
    /// field, so the second Shift+Enter has to land somewhere — and sending
    /// is the only thing it could sanely mean.
    #[test]
    fn a_second_shift_enter_sends_the_prompt_too() {
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = true;
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        for c in "ship it".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert!(sent_contains(&sent, "ship it"), "{:?}", sent.borrow());
        assert_eq!(app.mode, Mode::Normal);
    }

    /// Esc leaves without asking anything, the way every other field here
    /// does — a prompt is a turn of somebody's conversation and must never
    /// be delivered by a key that means "never mind".
    #[test]
    fn esc_abandons_a_prompt_without_sending_it() {
        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = true;
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        for c in "oops".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert_eq!(app.mode, Mode::Normal);
        assert!(!sent_contains(&sent, "PromptSession"), "{:?}", sent.borrow());
    }

    /// A ticket saved with plain Enter is one press behind a Shift+Enter one:
    /// on the board, Shift+Enter over an empty claude seat starts claude on
    /// the title, submitted, and stays — no field opens, and nothing is
    /// attached. A shell on the ticket is not a claude, so the seat is still
    /// empty and the press still starts one.
    #[test]
    fn shift_enter_on_a_ticket_without_claude_starts_it_on_the_title() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        app.rich_keys = true;
        assert!(app.ctx().has_ticket);
        assert!(!app.ctx().ticket_has_claude);
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert_eq!(app.mode, Mode::Normal, "no field: the title is the prompt");
        assert!(sent_contains(&sent, "submit_prompt: true"), "{:?}", sent.borrow());
        assert!(!sent_contains(&sent, "PromptSession"), "{:?}", sent.borrow());
        assert!(!sent_contains(&sent, "CreateTicket"), "nothing minted: {:?}", sent.borrow());
        assert_eq!(app.screen, Screen::Board, "the board never leaves");
        assert!(app.pending_attach.is_none(), "no handover");

        let (mut app, sent, _) = app_with_shell(SessionState::Running);
        app.rich_keys = true;
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert_eq!(app.mode, Mode::Normal);
        assert!(sent_contains(&sent, "submit_prompt: true"), "{:?}", sent.borrow());
    }

    /// A parked agent has no box to type into. `Sleeping` is LIVE — that is
    /// the trap this holds shut: gating on "has a claude session" would offer
    /// the key here and send the press at a pane that does not exist.
    #[test]
    fn a_sleeping_agent_is_not_promptable() {
        let (mut app, sent, _) = app_with_claude(SessionState::Sleeping, false);
        app.rich_keys = true;
        assert!(app.ctx().ticket_has_claude, "the session is live — parked, but live");
        assert!(!app.ctx().ticket_promptable, "…and has no pane to type into");
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert_eq!(app.mode, Mode::Normal, "no field opens");
        assert!(!sent_contains(&sent, "PromptSession"));
    }

    /// A ticket holds one claude (2026-09-02). `c` on a parked one is a wake
    /// — the resume road, then the attach — and never a second spawn beside
    /// it; the hint says the same word. `c` on a ticket with none starts one.
    #[test]
    fn c_wakes_a_parked_claude_instead_of_starting_a_second() {
        let (mut app, sent, sid) = app_with_claude(SessionState::Sleeping, false);
        assert_eq!(
            keymap::hint_for(Scope::Board, Verb::Claude, &app.ctx()),
            Some(("c", "wake claude"))
        );
        press(&mut app, 'c');
        assert!(
            sent_contains(&sent, &format!("ResumeSession {{ id: {sid}")),
            "{:?}",
            sent.borrow()
        );
        assert!(!sent_contains(&sent, "SpawnSession"), "{:?}", sent.borrow());

        let (mut app, sent, _) = app_with_claude(SessionState::Running, false);
        assert_eq!(keymap::hint_for(Scope::Board, Verb::Claude, &app.ctx()), Some(("c", "claude")));
        press(&mut app, 'c');
        assert!(sent_contains(&sent, "FocusStart"), "{:?}", sent.borrow());
        assert!(!sent_contains(&sent, "SpawnSession"), "{:?}", sent.borrow());
    }

    /// The legacy floor. A terminal that cannot spell Shift+Enter sends a
    /// plain Enter, which on the board means "go to the agent" — so the key
    /// must resolve to nothing here rather than half-working.
    #[test]
    fn without_rich_keys_the_prompt_key_is_the_plain_enter_it_arrives_as() {
        let (mut app, _sent, _) = app_with_claude(SessionState::Running, false);
        app.rich_keys = false;
        assert!(app.ctx().ticket_promptable);
        assert_eq!(
            mesimon_core::keymap::resolve(
                mesimon_core::keymap::Scope::Board,
                mesimon_core::keymap::Key::ShiftEnter,
                &app.ctx()
            ),
            None
        );
    }

    /// Shift+Enter is the whole gesture in one press: the ticket exists, an
    /// agent is on it, the title has been ASKED (not just typed), and the
    /// board never went away.
    #[test]
    fn shift_enter_composes_and_starts_without_leaving_the_board() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        app.rich_keys = true;
        press(&mut app, 'o');
        press(&mut app, 'n');
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert!(sent_contains(&sent, "CreateTicket"));
        assert!(
            sent_contains(&sent, "submit_prompt: true"),
            "the title is submitted, not merely prefilled: {:?}",
            sent.borrow()
        );
        assert_eq!(app.screen, Screen::Board, "the board never leaves");
        assert!(app.pending_attach.is_none(), "no handover — that is the point");
        assert_eq!(app.mode, Mode::Normal, "the composer closed");
        // The agent is already running, so the fresh-ticket Enter window must
        // NOT be armed — the next Enter means what it always means.
        assert_eq!(app.just_created, None);
    }

    /// Same gesture on a terminal that cannot spell the key: crossterm reports
    /// a plain Enter, and a plain Enter is exactly what the user gets — the
    /// ticket, the armed fast path, no surprise agent.
    #[test]
    fn shift_enter_degrades_to_plain_save_without_rich_keys() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        assert!(!app.rich_keys);
        press(&mut app, 'o');
        press(&mut app, 'n');
        // The atom still arrives (this test presses it); the keymap refuses it.
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert!(!sent_contains(&sent, "CreateTicket"), "the key is inert, not half-bound");
        assert!(matches!(app.mode, Mode::Input { .. }), "still composing");
    }

    fn ctrl(app: &mut App, c: char) {
        app.handle_key(KeyCode::Char(c), KeyModifiers::CONTROL).unwrap();
    }

    /// The picker opens on the first cell, and on a board with no tags that
    /// cell is `+ new` — so the very first thing Enter does is make one.
    /// Nothing is seeded, and there is no setup step before that works.
    #[test]
    fn the_picker_starts_on_new_when_nothing_is_registered() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        assert_eq!(app.scope(), Scope::TagChord);
        assert!(app.tag_cell().is_none(), "the only cell is `+ new`");

        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(
            app.tag_armed.as_ref().expect("armed").naming.is_some(),
            "enter on `+ new` opens the name field"
        );
        // Digits and hjkl are TEXT while naming, not picker keys.
        for c in "b1u2g".chars() {
            press(&mut app, c);
        }
        assert_eq!(
            app.tag_armed
                .as_ref()
                .and_then(|a| a.naming.as_ref())
                .map(|(_, b)| b.as_str().to_string()),
            Some("b1u2g".to_string()),
            "the field owns every printable key"
        );
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "RegisterTag"), "the name is registered");
        assert!(sent_contains(&sent, "b1u2g"));
    }

    /// Enter on a tag wears it; Enter again takes it off. Creating and
    /// wearing are separate gestures, so registering never tags anything.
    #[test]
    fn enter_toggles_the_tag_under_the_cursor() {
        let mut board = board_three_columns();
        board.register_tag(1, "BUG").expect("registered");
        let id = board.tickets[0].id;
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        app.cursor_col = 0;
        app.cursor_row = 0;
        assert_eq!(app.subject(), Some(id));

        ctrl(&mut app, 't');
        assert_eq!(app.tag_cell().map(|(g, n, _)| (g, n)), Some((1, "BUG".to_string())));
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "SetTag"));
        assert!(sent_contains(&sent, "Some(\"BUG\")"), "{:?}", sent.borrow());

        // Wearing it, Enter clears the axis.
        app.board.ticket_mut(id).expect("ticket").set_tag(1, Some("BUG".into()));
        sent.borrow_mut().clear();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent.borrow().join(" ").contains("name: None"), "{:?}", sent.borrow());
    }

    /// The terminal's late answer to the colour query, fed the way `tick`
    /// feeds it: `alt+]` `1` `1` `;` `rgb:…` `alt+\`. The `1`s are the
    /// quick-tag key and the `r` is rename, and neither may fire — and the
    /// machine is idle again afterwards, so the next real `r` does.
    #[test]
    fn a_late_colour_reply_neither_tags_nor_renames() {
        let mut board = board_three_columns();
        board.register_tag(1, "BUG").expect("registered");
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        app.cursor_col = 0;
        app.cursor_row = 0;
        app.on_key(KeyCode::Char(']'), KeyModifiers::ALT).unwrap();
        for c in "11;rgb:1e1e/1e1e/1e1e".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE).unwrap();
        }
        app.on_key(KeyCode::Char('\\'), KeyModifiers::ALT).unwrap();
        assert!(!sent_contains(&sent, "SetTag"), "{:?}", sent.borrow());
        assert!(!matches!(app.mode, Mode::Input { .. }), "rename opened");
        assert!(app.tag_flash.is_none());

        assert!(app.on_key(KeyCode::Char('r'), KeyModifiers::NONE).unwrap());
        assert!(matches!(app.mode, Mode::Input { .. }), "a real r still renames");
    }

    /// The same reply landing INSIDE a text field — the composer is open —
    /// types nothing into it, since the swallow sits ahead of the barrier
    /// that strips Alt.
    #[test]
    fn a_late_colour_reply_types_nothing_into_an_open_field() {
        let mut app = app_three_columns();
        press(&mut app, 'o');
        press(&mut app, 'x');
        app.on_key(KeyCode::Char(']'), KeyModifiers::ALT).unwrap();
        for c in "11;rgb:1e1e/1e1e/1e1e".chars() {
            app.on_key(KeyCode::Char(c), KeyModifiers::NONE).unwrap();
        }
        app.on_key(KeyCode::Char('\\'), KeyModifiers::ALT).unwrap();
        let Mode::Input { buffer, .. } = &app.mode else { panic!("composer closed") };
        assert_eq!(buffer.as_str(), "x");
    }

    /// hjkl walks the grid and a digit jumps to that group's row, stepping
    /// along it when pressed again — one finger reaches every tag on an axis.
    #[test]
    fn the_cursor_walks_and_digits_jump() {
        let mut board = board_three_columns();
        for n in ["BUG", "REGR", "FTR"] {
            board.register_tag(1, n).expect("registered");
        }
        board.register_tag(2, "DEV").expect("registered");
        let (mut app, _sent) = App::for_test_logged(board, theme(), false);
        ctrl(&mut app, 't');

        let name = |a: &App| a.tag_cell().map(|(_, n, _)| n);
        assert_eq!(name(&app).as_deref(), Some("BUG"));
        press(&mut app, 'l');
        assert_eq!(name(&app).as_deref(), Some("REGR"));
        press(&mut app, 'h');
        assert_eq!(name(&app).as_deref(), Some("BUG"));
        // Down into a shorter row: the column trims to what that row
        // actually reaches instead of pointing past its end.
        press(&mut app, 'l');
        press(&mut app, 'l');
        assert_eq!(app.tag_armed.as_ref().expect("armed").col, 2);
        press(&mut app, 'j');
        let arm = app.tag_armed.as_ref().expect("armed");
        assert_eq!(arm.row, 1);
        assert_eq!(arm.col, 1, "clamped to the last cell of the shorter row (`+ new`)");
        press(&mut app, 'h');
        assert_eq!(name(&app).as_deref(), Some("DEV"));

        // A digit jumps; the same digit again steps along, and the row
        // wraps rather than going dead on its last cell.
        press(&mut app, '1');
        assert_eq!(name(&app).as_deref(), Some("BUG"));
        press(&mut app, '1');
        assert_eq!(name(&app).as_deref(), Some("REGR"));
        press(&mut app, '1');
        assert_eq!(name(&app).as_deref(), Some("FTR"));
        press(&mut app, '1');
        assert_eq!(name(&app), None, "the `+ new` cell is the row's last stop");
        press(&mut app, '1');
        assert_eq!(name(&app).as_deref(), Some("BUG"), "and then round again");
        press(&mut app, '2');
        assert_eq!(name(&app).as_deref(), Some("DEV"));
        // The empty spare row holds one cell and cycles onto itself.
        press(&mut app, '3');
        press(&mut app, '3');
        let arm = app.tag_armed.as_ref().expect("armed");
        assert_eq!((arm.row, arm.col), (2, 0));
    }

    /// A quick-tag digit opens the card it tagged, and then lets go. The
    /// stripe is one cell and carries no words, so the press that changes it
    /// opens the row that names it — for a moment, and only on that card.
    #[test]
    fn a_quick_tag_flashes_its_card_open() {
        let mut board = board_three_columns();
        board.register_tag(1, "BUG").expect("registered");
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        app.cursor_col = 0;
        app.cursor_row = 0;
        let id = app.subject().expect("a ticket under the cursor");
        assert!(!app.peek_showing(id), "nothing is open before the press");

        press(&mut app, '1');
        assert!(sent_contains(&sent, "SetTag"), "the digit tagged it");
        assert!(app.peek_showing(id), "and opened the card it tagged");
        assert!(!app.peek_showing(ulid::Ulid(2)), "the reveal is one card's, never the board's");

        // A moment, not a mode: the reveal expires on its own clock, and the
        // `p` preference — which is what the footer describes — is untouched.
        app.tag_flash = Some((id, Instant::now() - TAG_FLASH));
        assert!(!app.peek_showing(id));
        assert!(!app.peek, "the flash never sets the preference");
        assert!(!app.ctx().peek_on, "so `p` still offers to show replies");

        // An axis with no vocabulary changed nothing, so it reveals nothing.
        press(&mut app, '4');
        assert!(!app.peek_showing(id));
        assert_eq!(app.status, "no tags in group 4 ∙ ^t makes one");
    }

    /// The board's nudge, in the picker's grid. Along the row it is order —
    /// which is what the row draws and what a repeated digit walks. Across
    /// rows it is the axis itself, the one repair a tag created on the wrong
    /// one has ever had (`d` is the other way off an axis, and it takes the
    /// tag off every ticket on the way).
    #[test]
    fn a_tag_is_carried_along_its_axis_and_onto_another() {
        let mut board = board_three_columns();
        for n in ["A", "B", "C"] {
            board.register_tag(1, n).expect("registered");
        }
        board.register_tag(2, "DEV").expect("registered");
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        ctrl(&mut app, 't');
        assert_eq!(app.tag_cell().map(|(_, n, _)| n).as_deref(), Some("A"));
        let moves = |sent: &std::cell::RefCell<Vec<String>>| -> Vec<String> {
            sent.borrow().iter().filter(|c| c.contains("MoveTag")).cloned().collect()
        };

        // The left edge of the row: nowhere to go, and nothing said about it.
        sent.borrow_mut().clear();
        press(&mut app, 'H');
        alt(&mut app, KeyCode::Left);
        assert!(moves(&sent).is_empty(), "{:?}", sent.borrow());
        assert!(app.tag_armed.is_some(), "a nudge with nowhere to go is inert, not a dismissal");

        // Along the axis, both spellings, one command.
        press(&mut app, 'L');
        alt(&mut app, KeyCode::Char('l'));
        let log = moves(&sent);
        assert_eq!(log.len(), 2, "{log:?}");
        for line in &log {
            assert!(line.contains("to_group: 1") && line.contains("to_index: 1"), "{line}");
        }

        // Down onto the next axis: the tag JOINS that row, at its end.
        sent.borrow_mut().clear();
        press(&mut app, 'J');
        let log = moves(&sent).join(" ");
        assert!(log.contains("to_group: 2") && log.contains("to_index: 1"), "{log}");
        // And up from the top row is the same nowhere as the left edge.
        sent.borrow_mut().clear();
        press(&mut app, 'K');
        assert!(moves(&sent).is_empty(), "{:?}", sent.borrow());
    }

    /// The accelerator is inert where the move is unavailable, on BOTH kinds
    /// of terminal: the one that sends the Alt atom, and the one that eats
    /// the modifier and composes a character instead. Neither may dismiss the
    /// panel, or `alt+h` would cost the user their place on one machine and
    /// nothing on the next.
    #[test]
    fn an_eaten_option_key_leaves_the_picker_standing() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        assert!(app.tag_cell().is_none(), "the only cell is `+ new` — nothing to carry");
        for code in [KeyCode::Char('h'), KeyCode::Char('j'), KeyCode::Left] {
            alt(&mut app, code);
            assert!(app.tag_armed.is_some(), "{code:?} closed the picker");
        }
        // What macOS Terminal sends for `⌥h` instead of a modifier at all.
        press(&mut app, '\u{2d9}');
        assert!(app.tag_armed.is_some(), "a composed character is not a stray key");
        assert!(!sent_contains(&sent, "MoveTag"), "{:?}", sent.borrow());
    }

    /// A name field types its letters, shifted ones included — the whole
    /// table stands down while `tag_naming`, and Alt goes back to meaning
    /// "by word" there.
    #[test]
    fn a_shifted_letter_is_text_inside_a_name_field() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        for c in "HJKL".chars() {
            press(&mut app, c);
        }
        assert_eq!(
            app.tag_armed
                .as_ref()
                .and_then(|a| a.naming.as_ref())
                .map(|(_, b)| b.as_str().to_string()),
            Some("HJKL".to_string()),
        );
        assert!(!sent_contains(&sent, "MoveTag"), "{:?}", sent.borrow());
    }

    /// Tab cycles the tint, and it is a registry property — so it repaints
    /// every card wearing that tag, not just this one.
    #[test]
    fn tab_cycles_the_colour() {
        let mut board = board_three_columns();
        board.register_tag(1, "BUG").expect("registered");
        let was = board.tag_def(1, "BUG").expect("registered").tint();
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        ctrl(&mut app, 't');
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE).unwrap();
        let log = sent.borrow().join(" ");
        assert!(log.contains("SetTagColor"), "{log}");
        let next = (was + 1) % mesimon_core::board::TAG_TINTS;
        assert!(log.contains(&format!("color: {next}")), "{log}");
    }

    /// `r` opens the field pre-filled, so a rename is an edit and not a
    /// retype.
    #[test]
    fn rename_starts_from_the_current_name() {
        let mut board = board_three_columns();
        board.register_tag(1, "BUG").expect("registered");
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        ctrl(&mut app, 't');
        press(&mut app, 'r');
        assert_eq!(
            app.tag_armed
                .as_ref()
                .and_then(|a| a.naming.as_ref())
                .map(|(_, b)| b.as_str().to_string()),
            Some("BUG".to_string())
        );
        for c in "S".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "RenameTag"));
        assert!(sent_contains(&sent, "BUGS"));
    }

    /// Esc backs out of the field first and closes the picker second: losing
    /// a half-typed name AND the panel to one key is a gesture nobody means.
    #[test]
    fn esc_leaves_the_field_before_the_picker() {
        let (mut app, _sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(app.tag_armed.as_ref().expect("armed").naming.is_some());
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(app.tag_armed.is_some(), "still in the picker");
        assert!(app.tag_armed.as_ref().expect("armed").naming.is_none(), "out of the field");
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(app.tag_armed.is_none(), "picker closed");
    }

    /// A rename has nothing to start, so the key stays out of its way.
    #[test]
    fn shift_enter_does_nothing_when_renaming() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        app.rich_keys = true;
        press(&mut app, 'r');
        press(&mut app, 'x');
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert!(!sent_contains(&sent, "RenameTicket"));
        assert!(!sent_contains(&sent, "SpawnSession"));
        assert!(matches!(app.mode, Mode::Input { .. }));
    }

    #[test]
    fn any_other_key_closes_the_compose_fast_path() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        press(&mut app, 'o');
        press(&mut app, 'n');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        press(&mut app, 'j'); // browsing away — the moment passed
        assert_eq!(app.just_created, None);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(!sent_contains(&sent, "SpawnSession"));
        assert!(matches!(app.screen, Screen::Ticket { .. }));
    }

    #[test]
    fn enter_with_running_claude_focuses_directly() {
        let (mut app, sent, _sid) = app_with_claude(SessionState::Running, false);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "FocusStart"));
        assert!(app.pending_attach.is_some());
        assert_eq!(app.screen, Screen::Board, "no ticket-screen detour");
    }

    #[test]
    fn enter_with_needs_you_claude_focuses_directly() {
        // A waiting permission prompt is exactly where Enter should land.
        let state =
            SessionState::RequiresAction { reason: mesimon_core::board::Reason::Permission };
        let (mut app, sent, _sid) = app_with_claude(state, false);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "FocusStart"));
        assert_eq!(app.screen, Screen::Board);
    }

    #[test]
    fn enter_fast_paths_only_a_hot_claude() {
        // Neither running nor needs-you — idle, sleeping — opens the ticket
        // page, where the state is visible before entering the pane.
        for state in [
            SessionState::Sleeping,
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn },
        ] {
            let (mut app, sent, _sid) = app_with_claude(state.clone(), false);
            app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
            assert!(!sent_contains(&sent, "FocusStart"), "{state:?} must not focus");
            assert_eq!(app.screen, Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 });
        }
    }

    /// Space always opens the ticket page, even when Enter would have gone
    /// straight to a live agent. It is the only spelling — 04 §2.0 bans
    /// shift+Enter as an atom, and Enter already means "get me working".
    #[test]
    fn space_opens_the_ticket_page_past_a_live_agent() {
        let (mut app, sent, _sid) = app_with_claude(SessionState::Running, false);
        app.handle_key(KeyCode::Char(' '), KeyModifiers::NONE).unwrap();
        assert!(!sent_contains(&sent, "FocusStart"));
        assert_eq!(app.screen, Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 });
    }

    #[test]
    fn refused_focus_falls_open_to_the_ticket_screen() {
        let (mut app, _sent, _sid) = app_with_claude(SessionState::Running, true);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(app.pending_attach.is_none());
        assert_eq!(app.screen, Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 });
        assert!(!app.status.is_empty(), "the refusal shows itself");
    }

    #[test]
    fn unfocus_returns_to_the_focus_origin() {
        // Board-born focus lands back on the board…
        let (mut app, _sent, sid) = app_with_claude(SessionState::Running, false);
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        app.focused_session_hint = Some((sid, FocusOrigin::Board));
        app.after_handover().unwrap();
        assert_eq!(app.screen, Screen::Board);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
        // …ticket-born focus lands back on the ticket screen.
        let (mut app, _sent, sid) = app_with_claude(SessionState::Running, false);
        app.focused_session_hint = Some((sid, FocusOrigin::Ticket));
        app.after_handover().unwrap();
        assert_eq!(app.screen, Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 });
    }

    #[test]
    fn merge_replies_land_in_the_note_never_the_status() {
        // The m flow talks through the identity line's merge_note; the footer
        // (status) stays silent so the hint never doubles top + bottom.
        let mut app = app_three_columns();
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        press(&mut app, 'm'); // no worktree binding in the fixture
        assert_eq!(app.merge_note, "no worktree on this ticket");
        assert!(app.status.is_empty());
    }

    #[test]
    fn merge_refuses_up_front_while_the_agent_works() {
        // Quiet-tickets rule in the TUI: a mid-turn agent means the first m
        // refuses outright — never an armed confirm the daemon would bounce.
        let (mut app, sent, _sid) = app_with_claude(SessionState::Running, false);
        app.worktrees.push(WorktreeItem {
            ticket: ulid::Ulid(1),
            branch: "msmn/T-1-work".into(),
            status: "attached".into(),
            merged: false,
            conflict: false,
            ahead: 2,
            needs_rebase: false,
            detail: None,
            path: None,
        });
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        press(&mut app, 'm');
        assert_eq!(app.merge_note, "agent still working — wait for it to finish");
        assert!(app.merge_armed.is_none(), "the flow never arms under a working agent");
        press(&mut app, 'm');
        assert!(!sent_contains(&sent, "MergeTicket"));
        // An idle agent lifts the gate: the first m arms as usual.
        app.board.sessions[0].state =
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
        press(&mut app, 'm');
        assert!(app.merge_armed.is_some());
    }

    #[test]
    fn merged_note_arms_notify_so_one_m_delivers() {
        // The post-merge note promises "m tells the agent" — that press must
        // notify, not re-arm a confirm the note already gave (author 2026-08-30).
        let (mut app, sent, _sid) = app_with_claude(
            SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn },
            false,
        );
        let wt = |merged, ahead| WorktreeItem {
            ticket: ulid::Ulid(1),
            branch: "msmn/T-1-work".into(),
            status: "attached".into(),
            merged,
            conflict: false,
            ahead,
            needs_rebase: false,
            detail: None,
            path: None,
        };
        app.worktrees.push(wt(false, 2));
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        press(&mut app, 'm'); // arms the merge confirm
        press(&mut app, 'm'); // ff merge lands
        assert!(app.merge_note.ends_with("∙ m tells the agent"));
        assert_eq!(app.merge_armed, Some((ulid::Ulid(1), MergeStage::Notify)));
        // The refresh's snapshot now carries the merged binding (the fake
        // transport returns none, so restore it by hand).
        app.worktrees.push(wt(true, 0));
        press(&mut app, 'm');
        assert!(sent_contains(&sent, "MergedNotice"), "one m after the merge notifies");
        assert_eq!(app.merge_note, "agent notified");
    }

    #[test]
    fn single_grab_shifts_ghost_one_column_pending() {
        let mut app = app_three_columns();
        press(&mut app, '>');
        // Ghost already sits one column over; nothing sent yet.
        assert!(matches!(app.mode, Mode::Move { col: 1, idx: 0, grab: '>', .. }));
        assert_eq!(app.board.column_tickets("todo").len(), 2);
        assert!(app.board.column_tickets("doing").is_empty());
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.board.column_tickets("todo").len(), 2);
    }

    #[test]
    fn opposite_key_cancels_pending_move() {
        let mut app = app_three_columns();
        press(&mut app, '>');
        press(&mut app, '<');
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.board.column_tickets("todo").len(), 2);
        assert!(app.board.column_tickets("doing").is_empty());
        press(&mut app, '<');
        press(&mut app, '>');
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.board.column_tickets("todo").len(), 2);
    }

    #[test]
    fn double_gt_moves_one_column_right_and_follows() {
        let mut app = app_three_columns();
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
        press(&mut app, '>');
        press(&mut app, '>');
        let doing: Vec<_> = app.board.column_tickets("doing").iter().map(|t| t.id).collect();
        assert_eq!(doing, vec![ulid::Ulid(1)]);
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.cursor_col, 1);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
    }

    #[test]
    fn double_grab_lands_on_top_of_the_next_column() {
        let mut app = app_three_columns();
        app.cursor_col = 2; // "done", holds ticket 3 at row 0
        press(&mut app, '<');
        press(&mut app, '<');
        // no wrap involved: done -> doing
        let doing: Vec<_> = app.board.column_tickets("doing").iter().map(|t| t.id).collect();
        assert_eq!(doing, vec![ulid::Ulid(3)]);
        // hop again into todo, which already has 1 and 2 — a foreign column
        // is entered at the top, before them
        press(&mut app, '<');
        press(&mut app, '<');
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(3), ulid::Ulid(1), ulid::Ulid(2)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 0));
    }

    fn alt(app: &mut App, code: KeyCode) {
        app.handle_key(code, KeyModifiers::ALT).unwrap();
    }

    /// `alt+<direction>` moves the CARD and takes the cursor with it — one
    /// press, no ghost, no Enter. Both spellings of a direction do it.
    #[test]
    fn alt_direction_moves_the_card_and_the_cursor_rides_along() {
        let mut app = app_three_columns();
        assert!(app.ctx().can_nudge);
        // Sideways: into the next column, at its top, cursor on it.
        alt(&mut app, KeyCode::Right);
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "doing");
        assert_eq!((app.cursor_col, app.cursor_row), (1, 0));
        // The letter spelling is the same atom.
        alt(&mut app, KeyCode::Char('l'));
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "done");
        assert_eq!((app.cursor_col, app.cursor_row), (2, 0));
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(
            done,
            vec![ulid::Ulid(1), ulid::Ulid(3)],
            "a foreign column takes it at the top"
        );
        // And back the way it came.
        alt(&mut app, KeyCode::Char('h'));
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "doing");
        assert_eq!((app.cursor_col, app.cursor_row), (1, 0));
    }

    /// Up and down reorder inside the column, and the cursor stays on the
    /// card it is carrying.
    #[test]
    fn alt_up_and_down_reorder_within_the_column() {
        let mut app = app_three_columns();
        alt(&mut app, KeyCode::Down);
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(2), ulid::Ulid(1)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 1));
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "todo");
        alt(&mut app, KeyCode::Char('k'));
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 0));
    }

    /// Every edge stays put rather than wrapping: a ghost you can cancel may
    /// wrap, a card that has already moved may not. And a reorder does not
    /// arm `.` — it filed nothing.
    #[test]
    fn alt_direction_stops_at_the_edges_and_arms_nothing() {
        let mut app = app_three_columns();
        alt(&mut app, KeyCode::Left);
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "todo");
        assert_eq!((app.cursor_col, app.cursor_row), (0, 0));
        alt(&mut app, KeyCode::Up);
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2)]);
        // Bottom of the column: down is inert too.
        alt(&mut app, KeyCode::Down);
        alt(&mut app, KeyCode::Down);
        assert_eq!(app.cursor_row, 1);
        assert!(!app.ctx().can_repeat, "a reorder is not a filing");
        // Right to the last column, then one more.
        app.cursor_col = 2;
        app.cursor_row = 0;
        alt(&mut app, KeyCode::Right);
        assert_eq!(app.board.ticket(ulid::Ulid(3)).unwrap().column, "done");
        assert_eq!(app.cursor_col, 2);
    }

    /// One column, one card: nowhere to send it, so the key is inert and the
    /// `?` overlay does not offer it.
    #[test]
    fn alt_direction_is_inert_with_nowhere_to_send_the_card() {
        let mut b = Board::default();
        b.columns.push(Column { name: "todo".into(), order: "0".into() });
        b.tickets.push(ticket(1, "todo", "a"));
        let mut app = App::for_test(b, theme());
        assert!(!app.ctx().can_nudge);
        alt(&mut app, KeyCode::Down);
        alt(&mut app, KeyCode::Right);
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "todo");
    }

    /// The composer keeps Alt as its "by word" modifier: `alt+←` walks a word
    /// in the title, it does not move a card.
    #[test]
    fn alt_in_the_composer_still_jumps_by_word() {
        let mut app = app_three_columns();
        press(&mut app, 'o');
        for c in "fix the thing".chars() {
            app.handle_key(KeyCode::Char(c), KeyModifiers::NONE).unwrap();
        }
        app.handle_key(KeyCode::Left, KeyModifiers::ALT).unwrap();
        app.handle_key(KeyCode::Char('X'), KeyModifiers::NONE).unwrap();
        let Mode::Input { buffer, .. } = &app.mode else { panic!("left the composer") };
        assert_eq!(buffer.as_str(), "fix the Xthing");
        // And the board did not move underneath it.
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "todo");
    }

    /// `.` does the last move again, and the cursor stays put — that is the
    /// whole reason the key exists. A drop follows the card; a repeat lets the
    /// next card slide up under the cursor, so `. . .` files three of them.
    #[test]
    fn dot_repeats_the_last_move_and_leaves_the_cursor_home() {
        let mut app = app_three_columns();
        // Ticket 1: grab, aim at column 3 ("done"), drop. The cursor follows.
        press(&mut app, '>');
        press(&mut app, '3');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!((app.cursor_col, app.cursor_row), (2, 0));
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "done");

        // Back to todo, on ticket 2 — the one press repeats the whole gesture.
        app.cursor_col = 0;
        app.cursor_row = 0;
        assert_eq!(app.ctx().repeat_word, "move again");
        assert!(app.ctx().can_repeat);
        press(&mut app, '.');
        assert_eq!(app.board.ticket(ulid::Ulid(2)).unwrap().column, "done");
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(2), ulid::Ulid(1), ulid::Ulid(3)]);
        // Still standing in the column we were filing from.
        assert_eq!(app.cursor_col, 0);
        assert!(app.board.column_tickets("todo").is_empty());
        assert_eq!(app.status, "moved to done");
    }

    /// A refused repeat reports the refusal, not the move. The daemon's DONE
    /// gate is the live case; the fake transport refuses the same shape.
    #[test]
    fn a_refused_repeat_keeps_the_refusal_on_screen() {
        let mut b = board_three_columns();
        b.columns.push(Column { name: "DONE".into(), order: "3".into() });
        // Ticket 2 wants a worktree, which is what the fake reads as unmerged.
        if let Some(t) = b.tickets.iter_mut().find(|t| t.id == ulid::Ulid(2)) {
            t.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
        }
        let mut app = App::for_test(b, theme());
        // Ticket 1 has no worktree, so it files into DONE and arms `.`.
        press(&mut app, '>');
        press(&mut app, '4');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "DONE");
        app.cursor_col = 0;
        app.cursor_row = 0;
        assert!(app.ctx().can_repeat);
        press(&mut app, '.');
        assert_eq!(app.board.ticket(ulid::Ulid(2)).unwrap().column, "todo");
        assert_eq!(app.status, "worktree unmerged — merge before DONE");
    }

    /// The other half: the key is inert AND unhinted where repeating means
    /// nothing — before any move, and on a card already in the target column.
    #[test]
    fn dot_is_inert_before_a_move_and_on_a_card_already_there() {
        let mut app = app_three_columns();
        assert!(!app.ctx().can_repeat);
        assert_eq!(app.ctx().repeat_word, "again");
        press(&mut app, '.');
        assert_eq!(app.board.ticket(ulid::Ulid(1)).unwrap().column, "todo");

        press(&mut app, '>');
        press(&mut app, '3');
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        // The cursor followed the card into "done": repeating here would be a
        // shuffle inside one column, not the same action again.
        assert!(!app.ctx().can_repeat);
        press(&mut app, '.');
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(1), ulid::Ulid(3)], "a no-op, not a reorder");

        // A column that went away takes the offer down with it.
        app.cursor_col = 0;
        assert!(app.ctx().can_repeat);
        app.board.columns.retain(|c| c.name != "done");
        assert!(!app.ctx().can_repeat);
    }

    #[test]
    fn move_enters_a_foreign_column_at_the_top() {
        let mut b = board_three_columns();
        b.tickets.push(ticket(4, "doing", "a"));
        b.tickets.push(ticket(5, "doing", "b"));
        let mut app = App::for_test(b, theme());
        app.cursor_row = 1; // ticket 2, second in todo
        press(&mut app, '>');
        // Top of doing, NOT the grabbed row — height is adjusted by hand.
        assert!(matches!(app.mode, Mode::Move { col: 1, idx: 0, .. }));
    }

    #[test]
    fn move_back_home_restores_the_original_height() {
        let mut b = board_three_columns();
        b.tickets.push(ticket(4, "doing", "a"));
        b.tickets.push(ticket(5, "doing", "b"));
        let mut app = App::for_test(b, theme());
        app.cursor_row = 1; // ticket 2, second in todo
        press(&mut app, '>');
        press(&mut app, 'j'); // adjusting abroad must not disturb the memory
        press(&mut app, 'h'); // back home
        assert!(matches!(app.mode, Mode::Move { col: 0, idx: 1, .. }));
        // Dropping home is a perfect no-op move.
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 1));
    }

    /// The in-column reorder: out with `>`, home with `h`, up a row, drop.
    /// The daemon used to answer this `Ok` and change nothing, because the
    /// column it lands in is the one it left.
    #[test]
    fn move_home_and_up_a_row_reorders_the_column() {
        let mut app = app_three_columns();
        app.cursor_row = 1; // ticket 2, second in todo
        press(&mut app, '>');
        press(&mut app, 'h'); // back home, at its own row
        press(&mut app, 'k'); // one row up: above ticket 1
        assert!(matches!(app.mode, Mode::Move { col: 0, idx: 0, .. }));
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(2), ulid::Ulid(1)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 0));
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(2)));
    }

    #[test]
    fn ctrl_bracket_pops_the_ticket_screen_to_board() {
        // Ctrl+] is the tmux detach key — right after an unfocus it keeps
        // popping outward. Both encodings: kitty (']') and legacy 0x1D ('5').
        let mut app = app_three_columns();
        for key in [']', '5'] {
            app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
            app.handle_key(KeyCode::Char(key), KeyModifiers::CONTROL).unwrap();
            assert_eq!(app.screen, Screen::Board);
        }
    }

    #[test]
    fn double_gt_cycles_off_last_column_to_first() {
        let mut app = app_three_columns();
        app.cursor_col = 2; // "done", ticket 3
        press(&mut app, '>');
        press(&mut app, '>');
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(3), ulid::Ulid(1), ulid::Ulid(2)]);
        assert_eq!(app.cursor_col, 0);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(3)));
    }

    #[test]
    fn double_lt_cycles_off_first_column_to_last() {
        let mut app = app_three_columns();
        press(&mut app, '<');
        press(&mut app, '<');
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(1), ulid::Ulid(3)]);
        assert_eq!(app.cursor_col, 2);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
    }

    #[test]
    fn grab_then_fine_placement_still_drops_on_enter() {
        let mut app = app_three_columns();
        press(&mut app, '>'); // grab ticket 1 — ghost lands in doing
        press(&mut app, 'l'); // ghost to done (holds 3)
        press(&mut app, 'j'); // below 3
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(3), ulid::Ulid(1)]);
        assert_eq!((app.cursor_col, app.cursor_row), (2, 1));
    }

    /// `m` is the merge key now, everywhere. It never grabs a card, and it
    /// never means two things depending on which screen you are on.
    #[test]
    fn m_is_not_a_board_key() {
        let mut app = app_three_columns();
        press(&mut app, 'm');
        assert!(matches!(app.mode, Mode::Normal), "m must not grab on the board");
        assert!(app.board.column_tickets("todo").len() == 2);
    }

    /// Digits address columns while a card is held (04 §2.5) — MOVE is the one
    /// scope where bare digits mean anything.
    #[test]
    fn digits_address_columns_while_holding_a_card() {
        let mut app = app_three_columns();
        press(&mut app, '>'); // grab ticket 1, ghost into doing
        press(&mut app, '3'); // straight to the third column
        assert!(matches!(app.mode, Mode::Move { col: 2, .. }));
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(1), ulid::Ulid(3)]);
    }

    #[test]
    fn grab_on_empty_column_is_a_noop() {
        let mut app = app_three_columns();
        app.cursor_col = 1; // "doing" is empty
        press(&mut app, '>');
        press(&mut app, '>');
        assert!(matches!(app.mode, Mode::Normal));
        assert!(app.board.column_tickets("doing").is_empty());
        assert_eq!(app.board.column_tickets("todo").len(), 2);
    }

    #[test]
    fn archive_key_removes_ticket_from_board() {
        let mut app = app_three_columns();
        archive(&mut app); // ticket 1 selected, no sessions — gate passes
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(2)]);
        assert_eq!(app.board.archived_tickets().len(), 1);
        assert!(app.status.starts_with("archived T-1"));
        // Cursor clamped onto the surviving row.
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(2)));
    }

    #[test]
    fn archive_refused_while_sessions_awake() {
        let mut app = app_three_columns();
        app.board.sessions.push(mesimon_core::board::SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            SessionState::Running,
        ));
        press(&mut app, 'a');
        assert_eq!(app.status, "its sessions are awake — sleep them first (x)");
        assert!(app.archive_armed.is_none(), "never arm a confirm we would only refuse");
        // Advisory fired client-side; nothing was sent, nothing archived.
        assert_eq!(app.board.column_tickets("todo").len(), 2);
        assert!(app.board.archived_tickets().is_empty());
    }

    #[test]
    fn archived_dialog_restores_to_same_column() {
        let mut app = app_three_columns();
        app.cursor_col = 2; // "done", ticket 3
        archive(&mut app);
        assert!(app.board.column_tickets("done").is_empty());
        open_archived(&mut app);
        assert!(matches!(app.mode, Mode::Archived { idx: 0 }));
        press(&mut app, 'a'); // restore
                              // Restoring the last archived ticket closes the dialog on refresh.
        assert!(matches!(app.mode, Mode::Normal));
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(3)]);
        assert!(app.status.starts_with("restored to done"));
    }

    /// The menu only ever offers rows that apply: with nothing archived there
    /// is no "archived tickets" row to land on.
    #[test]
    fn menu_hides_the_archived_row_when_nothing_is_archived() {
        let mut app = app_three_columns();
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Menu { idx: 0 }));
        let verbs: Vec<Verb> = keymap::menu_items(&app.ctx()).iter().map(|m| m.verb).collect();
        assert!(!verbs.contains(&Verb::ArchivedList));
        assert!(verbs.contains(&Verb::ExternalDrawer), "{verbs:?}");
    }

    /// Arrows move the menu selection — the letter motions and their aliases
    /// go through the same dispatch, so this drives the real key path rather
    /// than the resolver.
    #[test]
    fn arrows_move_the_menu_selection() {
        let mut app = app_three_columns();
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Menu { idx: 0 }));
        app.handle_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Menu { idx: 1 }), "↓ must move the menu");
        app.handle_key(KeyCode::Up, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Menu { idx: 0 }), "↑ must move the menu");
    }

    /// And in every other list, for the same reason.
    #[test]
    fn arrows_move_every_list() {
        let mut app = app_three_columns();
        app.handle_key(KeyCode::Down, KeyModifiers::NONE).unwrap();
        assert_eq!(app.cursor_row, 1, "board ↓");
        app.handle_key(KeyCode::Right, KeyModifiers::NONE).unwrap();
        assert_eq!(app.cursor_col, 1, "board →");
        app.handle_key(KeyCode::Left, KeyModifiers::NONE).unwrap();
        assert_eq!(app.cursor_col, 0, "board ←");
        app.handle_key(KeyCode::Up, KeyModifiers::NONE).unwrap();
        assert_eq!(app.cursor_row, 0, "board ↑");
        // …and while holding a card.
        press(&mut app, '>');
        app.handle_key(KeyCode::Left, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Move { col: 0, .. }), "move ←");
    }

    /// Esc opens the menu, and picking a row runs its verb.
    #[test]
    fn menu_opens_on_esc_and_runs_the_chosen_row() {
        let mut app = app_three_columns();
        archive(&mut app); // archive ticket 1 so the row exists
        open_archived(&mut app);
        assert!(matches!(app.mode, Mode::Archived { idx: 0 }));
    }

    /// `a` alone never archives, and a stray key after it cancels cleanly.
    #[test]
    fn archive_needs_the_second_press() {
        let mut app = app_three_columns();
        press(&mut app, 'a');
        assert!(app.archive_armed.is_some(), "the first a arms");
        assert!(app.board.archived_tickets().is_empty(), "the first a must not archive");
        press(&mut app, 'j'); // anything else
        assert!(app.archive_armed.is_none());
        assert_eq!(app.status, "archive cancelled");
        assert!(app.board.archived_tickets().is_empty());
        // And the full chord does archive.
        app.cursor_row = 0;
        archive(&mut app);
        assert_eq!(app.board.archived_tickets().len(), 1);
    }

    /// `u` undoes an archive, and says so before you press it.
    #[test]
    fn u_undoes_an_archive() {
        let mut app = app_three_columns();
        archive(&mut app);
        assert_eq!(app.board.archived_tickets().len(), 1);
        assert!(app.ctx().can_undo);
        assert_eq!(app.ctx().undo_word, "undo archive");
        press(&mut app, 'u');
        assert!(app.board.archived_tickets().is_empty(), "u put it back");
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2)]);
        // Nothing left to undo, so the key goes quiet again.
        assert!(!app.ctx().can_undo);
        assert_eq!(keymap::hint_for(Scope::Board, Verb::Undo, &app.ctx()), None);
    }

    /// The archive undo is re-derived from the board, never trusted: a ticket
    /// restored from another client stops being `u`'s target.
    #[test]
    fn archive_undo_expires_when_the_ticket_comes_back() {
        let mut app = app_three_columns();
        archive(&mut app);
        assert!(app.ctx().can_undo);
        app.unarchive(ulid::Ulid(1)).unwrap();
        assert!(!app.ctx().can_undo, "nothing archived any more");
    }

    #[test]
    fn archived_dialog_enter_opens_ticket_screen() {
        let mut app = app_three_columns();
        archive(&mut app);
        open_archived(&mut app);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(matches!(app.mode, Mode::Normal));
        assert!(matches!(app.screen, Screen::Ticket { ticket, .. } if ticket == ulid::Ulid(1)));
    }

    #[test]
    fn ticket_page_a_toggles_archive() {
        let mut app = app_three_columns();
        app.screen = Screen::Ticket { ticket: ulid::Ulid(1), rail_idx: 0 };
        archive(&mut app);
        // The page keeps showing the archived ticket (still in the snapshot).
        assert!(matches!(app.screen, Screen::Ticket { .. }));
        assert!(app.board.ticket(ulid::Ulid(1)).unwrap().is_archived());
        press(&mut app, 'a'); // one press to restore — not a chord
        assert!(!app.board.ticket(ulid::Ulid(1)).unwrap().is_archived());
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2)]);
    }
}
