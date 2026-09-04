//! The one instruction mesimon offers to put in a repo's own `CLAUDE.md`.
//!
//! A session mesimon spawns is told which ticket it is on twice: `MESIMON_TICKET`
//! in the pane's environment tells the *shell*, and the `get_ticket` MCP tool tells
//! the *model* (`server.rs::session_vars` names the two layers). Nothing told the
//! model to USE the second one — and the prompt it is handed is often only the
//! ticket's TITLE, since the composer's Shift+Enter, `start_composed` on an empty
//! seat and a wake-and-ask all submit the title while the description lives in
//! `notes[0]`. So agents skipped the description and users typed "read ticket for
//! more context" into every prompt by hand (T-217).
//!
//! This module is the text and the arithmetic; the daemon does the I/O
//! (`mesimon_daemon::claudemd`) and the TUI shows this exact string before any of
//! it is written.

/// The literal the offer scans for, and the variable the snippet is about.
///
/// Using the snippet's own subject as its marker means there is nothing to keep
/// in step: applying twice is impossible, a user who wrote the instruction in
/// their own words is never nagged, and mesimon's own repo — whose CLAUDE.md says
/// `MESIMON_TICKET` a dozen times — is correctly offered nothing.
pub const MARKER: &str = "MESIMON_TICKET";

/// What gets appended, verbatim.
///
/// **"may carry context the prompt does not"**, never "the prompt is only the
/// title": only the composed spawn submits the title alone, and an ask field or a
/// prompt typed into the pane is the user's own words. The stronger sentence would
/// be false on the commonest road, and a CLAUDE.md that is wrong once is disbelieved
/// everywhere.
///
/// Hard-wrapped at [`WRAP`] because the confirm dialog shows it VERBATIM and
/// `dialog::MAX_W` is 64 — a dialog that re-wraps the text is not showing what will
/// be written. `the_snippet_fits_the_dialog` is what holds the two together.
pub const SNIPPET: &str = "\
## mesimon

When `MESIMON_TICKET` is set, this session is working a
mesimon ticket. Call `get_ticket` before you start — the
ticket's description and notes may carry context the
prompt does not.
";

/// The column the snippet is authored to. The dialog's inner width is
/// `MAX_W - 2` = 62, less the two-space indent a fenced block gets and a cell of
/// air on the right.
pub const WRAP: usize = 56;

/// Does this file already say it? Any spelling — the marker is the subject, not a
/// signature, so a user's own paragraph counts and is left alone.
pub fn has_marker(body: &str) -> bool {
    body.contains(MARKER)
}

/// `body` with the snippet appended: exactly one blank line between whatever was
/// there and the heading, and a file that did not end in a newline gains one first.
/// An empty file (or a missing one, which reaches here as `""`) becomes the snippet
/// alone, with no leading blank.
///
/// Idempotent by construction — the caller checks [`has_marker`] first, and
/// `appending_twice_is_impossible` proves the output of this function would refuse
/// a second pass.
pub fn appended(body: &str) -> String {
    if body.trim().is_empty() {
        return SNIPPET.to_string();
    }
    let mut out = body.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(SNIPPET);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snippet has to say the two names it is about, or it teaches nothing.
    #[test]
    fn the_snippet_names_the_variable_and_the_tool() {
        assert!(SNIPPET.contains(MARKER), "the snippet must name its own marker:\n{SNIPPET}");
        assert!(SNIPPET.contains("get_ticket"), "the snippet must name the tool:\n{SNIPPET}");
        assert!(has_marker(SNIPPET));
    }

    /// The confirm dialog shows this string verbatim inside a 64-wide frame. A line
    /// past the measure would be re-wrapped on screen, and then the bytes the user
    /// approved would not be the bytes that get written.
    #[test]
    fn the_snippet_fits_the_dialog() {
        // Every character in the snippet is one cell wide (the em dash included,
        // which is why the measure can be a char count and `unicode-width` stays
        // out of core's dependency graph).
        for line in SNIPPET.lines() {
            let cells = line.chars().count();
            assert!(cells <= WRAP, "`{line}` is {cells} cells; the dialog measures {WRAP}");
        }
    }

    /// The offer's whole guarantee: what it writes suppresses the offer forever.
    #[test]
    fn appending_twice_is_impossible() {
        let once = appended("# Project\n\nSome rules.\n");
        assert!(has_marker(&once));
        assert!(once.starts_with("# Project\n\nSome rules.\n\n## mesimon\n"), "{once:?}");
    }

    /// A file that does not end in a newline still gets exactly one blank line of
    /// separation — never a heading welded onto somebody's last sentence.
    #[test]
    fn the_separator_is_one_blank_line_however_the_file_ended() {
        assert_eq!(appended("rules"), format!("rules\n\n{SNIPPET}"));
        assert_eq!(appended("rules\n"), format!("rules\n\n{SNIPPET}"));
        assert_eq!(appended("rules\n\n"), format!("rules\n\n{SNIPPET}"));
        assert_eq!(appended("rules\n\n\n"), format!("rules\n\n\n{SNIPPET}"));
    }

    /// A repo with no CLAUDE.md at all gets the snippet as the whole file, with no
    /// blank line above the heading.
    #[test]
    fn a_missing_file_becomes_the_snippet_alone() {
        assert_eq!(appended(""), SNIPPET);
        assert_eq!(appended("\n  \n"), SNIPPET);
    }
}
