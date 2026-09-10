//! The one place crossterm's input vocabulary becomes the keymap's. Core owns
//! the keymap and must not know about the input stack (04 §1.0), so every
//! keypress crosses here exactly once, on its way into `keymap::resolve`.

use mesimon_core::keymap::Key;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// `None` for atoms the keymap has no vocabulary for — they are unbound by
/// construction rather than by omission.
pub fn to_key(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT) && !ctrl;
    Some(match code {
        // Option/Alt on a direction, either spelling, is one atom. Whether it
        // arrives at all is the terminal's call and nothing detects it in
        // advance, which is why the keymap spends no capability on it
        // (`keymap::alt_is_admitted_only_for_a_nudge`): macOS Terminal
        // composes `⌥h` into `˙`, iTerm2 sends the modifier only with
        // `Option Key Sends: Esc+`, and a profile that maps `⌥←` to a word
        // jump sends `esc b` — an alt+`b` this map has no atom for, so it
        // resolves to nothing rather than to the wrong verb.
        KeyCode::Char('h') | KeyCode::Left if alt => Key::AltLeft,
        KeyCode::Char('l') | KeyCode::Right if alt => Key::AltRight,
        KeyCode::Char('k') | KeyCode::Up if alt => Key::AltUp,
        KeyCode::Char('j') | KeyCode::Down if alt => Key::AltDown,
        // Shift held on a ctrl+letter is its own atom, uppercase — only the
        // kitty tier reports it (a legacy terminal sends the bare control
        // byte, no modifier), and the keymap gates every such binding on
        // `Ctx::rich_keys`, so the press degrades to the lowercase atom.
        KeyCode::Char(c)
            if ctrl && c.is_ascii_alphabetic() && mods.contains(KeyModifiers::SHIFT) =>
        {
            Key::Ctrl(c.to_ascii_uppercase())
        }
        // `to_ascii_lowercase` leaves `]` and `5` alone, which is what makes
        // the two spellings of ctrl+] (kitty's true `C-]`, and the `C-5` that
        // legacy terminals send for 0x1D) land on the same atom.
        KeyCode::Char(c) if ctrl => Key::Ctrl(c.to_ascii_lowercase()),
        KeyCode::Char(' ') => Key::Space,
        KeyCode::Char(c) => Key::Char(c),
        // Shift+Enter is its own atom ONLY where the kitty disambiguate tier
        // is on; every other terminal reports it as a bare Enter, and the
        // keymap gates the atom on `Ctx::rich_keys` so it stays unbound there
        // rather than half-working.
        KeyCode::Enter if mods.contains(KeyModifiers::SHIFT) => Key::ShiftEnter,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    })
}

/// The same conversion, as a text field reads it: Alt there is the "by word"
/// modifier ([`word_wise`]), never an atom of its own — a composer that let
/// `alt+←` become [`Key::AltLeft`] would stop jumping by word, and the board's
/// nudge is not something a half-typed title can do anyway.
pub fn to_key_text(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    to_key(text_code(code, mods), mods - KeyModifiers::ALT)
}

/// Some terminal profiles send Option+Left/Right as ESC b/f (Alt+b/f).
/// Normalize those to arrows only inside text fields, retaining the modifier
/// so the existing word movement applies. Control still owns its chords.
pub(crate) fn text_code(code: KeyCode, mods: KeyModifiers) -> KeyCode {
    if mods.contains(KeyModifiers::ALT) && !mods.contains(KeyModifiers::CONTROL) {
        match code {
            KeyCode::Char('b') => KeyCode::Left,
            KeyCode::Char('f') => KeyCode::Right,
            _ => code,
        }
    } else {
        code
    }
}

/// Ctrl or Alt both mean "by word" in a text field — terminals disagree about
/// which one ctrl+backspace and option+arrow actually report. This stays out
/// of the keymap table because it is a modifier reading, not a binding.
pub fn word_wise(mods: KeyModifiers) -> bool {
    mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both spellings of a direction land on one atom, and the modifier only
    /// counts on the four the keymap knows: an `alt+b` (what iTerm2's word
    /// jump sends for `⌥←`) must not arrive as a bare `b` and file a card.
    #[test]
    fn alt_directions_are_one_atom_and_nothing_else_is() {
        let a = KeyModifiers::ALT;
        assert_eq!(to_key(KeyCode::Char('h'), a), Some(Key::AltLeft));
        assert_eq!(to_key(KeyCode::Left, a), Some(Key::AltLeft));
        assert_eq!(to_key(KeyCode::Char('j'), a), Some(Key::AltDown));
        assert_eq!(to_key(KeyCode::Down, a), Some(Key::AltDown));
        assert_eq!(to_key(KeyCode::Char('k'), a), Some(Key::AltUp));
        assert_eq!(to_key(KeyCode::Char('l'), a), Some(Key::AltRight));
        assert_eq!(to_key(KeyCode::Char('b'), a), Some(Key::Char('b')));
        // Ctrl wins the letter it already owns.
        assert_eq!(to_key(KeyCode::Char('t'), a | KeyModifiers::CONTROL), Some(Key::Ctrl('t')));
        // ctrl+shift+letter is the uppercase atom, whichever case the
        // terminal reports the letter in; without Shift, lowercase always.
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(to_key(KeyCode::Char('s'), cs), Some(Key::Ctrl('S')));
        assert_eq!(to_key(KeyCode::Char('S'), cs), Some(Key::Ctrl('S')));
        assert_eq!(to_key(KeyCode::Char('S'), KeyModifiers::CONTROL), Some(Key::Ctrl('s')));
        assert_eq!(to_key(KeyCode::Char(']'), cs), Some(Key::Ctrl(']')));
        // Unmodified, they are the plain atoms they always were.
        assert_eq!(to_key(KeyCode::Char('h'), KeyModifiers::NONE), Some(Key::Char('h')));
        assert_eq!(to_key(KeyCode::Left, KeyModifiers::NONE), Some(Key::Left));
    }

    /// A text field reads Alt as "by word": the arrows must stay arrows there
    /// or the composer loses `alt+←`.
    #[test]
    fn a_text_field_never_sees_an_alt_atom() {
        let a = KeyModifiers::ALT;
        assert_eq!(to_key_text(KeyCode::Left, a), Some(Key::Left));
        assert_eq!(to_key_text(KeyCode::Char('h'), a), Some(Key::Char('h')));
        assert!(word_wise(a));
    }

    #[test]
    fn terminal_word_sequences_are_arrows_only_in_text_fields() {
        for (letter, direction) in [('b', Key::Left), ('f', Key::Right)] {
            let code = KeyCode::Char(letter);
            assert_eq!(to_key_text(code, KeyModifiers::ALT), Some(direction));
            assert_eq!(to_key_text(code, KeyModifiers::NONE), Some(Key::Char(letter)));
            assert_eq!(to_key(code, KeyModifiers::ALT), Some(Key::Char(letter)));
            // Control retains precedence over Alt on letter chords.
            for mods in [KeyModifiers::CONTROL, KeyModifiers::CONTROL | KeyModifiers::ALT] {
                assert_eq!(to_key_text(code, mods), Some(Key::Ctrl(letter)));
            }
        }
    }

    #[test]
    fn both_spellings_of_ctrl_bracket_agree() {
        let c = KeyModifiers::CONTROL;
        assert_eq!(to_key(KeyCode::Char(']'), c), Some(Key::Ctrl(']')));
        assert_eq!(to_key(KeyCode::Char('5'), c), Some(Key::Ctrl('5')));
    }

    #[test]
    fn shift_letter_is_the_uppercase_atom() {
        assert_eq!(to_key(KeyCode::Char('D'), KeyModifiers::SHIFT), Some(Key::Char('D')));
        assert_eq!(to_key(KeyCode::Char('d'), KeyModifiers::NONE), Some(Key::Char('d')));
    }

    #[test]
    fn space_is_its_own_atom_not_a_char() {
        assert_eq!(to_key(KeyCode::Char(' '), KeyModifiers::NONE), Some(Key::Space));
    }

    /// Shift+Enter is a separate atom; plain Enter must never become it (the
    /// composer's save and its save+start are different commitments).
    #[test]
    fn shift_enter_is_its_own_atom() {
        assert_eq!(to_key(KeyCode::Enter, KeyModifiers::SHIFT), Some(Key::ShiftEnter));
        assert_eq!(to_key(KeyCode::Enter, KeyModifiers::NONE), Some(Key::Enter));
        // Ctrl+Enter is not Shift+Enter — it falls back to the plain atom
        // rather than borrowing the harder verb.
        assert_eq!(to_key(KeyCode::Enter, KeyModifiers::CONTROL), Some(Key::Enter));
    }

    /// A ctrl-letter must never arrive as a bare char, or it would type its
    /// letter into a field instead of running its chord.
    #[test]
    fn ctrl_letters_never_look_printable() {
        for c in ['a', 'w', 'u', 'l', 'z', 'c'] {
            assert_eq!(
                to_key(KeyCode::Char(c), KeyModifiers::CONTROL),
                Some(Key::Ctrl(c)),
                "ctrl+{c}"
            );
        }
    }
}
