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
//! - **quiet while you are looking** — [`Presence`], below.
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
}

impl Event {
    pub fn needs_you(ticket: Ulid, key: impl Into<String>, why: impl Into<String>) -> Event {
        Event { kind: Kind::NeedsYou, ticket, key: key.into(), why: why.into() }
    }

    pub fn turn_done(ticket: Ulid, key: impl Into<String>) -> Event {
        Event { kind: Kind::TurnDone, ticket, key: key.into(), why: String::new() }
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
/// `body` empty means "no banner, just the sound" — which is what the focus
/// rule produces while the board's own terminal has focus, and what a
/// Settings row's sound preview produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub title: String,
    pub body: String,
    pub sound: Sound,
}

impl Post {
    /// A sound and nothing else — the Settings row's preview, and what the
    /// focus rule leaves of a batch while you are looking at the board.
    pub fn sound_only(sound: Sound) -> Post {
        Post { title: String::new(), body: String::new(), sound }
    }

    pub fn is_silent(&self) -> bool {
        self.body.is_empty() && self.sound.is_off()
    }
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

    /// The batch, if the window is up. `title` names the board — a user with
    /// two of them has to be told which one is talking.
    pub fn due(&mut self, now: u64, title: &str, needs_you: Sound, done: Sound) -> Option<Post> {
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
            Kind::NeedsYou => needs_you,
            Kind::TurnDone => done,
        };
        Some(Post { title: title.to_string(), body: body(&held), sound })
    }

    /// Drop everything held without saying it — the board went away, or the
    /// preference did.
    pub fn clear(&mut self) {
        self.held.clear();
    }
}

/// The words, in mesimon's own voice: lower case, `∙` between clauses, the
/// ticket's key because that is what the card shows.
///
/// One of a kind names the ticket; several count them and then name as many
/// as fit. Mixed leads with needs-you, because that is the half that cannot
/// proceed without the user.
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

/// Is somebody looking at the board?
///
/// Two sources, in that order. A terminal that reports focus (DECSET 1004,
/// crossterm's `FocusGained`/`FocusLost`) is simply believed. A terminal that
/// never reports one falls back to **keystroke presence** — a key inside
/// [`KEY_PRESENCE_MS`] means the user is here — which is the gate Claude Code
/// itself uses.
///
/// The fallback direction is the point: with no evidence at all this answers
/// "not focused", so the banner fires. Silence is the failure that would make
/// the feature look broken, and it is the one this cannot fall into.
#[derive(Debug, Default)]
pub struct Presence {
    focus: Option<bool>,
    last_key: Option<u64>,
}

impl Presence {
    /// The terminal spoke. From here on it is the only source consulted.
    pub fn saw_focus(&mut self, focused: bool, _now: u64) {
        self.focus = Some(focused);
    }

    pub fn saw_key(&mut self, now: u64) {
        self.last_key = Some(now);
    }

    /// Whether the terminal ever answered — `doctor` says so, because "why
    /// did I get a banner while looking at the board" has exactly two
    /// answers and this is one of them.
    pub fn reports_focus(&self) -> bool {
        self.focus.is_some()
    }

    pub fn focused(&self, now: u64) -> bool {
        match self.focus {
            Some(f) => f,
            None => self.last_key.is_some_and(|t| now.saturating_sub(t) < KEY_PRESENCE_MS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: u128) -> Ulid {
        Ulid(n)
    }

    fn post(c: &mut Coalescer, now: u64) -> Option<Post> {
        c.due(now, "mesimon", Sound::Glass, Sound::Tink)
    }

    #[test]
    fn a_quiet_board_says_it_at_once() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        let p = post(&mut c, 0).expect("the first one waits for nothing");
        assert_eq!(p.body, "T-1 needs you ∙ PERMISSION");
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
        assert_eq!(p.body, "T-2 finished a turn");
    }

    #[test]
    fn twenty_agents_are_one_line() {
        let mut c = Coalescer::default();
        for i in 1..=20u128 {
            c.offer(Event::turn_done(t(i), format!("T-{i}")), 0);
        }
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "20 agents finished ∙ T-1 T-2 T-3 T-4 +16");
        assert!(post(&mut c, WINDOW_MS).is_none(), "and nothing is left over");
    }

    #[test]
    fn the_same_ticket_twice_is_one_event() {
        let mut c = Coalescer::default();
        c.offer(Event::needs_you(t(1), "T-1", "PERMISSION"), 0);
        c.offer(Event::needs_you(t(1), "T-1", "QUESTION"), 10);
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "T-1 needs you ∙ PERMISSION", "the first word stands");
    }

    #[test]
    fn a_mixed_batch_leads_with_needs_you_and_takes_its_sound() {
        let mut c = Coalescer::default();
        c.offer(Event::turn_done(t(2), "T-2"), 0);
        c.offer(Event::needs_you(t(1), "T-1", "PLAN"), 0);
        let p = post(&mut c, 0).expect("one post");
        assert_eq!(p.body, "T-1 needs you ∙ PLAN ∙ T-2 finished a turn");
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
        assert_eq!(post(&mut c, 0).expect("a post").body, "T-9 needs you");
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

    #[test]
    fn a_sound_only_post_carries_no_words() {
        let p = Post::sound_only(Sound::Tink);
        assert!(p.body.is_empty());
        assert!(!p.is_silent());
        assert!(Post::sound_only(Sound::Off).is_silent());
    }
}
