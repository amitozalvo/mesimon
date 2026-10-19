//! The user's own editor on a note (T-181, 2026-09-03: "vim editing in
//! notes / description"). `^g` in the note editor hands the body to the
//! editor the ladder finds — `$VISUAL`, `$EDITOR`, git's `core.editor`, the
//! editor whose terminal the board runs in, then `vi` (T-864) — or the one
//! the Settings row names, in git's shell form, so `code --wait` and a
//! quoted path both work, on the terminal we give back for the duration (the
//! focus handover's road: restore, run, drain, re-init), and what the editor
//! wrote comes back into the body. On a note it is saved at once: leaving
//! the editor IS the commit, as it is for a commit message, and an owed `^s`
//! afterwards would be the one step no `$EDITOR` integration the user knows
//! asks for.
//!
//! A window editor (VS Code, Cursor, Zed, Sublime) returns at once unless it
//! is told to wait, and then the body would come back unchanged and the file
//! be deleted under the open tab. Such an editor named without its wait flag
//! gets it (`code` runs as `code --wait`), and a link to a file opens in it
//! detached, at its line, with the board left up.
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

use anyhow::{Context, Result};

use crate::handover;

/// What `^g` asked for: the body as it stands, the file's name and the
/// editor that opens it, parked on `App::pending_external_edit` for the
/// main loop, which owns the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEdit {
    pub text: String,
    pub file_name: String,
    pub editor: Resolved,
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

/// How an editor is told a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// `+N file`: vi's family, nano, emacs, micro, kakoune.
    Plus,
    /// `file:N`: Helix, Zed, Sublime.
    Colon,
    /// `-g file:N`: VS Code and the editors built on it.
    Goto,
}

/// An editor mesimon knows by name: how it takes a line, and, for one that
/// opens a window of its own, the flag that makes it wait for the tab to
/// close and where macOS keeps its command when it is not on PATH.
#[derive(Debug, PartialEq, Eq)]
pub struct Known {
    pub word: &'static str,
    line: Line,
    wait: Option<&'static str>,
    bundle: Option<&'static str>,
}

const fn term(word: &'static str, line: Line) -> Known {
    Known { word, line, wait: None, bundle: None }
}

const fn window(word: &'static str, line: Line, wait: &'static str, bundle: &'static str) -> Known {
    Known { word, line, wait: Some(wait), bundle: Some(bundle) }
}

/// Every editor the Settings row can name, in the order it offers them.
const KNOWN: &[Known] = &[
    window("code", Line::Goto, "--wait", "Visual Studio Code.app/Contents/Resources/app/bin/code"),
    window("cursor", Line::Goto, "--wait", "Cursor.app/Contents/Resources/app/bin/cursor"),
    window("windsurf", Line::Goto, "--wait", "Windsurf.app/Contents/Resources/app/bin/windsurf"),
    window("codium", Line::Goto, "--wait", "VSCodium.app/Contents/Resources/app/bin/codium"),
    window("zed", Line::Colon, "--wait", "Zed.app/Contents/MacOS/cli"),
    window("subl", Line::Colon, "-w", "Sublime Text.app/Contents/SharedSupport/bin/subl"),
    term("nvim", Line::Plus),
    term("vim", Line::Plus),
    term("vi", Line::Plus),
    term("hx", Line::Colon),
    term("kak", Line::Plus),
    term("micro", Line::Plus),
    term("nano", Line::Plus),
    term("emacs", Line::Plus),
    term("vis", Line::Plus),
];

fn known(word: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.word == word)
}

/// A name the Settings row can hold, as the table spells it: how
/// `prefs.json`'s `editor` is read.
pub(crate) fn known_word(word: &str) -> Option<&'static str> {
    known(word).map(|k| k.word)
}

/// Where the ladder's answer came from — what the Settings row and
/// `doctor` say beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Visual,
    Editor,
    Git,
    Host,
    Fallback,
}

impl Source {
    pub fn words(self) -> &'static str {
        match self {
            Source::Visual => "$VISUAL",
            Source::Editor => "$EDITOR",
            Source::Git => "git's core.editor",
            Source::Host => "the editor this terminal is in",
            Source::Fallback => "nothing set",
        }
    }
}

/// What `auto` opens: the ladder's command as found, where it came from,
/// and its word, worked out once.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Auto {
    command: String,
    source: Source,
    known: Option<&'static Known>,
    word: &'static str,
}

/// What this machine offers `^g` (T-864), found once per process in
/// `lib.rs::run` — never in `App::new`, so no test app reads a developer's
/// `$EDITOR`. A default one finds nothing, and `^g` is inert.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Editors {
    auto: Option<Auto>,
    /// The known editors this machine has, each with the command that runs
    /// it: its name on PATH, else its path inside its macOS app.
    installed: Vec<(&'static str, String)>,
}

/// The editor one `^g` or one link runs: the command with its wait flag,
/// and what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub command: String,
    pub word: &'static str,
    known: Option<&'static Known>,
}

/// How a link to a file opens: in this terminal, handed over (`App::
/// pending_attach`), or detached in an editor's own window, the board left
/// up (`App::pending_open`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    Here(Vec<String>),
    Outside(Vec<String>),
}

impl Editors {
    /// The ladder and the installed list, read from this process's
    /// environment, the repo's git config and the disk.
    pub fn find(repo: &Path) -> Editors {
        let env = |k: &str| std::env::var(k).ok();
        let git = mesimon_daemon::git::git(repo)
            .args(["config", "--get", "core.editor"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let installed = installed(|w| crate::opener::which_on_path(w).is_some(), mac_app);
        let host = host(
            env("TERM_PROGRAM").as_deref(),
            env("__CFBundleIdentifier").as_deref(),
            env("CURSOR_TRACE_ID").is_some(),
        );
        Editors::from(
            env("VISUAL").as_deref(),
            env("EDITOR").as_deref(),
            git.as_deref(),
            host,
            installed,
        )
    }

    fn from(
        visual: Option<&str>,
        editor: Option<&str>,
        git: Option<&str>,
        host: Option<&'static str>,
        installed: Vec<(&'static str, String)>,
    ) -> Editors {
        let set = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(String::from);
        let host = host.and_then(|w| installed.iter().find(|(i, _)| *i == w)).cloned();
        let (command, source, known) = if let Some(c) = set(visual) {
            (c, Source::Visual, None)
        } else if let Some(c) = set(editor) {
            (c, Source::Editor, None)
        } else if let Some(c) = set(git) {
            (c, Source::Git, None)
        } else if let Some((word, c)) = host {
            (c, Source::Host, known(word))
        } else {
            (FALLBACK.to_string(), Source::Fallback, None)
        };
        let known = known.or_else(|| self::known(&word_of(&command)));
        // The one word that is not a `&'static str` already is a program
        // the table does not know, named once per process.
        let word = known.map_or_else(|| &*word_of(&command).leak(), |k| k.word);
        Editors { auto: Some(Auto { command, source, known, word }), installed }
    }

    /// An `Editors` whose ladder finds `command` in `$EDITOR` and that has
    /// nothing installed: a test's.
    #[cfg(test)]
    pub fn of(command: &str) -> Editors {
        Editors::installing(command, &[])
    }

    /// The same, with these known editors on PATH.
    #[cfg(test)]
    pub fn installing(command: &str, installed: &[&'static str]) -> Editors {
        let installed = installed.iter().map(|w| (*w, w.to_string())).collect();
        Editors::from(None, Some(command), None, None, installed)
    }

    /// The Settings row's stops after `auto`: the known editors this
    /// machine has.
    pub fn installed(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.installed.iter().map(|(w, _)| *w)
    }

    /// What `^g` runs under the row's pick (`None` is `auto`), or `None`
    /// where nothing was found (a test app). A pick this machine does not
    /// have runs by its name, and its failure is the status line's.
    pub fn resolve(&self, pick: Option<&str>) -> Option<Resolved> {
        let auto = self.auto.as_ref()?;
        let picked = pick.and_then(known).map(|k| {
            let command = self
                .installed
                .iter()
                .find(|(w, _)| *w == k.word)
                .map_or_else(|| k.word.to_string(), |(_, c)| c.clone());
            (command, Some(k), k.word)
        });
        let (command, known, word) =
            picked.unwrap_or_else(|| (auto.command.clone(), auto.known, auto.word));
        Some(Resolved { command: with_wait(&command, known), word, known })
    }

    /// Where `auto`'s answer came from.
    pub fn source(&self) -> Option<Source> {
        self.auto.as_ref().map(|a| a.source)
    }
}

impl Resolved {
    /// An editor that opens a window of its own: the board waits for its
    /// tab to close, and a link opens in it detached.
    pub fn is_window(&self) -> bool {
        self.known.is_some_and(|k| k.wait.is_some())
    }

    /// A test's terminal editor, by its command.
    #[cfg(test)]
    pub fn plain(command: &str) -> Resolved {
        Editors::of(command).resolve(None).expect("found")
    }

    /// Git's form: through the shell, the command as `$0` and the file as
    /// `$1`, so an `$EDITOR` with flags or a path with a space in it both
    /// run.
    fn argv(&self, file: &Path) -> Vec<String> {
        shell(&self.command, vec![file.display().to_string()])
    }

    /// A file of the user's own (a link in a note, T-256), at a line where
    /// the editor is known to take one — any other program gets the path
    /// alone rather than a flag it may read as a file name. A window editor
    /// opens it detached and without its wait flag: the board stays up.
    pub fn open(&self, file: &Path, line: Option<u32>) -> Open {
        let path = file.display().to_string();
        let args = match (self.known.map(|k| k.line), line) {
            (Some(Line::Plus), Some(n)) => vec![format!("+{n}"), path],
            (Some(Line::Colon), Some(n)) => vec![format!("{path}:{n}")],
            (Some(Line::Goto), Some(n)) => vec!["-g".into(), format!("{path}:{n}")],
            _ => vec![path],
        };
        if self.is_window() {
            Open::Outside(shell(&without_wait(&self.command), args))
        } else {
            Open::Here(shell(&self.command, args))
        }
    }
}

fn shell(command: &str, args: Vec<String>) -> Vec<String> {
    let mut v = vec!["/bin/sh".into(), "-c".into(), format!("{command} \"$@\""), command.into()];
    v.extend(args);
    v
}

/// The known editors this machine has, in the table's order.
fn installed(
    on_path: impl Fn(&str) -> bool,
    app: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, String)> {
    KNOWN
        .iter()
        .filter_map(|k| {
            if on_path(k.word) {
                Some((k.word, k.word.to_string()))
            } else {
                k.bundle.and_then(&app).map(|c| (k.word, c))
            }
        })
        .collect()
}

/// A window editor's command inside its macOS app, in `/Applications` or
/// `~/Applications`, quoted for the shell: the app is often installed while
/// its command never was put on PATH.
fn mac_app(bundle: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Applications"));
    [Some(PathBuf::from("/Applications")), home]
        .into_iter()
        .flatten()
        .map(|d| d.join(bundle))
        .find(|p| p.is_file())
        .map(|p| format!("'{}'", p.display().to_string().replace('\'', "'\\''")))
}

/// The editor whose built-in terminal the board runs in: they all say
/// `TERM_PROGRAM=vscode` (Zed says `zed`), and macOS names the app that
/// started the process, which tells VS Code from its forks.
fn host(term_program: Option<&str>, bundle: Option<&str>, cursor: bool) -> Option<&'static str> {
    let bundle = bundle.unwrap_or_default().to_ascii_lowercase();
    match term_program? {
        "zed" => Some("zed"),
        "vscode" if cursor || bundle.contains("todesktop") => Some("cursor"),
        "vscode" if bundle.contains("windsurf") => Some("windsurf"),
        "vscode" if bundle.contains("vscodium") => Some("codium"),
        "vscode" => Some("code"),
        _ => None,
    }
}

fn is_wait(token: &str) -> bool {
    matches!(token, "--wait" | "-w")
}

/// A window editor's command with its wait flag: `code` runs as `code
/// --wait`, and a command that already waits is left as it is.
fn with_wait(command: &str, known: Option<&'static Known>) -> String {
    match known.and_then(|k| k.wait) {
        Some(flag) if !command.split_whitespace().any(is_wait) => format!("{command} {flag}"),
        _ => command.to_string(),
    }
}

/// The same command with no wait flag: a link opens and the board goes on.
fn without_wait(command: &str) -> String {
    command.split(' ').filter(|t| !is_wait(t)).collect::<Vec<_>>().join(" ")
}

/// The footer's word for the editor: the command's first word, quotes
/// honoured, as a file name (`/opt/homebrew/bin/nvim` is `nvim`), one line,
/// at most 16 cells.
pub(crate) fn word_of(command: &str) -> String {
    let command = command.trim_start();
    let first = match command.chars().next() {
        Some(q @ ('\'' | '"')) => command[1..].split(q).next().unwrap_or_default(),
        _ => command.split_whitespace().next().unwrap_or_default(),
    };
    let base = Path::new(first).file_name().and_then(|f| f.to_str()).unwrap_or(FALLBACK);
    crate::text::truncate(&crate::text::one_line(base), 16)
}

/// What `mesimon doctor` says: the program, and the pick or the variable
/// that chose it.
pub fn doctor_line(repo: &Path) -> String {
    let editors = Editors::find(repo);
    let prefs = crate::prefs::load_home().prefs;
    let Some(editor) = editors.resolve(prefs.editor) else {
        return "none".into();
    };
    let from = match (prefs.editor, editors.source()) {
        (Some(_), _) => "Settings".to_string(),
        (None, Some(s)) => s.words().to_string(),
        (None, None) => String::new(),
    };
    let waits = if editor.is_window() { ", waits for its tab to close" } else { "" };
    format!("{} ({from}{waits}) — ^g in a note", editor.word)
}

/// Where the files go: `<state>/edit/`, beside the board's other state.
pub fn edit_dir(repo_root: &Path) -> Result<PathBuf> {
    Ok(mesimon_daemon::Paths::for_repo(repo_root)?.state_dir.join("edit"))
}

/// Run outside raw mode / alt screen — the caller restores the terminal
/// first and re-initialises after, exactly as for a focus handover.
pub fn run(req: &ExternalEdit, dir: &Path) -> Result<Outcome> {
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
    let ran = handover::run(&req.editor.argv(&path), None);
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

    fn here(open: Open) -> Vec<String> {
        match open {
            Open::Here(v) => v,
            Open::Outside(v) => panic!("opened outside: {v:?}"),
        }
    }

    fn outside(open: Open) -> Vec<String> {
        match open {
            Open::Outside(v) => v,
            Open::Here(v) => panic!("opened here: {v:?}"),
        }
    }

    /// A terminal editor opens a link here, at its line where it takes
    /// one; a window editor opens it detached, at its line, not waiting.
    #[test]
    fn a_link_opens_at_its_line_where_the_editor_takes_one() {
        let f = Path::new("/repo/src/a.rs");
        let vim = here(Resolved::plain("nvim -u NONE").open(f, Some(42)));
        assert_eq!(&vim[3..], &["nvim -u NONE", "+42", "/repo/src/a.rs"]);
        let hx = here(Resolved::plain("hx").open(f, Some(42)));
        assert_eq!(&hx[3..], &["hx", "/repo/src/a.rs:42"]);
        let other = here(Resolved::plain("ed").open(f, Some(42)));
        assert_eq!(&other[3..], &["ed", "/repo/src/a.rs"], "no flag it may misread");
        assert_eq!(
            here(Resolved::plain("vi").open(f, None)).last().map(String::as_str),
            Some("/repo/src/a.rs")
        );
        let code = outside(Resolved::plain("code --wait").open(f, Some(42)));
        assert_eq!(&code[2..], &["code \"$@\"", "code", "-g", "/repo/src/a.rs:42"]);
        let zed = outside(Resolved::plain("zed").open(f, Some(7)));
        assert_eq!(&zed[2..], &["zed \"$@\"", "zed", "/repo/src/a.rs:7"]);
        let subl = outside(Resolved::plain("subl -w -n").open(f, None));
        assert_eq!(&subl[2..], &["subl -n \"$@\"", "subl -n", "/repo/src/a.rs"]);
    }

    fn found(
        visual: Option<&str>,
        editor: Option<&str>,
        git: Option<&str>,
        host: Option<&'static str>,
        installed: &[&'static str],
    ) -> Editors {
        let installed = Editors::installing("", installed).installed;
        Editors::from(visual, editor, git, host, installed)
    }

    fn auto(e: &Editors) -> (String, Source) {
        (e.resolve(None).expect("found").command, e.source().expect("found"))
    }

    /// T-864: `$VISUAL`, `$EDITOR`, git's `core.editor`, the editor the
    /// terminal is in (where it is installed), then `vi`.
    #[test]
    fn the_ladder_is_visual_editor_git_host_then_vi() {
        let s = |c: &str, src| (c.to_string(), src);
        assert_eq!(
            auto(&found(Some("nvim"), Some("nano"), None, None, &[])),
            s("nvim", Source::Visual)
        );
        assert_eq!(
            auto(&found(Some("  "), Some("nano"), None, None, &[])),
            s("nano", Source::Editor)
        );
        assert_eq!(
            auto(&found(None, None, Some("code --wait"), Some("zed"), &["zed"])),
            s("code --wait", Source::Git)
        );
        assert_eq!(
            auto(&found(None, None, None, Some("code"), &["code"])),
            s("code --wait", Source::Host)
        );
        assert_eq!(auto(&found(None, None, None, Some("code"), &[])), s("vi", Source::Fallback));
        assert_eq!(
            auto(&found(Some(""), Some(""), Some(""), None, &[])),
            s("vi", Source::Fallback)
        );
        assert_eq!(Editors::default().resolve(None), None, "nothing found: inert");
    }

    /// A window editor named without its wait flag gets it; one that
    /// already waits, or a terminal editor, is left as written.
    #[test]
    fn a_window_editor_is_told_to_wait() {
        let cmd = |c: &str| Resolved::plain(c).command;
        assert_eq!(cmd("code"), "code --wait");
        assert_eq!(cmd("code -n"), "code -n --wait");
        assert_eq!(cmd("code -w"), "code -w");
        assert_eq!(cmd("/usr/local/bin/cursor --wait"), "/usr/local/bin/cursor --wait");
        assert_eq!(cmd("subl"), "subl -w");
        assert_eq!(cmd("zed"), "zed --wait");
        assert_eq!(cmd("nvim"), "nvim");
        assert!(Resolved::plain("code").is_window());
        assert!(!Resolved::plain("nvim").is_window());
        // An app's command off PATH, quoted, is still the editor it is.
        let quoted = "'/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code'";
        assert_eq!(Resolved::plain(quoted).word, "code");
        assert_eq!(cmd(quoted), format!("{quoted} --wait"));
    }

    /// The row's pick runs what the machine has under that name, its app's
    /// command included; `auto` is the ladder's.
    #[test]
    fn a_pick_runs_the_installed_editor() {
        let mut e = found(Some("vim"), None, None, None, &["nvim"]);
        e.installed.push(("code", "'/Applications/Code.app/code'".into()));
        assert_eq!(e.installed().collect::<Vec<_>>(), ["nvim", "code"]);
        let code = e.resolve(Some("code")).expect("found");
        assert_eq!(code.command, "'/Applications/Code.app/code' --wait");
        assert_eq!(code.word, "code");
        assert_eq!(e.resolve(Some("zed")).expect("found").command, "zed --wait");
        assert_eq!(e.resolve(Some("kakoune?")).expect("found").command, "vim", "unknown: auto");
        assert_eq!(e.resolve(None).expect("found").word, "vim");
    }

    #[test]
    fn the_installed_list_takes_path_then_the_app() {
        let got = installed(
            |w| matches!(w, "nvim" | "vi"),
            |b| b.starts_with("Zed").then(|| format!("'/Applications/{b}'")),
        );
        assert_eq!(
            got,
            [
                ("zed", "'/Applications/Zed.app/Contents/MacOS/cli'".to_string()),
                ("nvim", "nvim".into()),
                ("vi", "vi".into()),
            ]
        );
    }

    /// VS Code and its forks all say `vscode`; the app macOS names, or
    /// Cursor's own variable, tells them apart.
    #[test]
    fn the_host_editor_is_read_off_the_terminal() {
        assert_eq!(host(Some("vscode"), Some("com.microsoft.VSCode"), false), Some("code"));
        assert_eq!(host(Some("vscode"), None, true), Some("cursor"));
        assert_eq!(
            host(Some("vscode"), Some("com.todesktop.230313mzl4w4u92"), false),
            Some("cursor")
        );
        assert_eq!(host(Some("vscode"), Some("com.exafunction.windsurf"), false), Some("windsurf"));
        assert_eq!(host(Some("zed"), None, false), Some("zed"));
        assert_eq!(host(Some("iTerm.app"), None, false), None);
        assert_eq!(host(None, None, true), None);
    }

    /// The footer's word is the program, not its path or its flags.
    #[test]
    fn the_word_is_the_programs_name() {
        assert_eq!(word_of("nvim"), "nvim");
        assert_eq!(word_of("/opt/homebrew/bin/nvim"), "nvim");
        assert_eq!(word_of("code --wait"), "code");
        assert_eq!(word_of("  "), "vi");
        assert_eq!(word_of("\"/Apps/Sublime Text.app/subl\" -w"), "subl");
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
        let editor = Resolved::plain("sh -c 'printf \"%s\\n\" \"$(cat \"$1\")\" not > \"$1\"' sh");
        let mut req =
            ExternalEdit { text: "# Why\n\nbecause".into(), file_name: "T-1.md".into(), editor };
        let out = run(&req, &d).unwrap();
        assert_eq!(out, Outcome::Changed("# Why\n\nbecause\nnot".into()));
        assert!(!d.join("T-1.md").exists(), "the file is gone after");
        // `true` touches nothing; the trailing newline we wrote is not a change.
        req.editor = Resolved::plain("true");
        assert_eq!(run(&req, &d).unwrap(), Outcome::Unchanged);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An editor that exits non-zero is an error and the body stays what it
    /// was — nothing is read back from a run that failed.
    #[test]
    fn a_failed_editor_is_an_error_and_leaves_no_file() {
        let d = dir("fail");
        let editor = Resolved::plain("false");
        let req = ExternalEdit { text: "keep".into(), file_name: "T-2.md".into(), editor };
        let err = run(&req, &d).unwrap_err();
        assert!(err.to_string().contains("exited"), "{err}");
        assert!(!d.join("T-2.md").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
