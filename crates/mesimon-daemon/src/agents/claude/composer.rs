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
//!
//! In bash mode (T-876) the row's glyph is `!` in place of `❯`: a `!` typed
//! or pasted into the empty box switches the mode, and the row reads
//! `! echo hi` with `! for shell mode` in the footer (measured, 2.1.306).
//! Enter runs the line, the box reads `❯ ` again and the line's echo and
//! output sit above it. That empty box is the only receipt a shell line
//! gets, since no hook fires for one (T-707).

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
/// The row's glyph in bash mode (T-876).
const SHELL: char = '!';
const RULE: char = '─';
/// The shortest run of `─` read as a rule. Claude draws it across the pane,
/// so anything this short is something else.
const RULE_MIN: usize = 8;

/// Claude Code's spinner line while it compacts (2.1.308, T-894):
/// `✻ Compacting conversation… (3s · ↓ 77 tokens)`, drawn above the
/// composer for the whole of a compaction and gone the moment it ends or
/// is interrupted — and an Esc fires no hook and writes no transcript, so
/// this line's absence is the one close an interrupted compaction has.
/// Its words up to the timer's paren, which a quotation of them lacks.
pub const COMPACTING_LINE: &str = "Compacting conversation… (";

/// The spinner's glyphs as 2.1.308 cycles them (`·` `✢` `✳` `✶` `✻` `✽`),
/// and `*` where a terminal draws none of them.
const SPINNER: &[char] = &['·', '✢', '✳', '✶', '✻', '✽', '*'];

/// The most rows the spinner's block may span above the composer: the
/// line wrapped on a narrow pane, and the tip Claude Code hangs under it.
const SPINNER_BLOCK_MAX: usize = 16;

/// The effort indicator's glyphs (`◐ medium · /effort`), hung against the
/// composer's rule: chrome, never the spinner's block, at any width.
const EFFORT: &[char] = &['○', '◔', '◑', '◐', '◕', '●'];

/// Whether `screen` shows a compaction in flight (see `COMPACTING_LINE`),
/// read only where the spinner stands (T-894 review): the nearest block of
/// rows above the composer's top rule, past the blank and right-hung
/// status rows between, never the composer itself and never scrollback
/// above that block. The spinner row is the block's row that starts with
/// a spinner glyph, joined with the rows it wrapped onto (up to a `⎿` tip
/// or the block's end), so a narrow pane that wraps the line still reads
/// as compacting. No composer, no reading: `Absent` is no screen to judge.
pub fn compacting(screen: &InputScreen) -> bool {
    let lines = &screen.lines;
    let Some(top) = composer_top(lines) else { return false };
    let mut row = top;
    // Up past the blank rows and the right-hung status rows (`◐ medium ·
    // /effort` sits against the rule) to the nearest block's last row.
    let bottom = loop {
        if row == 0 {
            return false;
        }
        row -= 1;
        let line = &lines[row];
        let body = line.trim_start();
        let indent = line.len() - body.len();
        if body.is_empty() || indent >= 20 || body.starts_with(EFFORT) {
            continue;
        }
        break row;
    };
    let mut first = bottom;
    while first > 0 && bottom - first + 1 < SPINNER_BLOCK_MAX && !lines[first - 1].trim().is_empty()
    {
        first -= 1;
    }
    (first..=bottom).any(|r| {
        let body = lines[r].trim_start();
        let Some(rest) = body.strip_prefix(SPINNER) else { return false };
        if !rest.starts_with(' ') {
            return false;
        }
        let mut joined = rest.trim().to_string();
        for next in &lines[r + 1..=bottom] {
            let next = next.trim();
            if next.starts_with('⎿') {
                break;
            }
            joined.push(' ');
            joined.push_str(next);
        }
        let joined = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        joined.starts_with(COMPACTING_LINE)
    })
}

/// The row of the composer's top rule: the same composer `read` finds.
fn composer_top(lines: &[String]) -> Option<usize> {
    for (row, line) in lines.iter().enumerate().rev() {
        let body = line.trim_start();
        if body.strip_prefix(PROMPT).or_else(|| body.strip_prefix(SHELL)).is_none() {
            continue;
        }
        if row == 0 || !is_rule(&lines[row - 1]) {
            continue;
        }
        if lines[row + 1..].iter().any(|l| is_rule(l)) {
            return Some(row - 1);
        }
    }
    None
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() >= RULE_MIN && line.chars().all(|c| c == RULE)
}

/// Read the composer off a captured screen: the bottom-most `❯` (or, in
/// bash mode, `!`) row with a rule directly above it and a rule somewhere
/// below it.
pub fn read(screen: &InputScreen) -> Composer {
    let lines = &screen.lines;
    for (row, line) in lines.iter().enumerate().rev() {
        let body = line.trim_start();
        let Some(rest) = body.strip_prefix(PROMPT).or_else(|| body.strip_prefix(SHELL)) else {
            continue;
        };
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

    /// The pane as captured mid-`/compact` on 2.1.308 (T-894), 140 columns:
    /// the spinner, the tip under it, a blank, the effort row hung against
    /// the rule, then the composer.
    fn compacting_pane(spinner: &[&str], input: &str) -> InputScreen {
        let mut rows = vec![
            "❯ say the single word hello".to_string(),
            "⏺ hello".to_string(),
            String::new(),
            "❯ /compact".to_string(),
        ];
        rows.extend(spinner.iter().map(|s| s.to_string()));
        rows.push(
            "  ⎿  Tip: Use /permissions to pre-approve and pre-deny bash, edit, and MCP tools"
                .into(),
        );
        rows.push(String::new());
        rows.push(format!("{}◐ medium · /effort", " ".repeat(100)));
        rows.push(RULE_ROW.to_string());
        rows.push(format!("❯ {input}"));
        rows.push(RULE_ROW.to_string());
        rows.push("  ⏵⏵ auto mode on (shift+tab to cycle)".to_string());
        InputScreen { lines: rows, cursor: None }
    }

    /// Claude Code 2.1.308's line for the whole of a compaction (T-894),
    /// every glyph it cycles, and the pane after an Esc out of it: the line
    /// gone and `/compact` back in the box.
    #[test]
    fn a_compaction_reads_off_its_spinner_line() {
        for glyph in ["·", "✢", "✳", "✶", "✻", "✽"] {
            let line = format!("{glyph} Compacting conversation… (3s · ↓ 77 tokens)");
            let during = compacting_pane(&[&line], "");
            assert!(compacting(&during), "{glyph}");
            assert_eq!(read(&during), Composer::Empty);
        }
        let after_esc = screen(
            &format!(
                "✻ Cogitated for 1s\n{RULE_ROW}\n❯ /compact  <optional custom summarization instructions>\n{RULE_ROW}"
            ),
            None,
        );
        assert!(!compacting(&after_esc));
        assert_eq!(read(&after_esc), Composer::Holding, "an Esc puts `/compact` back");
    }

    /// A narrow pane wraps the spinner line across rows (T-894 review: at
    /// ~20 columns the phrase never stood on one row, and two samples later
    /// a running compaction read as over). The rows it wrapped onto are
    /// joined, the effort row is chrome at any width.
    #[test]
    fn a_wrapped_spinner_line_still_reads_as_compacting() {
        let rule = "────────────────────";
        let narrow = screen(
            &format!(
                "❯ /compact\n✻ Compacting\n  conversation… (3s\n  · ↓ 77 tokens)\n  ⎿  Tip: Use\n     /permissions to\n     pre-approve\n\n ◐ medium · /effort\n{rule}\n❯ \n{rule}"
            ),
            None,
        );
        assert!(compacting(&narrow));
        let mid_word =
            screen(&format!("✻ Compacting conver\n  sation… (0s)\n\n{rule}\n❯ \n{rule}"), None);
        assert!(!compacting(&mid_word), "a word split mid-way is not the phrase");
        let at_words =
            screen(&format!("✽ Compacting\n  conversation…\n  (12s)\n{rule}\n❯ \n{rule}"), None);
        assert!(compacting(&at_words));
    }

    /// The phrase anywhere but the spinner's place is ordinary text (T-894
    /// review): `/compact` restored after an Esc with instructions that say
    /// it, a reply that quotes it, scrollback above the nearest block. Only
    /// the block nearest above the composer's rule, led by a spinner glyph,
    /// is read; with no composer, nothing is.
    #[test]
    fn the_phrase_in_ordinary_text_is_not_the_spinner() {
        let restored =
            compacting_pane(&[], "/compact keep the Compacting conversation… (3s) notes");
        assert!(!compacting(&restored), "the composer is never read");
        let quoted_reply = compacting_pane(
            &["⏺ Claude Code shows \"✻ Compacting conversation… (3s)\" while it works."],
            "",
        );
        assert!(!compacting(&quoted_reply), "a reply is not the spinner");
        let scrollback = screen(
            &format!(
                "✻ Compacting conversation… (3s · ↓ 77 tokens)\n\n⏺ done\n\n✻ Cogitated for 2s\n{RULE_ROW}\n❯ \n{RULE_ROW}"
            ),
            None,
        );
        assert!(!compacting(&scrollback), "scrollback above the nearest block");
        let no_composer =
            screen("✻ Compacting conversation… (3s)\n Do you trust this folder?", None);
        assert!(!compacting(&no_composer));
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

    /// T-876: the box in bash mode, as 2.1.306 draws it after a `!` paste.
    #[test]
    fn a_box_in_bash_mode_is_the_composer() {
        let shell = composer("", &[]).replace("❯ ", "! echo mesimon-876-a");
        assert_eq!(read(&screen(&shell, Some((20, 5)))), Composer::Holding);
        // A long line is a paste chip in the box, still held.
        let chip = composer("", &[]).replace("❯ ", "! [Pasted text #1]");
        assert_eq!(read(&screen(&chip, Some((18, 5)))), Composer::Holding);
        // The mode entered and nothing typed yet.
        let bare = composer("", &[]).replace("❯ ", "! ");
        assert_eq!(read(&screen(&bare, Some((2, 5)))), Composer::Empty);
        // Run: the box reads `❯ ` again, the line's echo above it is not a box.
        let ran =
            format!("! echo mesimon-876-a\n  ⎿  mesimon-876-a\n⏺ echo ran.\n{}", composer("", &[]));
        assert_eq!(read(&screen(&ran, Some((2, 8)))), Composer::Empty);
    }

    #[test]
    fn the_bottom_most_composer_is_the_one() {
        // A `❯` row in the history above the live one is not read.
        let history = format!("{RULE_ROW}\n❯ an old prompt\n{RULE_ROW}\n\n{}", composer("", &[]));
        assert_eq!(read(&screen(&history, Some((2, 9)))), Composer::Empty);
    }
}
