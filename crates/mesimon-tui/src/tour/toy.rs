//! The practice board's daemon (T-763): a board in memory that answers the
//! wire the way the real daemon does for the moves a lesson teaches, and
//! writes nowhere. It runs no agent, opens no pane and reads no repository;
//! a lesson about agents plays their part itself, by editing the world
//! (`World::session`) the way the hooks would have.
//!
//! A command the toy has no answer for is refused in words that say so, so
//! a key tried on the practice board that would reach outside it does
//! nothing worse than say it belongs to a real board.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Result;
use mesimon_core::board::{
    Archived, Board, NoteMeta, SessionKind, SessionRecord, SessionState, Ticket,
};
use mesimon_core::command::{Command, PrItem, Response, WorktreeItem};
use mesimon_core::fracindex;

use crate::client::Transport;

/// What the practice board holds: the board, the notes' bodies, and the
/// facts a snapshot carries beside the board.
#[derive(Default)]
pub(crate) struct World {
    pub board: Board,
    pub notes: HashMap<ulid::Ulid, String>,
    pub worktrees: Vec<WorktreeItem>,
    pub prs: Vec<PrItem>,
    pub external: Vec<mesimon_core::command::ExternalItem>,
    pub automation: mesimon_core::command::AutomationStatus,
    pub machine_tiers: mesimon_core::tier::MachineTiers,
    /// What each agent last said, by its session's transcript path: the
    /// practice board has no transcript, so the card reads these (T-816).
    pub peeks: HashMap<String, String>,
    /// Something changed that the board has not fetched yet.
    pub changed: bool,
    /// The commands the toy refused, by name: what a lesson's keys reached
    /// for past the practice board, for the tests to read.
    pub refused: Vec<String>,
    /// The ids the toy mints, counting up from here: stable, so a lesson's
    /// replay draws the same frames every time.
    serial: u128,
}

pub(crate) type Shared = Rc<RefCell<World>>;

impl World {
    pub fn new() -> Self {
        let mut board = Board::with_default_columns();
        board.next_key = 0;
        // A vocabulary to wear: the starter tags a fresh board is offered,
        // and a second group beside them.
        for (name, color) in mesimon_core::board::STARTER_TAGS {
            let _ = board.register_tag(mesimon_core::board::STARTER_GROUP, name);
            let _ = board.set_tag_color(mesimon_core::board::STARTER_GROUP, name, color);
        }
        for (name, color) in [("frontend", 4), ("backend", 8)] {
            let _ = board.register_tag(2, name);
            let _ = board.set_tag_color(2, name, color);
        }
        // Two tiers beside the built-in one, as a person might make them.
        let tier = |id: &str, name: &str, model: &str, effort, description: &str| {
            mesimon_core::tier::Tier {
                id: id.into(),
                name: name.into(),
                provider: mesimon_core::board::AgentProvider::ClaudeCode,
                model: model.into(),
                effort,
                description: description.into(),
            }
        };
        let machine_tiers = mesimon_core::tier::MachineTiers {
            default_tier: None,
            tiers: vec![
                tier(
                    "01PRACTICEQUICK00000000000",
                    "quick",
                    "sonnet",
                    mesimon_core::tier::Effort::Low,
                    "renames, docs, one-file fixes",
                ),
                tier(
                    "01PRACTICEDEEP000000000000",
                    "deep",
                    "opus",
                    mesimon_core::tier::Effort::High,
                    "design work and hard bugs",
                ),
            ],
        };
        Self { board, machine_tiers, serial: 1, ..Default::default() }
    }

    fn mint_id(&mut self) -> u128 {
        self.serial += 1;
        self.serial
    }

    /// A ticket at the bottom of `column`, with the next key.
    pub fn ticket(&mut self, column: &str, title: &str) -> ulid::Ulid {
        let last =
            self.board.column_tickets(column).last().map(|t| t.order.clone()).unwrap_or_default();
        let order = fracindex::between(&last, "");
        self.board.next_key += 1;
        let id = ulid::Ulid(self.mint_id());
        self.board.tickets.push(blank_ticket(
            id,
            format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
            title,
            column,
            order,
        ));
        self.changed = true;
        id
    }

    /// Give `ticket` a tag, registering the name as wearing one does.
    pub fn tag(&mut self, ticket: ulid::Ulid, group: u8, name: &str) {
        let _ = self.board.register_tag(group, name);
        if let Some(t) = self.board.ticket_mut(ticket) {
            t.set_tag(group, Some(name.to_string()));
        }
        self.changed = true;
    }

    /// Write `ticket`'s description (its first note).
    pub fn describe(&mut self, ticket: ulid::Ulid, text: &str) {
        let id = ulid::Ulid(self.mint_id());
        if let Some(t) = self.board.ticket_mut(ticket) {
            t.notes.insert(0, note_meta(id, text));
        }
        self.notes.insert(id, text.to_string());
        self.changed = true;
    }

    /// Put an agent of `kind` on `ticket` in `state`, or move the one there
    /// to it: the part the hooks play on a real board.
    pub fn session(&mut self, ticket: ulid::Ulid, kind: SessionKind, state: SessionState) {
        let now = mesimon_core::clock::now_ms();
        if let Some(s) =
            self.board.sessions.iter_mut().find(|s| s.ticket == ticket && s.kind == kind)
        {
            s.state = state.clone();
            s.state_changed_at = Some(now);
        } else {
            let id = uuid::Uuid::from_u128(self.mint_id());
            let mut s = SessionRecord::new(
                id,
                kind,
                ticket,
                vec!["claude".into()],
                "/practice".into(),
                state.clone(),
            );
            s.state_changed_at = Some(now);
            self.board.sessions.push(s);
        }
        self.changed = true;
        // The column's own rules move the card, as the daemon's automove
        // does on the same edge.
        if kind.is_agent() {
            let dest =
                self.board.ticket(ticket).and_then(|t| self.board.column(&t.column)).and_then(
                    |c| {
                        mesimon_core::automove::automove(
                            &c.settings,
                            &state,
                            mesimon_core::board::Confidence::High,
                        )
                        .map(str::to_string)
                    },
                );
            if let Some(dest) = dest {
                self.arrive(ticket, &dest);
            }
        }
    }

    /// `ticket`'s agent says `text`: its card's peek reads it (T-816).
    pub fn says(&mut self, ticket: ulid::Ulid, text: &str) {
        let Some(s) = self.board.sessions.iter_mut().find(|s| s.ticket == ticket) else { return };
        let path = format!("/practice/{}.jsonl", s.id);
        s.transcript_path = Some(path.clone());
        self.peeks.insert(path, text.to_string());
        self.changed = true;
    }

    /// A worktree for `ticket` with `ahead` commits on its branch.
    pub fn worktree(&mut self, ticket: ulid::Ulid, ahead: u32) {
        let Some(t) = self.board.ticket_mut(ticket) else { return };
        t.workspace = Some(mesimon_core::board::WorkspaceStrategy::Worktree);
        let branch = format!("msmn/{}-{}", t.short_key, mesimon_core::workspace::slug(&t.title));
        self.worktrees.retain(|w| w.ticket != ticket);
        self.worktrees.push(WorktreeItem {
            ticket,
            branch,
            status: "attached".into(),
            merged: false,
            merged_in: String::new(),
            merged_oid: String::new(),
            conflict: false,
            ahead,
            needs_rebase: false,
            counted: String::new(),
            detail: None,
            path: None,
            repos: Vec::new(),
        });
        self.changed = true;
    }

    /// `ticket`'s branch is on the default branch now.
    pub fn merged(&mut self, ticket: ulid::Ulid) {
        for w in self.worktrees.iter_mut().filter(|w| w.ticket == ticket) {
            w.merged = true;
            w.ahead = 0;
        }
        self.changed = true;
    }

    /// The ticket titled `title`, for a lesson's own steps.
    pub fn find(&self, title: &str) -> Option<ulid::Ulid> {
        self.board.tickets.iter().find(|t| t.title == title).map(|t| t.id)
    }

    /// Move `ticket` to the top of `column`, as an automatic move lands.
    pub fn arrive(&mut self, ticket: ulid::Ulid, column: &str) {
        let first =
            self.board.column_tickets(column).first().map(|t| t.order.clone()).unwrap_or_default();
        let order = fracindex::between("", &first);
        if let Some(t) = self.board.ticket_mut(ticket) {
            let from = std::mem::replace(&mut t.column, column.to_string());
            t.previous_column = Some(mesimon_core::board::ColumnStay { column: from, seconds: 0 });
            t.order = order;
        }
        self.changed = true;
    }

    /// What the board would draw after this world: the snapshot.
    fn snapshot(&self) -> Response {
        // As the daemon's: archived tickets stay home (T-842), asked for.
        let (board, archived) = self.board.for_snapshot();
        Response::Board {
            board,
            archived,
            grace: Vec::new(),
            external: self.external.clone(),
            external_scanning: false,
            resources: Default::default(),
            worktrees: self.worktrees.clone(),
            notices: Vec::new(),
            shell_env: Default::default(),
            git: Default::default(),
            pending: Vec::new(),
            automation: self.automation.clone(),
            claude_md: Default::default(),
            claude_default_mode: None,
            status_top: false,
            team: Default::default(),
            mesophon: Default::default(),
            terminals: Vec::new(),
            crown_touches: Vec::new(),
            crown_said: Vec::new(),
            machine_tiers: self.machine_tiers.clone(),
            usage: Default::default(),
            costs: Vec::new(),
            prs: self.prs.clone(),
            forge: Default::default(),
            links: Vec::new(),
            board_touches: Vec::new(),
        }
    }

    /// The order a ticket takes landing in `column` above `before` (the
    /// bottom with none), leaving `moving` out of the reckoning.
    fn order_before(&self, column: &str, before: Option<ulid::Ulid>, moving: ulid::Ulid) -> String {
        let rest: Vec<&Ticket> =
            self.board.column_tickets(column).into_iter().filter(|t| t.id != moving).collect();
        match before.and_then(|b| rest.iter().position(|t| t.id == b)) {
            Some(i) => {
                let prev = if i == 0 { "" } else { rest[i - 1].order.as_str() };
                fracindex::between(prev, &rest[i].order)
            }
            None => fracindex::between(rest.last().map_or("", |t| t.order.as_str()), ""),
        }
    }

    fn answer(&mut self, command: Command) -> Response {
        match command {
            Command::Snapshot | Command::Subscribe => self.snapshot(),
            Command::ArchivedTickets => Response::Tickets {
                tickets: self.board.archived_tickets().into_iter().cloned().collect(),
            },
            Command::CreateTicket { column, title, .. } => self.create(&column, &title, ""),
            Command::CreateTicketWithNote { column, title, text, tags, .. } => {
                let made = self.create(&column, &title, &text);
                if let Response::Created { id } = made {
                    for tag in tags {
                        self.tag(id, tag.group, &tag.name);
                    }
                }
                made
            }
            Command::RenameTicket { id, title } => {
                let title = mesimon_core::board::sanitize_title(&title);
                if title.trim().is_empty() {
                    return refuse("a ticket needs a title");
                }
                self.edit(id, |t| t.title = title)
            }
            Command::MoveTicket { id, column, before } => {
                if self.board.column(&column).is_none() {
                    return refuse(&format!("no such column: {column}"));
                }
                let order = self.order_before(&column, before, id);
                self.edit(id, |t| {
                    if t.column != column {
                        let from = std::mem::replace(&mut t.column, column);
                        t.previous_column =
                            Some(mesimon_core::board::ColumnStay { column: from, seconds: 0 });
                    }
                    t.order = order;
                })
            }
            Command::SetTag { id, group, name } => {
                if let Some(n) = name.as_deref() {
                    let _ = self.board.register_tag(group, n);
                }
                self.edit(id, |t| t.set_tag(group, name))
            }
            Command::RegisterTag { group, name } => {
                self.changed = true;
                match self.board.register_tag(group, &name) {
                    Ok(()) => Response::Ok,
                    Err(message) => Response::Err { message },
                }
            }
            Command::ArchiveTicket { id } => self.edit(id, |t| {
                t.archived = Some(Archived {
                    at: "@0".into(),
                    by: "local".into(),
                    until: None,
                    needs_you: false,
                })
            }),
            Command::UnarchiveTicket { id } | Command::RestoreTicket { id } => {
                self.edit(id, |t| t.archived = None)
            }
            Command::DeleteTicket { id, .. } => {
                if self.board.ticket(id).is_none() {
                    return refuse("no such ticket");
                }
                self.board.tickets.retain(|t| t.id != id);
                self.board.sessions.retain(|s| s.ticket != id);
                self.changed = true;
                Response::Ok
            }
            Command::DuplicateTicket { id } => {
                let Some(source) = self.board.ticket(id).cloned() else {
                    return refuse("no such ticket");
                };
                let column = self.board.column_tickets(&source.column);
                let next = column.iter().position(|t| t.id == id).and_then(|i| column.get(i + 1));
                let order =
                    fracindex::between(&source.order, next.map_or("", |t| t.order.as_str()));
                self.board.next_key += 1;
                let new = ulid::Ulid(self.mint_id());
                let mut copy = blank_ticket(
                    new,
                    format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
                    &source.title,
                    &source.column,
                    order,
                );
                copy.tags = source.tags.clone();
                for n in &source.notes {
                    let body = self.notes.get(&n.id).cloned().unwrap_or_default();
                    let nid = ulid::Ulid(self.mint_id());
                    copy.notes.push(note_meta(nid, &body));
                    self.notes.insert(nid, body);
                }
                self.board.tickets.push(copy);
                self.changed = true;
                Response::Created { id: new }
            }
            Command::ReadNote { ticket, note } => {
                let meta = self.board.ticket(ticket).and_then(|t| t.note(note)).cloned();
                match (meta, self.notes.get(&note)) {
                    (Some(meta), Some(text)) => Response::Note { text: text.clone(), meta },
                    _ => refuse("no such note"),
                }
            }
            // The picker's corpus (T-882): the bodies the toy holds, as the
            // daemon would answer, so a search in the tour reaches the notes.
            Command::ReadNotes { notes } => Response::Notes {
                notes: notes
                    .iter()
                    .filter_map(|r| {
                        let meta = self.board.ticket(r.ticket)?.note(r.note)?;
                        let text = self.notes.get(&r.note)?.clone();
                        Some(mesimon_core::command::NoteBody { note: r.note, rev: meta.rev, text })
                    })
                    .collect(),
            },
            Command::WriteNote { ticket, note, text, .. }
            | Command::SaveNoteWithAttachments { ticket, note, text, .. } => {
                self.write_note(ticket, note, text)
            }
            // The board's own rules, as the daemon applies them.
            Command::AddColumn { name, after } => {
                let Some(name) = mesimon_core::board::sanitize_column_name(&name) else {
                    return refuse("a column needs a name");
                };
                let done = self.board.add_column(name, after.as_deref());
                self.applied(done)
            }
            Command::RenameColumn { name, to } => {
                let Some(to) = mesimon_core::board::sanitize_column_name(&to) else {
                    return refuse("a column needs a name");
                };
                let done = self.board.rename_column(&name, &to).map(drop);
                self.applied(done)
            }
            Command::SetColumnSettings { name, settings } => {
                let done = self.board.set_column_settings(&name, settings);
                self.applied(done)
            }
            Command::SetWorkspace { id, workspace } => self.edit(id, |t| t.workspace = workspace),
            Command::SetManualMerge { id, on } => self.edit(id, |t| t.manual_merge = on),
            Command::SetTicketTier { id, tier } => self.edit(id, |t| t.tier = tier),
            Command::CrownTicket { id } => {
                if self.board.ticket(id).is_none() {
                    return refuse("no such ticket");
                }
                self.board.crown = Some(id);
                self.changed = true;
                Response::Ok
            }
            Command::Uncrown => {
                self.board.crown = None;
                self.changed = true;
                Response::Ok
            }
            // The merge, played: the branch lands and the card says so.
            Command::MergeTicket { id } => {
                let ahead = self.worktrees.iter().find(|w| w.ticket == id).map(|w| w.ahead);
                match ahead {
                    Some(n) if n > 0 => {
                        self.merged(id);
                        Response::Merge {
                            outcome: mesimon_core::command::MergeOutcome::Merged,
                            detail: format!("merged {n} commit(s)"),
                            notified: false,
                        }
                    }
                    _ => refuse("nothing to merge"),
                }
            }
            // Adopting a conversation that started outside: a ticket of its
            // own, its agent seated on it.
            Command::AttachExternal { claude_session_id, .. }
            | Command::ResumeExternal { claude_session_id, .. } => {
                let Some(i) = self.external.iter().position(|e| e.id == claude_session_id) else {
                    return refuse("no such session");
                };
                let e = self.external.remove(i);
                let title = e.name.or(e.preview).unwrap_or_else(|| "Adopted session".into());
                let id = self.ticket("TODO", &title);
                let state =
                    SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
                self.session(id, SessionKind::Claude, state);
                let sid = self.board.sessions.last().map(|s| s.id).unwrap_or_default();
                Response::Spawned { id: sid, fresh: false }
            }
            // A start, played: the agent is up and working at once. The
            // lesson that teaches starting says the rest.
            Command::SpawnSession { ticket, kind, .. } => {
                if self.board.ticket(ticket).is_none() {
                    return refuse("no such ticket");
                }
                self.session(ticket, kind, SessionState::Running);
                let id = self
                    .board
                    .sessions
                    .iter()
                    .find(|s| s.ticket == ticket && s.kind == kind)
                    .map(|s| s.id)
                    .unwrap_or_default();
                Response::Spawned { id, fresh: true }
            }
            // A person's words to the agent (T-816): it takes them and
            // starts a turn, as a pane would.
            Command::PromptSession { ticket, .. } => {
                if self.board.ticket(ticket).is_none() {
                    return refuse("no such ticket");
                }
                self.session(ticket, SessionKind::Claude, SessionState::Running);
                Response::Asked {
                    sent: 1,
                    woke: 0,
                    started: 0,
                    queued: 0,
                    skipped: 0,
                    failed: 0,
                    accepts: 0,
                }
            }
            // The board's own settings, pushed as it opens: the practice
            // board keeps none of them.
            Command::SetAutomation { .. }
            | Command::SetUsageWants { .. }
            | Command::SetStatusLine { .. }
            | Command::RefreshUsage { .. } => Response::Ok,
            // The census of sessions started elsewhere: the lesson's own.
            Command::RescanExternal => self.snapshot(),
            // What the board says it saw: nothing to keep on a practice board.
            Command::SeenTicket { .. }
            | Command::OpenedTicket { .. }
            | Command::LowerHand { .. }
            | Command::FocusQuiet
            | Command::SetFollowUpMode { .. } => Response::Ok,
            Command::GateStatus => Response::Gate { passed: true, attach_argv: None },
            // Going to an agent's session leaves the board: on the practice
            // board the card is the whole of it.
            Command::FocusStart { .. } | Command::OpenTerminal { .. } => {
                refuse("on your own board this opens the agent's session")
            }
            Command::Hello { .. } => Response::Hello {
                version: mesimon_core::command::PROTOCOL_VERSION,
                daemon_pid: 0,
                build: String::new(),
                exe_stamp: None,
                detached: false,
                repo_root: String::new(),
                crown: None,
            },
            other => {
                let name = format!("{other:?}");
                self.refused.push(name.split([' ', '{', '(']).next().unwrap_or("").to_string());
                refuse("that reaches past the practice board ∙ try it on your own")
            }
        }
    }

    /// A board edit's answer: the board changed, or why not.
    fn applied(&mut self, done: std::result::Result<(), String>) -> Response {
        match done {
            Ok(()) => {
                self.changed = true;
                Response::Ok
            }
            Err(message) => refuse(&message),
        }
    }

    fn create(&mut self, column: &str, title: &str, text: &str) -> Response {
        if self.board.column(column).is_none() {
            return refuse(&format!("no such column: {column}"));
        }
        if let Some(why) = self.board.filing_refusal(column) {
            return refuse(&why);
        }
        let title = mesimon_core::board::sanitize_title(title);
        if title.trim().is_empty() {
            return refuse("a ticket needs a title");
        }
        let id = self.ticket(column, &title);
        if !text.trim().is_empty() {
            self.describe(id, text);
        }
        Response::Created { id }
    }

    fn edit(&mut self, id: ulid::Ulid, change: impl FnOnce(&mut Ticket)) -> Response {
        let Some(t) = self.board.ticket_mut(id) else { return refuse("no such ticket") };
        change(t);
        self.changed = true;
        Response::Ok
    }

    fn write_note(
        &mut self,
        ticket: ulid::Ulid,
        note: Option<ulid::Ulid>,
        text: String,
    ) -> Response {
        if self.board.ticket(ticket).is_none() {
            return refuse("no such ticket");
        }
        self.changed = true;
        if text.trim().is_empty() {
            let Some(id) = note else { return refuse("nothing to save") };
            if let Some(t) = self.board.ticket_mut(ticket) {
                t.notes.retain(|n| n.id != id);
            }
            self.notes.remove(&id);
            return Response::NoteWritten { note: None };
        }
        let name = mesimon_core::board::note_name(&text);
        let id = match note {
            Some(id) => {
                let Some(t) = self.board.ticket_mut(ticket) else {
                    return refuse("no such ticket");
                };
                let Some(n) = t.notes.iter_mut().find(|n| n.id == id) else {
                    return refuse("no such note");
                };
                n.rev += 1;
                n.name = name;
                id
            }
            None => {
                let id = ulid::Ulid(self.mint_id());
                if let Some(t) = self.board.ticket_mut(ticket) {
                    t.notes.push(note_meta(id, &text));
                }
                id
            }
        };
        self.notes.insert(id, text);
        Response::NoteWritten { note: Some(id) }
    }
}

fn refuse(message: &str) -> Response {
    Response::Err { message: message.to_string() }
}

fn note_meta(id: ulid::Ulid, text: &str) -> NoteMeta {
    NoteMeta {
        id,
        name: mesimon_core::board::note_name(text),
        rev: 1,
        created_at: "@0".into(),
        created_by: "local".into(),
        edited_at: "@0".into(),
        edited_by: "local".into(),
    }
}

fn blank_ticket(id: ulid::Ulid, key: String, title: &str, column: &str, order: String) -> Ticket {
    let now = format!("@{}", mesimon_core::clock::now_ms() / 1000);
    Ticket {
        id,
        short_key: key,
        title: title.to_string(),
        column: column.to_string(),
        order,
        created_at: now.clone(),
        created_by: "local".into(),
        created_from: None,
        entered_at: Some(now),
        previous_column: None,
        picked: None,
        woke_at: None,
        manual_merge: false,
        execution_policy: Default::default(),
        tier: None,
        import_origin: None,
        envelope: None,
        raised: None,
        workspace: None,
        tags: Vec::new(),
        notes: Vec::new(),
        archived: None,
    }
}

/// The practice board's end of the wire: the world, shared with the tour
/// that plays the agents' part in it.
pub(crate) struct Toy(pub Shared);

impl Transport for Toy {
    fn request(&mut self, command: Command) -> Result<Response> {
        Ok(self.0.borrow_mut().answer(command))
    }

    fn poll_event(&mut self) -> bool {
        std::mem::take(&mut self.0.borrow_mut().changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_lands_above_the_card_it_names_and_at_the_bottom_with_none() {
        let mut w = World::new();
        let a = w.ticket("TODO", "a");
        let b = w.ticket("TODO", "b");
        let c = w.ticket("TODO", "c");
        w.answer(Command::MoveTicket { id: c, column: "TODO".into(), before: Some(a) });
        let titles: Vec<&str> =
            w.board.column_tickets("TODO").iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["c", "a", "b"]);
        w.answer(Command::MoveTicket { id: c, column: "TODO".into(), before: None });
        let titles: Vec<&str> =
            w.board.column_tickets("TODO").iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["a", "b", "c"]);
        let _ = b;
    }

    #[test]
    fn a_column_that_refuses_new_tickets_refuses_them_here_too() {
        let mut w = World::new();
        let r = w.answer(Command::CreateTicket {
            column: "IN PROGRESS".into(),
            title: "x".into(),
            workspace: None,
            tier: None,
        });
        assert!(matches!(r, Response::Err { .. }), "{r:?}");
    }

    #[test]
    fn nothing_past_the_practice_board_is_done() {
        let mut w = World::new();
        let r = w.answer(Command::Shutdown);
        assert!(matches!(r, Response::Err { .. }));
    }
}
