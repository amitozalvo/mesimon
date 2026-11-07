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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
    /// Its row in the held rows, whose `text` it is (`Transcript::text`):
    /// a message is rebuilt with every page that lands, so it names its
    /// words rather than copying them.
    row: usize,
    pub ms: Option<u64>,
    /// How many tool calls followed it, and their tools' names in order,
    /// once each.
    pub steps: usize,
    pub tools: Vec<String>,
    /// A raised hand's reason (T-1155), and whether the hand is up.
    pub hand: Option<Raise>,
}

/// A raised hand among the messages (T-1155).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Raise {
    /// The call in the file, its hand down since (or raised again later).
    Down,
    /// The call in the file, the board's hand up with its words.
    Up,
    /// The board's hand alone, its call not among the held rows (a Codex
    /// call made from a cell, an earlier page still landing): its words
    /// are `Transcript::hand`'s.
    Board,
}

impl Raise {
    pub(crate) fn up(self) -> bool {
        self != Raise::Down
    }
}

/// The ticket's raised hand as the board holds it (T-1155): what says
/// which of the conversation's hands is up, and the ask itself where the
/// file does not hold its call.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Hand {
    /// When it went up, epoch ms: the board's stamp is whole seconds.
    pub ms: u64,
    pub reason: String,
}

/// `ticket`'s raised hand, for `session`'s conversation: none when another
/// agent raised it.
pub(crate) fn hand_of(
    board: &mesimon_core::board::Board,
    ticket: ulid::Ulid,
    session: uuid::Uuid,
) -> Option<Hand> {
    let raised = board.ticket(ticket)?.raised.as_ref()?;
    if raised.by.strip_prefix("agent:").is_some_and(|by| by != session.to_string()) {
        return None;
    }
    let secs = mesimon_core::board::stamp_secs(&raised.at)?;
    Some(Hand { ms: secs * 1000, reason: raised.reason.clone() })
}

/// The hand among `out` by its time: after the last message written in or
/// before its second, and that message's record (a key stays in order). A
/// conversation with no clock has it last; one whose every message is
/// later never held the ask (an earlier page, another conversation).
fn place_hand(out: &mut Vec<Message>, hand: &Hand) {
    let up_to = hand.ms + 999;
    let before = match out.iter().rposition(|m| m.ms.is_some_and(|ms| ms <= up_to)) {
        Some(i) => i,
        None if out.iter().all(|m| m.ms.is_none()) && !out.is_empty() => out.len() - 1,
        None => return,
    };
    let mut at = before;
    while out.get(at + 1).is_some_and(|m| m.key.0 == out[before].key.0) {
        at += 1;
    }
    let key = (out[at].key.0, usize::MAX);
    let row = out[at].row;
    out.insert(
        at + 1,
        Message {
            key,
            kind: RowKind::Reply,
            row,
            ms: Some(hand.ms),
            steps: 0,
            tools: Vec::new(),
            hand: Some(Raise::Board),
        },
    );
}

/// Words compared as a card reads them: the board's reason is scrubbed to
/// one line, the file's row keeps its lines.
fn same_words(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

use mesimon_core::mesophon::step_tool as tool_name;

/// The rows as messages, oldest first. Steps before the first message
/// (a page that begins mid-run) belong to a message not held, and are
/// dropped.
pub(crate) fn messages(rows: &[TranscriptRow]) -> Vec<Message> {
    let mut out: Vec<Message> = Vec::new();
    let mut nth = 0;
    let mut last_at = None;
    for (i, row) in rows.iter().enumerate() {
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
        let hand = mesimon_core::mesophon::is_hand_reason(rows, i).then_some(Raise::Down);
        out.push(Message {
            key: (row.at, nth),
            kind: row.kind,
            row: i,
            ms: row.ms,
            steps: 0,
            tools: Vec::new(),
            hand,
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
    /// The ticket's raised hand, while it is up (T-1155).
    hand: Option<Hand>,
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
    /// Parsed once per displayed message, then resolved against the current board.
    links: RefCell<Option<(u64, Rc<Vec<mesimon_core::links::Link>>)>>,
    /// A link-cycle press asks the next measured frame to reveal its mark.
    pub reveal_link: Cell<bool>,
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
    /// The newest page of `rec`'s conversation, on the newest message (a
    /// raised `hand`, when it is the newest). `None` when there is no file
    /// to read or nothing said in it.
    pub(crate) fn open(rec: &SessionRecord, hand: Option<Hand>) -> Option<Self> {
        let record = PathBuf::from(rec.transcript_path.as_ref()?);
        let path = mesimon_daemon::agents::transcript_file(rec.kind, &record);
        let stat = stat_of(&path);
        let page = read(rec.kind, &path, Ask::default(), None)?;
        if page.rows.iter().all(|r| r.kind == RowKind::Tool) {
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
            hand,
            messages: Vec::new(),
            idx: 0,
            seen: None,
            left: HashMap::new(),
            pager: Pager::default(),
            links: RefCell::new(None),
            reveal_link: Cell::new(false),
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
        self.links.get_mut().take();
        self.reveal_link.set(false);
        self.conversation = page.conversation;
        self.rows = page.rows;
        self.end = page.end;
        self.floor = page.next_before;
        self.messages = self.composed();
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

    /// A message's words, whole up to `ROW_MAX`.
    pub(crate) fn text(&self, m: &Message) -> &str {
        match (&self.hand, m.hand) {
            (Some(h), Some(Raise::Board)) => &h.reason,
            _ => &self.rows[m.row].text,
        }
    }

    /// The rows' messages, the board's hand among them: up on the newest
    /// call that says its words, else placed by its time.
    fn composed(&self) -> Vec<Message> {
        let mut out = messages(&self.rows);
        let Some(h) = &self.hand else { return out };
        let newest = out.iter().rposition(|m| m.hand.is_some());
        match newest.filter(|&i| same_words(&self.rows[out[i].row].text, &h.reason)) {
            Some(i) => out[i].hand = Some(Raise::Up),
            None => place_hand(&mut out, h),
        }
        out
    }

    pub(crate) fn hand(&self) -> Option<&Hand> {
        self.hand.as_ref()
    }

    /// The board's hand for this conversation, as each snapshot says it:
    /// raised, it is a message (new, after the one on show); lowered, it
    /// is gone. True when the messages moved.
    pub(crate) fn set_hand(&mut self, hand: Option<Hand>) -> bool {
        if self.hand == hand {
            return false;
        }
        self.hand = hand;
        self.rebuild();
        true
    }

    pub(crate) fn links(&self) -> Rc<Vec<mesimon_core::links::Link>> {
        let Some(m) = self.message() else { return Rc::new(Vec::new()) };
        let key = doc_key(m);
        let mut cache = self.links.borrow_mut();
        if let Some((held, links)) = cache.as_ref() {
            if *held == key {
                return Rc::clone(links);
            }
        }
        let links = Rc::new(mesimon_core::links::extract(&crate::peek::sanitize(self.text(m))));
        *cache = Some((key, Rc::clone(&links)));
        links
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
        self.links.get_mut().take();
        let on = self.message().map(|m| m.key);
        self.messages = self.composed();
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

    fn timed(at: u64, kind: RowKind, text: &str, ms: u64) -> TranscriptRow {
        TranscriptRow { at, kind, text: text.into(), ms: Some(ms) }
    }

    /// A raised hand (T-1155) is a message where it was raised: after the
    /// record written in or before its second, before what came later.
    #[test]
    fn a_hand_lands_among_the_messages_by_its_time() {
        let rows = [
            timed(10, RowKind::Prompt, "fix it", 1_000),
            timed(20, RowKind::Reply, "two ways; asking", 5_400),
            timed(20, RowKind::Reply, "and a second row", 5_400),
            timed(30, RowKind::Tool, "mcp__mesimon__raise_hand", 5_600),
            timed(40, RowKind::Reply, "waiting on the person", 9_000),
        ];
        let hand = Hand { ms: 5_000, reason: "Auth0 or cookie?".into() };
        let mut m = messages(&rows);
        place_hand(&mut m, &hand);
        let hands: Vec<usize> = (0..m.len()).filter(|&i| m[i].hand.is_some()).collect();
        assert_eq!(hands, [3], "after its second's record, before what came after: {m:?}");
        assert!(m.windows(2).all(|w| w[0].key < w[1].key), "keys stay in order: {m:?}");
        assert_eq!(m[3].ms, Some(5_000));
        // No clock in the file: the hand is the newest thing said.
        let bare = [row(10, RowKind::Prompt, "fix it"), row(20, RowKind::Reply, "asking")];
        let mut m = messages(&bare);
        place_hand(&mut m, &hand);
        assert!(m.last().is_some_and(|m| m.hand == Some(Raise::Board)), "{m:?}");
        // Every message held came later: the ask is not in what is held.
        let later = [timed(10, RowKind::Prompt, "after a /clear", 60_000)];
        let mut m = messages(&later);
        place_hand(&mut m, &hand);
        assert!(!m.iter().any(|m| m.hand.is_some()), "{m:?}");
    }

    /// A `raise_hand` call's reason row (T-1155) is a hand for good: down
    /// with no hand on the board, up while the board's hand says its words.
    #[test]
    fn the_files_hands_stay_and_the_newest_with_the_boards_words_is_up() {
        let rows = [
            timed(10, RowKind::Prompt, "fix it", 1_000),
            timed(20, RowKind::Tool, "mcp__mesimon__raise_hand", 2_000),
            timed(20, RowKind::Reply, "first ask", 2_000),
            timed(30, RowKind::Prompt, "answered", 3_000),
            timed(40, RowKind::Tool, "Read raise_hand.rs", 4_000),
            timed(40, RowKind::Reply, "a reply after a step, no hand", 4_000),
            timed(50, RowKind::Tool, "mcp__mesimon__raise_hand", 5_000),
            timed(50, RowKind::Reply, "second\nask", 5_000),
        ];
        let marks = |t: &[Message]| t.iter().map(|m| m.hand).collect::<Vec<_>>();
        let down = Some(Raise::Down);
        assert_eq!(marks(&messages(&rows)), [None, down, None, None, down]);
        let dir = scratch("file-hands");
        let path = dir.join("t.jsonl");
        std::fs::write(&path, lines(&[("user", "x")])).unwrap();
        let mut held = Transcript::open(&rec(&path), None).expect("a conversation");
        held.rows = rows.to_vec();
        held.rebuild();
        // The board's hand up with the newest call's words: that one is up.
        held.set_hand(Some(Hand { ms: 5_000, reason: "second ask".into() }));
        let up = Some(Raise::Up);
        assert_eq!(marks(held.messages()), [None, down, None, None, up]);
        // Up with words the file does not hold yet: the board's, placed.
        held.set_hand(Some(Hand { ms: 6_000, reason: "third ask".into() }));
        let board = Some(Raise::Board);
        assert_eq!(marks(held.messages()), [None, down, None, None, down, board]);
        assert_eq!(held.text(&held.messages()[5]), "third ask");
        // Down: every call stays, quiet.
        held.set_hand(None);
        assert_eq!(marks(held.messages()), [None, down, None, None, down]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dialog's hand follows the board's: raised, a new message after
    /// the one on show, whose words are the reason; lowered, gone.
    #[test]
    fn a_raised_hand_is_new_and_a_lowered_one_is_gone() {
        let dir = scratch("raised");
        let path = dir.join("t.jsonl");
        std::fs::write(&path, lines(&[("user", "pick one"), ("a", "asking")])).unwrap();
        let mut t = Transcript::open(&rec(&path), None).expect("a conversation");
        let hand = Hand { ms: 0, reason: "Auth0 or cookie?".into() };
        assert!(t.set_hand(Some(hand.clone())));
        assert!(!t.set_hand(Some(hand.clone())), "the same hand moves nothing");
        assert_eq!((t.idx, t.messages().len(), t.unseen()), (1, 3, 1));
        t.step(1);
        let on = t.message().map(|m| (m.hand, t.text(m)));
        assert_eq!(on, Some((Some(Raise::Board), "Auth0 or cookie?")));
        assert!(t.set_hand(None));
        assert_eq!(texts(&t), ["pick one", "asking"]);
        assert_eq!(t.idx, 1, "back on the message it followed");
        // Opened with the hand up, the dialog is on it.
        let t = Transcript::open(&rec(&path), Some(hand)).expect("a conversation");
        assert!(t.message().is_some_and(|m| m.hand.is_some()));
        let _ = std::fs::remove_dir_all(&dir);
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
        t.messages().iter().map(|m| t.text(m)).collect()
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
        let mut t = Transcript::open(&rec, None).expect("a conversation");
        assert_eq!((t.idx, t.messages().len(), t.unseen()), (1, 2, 0));
        assert_eq!(t.message().map(|m| t.text(m)), Some("found it"));
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
        let t = Transcript::open(&rec(&path), None).expect("a conversation");
        assert_eq!(t.message().map(|m| t.text(m).len()), Some(long.trim_end().len()));
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
        let mut t = Transcript::open(&rec, None).expect("a conversation");
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
        let mut t = Transcript::open(&rec, None).expect("a conversation");
        std::fs::write(&path, "").unwrap();
        settle(&mut t, &rec, |t| t.messages().is_empty());
        t.step(1);
        t.step(-1);
        assert!(t.message().is_none() && t.unseen() == 0);
        std::fs::write(&path, lines(&[("user", "again")])).unwrap();
        settle(&mut t, &rec, |t| t.messages().len() == 1);
        assert_eq!(t.message().map(|m| t.text(m)), Some("again"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
