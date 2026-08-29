//! Board app: state, keymap (M1 subset — the full 04 keymap lands in M2/M6),
//! MOVE mode with client-side ghost (07 §7 core rules), focus flow with GATE.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use mesimon_core::board::{Board, SessionKind, SessionState, Ticket};
use mesimon_core::command::{Command, ExternalItem, GraceItem, Resources, Response};
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::client::Client;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    /// MOVE: ghost position tracked client-side, committed on Enter (M-rules).
    Move { ticket: ulid::Ulid, col: usize, idx: usize },
    Input { purpose: InputPurpose, buffer: String },
    /// Session picker: a ticket has more than one live session.
    Pick { ticket: ulid::Ulid, idx: usize },
    /// External drawer: discovered foreign sessions (19 §4 tier 1).
    External { idx: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputPurpose {
    Create,
    Rename { id: ulid::Ulid },
}

pub struct App {
    pub client: Client,
    pub repo_root: PathBuf,
    pub board: Board,
    pub grace: Vec<GraceItem>,
    pub external: Vec<ExternalItem>,
    pub resources: Resources,
    /// The drawer row whose resume was refused as running-elsewhere — a
    /// second R on the same row sends the confirm override.
    resume_refused: Option<uuid::Uuid>,
    pub cursor_col: usize,
    pub cursor_row: usize,
    pub mode: Mode,
    pub status: String,
    pub quit: bool,
    /// Set when the user asked to focus: the main loop performs the handover
    /// outside the render loop.
    pub pending_attach: Option<Vec<String>>,
    pub pending_gate_then: Option<uuid::Uuid>,
    /// The session a running handover holds focus on — released on return.
    focused_session_hint: Option<uuid::Uuid>,
}

impl App {
    pub fn new(mut client: Client, repo_root: PathBuf) -> Result<Self> {
        let (board, grace, external, resources) = fetch(&mut client)?;
        Ok(Self {
            client,
            repo_root,
            board,
            grace,
            external,
            resources,
            resume_refused: None,
            cursor_col: 0,
            cursor_row: 0,
            mode: Mode::Normal,
            status: String::new(),
            quit: false,
            pending_attach: None,
            pending_gate_then: None,
            focused_session_hint: None,
        })
    }

    pub fn refresh(&mut self) -> Result<()> {
        let (board, grace, external, resources) = fetch(&mut self.client)?;
        self.board = board;
        self.grace = grace;
        self.external = external;
        self.resources = resources;
        self.clamp_cursor();
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

    /// Poll one terminal event; returns whether a redraw is needed.
    pub fn tick(&mut self) -> Result<bool> {
        // Async board-changed events from the daemon.
        let mut dirty = false;
        while self.client.events.try_recv().is_ok() {
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
        // Tab / Shift+Tab: next/previous needs-you card. Global, BEFORE the
        // mode dispatch (04/07 §21: the only exception is a text field).
        if !matches!(self.mode, Mode::Input { .. }) {
            match key.code {
                KeyCode::Tab => {
                    self.cycle_attention(false);
                    return Ok(true);
                }
                KeyCode::BackTab => {
                    self.cycle_attention(true);
                    return Ok(true);
                }
                _ => {}
            }
        }
        match self.mode.clone() {
            Mode::Normal => self.key_normal(key.code, key.modifiers)?,
            Mode::Move { ticket, col, idx } => self.key_move(key.code, ticket, col, idx)?,
            Mode::Pick { ticket, idx } => self.key_pick(key.code, ticket, idx)?,
            Mode::External { idx } => self.key_external(key.code, idx)?,
            Mode::Input { purpose, mut buffer } => match key.code {
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
            },
        }
        Ok(true)
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
            KeyCode::Char('o') => {
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
            KeyCode::Char('s') => self.spawn_and_focus(SessionKind::Claude)?,
            KeyCode::Char('S') => self.spawn_and_focus(SessionKind::Bash)?,
            KeyCode::Char('e') => self.open_drawer()?,
            KeyCode::Char('Z') => {
                match self.client.request(Command::ReclaimAll)? {
                    Response::Reclaimed { slept, skipped } => {
                        self.status = match (slept, skipped) {
                            (0, 0) => "nothing to reclaim".into(),
                            (n, 0) => format!("slept {n}"),
                            (n, k) => format!("slept {n} · {k} not eligible"),
                        };
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.refresh()?;
            }
            KeyCode::Enter => self.focus_selected()?,
            _ => {}
        }
        Ok(())
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
                let claude_session_id = self.external[idx].claude_session_id;
                let Some(t) = self.selected_ticket() else {
                    self.status = "select a ticket first (Esc, move, e again)".into();
                    return Ok(());
                };
                let ticket = t.id;
                match self
                    .client
                    .request(Command::AttachExternal { claude_session_id, ticket })?
                {
                    Response::Spawned { .. } => {
                        self.status = "attached as external — observe-only until resumed".into();
                        self.mode = Mode::Normal;
                    }
                    Response::Err { message } => self.status = message,
                    _ => {}
                }
                self.refresh()?;
            }
            KeyCode::Char('R') | KeyCode::Enter => {
                let claude_session_id = self.external[idx].claude_session_id;
                let Some(t) = self.selected_ticket() else {
                    self.status = "select a ticket first (Esc, move, e again)".into();
                    return Ok(());
                };
                let ticket = t.id;
                let confirm = self.resume_refused == Some(claude_session_id);
                match self.client.request(Command::ResumeExternal {
                    claude_session_id,
                    ticket,
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

    /// Live sessions of a ticket, in spawn order.
    pub fn live_sessions_of(&self, ticket: ulid::Ulid) -> Vec<&mesimon_core::board::SessionRecord> {
        self.board
            .sessions
            .iter()
            .filter(|s| s.ticket == ticket && s.state.is_live())
            .collect()
    }

    fn spawn_and_focus(&mut self, kind: SessionKind) -> Result<()> {
        let Some(t) = self.selected_ticket() else { return Ok(()) };
        let ticket = t.id;
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

    fn focus_selected(&mut self) -> Result<()> {
        // Enter on nothing does nothing.
        let Some(t) = self.selected_ticket() else { return Ok(()) };
        let ticket = t.id;
        let live: Vec<uuid::Uuid> = self.live_sessions_of(ticket).iter().map(|s| s.id).collect();
        match live.len() {
            0 => self.status = "no live session on this ticket — s spawns claude, S bash".into(),
            1 => self.focus_session(live[0])?,
            _ => self.mode = Mode::Pick { ticket, idx: live.len() - 1 },
        }
        Ok(())
    }

    fn key_pick(&mut self, code: KeyCode, ticket: ulid::Ulid, idx: usize) -> Result<()> {
        let live: Vec<uuid::Uuid> = self.live_sessions_of(ticket).iter().map(|s| s.id).collect();
        if live.is_empty() {
            self.mode = Mode::Normal;
            return Ok(());
        }
        match code {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
            KeyCode::Char('j') | KeyCode::Down => {
                self.mode = Mode::Pick { ticket, idx: (idx + 1).min(live.len() - 1) };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.mode = Mode::Pick { ticket, idx: idx.saturating_sub(1) };
            }
            KeyCode::Enter => {
                let sid = live[idx.min(live.len() - 1)];
                self.mode = Mode::Normal;
                self.focus_session(sid)?;
            }
            KeyCode::Char('x') => {
                let sid = live[idx.min(live.len() - 1)];
                self.send(Command::KillSession { id: sid })?;
                let remaining = self.live_sessions_of(ticket).len();
                self.mode = if remaining > 1 {
                    Mode::Pick { ticket, idx: idx.min(remaining - 1) }
                } else {
                    Mode::Normal
                };
            }
            KeyCode::Char('z') => {
                // Sleep/wake toggle on the selected session.
                let sid = live[idx.min(live.len() - 1)];
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
            KeyCode::Char('p') => {
                let sid = live[idx.min(live.len() - 1)];
                let pinned = self
                    .board
                    .sessions
                    .iter()
                    .find(|s| s.id == sid)
                    .map(|s| !s.pinned_awake)
                    .unwrap_or(true);
                self.send(Command::PinAwake { id: sid, pinned })?;
                self.status =
                    if pinned { "pinned awake".into() } else { "unpinned".into() };
            }
            _ => {}
        }
        Ok(())
    }

    fn focus_session(&mut self, sid: uuid::Uuid) -> Result<()> {
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
            // Unfocus lands on the ticket's session list, not the board root —
            // the picker stands in for the ticket screen until it exists.
            if let Some(rec) = self.board.sessions.iter().find(|s| s.id == sid) {
                let ticket = rec.ticket;
                let live = self.live_sessions_of(ticket);
                if !live.is_empty() {
                    let idx = live.iter().position(|s| s.id == sid).unwrap_or(0);
                    self.select_ticket(ticket);
                    self.mode = Mode::Pick { ticket, idx };
                }
            }
            return Ok(());
        }
        self.refresh()
    }

    /// Jump the cursor to the next (or previous) card needing attention.
    /// Queue order: precedence rank, then longest-waiting (daemon-minted
    /// `waiting_since`), wrapping. Inert when nothing waits.
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

    /// Point the board cursor at a ticket (so Esc from the picker lands on it).
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

fn fetch(client: &mut Client) -> Result<(Board, Vec<GraceItem>, Vec<ExternalItem>, Resources)> {
    match client.request(Command::Snapshot)? {
        Response::Board { board, grace, external, resources } => {
            Ok((board, grace, external, resources))
        }
        other => anyhow::bail!("unexpected snapshot response: {other:?}"),
    }
}
