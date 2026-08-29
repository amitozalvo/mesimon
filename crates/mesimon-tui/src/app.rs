//! Board app: state, keymap (M1 subset — the full 04 keymap lands in M2/M6),
//! MOVE mode with client-side ghost (07 §7 core rules), focus flow with GATE,
//! and the M3.5 ticket screen (Enter opens it; the old session picker is its
//! SESSIONS rail now).

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use mesimon_core::board::{Board, Provenance, SessionKind, SessionState, Ticket};
use mesimon_core::command::{Command, ExternalItem, GraceItem, Resources, Response};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::client::Transport;
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
    Input { purpose: InputPurpose, buffer: String },
    /// External drawer: discovered foreign sessions (19 §4 tier 1).
    External { idx: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputPurpose {
    Create,
    Rename { id: ulid::Ulid },
}

pub struct App {
    pub client: Box<dyn Transport>,
    pub repo_root: PathBuf,
    pub board: Board,
    pub grace: Vec<GraceItem>,
    pub external: Vec<ExternalItem>,
    pub resources: Resources,
    pub theme: Theme,
    /// The drawer row whose resume was refused as running-elsewhere — a
    /// second R on the same row sends the confirm override.
    resume_refused: Option<uuid::Uuid>,
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
    /// Set when the user asked to focus: the main loop performs the handover
    /// outside the render loop.
    pub pending_attach: Option<Vec<String>>,
    pub pending_gate_then: Option<uuid::Uuid>,
    /// The session a running handover holds focus on — released on return.
    focused_session_hint: Option<uuid::Uuid>,
}

impl App {
    pub fn new(mut client: Box<dyn Transport>, repo_root: PathBuf, theme: Theme) -> Result<Self> {
        let (board, grace, external, resources) = fetch(client.as_mut())?;
        Ok(Self {
            client,
            repo_root,
            board,
            grace,
            external,
            resources,
            theme,
            resume_refused: None,
            screen: Screen::Board,
            cursor_col: 0,
            cursor_row: 0,
            mode: Mode::Normal,
            status: String::new(),
            quit: false,
            col_window: Cell::new(0),
            marquee: Cell::new(None),
            scroll_row: Cell::new(0),
            pending_attach: None,
            pending_gate_then: None,
            focused_session_hint: None,
        })
    }

    pub fn refresh(&mut self) -> Result<()> {
        let (board, grace, external, resources) = fetch(self.client.as_mut())?;
        self.board = board;
        self.grace = grace;
        self.external = external;
        self.resources = resources;
        self.clamp_cursor();
        self.clamp_screen();
        Ok(())
    }

    /// Take whatever board a command replied with (RescanExternal does this).
    fn absorb_board(&mut self, resp: Response) {
        if let Response::Board { board, grace, external, resources } = resp {
            self.board = board;
            self.grace = grace;
            self.external = external;
            self.resources = resources;
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

    /// A refresh can delete the ticket the ticket screen shows.
    fn clamp_screen(&mut self) {
        if let Screen::Ticket { ticket, rail_idx } = &self.screen {
            if self.board.ticket(*ticket).is_none() {
                self.screen = Screen::Board;
            } else {
                let n = self.rail_sessions(*ticket).len();
                let idx = (*rail_idx).min(n.saturating_sub(1));
                self.screen = Screen::Ticket { ticket: *ticket, rail_idx: idx };
            }
        }
    }

    /// Poll one terminal event; returns whether a redraw is needed.
    pub fn tick(&mut self) -> Result<bool> {
        // Async board-changed events from the daemon.
        let mut dirty = false;
        while self.client.poll_event() {
            dirty = true;
        }
        if dirty {
            self.refresh()?;
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
        if let Mode::Input { purpose, mut buffer } = self.mode.clone() {
            match code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Enter => {
                    self.mode = Mode::Normal;
                    self.commit_input(purpose, buffer)?;
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    self.mode = Mode::Input { purpose, buffer };
                }
                KeyCode::Char(c) => {
                    buffer.push(c);
                    self.mode = Mode::Input { purpose, buffer };
                }
                _ => {}
            }
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
                self.mode = Mode::Input { purpose: InputPurpose::Create, buffer: String::new() };
            }
            KeyCode::Char('r') => {
                if let Some(t) = self.selected_ticket() {
                    self.mode = Mode::Input {
                        purpose: InputPurpose::Rename { id: t.id },
                        buffer: t.title.clone(),
                    };
                }
            }
            KeyCode::Char('d') => {
                if let Some(t) = self.selected_ticket() {
                    let id = t.id;
                    self.send(Command::DeleteTicket { id })?;
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
            KeyCode::Char('Z') => {
                match self.client.request(Command::ReclaimAll)? {
                    Response::Reclaimed { slept, skipped } => {
                        self.status = match (slept, skipped) {
                            (0, 0) => "nothing to reclaim".into(),
                            (n, 0) => format!("slept {n}"),
                            (n, k) => format!("slept {n} ∙ {k} not eligible"),
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
            KeyCode::Esc | KeyCode::Char('q') => self.screen = Screen::Board,
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
                        buffer: t.title.clone(),
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
            KeyCode::Char('d') => {
                self.screen = Screen::Board;
                self.send(Command::DeleteTicket { id: ticket })?;
            }
            _ => {}
        }
        Ok(())
    }

    fn focus_kind_or_spawn(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Result<()> {
        let existing = self
            .rail_sessions(ticket)
            .iter()
            .find(|s| s.kind == kind)
            .map(|s| s.id);
        match existing {
            Some(sid) => self.focus_session(sid),
            None => self.spawn_and_focus(ticket, kind),
        }
    }

    /// `e`: rescan (lazy census — this is the only trigger) and open the drawer.
    fn open_drawer(&mut self) -> Result<()> {
        let resp = self.client.request(Command::RescanExternal)?;
        self.absorb_board(resp);
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
                match self
                    .client
                    .request(Command::AttachExternal { claude_session_id, ticket: None })?
                {
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
                match self.client.request(Command::ResumeExternal {
                    claude_session_id,
                    ticket: None,
                    confirm,
                })? {
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
            InputPurpose::Create => {
                let cols = self.columns();
                let column = cols.get(self.cursor_col).cloned().unwrap_or_default();
                self.send(Command::CreateTicket { column, title })?;
            }
            InputPurpose::Rename { id } => {
                self.send(Command::RenameTicket { id, title })?;
            }
        }
        Ok(())
    }

    /// Live sessions of a ticket, in spawn order. `Sleeping` is live-but-parked;
    /// only `Exited` drops out. This is the ticket screen's SESSIONS rail —
    /// fixed creation order, never resorted by activity (06 §7 R4).
    pub fn rail_sessions(&self, ticket: ulid::Ulid) -> Vec<&mesimon_core::board::SessionRecord> {
        self.board
            .sessions
            .iter()
            .filter(|s| s.ticket == ticket && s.state.is_live())
            .collect()
    }

    fn spawn_and_focus(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Result<()> {
        match self.client.request(Command::SpawnSession { ticket, kind })? {
            Response::Spawned { id } => {
                self.refresh()?;
                self.focus_session(id)?;
            }
            Response::Err { message } => self.status = message,
            _ => self.refresh()?,
        }
        Ok(())
    }

    fn focus_session(&mut self, sid: uuid::Uuid) -> Result<()> {
        // Enter means "get me into this session": paneless records (imported
        // observe-only, sleeping) resume first, then the focus flow runs.
        if let Some(rec) = self.board.sessions.iter().find(|s| s.id == sid) {
            let observe_only = rec.provenance == Provenance::Adopted && rec.argv.is_empty();
            let sleeping = matches!(rec.state, SessionState::Sleeping);
            if observe_only || sleeping {
                let cmd = if sleeping && rec.kind == SessionKind::Bash {
                    Command::WakeSession { id: sid }
                } else {
                    Command::ResumeSession { id: sid, confirm: self.resume_refused == Some(sid) }
                };
                match self.client.request(cmd)? {
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
        match self.client.request(Command::GateStatus)? {
            Response::Gate { passed: true, .. } => {
                match self.client.request(Command::FocusStart { session: sid })? {
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
            match self.client.request(Command::FocusStart { session: sid })? {
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
                    self.screen = Screen::Board;
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
        self.screen = Screen::Board;
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
        if let Response::Err { message } = self.client.request(command)? {
            self.status = message;
        }
        self.refresh()
    }
}

fn fetch(client: &mut dyn Transport) -> Result<(Board, Vec<GraceItem>, Vec<ExternalItem>, Resources)> {
    match client.request(Command::Snapshot)? {
        Response::Board { board, grace, external, resources } => {
            Ok((board, grace, external, resources))
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
                }),
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
