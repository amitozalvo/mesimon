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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    /// MOVE: ghost position tracked client-side, committed on Enter (M-rules).
    Move { ticket: ulid::Ulid, col: usize, idx: usize },
    Input { purpose: InputPurpose, buffer: EditBuffer },
    /// External drawer: discovered foreign sessions (19 §4 tier 1).
    External { idx: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputPurpose {
    /// New-ticket composer. `workspace` is the Shift+Tab selector below the
    /// name (M4 layering): None = the board default (shared checkout).
    Create { workspace: Option<WorkspaceStrategy> },
    Rename { id: ulid::Ulid },
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
    pub theme: Theme,
    /// The drawer row whose resume was refused as running-elsewhere — a
    /// second R on the same row sends the confirm override.
    resume_refused: Option<uuid::Uuid>,
    /// Ticket armed by a first `m` — the second `m` performs the merge.
    merge_armed: Option<ulid::Ulid>,
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
    /// First visible card row of the cursor column (draw-side scroll state).
    pub scroll_row: Cell<usize>,
    /// Transcript peek (`p`): the cursor card also shows its latest assistant
    /// reply, read from the transcript at draw time (peek.rs).
    pub peek: bool,
    pub peek_cache: crate::peek::PeekCache,
    /// Working-spinner clock: epoch of the first draw (draw-side state, so
    /// the first rendered frame is always frame 0 — goldens stay stable).
    pub spin_epoch: Cell<Option<std::time::Instant>>,
    /// Set when the user asked to focus: the main loop performs the handover
    /// outside the render loop.
    pub pending_attach: Option<Vec<String>>,
    pub pending_gate_then: Option<uuid::Uuid>,
    /// The session a running handover holds focus on — released on return.
    focused_session_hint: Option<uuid::Uuid>,
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
            screen: Screen::Board,
            cursor_col: 0,
            cursor_row: 0,
            mode: Mode::Normal,
            status: String::new(),
            quit: false,
            col_window: Cell::new(0),
            marquee: Cell::new(None),
            scroll_row: Cell::new(0),
            peek: false,
            peek_cache: crate::peek::PeekCache::default(),
            spin_epoch: Cell::new(None),
            pending_attach: None,
            pending_gate_then: None,
            focused_session_hint: None,
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
            }
            Err(_) => self.note_daemon_down(),
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
        self.screen = Screen::Board;
    }

    /// A refresh can delete the ticket the ticket screen shows.
    fn clamp_screen(&mut self) {
        if let Screen::Ticket { ticket, rail_idx } = &self.screen {
            if self.board.ticket(*ticket).is_none() {
                self.to_board();
            } else {
                let n = self.rail_sessions(*ticket).len();
                let idx = (*rail_idx).min(n.saturating_sub(1));
                self.screen = Screen::Ticket { ticket: *ticket, rail_idx: idx };
            }
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
        self.status.clear();
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
        if let Screen::Ticket { ticket, rail_idx } = self.screen.clone() {
            return self.key_ticket(code, ticket, rail_idx);
        }
        match self.mode.clone() {
            Mode::Normal => self.key_normal(code, mods)?,
            Mode::Move { ticket, col, idx } => self.key_move(code, ticket, col, idx)?,
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
            KeyCode::Char('m') => {
                if let Some(t) = self.selected_ticket() {
                    self.mode = Mode::Move { ticket: t.id, col: self.cursor_col, idx: self.cursor_row };
                }
            }
            KeyCode::Char(c @ ('>' | '<')) => {
                if let Some(t) = self.selected_ticket() {
                    let id = t.id;
                    let cols = self.columns();
                    if cols.len() > 1 {
                        let target = if c == '>' {
                            (self.cursor_col + 1) % cols.len()
                        } else {
                            (self.cursor_col + cols.len() - 1) % cols.len()
                        };
                        let column = cols[target].clone();
                        self.send(Command::MoveTicket { id, column, before: None })?;
                        self.select_ticket(id);
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
            KeyCode::Enter => {
                // Enter opens the ticket screen (07 §1) — the direct-focus
                // fast path moved onto the ticket screen's Enter.
                if let Some(t) = self.selected_ticket() {
                    self.screen = Screen::Ticket { ticket: t.id, rail_idx: 0 };
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// TICKET keymap (04 §2.6 subset; the M3 picker keys re-homed here).
    /// Note: inside TICKET, `s` is the shell session per 04 — this differs
    /// from BOARD's `s` (claude) until the M6 keymap pass reconciles them.
    fn key_ticket(&mut self, code: KeyCode, ticket: ulid::Ulid, rail_idx: usize) -> Result<()> {
        let rail: Vec<uuid::Uuid> = self.rail_sessions(ticket).iter().map(|s| s.id).collect();
        let idx = rail_idx.min(rail.len().saturating_sub(1));
        match code {
            KeyCode::Esc | KeyCode::Char('q') => self.to_board(),
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
                    _ => "shared checkout",
                };
                self.send(Command::SetWorkspace { id: ticket, workspace: next })?;
                if self.status.is_empty() {
                    self.status = format!("workspace: {word}");
                }
            }
            KeyCode::Char('m') => self.merge_key(ticket)?,
            KeyCode::Char('M') => self.merge_to_agent_key(ticket)?,
            _ => {}
        }
        Ok(())
    }

    /// The ticket's worktree binding, as the last snapshot reported it.
    pub fn wt_item(&self, ticket: ulid::Ulid) -> Option<&WorktreeItem> {
        self.worktrees.iter().find(|w| w.ticket == ticket)
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

    /// First `m` arms, second performs (confirm default-No shape without a
    /// dialog). Conflicts answer with the one suggestion: M sends the merge
    /// to the agent — mesimon never resolves conflicts itself.
    fn merge_key(&mut self, ticket: ulid::Ulid) -> Result<()> {
        let Some(w) = self.wt_item(ticket) else {
            self.status = "no worktree on this ticket".into();
            return Ok(());
        };
        let branch = w.branch.clone();
        if self.merge_armed != Some(ticket) {
            self.merge_armed = Some(ticket);
            self.status = format!("merge {branch}? m again confirms");
            return Ok(());
        }
        self.merge_armed = None;
        match self.req(Command::MergeTicket { id: ticket }) {
            Response::Merge { outcome, detail } => {
                self.status = match outcome {
                    MergeOutcome::Merged | MergeOutcome::AlreadyMerged => detail,
                    MergeOutcome::Conflicts => {
                        format!("{detail} ∙ M sends the merge to the agent")
                    }
                    MergeOutcome::Refused => detail,
                };
            }
            Response::Err { message } => self.status = message,
            _ => {}
        }
        self.refresh()
    }

    fn merge_to_agent_key(&mut self, ticket: ulid::Ulid) -> Result<()> {
        match self.req(Command::MergeToAgent { id: ticket }) {
            Response::Ok => self.status = "merge request sent to the agent".into(),
            Response::Err { message } => self.status = message,
            _ => {}
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

    fn key_move(&mut self, code: KeyCode, ticket: ulid::Ulid, col: usize, idx: usize) -> Result<()> {
        let cols = self.columns();
        match code {
            KeyCode::Esc => self.mode = Mode::Normal, // total cancel (M2 rule)
            KeyCode::Char('h') | KeyCode::Left => {
                let col = col.saturating_sub(1);
                let n = self.ghost_len(&cols, col, ticket);
                self.mode = Mode::Move { ticket, col, idx: idx.min(n) };
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let col = (col + 1).min(cols.len().saturating_sub(1));
                let n = self.ghost_len(&cols, col, ticket);
                self.mode = Mode::Move { ticket, col, idx: idx.min(n) };
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let n = self.ghost_len(&cols, col, ticket);
                self.mode = Mode::Move { ticket, col, idx: (idx + 1).min(n) };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::Move { ticket, col, idx: idx.saturating_sub(1) };
            }
            KeyCode::Enter => {
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
            }
            _ => {}
        }
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
    /// it — the transcript survives the process) unless `x` dismissed it.
    /// Older corpses re-import through the drawer.
    pub fn rail_sessions(&self, ticket: ulid::Ulid) -> Vec<&mesimon_core::board::SessionRecord> {
        let corpse = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.ticket == ticket
                    && s.kind == SessionKind::Claude
                    && matches!(s.state, SessionState::Exited { reason } if reason != ExitReason::Killed)
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
            // when it is ready (a BoardChanged follows).
            Response::Provisioning => {
                self.status = "provisioning worktree ∙ session starts when ready".into();
                self.refresh()?;
            }
            Response::Err { message } => self.status = message,
            _ => self.refresh()?,
        }
        Ok(())
    }

    fn focus_session(&mut self, sid: uuid::Uuid) -> Result<()> {
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
                        self.focused_session_hint = Some(sid);
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
            }
            Response::Gate { passed: false, attach_argv: Some(argv) } => {
                self.pending_attach = Some(argv);
                self.pending_gate_then = Some(sid);
            }
            Response::Err { message } => self.status = message,
            _ => {}
        }
        Ok(())
    }

    /// Called by the main loop after a handover returns.
    pub fn after_handover(&mut self) -> Result<()> {
        if let Some(sid) = self.pending_gate_then.take() {
            // Detaching from the gate session IS the proof (D20).
            self.send(Command::GatePassed)?;
            match self.req(Command::FocusStart { session: sid }) {
                Response::Attach { argv } => {
                    self.pending_attach = Some(argv);
                    self.focused_session_hint = Some(sid);
                    return Ok(());
                }
                Response::Err { message } => self.status = message,
                _ => {}
            }
        } else if let Some(sid) = self.focused_session_hint.take() {
            self.send(Command::FocusEnd { session: sid })?;
            self.refresh()?;
            // Unfocus lands where a choice remains: the ticket screen when the
            // ticket still holds several sessions, otherwise the board with
            // the ticket selected (a one-session screen is a dead stop).
            if let Some(rec) = self.board.sessions.iter().find(|s| s.id == sid) {
                let ticket = rec.ticket;
                self.select_ticket(ticket);
                let rail = self.rail_sessions(ticket);
                if rail.len() > 1 {
                    let idx = rail.iter().position(|s| s.id == sid).unwrap_or(0);
                    self.screen = Screen::Ticket { ticket, rail_idx: idx };
                } else {
                    self.to_board();
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
    }

    impl Transport for FakeTransport {
        fn request(&mut self, command: Command) -> Result<Response> {
            match command {
                Command::Snapshot => Ok(Response::Board {
                    board: self.board.clone(),
                    grace: self.grace.clone(),
                    external: self.external.clone(),
                    resources: self.resources.clone(),
                    worktrees: Vec::new(),
                }),
                // Append-only move (before ignored): enough for the key tests,
                // which only exercise `before: None`.
                Command::MoveTicket { id, column, before: _ } => {
                    let order = self
                        .board
                        .column_tickets(&column)
                        .iter()
                        .rfind(|t| t.id != id)
                        .map(|t| format!("{}~", t.order))
                        .unwrap_or_else(|| "~".into());
                    if let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == id) {
                        t.column = column;
                        t.order = order;
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
            let fake = FakeTransport {
                board,
                grace: vec![],
                external: vec![],
                resources: Resources::default(),
            };
            App::new(Box::new(fake), PathBuf::from("/repo/kanban-tui"), theme)
                .expect("fake transport snapshot")
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

    fn app_three_columns() -> App {
        let mut b = Board::default();
        for (i, name) in ["todo", "doing", "done"].iter().enumerate() {
            b.columns.push(Column { name: (*name).into(), order: format!("{i}") });
        }
        b.tickets.push(ticket(1, "todo", "a"));
        b.tickets.push(ticket(2, "todo", "b"));
        b.tickets.push(ticket(3, "done", "a"));
        App::for_test(b, Theme::new(Flavor::Graphite, Profile::TrueColor))
    }

    #[test]
    fn gt_moves_selected_ticket_one_column_right_and_follows() {
        let mut app = app_three_columns();
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
        app.handle_key(KeyCode::Char('>'), KeyModifiers::NONE).unwrap();
        let doing: Vec<_> = app.board.column_tickets("doing").iter().map(|t| t.id).collect();
        assert_eq!(doing, vec![ulid::Ulid(1)]);
        assert_eq!(app.cursor_col, 1);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
    }

    #[test]
    fn gt_appends_to_end_of_target_column() {
        let mut app = app_three_columns();
        app.cursor_col = 2; // "done", holds ticket 3
        app.handle_key(KeyCode::Char('<'), KeyModifiers::NONE).unwrap();
        // no wrap involved: done -> doing
        let doing: Vec<_> = app.board.column_tickets("doing").iter().map(|t| t.id).collect();
        assert_eq!(doing, vec![ulid::Ulid(3)]);
        // now move it into todo, which already has 1 and 2 — lands last
        app.handle_key(KeyCode::Char('<'), KeyModifiers::NONE).unwrap();
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2), ulid::Ulid(3)]);
        assert_eq!((app.cursor_col, app.cursor_row), (0, 2));
    }

    #[test]
    fn gt_cycles_off_last_column_to_first() {
        let mut app = app_three_columns();
        app.cursor_col = 2; // "done", ticket 3
        app.handle_key(KeyCode::Char('>'), KeyModifiers::NONE).unwrap();
        let todo: Vec<_> = app.board.column_tickets("todo").iter().map(|t| t.id).collect();
        assert_eq!(todo, vec![ulid::Ulid(1), ulid::Ulid(2), ulid::Ulid(3)]);
        assert_eq!(app.cursor_col, 0);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(3)));
    }

    #[test]
    fn lt_cycles_off_first_column_to_last() {
        let mut app = app_three_columns();
        app.handle_key(KeyCode::Char('<'), KeyModifiers::NONE).unwrap();
        let done: Vec<_> = app.board.column_tickets("done").iter().map(|t| t.id).collect();
        assert_eq!(done, vec![ulid::Ulid(3), ulid::Ulid(1)]);
        assert_eq!(app.cursor_col, 2);
        assert_eq!(app.selected_ticket().map(|t| t.id), Some(ulid::Ulid(1)));
    }

    #[test]
    fn gt_on_empty_column_is_a_noop() {
        let mut app = app_three_columns();
        app.cursor_col = 1; // "doing" is empty
        app.handle_key(KeyCode::Char('>'), KeyModifiers::NONE).unwrap();
        assert!(app.board.column_tickets("doing").is_empty());
        assert_eq!(app.board.column_tickets("todo").len(), 2);
    }
}
