//! The board speaks from a thread of its own (T-291).
//!
//! T-282 hung the whole notification channel off `App::tick` — the differ ran
//! in `absorb`, the coalescer beat once per frame, and the post went out on
//! the main loop's spawn-and-forget seam beside `pending_open`. That works
//! exactly while the board is the thing running, and there is one case where
//! it is not: a handover. `handover::run` blocks on `cmd.status()` for the
//! whole life of an attached pane, a `!` terminal or a `^g` editor, so no
//! tick runs, no snapshot arrives, and nothing is said. Twenty minutes
//! heads-down in one agent's pane, nine other agents finishing behind it,
//! silence until you detach — which is the case the feature exists for.
//!
//! So dispatch moves here: a thread with its own daemon connection, alive for
//! the life of the board process. It subscribes, diffs the snapshots
//! ([`mesimon_core::notify::Differ`]), coalesces them
//! ([`mesimon_core::notify::Coalescer`]) and posts, and none of that depends
//! on the main loop being anywhere in particular.
//!
//! **Why a thread and not the daemon.** The daemon already knows every edge
//! (`Change.attention_added` has been sitting in `attention.rs` with no
//! consumer since M2), so it could post these itself — and then "a closed
//! board is silent", D15's constraint and the sentence the Settings row
//! says, would stop being true. Keeping the speaker inside the board process
//! keeps the process dying as the off switch, needs no wire command, and does
//! not spend the wire protocol on what is a view concern. Notifying with no
//! board open at all is a different feature, and a surprising one.
//!
//! **What the main loop still owns.** Three facts, pushed here rather than
//! polled from here, because only the loop has them: whether anybody is
//! looking ([`Presence`] — focus events, keypresses, and the board going off
//! screen around every handover), the five preference fields, and the
//! terminal itself ([`Console`], for the two rungs that write an escape
//! rather than spawning a program).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mesimon_core::board::Board;
use mesimon_core::command::{Command, Response};
use mesimon_core::notify::{Coalescer, Detail, Differ, Post, Presence, Sound, Voice};

use crate::client::{Client, Transport};
use crate::notify::{Channels, Console};

/// How often the thread looks. The coalescing window is 5 s, so a quarter of
/// a second is far below anything a person can perceive on a channel whose
/// whole job is "something changed, go look" — and a control message (a sound
/// preview, the board going away) wakes it at once rather than waiting it out.
const BEAT: Duration = Duration::from_millis(250);

/// How often a thread with no connection dials again — the board's own
/// reconnect cadence, for the same reason: a daemon that is down does not
/// improve for being asked four times a second.
const DIAL_EVERY: Duration = Duration::from_secs(2);

/// The five preference fields the thread reads. Copied out of `prefs.json`
/// rather than shared with it: `Prefs` is the main loop's, and this is the
/// whole of what a notification decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NotifyPrefs {
    /// The switch. Off is the default, and off means the thread holds no
    /// connection at all.
    pub on: bool,
    /// Also say something when a turn finishes, not only when an agent needs
    /// you.
    pub done: bool,
    /// Show the banner even while somebody is looking at the board.
    pub focused: bool,
    /// Say it even about the ticket whose agent pane the user is attached to
    /// (T-292) — the one row that governs the sound as well.
    pub in_pane: bool,
    /// May a banner quote the agent's own words, or only name the ticket
    /// (T-292)? Read here rather than in the wording because it decides
    /// whether the transcript is read at all.
    pub words: bool,
    pub sound_needs_you: Sound,
    pub sound_done: Sound,
}

impl From<&crate::prefs::Prefs> for NotifyPrefs {
    fn from(p: &crate::prefs::Prefs) -> Self {
        NotifyPrefs {
            on: p.notify,
            done: p.notify_done,
            focused: p.notify_focused,
            in_pane: p.notify_in_pane,
            words: p.notify_words,
            sound_needs_you: p.notify_sound_needs_you,
            sound_done: p.notify_sound_done,
        }
    }
}

/// What the main loop writes and the thread reads.
///
/// The clock is here rather than on either side because both sides use it:
/// the main loop stamps a keypress with it and the thread reads that stamp
/// back thirty seconds later. Two `Instant`s of their own would disagree by
/// however long the board took to start, which is exactly the length of a
/// presence window.
#[derive(Debug)]
struct Shared {
    /// Behind a lock only so a test can rewind it; the read is a few times a
    /// second and never contended.
    started: Mutex<Instant>,
    presence: Mutex<Presence>,
    prefs: Mutex<NotifyPrefs>,
    /// Something the ladder could not do (no such program, a terminal that
    /// refused the write), for the status line to say once. The thread has no
    /// status line of its own.
    trouble: Mutex<Option<String>>,
}

impl Shared {
    fn new(prefs: NotifyPrefs) -> Shared {
        Shared {
            started: Mutex::new(Instant::now()),
            presence: Mutex::new(Presence::default()),
            prefs: Mutex::new(prefs),
            trouble: Mutex::new(None),
        }
    }

    /// The board's own monotonic clock, in milliseconds. Monotonic on
    /// purpose — a wall clock that steps back would hold a notification for
    /// hours.
    fn now_ms(&self) -> u64 {
        self.started.lock().unwrap_or_else(|e| e.into_inner()).elapsed().as_millis() as u64
    }

    /// A poisoned lock is a panic somewhere else, not a reason to stop
    /// talking: step over it, as `prefs` and `Console` do.
    fn presence(&self) -> MutexGuard<'_, Presence> {
        self.presence.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn prefs(&self) -> NotifyPrefs {
        *self.prefs.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The one message worth waking the thread for.
enum Ctrl {
    /// A Settings row's sound preview. The row's cursor IS the preview (the
    /// theme picker's rule), so it may not wait for a beat — and this is why
    /// the loop blocks on a channel rather than sleeping.
    Preview(Sound),
}

/// The handle the board keeps. Parked on `App` by `lib.rs` and nowhere else
/// — the rule the two ladders and `opener` already follow, so no test app and
/// no golden opens a second connection or makes a noise.
///
/// Dropping it ends the thread: the control channel disconnects, the loop
/// returns, and the daemon prunes the subscription.
pub struct Notifier {
    shared: Arc<Shared>,
    console: Arc<Console>,
    ctrl: Sender<Ctrl>,
}

impl Notifier {
    pub fn start(repo_root: &Path, channels: Channels, prefs: NotifyPrefs) -> Notifier {
        let shared = Arc::new(Shared::new(prefs));
        let console = Arc::new(Console::default());
        let (tx, rx) = channel();
        // The two per-board constants are resolved in one place: what this
        // board is CALLED, and what its banners are grouped under (T-292).
        // `notify::find` knows no repo, so the group lands here.
        let channels = Channels {
            group: crate::notify::group_for(repo_root),
            icon_dir: mesimon_daemon::Paths::for_repo(repo_root)
                .ok()
                .and_then(|p| p.state_dir.parent().map(|home| home.join("notifications"))),
            ..channels
        };
        let worker = Worker::new(
            repo_root.to_path_buf(),
            title_of(repo_root),
            shared.clone(),
            say_through(channels, console.clone(), shared.clone()),
        );
        std::thread::spawn(move || worker.run(&rx));
        Notifier { shared, console, ctrl: tx }
    }

    /// The terminal handle, cloned out so the draw can hold its lock while
    /// the rest of `App` is borrowed.
    pub fn console(&self) -> Arc<Console> {
        self.console.clone()
    }

    /// A key is a person: it stands in for focus on a terminal that reports
    /// none.
    pub fn saw_key(&self) {
        let now = self.shared.now_ms();
        self.shared.presence().saw_key(now);
    }

    /// The terminal said whether anybody is looking. From the first one of
    /// these on it is the only source the presence rule consults.
    pub fn saw_focus(&self, focused: bool) {
        let now = self.shared.now_ms();
        self.shared.presence().saw_focus(focused, now);
    }

    /// The board gave its terminal away (a handover), or took it back — and
    /// `watching` names the ticket whose agent pane took its place, when one
    /// did. Both halves move together: the presence rule stops believing
    /// focus, and the escape rungs stop writing.
    pub fn saw_board(&self, on_screen: bool, watching: Option<ulid::Ulid>) {
        self.shared.presence().saw_board(on_screen, watching);
        self.console.set_held(on_screen);
    }

    /// A preference changed. Pushed on every `App::set_pref`, so the thread
    /// never reads a stale switch.
    pub fn set_prefs(&self, p: NotifyPrefs) {
        *self.shared.prefs.lock().unwrap_or_else(|e| e.into_inner()) = p;
    }

    /// A Settings row's sound preview, said at once.
    pub fn preview(&self, s: Sound) {
        if !s.is_off() {
            let _ = self.ctrl.send(Ctrl::Preview(s));
        }
    }

    /// What the ladder could not do, for the status line. Taken once.
    pub fn take_trouble(&self) -> Option<String> {
        self.shared.trouble.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// What a ticket is CALLED and what its agent last SAID (T-292) — the two
/// facts the pure module cannot reach, since one is on a board and the other
/// on a file.
///
/// A free function rather than a method so the closure that carries it into
/// `Coalescer::due` borrows [`Worker::board`] alone: `batch` is borrowed
/// mutably in the same expression, and Rust only splits those when the
/// closure names the field.
///
/// The reply is taken ONLY when [`crate::peek::Peek::reply_key`] is set. The
/// peek falls back to the user's own words prefixed `>` where the window
/// holds no reply — right for a card, wrong here: a banner that quotes the
/// prompt back at the person who typed it says nothing, and says it as
/// though the agent had.
fn detail_for(board: &Board, ticket: ulid::Ulid, words: bool) -> Option<Detail> {
    let title = board.ticket(ticket)?.title.clone();
    let said = words
        .then(|| board.pane_target(ticket))
        .flatten()
        .and_then(|session| {
            let path = crate::peek::preview_path(session)?;
            mesimon_daemon::agents::read_preview(session.kind, Path::new(path))
        })
        .filter(|peek| peek.reply_key.is_some())
        .and_then(|peek| peek.text)
        .map(|reply| crate::text::one_line(&reply))
        .unwrap_or_default();
    Some(Detail { title, said })
}

/// WHO is talking, and about which board: `mesimon - simbly`.
///
/// Two halves, because a banner's title answers two questions and the OS
/// answers neither consistently. The private macOS helper now has Mesimon's
/// own identity, while osascript and other fallback rungs still use their
/// host application's. The title identifies the product on every rung.
/// The board's name is there because a user with two of them open
/// has to be told which one, and the checkout's own directory name is what
/// they call it — the breadcrumb's `mesimon › simbly`, in a field that has
/// no room for a breadcrumb.
///
/// A hyphen rather than mesimon's own `∙`, deliberately: the rungs with one
/// field fold the whole post together with `∙`, so keeping a different mark
/// here is what stops `mesimon ∙ simbly ∙ T-12 ∙ …` from reading as four
/// peers when the first two are one source and the rest are its news.
///
/// A root with no name is the product alone rather than a dangling hyphen.
fn title_of(repo_root: &Path) -> String {
    let board =
        repo_root.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty());
    match board {
        Some(name) => format!("mesimon - {name}"),
        None => "mesimon".into(),
    }
}

/// How a post reaches a person. A closure rather than a `Channels` field so a
/// test can drive the whole worker without a program on the machine ever
/// being run — the same seam `App::notify` being `None` used to be.
type Say = Box<dyn FnMut(&Post) + Send>;

fn say_through(ch: Channels, console: Arc<Console>, shared: Arc<Shared>) -> Say {
    Box::new(move |p| {
        if let Err(e) = crate::notify::post(&ch, p, &console) {
            // The status line belongs to the main loop; leave it there for
            // the next tick to take, and never stop the board over a banner.
            *shared.trouble.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("notification: {e}"));
        }
    })
}

struct Worker {
    repo: PathBuf,
    title: String,
    shared: Arc<Shared>,
    say: Say,
    /// The thread's own connection, opened on the first pass with the
    /// preference ON. A board that never turns notifications on never opens
    /// one, which is what keeps a default-off feature free.
    client: Option<Box<dyn Transport + Send>>,
    differ: Differ,
    batch: Coalescer,
    /// The board as of the last look, kept rather than dropped (T-292): the
    /// words a post carries are resolved when the post is BUILT, which is a
    /// beat or twenty after the edge that put the event in the batch.
    board: Option<Board>,
    /// Whether the last pass ran armed, so that the preference going off is
    /// an edge and not a state re-applied four times a second.
    armed: bool,
    /// This connection owes one snapshot whether or not the daemon says
    /// anything: it is how a fresh one seeds the differ, and how a reopened
    /// one catches up on the changes it was not there for.
    owes_look: bool,
    last_dial: Option<Instant>,
}

impl Worker {
    fn new(repo: PathBuf, title: String, shared: Arc<Shared>, say: Say) -> Worker {
        Worker {
            repo,
            title,
            shared,
            say,
            client: None,
            differ: Differ::default(),
            batch: Coalescer::default(),
            board: None,
            armed: false,
            owes_look: false,
            last_dial: None,
        }
    }

    fn run(mut self, ctrl: &Receiver<Ctrl>) {
        loop {
            match ctrl.recv_timeout(BEAT) {
                Ok(Ctrl::Preview(s)) => (self.say)(&Post::sound_only(s)),
                Err(RecvTimeoutError::Timeout) => {}
                // The board is gone, and so is the thread. This is the off
                // switch D15 asked for: the process dying is what stops it.
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.pass();
        }
    }

    /// One beat: look if there is anything to look at, then say whatever the
    /// window is up on.
    fn pass(&mut self) {
        let prefs = self.shared.prefs();
        if !prefs.on {
            if self.armed {
                // Off is off. Whatever was held is not owed to anybody now,
                // and arming it again seeds afresh rather than saying
                // everything that happened while nobody was listening.
                self.stand_down();
            }
            return;
        }
        self.armed = true;
        let now = self.shared.now_ms();
        if let Some(board) = self.look() {
            for e in self.differ.scan(&board, prefs.done) {
                self.batch.offer(e, now);
            }
            self.board = Some(board);
        }
        // The ticket whose agent pane is on the terminal has nothing to say
        // to a person already reading it (T-292): the permission prompt IS
        // the pane and the finished turn IS its last line, so the banner
        // points at nothing and the chime is for something already watched.
        // Dropped from the batch rather than never offered, so a line still
        // waiting from before the attach goes too — and the differ keeps its
        // mark either way, so detaching does not then announce what was seen.
        // The stronger half of the focus rule, and the one row that takes
        // the sound with it — so it asks whether anybody is actually in
        // there, and an attach left sitting behind another window says its
        // piece like anything else (T-299).
        if !prefs.in_pane {
            self.ask_who_is_typing(now);
            if let Some(t) = self.shared.presence().watching(now) {
                self.batch.forget(t);
            }
        }
        // The words are resolved HERE, not at the edge (T-292): a turn's
        // closing record lands on the transcript around the moment the state
        // flips, and the batch has been held for up to a window since. The
        // lookup is asked about one ticket and only past that window check,
        // so a busy board reads no transcripts at all.
        let voice = Voice {
            board: &self.title,
            needs_you: prefs.sound_needs_you,
            done: prefs.sound_done,
            words: prefs.words,
        };
        let board = self.board.as_ref();
        let Some(mut post) =
            self.batch.due(now, &voice, &|t| board.and_then(|b| detail_for(b, t, prefs.words)))
        else {
            return;
        };
        // The focus rule (T-282), asking T-291's corrected question: the
        // banner goes only while somebody is LOOKING at the board — it is on
        // screen and the terminal has focus. The card already says it in the
        // one saturated colour, so a banner over it would be noise; a chime
        // beside it is still a cue. Attached to a pane neither holds, which
        // is the whole point.
        if !prefs.focused && self.shared.presence().looking(now) {
            post.hush();
        }
        if post.is_silent() {
            return;
        }
        (self.say)(&post);
    }

    /// Is the person still inside the pane they attached to? (T-299)
    ///
    /// The board cannot see them — focus reporting is off for the handover
    /// and their keys go to tmux — so tmux is asked, and its answer feeds
    /// `Presence` as the keystrokes that never reached us.
    ///
    /// Asked only when the answer can change what happens: there is an
    /// attach, and something is held ABOUT that ticket. Everything else
    /// pays nothing, which matters because this is a `list-clients` fork on
    /// the daemon's writer thread. A connection we do not have, or an answer
    /// that does not come, leaves the memory to age out on its own — and
    /// ageing out means speaking, which is the direction this feature falls
    /// in every time it does not know.
    fn ask_who_is_typing(&mut self, now: u64) {
        let Some(t) = self.shared.presence().attached() else { return };
        if !self.batch.holds(t) {
            return;
        }
        let Some(client) = self.client.as_deref_mut() else { return };
        let Ok(Response::FocusQuiet { quiet_ms }) = client.request(Command::FocusQuiet) else {
            return;
        };
        self.shared.presence().saw_pane_quiet(now, quiet_ms);
    }

    fn stand_down(&mut self) {
        self.differ.reset();
        self.batch.clear();
        self.board = None;
        self.armed = false;
        self.owes_look = false;
        // A board with nothing to say owes the daemon no subscription.
        self.client = None;
    }

    /// The board as the daemon has it, when the daemon says it moved — or
    /// when this connection still owes the differ its seed. `None` means
    /// nothing to scan this beat, which is almost every beat.
    fn look(&mut self) -> Option<Board> {
        if !self.dial() {
            return None;
        }
        let client = self.client.as_deref_mut()?;
        let mut dirty = false;
        while client.poll_event() {
            dirty = true;
        }
        if !dirty && !self.owes_look {
            return None;
        }
        // A dead connection reopens (and re-subscribes) inside `request`; a
        // failure just means the next dial tries again.
        let Ok(Response::Board { board, .. }) = client.request(Command::Snapshot) else {
            return None;
        };
        self.owes_look = false;
        Some(board)
    }

    /// Open or reopen the connection on its own cadence. False means there is
    /// nothing to talk to this beat.
    fn dial(&mut self) -> bool {
        if self.client.as_mut().is_some_and(|c| c.healthy()) {
            return true;
        }
        if self.last_dial.is_some_and(|t| t.elapsed() < DIAL_EVERY) {
            return false;
        }
        self.last_dial = Some(Instant::now());
        if self.client.is_none() {
            self.client = Client::connect_observer(&self.repo)
                .ok()
                .map(|c| Box::new(c) as Box<dyn Transport + Send>);
        }
        // A fresh or reopened connection has no event backlog to poll, so the
        // next look asks outright. The differ keeps what it knows across a
        // daemon blip — that memory is the board's, not the connection's.
        self.owes_look = true;
        self.client.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{
        Confidence, Reason, SessionKind, SessionRecord, SessionState, StopReason, Ticket,
    };

    /// A daemon that answers with whatever board the test last put in it, and
    /// reports an event once per board it was given.
    struct FakeDaemon {
        board: Arc<Mutex<Board>>,
        events: Receiver<()>,
        healthy: bool,
        /// What tmux would say about the client inside the attached pane
        /// (T-299). `None` is nobody there, which is also the default: a
        /// test that says nothing about the pane is a test with nobody in
        /// one.
        quiet: Arc<Mutex<Option<u64>>>,
    }

    impl Transport for FakeDaemon {
        fn request(&mut self, command: Command) -> anyhow::Result<Response> {
            if matches!(command, Command::FocusQuiet) {
                return Ok(Response::FocusQuiet {
                    quiet_ms: *self.quiet.lock().expect("the pane"),
                });
            }
            assert!(matches!(command, Command::Snapshot), "the thread asks for two things");
            Ok(Response::Board {
                board: self.board.lock().expect("the board").clone(),
                grace: Vec::new(),
                external: Vec::new(),
                resources: Default::default(),
                worktrees: Vec::new(),
                notices: Vec::new(),
                shell_env: Default::default(),
                git: Default::default(),
                pending: Vec::new(),
                automation: Default::default(),
                claude_md: Default::default(),
                claude_default_mode: None,
                status_top: false,
                team: Default::default(),
                terminals: Vec::new(),
            })
        }

        fn poll_event(&mut self) -> bool {
            self.events.try_recv().is_ok()
        }

        fn healthy(&mut self) -> bool {
            self.healthy
        }
    }

    /// A worker wired to a fake daemon and a recording poster: nothing here
    /// opens a socket, spawns a program or writes to a terminal.
    struct Rig {
        worker: Worker,
        shared: Arc<Shared>,
        board: Arc<Mutex<Board>>,
        moved: Sender<()>,
        said: Arc<Mutex<Vec<Post>>>,
        quiet: Arc<Mutex<Option<u64>>>,
    }

    fn ticket(n: u128) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": ulid::Ulid(n).to_string(),
            "short_key": format!("T-{n}"),
            "title": "t",
            "column": "TODO",
            "order": format!("{n}"),
            "created_at": "@0",
        }))
        .expect("a ticket from its required fields")
    }

    fn rig(prefs: NotifyPrefs) -> Rig {
        let mut b = Board::default();
        b.tickets.push(ticket(1));
        b.sessions.push(SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            SessionState::Running,
        ));
        let board = Arc::new(Mutex::new(b));
        let (moved, events) = channel();
        let shared = Arc::new(Shared::new(prefs));
        let said: Arc<Mutex<Vec<Post>>> = Default::default();
        let mut worker = Worker::new(
            PathBuf::from("/repo"),
            "board".into(),
            shared.clone(),
            Box::new({
                let said = said.clone();
                move |p: &Post| said.lock().expect("said").push(p.clone())
            }),
        );
        let quiet: Arc<Mutex<Option<u64>>> = Default::default();
        worker.client = Some(Box::new(FakeDaemon {
            board: board.clone(),
            events,
            healthy: true,
            quiet: quiet.clone(),
        }));
        // The seed: what the board already holds is not news. The fake daemon
        // never dials, so the worker is handed its connection above and owes
        // exactly this one look.
        worker.owes_look = true;
        let mut rig = Rig { worker, shared, board, moved, said, quiet };
        rig.beat();
        assert!(rig.said().is_empty(), "an opening board announces no backlog");
        rig
    }

    impl Rig {
        fn beat(&mut self) {
            self.worker.pass();
        }

        /// Somebody is inside the attached pane, and typed just now — what
        /// tmux reports for a conversation in progress (T-299).
        fn someone_is_typing(&self) {
            *self.quiet.lock().expect("the pane") = Some(0);
        }

        fn set_prefs(&self, p: NotifyPrefs) {
            *self.shared.prefs.lock().expect("prefs") = p;
        }

        /// Hand the worker a fresh connection, as `dial` would — without one
        /// being opened, which no unit test may do.
        fn reconnect(&mut self) {
            let (moved, events) = channel();
            self.moved = moved;
            self.worker.client = Some(Box::new(FakeDaemon {
                board: self.board.clone(),
                events,
                healthy: true,
                quiet: self.quiet.clone(),
            }));
            self.worker.owes_look = true;
        }

        /// Change the board and tell the worker the daemon said so.
        fn change(&mut self, f: impl FnOnce(&mut Board)) {
            f(&mut self.board.lock().expect("the board"));
            self.moved.send(()).expect("the worker's event channel");
            self.beat();
        }

        fn said(&self) -> Vec<Post> {
            self.said.lock().expect("said").clone()
        }

        fn took(&self) -> Option<Post> {
            self.said.lock().expect("said").pop()
        }

        /// Past the coalescing window: the clock is `started.elapsed()`, so
        /// ageing the start is how a test gets to the next thing it may say.
        fn past_the_window(&mut self) {
            let w = Duration::from_millis(mesimon_core::notify::WINDOW_MS + 1);
            let mut started = self.shared.started.lock().expect("the clock");
            *started = started.checked_sub(w).expect("a young clock");
        }
    }

    fn on() -> NotifyPrefs {
        NotifyPrefs {
            on: true,
            done: true,
            focused: false,
            in_pane: false,
            words: true,
            sound_needs_you: Sound::Glass,
            sound_done: Sound::Tink,
        }
    }

    fn blocked() -> SessionState {
        SessionState::RequiresAction { reason: Reason::Permission }
    }

    fn block(b: &mut Board) {
        if let Some(rec) = b.sessions.first_mut() {
            rec.state = blocked();
            rec.confidence = Confidence::High;
        }
    }

    /// The plain road: an agent starts needing you and the thread says so,
    /// with the board's own name on it.
    #[test]
    fn a_rising_edge_is_said_once() {
        let mut r = rig(on());
        r.change(block);
        let post = r.took().expect("a banner");
        assert_eq!(
            (post.subtitle.as_str(), post.body.as_str()),
            ("T-1 ∙ t", "needs you ∙ PERMISSION")
        );
        assert_eq!(post.title, "board");
        assert_eq!(post.sound, Sound::Glass);
        r.beat();
        assert!(r.said().is_empty(), "and nothing is repeated on the next beat");
    }

    /// T-291's bug, end to end: the terminal has focus and the board is not
    /// on screen. Before this the banner was swallowed and only the sound
    /// went out — while attached to a pane, which is the case the whole
    /// feature exists for.
    #[test]
    fn a_handover_does_not_count_as_looking_at_the_board() {
        let mut r = rig(on());
        r.shared.presence().saw_focus(true, 0);
        r.shared.presence().saw_board(false, None);
        r.change(block);
        let post = r.took().expect("a banner");
        assert_eq!(post.body, "needs you ∙ PERMISSION", "the banner goes out anyway");
    }

    /// …and with the board back on screen the focus rule applies again:
    /// the banner goes and the sound stays.
    #[test]
    fn looking_at_the_board_takes_the_banner_and_leaves_the_sound() {
        let mut r = rig(on());
        r.shared.presence().saw_focus(true, 0);
        r.change(block);
        let post = r.took().expect("a sound");
        assert_eq!(post.body, "", "no banner over the card that already says it");
        assert_eq!(post.sound, Sound::Glass, "the sound is not what focus takes");
    }

    /// A silenced sound and a suppressed banner leave nothing to say, and
    /// nothing is what is said — never an empty notification.
    #[test]
    fn a_suppressed_banner_with_no_sound_says_nothing() {
        let mut r = rig(NotifyPrefs { sound_needs_you: Sound::Off, ..on() });
        r.shared.presence().saw_focus(true, 0);
        r.change(block);
        assert!(r.said().is_empty());
    }

    /// The row that turns the suppression off.
    #[test]
    fn the_row_that_shows_the_banner_anyway() {
        let mut r = rig(NotifyPrefs { focused: true, ..on() });
        r.shared.presence().saw_focus(true, 0);
        r.change(block);
        assert_eq!(r.took().expect("a banner").body, "needs you ∙ PERMISSION");
    }

    /// The switch, both ways: off holds nothing and drops the connection,
    /// and arming it again seeds rather than announcing the backlog.
    #[test]
    fn off_drops_the_connection_and_arming_it_again_seeds_afresh() {
        let mut r = rig(on());
        r.set_prefs(NotifyPrefs { on: false, ..on() });
        r.change(block);
        assert!(r.said().is_empty(), "off is off");
        assert!(r.worker.client.is_none(), "and it owes the daemon no subscription");
        // Arming it again: a fresh connection, standing in for the one `dial`
        // would open (nothing in this module opens a socket). The board still
        // holds a blocked agent, and the first look after arming SEEDS.
        r.set_prefs(on());
        r.reconnect();
        r.beat();
        assert!(r.said().is_empty(), "the first look after arming seeds");
        // …and the next real edge still speaks.
        r.change(|b| b.tickets[0].woke_at = Some("@1000".into()));
        assert_eq!(r.took().expect("a banner").body, "needs you");
    }

    /// One beat says one thing however much arrived, and the window is
    /// rolling from the last thing SAID.
    #[test]
    fn a_busy_board_is_one_line_per_window() {
        let mut r = rig(on());
        r.change(block);
        assert_eq!(r.said().len(), 1);
        r.change(|b| b.tickets[0].woke_at = Some("@1000".into()));
        assert_eq!(r.said().len(), 1, "inside the window");
        r.past_the_window();
        r.beat();
        assert_eq!(r.said().len(), 2, "and the window being up is enough — no new edge");
    }

    /// A turn that finished is the quieter half, and the row can take it away
    /// without touching the other one.
    #[test]
    fn the_finished_half_has_its_own_row_and_its_own_sound() {
        let mut r = rig(on());
        r.change(|b| {
            if let Some(rec) = b.sessions.first_mut() {
                rec.state = SessionState::Idle { stop_reason: StopReason::EndTurn };
                rec.confidence = Confidence::High;
            }
        });
        let post = r.took().expect("a chime");
        assert_eq!(post.body, "finished a turn");
        assert_eq!(post.sound, Sound::Tink, "the quieter of the two");

        let mut r = rig(NotifyPrefs { done: false, ..on() });
        r.change(|b| {
            if let Some(rec) = b.sessions.first_mut() {
                rec.state = SessionState::Idle { stop_reason: StopReason::EndTurn };
                rec.confidence = Confidence::High;
            }
        });
        assert!(r.said().is_empty(), "the row took that half away");
        r.change(block);
        assert_eq!(r.took().expect("a banner").body, "needs you ∙ PERMISSION");
    }

    /// A beat the daemon said nothing about costs no round trip: the thread
    /// polls its event channel and stops there.
    #[test]
    fn a_quiet_daemon_is_never_asked() {
        let mut r = rig(on());
        // A refusing daemon would panic in `request`; the pass must not make
        // one at all.
        r.worker.client = Some(Box::new(Refuser));
        r.beat();
        r.beat();
    }

    struct Refuser;

    impl Transport for Refuser {
        fn request(&mut self, _: Command) -> anyhow::Result<Response> {
            panic!("a quiet daemon must not be asked")
        }
        fn poll_event(&mut self) -> bool {
            false
        }
    }

    /// T-292's own half, through the real worker: the ticket's TITLE reaches
    /// the banner, and the withholding row takes the agent's words back out
    /// without taking the ticket with them.
    #[test]
    fn the_banner_names_the_ticket_and_the_row_can_withhold_the_words() {
        let mut r = rig(on());
        r.change(|b| b.tickets[0].title = "Add auth to the API".into());
        r.change(|b| {
            b.tickets[0].raised = Some(mesimon_core::board::Raised {
                at: "@1000".into(),
                by: "agent:x".into(),
                reason: "pick an auth provider".into(),
            })
        });
        let post = r.took().expect("a banner");
        assert_eq!(post.subtitle, "T-1 ∙ Add auth to the API");
        assert_eq!(post.body, "needs you ∙ pick an auth provider");

        // The same edge with the words withheld.
        let mut r = rig(NotifyPrefs { words: false, ..on() });
        r.change(|b| b.tickets[0].title = "Add auth to the API".into());
        r.change(|b| {
            b.tickets[0].raised = Some(mesimon_core::board::Raised {
                at: "@1000".into(),
                by: "agent:x".into(),
                reason: "pick an auth provider".into(),
            })
        });
        let post = r.took().expect("a banner");
        assert_eq!(post.subtitle, "T-1 ∙ Add auth to the API", "the ticket is never withheld");
        assert_eq!(post.body, "needs you", "the agent's own sentence is");
    }

    /// T-292: inside a ticket's own agent pane, that ticket says nothing —
    /// banner AND sound, because the pane already showed you and there is
    /// nowhere to go look. The other nineteen agents are still invisible
    /// from in there, so they are not touched.
    #[test]
    fn the_watched_tickets_own_news_is_dropped_and_nobody_elses() {
        let mut r = rig(on());
        r.change(|b| b.tickets.push(ticket(2)));
        r.someone_is_typing();
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.change(|b| {
            block(b);
            b.tickets[1].woke_at = Some("@1000".into());
        });
        let post = r.took().expect("the other ticket still speaks");
        assert_eq!(post.subtitle, "T-2 ∙ t", "T-1 is the pane in front of them");
        assert!(r.said().is_empty(), "and it was one post, not two");

        // Alone, it is nothing at all: no banner and no sound, where the
        // focus rule would have left a chime.
        let mut r = rig(on());
        r.someone_is_typing();
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.change(block);
        assert!(r.said().is_empty());
        // Detaching says nothing about what was seen in there — the differ
        // marked it, so only the NEXT edge speaks.
        r.shared.presence().saw_board(true, None);
        r.past_the_window();
        r.beat();
        assert!(r.said().is_empty(), "what you watched happen is not announced on the way out");
    }

    /// A line already waiting when the user walks into the pane goes with
    /// the rest: the window is five seconds long, and `c` on a card that
    /// just lit up lands inside it.
    #[test]
    fn a_line_held_from_before_the_attach_goes_too() {
        let mut r = rig(on());
        r.change(|b| b.tickets.push(ticket(2)));
        // Something else spends the window, so the next edge is held.
        r.change(|b| b.tickets[1].woke_at = Some("@1000".into()));
        assert_eq!(r.took().expect("the first one").subtitle, "T-2 ∙ t");
        r.change(block);
        assert!(r.said().is_empty(), "held, inside the window");
        r.someone_is_typing();
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.past_the_window();
        r.beat();
        assert!(r.said().is_empty(), "and the wait ended inside the pane it was about");
    }

    /// The Settings row that takes the suppression away.
    #[test]
    fn the_row_that_says_it_inside_the_pane_anyway() {
        let mut r = rig(NotifyPrefs { in_pane: true, ..on() });
        r.someone_is_typing();
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.change(block);
        assert_eq!(r.took().expect("a banner").body, "needs you ∙ PERMISSION");
        // And the row spares the fork as well as the suppression: nothing
        // asks tmux about a pane whose answer cannot change anything.
        assert!(r.shared.presence().watching(r.shared.now_ms()).is_none());
    }

    /// T-299, end to end: attached to the ticket's own pane and then away
    /// from the terminal. The pane suppression is the one that takes the
    /// sound as well, so applying it to a screen nobody is facing left the
    /// board with no way at all to say that its agent had stopped.
    ///
    /// The terminal reported focus right up to the attach, which is what
    /// used to carry the silence through: off screen that report is no
    /// longer evidence, and tmux — the program actually reading the
    /// terminal — says the client has not typed.
    #[test]
    fn a_watched_pane_behind_another_window_speaks_after_all() {
        let mut r = rig(on());
        r.shared.presence().saw_focus(true, 0);
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.change(block);
        let post = r.took().expect("a banner");
        assert_eq!(post.body, "needs you ∙ PERMISSION", "the ticket says it anyway");
        assert_eq!(post.sound, Sound::Glass, "and the chime is what says go and look");
    }

    /// …and the same attach with somebody in it stays quiet, which is what
    /// T-292 was for. One rig, one difference: who tmux says is typing.
    #[test]
    fn the_same_pane_with_somebody_in_it_stays_quiet() {
        let mut r = rig(on());
        r.shared.presence().saw_focus(true, 0);
        r.shared.presence().saw_board(false, Some(ulid::Ulid(1)));
        r.someone_is_typing();
        r.change(block);
        assert!(r.said().is_empty(), "the pane in front of them said it already");
        // And when they walk away, the line that was held speaks — the ask
        // is every beat, so the answer going stale is enough on its own.
        *r.quiet.lock().expect("the pane") = Some(10 * mesimon_core::notify::KEY_PRESENCE_MS);
        r.past_the_window();
        r.change(|b| b.tickets[0].woke_at = Some("@1000".into()));
        assert_eq!(r.took().expect("a banner").subtitle, "T-1 ∙ t");
    }

    /// The loop itself: a control message is said AT ONCE (the theme
    /// picker's rule, that the cursor is the preview), and the sender going
    /// away is the off switch — the board process dying is what stops the
    /// thread, which is what keeps "a closed board is silent" true.
    #[test]
    fn the_loop_previews_at_once_and_stops_when_the_board_does() {
        let shared = Arc::new(Shared::new(NotifyPrefs::default()));
        let said: Arc<Mutex<Vec<Post>>> = Default::default();
        let worker = Worker::new(
            PathBuf::from("/repo"),
            "board".into(),
            shared,
            Box::new({
                let said = said.clone();
                move |p: &Post| said.lock().expect("said").push(p.clone())
            }),
        );
        let (tx, rx) = channel();
        let ran = std::thread::spawn(move || worker.run(&rx));
        tx.send(Ctrl::Preview(Sound::Purr)).expect("the thread is listening");
        // Well inside the beat: the preview must not wait one out.
        let deadline = Instant::now() + Duration::from_secs(5);
        while said.lock().expect("said").is_empty() {
            assert!(Instant::now() < deadline, "the preview never arrived");
            std::thread::yield_now();
        }
        assert_eq!(said.lock().expect("said").first().map(|p| p.sound), Some(Sound::Purr));
        assert!(
            said.lock().expect("said")[0].body.is_empty(),
            "a preview is a sound, not a banner"
        );
        drop(tx);
        ran.join().expect("the thread returns when the board goes");
    }

    /// Who is talking and about which board. The product name because the
    /// banner is posted under the helper's identity and not mesimon's, and
    /// the checkout's own directory name because two boards have to be told
    /// apart.
    #[test]
    fn the_title_names_the_product_and_the_board() {
        assert_eq!(title_of(Path::new("/home/a/code/simbly")), "mesimon - simbly");
        assert_eq!(title_of(Path::new("/home/a/code/mesimon")), "mesimon - mesimon");
        assert_eq!(title_of(Path::new("/")), "mesimon", "a root with no name is not a hyphen");
    }
    #[test]
    fn codex_notification_reads_its_preview_artifact_and_respects_words_setting() {
        let path =
            std::env::temp_dir().join(format!("msmn-notify-codex-{}.json", uuid::Uuid::new_v4()));
        let preview = mesimon_daemon::agents::AgentPreview {
            text: Some("Codex completed\nthe change".into()),
            activity: None,
            reply_key: Some(7),
        };
        std::fs::write(&path, serde_json::to_vec(&preview).unwrap()).unwrap();
        let rig = rig(on());
        let mut board = rig.board.lock().unwrap();
        board.sessions[0].kind = SessionKind::Codex;
        board.sessions[0].agent_preview_path = Some(path.display().to_string());
        board.sessions[0].transcript_path = Some("/unused/native/codex/history.jsonl".into());
        let ticket = board.tickets[0].id;
        assert_eq!(detail_for(&board, ticket, true).unwrap().said, "Codex completed the change");
        assert!(detail_for(&board, ticket, false).unwrap().said.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
