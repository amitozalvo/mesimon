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

/// The numbered rows of a native menu: `(number, label, selected)`, in
/// screen order. A row is `N. label`, selected when it opens with `❯`.
fn numbered_rows(lines: &[String]) -> Vec<(usize, String, bool)> {
    let mut rows = Vec::new();
    for line in lines {
        let line = line.trim();
        let (selected, rest) = match line.strip_prefix('❯') {
            Some(rest) => (true, rest.trim()),
            None => (false, line),
        };
        let Some((number, label)) = rest.split_once(". ") else { continue };
        let Ok(number) = number.parse::<usize>() else { continue };
        rows.push((number, label.trim().to_string(), selected));
    }
    rows
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
