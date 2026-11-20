//! The transcript dialog's conversation (T-921): the cursor card's agent,
//! read straight from its transcript file the way the card's peek is
//! (`peek.rs`), a page at a time through the reader Remote Control pages
//! with (T-626). No daemon round-trip: the snapshot carries the path and
//! the read is observe-tier.
//!
//! The newest page is read on the key, so the dialog opens on its last
//! message at once; the pages before it are read on a thread of their own,
//! back to the file's start, so `i of n` counts the whole conversation
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
    /// The crown's (T-1162): an act of its own, or a prompt whose words
    /// the board carried (the crown's ask, a wake, another board's).
    pub crown: Option<Crowned>,
}

/// What a crown row says over its words (T-1162).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Crowned {
    /// The sentence the head says, without its mark or time.
    pub head: String,
    /// The other ticket's title as `head` spells it, the part a narrow
    /// row shortens (T-1263); empty where the head names none.
    pub title: String,
    /// Words across boards, which wear the bolt's mark instead.
    pub road: bool,
    /// An act's own row: its words are `ActLine::body`, by this index.
    pub act: Option<usize>,
    /// What started the crown's turn, said on a line under the words.
    pub cause: Option<String>,
}

// One crown act as a conversation places it (T-1162): core's, shared
// with the phone's conversation (T-1264).
pub(crate) use mesimon_core::crownact::{lines_of as acts_of, Line as ActLine};

/// How long before an act a prompt it carried may be dated: the two
/// clocks are the daemon's and the transcript's.
const ACT_SLACK_MS: u64 = 5_000;

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
    Some(Hand { ms: raised.ms_for(session)?, reason: raised.reason.clone() })
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
            crown: None,
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
            crown: None,
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
        Reply::Transcript { conversation, rows, from, end, next_before, .. } => {
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
    /// The crown's acts this conversation says (T-1162), and whether it is
    /// the ticket's newest, which alone says the acts after its last words.
    acts: Vec<ActLine>,
    newest: bool,
    /// What the two above are (`acts_stamp`), hashed where they are set.
    acts_stamp: u64,
    /// The `crownacts::Acts::version` the acts were read from.
    pub acts_seen: Option<u64>,
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

/// A conversation's acts and whether it is the newest, as one number.
fn stamp_acts(acts: &[ActLine], newest: bool) -> u64 {
    crate::text::hash64((acts, newest))
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
            acts: Vec::new(),
            newest: true,
            acts_stamp: stamp_acts(&[], true),
            acts_seen: None,
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
    pub(crate) fn is_rename(&self, m: &Message) -> bool {
        m.crown.as_ref().and_then(|c| c.act).is_some_and(|i| self.acts[i].rename)
    }

    pub(crate) fn text(&self, m: &Message) -> &str {
        if let Some(i) = m.crown.as_ref().and_then(|c| c.act) {
            return self.acts.get(i).map_or("", |a| a.body.as_str());
        }
        match (&self.hand, m.hand) {
            (Some(h), Some(Raise::Board)) => &h.reason,
            _ => &self.rows[m.row].text,
        }
    }

    /// The rows' messages, the board's hand among them: up on the newest
    /// call that says its words, else placed by its time.
    fn with_hand(&self) -> Vec<Message> {
        let mut out = messages(&self.rows);
        let Some(h) = &self.hand else { return out };
        let newest = out.iter().rposition(|m| m.hand.is_some());
        match newest.filter(|&i| same_words(&self.rows[out[i].row].text, &h.reason)) {
            Some(i) => out[i].hand = Some(Raise::Up),
            None => place_hand(&mut out, h),
        }
        out
    }

    /// The rows' messages with the board's hand and the crown's acts.
    fn composed(&self) -> Vec<Message> {
        let mut out = self.with_hand();
        self.place_acts(&mut out);
        out
    }

    /// The crown's acts among the messages (T-1162): an act that carried
    /// words marks the prompt that holds them, the first not yet marked
    /// dated no earlier than the act; any other is a row of its own, after
    /// the last message written by its time. An act before the first
    /// message is placed only once the file's start is held, and one after
    /// the last only in the ticket's newest conversation.
    fn place_acts(&self, out: &mut Vec<Message>) {
        use mesimon_core::crownact::words_key;
        let Some(first) = out.first().map(|m| m.key) else { return };
        if out.iter().all(|m| m.ms.is_none()) {
            return;
        }
        let first_ms = out.iter().find_map(|m| m.ms).unwrap_or(0);
        let last_ms = out.iter().rev().find_map(|m| m.ms).unwrap_or(0);
        // A prompt's key by its row, hashed the first time an act asks: an
        // act whose words no prompt holds asks every later one.
        let mut keys: HashMap<usize, u64> = HashMap::new();
        for (i, a) in self.acts.iter().enumerate() {
            // Before what is held, while the earlier pages are still
            // landing: its prompt may be among them, and a later prompt of
            // the same words is another act's (T-1289: the same wake twice).
            let unheld = self.floor.is_some();
            if unheld && a.ms + ACT_SLACK_MS < first_ms {
                continue;
            }
            let crowned = |head: &String, act| Crowned {
                head: head.clone(),
                title: a.title.clone(),
                road: a.road,
                act,
                cause: a.cause.clone(),
            };
            if let Some(w) = a.words {
                let carried = out.iter().position(|m| {
                    m.kind == RowKind::Prompt
                        && m.crown.is_none()
                        && m.hand.is_none()
                        && m.ms.is_none_or(|ms| ms + ACT_SLACK_MS >= a.ms)
                        && *keys.entry(m.row).or_insert_with(|| words_key(&self.rows[m.row].text))
                            == w
                });
                if let Some(j) = carried {
                    out[j].crown = Some(crowned(&a.head, None));
                    continue;
                }
            }
            if (a.ms < first_ms && unheld) || (a.ms > last_ms && !self.newest) {
                continue;
            }
            let at = out.iter().rposition(|m| m.ms.is_some_and(|ms| ms <= a.ms));
            let (index, key, row) = match at {
                Some(mut at) => {
                    // After the record's own rows, before its hand: the
                    // keys stay in order.
                    let (rec, nth) = (out[at].key.0, usize::MAX / 2 + i);
                    while out.get(at + 1).is_some_and(|m| m.key.0 == rec && m.key.1 < nth) {
                        at += 1;
                    }
                    (at + 1, (rec, nth), out[at].row)
                }
                // Before every message: under the first one's record,
                // which no other message's key reaches.
                None => (0, (first.0.saturating_sub(1), i), out[0].row),
            };
            out.insert(
                index,
                Message {
                    key,
                    kind: RowKind::Reply,
                    row,
                    ms: Some(a.ms),
                    steps: 0,
                    tools: Vec::new(),
                    hand: None,
                    // No prompt held its words: the act's sentence, not
                    // a head over nothing (T-1289).
                    crown: Some(crowned(&a.alone, Some(i))),
                },
            );
        }
    }

    /// The crown's acts for this conversation, as each poll reads them:
    /// true when the messages moved.
    pub(crate) fn set_acts(&mut self, acts: Vec<ActLine>, newest: bool) -> bool {
        if self.acts == acts && self.newest == newest {
            return false;
        }
        self.acts_stamp = stamp_acts(&acts, newest);
        self.acts = acts;
        self.newest = newest;
        self.rebuild();
        true
    }

    /// What the acts are, for a draw cache's key.
    pub(crate) fn acts_stamp(&self) -> u64 {
        self.acts_stamp
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
        if self.is_rename(m) {
            return Rc::new(Vec::new());
        }
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

    /// For a test: the rows before message `m` let go, as if only the rest
    /// had been read so far; they come back through `land_earlier`.
    #[cfg(test)]
    pub(crate) fn hold_back(&mut self, m: usize) -> Vec<TranscriptRow> {
        let rest = self.rows.split_off(self.messages[m].row);
        let earlier = std::mem::replace(&mut self.rows, rest);
        self.floor = self.rows.first().map(|r| r.at);
        self.rebuild();
        earlier
    }

    /// For a test: `rows` arrive as the earlier pages' reader sends a page,
    /// landed by the next `poll`.
    #[cfg(test)]
    pub(crate) fn land_earlier(&mut self, rows: Vec<TranscriptRow>) {
        let end = self.floor.expect("rows held back");
        let from = rows.first().map_or(end, |r| r.at);
        let (tx, rx) = std::sync::mpsc::channel();
        let conversation = self.conversation.clone();
        let _ = tx.send(Page { conversation, rows, from, end, next_before: None });
        self.earlier = Some(rx);
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

    /// The crown's own conversation names the ticket each act touched and
    /// says no cause (its own prompts above say it); a worker's says "this
    /// ticket" and why; the words the board carried head their prompt with
    /// whose they were, across boards with the bolt.
    #[test]
    fn the_crown_s_page_names_the_ticket_and_the_worker_s_says_why() {
        use mesimon_core::crownact::{words_key, Cause, CrownAct, BOARD_REPLIED};
        let mut board = mesimon_core::board::Board::default();
        let worker = ulid::Ulid(14);
        let t: mesimon_core::board::Ticket = serde_json::from_value(serde_json::json!({
            "id": worker, "short_key": "T-14", "title": "Fix login", "column": "TODO",
            "order": "a", "created_at": "@1",
        }))
        .expect("a ticket");
        board.tickets.push(t);
        use mesimon_core::board::AgentProvider;
        use mesimon_core::tier::{Book, MachineTiers, Tier};
        let opus = ulid::Ulid(77).to_string();
        let tiers = MachineTiers {
            default_tier: None,
            tiers: vec![Tier {
                id: opus.clone(),
                name: "Opus".into(),
                provider: AgentProvider::ClaudeCode,
                model: "opus".into(),
                effort: Default::default(),
                description: String::new(),
            }],
        };
        let crown = ulid::Ulid(1);
        let started = |at_ms, detail: &str| CrownAct {
            at_ms,
            crown,
            ticket: worker,
            act: "started".into(),
            detail: detail.into(),
            previous_title: None,
            words: None,
            cause: None,
        };
        let acts = [
            // An older line holds the tier's id: named, or left unsaid.
            started(3, &opus),
            started(4, &ulid::Ulid(78).to_string()),
            CrownAct {
                at_ms: 5,
                crown,
                ticket: worker,
                act: "merged".into(),
                detail: String::new(),
                previous_title: None,
                words: None,
                cause: Some(Cause::Person),
            },
            CrownAct {
                at_ms: 6,
                crown,
                ticket: crown,
                act: BOARD_REPLIED.into(),
                detail: "relay".into(),
                previous_title: None,
                words: Some(words_key("cookie path stays")),
                cause: None,
            },
            CrownAct {
                act: "renamed".into(),
                previous_title: Some("First title".into()),
                detail: "Second title".into(),
                cause: Some(Cause::Person),
                ..started(7, "")
            },
            // Another crown's act on another ticket: neither page's.
            CrownAct {
                at_ms: 7,
                crown: ulid::Ulid(9),
                ticket: ulid::Ulid(8),
                act: "moved".into(),
                detail: String::new(),
                previous_title: None,
                words: None,
                cause: None,
            },
        ];
        let own = acts_of(&board, &Book::new(&tiers, &board), &acts, crown);
        let heads: Vec<_> =
            own.iter().map(|a| (a.head.as_str(), a.road, a.cause.clone())).collect();
        assert_eq!(
            heads,
            vec![
                ("crown started T-14 “Fix login”'s agent on Opus", false, None),
                ("crown started T-14 “Fix login”'s agent", false, None),
                ("crown merged T-14 “Fix login”'s branch", false, None),
                ("the relay board's crown replied", true, None),
                ("crown renamed T-14", false, None),
            ]
        );
        assert_eq!(own[3].words, Some(words_key("cookie path stays")));
        let theirs = acts_of(&board, &Book::new(&tiers, &board), &acts, worker);
        assert_eq!(theirs.len(), 4);
        assert_eq!(theirs[3].head, "crown renamed this ticket");
        assert_eq!(theirs[3].body, "Before  First title\nAfter   Second title");
        assert_eq!(own[4].body, theirs[3].body);
        assert_eq!(theirs[3].cause.as_deref(), Some("after you asked the crown"));
        assert_eq!(theirs[0].head, "crown started this ticket's agent on Opus");
        assert_eq!(theirs[2].head, "crown merged this ticket's branch");
        assert_eq!(theirs[2].cause.as_deref(), Some("after you asked the crown"));
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

    /// T-1289, from a board's files (words replaced, fields as written):
    /// the crown's ask to a worker, the worker's prompt of it on the mod's
    /// fill road, and a wake the board sent the crown twice in a day.
    const ASKED_AT: u64 = 1_794_848_150_974;
    const ASK: &str = "Crown — last retarget tonight: the owner approved two more.\n\n\
        1. Wait for the new build.\n2. Run the dialog checks again.\n\nReport as before.";
    const WAKE: &str = "T-472 \"Group by\" merged (merge_state ahead → merged) ∙ \
        get_ticket key=T-472 for state and notes";

    fn said_at(ts: &str, kind: &str, text: &str, extra: serde_json::Value) -> String {
        let mut v = match kind {
            "user" => serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": text },
            }),
            _ => serde_json::json!({
                "type": "assistant",
                "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] },
            }),
        };
        v["uuid"] = format!("{kind}-{ts}").into();
        v["timestamp"] = ts.into();
        v["isSidechain"] = false.into();
        v["userType"] = "external".into();
        if let (Some(v), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
            v.extend(extra.clone());
        }
        format!("{v}\n")
    }

    /// The crown's ask is the worker's prompt, headed as the crown's; where
    /// no prompt holds it (the crown's own page, which holds the call and
    /// not the words, or a worker's before the words land) the row says the
    /// act's sentence, never a bare head over nothing. A wake sent twice
    /// marks its own prompt, not the other's.
    #[test]
    fn a_crown_ask_no_prompt_holds_says_its_sentence() {
        use mesimon_core::crownact::{words_key, Cause, CrownAct, WOKE_CROWN};
        let crown = ulid::Ulid(1);
        let worker = ulid::Ulid(479);
        let mut board = mesimon_core::board::Board::default();
        for (id, key, title) in [(crown, "T-1", "Crown"), (worker, "T-479", "Staging tester")] {
            board.tickets.push(
                serde_json::from_value(serde_json::json!({
                    "id": id, "short_key": key, "title": title, "column": "TODO",
                    "order": "a", "created_at": "@1",
                }))
                .expect("a ticket"),
            );
        }
        let act = |at_ms, ticket, act: &str, detail: &str, words, cause| CrownAct {
            at_ms,
            crown,
            ticket,
            act: act.into(),
            detail: detail.into(),
            previous_title: None,
            words,
            cause,
        };
        let acts = [
            act(1_794_836_565_418, crown, WOKE_CROWN, "T-472 merged", Some(words_key(WAKE)), None),
            act(1_794_847_835_692, crown, WOKE_CROWN, "T-472 merged", Some(words_key(WAKE)), None),
            act(1_794_848_133_400, worker, "tagged", "MAINTENANCE", None, Some(Cause::Person)),
            act(ASKED_AT, worker, "asked", "", Some(words_key(ASK)), Some(Cause::Person)),
        ];
        let tiers = mesimon_core::tier::MachineTiers::default();
        let book = mesimon_core::tier::Book::new(&tiers, &board);
        let heads = |t: &Transcript| -> Vec<(String, String)> {
            let ms = t.messages().iter();
            ms.filter_map(|m| Some((m.crown.as_ref()?.head.clone(), t.text(m).to_string())))
                .collect()
        };
        let dir = scratch("crown-ask");

        // The worker's page, its prompt landed: the crown's, with its words.
        let prompt = serde_json::json!({
            "origin": { "kind": "human" }, "promptSource": "typed", "turnOrigin": "human",
            "turnPosition": { "promptIndex": 2, "turnIndex": 8 }, "entrypoint": "cli",
        });
        let before =
            said_at("2026-11-16T16:50:24.765Z", "a", "Selectors noted.", Default::default());
        let after =
            said_at("2026-11-16T16:56:02.286Z", "a", "Watching for the build.", Default::default());
        let path = dir.join("worker.jsonl");
        let whole = [
            before.clone(),
            said_at("2026-11-16T16:55:51.062Z", "user", ASK, prompt),
            after.clone(),
        ]
        .concat();
        std::fs::write(&path, whole).unwrap();
        let mut t = Transcript::open(&rec(&path), None).expect("a conversation");
        t.set_acts(acts_of(&board, &book, &acts, worker), true);
        let tagged = ("crown tagged this ticket MAINTENANCE".to_string(), String::new());
        assert_eq!(heads(&t), [tagged.clone(), ("crown asked".into(), ASK.into())]);

        // Before the words land: the act's sentence, its cause under it.
        std::fs::write(&path, [before, after].concat()).unwrap();
        let mut t = Transcript::open(&rec(&path), None).expect("a conversation");
        t.set_acts(acts_of(&board, &book, &acts, worker), true);
        assert_eq!(heads(&t), [tagged, ("crown asked this ticket's agent".into(), String::new())]);
        let row = t.messages().iter().find_map(|m| m.crown.as_ref().filter(|c| c.act.is_some()));
        assert_eq!(row.and_then(|c| c.cause.as_deref()), Some("after you asked the crown"));

        // The crown's page holds its call, not the words: the sentence,
        // naming the ticket. The day's first wake is in a page not yet
        // landed; the second marks its own prompt.
        let wake = serde_json::json!({
            "origin": { "kind": "plugin", "name": "mesimon", "asUser": true },
            "promptSource": "system", "turnOrigin": "system", "queuePriority": "later",
        });
        let path = dir.join("crown.jsonl");
        let crown_file = [
            said_at("2026-11-16T13:42:45.579Z", "user", WAKE, wake.clone()),
            said_at("2026-11-16T13:42:50.000Z", "a", "No action.", Default::default()),
            said_at("2026-11-16T16:50:35.704Z", "user", WAKE, wake),
            said_at("2026-11-16T16:55:34.984Z", "a", "Merged; retargeting.", Default::default()),
            said_at("2026-11-16T16:55:56.499Z", "a", "All three are merged.", Default::default()),
        ]
        .concat();
        std::fs::write(&path, crown_file).unwrap();
        let mut t = Transcript::open(&rec(&path), None).expect("a conversation");
        t.set_acts(acts_of(&board, &book, &acts, crown), true);
        let _earlier = t.hold_back(2);
        let woke = "the board woke the crown".to_string();
        let tagged = "crown tagged T-479 “Staging tester” MAINTENANCE".to_string();
        let asked = "crown asked T-479 “Staging tester”'s agent".to_string();
        assert_eq!(
            heads(&t),
            [(woke, WAKE.into()), (tagged, String::new()), (asked, String::new())],
            "{:?}",
            t.messages()
        );
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
