//! A terminal's LATE answer to our own query, caught before it can type.
//!
//! `detect::query_flavor` asks the terminal its background (`OSC 11`) with a
//! 150 ms budget, at startup and every 3 s from the watch. A terminal that
//! answers after the budget — iTerm2 with a preferences sheet up, a
//! background tab it deprioritises, a machine pegged by `cargo build` — has
//! still answered, and nothing can unsend it: the reply sits in the tty queue
//! until crossterm reads it as keystrokes. crossterm drops the `DA1` half
//! (`CSI ? 1 ; 2 c` is an internal event) but has no parser for OSC, so the
//! colour half arrives one character at a time:
//!
//! ```text
//! ESC ] 1 1 ; r g b : 1 e 1 e / 1 e 1 e / 1 e 1 e ESC \
//! ```
//!
//! which the board reads as alt+`]` (inert), `1` `1` (quick-tag, twice — a
//! silent mutation), `;` (inert), `r` (rename), and then a title of
//! `gb:1e1e/1e1e/1e1e\` (dogfood 2026-09-02, under iTerm2's key-remap sheet).
//!
//! The cure is not a longer budget — a reply can always be later than any
//! deadline, and the read blocks the frame — but this: a small grammar over
//! the key events, permanently armed, that recognises the reply's PREFIX
//! (`alt+]` `1` `1` `;` — nothing a hand types) and discards everything up to
//! the terminator (`ESC \` as alt+`\`, or `BEL` as ctrl+g). It sits ahead of
//! the keymap AND ahead of the text-field barrier, on the raw crossterm event,
//! because a text field strips Alt (`keys::to_key_text`) and would miss the
//! prefix exactly where a stray reply does the most damage.
//!
//! The prefix keys are HELD while they could still be a reply and replayed in
//! order if they turn out not to be, so a real alt+`]` costs one keystroke of
//! latency and nothing else. Once the prefix is complete the reply is proven,
//! so a key outside the body grammar ends the swallow and passes through
//! alone: the body that came before it was never typing. A bare `ESC` is not
//! taken as the prefix's opener — holding a real Esc until the next key would
//! delay the menu it opens — so a reply crossterm happens to split at its
//! first byte still leaks; the tty hands the reply over in one write, and that
//! is the one case this does not cover.

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
    /// The prefix so far: `alt+]`, then `1`, then `1 1` (3 means `;` is next).
    Prefix(u8),
    /// Inside the payload, discarding until a terminator.
    Body,
    /// A bare `ESC` inside the body: the `\` of `ESC \` is next.
    BodyEsc,
}

const PREFIX: [RawKey; 4] = [
    (KeyCode::Char(']'), KeyModifiers::ALT),
    (KeyCode::Char('1'), KeyModifiers::NONE),
    (KeyCode::Char('1'), KeyModifiers::NONE),
    (KeyCode::Char(';'), KeyModifiers::NONE),
];

#[derive(Debug, Default)]
pub struct ReplySwallow {
    state: State,
    held: Vec<RawKey>,
}

impl ReplySwallow {
    pub fn feed(&mut self, code: KeyCode, mods: KeyModifiers) -> Feed {
        let key = (code, mods);
        match self.state {
            State::Idle | State::Prefix(_) => {
                let n = match self.state {
                    State::Prefix(n) => n as usize,
                    _ => 0,
                };
                if key == PREFIX[n] {
                    if n + 1 == PREFIX.len() {
                        self.held.clear();
                        self.state = State::Body;
                    } else {
                        self.held.push(key);
                        self.state = State::Prefix(n as u8 + 1);
                    }
                    Feed::Swallowed
                } else {
                    self.state = State::Idle;
                    let mut keys = std::mem::take(&mut self.held);
                    keys.push(key);
                    Feed::Pass(keys)
                }
            }
            State::Body => match key {
                // `ESC \` in one event, or `BEL`: the reply is over.
                (KeyCode::Char('\\'), KeyModifiers::ALT) => self.done(),
                (KeyCode::Char('g'), KeyModifiers::CONTROL) => self.done(),
                (KeyCode::Esc, KeyModifiers::NONE) => {
                    self.state = State::BodyEsc;
                    Feed::Swallowed
                }
                (KeyCode::Char(c), KeyModifiers::NONE) if body_char(c) => Feed::Swallowed,
                _ => {
                    self.state = State::Idle;
                    Feed::Pass(vec![key])
                }
            },
            State::BodyEsc => match key {
                (KeyCode::Char('\\'), KeyModifiers::NONE) => self.done(),
                _ => {
                    self.state = State::Idle;
                    Feed::Pass(vec![key])
                }
            },
        }
    }

    fn done(&mut self) -> Feed {
        self.state = State::Idle;
        Feed::Swallowed
    }
}

/// What a colour payload is spelled with: `rgb:1e1e/1e1e/1e1e`, `rgba:…`,
/// `#1e1e1e`, in either case. Hex covers `a`–`f`; `r` and `g` are the rest.
fn body_char(c: char) -> bool {
    c.is_ascii_hexdigit() || matches!(c, '/' | ':' | '#' | 'r' | 'g' | 'R' | 'G')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(c: char) -> RawKey {
        (KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn alt(c: char) -> RawKey {
        (KeyCode::Char(c), KeyModifiers::ALT)
    }

    /// iTerm2's reply, exactly as crossterm hands it over.
    fn iterm_reply() -> Vec<RawKey> {
        let mut keys = vec![alt(']')];
        keys.extend("11;rgb:1e1e/1e1e/1e1e".chars().map(plain));
        keys.push(alt('\\'));
        keys
    }

    fn feed_all(s: &mut ReplySwallow, keys: &[RawKey]) -> Vec<Feed> {
        keys.iter().map(|&(c, m)| s.feed(c, m)).collect()
    }

    #[test]
    fn a_whole_reply_types_nothing_and_leaves_the_machine_idle() {
        let mut s = ReplySwallow::default();
        for f in feed_all(&mut s, &iterm_reply()) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.state, State::Idle);
        assert_eq!(s.feed(KeyCode::Char('r'), KeyModifiers::NONE), Feed::Pass(vec![plain('r')]));
    }

    /// Some terminals end with BEL (crossterm: ctrl+g), some with a split
    /// `ESC` `\`; both close the reply.
    #[test]
    fn every_terminator_closes_the_reply() {
        for tail in [
            vec![(KeyCode::Char('g'), KeyModifiers::CONTROL)],
            vec![(KeyCode::Esc, KeyModifiers::NONE), plain('\\')],
        ] {
            let mut s = ReplySwallow::default();
            let mut keys = vec![alt(']')];
            keys.extend("11;#1E1E1E".chars().map(plain));
            keys.extend(tail);
            for f in feed_all(&mut s, &keys) {
                assert_eq!(f, Feed::Swallowed);
            }
            assert_eq!(s.state, State::Idle);
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
        // And an ordinary key never waits at all.
        assert_eq!(s.feed(KeyCode::Char('k'), KeyModifiers::NONE), Feed::Pass(vec![plain('k')]));
    }

    /// Once the prefix is complete the reply is proven: a key outside the
    /// body grammar passes through alone, and the body before it is dropped.
    #[test]
    fn a_stray_key_inside_the_body_passes_through_by_itself() {
        let mut s = ReplySwallow::default();
        let mut keys = vec![alt(']')];
        keys.extend("11;rg".chars().map(plain));
        for f in feed_all(&mut s, &keys) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.feed(KeyCode::Char('q'), KeyModifiers::NONE), Feed::Pass(vec![plain('q')]));
        assert_eq!(s.state, State::Idle);
    }

    /// An Esc inside the body that is not followed by `\` ends the swallow;
    /// the key after it is real and passes through (the Esc itself was the
    /// reply's, and is dropped with the rest of the body).
    #[test]
    fn a_bare_esc_in_the_body_is_not_a_terminator() {
        let mut s = ReplySwallow::default();
        let mut keys = vec![alt(']')];
        keys.extend("11;rgb:".chars().map(plain));
        keys.push((KeyCode::Esc, KeyModifiers::NONE));
        for f in feed_all(&mut s, &keys) {
            assert_eq!(f, Feed::Swallowed);
        }
        assert_eq!(s.feed(KeyCode::Char('x'), KeyModifiers::NONE), Feed::Pass(vec![plain('x')]));
    }
}

/// Request a clipboard write from the terminal. OSC 52 has no acknowledgement;
/// callers must distinguish this request from a successful native copy.
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
fn base64(bytes: &[u8]) -> String {
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
