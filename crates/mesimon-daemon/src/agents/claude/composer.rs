//! Is Claude Code's composer on screen, and is anything in it? (T-570.)
//!
//! A launch road's words go only into a composer that has painted.
//! `SessionStart` fires during startup, before Claude Code has put the tty in
//! raw mode or asked for bracketed paste, and tmux's `paste-buffer -p`
//! brackets only for an application that asked. A paste that beats that
//! moment goes in as plain bytes: the cooked tty echoes it above the banner,
//! its line discipline keeps 1 KiB of a line, and what survives reaches
//! Claude as keystrokes, where no Enter submits it (T-566, 2026-10-02).
//!
//! tmux has no format for the bracketed-paste mode (it has the cursor, keypad
//! and mouse modes, not that one), so the screen is the signal. The composer
//! measured on Claude Code 2.1 (2026-10-02) is a rule of `─` across the
//! pane, the `❯ ` row holding the input with the cursor after it, and a
//! second rule, with the footer rows under that. Wrapped input sits between
//! the `❯` row and the second rule. A trust or plan dialog's `❯ 1. Yes` row
//! has no rule directly above it, so a dialog never reads as a composer.
//!
//! The footer is not required. Its words change with the box's contents and
//! the person's settings, and the composed spawn's box always holds the
//! typed title, so requiring it would risk a brief that never goes, on a
//! screen whose input loop is already up.

use mesimon_backend_tmux::InputScreen;

/// What the screen shows of the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Composer {
    /// No composer: still starting, a dialog, or a screen this does not know.
    Absent,
    /// The composer, with nothing typed in it. A placeholder is nothing: it
    /// is drawn where the input starts, with the cursor parked before it.
    Empty,
    /// The composer, holding text.
    Holding,
}

const PROMPT: char = '❯';
const RULE: char = '─';
/// The shortest run of `─` read as a rule. Claude draws it across the pane,
/// so anything this short is something else.
const RULE_MIN: usize = 8;

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() >= RULE_MIN && line.chars().all(|c| c == RULE)
}

/// Read the composer off a captured screen: the bottom-most `❯` row with a
/// rule directly above it and a rule somewhere below it.
pub fn read(screen: &InputScreen) -> Composer {
    let lines = &screen.lines;
    for (row, line) in lines.iter().enumerate().rev() {
        let body = line.trim_start();
        let Some(rest) = body.strip_prefix(PROMPT) else { continue };
        if row == 0 || !is_rule(&lines[row - 1]) {
            continue;
        }
        let Some(bottom) = lines[row + 1..].iter().position(|l| is_rule(l)) else { continue };
        let wrapped = &lines[row + 1..row + 1 + bottom];
        let wrapped_text = wrapped.iter().any(|l| !l.trim().is_empty());
        if rest.trim().is_empty() && !wrapped_text {
            return Composer::Empty;
        }
        // The input starts after `❯ ` (both one cell). A cursor parked there
        // with nothing wrapped below is sitting in front of a placeholder.
        let start = (line.len() - body.len()) + 2;
        if !wrapped_text && screen.cursor == Some((start, row)) {
            return Composer::Empty;
        }
        return Composer::Holding;
    }
    Composer::Absent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(s: &str, cursor: Option<(usize, usize)>) -> InputScreen {
        InputScreen { lines: s.lines().map(str::to_string).collect(), cursor }
    }

    const RULE_ROW: &str =
        "────────────────────────────────────────────────────────────────────────────────";

    /// The composer as tmux captured a live Claude Code 2.1 pane, 80 columns.
    fn composer(input: &str, below: &[&str]) -> String {
        let mut rows = vec![
            "⏺ Capturing my own Claude pane's composer rows".to_string(),
            String::new(),
            "✻ Composing… (1m 5s · ↓ 4.6k tokens)".to_string(),
            String::new(),
            RULE_ROW.to_string(),
            format!("❯ {input}"),
        ];
        rows.extend(below.iter().map(|s| s.to_string()));
        rows.push(RULE_ROW.to_string());
        rows.push("  ~/.local/state/mesimon/c2c1bfcc36ead923/worktrees/T-570".to_string());
        rows.push("  ⏵⏵ auto mode on (shift+tab to cycle)".to_string());
        rows.join("\n")
    }

    #[test]
    fn an_empty_composer_is_ready() {
        assert_eq!(read(&screen(&composer("", &[]), Some((2, 5)))), Composer::Empty);
        // Hidden cursor, and a composer with no footer at all.
        let bare = format!("{RULE_ROW}\n❯ \n{RULE_ROW}");
        assert_eq!(read(&screen(&bare, None)), Composer::Empty);
    }

    #[test]
    fn typed_or_wrapped_text_is_held() {
        let typed = composer("crown). The crown guessed", &[]);
        assert_eq!(read(&screen(&typed, Some((27, 5)))), Composer::Holding);
        let wrapped = composer("first line of a long paste that", &["  wrapped onto the next row"]);
        assert_eq!(read(&screen(&wrapped, Some((28, 6)))), Composer::Holding);
        // Cursor unknown: the text is all there is to go on.
        assert_eq!(read(&screen(&typed, None)), Composer::Holding);
    }

    #[test]
    fn a_placeholder_in_front_of_the_cursor_is_empty() {
        let hint = composer("Try \"write a test for <filepath>\"", &[]);
        assert_eq!(read(&screen(&hint, Some((2, 5)))), Composer::Empty);
        // The same words with the cursor after them were typed.
        assert_eq!(read(&screen(&hint, Some((36, 5)))), Composer::Holding);
    }

    #[test]
    fn startup_and_dialogs_are_not_a_composer() {
        // The brief echoed by a cooked tty above the banner, no composer yet.
        let echoed = "A spawn's brief can beat Claude Code's raw mode\n\nFound by the crown\n";
        assert_eq!(read(&screen(echoed, Some((0, 3)))), Composer::Absent);
        assert_eq!(read(&screen("", None)), Composer::Absent);
        let trust = format!(
            "{RULE_ROW}\n Do you trust the files in this folder?\n\n ❯ 1. Yes, I trust this folder\n   2. No, exit\n\n Enter to confirm · Esc to exit"
        );
        assert_eq!(read(&screen(&trust, None)), Composer::Absent);
        let plan = " Would you like to proceed?\n\n ❯ 1. Yes, and use auto mode\n   2. Yes, manually approve edits\n   3. No, keep planning\n   4. Tell Claude what to change";
        assert_eq!(read(&screen(plan, None)), Composer::Absent);
        // Half painted: the top rule and the prompt, no bottom rule yet.
        let half = format!("{RULE_ROW}\n❯ ");
        assert_eq!(read(&screen(&half, None)), Composer::Absent);
        // A short dash run is not the composer's rule.
        assert_eq!(read(&screen("───\n❯ \n───", None)), Composer::Absent);
    }

    #[test]
    fn the_bottom_most_composer_is_the_one() {
        // A `❯` row in the history above the live one is not read.
        let history = format!("{RULE_ROW}\n❯ an old prompt\n{RULE_ROW}\n\n{}", composer("", &[]));
        assert_eq!(read(&screen(&history, Some((2, 9)))), Composer::Empty);
    }
}
