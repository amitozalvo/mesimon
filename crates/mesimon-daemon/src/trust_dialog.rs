//! Is the pane showing its harness's folder trust dialog, and which row is
//! selected? (T-844.)
//!
//! Claude Code asks whether to trust a folder before it starts there, and
//! the question comes before any hook fires: `SessionStart` lands only once
//! the dialog is answered, so the screen is the one signal. Codex asks the
//! same question in its own words at the same moment (T-824). Either way
//! the card read SETUP or TRUST and only a person at the pane could go on;
//! a phone saw "needs attention" with nothing to tap.
//!
//! The board's accept is the dialog's own keys — an arrow until the Yes
//! row is selected, then Enter — pressed for a person who asked for it,
//! with the screen read before every key. This module is the whole of what
//! decides whether a key may go: the measured shape, both rows as whole
//! rows, exactly one of them selected. A screen it does not recognise is
//! `None`, and the dialog stays up for the person. It never writes
//! `~/.claude.json` or any trust setting: Claude Code keeps its own answer,
//! exactly as it does when the person presses the key themselves.
//!
//! Measured on Claude Code 2.1.302 (its binary spells the dialog: the
//! `Accessing workspace:` heading, the `Quick safety check` paragraph,
//! `cancelFirst`, `focus: "cancel"` and `hideIndexes`, so the cursor opens
//! on `No, exit` and the rows carry no numbers) and on the 2.1.28x shape
//! the lab drivers answered (`❯ 1. Yes, I trust this folder` first, numbered;
//! `ci/claude-state-e2e.py`). The cancel row reads `No, continue without
//! these permissions` where the folder pre-approves permissions. Codex's is
//! `Trust this folder?` with `› 1. Trust and continue` and `2. Quit`
//! (0.163, the fixtures in `agents::codex`).

use mesimon_backend_tmux::InputScreen;

/// Why the board's `w` may not go: the pane shows no dialog this module
/// recognises (the status line's words).
pub const NOT_RECOGNISED: &str = "trust dialog not recognised ∙ enter to attach";
/// A walk that sent no key, on the card (T-844).
pub const NOT_ACCEPTED: &str = "could not accept the trust dialog ∙ answer it in its pane";

/// Which row the dialog's cursor sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected {
    Yes,
    No,
    /// Neither row carries the marker. Codex's plan dialog has been seen
    /// without one; the trust dialog has not, so a key never goes blind.
    Unknown,
}

/// The dialog as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub selected: Selected,
    /// Whether the Yes row is drawn above the No row: which arrow reaches it.
    pub yes_first: bool,
    /// The folder the dialog names, when the screen says.
    pub folder: Option<String>,
}

/// The one key toward the accept, from where the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
    Enter,
}

impl Dialog {
    /// The next key of an accept: Enter on the Yes row, the arrow toward it
    /// from the No row, nothing where the cursor is nowhere.
    pub fn toward_yes(&self) -> Option<Step> {
        match self.selected {
            Selected::Yes => Some(Step::Enter),
            Selected::No if self.yes_first => Some(Step::Up),
            Selected::No => Some(Step::Down),
            Selected::Unknown => None,
        }
    }
}

const CLAUDE_YES: &str = "Yes, I trust this folder";
const CLAUDE_NO: &[&str] = &["No, exit", "No, continue without these permissions"];
const CLAUDE_HEADINGS: &[&str] =
    &["Quick safety check", "Do you trust the files in this folder", "Accessing workspace:"];
const CODEX_HEADING: &str = "Trust this folder?";
const CODEX_YES: &str = "Trust and continue";
const CODEX_NO: &str = "Quit";

/// A row of a native menu: whether the marker is on it, and its label with
/// the marker and any `N. ` number stripped.
fn row(line: &str) -> (bool, &str) {
    let line = line.trim();
    let (selected, rest) = match line.strip_prefix('❯').or_else(|| line.strip_prefix('›')) {
        Some(rest) => (true, rest.trim()),
        None => (false, line),
    };
    let label = match rest.split_once(". ") {
        Some((n, label)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => label.trim(),
        _ => rest,
    };
    (selected, label)
}

/// The two rows of a dialog, Yes and No, by their exact labels: their
/// order, and which one is selected. `None` unless both are on screen as
/// whole rows and at most one carries the marker.
fn rows(lines: &[&str], yes: &str, no: &[&str]) -> Option<(bool, Selected)> {
    let mut yes_at = None;
    let mut no_at = None;
    let mut selected = Selected::Unknown;
    let mut markers = 0;
    for (i, line) in lines.iter().enumerate() {
        let (marked, label) = row(line);
        let is_yes = label == yes;
        let is_no = no.contains(&label);
        if !is_yes && !is_no {
            continue;
        }
        if is_yes {
            yes_at = Some(i);
        } else {
            no_at = Some(i);
        }
        if marked {
            markers += 1;
            selected = if is_yes { Selected::Yes } else { Selected::No };
        }
    }
    if markers > 1 {
        return None;
    }
    Some((yes_at? < no_at?, selected))
}

/// Claude Code's folder trust dialog.
pub fn claude(lines: &[&str]) -> Option<Dialog> {
    if !lines.iter().any(|l| CLAUDE_HEADINGS.iter().any(|h| l.contains(h))) {
        return None;
    }
    let (yes_first, selected) = rows(lines, CLAUDE_YES, CLAUDE_NO)?;
    let folder = lines
        .iter()
        .find_map(|l| l.trim().strip_prefix("Accessing workspace:"))
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    Some(Dialog { selected, yes_first, folder })
}

/// Codex's folder trust dialog (its hook review is another dialog, and
/// stays the pane's).
pub fn codex(lines: &[&str]) -> Option<Dialog> {
    if !lines.iter().any(|l| l.trim_start().starts_with(CODEX_HEADING)) {
        return None;
    }
    let (yes_first, selected) = rows(lines, CODEX_YES, &[CODEX_NO])?;
    Some(Dialog { selected, yes_first, folder: None })
}

/// Either harness's dialog, off a captured screen.
pub fn read(screen: &InputScreen) -> Option<Dialog> {
    let lines: Vec<&str> = screen.lines.iter().map(String::as_str).collect();
    read_lines(&lines)
}

/// Either harness's dialog, off the pane's text.
pub fn read_text(screen: &str) -> Option<Dialog> {
    let lines: Vec<&str> = screen.lines().collect();
    read_lines(&lines)
}

fn read_lines(lines: &[&str]) -> Option<Dialog> {
    claude(lines).or_else(|| codex(lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code 2.1.302's dialog as tmux captures it: the cursor opens
    /// on `No, exit`, rows unnumbered.
    const CLAUDE_302: &str = "\
 Accessing workspace: /Users/me/code/app

 Quick safety check: Is this a project you created or one you trust? (Like
 your own code, a well-known open source project, or work from your team).
 If not, take a moment to review what's in this folder first.

 Claude Code'll be able to read, edit, and execute files here.

 ❯ No, exit
   Yes, I trust this folder

 Enter to confirm · Esc to cancel
";

    /// The 2.1.28x shape the lab drivers answered: Yes first, numbered.
    const CLAUDE_28X: &str = "\
 Do you trust the files in this folder?

 ❯ 1. Yes, I trust this folder
   2. No, exit

 Enter to confirm · Esc to exit
";

    const CODEX: &str = "\
  Trust this folder? Codex can read, edit, and run files here, subject to
  your permission settings.
› 1. Trust and continue
  2. Quit
";

    fn lines(s: &str) -> Vec<&str> {
        s.lines().collect()
    }

    #[test]
    fn claude_302_opens_on_no_and_takes_a_down_then_enter() {
        let d = claude(&lines(CLAUDE_302)).expect("the dialog");
        assert_eq!(d.selected, Selected::No);
        assert!(!d.yes_first);
        assert_eq!(d.folder.as_deref(), Some("/Users/me/code/app"));
        assert_eq!(d.toward_yes(), Some(Step::Down));
        let moved = CLAUDE_302.replace(" ❯ No, exit", "   No, exit").replace("   Yes,", " ❯ Yes,");
        let d = claude(&lines(&moved)).expect("the dialog, moved");
        assert_eq!(d.selected, Selected::Yes);
        assert_eq!(d.toward_yes(), Some(Step::Enter));
        // A folder that pre-approves permissions words its cancel row
        // differently; the accept row is the same.
        let perms = CLAUDE_302.replace("No, exit", "No, continue without these permissions");
        assert_eq!(claude(&lines(&perms)).map(|d| d.selected), Some(Selected::No));
    }

    #[test]
    fn claude_28x_opens_on_yes_and_an_up_reaches_it_from_no() {
        let d = claude(&lines(CLAUDE_28X)).expect("the dialog");
        assert_eq!(d.selected, Selected::Yes);
        assert!(d.yes_first);
        assert_eq!(d.folder, None);
        assert_eq!(d.toward_yes(), Some(Step::Enter));
        let moved = CLAUDE_28X.replace(" ❯ 1.", "   1.").replace("   2.", " ❯ 2.");
        assert_eq!(claude(&lines(&moved)).and_then(|d| d.toward_yes()), Some(Step::Up));
    }

    #[test]
    fn a_marker_nowhere_or_everywhere_sends_no_key() {
        let unmarked = CLAUDE_302.replace(" ❯ No, exit", "   No, exit");
        let d = claude(&lines(&unmarked)).expect("the shape still reads");
        assert_eq!(d.selected, Selected::Unknown);
        assert_eq!(d.toward_yes(), None);
        let two = CLAUDE_302.replace("   Yes,", " ❯ Yes,");
        assert_eq!(claude(&lines(&two)), None);
    }

    #[test]
    fn the_words_alone_are_not_the_dialog() {
        // A conversation that quotes the dialog, a heading with one row, a
        // row with other words on it, the composer, an empty pane.
        for not in [
            "⏺ The trust dialog says \"Yes, I trust this folder\" and \"No, exit\".\n────\n❯ \n────",
            "Quick safety check: is this a project you trust?\n ❯ Yes, I trust this folder\n",
            "Accessing workspace: /x\n ❯ Yes, I trust this folder now\n   No, exit\n",
            "────\n❯ \n────\n  ? for shortcuts",
            "",
        ] {
            assert_eq!(read_text(not), None, "{not}");
        }
        // Codex's hook review is another dialog, and the plan dialog another.
        assert_eq!(read_text("Hooks need review\n› 1. Trust these hooks\n  2. Skip"), None);
        assert_eq!(
            read_text("Implement this plan?\n› Yes, implement this plan\n  No, stay in Plan mode"),
            None
        );
    }

    #[test]
    fn codex_opens_on_trust_and_continue() {
        let d = codex(&lines(CODEX)).expect("the dialog");
        assert_eq!(d.selected, Selected::Yes);
        assert!(d.yes_first);
        assert_eq!(d.toward_yes(), Some(Step::Enter));
        let quit = CODEX.replace("› 1.", "  1.").replace("  2. Quit", "› 2. Quit");
        assert_eq!(codex(&lines(&quit)).and_then(|d| d.toward_yes()), Some(Step::Up));
        let older = "/repo/worktree\n  Trust this folder?\n› 1. Trust and continue\n  2. Quit";
        assert_eq!(read_text(older).map(|d| d.selected), Some(Selected::Yes));
        // The screen capture carries the two rows joined by a CR in a
        // fixture; the real pane has them on their own lines.
        let unmarked = CODEX.replace("› 1.", "  1.");
        assert_eq!(codex(&lines(&unmarked)).and_then(|d| d.toward_yes()), None);
    }

    #[test]
    fn a_captured_screen_reads_the_same() {
        let screen =
            InputScreen { lines: CLAUDE_302.lines().map(str::to_string).collect(), cursor: None };
        assert_eq!(read(&screen).map(|d| d.selected), Some(Selected::No));
    }
}
