//! The agent brief: mesimon's one sentence for the agents it starts, delivered
//! through Claude Code's `--append-system-prompt` (T-224, 2026-09-05).
//!
//! T-217 offered to write the same instruction into the repo's `CLAUDE.md`. A
//! day later the author asked for this instead: the system prompt is a better
//! home — it reaches ONLY sessions mesimon starts (a CLAUDE.md speaks to every
//! session in the repo, mesimon's or not), it writes no file the user tracks
//! in git, it cannot drift from the binary that spawned it, and turning it off
//! is one Settings row. The flag is opt-in, off by default, and the dialog that
//! offers it shows [`TEXT`] verbatim, because README promise 3 says mesimon
//! adds no token to a conversation and this is the one named, consented
//! exception.
//!
//! `claudemd::SNIPPET` stays as the CLAUDE.md FORM of the same instruction:
//! `c` in the dialog copies it for a user who would rather keep the words in
//! their own file, and `mesimon doctor` prints it.

/// The Claude Code flag. A general flag, so it rides `--resume` as happily as
/// a fresh spawn — which is what lets a wake pick the switch up either way.
pub const FLAG: &str = "--append-system-prompt";

/// What is appended, verbatim — the bytes on the dialog are the bytes on the
/// argv, newlines included. Hard-wrapped at [`crate::claudemd::WRAP`] for the
/// same reason the snippet is: the dialog draws it as written.
///
/// It opens by saying WHO started the session, because the text lands in a
/// system prompt with no other mention of mesimon, and it says `get_ticket`
/// by name because that is the tool the sentence is about; `MESIMON_TICKET`
/// is not mentioned — the model does not read the environment.
pub const TEXT: &str = "\
This session was started by mesimon on a ticket. FIRST,
before reading code or planning, call the get_ticket
tool and read the ticket's description and notes: they
are the brief, and the prompt is often only the ticket's
title. Do not start work without them.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claudemd::WRAP;

    /// The confirm dialog shows this string verbatim inside a 64-wide frame,
    /// and a line past the measure would be re-wrapped on screen — then the
    /// bytes the user approved would not be the bytes on the argv.
    #[test]
    fn the_text_fits_the_dialog() {
        for line in TEXT.lines() {
            let cells = line.chars().count();
            assert!(cells <= WRAP, "`{line}` is {cells} cells; the dialog measures {WRAP}");
        }
    }

    /// It names the tool and who started the session, or it teaches nothing.
    #[test]
    fn the_text_names_mesimon_and_the_tool() {
        assert!(TEXT.contains("mesimon"), "{TEXT}");
        assert!(TEXT.contains("get_ticket"), "{TEXT}");
        assert!(!TEXT.ends_with('\n'), "a trailing newline on an argv value is noise");
    }
}
