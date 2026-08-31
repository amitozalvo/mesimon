//! Board app: state, keymap (M1 subset — the full 04 keymap lands in M2/M6),
//! MOVE mode with client-side ghost (07 §7 core rules), focus flow with GATE,
//! and the M3.5 ticket screen (Enter opens it; the old session picker is its
//! SESSIONS rail now).

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use mesimon_core::board::{
    sanitize_tag, Board, ExitReason, Provenance, SessionKind, SessionState, Tag, Ticket,
    WorkspaceStrategy,
};
use mesimon_core::command::{
    Command, ExternalItem, GraceItem, MergeOutcome, Resources, Response, WorktreeItem,
};
use mesimon_core::keymap::{self, Ctx, Key, Scope, Verb};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::client::Transport;
use crate::text::EditBuffer;
use crate::theme::Theme;

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
}

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
        tags: Vec<Tag>,
    },
    Rename {
        id: ulid::Ulid,
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
    /// The axis the digits last pointed at.
    pub group: Option<u8>,
    /// Typing a new tag name.
    pub naming: Option<EditBuffer>,
}

/// `{`/`}` (and PgUp/PgDn) hunk-pane page step. The key handler cannot see
/// the rendered height, so this approximates a screenful; draw clamps.
const DIFF_PAGE: usize = 20;

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
    /// New-binary watch (dev rebuild or prod upgrade — same signal).
    update_watch: crate::update::UpdateWatch,
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
    /// Daemon connection lost: keep the last board, re-dial on a slow cadence.
    daemon_down: bool,
    last_reconnect: Option<Instant>,
}

impl App {
    pub fn new(mut client: Box<dyn Transport>, repo_root: PathBuf, theme: Theme) -> Result<Self> {
        let (board, grace, external, resources, worktrees, notices) = fetch(client.as_mut())?;
        Ok(Self {
            client,
            repo_root,
            board,
            grace,
            external,
            resources,
            worktrees,
            notices,
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
            spin_epoch: Cell::new(None),
            diff: None,
            pending_attach: None,
            pending_attach_cwd: None,
            pending_gate_then: None,
            pending_spawn_focus: None,
            focused_session_hint: None,
            just_created: None,
            rich_keys: false,
            update_watch: crate::update::UpdateWatch::new(),
            pending_reexec: false,
            delete_armed: None,
            tag_armed: None,
            archive_armed: None,
            last_undo: None,
            help: false,
            force_redraw: false,
            pending_suspend: false,
            diff_two_pane: Cell::new(true),
            daemon_down: false,
            last_reconnect: None,
        })
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

    /// Take whatever board a command replied with (RescanExternal does this).
    fn absorb_board(&mut self, resp: Response) {
        if let Response::Board { board, grace, external, resources, worktrees, notices } = resp {
            self.board = board;
            self.grace = grace;
            self.external = external;
            self.resources = resources;
            self.worktrees = worktrees;
            self.notices = notices;
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
        match &self.screen {
            Screen::Ticket { ticket, rail_idx } => {
                if self.board.ticket(*ticket).is_none() {
                    self.to_board();
                } else {
                    let n = self.rail_sessions(*ticket).len();
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
        if self.update_watch.tick() {
            self.status = "update ready ∙ U reloads".into();
            dirty = true;
        }
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
        if !event::poll(Duration::from_millis(100))? {
            return Ok(dirty);
        }
        let ev = event::read()?;
        let TermEvent::Key(key) = ev else {
            return Ok(dirty);
        };
        if key.kind != KeyEventKind::Press {
            return Ok(dirty);
        }
        // Any keypress abandons a parked spawn-focus: the user moved on, and
        // yanking them into a session mid-thought is worse than not focusing.
        // (A new `c` re-arms it below; the session itself still spawns.)
        self.pending_spawn_focus = None;
        self.status.clear();
        self.merge_note.clear();
        self.handle_key(key.code, key.modifiers)?;
        Ok(true)
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
        let rail_idx = match &self.screen {
            Screen::Ticket { rail_idx, .. } => *rail_idx,
            _ => usize::MAX,
        };
        let selected = sessions.get(rail_idx.min(sessions.len().saturating_sub(1)));
        let wt = subject.and_then(|t| self.wt_item(t));
        let merge = subject.map(|t| self.merge_stage_word(t)).unwrap_or(None);
        Ctx {
            has_ticket: subject.is_some(),
            multi_column: self.columns().len() > 1,
            ticket_has_sessions: !sessions.is_empty(),
            ticket_has_claude: sessions
                .iter()
                .any(|s| s.kind == SessionKind::Claude && s.state.is_live()),
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
            bulk_sleep: self.resources.reclaim_sessions,
            bulk_sleep_bytes: self.resources.reclaim_bytes,
            bulk_archive: self.resources.archive_tickets,
            has_archived: !self.board.archived_tickets().is_empty(),
            peek_on: self.peek,
            update_ready: self.update_ready(),
            any_attention: !mesimon_core::attention::attention_queue(&self.board).is_empty(),
            sel_session: !sessions.is_empty() && rail_idx != usize::MAX,
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
            tag_group: self.tag_armed.as_ref().and_then(|a| a.group),
            tag_naming: self.tag_armed.as_ref().is_some_and(|a| a.naming.is_some()),
            tag_group_has_tags: self
                .tag_armed
                .as_ref()
                .and_then(|a| a.group)
                .is_some_and(|g| !self.board.group_tags(g).is_empty()),
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
                    buffer: EditBuffer::new(),
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
                        buffer: EditBuffer::from_text(t.title.clone()),
                    };
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
            // `a` on a ticket that is already archived restores it right
            // away; otherwise it arms, and the second `a` archives.
            Verb::TagPrefix => {
                // While composing the ticket does not exist yet; the picks
                // buffer on the composer and travel with it on save.
                let ticket = if ctx.composing { None } else { self.subject() };
                if ticket.is_none() && !ctx.composing {
                    return Ok(());
                }
                self.tag_armed = Some(TagArm { ticket, group: None, naming: None });
                self.status = "1-9 pick a group ∙ esc done".into();
            }
            Verb::TagGroup => {
                let Key::Char(c) = key else { return Ok(()) };
                let Some(group) = c.to_digit(10).filter(|d| *d > 0).map(|d| d as u8) else {
                    return Ok(());
                };
                if let Some(arm) = self.tag_armed.as_mut() {
                    arm.group = Some(group);
                }
                let vocab: Vec<String> =
                    self.board.group_tags(group).iter().map(|s| s.to_string()).collect();
                if vocab.is_empty() {
                    // Nothing to cycle: an empty axis falls straight into
                    // naming, which is what "create on the fly" means.
                    if let Some(arm) = self.tag_armed.as_mut() {
                        arm.naming = Some(EditBuffer::new());
                    }
                    self.status = format!("name a tag for group {group}");
                    return Ok(());
                }
                // none -> first -> … -> last -> none.
                let current =
                    self.tag_subject().and_then(|t| t.iter().find(|t| t.group == group)).cloned();
                let next = match current {
                    None => Some(vocab[0].clone()),
                    Some(cur) => match vocab.iter().position(|v| *v == cur.name) {
                        Some(i) if i + 1 < vocab.len() => Some(vocab[i + 1].clone()),
                        // Past the end, or wearing a tag no longer in the
                        // vocabulary: land on "none" so every value is
                        // reachable by pressing again.
                        _ => None,
                    },
                };
                self.status = match &next {
                    Some(n) => format!("{n} — group {group}"),
                    None => format!("group {group} cleared"),
                };
                self.apply_tag(group, next)?;
            }
            Verb::TagNew => {
                if let Some(arm) = self.tag_armed.as_mut() {
                    if arm.group.is_some() {
                        arm.naming = Some(EditBuffer::new());
                    }
                }
                if let Some(g) = ctx.tag_group {
                    self.status = format!("name a tag for group {g}");
                }
            }
            Verb::TagClear => {
                if let Some(g) = ctx.tag_group {
                    self.status = format!("group {g} cleared");
                    self.apply_tag(g, None)?;
                }
            }
            Verb::TagDone => {
                self.tag_armed = None;
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
            Verb::ClaudeNew | Verb::ShellNew => {
                let kind =
                    if verb == Verb::ClaudeNew { SessionKind::Claude } else { SessionKind::Bash };
                if let Some(id) = self.subject() {
                    self.spawn_and_focus(id, kind)?;
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
            Verb::PageDown => self.diff_scroll(DIFF_PAGE as isize),
            Verb::PageUp => self.diff_scroll(-(DIFF_PAGE as isize)),
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
            | Verb::EditKillToStart => {}
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
        let Screen::Ticket { ticket, rail_idx } = &self.screen else {
            return None;
        };
        let rail = self.rail_sessions(*ticket);
        rail.get((*rail_idx).min(rail.len().saturating_sub(1))).map(|s| s.id)
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
                let n = self.rail_sessions(ticket).len();
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
            _ => {}
        }
    }

    /// Enter: act on the selection, whatever the selection is here.
    fn act(&mut self, scope: Scope) -> Result<()> {
        match scope {
            Scope::Board => self.board_enter(),
            Scope::Ticket => {
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
                Response::Spawned { id } => {
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
        let key = crate::keys::to_key(code, mods);
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
            Some(Verb::TagPrefix) => {
                // Arm the tail and hand the following keys to it. The mode
                // stays `Input`, so the half-typed title is untouched
                // underneath and Esc comes back to it.
                self.mode = Mode::Input { purpose, buffer };
                self.tag_armed = Some(TagArm { ticket: None, group: None, naming: None });
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

    /// The `^t` tail. Owns every key while it is open, exactly as the input
    /// barrier does — including the digits, which is why the tail's bindings
    /// all stand down while a name is being typed.
    fn key_tag(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        let Some(mut arm) = self.tag_armed.take() else { return Ok(()) };
        let Some(key) = crate::keys::to_key(code, mods) else {
            self.tag_armed = Some(arm);
            return Ok(());
        };
        let word = crate::keys::word_wise(mods);

        // Naming: the tail is silent, the field owns the keys.
        if let Some(buf) = arm.naming.as_mut() {
            match keymap::resolve(Scope::Input, key, &self.ctx()) {
                Some(Verb::Save) | Some(Verb::SaveStart) => {
                    let name = buf.as_str().to_string();
                    arm.naming = None;
                    self.tag_armed = Some(arm);
                    return self.commit_tag_name(name);
                }
                Some(Verb::Cancel) | Some(Verb::Back) | Some(Verb::Quit) => {
                    arm.naming = None;
                    self.status = "tag cancelled".into();
                }
                Some(Verb::EditBackspace) if word => buf.delete_word_back(),
                Some(Verb::EditBackspace) => buf.backspace(),
                Some(Verb::EditDeleteWord) => buf.delete_word_back(),
                Some(Verb::EditKillToStart) => buf.kill_to_start(),
                Some(Verb::EditDelete) => buf.delete(),
                Some(Verb::EditLeft) if word => buf.word_left(),
                Some(Verb::EditLeft) => buf.left(),
                Some(Verb::EditRight) if word => buf.word_right(),
                Some(Verb::EditRight) => buf.right(),
                Some(Verb::EditHome) => buf.home(),
                Some(Verb::EditEnd) => buf.end(),
                _ => match code {
                    KeyCode::Char(_) if word => {}
                    KeyCode::Char(c) => buf.insert(c),
                    _ => {}
                },
            }
            self.tag_armed = Some(arm);
            return Ok(());
        }

        self.tag_armed = Some(arm);
        let ctx = self.ctx();
        let Some(verb) = keymap::resolve(Scope::TagChord, key, &ctx) else {
            // A stray key inside a tail cancels rather than acting — and says
            // so, because a chord that silently evaporates is worse than one
            // that reports.
            self.tag_armed = None;
            self.status = "tags closed".into();
            return Ok(());
        };
        self.dispatch(verb, key, Scope::TagChord, &ctx)
    }

    /// Which ticket the tag tail is acting on, and the tags it already wears.
    /// While composing the ticket does not exist yet, so the picks live on
    /// `InputPurpose::Create` until `Response::Created` hands us an id.
    pub(crate) fn tag_subject(&self) -> Option<&[Tag]> {
        let arm = self.tag_armed.as_ref()?;
        match arm.ticket {
            Some(id) => self.board.ticket(id).map(|t| t.tags.as_slice()),
            None => match &self.mode {
                Mode::Input { purpose: InputPurpose::Create { tags, .. }, .. } => Some(tags),
                _ => None,
            },
        }
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
                if let Mode::Input { purpose: InputPurpose::Create { tags, .. }, .. } =
                    &mut self.mode
                {
                    tags.retain(|t| t.group != group);
                    if let Some(name) = name {
                        tags.push(Tag { name, group });
                    }
                    tags.sort_by_key(|t| t.group);
                }
            }
        }
        Ok(())
    }

    /// Finish naming: sanitize here too so the status line and the composer
    /// buffer agree with what the daemon will actually store.
    fn commit_tag_name(&mut self, raw: String) -> Result<()> {
        let Some(group) = self.tag_armed.as_ref().and_then(|a| a.group) else { return Ok(()) };
        match sanitize_tag(&raw) {
            Some(name) => {
                self.status = format!("{name} — group {group}");
                self.apply_tag(group, Some(name))
            }
            None => {
                self.status = "a tag needs a name".into();
                Ok(())
            }
        }
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
        self.send(Command::MoveTicket { id: ticket, column: target_col, before })?;
        self.cursor_col = col;
        self.cursor_row = idx;
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
                            let _ = self.req(Command::SetTag {
                                id,
                                group: tag.group,
                                name: Some(tag.name),
                            });
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
            }
            InputPurpose::Rename { id } => {
                self.send(Command::RenameTicket { id, title })?;
            }
        }
        Ok(())
    }

    /// Shift+Enter's second half: start claude on the ticket the composer just
    /// minted, with its title submitted as the first prompt, and STAY on the
    /// board. No `focus_session` — the whole point of the key is to queue work
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
    /// included) unless dismissed (`x` on the corpse marks it `Dismissed` —
    /// the one exit the rail hides). Older corpses re-import through the
    /// drawer.
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

    fn spawn_and_focus(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Result<()> {
        match self.req(Command::SpawnSession { ticket, kind, submit_prompt: false }) {
            Response::Spawned { id } => {
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
                    Response::Spawned { .. } => {
                        self.resume_refused = None;
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
        Response::Board { board, grace, external, resources, worktrees, notices } => {
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
        /// Debug-formatted log of every request, for behavior assertions.
        pub sent: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
        /// Make FocusStart answer Err (the daemon refusing a focus).
        pub refuse_focus: bool,
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
                        workspace: None,
                        tags: Vec::new(),
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
                    return Ok(Response::Spawned { id });
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
                }),
                Command::MoveTicket { id, column, before } => {
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
                sent: sent.clone(),
                refuse_focus,
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
            workspace: None,
            tags: Vec::new(),
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
        let mut b = board_three_columns();
        let sid = uuid::Uuid::from_u128(7);
        b.sessions.push(mesimon_core::board::SessionRecord::new(
            sid,
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            state,
        ));
        let (app, sent) = App::for_test_logged(b, theme(), refuse_focus);
        (app, sent, sid)
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

    /// An empty axis has nothing to cycle, so the first press on it goes
    /// straight to naming — that IS "create on the fly, nothing seeded".
    #[test]
    fn a_fresh_group_asks_for_a_name() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        assert_eq!(app.scope(), Scope::TagChord);
        press(&mut app, '1');
        let arm = app.tag_armed.as_ref().expect("still armed");
        assert_eq!(arm.group, Some(1));
        assert!(arm.naming.is_some(), "an empty group falls into naming");
        // While naming, the field owns the keys — including the digits.
        for c in "BUG2".chars() {
            press(&mut app, c);
        }
        assert_eq!(
            app.tag_armed.as_ref().and_then(|a| a.naming.as_ref()).map(|b| b.as_str()),
            Some("BUG2"),
            "digits are text while naming, not group picks"
        );
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "SetTag"), "the name commits");
        assert!(sent_contains(&sent, "BUG2"));
    }

    /// A group with a vocabulary cycles: none -> first -> … -> last -> none,
    /// so every value including "no tag" is reachable by pressing again.
    #[test]
    fn a_digit_cycles_its_group_and_wraps_through_none() {
        let mut board = board_three_columns();
        let id = board.tickets[0].id;
        // Two tags in group 1, worn by other tickets — the derived registry.
        board.tickets[1].set_tag(1, Some("BUG".into()));
        board.tickets[2].set_tag(1, Some("REGR".into()));
        let (mut app, sent) = App::for_test_logged(board, theme(), false);
        app.cursor_col = 0;
        app.cursor_row = 0;
        assert_eq!(app.subject(), Some(id));

        ctrl(&mut app, 't');
        press(&mut app, '1');
        assert!(sent_contains(&sent, "BUG"), "none -> first");
        // The fake transport does not echo state back, so drive the cycle
        // through the board directly: what is under test is the step, and
        // the step reads the ticket's current tag.
        app.board.ticket_mut(id).expect("ticket").set_tag(1, Some("BUG".into()));
        sent.borrow_mut().clear();
        press(&mut app, '1');
        assert!(sent_contains(&sent, "REGR"), "first -> second");

        app.board.ticket_mut(id).expect("ticket").set_tag(1, Some("REGR".into()));
        sent.borrow_mut().clear();
        press(&mut app, '1');
        let log = sent.borrow().join(" ");
        assert!(log.contains("SetTag"), "last -> none still sends");
        assert!(log.contains("name: None"), "clearing sends no name, got {log}");
    }

    /// The tail stays open between digits — two axes, one `^t`.
    #[test]
    fn the_tail_holds_across_groups_and_a_stray_key_closes_it() {
        let (mut app, _sent) = App::for_test_logged(board_three_columns(), theme(), false);
        ctrl(&mut app, 't');
        press(&mut app, '1');
        // Cancel the name prompt, not the tail.
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        assert!(app.tag_armed.is_some(), "esc leaves naming, not the tail");
        assert!(app.tag_armed.as_ref().expect("armed").naming.is_none());
        // A key the tail does not bind cancels it, like every other chord.
        press(&mut app, 'j');
        assert!(app.tag_armed.is_none(), "a stray key closes the tail");
        assert_eq!(app.scope(), Scope::Board);
    }

    /// `^t` from the composer: the picks ride on the half-typed ticket and
    /// are replayed once it has an id. A bare `t` could never do this — it
    /// would be typed into the title.
    #[test]
    fn tags_survive_the_composer() {
        let (mut app, sent) = App::for_test_logged(board_three_columns(), theme(), false);
        press(&mut app, 'o');
        for c in "fix it".chars() {
            press(&mut app, c);
        }
        ctrl(&mut app, 't');
        assert_eq!(app.scope(), Scope::TagChord, "the tail outranks the input barrier");
        press(&mut app, '2');
        for c in "DEV".chars() {
            press(&mut app, c);
        }
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        // Nothing has been sent yet — the ticket does not exist.
        assert!(!sent_contains(&sent, "SetTag"), "no ticket to tag yet");
        match &app.mode {
            Mode::Input { purpose: InputPurpose::Create { tags, .. }, buffer } => {
                assert_eq!(buffer.as_str(), "fix it", "the title survived the round trip");
                assert_eq!(tags.len(), 1);
                assert_eq!(tags[0].name, "DEV");
                assert_eq!(tags[0].group, 2);
            }
            other => panic!("still composing, got {other:?}"),
        }
        // Leave the tail, then save: now the tag travels.
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE).unwrap();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE).unwrap();
        assert!(sent_contains(&sent, "CreateTicket"));
        assert!(sent_contains(&sent, "SetTag"), "the buffered tag is replayed");
        assert!(sent_contains(&sent, "DEV"));
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
