//! A terminal's LATE answer to our own query, caught before it can type.
//!
//! `detect::query_ground` asks the terminal its colours once, at startup,
//! with a 150 ms budget, through terminal-colorsaurus: `OSC 10` (the
//! foreground) and `OSC 11` (the background), each closed with `BEL`, then
//! `DA1`. A terminal that answers after the budget — iTerm2 with a
//! preferences sheet up, a background tab it deprioritises, a machine pegged
//! by `cargo build`, the first second after a `U` re-exec — has still
//! answered, and nothing can unsend it: the reply sits in the tty queue until
//! crossterm reads it as keystrokes, which after a `U` is only once the
//! daemon's handover and the first snapshot are done. crossterm drops the
//! `DA1` answer (`CSI ? 6 2 ; … c` is an internal event) but has no parser for
//! OSC, so each colour arrives one character at a time:
//!
//! ```text
//! ESC ] 1 0 ; r g b : d c a a / d c a b / d c a a BEL
//! ESC ] 1 1 ; r g b : 1 e 1 e / 1 e 1 e / 1 e 1 e BEL
//! ```
//!
//! which the board reads as alt+`]` (inert), `1` and `0` (the quick tag
//! cycle, groups 1 and 10 — silent mutations), `;` (inert), `r` (rename),
//! and then a title of `gb:dcaa/dcab/dcaa` (dogfood 2026-09-02 under the
//! key-remap sheet with `11;`, and T-953 on 2026-10-24 after every `U`, with
//! `10;`, the half this grammar did not yet know).
//!
//! The cure is not a longer budget — a reply can always be later than any
//! deadline, and the read blocks the frame — but this: a small grammar over
//! the key events, permanently armed, that recognises an OSC reply's PREFIX
//! (`alt+]`, one to five digits, `;` — nothing a hand types) and discards
//! everything up to the terminator (`BEL` as ctrl+g, or `ESC \` as alt+`\`).
//! Any OSC number, not only the two we ask: tmux asks colours of its own at
//! an attach (`OSC 10`/`11`, and palette entries as `OSC 4 ; n ;`), and a
//! reply nobody here is waiting on is still not a person. Each reply dropped
//! is counted (`dropped`) and, under `MESIMON_TTY_LOG`, written down. It sits
//! ahead of the keymap AND ahead of the text-field barrier, on the raw
//! crossterm event, because a text field strips Alt (`keys::to_key_text`) and
//! would miss the prefix exactly where a stray reply does the most damage.
//!
//! The prefix keys are HELD while they could still be a reply and replayed in
//! order if they turn out not to be, so a real alt+`]` costs one keystroke of
//! latency and nothing else. Once the prefix is complete the reply is proven,
//! so a key outside the body grammar (a colour, its `;`s) ends the swallow
//! and passes through alone: the body that came before it was never typing.
//! A bare `ESC` is not taken as the prefix's opener — holding a real Esc
//! until the next key would delay the menu it opens — so a reply crossterm
//! happens to split at its first byte still leaks; the tty hands the reply
//! over in one write, and that is the one case this does not cover.

use std::path::PathBuf;
use std::sync::OnceLock;

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// One raw key event, as crossterm reports it.
pub type RawKey = (KeyCode, KeyModifiers);

/// What to do with the event just fed.
#[derive(Debug, PartialEq, Eq)]
pub enum Feed {
    /// Part of a reply: nothing to handle.
    Swallowed,
    /// Real keys, in order — the ones held while they could still have been
    /// a prefix, then the one just fed.
    Pass(Vec<RawKey>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Idle,
    /// `alt+]`, then this many digits of the OSC number.
    Prefix(u8),
    /// Inside the payload, discarding until a terminator.
    Body,
    /// A bare `ESC` inside the body: the `\` of `ESC \` is next.
    BodyEsc,
}

/// The longest OSC number a reply opens with (`21337`, iTerm2's).
const MAX_DIGITS: u8 = 5;

#[derive(Debug, Default)]
pub struct ReplySwallow {
    state: State,
    held: Vec<RawKey>,
    /// The reply being dropped, as text (`]10;rgb:…`), for the log.
    reply: String,
    dropped: u64,
}

impl ReplySwallow {
    pub fn feed(&mut self, code: KeyCode, mods: KeyModifiers) -> Feed {
        let key = (code, mods);
        match (self.state, key) {
            (State::Idle, (KeyCode::Char(']'), KeyModifiers::ALT)) => {
                self.held.push(key);
                self.state = State::Prefix(0);
                Feed::Swallowed
            }
            (State::Prefix(n), (KeyCode::Char(c), KeyModifiers::NONE))
                if c.is_ascii_digit() && n < MAX_DIGITS =>
            {
                self.held.push(key);
                self.state = State::Prefix(n + 1);
                Feed::Swallowed
            }
            (State::Prefix(n), (KeyCode::Char(';'), KeyModifiers::NONE)) if n > 0 => {
                self.reply = std::iter::once(']')
                    .chain(self.held[1..].iter().map(|(c, _)| match c {
                        KeyCode::Char(c) => *c,
                        _ => '?',
                    }))
                    .chain([';'])
                    .collect();
                self.held.clear();
                self.dropped += 1;
                self.state = State::Body;
                Feed::Swallowed
            }
            (State::Idle | State::Prefix(_), _) => {
                self.state = State::Idle;
                let mut keys = std::mem::take(&mut self.held);
                keys.push(key);
                Feed::Pass(keys)
            }
            // `BEL` or `ESC \` in one event: the reply is over.
            (State::Body, (KeyCode::Char('g'), KeyModifiers::CONTROL))
            | (State::Body, (KeyCode::Char('\\'), KeyModifiers::ALT))
            | (State::BodyEsc, (KeyCode::Char('\\'), KeyModifiers::NONE)) => self.done(),
            (State::Body, (KeyCode::Esc, KeyModifiers::NONE)) => {
                self.state = State::BodyEsc;
                Feed::Swallowed
            }
            (State::Body, (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT))
                if body_char(c) =>
            {
                self.reply.push(c);
                Feed::Swallowed
            }
            (State::Body | State::BodyEsc, _) => {
                self.done();
                Feed::Pass(vec![key])
            }
        }
    }

    /// How many replies were dropped since the board started (the log
    /// numbers them).
    #[cfg(test)]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    fn done(&mut self) -> Feed {
        self.state = State::Idle;
        log(&format!("dropped reply {}: {}", self.dropped, std::mem::take(&mut self.reply)));
        Feed::Swallowed
    }
}

/// What a colour payload is spelled with: `rgb:1e1e/1e1e/1e1e`, `rgba:…`,
/// `#1e1e1e`, in either case (crossterm reports a capital as Shift), and the
/// `;` of a palette entry's `4;n;rgb:…`. Hex covers `a`–`f`; `r` and `g` are
/// the rest.
fn body_char(c: char) -> bool {
    c.is_ascii_hexdigit() || matches!(c, '/' | ':' | '#' | ';' | 'r' | 'g' | 'R' | 'G')
}

/// `MESIMON_TTY_LOG`: a file the board appends what it asks its terminal and
/// what it drops from the key stream to, one timed line each. A diagnosis
/// seam, set in `lib.rs::run` and nowhere else, so no test writes one.
static TTY_LOG: OnceLock<Option<PathBuf>> = OnceLock::new();

pub(crate) fn set_tty_log(path: Option<PathBuf>) {
    let _ = TTY_LOG.set(path);
}

pub(crate) fn log(line: &str) {
    let Some(Some(path)) = TTY_LOG.get() else { return };
    use std::io::Write;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{ms} {} {line}", std::process::id());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn plain(c: char) -> RawKey {
        (KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn alt(c: char) -> RawKey {
        (KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn bel() -> RawKey {
        (KeyCode::Char('g'), KeyModifiers::CONTROL)
    }

    /// Text as crossterm reports it: a capital is Shift.
    fn typed(s: &str) -> Vec<RawKey> {
        s.chars()
            .map(|c| {
                let m = if c.is_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
                (KeyCode::Char(c), m)
            })
            .collect()
    }

    /// One reply, `ESC ] <osc> <payload> <terminator>`, as crossterm hands
    /// it over.
    fn reply(body: &str, end: RawKey) -> Vec<RawKey> {
        let mut keys = vec![alt(']')];
        keys.extend(typed(body));
        keys.push(end);
        keys
    }

    /// What terminal-colorsaurus's query earns from iTerm2 when it comes
    /// late: the foreground, then the background, each closed with `BEL` as
    /// the query was (`DA1`'s answer never reaches the key stream).
    pub(crate) fn late_colour_reply() -> Vec<RawKey> {
        let mut keys = reply("10;rgb:dcaa/dcab/dcaa", bel());
        keys.extend(reply("11;rgb:53ec/53ec/53ec", bel()));
        keys
    }

    fn feed_all(s: &mut ReplySwallow, keys: &[RawKey]) -> Vec<Feed> {
        keys.iter().map(|&(c, m)| s.feed(c, m)).collect()
    }

    /// T-953: the foreground half of the reply is dropped like the
    /// background half, and each one counts.
    #[test]
    fn a_late_colour_reply_types_nothing_and_counts_both_halves() {
        let mut s = ReplySwallow::default();
        for f in feed_all(&mut s, &late_colour_reply()) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.dropped(), 2);
        assert_eq!(s.state, State::Idle);
        assert_eq!(s.feed(KeyCode::Char('r'), KeyModifiers::NONE), Feed::Pass(vec![plain('r')]));
    }

    /// Every OSC number a reply can open with, every spelling of a colour,
    /// and both terminators: `BEL`, and `ESC \` whole or split.
    #[test]
    fn every_osc_reply_and_every_terminator_is_dropped() {
        let bodies =
            ["11;rgb:1e1e/1e1e/1e1e", "10;#1E1E1E", "4;12;rgb:aa/bb/cc", "12;rgba:0/0/0/ff"];
        let ends =
            [vec![bel()], vec![alt('\\')], vec![(KeyCode::Esc, KeyModifiers::NONE), plain('\\')]];
        for body in bodies {
            for end in &ends {
                let mut s = ReplySwallow::default();
                let mut keys = vec![alt(']')];
                keys.extend(typed(body));
                keys.extend(end.iter().copied());
                for f in feed_all(&mut s, &keys) {
                    assert_eq!(f, Feed::Swallowed, "{body} {end:?}");
                }
                assert_eq!((s.state, s.dropped()), (State::Idle, 1), "{body} {end:?}");
            }
        }
    }

    /// A real alt+`]` is held for one keystroke and then replayed ahead of
    /// the key that proved it real, so nothing is lost or reordered.
    #[test]
    fn a_prefix_that_was_not_a_reply_is_replayed_in_order() {
        let mut s = ReplySwallow::default();
        assert_eq!(s.feed(KeyCode::Char(']'), KeyModifiers::ALT), Feed::Swallowed);
        assert_eq!(s.feed(KeyCode::Char('1'), KeyModifiers::NONE), Feed::Swallowed);
        assert_eq!(
            s.feed(KeyCode::Char('j'), KeyModifiers::NONE),
            Feed::Pass(vec![alt(']'), plain('1'), plain('j')])
        );
        assert_eq!(s.state, State::Idle);
        // An alt+`]` with no number, and one with more digits than any OSC.
        assert_eq!(s.feed(KeyCode::Char(']'), KeyModifiers::ALT), Feed::Swallowed);
        assert_eq!(
            s.feed(KeyCode::Char(';'), KeyModifiers::NONE),
            Feed::Pass(vec![alt(']'), plain(';')])
        );
        let mut keys = vec![alt(']')];
        keys.extend(typed("123456"));
        let fed = feed_all(&mut s, &keys);
        assert_eq!(fed.last(), Some(&Feed::Pass(keys.clone())));
        // And an ordinary key never waits at all.
        assert_eq!(s.feed(KeyCode::Char('k'), KeyModifiers::NONE), Feed::Pass(vec![plain('k')]));
        assert_eq!(s.dropped(), 0);
    }

    /// Once the prefix is complete the reply is proven: a key outside the
    /// body grammar passes through alone, and the body before it is dropped.
    #[test]
    fn a_stray_key_inside_the_body_passes_through_by_itself() {
        let mut s = ReplySwallow::default();
        let mut keys = vec![alt(']')];
        keys.extend(typed("11;rg"));
        for f in feed_all(&mut s, &keys) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.feed(KeyCode::Char('q'), KeyModifiers::NONE), Feed::Pass(vec![plain('q')]));
        assert_eq!((s.state, s.dropped()), (State::Idle, 1));
    }

    /// An Esc inside the body that is not followed by `\` ends the swallow;
    /// the key after it is real and passes through (the Esc itself was the
    /// reply's, and is dropped with the rest of the body).
    #[test]
    fn a_bare_esc_in_the_body_is_not_a_terminator() {
        let mut s = ReplySwallow::default();
        let mut keys = vec![alt(']')];
        keys.extend(typed("11;rgb:"));
        keys.push((KeyCode::Esc, KeyModifiers::NONE));
        for f in feed_all(&mut s, &keys) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.feed(KeyCode::Char('x'), KeyModifiers::NONE), Feed::Pass(vec![plain('x')]));
    }
}

/// Request a clipboard write from the terminal. OSC 52 has no acknowledgement;
/// callers must distinguish this request from a successful native copy.
#[cfg_attr(test, allow(dead_code))]
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    out.flush()
}

/// Base64, standard alphabet with padding — the ~20 lines that keep a
/// dependency out of the graph for one escape sequence. The workspace pins a
/// single major of everything and CI fails on a duplicate, so a crate is never
/// free here.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod copy_tests {
    use super::base64;

    /// RFC 4648's own vectors, plus the padding cases either side of them —
    /// a hand-rolled encoder is worth exactly as much as its test.
    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// The snippet is what actually gets copied, em dash and all: a multi-byte
    /// character must not fall off the end of a chunk.
    #[test]
    fn the_snippet_survives_the_encoder() {
        let text = mesimon_core::claudemd::SNIPPET;
        let encoded = base64(text.as_bytes());
        assert_eq!(encoded.len() % 4, 0, "a padded encoding is a multiple of four");
        // Decode it back by hand and compare: the only proof that matters.
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bits = Vec::new();
        for c in encoded.bytes().filter(|c| *c != b'=') {
            let v = ALPHABET.iter().position(|a| *a == c).expect("in the alphabet") as u32;
            bits.push(v);
        }
        let mut bytes = Vec::new();
        for quad in bits.chunks(4) {
            let mut n = 0u32;
            for (i, v) in quad.iter().enumerate() {
                n |= v << (18 - 6 * i);
            }
            for i in 0..quad.len() - 1 {
                bytes.push((n >> (16 - 8 * i)) as u8);
            }
        }
        assert_eq!(String::from_utf8(bytes).expect("utf8"), text);
    }
}
