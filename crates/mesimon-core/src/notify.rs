//! What the board says out loud, and when (T-282).
//!
//! The board is deliberately quiet — one saturated colour, no motion, no
//! channel out — and that works exactly while somebody is looking at it. This
//! is the missing outward edge: an OS banner and a sound when an agent needs
//! you, and when an agent finishes a turn. Off by default, both of them.
//!
//! D15 said never build a notification channel and ship `watch --json`
//! instead. The composable answer never shipped, so the quiet stayed total;
//! what survives of D15 here is its reasoning, as four constraints this file
//! keeps:
//!
//! - **default off** — nothing in this module is reached until a preference
//!   says so;
//! - **coalesced** — at most one notification per [`WINDOW_MS`], carrying an
//!   aggregate. Twenty agents finishing together are one line, not twenty;
//!   that multiplication is the whole reason D15 was written;
//! - **the client speaks, not the daemon** — which is why this file is pure
//!   and its two ladders live in the TUI;
//! - **quiet while you are looking** — [`Presence`], below, which since
//!   T-291 means the board is ON SCREEN and focused, not merely that the
//!   terminal has focus.
//!
//! Everything here is pure and time-injected, the way `attention.rs` is: the
//! caller passes `now_ms` and the platform half holds no policy.

use ulid::Ulid;

/// The coalescing window: one notification per 5 s, carrying everything that
/// arrived inside it (06 §, "coalesced to at most one per 5 s rolling window
/// carrying an aggregate"). It is the whole rate-limiting policy — there are
/// no quiet hours and no digest, because the notification the user actually
/// wants is "something changed, go look", and one of those is enough.
pub const WINDOW_MS: u64 = 5_000;

/// How long a keypress stands in for focus, on a terminal that never reports
/// it. See [`Presence`].
pub const KEY_PRESENCE_MS: u64 = 30_000;

/// The two things worth interrupting somebody for.
///
/// `NeedsYou` is every road to the saturated colour: an attention-set session
/// (`attention::is_attention`, ranks 0–8), a snooze that woke a ticket (T-74)
/// and an agent's raised hand (T-107) — the same three
/// `Board::needs_you_tickets` collects, so a banner and the `!N` chip can
/// never disagree about what needs you. `TurnDone` is a session reaching
/// `Idle{EndTurn}`, the state automove reads to move a card to REVIEW.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The louder one, and the one that sorts first in a mixed batch.
    NeedsYou,
    TurnDone,
}

/// One thing that just happened, offered to the coalescer.
///
/// `key` is the ticket's short key (`T-12`) because that is what the user
/// reads on the card. `why` is what the card would say beside it: the reason
/// word for an attention-set session (`attention::reason_word`, so the banner
/// and the card say the same word), or, for a raised hand, the AGENT'S OWN
/// SENTENCE (`Raised::reason`) — which is the whole point of T-107, since "I
/// finished the refactor" and "I cannot proceed until somebody picks an auth
/// provider" are the two things a banner exists to tell apart. A woken ticket
/// has nobody to quote, so it is empty. It is a `String` rather than the
/// `&'static str` a hint is, precisely because one of the three is not from a
/// fixed set — and being user-ish text is why it crosses `scrub_text` on the
/// way out of the process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: Kind,
    pub ticket: Ulid,
    pub key: String,
    pub why: String,
    /// Whether `why` is the AGENT'S own sentence rather than mesimon's word
    /// (T-292). The two are indistinguishable as strings and the reader who
    /// withholds an agent's words has to tell them apart: a reason word is
    /// ours, from a fixed set, and says nothing about the work; a raised
    /// hand's sentence is the agent talking. Only [`Event::raised`] sets it.
    pub quoted: bool,
}

impl Event {
    pub fn needs_you(ticket: Ulid, key: impl Into<String>, why: impl Into<String>) -> Event {
        Event { kind: Kind::NeedsYou, ticket, key: key.into(), why: why.into(), quoted: false }
    }

    /// A raised hand (T-107): needs-you, and `why` is the agent's own words.
    pub fn raised(ticket: Ulid, key: impl Into<String>, reason: impl Into<String>) -> Event {
        Event { quoted: true, ..Event::needs_you(ticket, key, reason) }
    }

    pub fn turn_done(ticket: Ulid, key: impl Into<String>) -> Event {
        Event { kind: Kind::TurnDone, ticket, key: key.into(), why: String::new(), quoted: false }
    }
}

/// A sound, by name.
///
/// Six, because a ring the Settings row cycles has to be walkable in a few
/// presses, and because the ring exists to make the two events
/// *distinguishable* — not to be a sound library. The names are macOS's own
/// (they are the filenames); on a freedesktop desktop they collapse to the
/// three events that theme actually ships, which is honest about what is
/// there rather than pretending six.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sound {
    Off,
    #[default]
    Glass,
    Ping,
    Tink,
    Purr,
    Submarine,
    Hero,
}

impl Sound {
    /// The ring, in the order a Settings row cycles it: bright first, then
    /// quieter, then off — so a user walking it hears the useful ones before
    /// they hear silence.
    pub const ALL: [Sound; 7] = [
        Sound::Glass,
        Sound::Ping,
        Sound::Tink,
        Sound::Purr,
        Sound::Submarine,
        Sound::Hero,
        Sound::Off,
    ];

    /// The one after this, wrapping — the Settings row's Enter.
    pub fn next(self) -> Sound {
        let i = Sound::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Sound::ALL[(i + 1) % Sound::ALL.len()]
    }

    /// The name, as the Settings row and `doctor` spell it.
    pub fn name(self) -> &'static str {
        match self {
            Sound::Off => "off",
            Sound::Glass => "Glass",
            Sound::Ping => "Ping",
            Sound::Tink => "Tink",
            Sound::Purr => "Purr",
            Sound::Submarine => "Submarine",
            Sound::Hero => "Hero",
        }
    }

    /// `prefs.json`'s spelling — the name itself. Round-trips through
    /// [`Sound::from_key`], which is case-insensitive because a hand edit is
    /// the file's other writer.
    pub fn key(self) -> &'static str {
        self.name()
    }

    pub fn from_key(s: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|v| v.key().eq_ignore_ascii_case(s))
    }

    pub fn is_off(self) -> bool {
        matches!(self, Sound::Off)
    }

    /// macOS ships these as files, under one directory, with these names.
    pub fn file_macos(self) -> Option<String> {
        match self {
            Sound::Off => None,
            _ => Some(format!("/System/Library/Sounds/{}.aiff", self.name())),
        }
    }

    /// The freedesktop sound-theme event this name collapses to. The theme
    /// has no fourteen-sound palette to map onto, so six names become three
    /// events, chosen by what the sound is FOR rather than what it sounds
    /// like: an alert, a completion, a nudge.
    pub fn event_freedesktop(self) -> Option<&'static str> {
        match self {
            Sound::Off => None,
            Sound::Glass | Sound::Ping => Some("message"),
            Sound::Hero | Sound::Submarine => Some("complete"),
            Sound::Tink | Sound::Purr => Some("bell"),
        }
    }
}

/// What one coalesced batch asks the platform to do.
///
/// Three fields since T-292, because the two the platform gives us were one
/// short: `title` is WHICH BOARD (a user with two of them has to be told),
/// `subtitle` is WHICH TICKET (`T-12 ∙ Add auth to the API`) and `body` is
/// WHAT HAPPENED. `terminal-notifier` and `osascript` each have all three;
/// the rungs that have one fold them with [`Post::folded`].
///
/// `body` empty means "no banner, just the sound" — which is what the focus
/// rule produces while the board's own terminal has focus, and what a
/// Settings row's sound preview produces. A subtitle never exists without a
/// body, which is why `body` stays the one discriminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    /// The batch includes an attention event. Presentation must not infer
    /// this from a customizable sound or from quoted notification words.
    pub needs_you: bool,
    pub title: String,
    /// The ticket this is about, named — empty for an aggregate, which has
    /// no one ticket to name, and for a sound-only post.
    pub subtitle: String,
    pub body: String,
    pub sound: Sound,
}

impl Post {
    /// A sound and nothing else — the Settings row's preview, and what the
    /// focus rule leaves of a batch while you are looking at the board.
    pub fn sound_only(sound: Sound) -> Post {
        Post {
            needs_you: false,
            title: String::new(),
            subtitle: String::new(),
            body: String::new(),
            sound,
        }
    }

    pub fn is_silent(&self) -> bool {
        self.body.is_empty() && self.sound.is_off()
    }

    /// Take the banner and leave the sound — what the focus rule does to a
    /// batch while somebody is looking at the board. The card is already
    /// saying it in the one saturated colour, so a banner over it would be
    /// noise; a chime beside it is still a cue.
    pub fn hush(&mut self) {
        self.title.clear();
        self.subtitle.clear();
        self.body.clear();
    }

    /// The subtitle and the body as one line, for a rung that has only one
    /// field (OSC 9, `notify-send`, a user's own program). Spelled here
    /// rather than in the platform half so the separator is decided once, in
    /// the module that owns mesimon's voice.
    pub fn folded(&self) -> String {
        if self.subtitle.is_empty() {
            return self.body.clone();
        }
        format!("{} ∙ {}", self.subtitle, self.body)
    }
}

/// What only the client can answer about one ticket, asked when the post is
/// BUILT rather than when the edge fired (T-292).
///
/// The freshness is the whole reason for the indirection: a turn's closing
/// record lands on the transcript around the moment the state flips, and the
/// coalescer already holds a batch for up to [`WINDOW_MS`] before it says
/// anything. Reading the reply at the edge would race the writer; reading it
/// here is five seconds later on a busy board and no later at all on a quiet
/// one, because the window rolls from the last thing said.
///
/// Both fields may be empty — no such ticket, no transcript, nothing said
/// yet, or the user asked for the words to be withheld — and the wording
/// falls back to what shipped before this existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detail {
    /// The ticket's own title.
    pub title: String,
    /// What its agent last said, already flattened to one line.
    pub said: String,
}

/// The voice the board speaks in, for one post.
///
/// A struct rather than four more arguments on [`Coalescer::due`]: the board
/// name and the two sounds were parameters already, and `words` is the fourth
/// thing a caller has to decide before a batch can be worded.
#[derive(Debug, Clone, Copy)]
pub struct Voice<'a> {
    /// Who is talking and about which board — `mesimon - simbly`, built by
    /// `notifier::title_of`. Two boards open at once is why the second half
    /// exists; the helper that posts the banner carrying its own identity
    /// rather than mesimon's is why the first does.
    pub board: &'a str,
    pub needs_you: Sound,
    pub done: Sound,
    /// May the agent's own words be quoted, or is the ticket all that is
    /// said? A banner lands on a lock screen; on a client's codebase the
    /// ticket may be sayable where the agent's sentence is not. mesimon's own
    /// reason word is not the agent's words and is never withheld.
    pub words: bool,
}

/// Holds what happened until the window is up, then says it once.
///
/// The window is *rolling from the last thing said*, not from the first thing
/// offered: a quiet board notifies immediately, and a busy one settles into
/// one line per [`WINDOW_MS`] no matter how much arrives.
#[derive(Debug, Default)]
pub struct Coalescer {
    held: Vec<Event>,
    said_at: Option<u64>,
}

impl Coalescer {
    /// Take one event. Same ticket and kind twice inside a window is one
    /// event: a session that flickers back into the attention set must not
    /// buy a second line in the aggregate.
    pub fn offer(&mut self, e: Event, _now: u64) {
        if self.held.iter().any(|h| h.kind == e.kind && h.ticket == e.ticket) {
            return;
        }
        self.held.push(e);
    }

    /// Whether anything is waiting to be said.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// Whether anything waiting is about THIS ticket. The notification
    /// thread asks before it forks tmux (T-299): the only thing that answer
    /// can change is whether this ticket's line is swallowed, so a board
    /// with nothing held about it pays nothing.
    pub fn holds(&self, ticket: Ulid) -> bool {
        self.held.iter().any(|e| e.ticket == ticket)
    }

    /// The batch, if the window is up.
    ///
    /// `look` answers what only the client knows about a ticket — its title,
    /// and what its agent last said (T-292). It is asked for ONE event and
    /// only past the window check, so a batch of several costs nothing and a
    /// held batch costs nothing until it is actually said: three titles do
    /// not fit a banner, so several keep the count shape they had before.
    pub fn due(
        &mut self,
        now: u64,
        v: &Voice<'_>,
        look: &dyn Fn(Ulid) -> Option<Detail>,
    ) -> Option<Post> {
        if self.held.is_empty() {
            return None;
        }
        if self.said_at.is_some_and(|t| now.saturating_sub(t) < WINDOW_MS) {
            return None;
        }
        self.said_at = Some(now);
        let held = std::mem::take(&mut self.held);
        let loudest = held.iter().map(|e| e.kind).min().unwrap_or(Kind::TurnDone);
        let sound = match loudest {
            Kind::NeedsYou => v.needs_you,
            Kind::TurnDone => v.done,
        };
        let (subtitle, body) = match held.as_slice() {
            [one] => {
                let d = look(one.ticket);
                (names(one, d.as_ref()), said(one, d.as_ref(), v.words))
            }
            many => (String::new(), body(many)),
        };
        Some(Post {
            needs_you: loudest == Kind::NeedsYou,
            title: v.board.to_string(),
            subtitle,
            body,
            sound,
        })
    }

    /// Drop everything held without saying it — the board went away, or the
    /// preference did.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Drop what is held about ONE ticket. The user walked into that
    /// ticket's own pane while the window was still running, so the line
    /// waiting to be said is about the screen they are now looking at
    /// (T-292). Called every beat rather than on the edge, because "is this
    /// ticket being watched" is a state and not an event — and because a
    /// batch offered in the same beat is caught by the same call.
    pub fn forget(&mut self, ticket: Ulid) {
        self.held.retain(|e| e.ticket != ticket);
    }
}

/// The words for a batch of SEVERAL, in mesimon's own voice: lower case, `∙`
/// between clauses, the ticket's key because that is what the card shows.
///
/// One of a KIND names the ticket; several count them and then name as many
/// as fit. Mixed leads with needs-you, because that is the half that cannot
/// proceed without the user. A batch of one never reaches here — it has a
/// subtitle to carry the ticket and room for what the agent said
/// ([`names`], [`said`]) — but a single needs-you beside a single done
/// still does, and keeps its keys inline.
fn body(held: &[Event]) -> String {
    let mut waiting: Vec<&Event> = held.iter().filter(|e| e.kind == Kind::NeedsYou).collect();
    let mut done: Vec<&Event> = held.iter().filter(|e| e.kind == Kind::TurnDone).collect();
    waiting.sort_by_key(|e| e.ticket);
    done.sort_by_key(|e| e.ticket);

    let mut parts: Vec<String> = Vec::new();
    match waiting.as_slice() {
        [] => {}
        [one] if one.why.is_empty() => parts.push(format!("{} needs you", one.key)),
        [one] => parts.push(format!("{} needs you ∙ {}", one.key, one.why)),
        many => parts.push(format!("{} agents need you ∙ {}", many.len(), keys(many))),
    }
    match done.as_slice() {
        [] => {}
        [one] => parts.push(format!("{} finished a turn", one.key)),
        many => parts.push(format!("{} agents finished ∙ {}", many.len(), keys(many))),
    }
    parts.join(" ∙ ")
}

/// WHICH TICKET, for the one-event batch: the key the card shows, and the
/// ticket's own title after it (T-292). The key alone where the board no
/// longer holds the ticket — it is the half that identifies, and the half
/// this can always spell.
fn names(e: &Event, d: Option<&Detail>) -> String {
    match d.map(|d| d.title.trim()).filter(|t| !t.is_empty()) {
        Some(title) => format!("{} ∙ {}", e.key, clip(title, TITLE_CHARS)),
        None => e.key.clone(),
    }
}

/// WHAT HAPPENED, for the one-event batch.
///
/// The verb comes first and always: `needs you` and `finished` are the two
/// moments, the subtitle already said which ticket, and a reader must not
/// have to identify the moment by its chime. Content is APPENDED to it — so
/// the sentence with nothing to append is the one that shipped before this,
/// and a withheld word is a shorter line rather than a different one.
fn said(e: &Event, d: Option<&Detail>, words: bool) -> String {
    // A reason word is mesimon's own, from a fixed set, and says nothing
    // about the work: it is not the agent's words and is never withheld.
    let quoted_ok = words || !e.quoted;
    match e.kind {
        Kind::NeedsYou if !e.why.is_empty() && quoted_ok => {
            format!("needs you ∙ {}", clip(&e.why, SAID_CHARS))
        }
        Kind::NeedsYou => "needs you".to_string(),
        Kind::TurnDone => match d.map(|d| d.said.trim()).filter(|s| !s.is_empty() && words) {
            Some(reply) => format!("finished ∙ {}", clip(reply, SAID_CHARS)),
            None => "finished a turn".to_string(),
        },
    }
}

/// A ticket title is capped at 2 KB (`board::TITLE_MAX_BYTES`) and a reply
/// line at nothing at all, so both are cut to something a banner can hold.
/// On a word boundary with an ellipsis, because unlike the byte cap the
/// platform half applies as a backstop, this one is read by a person.
const TITLE_CHARS: usize = 72;
const SAID_CHARS: usize = 120;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    // Back up to the last whole word, unless that would leave almost
    // nothing — half a long word still reads better than three characters.
    let cut = match head.rfind(char::is_whitespace) {
        Some(i) if i * 2 >= max => i,
        _ => head.len(),
    };
    format!("{}…", head[..cut].trim_end())
}

/// At most [`KEYS_NAMED`] keys, then a count of the rest. A notification body
/// is one line on somebody's screen, not a list.
const KEYS_NAMED: usize = 4;

fn keys(items: &[&Event]) -> String {
    let named: Vec<&str> = items.iter().take(KEYS_NAMED).map(|e| e.key.as_str()).collect();
    let rest = items.len().saturating_sub(named.len());
    if rest == 0 {
        named.join(" ")
    } else {
        format!("{} +{rest}", named.join(" "))
    }
}

/// What the board has already said, so that only a RISING edge speaks.
///
/// TUI-local and derived, the way the spoke marks are: nothing on the wire
/// knows what has been said out loud, so the only place this can live is
/// beside whoever is watching. Since T-291 that is the notification thread
/// rather than `App` — the board's own loop stops for the whole life of a
/// handover, which is precisely when a notification matters most.
///
/// Two guards keep it from crying wolf. `Idle{EndTurn}` counts only at High
/// or Medium confidence — the bar the attention queue itself uses — because
/// after a daemon restart every session is `Unknown` and the transcript tail
/// re-derives a finished turn at LOW for each one, which would be a burst of
/// "finished" for turns that ended long ago. And the first scan only SEEDS:
/// an opening board announces no backlog, and `U` restarts the process.
#[derive(Debug, Default)]
pub struct Differ {
    /// What was last said about each session.
    seen: std::collections::HashMap<uuid::Uuid, Mark>,
    /// The same for the two session-less producers: tickets a snooze woke
    /// (T-74) and tickets whose agent has its hand up (T-107). Two sets, not
    /// one, because a ticket can be lit by both and a hand going up on an
    /// already-woken ticket is news — it arrives with words.
    woke: std::collections::HashSet<Ulid>,
    raised: std::collections::HashSet<Ulid>,
    primed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    NeedsYou,
    Done,
    Other,
}

impl Differ {
    /// The edges in `next` worth interrupting somebody for.
    ///
    /// **needs-you** is all three roads to the saturated colour:
    /// `attention::attention_queue` (the rank 0–8 set at usable confidence),
    /// `Board::woke_tickets` (T-74) and `Board::raised_tickets` (T-107).
    /// Those are the three `Board::needs_you_tickets` collects, so a banner
    /// and the `!N` chip cannot disagree about what needs you. **A turn
    /// finished** is `Idle{EndTurn}`, the state automove reads to move a card
    /// to REVIEW, so the ding and the card move say the same thing.
    ///
    /// `done_too` is the preference: with it off a finished turn is still
    /// MARKED and simply not announced, so turning the row back on does not
    /// then say what it missed.
    pub fn scan(&mut self, next: &crate::board::Board, done_too: bool) -> Vec<Event> {
        use crate::board::{Confidence, SessionState, StopReason};

        let mut marks: std::collections::HashMap<uuid::Uuid, Mark> = Default::default();
        let mut events: Vec<Event> = Vec::new();
        for rec in crate::attention::attention_queue(next) {
            marks.insert(rec.id, Mark::NeedsYou);
            if self.seen.get(&rec.id) == Some(&Mark::NeedsYou) {
                continue;
            }
            // The reason the card prints, so the banner says the same word.
            let word = match &rec.state {
                SessionState::RequiresAction { reason } => crate::attention::reason_word(*reason),
                _ => "",
            };
            if let Some(t) = next.ticket(rec.ticket) {
                events.push(Event::needs_you(t.id, t.short_key.clone(), word));
            }
        }
        for rec in &next.sessions {
            if marks.contains_key(&rec.id) {
                continue;
            }
            let done = matches!(rec.state, SessionState::Idle { stop_reason: StopReason::EndTurn })
                && matches!(rec.confidence, Confidence::High | Confidence::Medium);
            marks.insert(rec.id, if done { Mark::Done } else { Mark::Other });
            if !done || !done_too {
                continue;
            }
            if self.seen.get(&rec.id) == Some(&Mark::Done) {
                continue;
            }
            if let Some(t) = next.ticket(rec.ticket) {
                events.push(Event::turn_done(t.id, t.short_key.clone()));
            }
        }
        let mut woke: std::collections::HashSet<Ulid> = Default::default();
        for t in next.woke_tickets() {
            woke.insert(t.id);
            if !self.woke.contains(&t.id) {
                // A snooze has nobody to quote, so no words beside it.
                events.push(Event::needs_you(t.id, t.short_key.clone(), ""));
            }
        }
        // A raised hand carries the agent's OWN sentence (T-107), and that
        // sentence is the reason to interrupt somebody: "I finished the
        // refactor" and "I cannot proceed until somebody picks an auth
        // provider" are exactly what a banner exists to tell apart.
        let mut raised: std::collections::HashSet<Ulid> = Default::default();
        for t in next.raised_tickets() {
            raised.insert(t.id);
            if !self.raised.contains(&t.id) {
                let why = t.raised.as_ref().map(|r| r.reason.clone()).unwrap_or_default();
                events.push(Event::raised(t.id, t.short_key.clone(), why));
            }
        }
        self.seen = marks;
        self.woke = woke;
        self.raised = raised;
        if !self.primed {
            self.primed = true;
            return Vec::new();
        }
        events
    }

    /// Forget everything said: the next scan seeds again. What the
    /// preference going OFF does, so that turning it back on says what
    /// happens next rather than everything that happened while nobody was
    /// listening.
    pub fn reset(&mut self) {
        *self = Differ::default();
    }
}

/// Is somebody looking at the board?
///
/// **Two questions, and both have to answer yes** (T-291). The board must be
/// what the terminal is SHOWING, and the terminal must have focus. The
/// second alone was the original rule and it was wrong in exactly the case
/// the feature exists for: attached to an agent's pane, heads-down in one
/// conversation, the terminal is focused and the board is nowhere on screen
/// — so every banner was suppressed while nine other agents finished behind
/// it. A handover (`!`, `^g`, a focus attach) is the main loop saying
/// [`saw_board`](Presence::saw_board)`(false)`.
///
/// Focus itself has two sources, in order. A terminal that reports it
/// (DECSET 1004, crossterm's `FocusGained`/`FocusLost`) is simply believed.
/// A terminal that never reports one falls back to **keystroke presence** —
/// a key inside [`KEY_PRESENCE_MS`] means the user is here — which is the
/// gate Claude Code itself uses.
///
/// The fallback direction is the point: with no evidence at all this answers
/// "not focused", so the banner fires. Silence is the failure that would make
/// the feature look broken, and it is the one this cannot fall into — which
/// is also why [`watching`](Presence::watching), the stronger suppression,
/// asks both questions rather than one (T-299).
#[derive(Debug)]
pub struct Presence {
    focus: Option<bool>,
    last_key: Option<u64>,
    /// Is the board what the terminal shows? True until a handover says
    /// otherwise — a board that has never given its terminal away is on it.
    on_screen: bool,
    /// The ticket whose AGENT PANE is on the terminal instead (T-292). Set
    /// only for an attach to that ticket's claude — see [`watching`].
    ///
    /// [`watching`]: Presence::watching
    watching: Option<Ulid>,
    /// When the person inside that pane last typed, on this clock — tmux's
    /// answer, and the only source of presence there is while the board is
    /// off screen (T-299). Held as a moment rather than as the silence tmux
    /// reported, so it ages between asks like every other keystroke here.
    /// Cleared by the board coming back, and by an answer that says nobody
    /// is there.
    pane_key: Option<u64>,
}

impl Default for Presence {
    fn default() -> Self {
        Presence { focus: None, last_key: None, on_screen: true, watching: None, pane_key: None }
    }
}

impl Presence {
    /// The terminal spoke. From here on it is the only source consulted.
    pub fn saw_focus(&mut self, focused: bool, _now: u64) {
        self.focus = Some(focused);
    }

    pub fn saw_key(&mut self, now: u64) {
        self.last_key = Some(now);
    }

    /// What tmux says about the person inside the attached pane: how long
    /// the client reading that terminal has been silent (T-299).
    ///
    /// This is [`saw_key`](Presence::saw_key) for the keystrokes that never
    /// reach us. Everything about a handover conspires to hide the user —
    /// focus reporting is off for the duration and the keys go to tmux —
    /// and tmux is the one program that can still see them, so its
    /// `client_activity` is asked for rather than guessed at.
    ///
    /// `None` is every way of not knowing (nobody attached, no such session,
    /// tmux unable to say) and it CLEARS the memory rather than keeping the
    /// last answer: no evidence reads as away, here as everywhere else.
    ///
    /// An answer outside the window is stored as absence rather than as an
    /// old moment. Subtracting a long silence from a young monotonic clock
    /// saturates at zero, and zero is a keypress at start-up — so a board
    /// thirty seconds old would read "quiet for an hour" as "typing now".
    pub fn saw_pane_quiet(&mut self, now: u64, quiet_ms: Option<u64>) {
        self.pane_key =
            quiet_ms.filter(|ms| *ms < KEY_PRESENCE_MS).map(|ms| now.saturating_sub(ms));
    }

    /// The board took the terminal back, or gave it away — and, when it gave
    /// it away to an agent's pane, whose ticket that pane belongs to.
    ///
    /// Set around every handover: while `on_screen` is false nothing on the
    /// terminal is ours, and a keystroke reaches tmux or the editor rather
    /// than us — so neither source below is evidence that anybody is looking
    /// at the BOARD. `watching` is normalised away when the board is back,
    /// because the two always move together and one of them is derived.
    pub fn saw_board(&mut self, on_screen: bool, watching: Option<Ulid>) {
        self.on_screen = on_screen;
        self.watching = if on_screen { None } else { watching };
        // Whoever was typing in the pane is not evidence about the board:
        // the three move together, and a stale answer left behind would
        // stand in for focus on a terminal that reports none.
        if on_screen {
            self.pane_key = None;
        }
    }

    /// The ticket whose pane took the terminal, whether or not anybody is
    /// still in front of it — [`watching`](Presence::watching) before the
    /// question of presence is put. The notification thread asks this to
    /// decide whether tmux is worth a fork, and nothing else should.
    pub fn attached(&self) -> Option<Ulid> {
        self.watching
    }

    pub fn on_screen(&self) -> bool {
        self.on_screen
    }

    /// The ticket whose claude the user is attached to AND looking at, if
    /// they are.
    ///
    /// This is [`looking`](Presence::looking) one level finer, and it is a
    /// STRONGER suppression: looking at the board takes the banner and
    /// leaves the sound, because a card is small and a chime says go look;
    /// being inside the agent's own pane takes both, because the thing that
    /// happened is the thing on the screen — the permission prompt IS the
    /// pane, the finished turn IS the last thing printed in it. There is
    /// nothing left to point at.
    ///
    /// **Which is why it asks whether anybody is in there** (T-299). This
    /// was the attach and nothing else, so a pane left open behind a browser
    /// silenced its own ticket completely — no banner, no chime — for as
    /// long as the user stayed away, in the case the feature exists for.
    /// [`attached`](Presence::attached) is that bare fact;
    /// [`focused`](Presence::focused) is the person, answered off screen by
    /// [`saw_pane_quiet`](Presence::saw_pane_quiet) — tmux's account of the
    /// keystrokes that never reach us. Short of a person the ordinary rule
    /// applies, and since the board is off screen wherever this can be
    /// `Some`, that rule is a banner AND a sound.
    ///
    /// Only an attach to that ticket's CLAUDE counts. A shell on the same
    /// ticket, the `!` terminal in its worktree and a `^g` editor all show
    /// the user's own words, not the agent's turn, so an agent's news there
    /// is news. A ticket holds one claude (2026-09-02), which is why one
    /// ticket id says all of this.
    pub fn watching(&self, now: u64) -> Option<Ulid> {
        self.watching.filter(|_| self.focused(now))
    }

    /// Whether the terminal ever answered — `doctor` says so, because "why
    /// did I get a banner while looking at the board" has exactly two
    /// answers and this is one of them.
    pub fn reports_focus(&self) -> bool {
        self.focus.is_some()
    }

    /// Does the terminal have focus? Half the question — [`looking`] is the
    /// one the banner rule asks.
    ///
    /// [`looking`]: Presence::looking
    pub fn focused(&self, now: u64) -> bool {
        // A focus report is believed only while it can still be REFUTED
        // (T-299). Reporting is off for the whole of a handover and the
        // terminal is tmux's, so a `true` from the moment before an attach
        // would otherwise stand for as long as the user stayed in the pane
        // — which is exactly how an agent went quiet for somebody who had
        // walked away.
        if self.on_screen {
            if let Some(f) = self.focus {
                return f;
            }
        }
        // Keystroke presence, from both keyboards: ours while the board has
        // the terminal, and tmux's account of the pane's while it does not.
        // The later of the two, because either one is a person.
        let last = self.last_key.max(self.pane_key);
        last.is_some_and(|t| now.saturating_sub(t) < KEY_PRESENCE_MS)
    }

    /// Both halves: the board is on screen AND the terminal has focus. This
    /// is what decides whether a batch keeps its banner.
    pub fn looking(&self, now: u64) -> bool {
        self.on_screen && self.focused(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: u128) -> Ulid {
        Ulid(n)
    }

    /// The voice every test speaks in unless it is testing the voice.
    fn voice(words: bool) -> Voice<'static> {
        Voice { board: "mesimon", needs_you: Sound::Glass, done: Sound::Tink, words }
    }

    /// A board that knows nothing about any ticket: the shape a lookup takes
    /// when there is no transcript and no title, which is what every test
    /// written before T-292 was implicitly asserting against.
    fn blind(_: Ulid) -> Option<Detail> {
        None
    }

    fn post(c: &mut Coalescer, now: u64) -> Option<Post> {
        c.due(now, &voice(true), &blind)
    }

    /// A lookup over a stub map — the seam T-292 exists for: the words are
    /// resolved when the post is built, and a test decides what is there to
    /// find without a transcript, a board or a thread.
    fn knowing<'a>(
        rows: &'a [(Ulid, &'static str, &'static str)],
    ) -> impl Fn(Ulid) -> Option<Detail> + 'a {
        move |id| {
            rows.iter().find(|(t, _, _)| *t == id).map(|(_, title, said)| Detail {
                title: (*title).to_string(),
                said: (*said).to_string(),
            })
        }
    }

    #[test]
    fn a_quiet_board_says_it_at_once() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        let p = post(&mut c, 0).expect("the first one waits for nothing");
        // The key is in the subtitle now (T-292); with no lookup to answer,
        // that is all the subtitle can say.
        assert_eq!(p.subtitle, "T-1");
        assert_eq!(p.body, "needs you ∙ PERMISSION");
        assert_eq!(p.sound, Sound::Glass);
        assert_eq!(p.title, "mesimon");
    }

    #[test]
    fn a_busy_board_says_it_once_per_window() {
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        assert!(post(&mut c, 0).is_some());
        c.offer(Event::turn_done(t(2), "T-2"), 100);
        assert!(post(&mut c, 100).is_none(), "inside the window");
        assert!(post(&mut c, WINDOW_MS - 1).is_none(), "still inside it");
        let p = post(&mut c, WINDOW_MS).expect("the window is up");
        assert_eq!((p.subtitle.as_str(), p.body.as_str()), ("T-2", "finished a turn"));
    }

    #[test]
    fn twenty_agents_are_one_line() {
        let mut c = Coalescer::default();
        for i in 1..=20u128 {
            c.offer(Event::turn_done(t(i), format!("T-{i}")), 0);
        }
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "20 agents finished ∙ T-1 T-2 T-3 T-4 +16");
        assert_eq!(p.subtitle, "", "three titles do not fit a banner, so none does");
        assert!(post(&mut c, WINDOW_MS).is_none(), "and nothing is left over");
    }

    #[test]
    fn the_same_ticket_twice_is_one_event() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        c.offer(Event::needs_you(t(1), "T-1", "QUESTION"), 10);
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "needs you ∙ PERMISSION", "the first word stands");
    }

    /// T-292's whole point: one event names its ticket in the subtitle and
    /// quotes its agent in the body.
    #[test]
    fn one_event_names_its_ticket_and_quotes_its_agent() {
        let rows = [(t(1), "Add auth to the API", "Tests pass; the migration is on main.")];
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        let p = c.due(0, &voice(true), &knowing(&rows)).expect("a post");
        assert_eq!(p.title, "mesimon", "which board");
        assert_eq!(p.subtitle, "T-1 ∙ Add auth to the API", "which ticket");
        assert_eq!(p.body, "finished ∙ Tests pass; the migration is on main.", "what happened");
        // The rungs with one field get the same three facts in one line.
        assert_eq!(
            p.folded(),
            "T-1 ∙ Add auth to the API ∙ finished ∙ Tests pass; the migration is on main."
        );

        // A needs-you takes mesimon's own reason word, not the transcript.
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        let p = c.due(0, &voice(true), &knowing(&rows)).expect("a post");
        assert_eq!(p.subtitle, "T-1 ∙ Add auth to the API");
        assert_eq!(p.body, "needs you ∙ PERMISSION");
    }

    /// The verb survives everything: with nothing to append, the line is the
    /// one that shipped before the lookup existed.
    #[test]
    fn nothing_to_quote_says_what_it_always_said() {
        let rows = [(t(1), "Add auth to the API", "")];
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        let p = c.due(0, &voice(true), &knowing(&rows)).expect("a post");
        assert_eq!(p.subtitle, "T-1 ∙ Add auth to the API");
        assert_eq!(p.body, "finished a turn");
    }

    /// The withholding row (T-292): the ticket is still named, the agent is
    /// not quoted, and mesimon's own reason word is not the agent's words.
    #[test]
    fn withholding_the_words_keeps_the_ticket() {
        let rows = [(t(1), "Add auth to the API", "Tests pass; the migration is on main.")];
        let say = |e: Event| {
            let mut c = Coalescer::default();
            c.offer(e, 0);
            let p = c.due(0, &voice(false), &knowing(&rows)).expect("a post");
            (p.subtitle, p.body)
        };
        assert_eq!(
            say(Event::turn_done(t(1), "T-1")),
            ("T-1 ∙ Add auth to the API".into(), "finished a turn".into()),
            "the reply is the agent's own words"
        );
        assert_eq!(
            say(Event::raised(t(1), "T-1", "pick an auth provider")),
            ("T-1 ∙ Add auth to the API".into(), "needs you".into()),
            "so is a raised hand's sentence"
        );
        assert_eq!(
            say(Event::needs_you(t(1), "T-1", "PERMISSION")),
            ("T-1 ∙ Add auth to the API".into(), "needs you ∙ PERMISSION".into()),
            "but a reason word is ours, from a fixed set, and stays"
        );
    }

    /// A batch of several costs no lookup at all: there is no one ticket to
    /// name, so nothing is asked about one.
    #[test]
    fn an_aggregate_asks_nothing_of_the_lookup() {
        let asked = std::cell::Cell::new(0usize);
        let count = |_: Ulid| {
            asked.set(asked.get() + 1);
            None
        };
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        c.offer(Event::turn_done(t(2), "T-2"), 0);
        let p = c.due(0, &voice(true), &count).expect("a post");
        assert_eq!(p.body, "2 agents finished ∙ T-1 T-2");
        assert_eq!(p.subtitle, "");
        assert_eq!(asked.get(), 0, "nobody was asked about a ticket nobody names");

        // And a held batch is not asked either — the window check comes first.
        c.offer(Event::turn_done(t(3), "T-3"), 0);
        assert!(c.due(10, &voice(true), &count).is_none());
        assert_eq!(asked.get(), 0, "a batch not yet said reads no transcript");
        assert!(c.due(WINDOW_MS, &voice(true), &count).is_some());
        assert_eq!(asked.get(), 1, "and exactly one when it finally is");
    }

    /// A title is capped at 2 KB on disk and a reply at nothing at all, so
    /// both are cut where a person will read them — on a word, with an
    /// ellipsis, not mid-syllable.
    #[test]
    fn a_long_title_and_a_long_reply_are_clipped() {
        let long_title = "Add authentication to the public API and to every internal one \
                          besides it, then write it up";
        let long_reply = "The refactor is done and every test passes, including the three that \
                          were failing on main before I started, which turned out to be a \
                          fixture problem rather than anything I touched";
        let rows = [(t(1), long_title, long_reply)];
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        let p = c.due(0, &voice(true), &knowing(&rows)).expect("a post");
        for (field, max) in [(&p.subtitle, TITLE_CHARS), (&p.body, SAID_CHARS)] {
            assert!(field.ends_with('…'), "{field}");
            // Key/verb plus separator, then the clip itself.
            assert!(field.chars().count() <= max + 16, "{field}");
            assert!(!field.contains("  "), "cut on a word, not inside one: {field}");
        }
        // Short enough is left exactly alone.
        assert_eq!(clip("Add auth", TITLE_CHARS), "Add auth");
        // A single word longer than the budget still gets cut, not dropped.
        let one_word = "x".repeat(SAID_CHARS + 20);
        assert_eq!(clip(&one_word, SAID_CHARS).chars().count(), SAID_CHARS + 1);
    }

    /// A ticket the board no longer holds still says which key it was: the
    /// key is the half that identifies and the half this can always spell.
    #[test]
    fn a_ticket_the_lookup_cannot_find_keeps_its_key() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        let p = c.due(0, &voice(true), &blind).expect("a post");
        assert_eq!(p.subtitle, "T-1");
        assert_eq!(p.folded(), "T-1 ∙ needs you ∙ PERMISSION");
    }

    #[test]
    fn a_mixed_batch_leads_with_needs_you_and_takes_its_sound() {
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(2), "T-2"), 0);
        c.offer(Event::needs_you(t(1), "T-1", "PLAN"), 0);
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "T-1 needs you ∙ PLAN ∙ T-2 finished a turn");
        assert_eq!(p.subtitle, "", "two tickets, so neither is the subject");
        assert_eq!(p.sound, Sound::Glass, "the louder half names the sound");
    }

    #[test]
    fn several_of_each_are_counted() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        c.offer(Event::needs_you(t(2), "T-2", "TRUST"), 0);
        c.offer(Event::turn_done(t(3), "T-3"), 0);
        c.offer(Event::turn_done(t(4), "T-4"), 0);
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "2 agents need you ∙ T-1 T-2 ∙ 2 agents finished ∙ T-3 T-4");
    }

    #[test]
    fn a_woken_ticket_has_no_reason_word() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(9), "T-9", ""), 0);
        assert_eq!(post(&mut c, 0).expect("a post").body, "needs you");
    }

    #[test]
    fn nothing_held_says_nothing() {
        let mut c = Coalescer::default();
        assert!(post(&mut c, 0).is_none());
        assert!(!c.holding());
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        assert!(c.holding());
        c.clear();
        assert!(!c.holding());
        assert!(post(&mut c, 0).is_none());
    }

    #[test]
    fn the_sound_ring_walks_and_round_trips() {
        assert_eq!(Sound::default(), Sound::Glass);
        let mut s = Sound::Glass;
        for _ in 0..Sound::ALL.len() {
            assert_eq!(Sound::from_key(s.key()), Some(s));
            assert_eq!(Sound::from_key(&s.key().to_lowercase()), Some(s));
            s = s.next();
        }
        assert_eq!(s, Sound::Glass, "the ring closes");
        assert_eq!(Sound::from_key("nonesuch"), None);
        assert_eq!(Sound::Off.next(), Sound::Glass, "off is a rung, not a stop");
    }

    #[test]
    fn every_sound_but_off_names_a_file_and_an_event() {
        for s in Sound::ALL {
            if s.is_off() {
                assert_eq!(s.file_macos(), None);
                assert_eq!(s.event_freedesktop(), None);
                continue;
            }
            let f = s.file_macos().expect("a file");
            assert!(f.starts_with("/System/Library/Sounds/"), "{f}");
            assert!(f.ends_with(".aiff"), "{f}");
            let e = s.event_freedesktop().expect("an event");
            assert!(["message", "complete", "bell"].contains(&e), "{e} is not in the theme");
        }
    }

    #[test]
    fn a_terminal_that_reports_focus_is_believed() {
        let mut p = Presence::default();
        assert!(!p.reports_focus());
        p.saw_key(0);
        assert!(p.focused(0), "a keypress stands in until the terminal speaks");
        p.saw_focus(false, 1);
        assert!(p.reports_focus());
        assert!(!p.focused(1), "and once it has spoken, it is the only source");
        p.saw_key(2);
        assert!(!p.focused(2), "a keypress does not override it");
        p.saw_focus(true, 3);
        assert!(p.focused(3));
    }

    #[test]
    fn a_silent_terminal_falls_back_to_the_keyboard_and_then_to_away() {
        let p = Presence::default();
        assert!(!p.focused(0), "no evidence at all reads as away, never as silence");
        let mut p = Presence::default();
        p.saw_key(1_000);
        assert!(p.focused(1_000 + KEY_PRESENCE_MS - 1));
        assert!(!p.focused(1_000 + KEY_PRESENCE_MS), "a stale keypress is not presence");
    }

    /// T-291, the bug the whole feature existed for: attached to an agent's
    /// pane the terminal is FOCUSED and the board is nowhere on screen, and
    /// the old rule read that as "you are looking at it" and swallowed every
    /// banner.
    #[test]
    fn a_board_off_screen_is_not_being_looked_at_however_focused_the_terminal_is() {
        let mut p = Presence::default();
        assert!(p.on_screen(), "a board that has never handed its terminal over is on it");
        p.saw_focus(true, 0);
        assert!(p.looking(0));
        p.saw_board(false, None);
        assert!(!p.looking(0), "the board is not what the terminal is showing");
        // And the report itself stops counting there (T-299): reporting is
        // off for the duration, so `true` cannot be refuted and must not be
        // believed. Presence off screen is keystrokes and nothing else.
        assert!(!p.focused(KEY_PRESENCE_MS), "a report nobody can contradict is not evidence");
        // Nor does a keypress into the pane count: it never reaches us, and
        // the last one before the handover must not stand in for presence.
        p.saw_key(0);
        assert!(!p.looking(0));
        p.saw_board(true, None);
        assert!(p.looking(0), "and the return puts it back");
        assert!(p.focused(KEY_PRESENCE_MS), "the report is believed again on the board");
    }

    /// T-292: one level finer than `looking`. Inside a ticket's own agent
    /// pane, that ticket's news is on the screen already — and only that
    /// ticket's, because the other nineteen agents are still invisible.
    #[test]
    fn the_watched_ticket_is_the_one_whose_pane_is_on_the_terminal() {
        let mut p = Presence::default();
        p.saw_focus(true, 0);
        assert_eq!(p.watching(0), None, "a board on screen is watching no pane");
        p.saw_board(false, Some(t(5)));
        assert_eq!(p.attached(), Some(t(5)), "the attach is a fact of its own");
        // Somebody is in there — tmux says they typed a moment ago.
        p.saw_pane_quiet(0, Some(0));
        assert_eq!(p.watching(0), Some(t(5)));
        assert!(!p.looking(0), "and it is still not the board");
        // The return clears it: the two always move together, and leaving one
        // set would silence a ticket for the rest of the session.
        p.saw_board(true, None);
        assert_eq!(p.watching(0), None);
        // A handover that is not an agent's pane — `!`, `^g`, the gate —
        // watches nothing, so nothing is silenced.
        p.saw_board(false, None);
        assert_eq!(p.watching(0), None);
        // And a caller that names one while the board is BACK is normalised:
        // the invariant lives here rather than at every call site.
        p.saw_board(true, Some(t(5)));
        assert_eq!(p.watching(0), None);
    }

    /// T-299, the dogfooded bug: attached to a ticket's pane and then away
    /// from the terminal. The board cannot see that happen — reporting is
    /// off and the keys go to tmux — so tmux is asked, and the answer is
    /// what separates "heads-down in the conversation" from "left it open
    /// behind a browser".
    #[test]
    fn tmux_says_whether_anybody_is_still_in_the_watched_pane() {
        let mut p = Presence::default();
        // Attached, and focused right up to the handover — which is the
        // state that used to silence the ticket for the whole attach.
        p.saw_focus(true, 0);
        p.saw_board(false, Some(t(5)));
        p.saw_pane_quiet(0, Some(2_000));
        assert_eq!(p.watching(0), Some(t(5)), "typing at the agent two seconds ago");
        // Still nothing but that same answer, a window later: it ages.
        assert_eq!(p.watching(KEY_PRESENCE_MS), None, "and an answer ages like a keypress");
        // Walked away: tmux reports a long silence, which is stored as
        // absence rather than as an old moment.
        p.saw_pane_quiet(0, Some(10 * KEY_PRESENCE_MS));
        assert_eq!(p.watching(0), None, "behind another window, the ticket speaks");
        // Back at the keyboard.
        p.saw_pane_quiet(0, Some(0));
        assert_eq!(p.watching(0), Some(t(5)), "coming back is coming back");
        // Nobody attached at all, or tmux could not say: no evidence reads
        // as away, which is the direction that keeps silence from being the
        // failure mode.
        p.saw_pane_quiet(0, None);
        assert_eq!(p.watching(0), None);
    }

    /// The answer belongs to the handover it was given in. Coming back to
    /// the board drops it, so a keystroke into a pane cannot stand in for
    /// focus afterwards on a terminal that reports none.
    #[test]
    fn the_pane_answer_does_not_outlive_the_attach() {
        let mut p = Presence::default();
        p.saw_board(false, Some(t(5)));
        p.saw_pane_quiet(0, Some(0));
        assert!(p.focused(0), "somebody is at the terminal, in the pane");
        p.saw_board(true, None);
        assert!(!p.focused(0), "and the board has no evidence of its own yet");
    }

    /// A young board and a long silence: the subtraction saturates at zero,
    /// and zero is a keypress at start-up. Storing absence instead is what
    /// keeps "quiet for an hour" from reading as "typing now".
    #[test]
    fn a_long_silence_on_a_young_clock_is_absence_not_a_keypress() {
        let mut p = Presence::default();
        p.saw_board(false, Some(t(5)));
        p.saw_pane_quiet(5_000, Some(3_600_000));
        assert_eq!(p.watching(5_000), None);
    }

    #[test]
    fn forgetting_one_ticket_leaves_the_rest_of_the_batch() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        c.offer(Event::turn_done(t(2), "T-2"), 0);
        c.forget(t(1));
        assert_eq!(post(&mut c, 0).expect("a post").body, "finished a turn");
        // The whole batch going leaves nothing to say, and nothing is said.
        c.offer(Event::turn_done(t(3), "T-3"), 0);
        c.forget(t(3));
        assert!(!c.holding());
        assert!(post(&mut c, WINDOW_MS).is_none());
        c.forget(t(9));
        assert!(!c.holding(), "forgetting what was never held is not an error");
    }

    #[test]
    fn a_sound_only_post_carries_no_words() {
        let p = Post::sound_only(Sound::Tink);
        assert!(p.body.is_empty());
        assert!(!p.is_silent());
        assert!(Post::sound_only(Sound::Off).is_silent());
    }

    #[test]
    fn attention_presentation_follows_events_not_words_or_sound() {
        let voice = Voice { board: "board", needs_you: Sound::Off, done: Sound::Off, words: true };
        let detail = |_: Ulid| Some(Detail { title: "needs you".into(), said: "needs you".into() });
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(1), "T-1"), 0);
        assert!(!c.due(0, &voice, &detail).unwrap().needs_you);
        c.offer(Event::needs_you(t(1), "T-1", ""), WINDOW_MS);
        c.offer(Event::turn_done(t(2), "T-2"), WINDOW_MS);
        let mixed = c.due(WINDOW_MS, &voice, &detail).unwrap();
        assert!(mixed.needs_you, "attention wins in a mixed batch, even with sound off");
        assert!(mixed.sound.is_off());
    }

    #[test]
    fn hushing_takes_the_banner_and_leaves_the_sound() {
        let mut p = Post {
            needs_you: false,
            title: "board".into(),
            subtitle: "T-1 ∙ a ticket".into(),
            body: "needs you".into(),
            sound: Sound::Glass,
        };
        p.hush();
        assert!(p.title.is_empty());
        assert!(p.body.is_empty());
        assert_eq!(p.sound, Sound::Glass, "the sound is not what the focus rule takes");
        assert!(!p.is_silent());
    }

    // ---- the differ (moved off `App` by T-291) --------------------------

    use crate::board::{
        Board, Confidence, Reason, SessionKind, SessionRecord, SessionState, StopReason, Ticket,
    };

    fn ticket(n: u128) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "id": Ulid(n).to_string(),
            "short_key": format!("T-{n}"),
            "title": "t",
            "column": "TODO",
            "order": format!("{n}"),
            "created_at": "@0",
        }))
        .expect("a ticket from its required fields")
    }

    fn claude(n: u128, state: SessionState, confidence: Confidence) -> SessionRecord {
        let mut s = SessionRecord::new(
            uuid::Uuid::from_u128(n),
            SessionKind::Claude,
            Ulid(n),
            vec!["claude".into()],
            "/repo".into(),
            state,
        );
        s.confidence = confidence;
        s
    }

    /// A board with three tickets and one claude on the first, already
    /// seeded: the next scan is a real edge.
    fn seeded(state: SessionState) -> (Differ, Board) {
        let mut b = Board::default();
        for n in 1..=3 {
            b.tickets.push(ticket(n));
        }
        b.sessions.push(claude(1, state, Confidence::High));
        let mut d = Differ::default();
        assert!(d.scan(&b, true).is_empty(), "an opening board announces no backlog");
        (d, b)
    }

    /// Move the one session and look again.
    fn moved(b: &Board, state: SessionState, confidence: Confidence) -> Board {
        let mut b = b.clone();
        if let Some(rec) = b.sessions.first_mut() {
            rec.state = state;
            rec.confidence = confidence;
        }
        b
    }

    fn blocked() -> SessionState {
        SessionState::RequiresAction { reason: Reason::Permission }
    }

    fn finished() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }

    /// The first scan SEEDS. A board that opens with a blocked agent on it
    /// must not raise a banner for something that happened while it was
    /// closed — and `U` restarts the process, so a reload is that case.
    #[test]
    fn an_opening_board_announces_no_backlog() {
        let (mut d, b) = seeded(blocked());
        assert!(d.scan(&b, true).is_empty(), "and it stays seeded");
    }

    #[test]
    fn an_agent_that_starts_needing_you_is_announced_once() {
        let (mut d, b) = seeded(SessionState::Running);
        let b = moved(&b, blocked(), Confidence::High);
        let events = d.scan(&b, true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], Event::needs_you(Ulid(1), "T-1", "PERMISSION"));
        // Still blocked is not news again — the ticket already needs you.
        let b =
            moved(&b, SessionState::RequiresAction { reason: Reason::Question }, Confidence::High);
        assert!(d.scan(&b, true).is_empty());
    }

    #[test]
    fn a_finished_turn_is_announced_and_the_row_turns_it_off() {
        let (mut d, b) = seeded(SessionState::Running);
        let done = moved(&b, finished(), Confidence::High);
        assert_eq!(d.scan(&done, true), vec![Event::turn_done(Ulid(1), "T-1")]);

        // With the row off the turn is MARKED and not announced, so turning
        // it back on does not then say what it missed.
        let (mut d, b) = seeded(SessionState::Running);
        let done = moved(&b, finished(), Confidence::High);
        assert!(d.scan(&done, false).is_empty(), "the row took that half away");
        assert!(d.scan(&done, true).is_empty(), "and the same finished turn is not news later");
        // …and the other half still speaks.
        let plan =
            moved(&b, SessionState::RequiresAction { reason: Reason::Plan }, Confidence::High);
        assert_eq!(d.scan(&plan, false).len(), 1, "only the finished half was turned off");
    }

    /// After a daemon restart every session is `Unknown` and the transcript
    /// tail re-derives a finished turn at LOW for each one. Those turns ended
    /// long ago; announcing them would be a burst of stale chimes.
    #[test]
    fn a_low_confidence_finish_is_not_news() {
        let (mut d, b) = seeded(SessionState::Unknown { reason: Default::default() });
        let low = moved(&b, finished(), Confidence::Low);
        assert!(d.scan(&low, true).is_empty());
        // The next real turn still lands.
        let running = moved(&b, SessionState::Running, Confidence::High);
        assert!(d.scan(&running, true).is_empty());
        let high = moved(&b, finished(), Confidence::High);
        assert_eq!(d.scan(&high, true), vec![Event::turn_done(Ulid(1), "T-1")]);
    }

    /// T-74's session-less half: a snooze that woke a ticket lit, which has
    /// no reason word to quote.
    #[test]
    fn a_woken_ticket_is_announced_once() {
        let (mut d, mut b) = seeded(SessionState::Running);
        b.tickets[0].woke_at = Some("@1000".into());
        assert_eq!(d.scan(&b, true), vec![Event::needs_you(Ulid(1), "T-1", "")]);
        assert!(d.scan(&b, true).is_empty(), "still woken is not woken again");
    }

    /// T-107's producer: an agent that asked for a person at the end of a
    /// turn. Its own sentence rides the banner, because that sentence is the
    /// whole reason to interrupt somebody rather than let them find the card.
    #[test]
    fn a_raised_hand_is_announced_with_the_agents_own_words() {
        let raise = |why: &str| {
            Some(crate::board::Raised {
                at: "@1000".into(),
                by: "agent:x".into(),
                reason: why.into(),
            })
        };
        let (mut d, mut b) = seeded(SessionState::Running);
        b.tickets[0].raised = raise("cannot proceed until somebody picks an auth provider");
        assert_eq!(
            d.scan(&b, true),
            vec![Event::raised(
                Ulid(1),
                "T-1",
                "cannot proceed until somebody picks an auth provider"
            )],
            "the agent's own sentence, marked as quoted"
        );
        assert!(d.scan(&b, true).is_empty(), "still up is not up again");
        // Lowered and raised again IS news: the words may be different.
        b.tickets[0].raised = None;
        assert!(d.scan(&b, true).is_empty(), "lowering says nothing");
        b.tickets[0].raised = raise("the migration needs a decision");
        assert_eq!(
            d.scan(&b, true),
            vec![Event::raised(Ulid(1), "T-1", "the migration needs a decision")]
        );
    }

    /// Every road to the saturated colour is a road to a notification: what
    /// the header counts and what the board says out loud are one set.
    #[test]
    fn every_road_to_needs_you_is_a_road_to_a_banner() {
        let (mut d, b) = seeded(SessionState::Running);
        let mut b = moved(&b, blocked(), Confidence::High);
        b.tickets[1].woke_at = Some("@1000".into());
        b.tickets[2].raised = Some(crate::board::Raised {
            at: "@1000".into(),
            by: "agent:x".into(),
            reason: "which provider".into(),
        });
        assert_eq!(b.needs_you_count(), 3, "the header's own number");
        let mut c = Coalescer::default();
        for e in d.scan(&b, true) {
            c.offer(e, 0);
        }
        assert_eq!(post(&mut c, 0).expect("one line").body, "3 agents need you ∙ T-1 T-2 T-3");
    }

    /// Arming the preference again seeds afresh rather than saying
    /// everything that happened while nobody was listening.
    #[test]
    fn a_reset_seeds_afresh() {
        let (mut d, b) = seeded(SessionState::Running);
        let b = moved(&b, blocked(), Confidence::High);
        d.reset();
        assert!(d.scan(&b, true).is_empty(), "the first look after arming seeds");
        assert!(d.scan(&b, true).is_empty());
    }
}
