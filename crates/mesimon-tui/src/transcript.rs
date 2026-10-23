//! The transcript dialog's conversation (T-921): the cursor card's agent,
//! read straight from its transcript file the way the card's peek is
//! (`peek.rs`), a page at a time through the reader Remote Control pages
//! with (T-626). No daemon round-trip: the snapshot carries the path and
//! the read is observe-tier.
//!
//! The newest page is read on the key, so the dialog opens on its last
//! message at once; the pages before it are read on a thread of their own,
//! back to the file's start, so `(i of n)` counts the whole conversation
//! once they land. While the dialog is open the UI thread only stats the
//! file, every `FRESH`; what was written since is read on another thread,
//! one read at a time, and lands in `pump`, as a snapshot does (T-812).
//! That read resolves the file again, so a `/clear`, a `/resume` or a
//! Codex continuation (T-840) is followed. The file is only appended to,
//! so a row's `at` is its place for good and a page joins where the held
//! part begins.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord};
use mesimon_core::mesophon::{Reply, RowKind, TranscriptRow};
use mesimon_daemon::agents::transcript::Ask;

use crate::app::Pager;

/// How often an open dialog stats the file for what was written since.
const FRESH: Duration = Duration::from_millis(250);
/// How often a Codex conversation is looked for its continuation rollout
/// while the dialog is open: a walk of `~/.codex/sessions`, on the tail
/// read's thread.
const WALK: Duration = Duration::from_secs(5);
/// The longest message the dialog reads whole; a longer one is cut and
/// ends in `…` (a phone gets 8 KiB, T-626).
pub(crate) const ROW_MAX: usize = 256 * 1024;

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

/// One page of the file at `path`, already resolved, its messages read
/// whole up to `ROW_MAX`.
fn read(kind: SessionKind, path: &Path, ask: Ask, conv: Option<&str>) -> Option<Page> {
    let ask = Ask { row_max: Some(ROW_MAX), ..ask };
    match mesimon_daemon::agents::read_transcript_reply_at(kind, path, ask, conv)? {
        Reply::Transcript { conversation, rows, from, end, next_before } => {
            Some(Page { conversation, rows, from, end, next_before })
        }
        _ => None,
    }
}

/// What a tail read found, off the UI thread: the file it read (resolved
/// again), its stat taken before the read, and either what was written
/// since the held part (`over: false`) or that file's newest page (another
/// conversation, a file cut short, more than a page since).
struct Tail {
    path: PathBuf,
    stat: (u64, u64),
    page: Page,
    over: bool,
}

fn read_tail(kind: SessionKind, record: &Path, path: &Path, conv: &str, end: u64) -> Option<Tail> {
    let file = mesimon_daemon::agents::transcript_file(kind, record);
    let stat = stat_of(&file);
    if file == path {
        let ask = Ask { after: Some(end), ..Ask::default() };
        let page = read(kind, &file, ask, Some(conv))?;
        if page.conversation == conv && page.from <= end && page.end >= end {
            return Some(Tail { path: file, stat, page, over: false });
        }
    }
    let page = read(kind, &file, Ask::default(), None)?;
    Some(Tail { path: file, stat, page, over: true })
}

/// The open dialog's conversation and where the reader stands in it.
pub(crate) struct Transcript {
    pub ticket: ulid::Ulid,
    pub kind: SessionKind,
    /// The record's transcript path, as the board last said it.
    record: PathBuf,
    /// The file read: the record's, or a Codex rollout's continuation.
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
    /// The newest message the reader has been on: what came after it is
    /// `new`.
    seen: Option<(u64, usize)>,
    /// Where each message was left (`Pager` rows hidden above), so a step
    /// back to one lands where the reader was.
    left: HashMap<(u64, usize), usize>,
    pub pager: Pager,
    earlier: Option<Receiver<Page>>,
    /// The tail read in flight, one at a time; `again` when the file moved
    /// meanwhile, so another follows it.
    tail: Option<Receiver<Option<Tail>>>,
    again: bool,
    stat: (u64, u64),
    checked: Instant,
    /// When the file was last resolved (a Codex continuation is looked for
    /// every `WALK` even while the file read is still).
    walked: Instant,
}

/// The file's (len, mtime): what moves when anything is written.
fn stat_of(path: &Path) -> (u64, u64) {
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
        let record = PathBuf::from(rec.transcript_path.as_ref()?);
        let path = mesimon_daemon::agents::transcript_file(rec.kind, &record);
        let stat = stat_of(&path);
        let page = read(rec.kind, &path, Ask::default(), None)?;
        if messages(&page.rows).is_empty() {
            return None;
        }
        let mut t = Transcript {
            ticket: rec.ticket,
            kind: rec.kind,
            record,
            path,
            conversation: String::new(),
            rows: Vec::new(),
            end: 0,
            floor: None,
            messages: Vec::new(),
            idx: 0,
            seen: None,
            left: HashMap::new(),
            pager: Pager::default(),
            earlier: None,
            tail: None,
            again: false,
            stat,
            checked: Instant::now(),
            walked: Instant::now(),
        };
        t.start_over(page);
        Some(t)
    }

    /// A page as the whole of what is held, on its newest message: the
    /// dialog's first read, and a new conversation's.
    fn start_over(&mut self, page: Page) {
        self.conversation = page.conversation;
        self.rows = page.rows;
        self.end = page.end;
        self.floor = page.next_before;
        self.messages = messages(&self.rows);
        self.idx = self.messages.len().saturating_sub(1);
        self.seen = None;
        self.saw();
        self.left.clear();
        self.pager.reset();
        self.read_earlier();
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

    /// What was written since, read on a thread of its own; the file is
    /// resolved again there.
    fn read_tail(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (kind, record, path) = (self.kind, self.record.clone(), self.path.clone());
        let (conv, end) = (self.conversation.clone(), self.end);
        std::thread::spawn(move || {
            let _ = tx.send(read_tail(kind, &record, &path, &conv, end));
        });
        self.tail = Some(rx);
        self.walked = Instant::now();
    }

    pub(crate) fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub(crate) fn message(&self) -> Option<&Message> {
        self.messages.get(self.idx)
    }

    /// Messages that came after the newest one the reader has been on.
    pub(crate) fn unseen(&self) -> usize {
        self.messages.iter().rev().take_while(|m| Some(m.key) > self.seen).count()
    }

    /// `{ }`: one message older or newer, landing where that one was left.
    pub(crate) fn step(&mut self, dir: isize) {
        let Some(last) = self.messages.len().checked_sub(1) else { return };
        let to = self.idx.saturating_add_signed(dir).min(last);
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
        self.seen = self.seen.max(self.message().map(|m| m.key));
    }

    /// The messages again from the rows, once per poll however many pages
    /// landed; the message on show stays on show, found by its key.
    fn rebuild(&mut self) {
        let on = self.message().map(|m| m.key);
        self.messages = messages(&self.rows);
        self.idx = match on {
            Some(k) => self.messages.partition_point(|m| m.key <= k).saturating_sub(1),
            None => self.messages.len().saturating_sub(1),
        };
        self.saw();
    }

    /// Land what the earlier pages' reader sent and what the tail read
    /// found, and stat the file for another. `record` is the conversation
    /// the board says the card's agent holds now. True when the dialog's
    /// words moved.
    pub(crate) fn poll(&mut self, record: Option<(SessionKind, &str)>) -> bool {
        let mut rows = false;
        while let Some(rx) = &self.earlier {
            match rx.try_recv() {
                Ok(page) => rows |= self.prepend(page),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => self.earlier = None,
            }
        }
        let mut over = false;
        if let Some(rx) = &self.tail {
            match rx.try_recv() {
                Ok(tail) => {
                    self.tail = None;
                    if let Some(Tail { path, stat, page, over: anew }) = tail {
                        (self.path, self.stat) = (path, stat);
                        if anew {
                            over = true;
                            self.start_over(page);
                        } else {
                            rows |= self.append(page);
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.tail = None,
            }
        }
        if rows && !over {
            self.rebuild();
        }
        if let Some((kind, path)) = record {
            if (kind, Path::new(path)) != (self.kind, self.record.as_path()) {
                (self.kind, self.record) = (kind, PathBuf::from(path));
                self.again = true;
            }
        }
        if self.checked.elapsed() >= FRESH {
            self.checked = Instant::now();
            let stat = stat_of(&self.path);
            if stat != self.stat {
                self.stat = stat;
                self.again = true;
            }
            self.again |= self.kind == SessionKind::Codex && self.walked.elapsed() >= WALK;
        }
        if self.again && self.tail.is_none() {
            self.again = false;
            self.read_tail();
        }
        rows || over
    }

    /// An earlier page joins only where the held part begins.
    fn prepend(&mut self, page: Page) -> bool {
        if page.conversation != self.conversation || Some(page.end) != self.floor {
            return false;
        }
        self.floor = page.next_before;
        let mut rows = page.rows;
        rows.append(&mut self.rows);
        self.rows = rows;
        true
    }

    /// What was written since the held part ends, from where its page
    /// begins.
    fn append(&mut self, page: Page) -> bool {
        if page.conversation != self.conversation || page.end == self.end {
            return false;
        }
        self.rows.retain(|r| r.at < page.from);
        self.rows.extend(page.rows);
        self.end = page.end;
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

    fn lines(lines: &[(&str, &str)]) -> String {
        lines.iter().map(|(k, t)| format!("{}\n", claude_line(k, t))).collect()
    }

    fn append(path: &std::path::Path, kind: &str, text: &str) {
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        std::io::Write::write_all(&mut f, format!("{}\n", claude_line(kind, text)).as_bytes())
            .unwrap();
    }

    /// Poll as `pump` does, the stat due every time, until `done`: the
    /// tail is read on a thread and lands on a later poll.
    fn settle(t: &mut Transcript, rec: &SessionRecord, done: impl Fn(&Transcript) -> bool) {
        for _ in 0..1000 {
            t.checked = Instant::now() - FRESH * 2;
            t.poll(Some((rec.kind, rec.transcript_path.as_deref().unwrap())));
            if done(t) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the dialog never caught up: {:?}", t.messages());
    }

    fn texts(t: &Transcript) -> Vec<&str> {
        t.messages().iter().map(|m| m.text.as_str()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("msmn-transcript-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn opens_on_the_newest_and_counts_what_comes_after_as_new() {
        let dir = scratch("newest");
        let path = dir.join("t.jsonl");
        std::fs::write(&path, lines(&[("user", "fix the race"), ("a", "found it")])).unwrap();
        let rec = rec(&path);
        let mut t = Transcript::open(&rec).expect("a conversation");
        assert_eq!((t.idx, t.messages().len(), t.unseen()), (1, 2, 0));
        assert_eq!(t.message().map(|m| m.text.as_str()), Some("found it"));
        // The agent says more: the reader stays where they are.
        append(&path, "a", "fixed");
        settle(&mut t, &rec, |t| t.messages().len() == 3);
        assert_eq!((t.idx, t.unseen()), (1, 1));
        t.step(1);
        assert_eq!((t.idx, t.unseen()), (2, 0), "the newest on show is seen");
        t.step(1);
        assert_eq!(t.idx, 2, "nothing past the newest");
        t.step(-1);
        t.step(-1);
        t.step(-1);
        assert_eq!(t.idx, 0, "nothing before the first");
        // Back on an older one, what comes next counts from the newest seen.
        append(&path, "user", "ship it");
        settle(&mut t, &rec, |t| t.messages().len() == 4);
        assert_eq!((t.idx, t.unseen()), (0, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A long reply reads whole, past the 8 KiB a phone is sent, so `c`
    /// copies all of it.
    #[test]
    fn a_long_reply_is_held_whole() {
        let dir = scratch("long");
        let path = dir.join("t.jsonl");
        let long = "word ".repeat(4096);
        std::fs::write(&path, lines(&[("user", "plan it"), ("a", &long)])).unwrap();
        let t = Transcript::open(&rec(&path)).expect("a conversation");
        assert_eq!(t.message().map(|m| m.text.len()), Some(long.trim_end().len()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/clear` (or `/resume`) moves the record to another file: the dialog
    /// follows it and starts over on its newest message.
    #[test]
    fn a_new_conversation_is_followed() {
        let dir = scratch("moved");
        let (old, new) = (dir.join("a.jsonl"), dir.join("b.jsonl"));
        std::fs::write(&old, lines(&[("user", "first"), ("a", "first reply")])).unwrap();
        let mut rec = rec(&old);
        let mut t = Transcript::open(&rec).expect("a conversation");
        t.step(-1);
        std::fs::write(&new, lines(&[("user", "second"), ("a", "on it"), ("a", "done")])).unwrap();
        rec.transcript_path = Some(new.to_string_lossy().into_owned());
        settle(&mut t, &rec, |t| t.messages().len() == 3);
        assert_eq!(texts(&t), ["second", "on it", "done"]);
        assert_eq!((t.idx, t.unseen()), (2, 0), "on the new conversation's newest");
        // And its tail is read from there on.
        append(&new, "a", "merged");
        settle(&mut t, &rec, |t| t.messages().len() == 4);
        assert_eq!((t.idx, t.unseen()), (2, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file cut short leaves the dialog open on nothing said yet, and a
    /// step there is inert.
    #[test]
    fn a_file_cut_short_starts_over() {
        let dir = scratch("cut");
        let path = dir.join("t.jsonl");
        std::fs::write(&path, lines(&[("user", "fix it"), ("a", "fixed")])).unwrap();
        let rec = rec(&path);
        let mut t = Transcript::open(&rec).expect("a conversation");
        std::fs::write(&path, "").unwrap();
        settle(&mut t, &rec, |t| t.messages().is_empty());
        t.step(1);
        t.step(-1);
        assert!(t.message().is_none() && t.unseen() == 0);
        std::fs::write(&path, lines(&[("user", "again")])).unwrap();
        settle(&mut t, &rec, |t| t.messages().len() == 1);
        assert_eq!(t.message().map(|m| m.text.as_str()), Some("again"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
