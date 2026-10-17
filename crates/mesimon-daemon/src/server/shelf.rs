//! The shelf (T-698): what this host leaves at the relay for each paired
//! browser to read while it is away. The board as its snapshot reads, the
//! notes of every ticket still in play, and the newest page of each
//! conversation Now lists, each sealed to one browser alone, so the relay
//! keeps what it cannot open. A slot is replaced only when what it holds
//! changed; the board is written again every few minutes regardless, so
//! the browser's "as of" says when this host was last there.
use super::*;
use mesimon_core::mesophon::{self as api, Reply, ShelfItem, Shelved};
use mesimon_team::{
    control::{self, Wire, SHELF_BYTES, SHELF_PLAIN_BYTES, SHELF_SLOTS},
    crypto::BoardId,
};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

/// How often a moving board is shelved at most.
const SHELF_EVERY: Duration = Duration::from_secs(30);
/// How often the board is shelved again unchanged: its `at` is how a
/// browser tells how long this host has been away.
const SHELF_REFRESH: Duration = Duration::from_secs(300);
/// Frames handed to the relay's worker per tick: its queue holds 32, and a
/// full queue drops the connection (`Control::send`).
const FRAMES_PER_TICK: usize = 2;
/// The most conversations shelved, and the most tickets' notes: with the
/// board, inside the relay's `SHELF_SLOTS`.
const TRANSCRIPTS: usize = 10;
const NOTES: usize = SHELF_SLOTS - 1 - TRANSCRIPTS;
/// The plain bytes one browser's shelf may hold: sealed and in hex it is
/// about twice this, inside the relay's `SHELF_TOTAL_BYTES`.
const PLAIN_TOTAL: usize = 960 * 1024;
/// What a conversation not yet read is counted as: a page's most.
const PAGE_GUESS: usize = 48 * 1024;
/// How long a stopped agent stays on Now, and its conversation on the
/// shelf: the page's `RECENT_MS`.
const RECENT_MS: u64 = 3_600_000;
/// How long a Codex seat's rollout is trusted once it stops growing before
/// `~/.codex/sessions` is walked again for a continuation (T-815, T-840).
const ROLLOUT_WALK: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Shelf {
    /// Each grant's slots as the relay holds them, by name.
    held: HashMap<BoardId, HashMap<String, Held>>,
    /// Each name's plain size when last built: the budget's measure of a
    /// slot that is not built again.
    sizes: HashMap<String, usize>,
    /// Frames on their way to the relay, a few a tick.
    outbox: VecDeque<Wire>,
    /// No build before this.
    next: Option<Instant>,
    /// The board moved since the last build.
    pub(super) dirty: bool,
    /// A build whose conversations are being read off the writer thread.
    reading: bool,
    /// Bumped whenever what the relay holds is unknown again (a new
    /// connection, the copy turned off): a read from before lands on
    /// nothing.
    generation: u64,
    /// Each Codex seat's rollout as the last walk found it, by session: a
    /// fact about the disk, kept across `forget`.
    rollouts: HashMap<String, Rollout>,
}
/// Where a Codex conversation is written now: its seat's rollout or a
/// continuation of it (T-815). Walking `~/.codex/sessions` for one is the
/// cost; a rollout still growing has none, so it is walked for again only
/// once it stops and a minute has passed since the last walk.
struct Rollout {
    original: std::path::PathBuf,
    current: std::path::PathBuf,
    len: u64,
    walked: Instant,
}
#[derive(Clone, Copy)]
struct Held {
    key: u64,
    sent: Instant,
}
impl Shelf {
    /// What the relay holds is unknown: everything goes again.
    pub(super) fn forget(&mut self) {
        self.held.clear();
        self.outbox.clear();
        self.reading = false;
        self.next = None;
        self.dirty = true;
        self.generation += 1;
    }
}

/// The rollout a Codex seat writes, while the last walk still holds; `None`
/// asks the reader to walk again.
fn rollout(
    rollouts: &mut HashMap<String, Rollout>,
    session: &str,
    original: &std::path::Path,
    now: Instant,
) -> Option<std::path::PathBuf> {
    let r = rollouts.get_mut(session).filter(|r| r.original == original)?;
    let len = std::fs::metadata(&r.current).ok()?.len();
    let grew = len > r.len;
    r.len = len;
    (grew || now.duration_since(r.walked) < ROLLOUT_WALK).then(|| r.current.clone())
}

/// One build: the items in shelf order, with the bodies read on the writer
/// thread; a conversation's body comes with `PageRead`.
pub(crate) struct Build {
    generation: u64,
    at: u64,
    items: Vec<Planned>,
}
struct Planned {
    name: String,
    key: u64,
    /// `None` where every grant holds it already, or while it is read.
    body: Option<Shelved>,
    /// A conversation being read; one that fails to read leaves the shelf.
    pending: bool,
}
/// A conversation's newest page, read off the writer thread.
pub(crate) struct PageRead {
    name: String,
    ticket: String,
    session: String,
    page: Option<Reply>,
    /// The slot's key, where the plan could not know it: a walk's.
    key: Option<u64>,
    rollout: Option<Rollout>,
}
struct Read {
    name: String,
    ticket: String,
    session: String,
    kind: SessionKind,
    path: std::path::PathBuf,
    /// A Codex seat's own rollout, whose continuation is looked for first.
    walk: bool,
}
impl Read {
    fn run(self) -> PageRead {
        let path = if self.walk {
            crate::agents::codex::discovery::current_rollout(&self.path)
        } else {
            self.path.clone()
        };
        // Taken before the read, so a write in between moves the key again.
        let stat = self.walk.then(|| stat_key(&path, &self.session)).flatten();
        let page = crate::agents::read_transcript_reply_at(
            self.kind,
            &path,
            crate::agents::transcript::Ask::default(),
            None,
        );
        let rollout = stat.map(|(_, len)| Rollout {
            original: self.path,
            current: path,
            len,
            walked: Instant::now(),
        });
        PageRead {
            name: self.name,
            ticket: self.ticket,
            session: self.session,
            page,
            key: stat.map(|(key, _)| key),
            rollout,
        }
    }
}

/// A conversation's slot key, off the file it is read from: its size and
/// age, and whose it is. With its size, for the rollout's growth.
fn stat_key(path: &std::path::Path, session: &str) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms).unwrap_or(0);
    Some((key_of(&(path, meta.len(), mtime, session)), meta.len()))
}

fn key_of(value: &impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}
fn plain_size(at: u64, body: &Shelved) -> usize {
    serde_json::to_vec(&ShelfItem { at, held: body.clone() }).map_or(usize::MAX, |b| b.len())
}

impl Daemon {
    /// Whether this board keeps a copy at the relay now: the setting, a
    /// relay that keeps one, and a connection to it.
    fn shelf_on(&self) -> bool {
        self.control.online
            && self.control.shelf_relay
            && self.control.keys.is_some()
            && self.control.stored.as_ref().is_some_and(|s| !s.shelf_off)
    }

    /// Every tick: hand a few frames on, and build when the board moved,
    /// an agent works, or the board's copy is due again.
    pub(super) fn shelf_tick(&mut self) {
        if !self.shelf_on() {
            return;
        }
        for _ in 0..FRAMES_PER_TICK {
            let Some(wire) = self.control.shelf.outbox.pop_front() else { break };
            self.control.send(wire);
        }
        let now = Instant::now();
        let shelf = &self.control.shelf;
        let Some(s) = self.control.stored.as_ref().filter(|s| !s.grants.is_empty()) else {
            return;
        };
        if shelf.reading || !shelf.outbox.is_empty() {
            return;
        }
        // A browser paired since, with no board on its shelf, is owed one
        // now; the others wait out `SHELF_EVERY`.
        let held = |g: &super::mesophon::Grant| shelf.held.get(&g.id);
        let unshelved = s.grants.iter().any(|g| held(g).is_none_or(|h| !h.contains_key("board")));
        if shelf.next.is_some_and(|n| now < n) && !unshelved {
            return;
        }
        let stale = s.grants.iter().any(|g| {
            held(g)
                .and_then(|h| h.get("board"))
                .is_none_or(|b| now.duration_since(b.sent) >= SHELF_REFRESH)
        });
        let working = self.board.sessions.iter().any(busy);
        if !(shelf.dirty || stale || working) {
            return;
        }
        self.control.shelf.dirty = false;
        self.control.shelf.next = Some(now + SHELF_EVERY);
        if let Some((build, reads)) = self.shelf_plan() {
            if reads.is_empty() {
                self.shelf_finish(build, Vec::new());
                return;
            }
            self.control.shelf.reading = true;
            let tx = self.control.tx.clone();
            std::thread::spawn(move || {
                let pages = reads.into_iter().map(Read::run).collect();
                let _ = tx.send(Msg::ShelfRead(Box::new(build), pages));
            });
        }
    }

    /// A build's conversations landed (`Msg::ShelfRead`).
    pub(super) fn shelf_read(&mut self, build: Build, mut pages: Vec<PageRead>) {
        for p in &mut pages {
            if let Some(r) = p.rollout.take() {
                self.control.shelf.rollouts.insert(p.session.clone(), r);
            }
        }
        if build.generation != self.control.shelf.generation {
            return;
        }
        self.control.shelf.reading = false;
        if self.shelf_on() {
            self.shelf_finish(build, pages);
        }
    }

    /// The setting (T-698): on, the next tick builds; off, every browser's
    /// shelf at the relay is emptied and nothing more is written.
    pub(super) fn shelf_set(&mut self, on: bool) -> Result<(), &'static str> {
        let Some(mut s) = self.control.stored.clone() else {
            return Err("Remote Control is disabled");
        };
        if s.shelf_off == !on {
            return Ok(());
        }
        // An older build that saved this file would drop the field and keep
        // a copy again; off is written as a schema it bars instead.
        (s.shelf_off, s.schema) = (!on, if on { 1 } else { 2 });
        if self.control_save(&s).is_err() {
            return Err("could not save the setting");
        }
        if !on && self.control.online && self.control.shelf_relay {
            for g in &s.grants {
                self.control.send(Wire::Shelve { device: g.device, keep: Vec::new(), item: None });
            }
        }
        self.control.stored = Some(s);
        self.control.shelf.forget();
        Ok(())
    }

    /// What the shelf holds, in order and inside its budget: the board, the
    /// conversations Now lists, then the notes of tickets still in play.
    /// Bodies are built here only for slots some grant lacks or holds an
    /// older copy of; conversations to read come back beside the build.
    fn shelf_plan(&mut self) -> Option<(Build, Vec<Read>)> {
        let grants: Vec<BoardId> =
            self.control.stored.as_ref()?.grants.iter().map(|g| g.id).collect();
        if grants.is_empty() {
            return None;
        }
        let at = mesimon_core::clock::now_ms();
        let held = &self.control.shelf.held;
        let everywhere = |name: &str, key: u64| {
            grants
                .iter()
                .all(|g| held.get(g).and_then(|h| h.get(name)).is_some_and(|h| h.key == key))
        };
        let mut items = Vec::new();
        let mut reads = Vec::new();
        let mut spent = 0;

        let board = self.shelf_board();
        let size = plain_size(at, &board);
        spent += size;
        let key = serde_json::to_vec(&board).map_or(0, |b| key_of(&b));
        items.push(Planned { name: "board".into(), key, body: Some(board), pending: false });

        // Now's conversations: whoever needs you or works, then the agents
        // that stopped within the hour, the latest first.
        let mut now_list: Vec<(&Ticket, &SessionRecord)> = self
            .board
            .tickets
            .iter()
            .filter_map(|t| Some((t, self.board.live_agent(t.id)?)))
            .filter(|(_, s)| s.kind.is_agent() && shelved_by_now(s, at))
            .collect();
        now_list.sort_by_key(|(_, s)| (!busy(s), std::cmp::Reverse(s.state_changed_at)));
        let shown: Vec<String> =
            now_list.iter().take(TRANSCRIPTS).map(|(_, s)| s.id.to_string()).collect();
        let rollouts = &mut self.control.shelf.rollouts;
        rollouts.retain(|session, _| shown.contains(session));
        let now = Instant::now();
        for ((t, s), session) in now_list.iter().zip(shown) {
            let Some(original) = crate::cost::transcript_of(s).map(|(p, _)| p) else { continue };
            // A Codex conversation may go on in a continued rollout (T-815):
            // one the last walk no longer vouches for is walked on the reader.
            let walk = s.kind == SessionKind::Codex;
            let path = if walk {
                rollout(rollouts, &session, &original, now)
            } else {
                Some(original.clone())
            };
            let stat = path.and_then(|p| Some((stat_key(&p, &session)?.0, p)));
            let (key, path, walk) = match stat {
                Some((key, path)) => (key, path, false),
                None if walk => (0, original, true),
                None => continue,
            };
            let name = format!("transcript:{}", t.id);
            spent += self.control.shelf.sizes.get(&name).copied().unwrap_or(PAGE_GUESS);
            if !walk && everywhere(&name, key) {
                items.push(Planned { name, key, body: None, pending: false });
                continue;
            }
            reads.push(Read {
                name: name.clone(),
                ticket: t.id.to_string(),
                session,
                kind: s.kind,
                path,
                walk,
            });
            items.push(Planned { name, key, body: None, pending: true });
        }

        // Notes: the tickets Now lists first, then the board's order; a
        // column that takes finished work (merged, or its worktree
        // reclaimed) is history and keeps its notes at the desk.
        let mut noted: Vec<&Ticket> = now_list.iter().map(|(t, _)| *t).collect();
        for c in self.board.sorted_columns() {
            if c.settings.requires_merge || c.settings.reclaim {
                continue;
            }
            for t in self.board.column_tickets(&c.name) {
                if !noted.iter().any(|n| n.id == t.id) {
                    noted.push(t);
                }
            }
        }
        let finished = |t: &Ticket| {
            self.board
                .column(&t.column)
                .is_some_and(|c| c.settings.requires_merge || c.settings.reclaim)
        };
        let mut count = 0;
        let mut built = Vec::new();
        for t in noted {
            if count >= NOTES || t.notes.is_empty() || finished(t) {
                continue;
            }
            let name = format!("notes:{}", t.id);
            let authors: Vec<String> = t.notes.iter().map(|n| self.control_author(n)).collect();
            let key = key_of(&(api::notes_stamp(&t.notes), &authors));
            let body = if everywhere(&name, key) {
                None
            } else {
                match self.shelf_notes(t) {
                    Some(body) => Some(body),
                    None => continue,
                }
            };
            let size = match &body {
                Some(b) => plain_size(at, b),
                None => self.control.shelf.sizes.get(&name).copied().unwrap_or(4 * 1024),
            };
            if spent + size > PLAIN_TOTAL {
                continue;
            }
            spent += size;
            count += 1;
            if body.is_some() {
                built.push((name.clone(), size));
            }
            items.push(Planned { name, key, body, pending: false });
        }
        for (name, size) in built {
            self.control.shelf.sizes.insert(name, size);
        }
        Some((Build { generation: self.control.shelf.generation, at, items }, reads))
    }

    /// The board as the shelf holds it: the snapshot a phone draws, with
    /// nothing on it to answer.
    fn shelf_board(&self) -> Shelved {
        let (mut board, _) = self.control_board();
        if let Reply::Board { tickets, .. } = &mut board {
            for agent in tickets.iter_mut().filter_map(|t| t.agent.as_mut()) {
                agent.dialog = None;
                agent.permission = None;
                agent.promptable = false;
            }
        }
        Shelved::Board { board }
    }

    /// A ticket's notes as the shelf holds them: the list and the
    /// description, as the page asks for them, then the other notes'
    /// bodies, the latest written first, while they fit one item.
    fn shelf_notes(&self, t: &Ticket) -> Option<Shelved> {
        let ticket = t.id.to_string();
        let principal = Principal::Local;
        let notes = self.control_notes(&principal, &ticket);
        let Reply::Notes { notes: rows, description, .. } = &notes else { return None };
        let mut budget = SHELF_PLAIN_BYTES
            .saturating_sub(serde_json::to_vec(&notes).map_or(usize::MAX, |b| b.len()) + 1024);
        let mut order: Vec<&api::NoteRow> = rows.iter().skip(1).collect();
        order.sort_by_key(|r| std::cmp::Reverse(r.at));
        // A note's body, where it is one and fits what is left.
        let mut fit = |id: &str| {
            let body = self.control_note(&principal, &ticket, id);
            let size = serde_json::to_vec(&body).map_or(usize::MAX, |b| b.len());
            (matches!(body, Reply::Note { .. }) && size <= budget).then(|| {
                budget -= size;
                body
            })
        };
        let mut bodies: Vec<Reply> = order.into_iter().filter_map(|row| fit(&row.id)).collect();
        // A description too long to ride with the list was left off it
        // (`control_notes`): it rides as a body, when it fits.
        if description.is_none() {
            if let Some(body) = rows.first().and_then(|first| fit(&first.id)) {
                bodies.insert(0, body);
            }
        }
        Some(Shelved::Notes { ticket, notes, bodies })
    }

    /// Seal what changed for each grant and queue its frames: every frame
    /// names the whole shelf, so a slot this build left out is emptied.
    fn shelf_finish(&mut self, mut build: Build, pages: Vec<PageRead>) {
        for p in pages {
            let Some(item) = build.items.iter_mut().find(|i| i.name == p.name) else { continue };
            item.pending = false;
            match p.page {
                Some(page) => {
                    if let Some(key) = p.key {
                        item.key = key;
                    }
                    let body = Shelved::Transcript { ticket: p.ticket, session: p.session, page };
                    self.control.shelf.sizes.insert(p.name, plain_size(build.at, &body));
                    item.body = Some(body);
                }
                None => item.pending = true,
            }
        }
        build.items.retain(|i| !i.pending);
        let (Some(s), Some(keys)) = (self.control.stored.as_ref(), self.control.keys.as_ref())
        else {
            return;
        };
        let now = Instant::now();
        let mut frames = Vec::new();
        let mut held_now = HashMap::new();
        for g in &s.grants {
            let mut choice = choose(self.control.shelf.held.get(&g.id), &build.items, now);
            let mut keep = Vec::new();
            let mut sealed = Vec::new();
            for (i, item) in build.items.iter().enumerate() {
                let slot = control::shelf_slot(g.id, &item.name);
                if choice.seal.contains(&i) {
                    let value = item.body.as_ref().and_then(|body| {
                        serde_json::to_value(ShelfItem { at: build.at, held: body.clone() }).ok()
                    });
                    let envelope = value
                        .and_then(|v| {
                            control::seal_shelf(s.board, g.id, slot, keys, &g.public, v).ok()
                        })
                        .filter(|e| serde_json::to_vec(e).is_ok_and(|b| b.len() <= SHELF_BYTES));
                    match envelope {
                        Some(e) => sealed.push(e),
                        None => {
                            choice.held.remove(&item.name);
                            continue;
                        }
                    }
                } else if !choice.held.contains_key(&item.name) {
                    continue;
                }
                keep.push(slot);
            }
            if sealed.is_empty() && choice.emptied {
                frames.push(Wire::Shelve { device: g.device, keep: keep.clone(), item: None });
            }
            for envelope in sealed {
                frames.push(Wire::Shelve {
                    device: g.device,
                    keep: keep.clone(),
                    item: Some(Box::new(envelope)),
                });
            }
            held_now.insert(g.id, choice.held);
        }
        self.control.shelf.held = held_now;
        self.control.shelf.outbox.extend(frames);
    }
}

/// What one grant's shelf gets from a build: the items to seal again (by
/// index), what its slots hold afterwards, and whether a slot it held is
/// left out, which a frame must say when nothing else goes.
struct Choice {
    seal: Vec<usize>,
    held: HashMap<String, Held>,
    emptied: bool,
}
fn choose(before: Option<&HashMap<String, Held>>, items: &[Planned], now: Instant) -> Choice {
    let mut seal = Vec::new();
    let mut held = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let old = before.and_then(|h| h.get(&item.name)).copied();
        let fresh = old.is_some_and(|h| {
            h.key == item.key
                && (item.name != "board" || now.duration_since(h.sent) < SHELF_REFRESH)
        });
        match (&item.body, old) {
            (_, Some(h)) if fresh => {
                held.insert(item.name.clone(), h);
            }
            (Some(_), _) => {
                seal.push(i);
                held.insert(item.name.clone(), Held { key: item.key, sent: now });
            }
            // Held unchanged by every grant when the build began; not by
            // this one, which paired since: the next build brings it.
            (None, _) => {}
        }
    }
    let emptied = before.is_some_and(|h| h.keys().any(|name| !held.contains_key(name)));
    Choice { seal, held, emptied }
}

/// Whether Now lists this agent, as the page's `nowGroup` does: one that
/// needs you or works, always; any other while its state is under an hour
/// old.
fn shelved_by_now(s: &SessionRecord, now: u64) -> bool {
    busy(s) || s.state_changed_at.is_some_and(|at| now.saturating_sub(at) < RECENT_MS)
}
fn busy(s: &SessionRecord) -> bool {
    matches!(
        s.state,
        SessionState::Spawning | SessionState::Running | SessionState::RequiresAction { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, key: u64, body: bool) -> Planned {
        let body = body.then_some(Shelved::Board { board: Reply::Changed });
        Planned { name: name.into(), key, body, pending: false }
    }
    fn names(items: &[Planned], seal: &[usize]) -> Vec<String> {
        seal.iter().map(|&i| items[i].name.clone()).collect()
    }

    #[test]
    fn codex_shelf_reads_follow_rotation_and_only_changed_pages_need_sending() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("rollout-start-thread.jsonl");
        let continued = directory.path().join("rollout-next-thread_segment.jsonl");
        let write = |path: &std::path::Path, text: &str, seconds| {
            let metadata =
                serde_json::json!({"type":"session_meta","payload":{"id":"thread","cwd":"/repo"}});
            let message = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}});
            std::fs::write(path, format!("{metadata}\n{message}\n")).unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        };
        let read = |session: &str| {
            Read {
                name: "transcript:a".into(),
                ticket: "a".into(),
                session: session.into(),
                kind: SessionKind::Codex,
                path: original.clone(),
                walk: true,
            }
            .run()
        };
        write(&original, "Original", 100);
        let old = read("session");
        write(&continued, "Continued", 200);
        let current = read("session");
        assert_ne!(old.key, current.key);
        assert_eq!(current.key, read("session").key);
        assert_ne!(current.key, read("replacement").key);
        let Some(Reply::Transcript { rows, .. }) = current.page else {
            panic!("shelved transcript")
        };
        assert_eq!(rows[0].text, "Continued");
        let now = Instant::now();
        let first = choose(None, &[item("transcript:a", old.key.unwrap(), true)], now);
        let changed =
            choose(Some(&first.held), &[item("transcript:a", current.key.unwrap(), true)], now);
        assert_eq!(changed.seal, [0]);
        let unchanged =
            choose(Some(&changed.held), &[item("transcript:a", current.key.unwrap(), true)], now);
        assert!(unchanged.seal.is_empty());

        // The walk is remembered (T-840): the continuation is read with no
        // walk while it grows or the walk is under a minute old.
        let walked = current.rollout.expect("a walk is remembered");
        assert_eq!(walked.current, continued);
        let t0 = walked.walked;
        let mut rollouts = HashMap::from([("session".to_string(), walked)]);
        assert_eq!(rollout(&mut rollouts, "session", &original, t0), Some(continued.clone()));
        assert_eq!(rollout(&mut rollouts, "other", &original, t0), None);
        assert_eq!(rollout(&mut rollouts, "session", &continued, t0), None, "another seat");
        let later = t0 + ROLLOUT_WALK;
        assert_eq!(rollout(&mut rollouts, "session", &original, later), None, "stopped a minute");
        write(&continued, "Continued, and more", 300);
        assert_eq!(rollout(&mut rollouts, "session", &original, later), Some(continued.clone()));
        assert_eq!(rollout(&mut rollouts, "session", &original, later), None, "stopped again");
    }

    /// A browser new to the shelf gets every item; then only what changed,
    /// the board again once its copy is five minutes old, and a frame that
    /// empties the slot of an item the build left out.
    #[test]
    fn a_grant_gets_what_changed_and_the_board_every_few_minutes() {
        let t0 = Instant::now();
        let first =
            [item("board", 1, true), item("notes:a", 7, true), item("transcript:a", 9, true)];
        let c = choose(None, &first, t0);
        assert_eq!(names(&first, &c.seal), ["board", "notes:a", "transcript:a"]);
        assert!(!c.emptied);

        // Unchanged everywhere: the notes are not built again (no body).
        let t1 = t0 + Duration::from_secs(30);
        let next =
            [item("board", 1, true), item("notes:a", 7, false), item("transcript:a", 10, true)];
        let c = choose(Some(&c.held), &next, t1);
        assert_eq!(names(&next, &c.seal), ["transcript:a"]);
        assert_eq!(c.held.len(), 3);

        let t2 = t0 + SHELF_REFRESH;
        let c = choose(Some(&c.held), &next, t2);
        assert_eq!(names(&next, &c.seal), ["board"], "the board's copy is due again");
        assert_eq!(c.held["board"].sent, t2);

        let gone = [item("board", 1, true), item("notes:a", 7, false)];
        let c = choose(Some(&c.held), &gone, t2);
        assert!(c.seal.is_empty() && c.emptied, "the conversation's slot is emptied");
        assert!(!c.held.contains_key("transcript:a"));

        // A browser paired mid-build lacks what the build did not carry.
        let c = choose(None, &gone, t2);
        assert_eq!(names(&gone, &c.seal), ["board"]);
        assert!(!c.held.contains_key("notes:a"));
    }

    /// Now's rule, as the page has it: a busy agent always, any other for
    /// an hour after its state changed.
    #[test]
    fn the_shelf_keeps_the_conversations_now_lists() {
        use mesimon_core::board::StopReason;
        let now = 10 * RECENT_MS;
        let mut s = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid(1),
            Vec::new(),
            "/".into(),
            SessionState::Running,
        );
        s.state_changed_at = Some(1);
        assert!(shelved_by_now(&s, now));
        s.state = SessionState::Idle { stop_reason: StopReason::EndTurn };
        assert!(!shelved_by_now(&s, now));
        s.state_changed_at = Some(now - RECENT_MS + 1);
        assert!(shelved_by_now(&s, now));
        s.state = SessionState::Sleeping;
        assert!(shelved_by_now(&s, now), "a parked agent stays on Now its hour");
    }
}
