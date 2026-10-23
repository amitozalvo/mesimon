//! The transcript dialog's conversation (T-921): the cursor card's agent,
//! read straight from its transcript file the way the card's peek is
//! (`peek.rs`), a page at a time through the reader Remote Control pages
//! with (T-626). No daemon round-trip: the snapshot carries the path and
//! the read is observe-tier.
//!
//! The newest page is read on the key, so the dialog opens on its last
//! message at once; the pages before it are read on a thread of their own,
//! back to the file's start, so `(i of n)` counts the whole conversation
//! once they land. The tail is read again while the dialog is open, a stat
//! every `FRESH`, and only what was written since. The file is only
//! appended to, so a row's `at` is its place for good and a page joins
//! where the held part begins.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord};
use mesimon_core::mesophon::{Reply, RowKind, TranscriptRow};
use mesimon_daemon::agents::transcript::Ask;

use crate::app::Pager;

/// How often an open dialog stats the file for what was written since.
const FRESH: Duration = Duration::from_millis(250);

/// One message: a prompt, a reply or a notice, with the tool calls that
/// followed it before the next one (the steps it led to; the newest
/// message's grow while the agent works).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Message {
    /// Its record's offset and its place among that record's rows: the
    /// message's name for good, which a page put on top re-keys nothing of.
    pub key: (u64, usize),
    pub kind: RowKind,
    pub text: String,
    pub ms: Option<u64>,
    /// How many tool calls followed it, and their tools' names in order,
    /// once each.
    pub steps: usize,
    pub tools: Vec<String>,
}

/// The tool a step's line names (`Read src/main.rs`, `Bash(cargo test)`).
fn tool_name(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}

/// The rows as messages, oldest first. Steps before the first message
/// (a page that begins mid-run) belong to a message not held, and are
/// dropped.
pub(crate) fn messages(rows: &[TranscriptRow]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut nth = 0;
    let mut last_at = None;
    for row in rows {
        nth = if last_at == Some(row.at) { nth + 1 } else { 0 };
        last_at = Some(row.at);
        if row.kind == RowKind::Tool {
            if let Some(m) = out.last_mut() {
                m.steps += 1;
                let name = tool_name(&row.text);
                if !name.is_empty() && !m.tools.iter().any(|t| t == name) {
                    m.tools.push(name.to_string());
                }
            }
            continue;
        }
        out.push(Message {
            key: (row.at, nth),
            kind: row.kind,
            text: row.text.clone(),
            ms: row.ms,
            steps: 0,
            tools: Vec::new(),
        });
    }
    out
}

/// The agent record whose conversation the dialog reads: the ticket's
/// seat holder, else its newest agent record, so a parked or finished
/// agent's conversation still opens — the file outlives the pane.
pub(crate) fn session_of(
    board: &mesimon_core::board::Board,
    ticket: ulid::Ulid,
) -> Option<&SessionRecord> {
    let readable = |s: &&SessionRecord| s.kind.is_agent() && s.transcript_path.is_some();
    board
        .live_agent(ticket)
        .filter(readable)
        .or_else(|| board.sessions.iter().rev().find(|s| s.ticket == ticket && readable(s)))
}

/// A page as the reader answers it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Page {
    conversation: String,
    rows: Vec<TranscriptRow>,
    from: u64,
    end: u64,
    next_before: Option<u64>,
}

fn read(kind: SessionKind, path: &std::path::Path, ask: Ask, conv: Option<&str>) -> Option<Page> {
    match mesimon_daemon::agents::read_transcript_reply(kind, path, ask, conv)? {
        Reply::Transcript { conversation, rows, from, end, next_before } => {
            Some(Page { conversation, rows, from, end, next_before })
        }
        _ => None,
    }
}

/// The open dialog's conversation and where the reader stands in it.
pub(crate) struct Transcript {
    pub ticket: ulid::Ulid,
    pub kind: SessionKind,
    path: PathBuf,
    conversation: String,
    rows: Vec<TranscriptRow>,
    /// Where the held part ends, and where the page before it does (`None`
    /// once the file's start is held).
    end: u64,
    floor: Option<u64>,
    messages: Vec<Message>,
    /// The message on show.
    pub idx: usize,
    /// How many messages the dialog held when the newest was last on show:
    /// what came after is `new`.
    seen: usize,
    /// Where each message was left (`Pager` rows hidden above), so a step
    /// back to one lands where the reader was.
    left: HashMap<(u64, usize), usize>,
    pub pager: Pager,
    earlier: Option<Receiver<Page>>,
    stat: (u64, u64),
    checked: Instant,
}

/// The file's (len, mtime): what moves when anything is written.
fn stat_of(path: &std::path::Path) -> (u64, u64) {
    std::fs::metadata(path)
        .map(|m| {
            let mtime = m.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
            (m.len(), mtime)
        })
        .unwrap_or_default()
}

impl Transcript {
    /// The newest page of `rec`'s conversation, on the newest message.
    /// `None` when there is no file to read or nothing said in it.
    pub(crate) fn open(rec: &SessionRecord) -> Option<Self> {
        let path = PathBuf::from(rec.transcript_path.as_ref()?);
        let stat = stat_of(&path);
        let page = read(rec.kind, &path, Ask::default(), None)?;
        let messages = messages(&page.rows);
        if messages.is_empty() {
            return None;
        }
        let mut t = Transcript {
            ticket: rec.ticket,
            kind: rec.kind,
            path,
            conversation: page.conversation,
            rows: page.rows,
            end: page.end,
            floor: page.next_before,
            idx: messages.len() - 1,
            seen: messages.len(),
            messages,
            left: HashMap::new(),
            pager: Pager::default(),
            earlier: None,
            stat,
            checked: Instant::now(),
        };
        t.read_earlier();
        Some(t)
    }

    /// Every page before the held part, read off this thread back to the
    /// file's start. Dropping the dialog drops the receiver, and the reader
    /// stops at its next send.
    fn read_earlier(&mut self) {
        let Some(mut before) = self.floor else {
            self.earlier = None;
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (kind, path, conv) = (self.kind, self.path.clone(), self.conversation.clone());
        std::thread::spawn(move || loop {
            let ask = Ask { before: Some(before), ..Ask::default() };
            let Some(page) = read(kind, &path, ask, Some(&conv)) else { return };
            let next = page.next_before;
            if page.end != before || tx.send(page).is_err() {
                return;
            }
            match next {
                Some(b) if b < before => before = b,
                _ => return,
            }
        });
        self.earlier = Some(rx);
    }

    pub(crate) fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub(crate) fn message(&self) -> Option<&Message> {
        self.messages.get(self.idx)
    }

    /// Messages that came after the newest one the reader saw.
    pub(crate) fn unseen(&self) -> usize {
        self.messages.len().saturating_sub(self.seen)
    }

    /// `{ }`: one message older or newer, landing where that one was left.
    pub(crate) fn step(&mut self, dir: isize) {
        let to = (self.idx as isize + dir).clamp(0, self.messages.len() as isize - 1) as usize;
        if to == self.idx {
            return;
        }
        if let Some(m) = self.messages.get(self.idx) {
            self.left.insert(m.key, self.pager.view.get().offset);
        }
        self.idx = to;
        self.pager.reset();
        if let Some(m) = self.messages.get(to) {
            if let Some(&at) = self.left.get(&m.key) {
                self.pager.request.set(Some((doc_key(m), at)));
            }
        }
        self.saw();
    }

    fn saw(&mut self) {
        if self.idx + 1 >= self.messages.len() {
            self.seen = self.messages.len();
        }
    }

    /// Land what the earlier pages' reader sent, and what was written since
    /// the last read. True when the dialog's words moved.
    pub(crate) fn poll(&mut self) -> bool {
        let mut moved = false;
        while let Some(rx) = &self.earlier {
            match rx.try_recv() {
                Ok(page) => moved |= self.prepend(page),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.earlier = None,
            }
        }
        if self.checked.elapsed() >= FRESH {
            self.checked = Instant::now();
            let stat = stat_of(&self.path);
            if stat != self.stat {
                self.stat = stat;
                moved |= self.read_since();
            }
        }
        moved
    }

    /// An earlier page joins only where the held part begins.
    fn prepend(&mut self, page: Page) -> bool {
        if page.conversation != self.conversation || Some(page.end) != self.floor {
            return false;
        }
        let before = self.messages.len();
        self.floor = page.next_before;
        let mut rows = page.rows;
        rows.append(&mut self.rows);
        self.rows = rows;
        self.messages = messages(&self.rows);
        // The message on show stays on show: its place moved by what came
        // before it.
        let added = self.messages.len().saturating_sub(before);
        self.idx += added;
        self.seen += added;
        added > 0
    }

    /// What was written since the held part ends. A new conversation
    /// (`/clear`, `/resume`), or more than a page since, starts over at the
    /// tail on its newest message.
    fn read_since(&mut self) -> bool {
        let ask = Ask { after: Some(self.end), ..Ask::default() };
        let Some(page) = read(self.kind, &self.path, ask, Some(&self.conversation)) else {
            return false;
        };
        if page.conversation == self.conversation && page.from <= self.end {
            if page.end == self.end {
                return false;
            }
            let from = page.from;
            self.rows.retain(|r| r.at < from);
            self.rows.extend(page.rows);
            self.end = page.end;
            let on_newest = self.idx + 1 >= self.messages.len();
            let had = self.messages.len();
            self.messages = messages(&self.rows);
            // A reader on the newest message keeps reading it: what came
            // after is `new`, never a jump under the eye.
            self.idx = self.idx.min(self.messages.len().saturating_sub(1));
            if on_newest && self.messages.len() > had {
                self.seen = self.seen.min(had);
            }
            return true;
        }
        let page = match read(self.kind, &self.path, Ask::default(), None) {
            Some(p) => p,
            None => return false,
        };
        let messages = messages(&page.rows);
        if messages.is_empty() {
            return false;
        }
        self.conversation = page.conversation;
        self.rows = page.rows;
        self.end = page.end;
        self.floor = page.next_before;
        self.idx = messages.len() - 1;
        self.seen = messages.len();
        self.messages = messages;
        self.left.clear();
        self.pager.reset();
        self.read_earlier();
        true
    }
}

/// The pager's document key for a message.
pub(crate) fn doc_key(m: &Message) -> u64 {
    crate::text::hash64((7u8, m.key))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn row(at: u64, kind: RowKind, text: &str) -> TranscriptRow {
        TranscriptRow { at, kind, text: text.into(), ms: None }
    }

    #[test]
    fn steps_belong_to_the_message_before_them() {
        let rows = [
            row(0, RowKind::Tool, "Read orphan.rs"),
            row(10, RowKind::Prompt, "fix it"),
            row(20, RowKind::Reply, "reading first"),
            row(20, RowKind::Tool, "Read src/main.rs"),
            row(30, RowKind::Tool, "Bash(cargo test)"),
            row(40, RowKind::Tool, "Read src/lib.rs"),
            row(50, RowKind::Reply, "done"),
        ];
        let m = messages(&rows);
        assert_eq!(m.len(), 3, "a step before every message is no message's");
        assert_eq!((m[0].steps, m[1].steps, m[2].steps), (0, 3, 0));
        assert_eq!(m[1].tools, ["Read", "Bash"], "each tool once, in order");
        assert_eq!(m[1].key, (20, 0));
        assert_eq!(m[2].key, (50, 0));
    }

    /// A conversation on disk, as Claude Code writes one.
    pub(crate) fn claude_line(kind: &str, text: &str) -> String {
        match kind {
            "user" => serde_json::json!({
                "uuid": format!("u{}", text.len()),
                "type": "user",
                "message": { "role": "user", "content": text },
            }),
            _ => serde_json::json!({
                "uuid": format!("a{}", text.len()),
                "type": "assistant",
                "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
            }),
        }
        .to_string()
    }

    fn rec(path: &std::path::Path) -> SessionRecord {
        let mut r = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec![],
            String::new(),
            mesimon_core::board::SessionState::Running,
        );
        r.transcript_path = Some(path.to_string_lossy().into_owned());
        r
    }

    #[test]
    fn opens_on_the_newest_and_counts_what_comes_after_as_new() {
        let dir = std::env::temp_dir().join(format!("msmn-transcript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let body: String = [claude_line("user", "fix the race"), claude_line("a", "found it")]
            .iter()
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(&path, body).unwrap();
        let mut t = Transcript::open(&rec(&path)).expect("a conversation");
        assert_eq!((t.idx, t.messages().len(), t.unseen()), (1, 2, 0));
        assert_eq!(t.message().map(|m| m.text.as_str()), Some("found it"));
        // The agent says more: the reader stays where they are.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n", claude_line("a", "fixed")).as_bytes())
            .unwrap();
        t.checked = Instant::now() - FRESH * 2;
        assert!(t.poll());
        assert_eq!((t.idx, t.messages().len(), t.unseen()), (1, 3, 1));
        t.step(1);
        assert_eq!((t.idx, t.unseen()), (2, 0), "the newest on show is seen");
        t.step(1);
        assert_eq!(t.idx, 2, "nothing past the newest");
        t.step(-1);
        t.step(-1);
        t.step(-1);
        assert_eq!(t.idx, 0, "nothing before the first");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
