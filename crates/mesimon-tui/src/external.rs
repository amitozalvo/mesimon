//! The user's own editor on a note (T-181, 2026-09-03: "vim editing in
//! notes / description"). `^g` in the note editor hands the body to
//! `$VISUAL`, else `$EDITOR`, else `vi` — git's ladder, and git's shell form,
//! so `code --wait` and a quoted path both work — on the terminal we give
//! back for the duration (the focus handover's road: restore, run, drain,
//! re-init), and what the editor wrote comes back into the body. On a note
//! it is saved at once: leaving the editor IS the commit, as it is for a
//! commit message, and an owed `^s` afterwards would be the one step no
//! `$EDITOR` integration the user knows asks for.
//!
//! The file lives under the state dir (`<state>/edit/<name>.md`, 0600, gone
//! when the editor exits) — inside the README's write allowlist, and out of
//! `.mesimon/`, where a swap file would be a stranger. `.md` so the editor
//! reads it as markdown, which is what a note is.
//!
//! The one seam vim has that the field does not: `fixeol` writes a trailing
//! newline the body never held, so a round trip that changed nothing would
//! come back "changed" and dirty every save after it. `back_to_body` gives
//! that one newline back.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};

use crate::handover;

/// What `^g` asked for: the body as it stands and the file's name, parked on
/// `App::pending_external_edit` for the main loop, which owns the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEdit {
    pub text: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The editor exited clean and the file reads back as it went out
    /// (`:q!`, or a write that changed nothing).
    Unchanged,
    /// The file as the editor left it, the body's shape restored.
    Changed(String),
}

/// The fallback of the ladder. `vi` is on every macOS and Linux install, and
/// on the author's Mac it IS vim.
const FALLBACK: &str = "vi";

/// `$VISUAL`, else `$EDITOR`, else `vi`; an empty value is unset.
pub fn command() -> String {
    command_from(std::env::var("VISUAL").ok().as_deref(), std::env::var("EDITOR").ok().as_deref())
}

fn command_from(visual: Option<&str>, editor: Option<&str>) -> String {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or(FALLBACK)
        .to_string()
}

/// The footer's word for the editor: the command's first token, as a file
/// name (`/opt/homebrew/bin/nvim` is `nvim`), one line, at most 16 cells.
/// Read once per process — the environment does not change under a running
/// TUI — and leaked once, because a hint is a `&'static str`.
pub fn word() -> &'static str {
    static WORD: OnceLock<String> = OnceLock::new();
    WORD.get_or_init(|| word_of(&command()))
}

pub(crate) fn word_of(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or(FALLBACK);
    let base = Path::new(first).file_name().and_then(|f| f.to_str()).unwrap_or(FALLBACK);
    crate::text::truncate(&crate::text::one_line(base), 16)
}

/// What `mesimon doctor` says: the program and where it came from.
pub fn doctor_line() -> String {
    let visual = std::env::var("VISUAL").ok().filter(|v| !v.trim().is_empty());
    let editor = std::env::var("EDITOR").ok().filter(|v| !v.trim().is_empty());
    let source = match (visual.is_some(), editor.is_some()) {
        (true, _) => "$VISUAL",
        (false, true) => "$EDITOR",
        (false, false) => "neither $VISUAL nor $EDITOR is set",
    };
    format!("{} ({source}) — ^g in a note", word())
}

/// Where the files go: `<state>/edit/`, beside the board's other state.
pub fn edit_dir(repo_root: &Path) -> Result<PathBuf> {
    Ok(mesimon_daemon::Paths::for_repo(repo_root)?.state_dir.join("edit"))
}

/// Git's form: through the shell, the command as `$0` and the file as `$1`,
/// so an `$EDITOR` with flags or a path with a space in it both run.
fn argv(command: &str, file: &Path) -> Vec<String> {
    vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("{command} \"$@\""),
        command.into(),
        file.display().to_string(),
    ]
}

/// Run outside raw mode / alt screen — the caller restores the terminal
/// first and re-initialises after, exactly as for a focus handover.
pub fn run(req: &ExternalEdit, dir: &Path) -> Result<Outcome> {
    run_with(&command(), req, dir)
}

pub(crate) fn run_with(command: &str, req: &ExternalEdit, dir: &Path) -> Result<Outcome> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(&req.file_name);
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("write {}", path.display()))?;
        // A text file ends with a newline; the editor would add one anyway.
        f.write_all(req.text.as_bytes())?;
        if !req.text.is_empty() && !req.text.ends_with('\n') {
            f.write_all(b"\n")?;
        }
    }
    let ran = handover::run(&argv(command, &path), None);
    let read = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    ran?;
    let edited = read.with_context(|| format!("read back {}", path.display()))?;
    let body = back_to_body(&req.text, edited);
    Ok(if body == req.text { Outcome::Unchanged } else { Outcome::Changed(body) })
}

/// The edited file in the body's shape: the one trailing newline the file
/// carries and the body did not is taken back, and nothing else is touched
/// (a body that ended with a blank line keeps it).
fn back_to_body(original: &str, mut edited: String) -> String {
    if !original.ends_with('\n') && edited.ends_with('\n') {
        edited.pop();
    }
    edited
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_is_visual_then_editor_then_vi() {
        assert_eq!(command_from(Some("nvim"), Some("nano")), "nvim");
        assert_eq!(command_from(None, Some("nano")), "nano");
        assert_eq!(command_from(Some("  "), Some("code --wait")), "code --wait");
        assert_eq!(command_from(None, None), "vi");
        assert_eq!(command_from(Some(""), Some("")), "vi");
    }

    /// The footer's word is the program, not its path or its flags.
    #[test]
    fn the_word_is_the_programs_name() {
        assert_eq!(word_of("nvim"), "nvim");
        assert_eq!(word_of("/opt/homebrew/bin/nvim"), "nvim");
        assert_eq!(word_of("code --wait"), "code");
        assert_eq!(word_of("  "), "vi");
        assert_eq!(word_of("a-very-long-editor-name-indeed"), "a-very-long-edi~");
    }

    #[test]
    fn the_editors_trailing_newline_is_given_back() {
        assert_eq!(back_to_body("a\nb", "a\nb\n".into()), "a\nb");
        assert_eq!(back_to_body("a\nb\n", "a\nb\n".into()), "a\nb\n");
        assert_eq!(back_to_body("a", "a\n\n".into()), "a\n");
        assert_eq!(back_to_body("", "".into()), "");
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-external-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// A real child through the shell: the file goes out with the body, an
    /// editor that rewrites it comes back `Changed`, one that leaves it comes
    /// back `Unchanged` (vim's newline included), and the file is gone after.
    #[test]
    fn the_file_goes_out_and_comes_back() {
        let d = dir("roundtrip");
        let req = ExternalEdit { text: "# Why\n\nbecause".into(), file_name: "T-1.md".into() };
        let out = run_with("sh -c 'printf \"%s\\n\" \"$(cat \"$1\")\" not > \"$1\"' sh", &req, &d)
            .unwrap();
        assert_eq!(out, Outcome::Changed("# Why\n\nbecause\nnot".into()));
        assert!(!d.join("T-1.md").exists(), "the file is gone after");
        // `true` touches nothing; the trailing newline we wrote is not a change.
        assert_eq!(run_with("true", &req, &d).unwrap(), Outcome::Unchanged);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An editor that exits non-zero is an error and the body stays what it
    /// was — nothing is read back from a run that failed.
    #[test]
    fn a_failed_editor_is_an_error_and_leaves_no_file() {
        let d = dir("fail");
        let req = ExternalEdit { text: "keep".into(), file_name: "T-2.md".into() };
        let err = run_with("false", &req, &d).unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
        assert!(!d.join("T-2.md").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
