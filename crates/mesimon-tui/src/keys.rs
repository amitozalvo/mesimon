//! The one place crossterm's input vocabulary becomes the keymap's. Core owns
//! the keymap and must not know about the input stack (04 §1.0), so every
//! keypress crosses here exactly once, on its way into `keymap::resolve`.

use mesimon_core::keymap::Key;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// `None` for atoms the keymap has no vocabulary for — they are unbound by
/// construction rather than by omission.
pub fn to_key(code: KeyCode, mods: KeyModifiers) -> Option<Key> {
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    Some(match code {
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

/// Ctrl or Alt both mean "by word" in a text field — terminals disagree about
/// which one ctrl+backspace and option+arrow actually report. This stays out
/// of the keymap table because it is a modifier reading, not a binding.
pub fn word_wise(mods: KeyModifiers) -> bool {
    mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

#[cfg(test)]
mod tests {
    use super::*;

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
