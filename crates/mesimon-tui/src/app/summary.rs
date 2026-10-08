//! A ticket's summary in the TUI (T-696): the rows its notes put under a
//! `Summary` heading, read off the note bodies and kept per `(note, rev)`,
//! the card's progress underline and open-card rows, and the `^j` dialog
//! that ticks a box, opens a row's line in its note or asks the agent
//! about it.
//!
//! Derived here and never on the snapshot, the links road (T-256) scaled to
//! every card: `poll_summaries` reads the bodies the cache lacks through
//! `Command::ReadNote`, a few per tick, and keeps only the parsed rows. A
//! tick rewrites the body it just fetched and sends it back with the
//! revision it read, so the daemon refuses the write when the ticket's
//! agent rewrote the note in between.

use super::*;
use mesimon_core::summary::{self, Count, Row};

/// How many note bodies one tick fetches for the board's summaries. Each
/// is one small read on the daemon's writer thread; a hundred-ticket board
/// is whole in about a second and asks nothing again until a `rev` moves.
const SUMMARY_READS_PER_TICK: usize = 8;

/// One note's summary rows as last parsed, keyed on the note's `rev` like
/// `NoteText`. `rows: None` is a read that failed, retried after
/// `NOTE_RETRY`.
pub struct NoteSummary {
    pub rev: u64,
    pub rows: Option<Vec<Row>>,
    tried: Instant,
}

/// One row of a ticket's summary: which note it is in, and the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub note: ulid::Ulid,
    pub line: usize,
    pub text: String,
    pub done: Option<bool>,
}

/// Every summary row of a ticket, description first then note order, and
/// the boxes counted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TicketSummary {
    pub rows: Vec<SummaryRow>,
    pub count: Count,
}

impl TicketSummary {
    /// The rows the open card shows (T-696): the first plain row, then up
    /// to `n` boxes — the open ones first, in their order, and ticked ones
    /// after when fewer than `n` are open — and how many boxes are left
    /// behind the fold with their aggregate state.
    pub fn card_rows(&self, n: usize) -> (Vec<&SummaryRow>, Option<(usize, Option<bool>)>) {
        let mut shown: Vec<&SummaryRow> =
            self.rows.iter().filter(|r| r.done.is_none()).take(1).collect();
        let open = self.rows.iter().filter(|r| r.done == Some(false));
        let ticked = self.rows.iter().filter(|r| r.done == Some(true));
        let boxes: Vec<&SummaryRow> = open.chain(ticked).take(n).collect();
        let hidden: Vec<Row> = self
            .rows
            .iter()
            .filter(|r| r.done.is_some() && !boxes.iter().any(|b| std::ptr::eq(*b, *r)))
            .map(|r| Row { line: r.line, text: String::new(), done: r.done })
            .collect();
        shown.extend(boxes);
        let fold = (!hidden.is_empty()).then(|| (hidden.len(), Count::of(&hidden).state()));
        (shown, fold)
    }
}

impl App {
    /// Parse one body's summary into the cache, beside the body itself
    /// (`remember_note` calls this on every fetch and every save).
    pub(crate) fn remember_summary(&mut self, id: ulid::Ulid, rev: u64, text: Option<&str>) {
        let rows = text.map(summary::extract);
        self.summaries.insert(id, NoteSummary { rev, rows, tried: Instant::now() });
    }

    /// The ticket's summary as the cache holds it: `None` while no note of
    /// it has a section with a row. Reads the board and the cache, never
    /// the disk — asked once a keypress and once a frame.
    pub fn ticket_summary(&self, ticket: ulid::Ulid) -> Option<TicketSummary> {
        let t = self.board.ticket(ticket)?;
        let mut rows: Vec<SummaryRow> = Vec::new();
        for meta in &t.notes {
            let Some(s) = self.summaries.get(&meta.id).filter(|s| s.rev == meta.rev) else {
                continue;
            };
            let Some(parsed) = &s.rows else { continue };
            rows.extend(parsed.iter().map(|r| SummaryRow {
                note: meta.id,
                line: r.line,
                text: r.text.clone(),
                done: r.done,
            }));
        }
        if rows.is_empty() {
            return None;
        }
        let count = rows.iter().fold(Count::default(), |mut c, r| {
            if let Some(d) = r.done {
                c.total += 1;
                c.done += usize::from(d);
            }
            c
        });
        Some(TicketSummary { rows, count })
    }

    /// `Ctx::ticket_summarised`: is there a dialog to open?
    pub(crate) fn ticket_summarised(&self, ticket: ulid::Ulid) -> bool {
        self.ticket_summary(ticket).is_some()
    }

    /// Is the cache current for this note? A failed read is current until
    /// `NOTE_RETRY` has passed.
    fn summary_fresh(&self, id: ulid::Ulid, rev: u64) -> bool {
        self.summaries
            .get(&id)
            .is_some_and(|s| s.rev == rev && (s.rows.is_some() || s.tried.elapsed() < NOTE_RETRY))
    }

    /// Fetch one note's body for its summary, through `remember_note` so the
    /// body cache and the links see it too.
    fn fetch_summary_note(&mut self, ticket: ulid::Ulid, id: ulid::Ulid, rev: u64) {
        let text = match self.req(Command::ReadNote { ticket, note: id }) {
            Response::Note { text, .. } => Some(crate::peek::sanitize(&text)),
            _ => None,
        };
        self.remember_note(id, rev, text);
    }

    /// The tick's road: the bodies the board's cards still need, a few per
    /// tick, the cursor column's tickets first. True when something was
    /// read, so the frame redraws.
    pub(crate) fn poll_summaries(&mut self) -> bool {
        let mut wanted: Vec<(ulid::Ulid, ulid::Ulid, u64)> = Vec::new();
        let cursor_col = self.cursor_column().map(|c| c.name.clone());
        let mut tickets: Vec<&Ticket> =
            self.board.tickets.iter().filter(|t| !t.is_archived()).collect();
        tickets.sort_by_key(|t| cursor_col.as_deref() != Some(t.column.as_str()));
        for t in tickets {
            for n in &t.notes {
                if !self.summary_fresh(n.id, n.rev) {
                    wanted.push((t.id, n.id, n.rev));
                }
            }
        }
        let mut read = false;
        for (ticket, id, rev) in wanted.into_iter().take(SUMMARY_READS_PER_TICK) {
            self.fetch_summary_note(ticket, id, rev);
            read = true;
        }
        read
    }

    /// The keypress road: every note of this ticket the cache lacks, now.
    fn fetch_summary(&mut self, ticket: ulid::Ulid) -> Option<TicketSummary> {
        let metas: Vec<(ulid::Ulid, u64)> = self
            .board
            .ticket(ticket)
            .map(|t| t.notes.iter().map(|n| (n.id, n.rev)).collect())
            .unwrap_or_default();
        for (id, rev) in metas {
            if !self.summary_fresh(id, rev) {
                self.fetch_summary_note(ticket, id, rev);
            }
        }
        self.ticket_summary(ticket)
    }

    /// `^j`: the SUMMARY dialog over the subject ticket. Nothing to list is
    /// a status line, never an empty dialog (the links dialog's rule).
    pub(crate) fn open_summary(&mut self) {
        let Some(ticket) = self.subject() else {
            return;
        };
        if self.fetch_summary(ticket).is_none() {
            let key =
                self.board.ticket(ticket).map(|t| t.short_key.as_str()).unwrap_or("the ticket");
            self.status = format!("no summary in {key}");
            return;
        }
        self.mode = Mode::Summary { ticket, idx: 0 };
    }

    /// The dialog's cursor row, clamped to the list as it stands now.
    pub(crate) fn summary_cursor(&self) -> Option<(ulid::Ulid, SummaryRow)> {
        let Mode::Summary { ticket, idx } = &self.mode else {
            return None;
        };
        let s = self.ticket_summary(*ticket)?;
        let row = s.rows.get((*idx).min(s.rows.len().saturating_sub(1)))?.clone();
        Some((*ticket, row))
    }

    /// Space in the dialog: flip the row's box in the note and write it
    /// back with the revision we read, so an agent's rewrite in between is
    /// refused rather than overwritten. The body is read again now — the
    /// cache is a parse, not the text — and a line that no longer holds a
    /// box means the note moved on since the dialog was drawn.
    pub(crate) fn summary_tick(&mut self) -> Result<()> {
        let Some((ticket, row)) = self.summary_cursor() else {
            return Ok(());
        };
        if row.done.is_none() {
            return Ok(());
        }
        let rev = self.board.ticket(ticket).and_then(|t| t.note(row.note)).map(|n| n.rev);
        let body = match self.req(Command::ReadNote { ticket, note: row.note }) {
            Response::Note { text, .. } => text,
            Response::Err { message } => {
                self.status = message;
                return Ok(());
            }
            _ => return Ok(()),
        };
        let Some(text) = summary::toggle(&body, row.line) else {
            self.status = "the note changed ∙ reopen".into();
            self.remember_note(row.note, rev.unwrap_or(0), Some(crate::peek::sanitize(&body)));
            return Ok(());
        };
        match self.req(Command::WriteNote { ticket, note: Some(row.note), text: text.clone(), rev })
        {
            Response::NoteWritten { .. } => {
                // The daemon bumped the revision by one; seed the cache from
                // our own text so the dialog ticks on this frame, and the
                // snapshot that follows agrees.
                self.remember_note(
                    row.note,
                    rev.unwrap_or(0) + 1,
                    Some(crate::peek::sanitize(&text)),
                );
                self.status = if row.done == Some(true) { "unticked" } else { "ticked" }.into();
                self.refresh()
            }
            Response::Err { message } => {
                self.status = message;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Enter in the dialog: the ticket page, on the row's note, scrolled so
    /// the row's line heads the zone (`ui::ticket` resolves the line to a
    /// rendered row once it knows the zone's width).
    pub(crate) fn summary_open_line(&mut self) {
        let Some((ticket, row)) = self.summary_cursor() else {
            return;
        };
        self.mode = Mode::Normal;
        let rail_idx = self
            .rail_rows(ticket)
            .iter()
            .position(|r| matches!(r, RailRow::Note(n) if n.id == row.note))
            .unwrap_or(0);
        self.screen = Screen::Ticket { ticket, rail_idx };
        self.summary_jump.set(Some((row.note, row.line)));
    }

    /// `a` / Shift+Enter in the dialog: close it and open the ticket's
    /// prompt field — the board's own Shift+Enter road, every rule of it —
    /// with the row's words quoted for the person to finish. Only an empty
    /// field takes the quote: a held ask reopening on its own words keeps
    /// them.
    pub(crate) fn summary_ask(&mut self, key: Key, ctx: &Ctx) -> Result<()> {
        let Some((_, row)) = self.summary_cursor() else {
            return Ok(());
        };
        self.mode = Mode::Normal;
        let scope = self.scope();
        self.dispatch(Verb::Prompt, key, scope, ctx)?;
        if let Mode::Input { purpose: InputPurpose::Prompt { .. }, buffer } = &mut self.mode {
            if buffer.as_str().is_empty() {
                let words = mesimon_core::text::scrub_text(&row.text);
                *buffer = EditBuffer::from_text(
                    format!("about \"{words}\": "),
                    mesimon_core::command::PROMPT_MAX_BYTES,
                );
            }
        }
        Ok(())
    }
}
