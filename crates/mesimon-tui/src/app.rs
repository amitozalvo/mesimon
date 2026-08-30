//! Board app: state, keymap (M1 subset — the full 04 keymap lands in M2/M6),
//! MOVE mode with client-side ghost (07 §7 core rules), focus flow with GATE,
//! and the M3.5 ticket screen (Enter opens it; the old session picker is its
//! SESSIONS rail now).

use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use mesimon_core::board::{
    Board, ExitReason, Provenance, SessionKind, SessionState, Ticket, WorkspaceStrategy,
};
use mesimon_core::command::{
    Command, ExternalItem, GraceItem, MergeOutcome, Resources, Response, WorktreeItem,
};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::client::Transport;
use crate::text::EditBuffer;
use crate::theme::Theme;

/// Which screen owns the keymap and the frame (07 §1). `Mode` remains the
/// board's sub-state; the ticket screen has no modes yet.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Board,
    Ticket { ticket: ulid::Ulid, rail_idx: usize },
    /// Read-only diff viewer (M4b): ticket `v`. State lives in `App::diff`,
    /// not here — Screen is cloned on every keypress.
    Diff { ticket: ulid::Ulid },
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
    /// drop. `grab` is the key that started it (`>` or `<`): the same key
    /// again (or Enter) commits, the opposite key cancels. `home` is where the
    /// grab happened (col, idx): a foreign column is always entered at the
    /// top, the home column at the ticket's own position (author 2026-08-30).
    Move { ticket: ulid::Ulid, col: usize, idx: usize, grab: char, home: (usize, usize) },
    Input { purpose: InputPurpose, buffer: EditBuffer },
    /// External drawer: discovered foreign sessions (19 §4 tier 1).
    External { idx: usize },
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
    Create { workspace: Option<WorkspaceStrategy> },
    Rename { id: ulid::Ulid },
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
    /// Daemon connection lost: keep the last board, re-dial on a slow cadence.
    daemon_down: bool,
    last_reconnect: Option<Instant>,
}

impl App {
    pub fn new(mut client: Box<dyn Transport>, repo_root: PathBuf, theme: Theme) -> Result<Self> {
        let (board, grace, external, resources, worktrees) = fetch(client.as_mut())?;
        Ok(Self {
            client,
            repo_root,
            board,
            grace,
            external,
            resources,
            worktrees,
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
            update_watch: crate::update::UpdateWatch::new(),
            pending_reexec: false,
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
            Ok((board, grace, external, resources, worktrees)) => {
                self.board = board;
                self.grace = grace;
                self.external = external;
                self.resources = resources;
                self.worktrees = worktrees;
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
        let Some((ticket, kind)) = self.pending_spawn_focus else { return Ok(()) };
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

    /// Take whatever board a command replied with (RescanExternal does this).
    fn absorb_board(&mut self, resp: Response) {
        if let Response::Board { board, grace, external, resources, worktrees } = resp {
            self.board = board;
            self.grace = grace;
            self.external = external;
            self.resources = resources;
            self.worktrees = worktrees;
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
    fn to_board(&mut self) {
        self.marquee.set(None);
        self.rail_marquee.set(None);
        self.screen = Screen::Board;
    }

    /// A refresh can delete the ticket the ticket screen shows.
    fn clamp_screen(&mut self) {
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
        let TermEvent::Key(key) = ev else { return Ok(dirty) };
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

    /// The full key dispatch, seam for the TestBackend harness.
    pub fn handle_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        // Text input first — it is inline (in the card / ticket title) and
        // owns every key on both screens, including Tab.
        if let Mode::Input { mut purpose, mut buffer } = self.mode.clone() {
            // Ctrl or Alt both mean "by word" — terminals disagree on which
            // one ctrl+backspace / option+arrow actually report.
            let word = mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            match code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                    return Ok(());
                }
                // Shift+Tab cycles the composer's workspace selector (M4):
                // shared checkout (default) ↔ own worktree.
                KeyCode::BackTab => {
                    if let InputPurpose::Create { workspace } = &mut purpose {
                        *workspace = match workspace {
                            None => Some(WorkspaceStrategy::Worktree),
                            Some(_) => None,
                        };
                    }
                }
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.commit_input(purpose, buffer.into_text())?;
                    return Ok(());
                }
                KeyCode::Backspace if word => buffer.delete_word_back(),
                KeyCode::Backspace => buffer.backspace(),
                KeyCode::Delete => buffer.delete(),
                KeyCode::Left if word => buffer.word_left(),
                KeyCode::Left => buffer.left(),
                KeyCode::Right if word => buffer.word_right(),
                KeyCode::Right => buffer.right(),
                KeyCode::Home => buffer.home(),
                KeyCode::End => buffer.end(),
                // Readline chords; ctrl+h is what legacy terminals send for
                // ctrl+backspace (0x08), so it deletes a word here, not a char.
                KeyCode::Char('h') if mods.contains(KeyModifiers::CONTROL) => {
                    buffer.delete_word_back()
                }
                KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
                    buffer.delete_word_back()
                }
                KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => {
                    buffer.kill_to_start()
                }
                KeyCode::Char('a') if mods.contains(KeyModifiers::CONTROL) => buffer.home(),
                KeyCode::Char('e') if mods.contains(KeyModifiers::CONTROL) => buffer.end(),
                KeyCode::Char('b') if mods.contains(KeyModifiers::ALT) => buffer.word_left(),
                KeyCode::Char('f') if mods.contains(KeyModifiers::ALT) => buffer.word_right(),
                // Unhandled chords must never type their letter.
                KeyCode::Char(_) if word => {}
                KeyCode::Char(c) => buffer.insert(c),
                _ => {}
            }
            self.mode = Mode::Input { purpose, buffer };
            return Ok(());
        }
        // A first `m` arms the merge confirm; any other key disarms it.
        if !matches!(code, KeyCode::Char('m')) {
            self.merge_armed = None;
        }
        // The fresh-ticket fast path lives exactly one Enter long: any other
        // key means the user is browsing, and Enter goes back to meaning
        // "open" (Shift+Enter consumes it too — they chose the page).
        if !matches!(code, KeyCode::Enter) {
            self.just_created = None;
        }
        // U on a ready update: reload in place — ask the daemon to shut down
        // clean (it comes back as the new binary via connect-spawn), then let
        // the main loop exec ourselves. Opt-in only, never automatic.
        if code == KeyCode::Char('U') && self.update_watch.ready() {
            let _ = self.client.request(Command::Shutdown);
            self.pending_reexec = true;
            return Ok(());
        }
        // Tab / Shift+Tab: next/previous needs-you card. Global, BEFORE the
        // mode dispatch (04/07 §21: the only exception is a text field).
        match code {
            KeyCode::Tab => {
                self.cycle_attention(false);
                return Ok(());
            }
            KeyCode::BackTab => {
                self.cycle_attention(true);
                return Ok(());
            }
            _ => {}
        }
        if let Screen::Diff { ticket } = self.screen.clone() {
            return self.key_diff(code, ticket);
        }
        if let Screen::Ticket { ticket, rail_idx } = self.screen.clone() {
            return self.key_ticket(code, mods, ticket, rail_idx);
        }
        match self.mode.clone() {
            Mode::Normal => self.key_normal(code, mods)?,
            Mode::Move { ticket, col, idx, grab, home } => {
                self.key_move(code, ticket, col, idx, grab, home)?
            }
            Mode::External { idx } => self.key_external(code, idx)?,
            Mode::Input { .. } => {} // handled above
        }
        Ok(())
    }

    fn key_normal(&mut self, code: KeyCode, mods: KeyModifiers) -> Result<()> {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Char('h') | KeyCode::Left => {
                self.cursor_col = self.cursor_col.saturating_sub(1);
                self.clamp_cursor();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.cursor_col += 1;
                self.clamp_cursor();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor_row += 1;
                self.clamp_cursor();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor_row = self.cursor_row.saturating_sub(1);
            }
            KeyCode::Char('g') => {
                self.cursor_row = 0;
            }
            KeyCode::Char('G') => {
                self.cursor_row = usize::MAX;
                self.clamp_cursor();
            }
            KeyCode::Char('o') | KeyCode::Char('a') => {
                self.mode = Mode::Input {
                    purpose: InputPurpose::Create { workspace: None },
                    buffer: EditBuffer::new(),
                };
            }
            KeyCode::Char('r') => {
                if let Some(t) = self.selected_ticket() {
                    self.mode = Mode::Input {
                        purpose: InputPurpose::Rename { id: t.id },
                        buffer: EditBuffer::from_text(t.title.clone()),
                    };
                }
            }
            KeyCode::Char('d') => {
                if let Some(t) = self.selected_ticket() {
                    let id = t.id;
                    self.delete_gated(id, false)?;
                }
            }
            KeyCode::Char('D') => {
                if let Some(t) = self.selected_ticket() {
                    let id = t.id;
                    self.delete_gated(id, true)?;
                }
            }
            KeyCode::Char('u') => {
                if let Some(g) = self.grace.last() {
                    let id = g.id;
                    self.send(Command::RestoreTicket { id })?;
                }
            }
            // `>` / `<` grab the card and shift its ghost one column that way
            // immediately (doc 04's `m`, remapped — STALE-MAP): the move is
            // pending and blinking; the same key again (or Enter) drops it,
            // the opposite key cancels, hjkl fine-place meanwhile.
            KeyCode::Char(c @ ('>' | '<')) => {
                if let Some(t) = self.selected_ticket() {
                    let id = t.id;
                    let cols = self.columns();
                    if !cols.is_empty() {
                        let col = if c == '>' {
                            (self.cursor_col + 1) % cols.len()
                        } else {
                            (self.cursor_col + cols.len() - 1) % cols.len()
                        };
                        let home = (self.cursor_col, self.cursor_row);
                        let idx = self.ghost_entry_idx(&cols, col, home, id);
                        self.mode = Mode::Move { ticket: id, col, idx, grab: c, home };
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(t) = self.selected_ticket() {
                    let ticket = t.id;
                    self.spawn_and_focus(ticket, SessionKind::Claude)?;
                }
            }
            KeyCode::Char('S') => {
                if let Some(t) = self.selected_ticket() {
                    let ticket = t.id;
                    self.spawn_and_focus(ticket, SessionKind::Bash)?;
                }
            }
            KeyCode::Char('e') => self.open_drawer()?,
            // Transcript peek toggle. Doc 04's BOARD `p` (duplicate-yanked) is
            // unimplemented; peek borrows the INBOX mnemonic until the M6
            // keymap pass (STALE-MAP, M3.5 deviations).
            KeyCode::Char('p') => {
                self.peek = !self.peek;
                self.status = if self.peek {
                    "peek on — latest reply shows under the cursor card".into()
                } else {
                    "peek off".into()
                };
            }
            KeyCode::Char('Z') => {
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
                self.refresh()?;
            }
            // Space always opens the ticket screen — the fallback spelling of
            // Shift+Enter for terminals without the kitty keyboard protocol
            // (plain Enter and Shift+Enter are the same byte there).
            KeyCode::Char(' ') => {
                if let Some(t) = self.selected_ticket() {
                    self.screen = Screen::Ticket { ticket: t.id, rail_idx: 0 };
                }
            }
            KeyCode::Enter => self.board_enter(mods)?,
            _ => {}
        }
        Ok(())
    }

    /// Board Enter is "get me working" (author 2026-08-30): a running claude
    /// focuses directly, a just-composed ticket spawns one, and only then does
    /// Enter mean the ticket screen. Shift+Enter forces the screen (kitty-
    /// protocol terminals only; Space is the everywhere fallback).
    fn board_enter(&mut self, mods: KeyModifiers) -> Result<()> {
        let Some(t) = self.selected_ticket() else { return Ok(()) };
        let ticket = t.id;
        let fresh = self.just_created.take() == Some(ticket);
        if mods.contains(KeyModifiers::SHIFT) {
            self.screen = Screen::Ticket { ticket, rail_idx: 0 };
            return Ok(());
        }
        // Running or needs-you claude (author 2026-08-30): both mean the pane
        // is where the action is. Anything else — idle, sleeping, unknown —
        // opens the ticket page, where the state is visible before you commit
        // to entering the pane.
        let hot = self.rail_sessions(ticket).iter().position(|s| {
            s.kind == SessionKind::Claude
                && matches!(s.state, SessionState::Running | SessionState::RequiresAction { .. })
        });
        if let Some(rail_idx) = hot {
            let sid = self.rail_sessions(ticket)[rail_idx].id;
            self.focus_session(sid)?;
            if self.pending_attach.is_none() {
                // Focus refused (daemon said no / unreachable): fall open to
                // the ticket screen, where the status explains itself.
                self.screen = Screen::Ticket { ticket, rail_idx };
            }
        } else if fresh {
            // The composer's Enter-Enter: name the ticket, start the work.
            // Provisioning worktrees park the focus half as usual.
            self.spawn_and_focus(ticket, SessionKind::Claude)?;
        } else {
            self.screen = Screen::Ticket { ticket, rail_idx: 0 };
        }
        Ok(())
    }

    /// TICKET keymap (04 §2.6 subset; the M3 picker keys re-homed here).
    /// Note: inside TICKET, `s` is the shell session per 04 — this differs
    /// from BOARD's `s` (claude) until the M6 keymap pass reconciles them.
    fn key_ticket(
        &mut self,
        code: KeyCode,
        mods: KeyModifiers,
        ticket: ulid::Ulid,
        rail_idx: usize,
    ) -> Result<()> {
        let rail: Vec<uuid::Uuid> = self.rail_sessions(ticket).iter().map(|s| s.id).collect();
        let idx = rail_idx.min(rail.len().saturating_sub(1));
        match code {
            KeyCode::Esc | KeyCode::Char('q') => self.to_board(),
            // Ctrl+] pops to the board too: it is the tmux detach key, so the
            // hand is already on it right after an unfocus lands here. Legacy
            // terminals send 0x1D, which crossterm reports as Ctrl+5 (the
            // kitty protocol reports a true Ctrl+]).
            KeyCode::Char(']' | '5') if mods.contains(KeyModifiers::CONTROL) => self.to_board(),
            KeyCode::Char('j') | KeyCode::Down => {
                let idx = (idx + 1).min(rail.len().saturating_sub(1));
                self.screen = Screen::Ticket { ticket, rail_idx: idx };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.screen = Screen::Ticket { ticket, rail_idx: idx.saturating_sub(1) };
            }
            // 04 §2.6's h/l = adjacent ticket is deliberately NOT bound:
            // a ticket screen holds one ticket (author, dogfood 2026-08-30).
            KeyCode::Char('r') => {
                if let Some(t) = self.board.ticket(ticket) {
                    self.mode = Mode::Input {
                        purpose: InputPurpose::Rename { id: t.id },
                        buffer: EditBuffer::from_text(t.title.clone()),
                    };
                }
            }
            KeyCode::Enter => {
                if let Some(&sid) = rail.get(idx) {
                    self.focus_session(sid)?;
                }
            }
            // c / s: focus the existing session of that kind, spawn if none
            // (04 §2.6); C / S always spawn fresh.
            KeyCode::Char('c') => self.focus_kind_or_spawn(ticket, SessionKind::Claude)?,
            KeyCode::Char('s') => self.focus_kind_or_spawn(ticket, SessionKind::Bash)?,
            KeyCode::Char('C') => self.spawn_and_focus(ticket, SessionKind::Claude)?,
            KeyCode::Char('S') => self.spawn_and_focus(ticket, SessionKind::Bash)?,
            KeyCode::Char('x') => {
                if let Some(&sid) = rail.get(idx) {
                    self.send(Command::KillSession { id: sid })?;
                }
            }
            KeyCode::Char('z') => {
                if let Some(&sid) = rail.get(idx) {
                    let asleep = self
                        .board
                        .sessions
                        .iter()
                        .any(|s| s.id == sid && matches!(s.state, SessionState::Sleeping));
                    let cmd = if asleep {
                        Command::WakeSession { id: sid }
                    } else {
                        Command::SleepSession { id: sid }
                    };
                    self.send(cmd)?;
                }
            }
            KeyCode::Char('p') => {
                if let Some(&sid) = rail.get(idx) {
                    let pinned = self
                        .board
                        .sessions
                        .iter()
                        .find(|s| s.id == sid)
                        .map(|s| !s.pinned_awake)
                        .unwrap_or(true);
                    self.send(Command::PinAwake { id: sid, pinned })?;
                    self.status = if pinned { "pinned awake".into() } else { "unpinned".into() };
                }
            }
            KeyCode::Char('d') => self.delete_gated(ticket, false)?,
            KeyCode::Char('D') => self.delete_gated(ticket, true)?,
            // M4 workspace cycle: shared checkout ↔ own worktree. The daemon
            // refuses once sessions or a worktree exist (the choice is locked).
            KeyCode::Char('w') => {
                let next = match self.board.ticket(ticket).map(|t| t.workspace_strategy()) {
                    Some(WorkspaceStrategy::Worktree) => None,
                    _ => Some(WorkspaceStrategy::Worktree),
                };
                let word = match next {
                    Some(WorkspaceStrategy::Worktree) => "worktree",
                    _ => "shared",
                };
                self.send(Command::SetWorkspace { id: ticket, workspace: next })?;
                if self.status.is_empty() {
                    self.status = format!("workspace: {word}");
                }
            }
            KeyCode::Char('m') => self.merge_key(ticket)?,
            // M4b: read-only diff viewer on any ticket with a binding.
            KeyCode::Char('v') => self.open_diff(ticket, rail_idx)?,
            _ => {}
        }
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
                d.file_idx = keep
                    .and_then(|p| files.iter().position(|f| f.path == p))
                    .unwrap_or(0);
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
        for (i, cursor) in [(idx as isize, true), (idx as isize + 1, false), (idx as isize - 1, false)]
        {
            let Some(d) = self.diff.as_ref() else { return };
            if i < 0 {
                continue;
            }
            let Some(f) = d.files.get(i as usize) else { continue };
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

    /// The diff screen's keys: j/k scroll, h/l (or J/K) file, R refresh,
    /// z z density, z p pane swap, ! shell in the worktree, q back.
    fn key_diff(&mut self, code: KeyCode, ticket: ulid::Ulid) -> Result<()> {
        // The z view-chord: a first z arms, z z cycles density, z p swaps
        // panes below the breakpoint; any other key disarms and acts.
        let armed = self.diff.as_ref().map(|d| d.z_armed).unwrap_or(false);
        if let Some(d) = self.diff.as_mut() {
            d.z_armed = false;
        }
        if armed {
            match code {
                KeyCode::Char('z') => {
                    let idx = {
                        let Some(d) = self.diff.as_mut() else { return Ok(()) };
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
                    return Ok(());
                }
                KeyCode::Char('p') => {
                    if let Some(d) = self.diff.as_mut() {
                        d.swap = !d.swap;
                    }
                    return Ok(());
                }
                _ => {} // disarmed; fall through to act on the key
            }
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                let rail_idx = self.diff.as_ref().map(|d| d.rail_idx).unwrap_or(0);
                self.diff = None;
                self.screen = Screen::Ticket { ticket, rail_idx };
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(d) = self.diff.as_ref() {
                    d.scroll.set(d.scroll.get() + 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(d) = self.diff.as_ref() {
                    d.scroll.set(d.scroll.get().saturating_sub(1));
                }
            }
            // Vim-adjacent paging; draw clamps against the built content.
            KeyCode::Char('}') | KeyCode::PageDown => {
                if let Some(d) = self.diff.as_ref() {
                    d.scroll.set(d.scroll.get() + DIFF_PAGE);
                }
            }
            KeyCode::Char('{') | KeyCode::PageUp => {
                if let Some(d) = self.diff.as_ref() {
                    d.scroll.set(d.scroll.get().saturating_sub(DIFF_PAGE));
                }
            }
            KeyCode::Char('l') | KeyCode::Char('J') | KeyCode::Right => self.diff_nav(1),
            KeyCode::Char('h') | KeyCode::Char('K') | KeyCode::Left => self.diff_nav(-1),
            KeyCode::Char('R') => self.diff_refresh(ticket),
            KeyCode::Char('z') => {
                if let Some(d) = self.diff.as_mut() {
                    d.z_armed = true;
                }
            }
            KeyCode::Char('!') => {
                let present = self.diff.as_ref().map(|d| d.worktree_present).unwrap_or(false);
                let path = self.wt_item(ticket).and_then(|w| w.path.clone());
                match (present, path) {
                    (true, Some(p)) => {
                        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                        // Plain argv, no sh -c; the main loop's handover runs
                        // it with this cwd. Gate/focus hints stay None so
                        // after_handover leaves the diff screen alone.
                        self.pending_attach = Some(vec![shell]);
                        self.pending_attach_cwd = Some(PathBuf::from(p));
                    }
                    _ => self.status = "worktree evicted — no directory to open".into(),
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Delete with the M4 worktree gate (author rule 2): an unmerged worktree
    /// must be dealt with first — `m` merges, `D` discards worktree + branch.
    fn delete_gated(&mut self, id: ulid::Ulid, discard: bool) -> Result<()> {
        if !discard {
            if let Some(w) = self.wt_item(id) {
                if !w.branch.is_empty() && !w.merged {
                    self.status =
                        "worktree unmerged ∙ m merge ∙ D delete + discard branch".into();
                    return Ok(());
                }
            }
        }
        if matches!(self.screen, Screen::Ticket { .. }) {
            self.to_board();
        }
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

    fn key_external(&mut self, code: KeyCode, idx: usize) -> Result<()> {
        if self.external.is_empty() {
            self.mode = Mode::Normal;
            return Ok(());
        }
        let idx = idx.min(self.external.len() - 1);
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => {
                self.mode = Mode::External { idx: (idx + 1).min(self.external.len() - 1) };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::External { idx: idx.saturating_sub(1) };
            }
            KeyCode::Char('a') => {
                // Import: the daemon mints a ticket named after the session.
                let claude_session_id = self.external[idx].claude_session_id;
                match self.req(Command::AttachExternal { claude_session_id, ticket: None }) {
                    Response::Spawned { id } => {
                        self.refresh()?;
                        self.status = self
                            .board
                            .sessions
                            .iter()
                            .find(|s| s.id == id)
                            .and_then(|s| self.board.ticket(s.ticket))
                            .map(|t| format!("imported \"{}\" — observe-only, R resumes", t.title))
                            .unwrap_or_else(|| "imported — observe-only".into());
                        self.mode = Mode::Normal;
                        return Ok(());
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.refresh()?;
            }
            KeyCode::Char('R') | KeyCode::Enter => {
                let claude_session_id = self.external[idx].claude_session_id;
                let confirm = self.resume_refused == Some(claude_session_id);
                match self.req(Command::ResumeExternal {
                    claude_session_id,
                    ticket: None,
                    confirm,
                }) {
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
                self.refresh()?;
            }
            _ => {}
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

    fn key_move(
        &mut self,
        code: KeyCode,
        ticket: ulid::Ulid,
        col: usize,
        idx: usize,
        grab: char,
        home: (usize, usize),
    ) -> Result<()> {
        let cols = self.columns();
        match code {
            KeyCode::Esc => self.mode = Mode::Normal, // total cancel (M2 rule)
            KeyCode::Char('h') | KeyCode::Left => {
                let to = col.saturating_sub(1);
                // A saturated edge press stays put — no entry, no reset.
                let idx = if to == col { idx } else { self.ghost_entry_idx(&cols, to, home, ticket) };
                self.mode = Mode::Move { ticket, col: to, idx, grab, home };
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let to = (col + 1).min(cols.len().saturating_sub(1));
                let idx = if to == col { idx } else { self.ghost_entry_idx(&cols, to, home, ticket) };
                self.mode = Mode::Move { ticket, col: to, idx, grab, home };
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let n = self.ghost_len(&cols, col, ticket);
                self.mode = Mode::Move { ticket, col, idx: (idx + 1).min(n), grab, home };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::Move { ticket, col, idx: idx.saturating_sub(1), grab, home };
            }
            KeyCode::Char(c @ ('>' | '<')) => {
                if c == grab {
                    // The grab key again: commit the pending move where the
                    // ghost stands (`>>` / `<<` — one column in one gesture).
                    self.drop_ghost(&cols, ticket, col, idx)?;
                } else {
                    // The opposite key cancels the whole move.
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Enter => self.drop_ghost(&cols, ticket, col, idx)?,
            _ => {}
        }
        Ok(())
    }

    /// Commit the MOVE ghost: reinsert `ticket` at (`col`, `idx`) and land the
    /// cursor on it.
    fn drop_ghost(&mut self, cols: &[String], ticket: ulid::Ulid, col: usize, idx: usize) -> Result<()> {
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

    fn commit_input(&mut self, purpose: InputPurpose, buffer: String) -> Result<()> {
        let title = buffer.trim().to_string();
        if title.is_empty() {
            return Ok(());
        }
        match purpose {
            InputPurpose::Create { workspace } => {
                let cols = self.columns();
                let column = cols.get(self.cursor_col).cloned().unwrap_or_default();
                match self.req(Command::CreateTicket { column, title }) {
                    Response::Created { id } => {
                        if workspace.is_some() {
                            let _ = self.req(Command::SetWorkspace { id, workspace });
                        }
                        self.refresh()?;
                        self.select_ticket(id);
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
        match self.req(Command::SpawnSession { ticket, kind }) {
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
            let exited_claude = rec.kind == SessionKind::Claude
                && matches!(rec.state, SessionState::Exited { .. });
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
                        let idx =
                            self.rail_sessions(ticket).iter().position(|s| s.id == sid).unwrap_or(0);
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

type Snapshot5 = (Board, Vec<GraceItem>, Vec<ExternalItem>, Resources, Vec<WorktreeItem>);

fn fetch(client: &mut dyn Transport) -> Result<Snapshot5> {
    match client.request(Command::Snapshot)? {
        Response::Board { board, grace, external, resources, worktrees } => {
            Ok((board, grace, external, resources, worktrees))
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
                Command::SpawnSession { ticket, kind } => {
                    let rec = mesimon_core::board::SessionRecord::new(
                        uuid::Uuid::from_u128(4242),
                        kind,
                        ticket,
                        vec!["claude".into()],
                        "/repo".into(),
                        SessionState::Running,
                    );
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

    #[test]
    fn shift_enter_forces_the_ticket_screen() {
        let (mut app, sent, _sid) = app_with_claude(SessionState::Running, false);
        app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
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
}
