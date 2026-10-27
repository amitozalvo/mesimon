//! Is the pane showing its harness's plan dialog at the harness's default
//! row? (T-420.) The board's "accept plan" is one Enter into that dialog,
//! and this is the whole of what decides whether the Enter may go: never an
//! option label mesimon picked, never a press into a screen it did not
//! recognise. A refusal names its reason, and the dialog stays up for the
//! person to answer in the pane.
//!
//! The Claude shape is the one Mesophon's `dialog_step` measured (T-317):
//! the `Would you like to proceed?` heading, numbered rows, one `❯` on the
//! selected row, and the `Tell Claude what to change` row that closes the
//! list. Row 1 is whatever the harness puts first — `Yes, and use auto mode`
//! where auto is available, `Yes, auto-accept edits` otherwise, the bypass
//! variant where the session was launched with bypass — which is exactly
//! why the check is "row 1 is selected" and not a label: the harness's
//! default is the harness's to define. The Codex shape is `codex::plan_dialog`
//! (2026-09-10), its selection marker unmeasured, so a Codex dialog with no
//! marker anywhere is taken at its shape alone (the dialog opens on `Yes`)
//! and one with a marker off the `Yes` row is refused.

/// Why the Enter may not go: the words the status line shows.
pub const NOT_RECOGNISED: &str = "plan dialog not recognised ∙ enter to attach";
pub const NOT_AT_DEFAULT: &str = "plan dialog is not at its default ∙ enter to attach";

use mesimon_core::text::squeeze;

/// A row of a native menu, Claude Code's or Codex's: whether the cursor
/// marker (`❯`, Codex's `›`) is on it, its `N.` number if it carries one,
/// and its label with both stripped. The one parse of a menu row; the
/// trust, plan and question dialogs all read it.
pub(crate) fn menu_row(line: &str) -> (bool, Option<usize>, &str) {
    let line = line.trim();
    let (selected, rest) = match line.strip_prefix('❯').or_else(|| line.strip_prefix('›')) {
        Some(rest) => (true, rest.trim()),
        None => (false, line),
    };
    match rest.split_once(". ") {
        Some((n, label)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
            (selected, n.parse().ok(), label.trim())
        }
        _ => (selected, None, rest),
    }
}

/// The numbered rows of a native menu: `(number, label, selected)`, in
/// screen order. A row is `N. label`, selected when it opens with the marker.
fn numbered_rows(lines: &[String]) -> Vec<(usize, String, bool)> {
    lines
        .iter()
        .filter_map(|line| {
            let (selected, number, label) = menu_row(line);
            Some((number?, label.to_string(), selected))
        })
        .collect()
}

/// Claude Code's plan dialog, at its first row.
pub fn claude_at_default(lines: &[String]) -> Result<(), &'static str> {
    let text = lines
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ");
    if !text.contains("Would you like to proceed?") || !text.contains("Tell Claude what to change")
    {
        return Err(NOT_RECOGNISED);
    }
    let rows = numbered_rows(lines);
    let selected: Vec<&(usize, String, bool)> = rows.iter().filter(|r| r.2).collect();
    let [one] = selected.as_slice() else {
        return Err(NOT_RECOGNISED);
    };
    let first = rows.iter().map(|r| r.0).min().unwrap_or(1);
    if one.0 != first {
        return Err(NOT_AT_DEFAULT);
    }
    // The default row is an acceptance by construction; a harness that
    // ever put "keep planning" first would not be accepted blind.
    if one.1.starts_with("No,") || one.1.starts_with("Tell ") {
        return Err(NOT_AT_DEFAULT);
    }
    Ok(())
}

/// Codex's native `Implement this plan?` dialog, at its `Yes` row.
pub fn codex_at_default(screen: &mesimon_backend_tmux::InputScreen) -> Result<(), &'static str> {
    if !crate::agents::codex::plan_dialog(screen) {
        return Err(NOT_RECOGNISED);
    }
    let marked: Vec<&str> = screen
        .lines
        .iter()
        .map(|l| l.trim())
        .filter_map(|l| {
            l.strip_prefix('❯').or_else(|| l.strip_prefix('›')).or_else(|| l.strip_prefix('>'))
        })
        .map(str::trim)
        .filter(|rest| rest.contains("this plan") || rest.contains("Plan mode"))
        .collect();
    match marked.as_slice() {
        [] => Ok(()),
        [one] if one.contains("Yes, implement this plan") => Ok(()),
        _ => Err(NOT_AT_DEFAULT),
    }
}

/// The plan dialog's feedback row as Claude Code draws it (measured on
/// 2.1.310, T-943): `N. Tell Claude what to change` while its field is
/// empty, the field's words in that place once typed or pasted, and under it
/// the hint `shift+tab to approve with this feedback` and the `ctrl+g to
/// edit in <editor> · <plan file>` line, which `dialog_rows` joins onto it.
pub(crate) const FEEDBACK_ROW: &str = "Tell Claude what to change";
const APPROVE_HINT: &str = "shift+tab to approve with this feedback";
const EDITOR_LINE: &str = "ctrl+g to edit in ";

/// One row of the plan dialog's menu as the walk reads it (T-943): whether
/// the cursor is on it, and its field, squeezed.
pub(crate) struct MenuRow {
    pub(crate) selected: bool,
    pub(crate) field: String,
}

/// The plan dialog's menu, parsed for the walk alone (T-943), as one extent:
/// it begins after the line that completes the dialog's question (`Would you
/// like to proceed?`, wrapped or not), and its rows are the numbered lines
/// after it, numbered 1, 2, 3 … with no skip and no restart. A numbered line
/// out of that order (a later `1.` among them) and a second question after
/// the first are a person's draft the walk cannot tell from the menu: `None`,
/// and the walk stops. A row's field is its numbered line and every line
/// under it to the next row, and the last row's runs to the end of the
/// screen, through blank and box-drawing lines, since a person's draft in the
/// native field may hold any of them. No footer is cut: 2.1.310 draws none,
/// and a line that merely holds `Enter to select` is the field's. From the
/// end of the last row come off trailing blank lines, then the editor line
/// (`ctrl+g to edit in <editor> · <file>`) with the approve hint line
/// directly above it when it is there, never the row's own first line. The
/// approve hint without its editor line under it is the field's. Whatever
/// is left is the field, compared whole. The cursor is read from the same
/// rows.
pub(crate) fn plan_menu(screen: &str) -> Option<Vec<MenuRow>> {
    let lines: Vec<&str> = screen.lines().map(str::trim).collect();
    let question = squeeze("Would you like to proceed?");
    let mut said = String::new();
    let asked = lines.iter().position(|line| {
        said.push_str(&squeeze(line));
        said.contains(&question)
    })?;
    let after: String = lines[asked + 1..].iter().map(|line| squeeze(line)).collect();
    if after.contains(&question) {
        return None;
    }
    let mut rows: Vec<(bool, Vec<&str>)> = Vec::new();
    for line in &lines[asked + 1..] {
        match menu_row(line) {
            (selected, Some(number), label) if number == rows.len() + 1 => {
                rows.push((selected, vec![label]))
            }
            (_, Some(_), _) => return None,
            (_, None, _) => {
                if let Some((_, block)) = rows.last_mut() {
                    block.push(line);
                }
            }
        }
    }
    let last = rows.len().checked_sub(1)?;
    for (at, (_, block)) in rows.iter_mut().enumerate() {
        while block.len() > 1 && block.last().is_some_and(|l| l.is_empty()) {
            block.pop();
        }
        if at != last {
            continue;
        }
        // The pair, or nothing: the approve hint is the hint only directly
        // above the editor line it is drawn with; alone, it is the field's.
        if block.len() > 2
            && block.last().is_some_and(|l| editor_line(l))
            && block[block.len() - 2] == APPROVE_HINT
        {
            block.truncate(block.len() - 2);
        } else if block.len() > 1 && block.last().is_some_and(|l| editor_line(l)) {
            block.pop();
        }
    }
    Some(
        rows.into_iter()
            .map(|(selected, block)| MenuRow { selected, field: squeeze(&block.concat()) })
            .collect(),
    )
}

/// The one row the cursor is on, or `None`: none of the measured shape.
pub(crate) fn menu_cursor(rows: &[MenuRow]) -> Option<usize> {
    let mut selected = rows.iter().enumerate().filter(|(_, r)| r.selected);
    match (selected.next(), selected.next()) {
        (Some((at, _)), None) => Some(at),
        _ => None,
    }
}

/// The editor line under the plan dialog, as measured on 2.1.310:
/// `ctrl+g to edit in Nvim · ~/.claude/plans/<name>.md`.
fn editor_line(line: &str) -> bool {
    line.strip_prefix(EDITOR_LINE).is_some_and(|rest| rest.contains(" · "))
}

/// Whether `screen` draws Claude Code's plan dialog, its feedback field
/// empty or not (T-943): the question and the manual accept row, which every
/// measured version shows.
pub(crate) fn drawn(screen: &str) -> bool {
    let screen = squeeze(screen);
    screen.contains("Wouldyouliketoproceed?") && screen.contains("Yes,manuallyapproveedits")
}

/// The plan dialog's cursor row on `screen`, if it shows exactly one.
pub(crate) fn plan_cursor(screen: &str) -> Option<usize> {
    plan_menu(screen).and_then(|rows| menu_cursor(&rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    const CLAUDE: &str = "\
 Would you like to proceed?

 ❯ 1. Yes, and use auto mode
   2. Yes, manually approve edits
   3. No, keep planning
   4. Tell Claude what to change
";

    #[test]
    fn claude_accepts_row_one_and_refuses_everything_else() {
        assert_eq!(claude_at_default(&lines(CLAUDE)), Ok(()));
        // The person attached and walked the cursor: not the default.
        let moved = CLAUDE.replace("❯ 1.", "  1.").replace("  2.", "❯ 2.");
        assert_eq!(claude_at_default(&lines(&moved)), Err(NOT_AT_DEFAULT));
        let no = CLAUDE.replace("❯ 1.", "  1.").replace("  3.", "❯ 3.");
        assert_eq!(claude_at_default(&lines(&no)), Err(NOT_AT_DEFAULT));
        // No dialog, two cursors, or a heading without the list: unknown.
        assert_eq!(claude_at_default(&lines("› ordinary composer")), Err(NOT_RECOGNISED));
        let two = CLAUDE.replace("  2.", "❯ 2.");
        assert_eq!(claude_at_default(&lines(&two)), Err(NOT_RECOGNISED));
        assert_eq!(
            claude_at_default(&lines("Would you like to proceed?\n❯ 1. Yes")),
            Err(NOT_RECOGNISED)
        );
        // The label of row 1 is the harness's: any acceptance wording goes.
        let edits = CLAUDE.replace("Yes, and use auto mode", "Yes, auto-accept edits");
        assert_eq!(claude_at_default(&lines(&edits)), Ok(()));
    }

    #[test]
    fn codex_accepts_its_dialog_on_the_yes_row() {
        let screen = |s: &str| mesimon_backend_tmux::InputScreen { lines: lines(s), cursor: None };
        let dialog = "Implement this plan?\n› Yes, implement this plan\n  No, stay in Plan mode\n";
        assert_eq!(codex_at_default(&screen(dialog)), Ok(()));
        let unmarked =
            "Implement this plan?\n  Yes, implement this plan\n  No, stay in Plan mode\n";
        assert_eq!(codex_at_default(&screen(unmarked)), Ok(()));
        let no = "Implement this plan?\n  Yes, implement this plan\n› No, stay in Plan mode\n";
        assert_eq!(codex_at_default(&screen(no)), Err(NOT_AT_DEFAULT));
        assert_eq!(codex_at_default(&screen("› ")), Err(NOT_RECOGNISED));
    }
}
