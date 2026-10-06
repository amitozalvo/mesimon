//! Saying it out loud (T-282): the two ladders behind `core::notify`.
//!
//! The pure half decides WHAT is said and when; this decides HOW, and it is
//! the only file that knows a platform. Both ladders follow `opener.rs`: an
//! environment variable first (the explicit choice, and the seam that keeps a
//! test from ever making a noise), then the platform's own program, then a
//! rung that always exists. Resolved once per process in `lib.rs` — never in
//! `App::new`, so no test app and no golden reaches a real program.
//!
//! **The banner ladder** is `MESIMON_NOTIFY` (`off`, `osc`, or a program) →
//! `terminal-notifier` → `osascript` on macOS (posted by a Mesimon applet, so
//! the banner wears the mascot, T-605) → `notify-send` on Linux →
//! **OSC 9**, written to our own terminal. The last rung cannot fail to
//! resolve, which is why it is last: on a terminal that draws it (iTerm2,
//! WezTerm, ghostty, kitty) the banner comes from the terminal itself, and on
//! one that does not, nothing happens — that is the rung's known limit and
//! the reason a helper program outranks it. An outer tmux of the user's own
//! swallows it too; the board itself never runs inside mesimon's private
//! server, so there is no DCS wrap to do here.
//!
//! **A banner you can click** (T-293) raises the terminal it came from, and
//! that is one rung's gift only: `terminal-notifier -activate <bundle-id>`
//! names an application to bring forward. `osascript`'s `display
//! notification` can carry no action at all, and `notify-send --action`
//! needs a process that stays alive to read the click on its stdout, which a
//! detached null-stdio launch deliberately is not. So the click belongs to
//! the rung the ladder already prefers, and `doctor` says so on the rung
//! that cannot deliver one rather than leaving it to be discovered.
//! `-sender` is the rival and is refused: it would give the banner the
//! terminal's own icon, but its own README says it cannot be combined with
//! `-activate`, which needs the sender to BE terminal-notifier — and a
//! banner that looks right and does nothing is what this ends.
//!
//! **Which terminal to raise** is a ladder of its own: `MESIMON_TERM_BUNDLE`
//! (`off`, or a bundle id) → an outer tmux VETOES the question →
//! `__CFBundleIdentifier`, which macOS stamps on the app it launches and
//! every child inherits, so it names the id exactly — no table — for kitty,
//! Alacritty, Warp and whatever ships next year → a small `TERM_PROGRAM`
//! table for a process tree that lost it. The veto is the interesting rung.
//! Inside the user's own tmux that inherited `__CFBundleIdentifier` names
//! whatever started the SERVER, not the client attached now, and `-activate`
//! LAUNCHES an application that is not running — so a stale id opens a fresh
//! window of the wrong terminal, which is worse than doing nothing, because
//! a notification is a promise. `TERM_PROGRAM` is rewritten to `tmux` in
//! every pane, which makes it the one reliable negative. `LC_TERMINAL` was
//! considered and refused: it can only answer where both the others are
//! absent, which on macOS is ssh, and there it would raise iTerm2 on the
//! machine nobody is looking at.
//!
//! **Which TAB inside it** is the other half, and without it the first is
//! half a promise (T-301): `-activate` names an application, so the click
//! raised the terminal and left whatever tab happened to be in front of it.
//! No application can answer this — only the terminal can, and it answers in
//! AppleScript — so the tab is a second flag on the same rung, `-execute`,
//! which terminal-notifier runs through `/bin/sh -c` AFTER the activation.
//! Both actions run, in that order: that is terminal-notifier's own source,
//! not an assumption, which is why the two flags are sent together rather
//! than one instead of the other. Two terminals can be asked where a tab is
//! — iTerm2 by the session uuid its dictionary calls `id of session`, which
//! is the tail of `ITERM_SESSION_ID`, and Apple Terminal by a tab's `tty`,
//! which is the one on our own stdin — and `MESIMON_TERM_REVEAL` is `off`
//! or a program of the user's own for every terminal neither fits.
//!
//! Three rules hold that together. **A script talks to the application the
//! click raises**: a `Reveal` names its own bundle id and is dropped when it
//! is not the one `-activate` was given, so a `MESIMON_TERM_BUNDLE` naming
//! some other application cannot leave a script aimed at this one. **It
//! raises a tab and never an application**: every script is wrapped in `if
//! application id … is running`, because `tell application` STARTS what is
//! not running, and a click that opens a fresh empty terminal is the tmux
//! veto's failure by another road. **And the command carries no word from a
//! payload**: it is a constant script plus one id this module validated,
//! single-quoted by `sh_line` — argv's rule kept where argv is not on offer.
//!
//! **`Delivered by: your terminal`** (T-676) is a row, not a rung: it takes
//! the banner off the ladder altogether and hands it to the terminal the
//! board runs in, by that terminal's own escape ([`Poster`]) — iTerm2's
//! OSC 9, kitty's OSC 99, WezTerm's and Ghostty's OSC 777 — so the banner is
//! posted under the terminal's signature and no program of ours is launched.
//! A terminal with no escape shows nothing: falling back to the helper would
//! launch exactly what the row was chosen to keep from launching.
//! `MESIMON_NOTIFY=off` still silences it; the sound ladder is untouched.
//!
//! **The sound ladder** is `MESIMON_SOUND` (`off` or a program) → `afplay` on
//! macOS → `paplay` / `pw-play` / `canberra-gtk-play` on Linux → the terminal
//! bell, which is the rung nothing can take away.
//!
//! **The two escape rungs write to OUR stdout, and since T-291 they say so.**
//! Every other rung spawns a program and is therefore indifferent to what the
//! terminal is doing; OSC 9 and the bell are writes into the same stream
//! ratatui draws on, from a thread of their own ([`crate::notifier`]). So
//! both go through [`Console`], which holds the two facts only the main loop
//! knows: whether the board still HOLDS the terminal (a handover gives it to
//! tmux or an editor for the whole life of the child) and whether a frame is
//! being written right now. That is the honest limit T-291 records — while
//! the board is off screen the escape rungs say nothing at all, which is why
//! the ladders prefer a helper program and why a machine that has one loses
//! nothing.
//!
//! Two rules hold across both, and both are load-bearing:
//!
//! - **the text rides argv, never a program's source.** A ticket key is ours,
//!   but a board's directory name is the user's, and `osascript` takes a
//!   PROGRAM. So the script is a constant with `on run argv` and the words go
//!   past it as arguments — `workspace.rs`'s "argv arrays always" rule.
//! - **everything crosses `text::scrub_text` first**, the boundary function
//!   for text leaving for another process. It is also what makes the OSC rung
//!   safe: the ESC and BEL it strips are exactly what would otherwise close
//!   the sequence early.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use mesimon_core::notify::{Post, Sound};
use mesimon_core::text;

use crate::opener::launch;

/// A notification field is one line on somebody's screen. All three are
/// bounded here rather than trusted: the title is a directory name, the
/// subtitle carries a ticket's own title and the body can carry an agent's
/// own sentence. Wide enough that a raised hand's full
/// `board::RAISE_REASON_MAX_BYTES` (160) still arrives with `needs you ∙ ` in
/// front of it — the words are why the banner was worth sending.
///
/// It is the BACKSTOP, not the shape: `core::notify` already clips a title
/// and a reply on a word boundary with an ellipsis, because those two are
/// read by a person. This one is a byte cap on what leaves the process, and
/// the folded line a one-field rung gets is bounded by construction, being
/// two already-capped fields with a separator between them.
const MAX_FIELD: usize = 240;

/// Where the freedesktop sound theme keeps the three events six names
/// collapse to (`core::notify::Sound::event_freedesktop`).
const FREEDESKTOP: &str = "/usr/share/sounds/freedesktop/stereo";

/// Which TAB a click should land in (T-301), for the terminals that can say.
///
/// The string in each variant is that terminal's own name for the board's
/// session, validated where it was read and never repaired: an id with a
/// character dropped names a DIFFERENT tab, which is the very failure this
/// rung exists to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reveal {
    /// iTerm2, by the session uuid its dictionary calls `id of session` —
    /// the tail of `ITERM_SESSION_ID`, which every session is born with and
    /// which outlives the `w0t1p0` coordinates in front of it.
    ITerm2(String),
    /// Apple Terminal, by a tab's `tty`. Its dictionary knows no session id
    /// at all, but every tab knows its tty and ours is the one on our stdin.
    AppleTerminal(String),
    /// `MESIMON_TERM_REVEAL=<program>`, run with no arguments — the escape
    /// hatch for a terminal with a remote control of its own (`kitty @
    /// focus-window`, `wezterm cli activate-pane`), which knows which window
    /// it means far better than a table here ever could.
    Custom(String),
}

/// The two applications a reveal can script, spelled the way `-activate`
/// spells them. Written once because the script and the pairing rule have to
/// agree, and a second spelling is how they would come apart.
const ITERM2_ID: &str = "com.googlecode.iterm2";
const TERMINAL_ID: &str = "com.apple.Terminal";

/// Select the session `argv` names. `select` is iTerm2's own verb for "make
/// receiver visible and selected" and all three receivers are needed: the
/// window may not be the front one, the tab may not be the front tab, and
/// the session may be one pane of a split.
///
/// The `is running` guard is the load-bearing line, not a politeness. `tell
/// application` starts what is not running, so without it a click on a
/// banner that outlived its terminal would LAUNCH iTerm2 and open an empty
/// window — a click that lies, which is what the tmux veto exists to
/// prevent by its own road. Asking whether an application is running starts
/// nothing, so the guard is also what makes the `activate` inside it safe.
///
/// That `activate` is the script's, not a duplicate of `-activate`'s: it
/// makes the reveal whole on its own rather than depending on the
/// activation ahead of it having landed. It is Apple Terminal that measures
/// this — that one reorders its windows only while it is the ACTIVE
/// application, and `set frontmost` in a background one returns success and
/// does nothing — but both scripts say it, because one of them relying on
/// the timing of another process is not a thing to leave to chance.
const ITERM2_SCRIPT: &str = "\
on run argv
if application id \"com.googlecode.iterm2\" is running then
tell application id \"com.googlecode.iterm2\"
activate
repeat with w in windows
repeat with t in tabs of w
repeat with s in sessions of t
if id of s is (item 1 of argv) then
select w
select t
select s
return
end if
end repeat
end repeat
end repeat
end tell
end if
end run";

/// The same thing in Apple Terminal's own dictionary, which has no session
/// and no `select`: a tab is `selected`, a window is `frontmost`, and both
/// are properties rather than verbs.
const TERMINAL_SCRIPT: &str = "\
on run argv
if application id \"com.apple.Terminal\" is running then
tell application id \"com.apple.Terminal\"
activate
repeat with w in windows
repeat with t in tabs of w
if tty of t is (item 1 of argv) then
set selected of t to true
set frontmost of w to true
return
end if
end repeat
end repeat
end tell
end if
end run";

impl Reveal {
    /// The application this script talks to, or `None` where the user's own
    /// program does — which is what pairs a reveal to the click carrying it.
    fn app(&self) -> Option<&'static str> {
        match self {
            Reveal::ITerm2(_) => Some(ITERM2_ID),
            Reveal::AppleTerminal(_) => Some(TERMINAL_ID),
            Reveal::Custom(_) => None,
        }
    }

    /// The `/bin/sh -c` line terminal-notifier runs on a click, or `None`
    /// where a word could not be quoted — refused, never escaped.
    fn command(&self) -> Option<String> {
        match self {
            Reveal::ITerm2(id) => sh_line(&["osascript", "-e", ITERM2_SCRIPT, id]),
            Reveal::AppleTerminal(tty) => sh_line(&["osascript", "-e", TERMINAL_SCRIPT, tty]),
            Reveal::Custom(prog) => sh_line(&[prog]),
        }
    }

    /// What `doctor` calls this rung, said as the tail of a sentence about
    /// the application — the tab is not a thing of its own to a reader.
    fn word(&self) -> String {
        match self {
            Reveal::ITerm2(_) | Reveal::AppleTerminal(_) => "and this board's own tab".into(),
            Reveal::Custom(p) => format!("and runs {p} ($MESIMON_TERM_REVEAL)"),
        }
    }
}

/// What a click does: an application to bring forward, and — where the
/// terminal can be asked — the tab the board is actually drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Click {
    /// The bundle id `-activate` raises.
    pub app: String,
    /// How to reach the board's own tab inside it. `None` is what T-293
    /// shipped: the application comes forward showing whatever was in front.
    pub reveal: Option<Reveal>,
}

impl Click {
    /// The `-execute` command, if there is one. Built before the argv so
    /// that vector of borrows has something to borrow.
    fn command(&self) -> Option<String> {
        self.reveal.as_ref().and_then(Reveal::command)
    }
}

/// Who draws the banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// `MESIMON_NOTIFY=off`. The sound, if any, still plays.
    Off,
    /// OSC 9 to our own stdout — the terminal draws it, or nobody does.
    Osc,
    /// kitty's own notification (OSC 99), which a click brings the window
    /// forward for — no helper program needed. Chosen over the two
    /// programs that cannot click, below terminal-notifier, and only
    /// where the board runs in kitty directly (`KITTY_WINDOW_ID`, and no
    /// outer tmux, which swallows it).
    Kitty,
    /// `MESIMON_NOTIFY=<program>`, called `<program> <title> <body>`. It is
    /// NOT handed the click's bundle id: a fourth argv word would silently
    /// change what `$3` means to a program written against T-282's shape,
    /// and `opener::launch` clears no environment, so one that wants the id
    /// reads `MESIMON_TERM_BUNDLE` or `__CFBundleIdentifier` for itself.
    Custom(String),
    TerminalNotifier,
    Osascript,
    NotifySend,
}

impl Banner {
    /// The argv, or `None` where the rung writes an escape instead.
    ///
    /// Two rungs have three fields and take the subtitle on its own (T-292);
    /// the rest are handed it folded into the body, so a user's own program
    /// keeps the two arguments it was written against. An EMPTY subtitle —
    /// what an aggregate produces, having no one ticket to name — takes the
    /// argv that shipped before, byte for byte.
    ///
    /// `click` is what a click should do (T-293, T-301) and reaches exactly
    /// one rung, for the same reason `group` does: it is the only one that
    /// has the concept.
    fn argv(&self, group: Option<&str>, click: Option<&Click>, p: &Fields) -> Option<Vec<String>> {
        let v = |args: &[&str]| Some(args.iter().map(|s| (*s).to_string()).collect());
        let (title, subtitle, body) = (p.title.as_str(), p.subtitle.as_str(), p.folded.as_str());
        // Built up here because the argv below is a vector of borrows and a
        // reveal's command is made on the spot.
        let reveal = click.and_then(Click::command);
        match self {
            Banner::Off | Banner::Osc | Banner::Kitty => None,
            Banner::Custom(prog) => v(&[prog, title, body]),
            Banner::TerminalNotifier => {
                let mut argv: Vec<&str> = vec!["terminal-notifier", "-title", title];
                if !p.subtitle.is_empty() {
                    argv.extend(["-subtitle", subtitle]);
                }
                argv.extend(["-message", &p.body]);
                // The click (T-293): bring this terminal forward. Only ever
                // an id this module resolved, never a word from a payload.
                if let Some(c) = click {
                    argv.extend(["-activate", &c.app]);
                    // And the tab inside it (T-301). Second, and never
                    // instead: terminal-notifier runs both in this order, so
                    // the application is already forward when the script
                    // picks this board's own tab out of it — and if the
                    // script is refused permission to script the terminal,
                    // what is left is exactly the click that shipped before.
                    if let Some(cmd) = reveal.as_deref() {
                        argv.extend(["-execute", cmd]);
                    }
                }
                // One group per BOARD, so a new banner replaces the last one
                // rather than stacking a column of them in Notification
                // Centre — the coalescing rule the module already follows,
                // extended into the OS for one argument. Per board and not
                // per ticket: two boards open at once are two conversations.
                if let Some(id) = group {
                    argv.extend(["-group", id]);
                }
                v(&argv)
            }
            // The words are `argv`, the script is a constant. `item 1` is the
            // title, `item 2` the subtitle and `item 3` the body, so none of
            // them can be read as code however they are spelled. Which is
            // also why no rung may grow a FLAG here: the words are found by
            // position, so one inserted argument silently shifts all three.
            Banner::Osascript if !subtitle.is_empty() => v(&[
                "osascript",
                "-e",
                "on run argv",
                "-e",
                "display notification (item 3 of argv) with title (item 1 of argv) \
                 subtitle (item 2 of argv)",
                "-e",
                "end run",
                title,
                subtitle,
                &p.body,
            ]),
            Banner::Osascript => v(&[
                "osascript",
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                title,
                body,
            ]),
            // `--` so a title that begins with a dash is a title. Every
            // notify-send since 0.7 takes it.
            Banner::NotifySend => v(&["notify-send", "--", title, body]),
        }
    }

    /// What `doctor` calls this rung.
    pub fn word(&self) -> String {
        match self {
            Banner::Off => "off".into(),
            Banner::Osc => "OSC 9 (your terminal draws it)".into(),
            Banner::Kitty => "kitty (OSC 99, a click raises the window)".into(),
            Banner::Custom(p) => format!("{p} ($MESIMON_NOTIFY)"),
            Banner::TerminalNotifier => "terminal-notifier".into(),
            Banner::Osascript => "osascript".into(),
            Banner::NotifySend => "notify-send".into(),
        }
    }
}

/// Who makes the sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Player {
    /// `MESIMON_SOUND=off`. The banner, if any, still shows.
    Off,
    /// The terminal bell — the rung nothing can take away, and the one every
    /// other rung falls back to when its file is missing.
    Bell,
    /// `MESIMON_SOUND=<program>`, called `<program> <sound>` with the name.
    Custom(String),
    Afplay,
    Paplay,
    PwPlay,
    Canberra,
}

impl Player {
    /// The argv, or `None` to fall back to the bell — which is what a rung
    /// whose file is not on this machine does.
    fn argv(&self, sound: Sound) -> Option<Vec<String>> {
        if sound.is_off() {
            return None;
        }
        let file = |name: &str| {
            let p = PathBuf::from(FREEDESKTOP).join(format!("{name}.oga"));
            std::fs::metadata(&p).is_ok().then(|| p.to_string_lossy().into_owned())
        };
        let v = |args: Vec<String>| Some(args);
        match self {
            Player::Off | Player::Bell => None,
            Player::Custom(prog) => v(vec![prog.clone(), sound.name().to_string()]),
            Player::Afplay => {
                let f = sound.file_macos()?;
                std::fs::metadata(&f).is_ok().then(|| vec!["afplay".to_string(), f])
            }
            Player::Paplay => v(vec!["paplay".into(), file(sound.event_freedesktop()?)?]),
            Player::PwPlay => v(vec!["pw-play".into(), file(sound.event_freedesktop()?)?]),
            // The theme-aware one: it takes the event, not a path, so it
            // needs no file check of ours.
            Player::Canberra => {
                v(vec!["canberra-gtk-play".into(), "-i".into(), sound.event_freedesktop()?.into()])
            }
        }
    }

    pub fn word(&self) -> String {
        match self {
            Player::Off => "off".into(),
            Player::Bell => "the terminal bell".into(),
            Player::Custom(p) => format!("{p} ($MESIMON_SOUND)"),
            Player::Afplay => "afplay".into(),
            Player::Paplay => "paplay".into(),
            Player::PwPlay => "pw-play".into(),
            Player::Canberra => "canberra-gtk-play".into(),
        }
    }
}

/// The escape a terminal posts a system notification from (T-676), for the
/// `Delivered by: your terminal` row. The terminal draws the banner under
/// its own signature; nothing of ours is launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poster {
    /// iTerm2: `OSC 9 ; <text> BEL`, one text.
    Osc9,
    /// kitty: `OSC 99`, a title chunk and then a body chunk under one id.
    Osc99,
    /// WezTerm and Ghostty: `OSC 777 ; notify ; <title> ; <body> ST`.
    Osc777,
}

impl Poster {
    /// The bytes to write, from fields already scrubbed (`Fields::of`), so
    /// no ESC, BEL or newline can close the sequence early. `id` ties
    /// kitty's two chunks into one notification.
    fn escape(self, f: &Fields, id: u32) -> String {
        match self {
            Poster::Osc9 => osc9(&f.title, &f.folded),
            Poster::Osc99 => format!(
                "\x1b]99;i={id}:d=0;{}\x1b\\\x1b]99;i={id}:p=body;{}\x1b\\",
                f.title, f.folded
            ),
            // The title ends at the first `;`, so a board named with one
            // would push the rest into the body.
            Poster::Osc777 => {
                format!("\x1b]777;notify;{};{}\x1b\\", f.title.replace(';', ","), f.folded)
            }
        }
    }

    /// What `doctor` calls it.
    fn word(self) -> &'static str {
        match self {
            Poster::Osc9 => "iTerm2 (OSC 9)",
            Poster::Osc99 => "kitty (OSC 99)",
            Poster::Osc777 => "OSC 777",
        }
    }
}

/// How one post goes out, from the notification preferences: whether the
/// dock bounces (iTerm2, T-492) and whether the terminal posts the banner
/// rather than the ladder (T-676).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Delivery {
    pub bounce: bool,
    pub by_terminal: bool,
}

/// Both rungs, resolved once. `lib.rs` parks this on `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channels {
    pub banner: Banner,
    pub player: Player,
    /// This board's own notification group (T-292), for the one rung that
    /// has the concept. `None` where the repo could not be identified, which
    /// only costs the replacing.
    pub group: Option<String>,
    /// What a click does (T-293, T-301), for the one rung that can carry a
    /// click at all. `None` means the banner behaves as it did before both:
    /// it appears, and clicking it does nothing.
    pub click: Option<Click>,
    /// The shared notification state directory. Assets are written only
    /// by an actual image-capable post, never by discovery or doctor.
    pub icon_dir: Option<PathBuf>,
    /// The installed helper bundle, discovered without running it. An actual
    /// macOS post prepares a private signed Mesimon copy under `icon_dir`.
    pub notifier_app: Option<PathBuf>,
    /// The escape this terminal posts a banner from, for the `your
    /// terminal` road (T-676). `None` where it cannot post one.
    pub poster: Option<Poster>,
}

/// The two ladders. The group is not resolved here — this function knows no
/// repo — and [`Notifier::start`](crate::notifier::Notifier::start) fills it
/// in beside the board's own name, the other per-board constant.
pub fn find() -> Channels {
    Channels {
        banner: find_banner(
            std::env::var("MESIMON_NOTIFY").ok().as_deref(),
            which_on_path,
            in_kitty(
                std::env::var("KITTY_WINDOW_ID").ok().as_deref(),
                std::env::var("TERM_PROGRAM").ok().as_deref(),
            ),
        ),
        player: find_player(std::env::var("MESIMON_SOUND").ok().as_deref(), which_on_path),
        group: None,
        icon_dir: None,
        notifier_app: which_on_path("terminal-notifier")
            .and_then(|path| crate::notification_app::discover(&path)),
        poster: crate::title::terminal().poster(),
        click: find_click(
            std::env::var("MESIMON_TERM_BUNDLE").ok().as_deref(),
            std::env::var("MESIMON_TERM_REVEAL").ok().as_deref(),
            std::env::var("__CFBundleIdentifier").ok().as_deref(),
            std::env::var("TERM_PROGRAM").ok().as_deref(),
            std::env::var("ITERM_SESSION_ID").ok().as_deref(),
            own_tty().as_deref(),
        ),
    }
}

/// Add presentation arguments only to backends that support them. Keep
/// custom helpers' two-argument contract and osascript's positional fields.
fn with_icon(banner: &Banner, mut argv: Vec<String>, path: &std::path::Path) -> Vec<String> {
    let flag = match banner {
        Banner::TerminalNotifier => "-contentImage",
        Banner::NotifySend => "--icon",
        _ => return argv,
    };
    // Insert before notify-send's `--`, never after its user-controlled
    // fields. argv stays an array; paths with spaces are one argument.
    argv.splice(1..1, [flag.to_string(), path.to_string_lossy().into_owned()]);
    argv
}

/// This board's notification group: one per REPO, keyed the way every other
/// per-repo thing is (D33b's `proj16`, a hash of the canonical path), so two
/// checkouts of the same project are two groups and a moved directory is a
/// new one — which is the same answer the sockets and the state dir give.
pub fn group_for(repo_root: &std::path::Path) -> Option<String> {
    let paths = mesimon_daemon::Paths::for_repo(repo_root).ok()?;
    Some(format!("mesimon-{}", paths.proj16))
}

/// What a `TERM_PROGRAM` is worth when macOS itself did not say. Every entry
/// has to be VERIFIED before it is added — `osascript -e 'id of app "…"'`, or
/// the app's own `Info.plist` — never written from memory: a wrong id is not
/// dangerous (macOS activates nothing) but it makes `doctor` promise a click
/// that cannot happen, and a confident wrong answer is worse than a blank.
/// The table is deliberately short; `__CFBundleIdentifier` above it already
/// answers for any terminal launched the ordinary way, and
/// `MESIMON_TERM_BUNDLE` answers for everything else.
const TERM_BUNDLES: &[(&str, &str)] =
    &[("iTerm.app", "com.googlecode.iterm2"), ("Apple_Terminal", "com.apple.Terminal")];

/// A bundle id is at most this many bytes. Real ones are reverse-DNS and far
/// shorter; this only stops an absurd value reaching a command line.
const BUNDLE_ID_MAX: usize = 128;

/// Which application a click should raise, or `None` for no click action.
///
/// Pure, and every input is a parameter, so the ladder is the same on every
/// platform a test runs on. The rungs, and why they are in this order, are in
/// the module doc; the one that is not obvious is the tmux veto, which sits
/// ABOVE `__CFBundleIdentifier` because inside an outer tmux that variable is
/// inherited and stale, and a stale id makes `-activate` launch the wrong
/// terminal rather than raise the right one.
fn find_activate(
    env: Option<&str>,
    cf: Option<&str>,
    term_program: Option<&str>,
) -> Option<String> {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return None,
        Some(v) => return bundle_id(v),
        None => {}
    }
    // An outer tmux rewrites this in every pane, so it is the one thing here
    // that cannot be stale — and what it says is "you cannot see the real
    // terminal from in here".
    if term_program.is_some_and(|v| v.eq_ignore_ascii_case("tmux")) {
        return None;
    }
    if let Some(id) = cf.and_then(bundle_id) {
        return Some(id);
    }
    let name = term_program?;
    TERM_BUNDLES.iter().find(|(k, _)| *k == name).and_then(|(_, id)| bundle_id(id))
}

/// A bundle id, or nothing. This boundary REJECTS where [`field`] scrubs, and
/// the difference is the point: dropping a character from a bundle id yields
/// a different, possibly real one, so a silent repair here would raise the
/// wrong application. A leading `-` is refused for a second reason — it would
/// be read as another flag by terminal-notifier's own argument parsing, and
/// `-activate -sound` is a flag with no value.
fn bundle_id(raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() || v.len() > BUNDLE_ID_MAX {
        return None;
    }
    if !v.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    v.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .then(|| v.to_string())
}

/// What a click will do, or `None` for a banner that does nothing when it
/// is clicked.
///
/// The application is settled first because the tab is inside it: a reveal
/// scripting a DIFFERENT application than the one being raised is dropped
/// rather than sent, which is what stops an explicit `MESIMON_TERM_BUNDLE`
/// from leaving a script aimed at the terminal we merely happen to be in.
/// The user's own program is exempt, naming no application to disagree with.
fn find_click(
    bundle_env: Option<&str>,
    reveal_env: Option<&str>,
    cf: Option<&str>,
    term_program: Option<&str>,
    iterm_session: Option<&str>,
    tty: Option<&str>,
) -> Option<Click> {
    let app = find_activate(bundle_env, cf, term_program)?;
    let reveal = find_reveal(reveal_env, term_program, iterm_session, tty)
        .filter(|r| r.app().is_none_or(|id| id == app));
    Some(Click { app, reveal })
}

/// Which tab, and how to reach it. Pure, like the ladder above it, and the
/// rungs are the module doc's.
///
/// The tmux veto needs no rung of its own here: an outer tmux rewrites
/// `TERM_PROGRAM` in every pane, so the table below simply does not answer —
/// which is the right answer, because in there `ITERM_SESSION_ID` is
/// INHERITED from whatever started the server and names a session that is
/// not this one. Selecting the wrong tab would be the bug, not the fix.
fn find_reveal(
    env: Option<&str>,
    term_program: Option<&str>,
    iterm_session: Option<&str>,
    tty: Option<&str>,
) -> Option<Reveal> {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return None,
        Some(v) => return sh_word(v).map(Reveal::Custom),
        None => {}
    }
    match term_program? {
        "iTerm.app" => session_uuid(iterm_session?).map(Reveal::ITerm2),
        "Apple_Terminal" => tty_path(tty?).map(Reveal::AppleTerminal),
        _ => None,
    }
}

/// A session id, a tty or a program name is at most this many bytes. Real
/// ones are far shorter — a uuid is 36 and a tty is a dozen — and this only
/// stops an absurd value reaching a command line.
const REVEAL_MAX: usize = 128;

/// The uuid out of `ITERM_SESSION_ID`, which is spelled `w0t1p0:<uuid>`: a
/// pane's coordinates, then the id the dictionary answers to. REJECTS where
/// [`field`] scrubs, for `bundle_id`'s reason — a repaired id is a different
/// tab, and landing in one is the bug being fixed here.
fn session_uuid(raw: &str) -> Option<String> {
    let v = raw.trim().rsplit(':').next()?.trim();
    if v.is_empty() || v.len() > REVEAL_MAX || !v.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-').then(|| v.to_string())
}

/// The tty of our own tab, as Apple Terminal spells a tab's: an absolute
/// path under `/dev`, and nothing else is one.
fn tty_path(raw: &str) -> Option<String> {
    let v = raw.trim();
    if !v.starts_with("/dev/") || v.len() > REVEAL_MAX {
        return None;
    }
    v.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
        .then(|| v.to_string())
}

/// A program named by `MESIMON_TERM_REVEAL`. A PROGRAM, as `MESIMON_NOTIFY`,
/// `MESIMON_SOUND` and `MESIMON_OPEN` all mean one: whitespace is refused
/// rather than split, because this word is quoted whole and a command line
/// quoted whole is one program name with spaces in it, which would never run
/// and would never say why. Arguments belong inside a script the variable
/// points at, where the user's own shell rules apply.
fn sh_word(raw: &str) -> Option<String> {
    let v = raw.trim();
    (!v.is_empty()
        && v.len() <= REVEAL_MAX
        && !v.contains('\'')
        && !v.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then(|| v.to_string())
}

/// One `/bin/sh -c` line, every word single-quoted.
///
/// The only place mesimon builds a shell command, and it exists because
/// `-execute` takes a command where every other rung takes argv. So it keeps
/// argv's rule by construction: inside single quotes `sh` reads every byte
/// literally, `'` is the one byte that can end the quoting, and a word
/// holding one is REFUSED rather than escaped — `bundle_id`'s answer, for
/// `bundle_id`'s reason. Newlines are ordinary bytes in there, which is what
/// lets a whole AppleScript ride one word.
fn sh_line(words: &[&str]) -> Option<String> {
    let mut out = String::new();
    for w in words {
        if w.is_empty() || w.contains('\'') || w.contains('\0') {
            return None;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('\'');
        out.push_str(w);
        out.push('\'');
    }
    (!out.is_empty()).then_some(out)
}

/// The tty the board is drawn on, which is the tab Apple Terminal knows it
/// by. Asked once per process from `find`, on the main thread, which is what
/// makes `ttyname`'s static buffer safe to read.
fn own_tty() -> Option<String> {
    // SAFETY: `ttyname` returns NULL or a pointer to static storage valid
    // until the next call in this thread, and the string is copied here
    // before anything else can make one.
    let p = unsafe { libc::ttyname(0) };
    if p.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(p) };
    s.to_str().ok().map(str::to_string)
}

fn find_banner(env: Option<&str>, which: impl Fn(&str) -> Option<PathBuf>, kitty: bool) -> Banner {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return Banner::Off,
        Some(v) if v.eq_ignore_ascii_case("osc") => return Banner::Osc,
        Some(v) if v.eq_ignore_ascii_case("kitty") => return Banner::Kitty,
        Some(v) => return Banner::Custom(v.to_string()),
        None => {}
    }
    if which("terminal-notifier").is_some() {
        return Banner::TerminalNotifier;
    }
    if kitty {
        return Banner::Kitty;
    }
    if cfg!(target_os = "macos") {
        return Banner::Osascript;
    }
    if which("notify-send").is_some() {
        return Banner::NotifySend;
    }
    Banner::Osc
}

/// The board runs in kitty, directly: kitty stamps every pane with its
/// window id, and an outer tmux — which would swallow OSC 99 — rewrites
/// `TERM_PROGRAM` to `tmux`, the same negative `find_activate` trusts.
fn in_kitty(window_id: Option<&str>, term_program: Option<&str>) -> bool {
    window_id.is_some_and(|v| !v.is_empty())
        && !term_program.is_some_and(|v| v.eq_ignore_ascii_case("tmux"))
}

fn find_player(env: Option<&str>, which: impl Fn(&str) -> Option<PathBuf>) -> Player {
    match env.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) if v.eq_ignore_ascii_case("off") => return Player::Off,
        Some(v) => return Player::Custom(v.to_string()),
        None => {}
    }
    if cfg!(target_os = "macos") {
        return Player::Afplay;
    }
    for (name, rung) in [
        ("paplay", Player::Paplay),
        ("pw-play", Player::PwPlay),
        ("canberra-gtk-play", Player::Canberra),
    ] {
        if which(name).is_some() {
            return rung;
        }
    }
    Player::Bell
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| std::fs::metadata(cand).is_ok_and(|m| m.is_file()))
}

/// The board's own terminal, shared with the notification thread (T-291).
///
/// One lock over one fact: does the board hold the terminal? A draw takes it
/// to draw, an escape-writing rung takes it to write, and a handover takes it
/// to change hands — so an escape can neither land inside a frame nor reach a
/// terminal that now belongs to tmux or to the user's editor.
///
/// A handover is the whole reason this exists, and it is also why the answer
/// there is "say nothing" rather than "wait": a `!` shell or an attached pane
/// lives for as long as the user wants it to, and a banner held for twenty
/// minutes is worse than one never raised.
#[derive(Debug)]
pub struct Console {
    held: Mutex<bool>,
}

impl Default for Console {
    /// A board that has never handed its terminal over holds it.
    fn default() -> Self {
        Console { held: Mutex::new(true) }
    }
}

impl Console {
    /// A poisoned lock is a panic somewhere else, not a reason to lose the
    /// terminal: step over it the way `prefs` steps over its own.
    fn lock(&self) -> MutexGuard<'_, bool> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Held for the length of one draw. The main loop takes this around
    /// `terminal.draw`, which is what keeps a banner out of a frame.
    pub fn drawing(&self) -> MutexGuard<'_, bool> {
        self.lock()
    }

    /// The board is giving the terminal away, or taking it back. Taken under
    /// the same lock, so a rung already inside a write finishes before the
    /// terminal changes hands.
    pub fn set_held(&self, held: bool) {
        *self.lock() = held;
    }

    /// Write, but only while the terminal is ours.
    fn write_if_held(&self, f: impl FnOnce() -> std::io::Result<()>) -> std::io::Result<()> {
        let held = self.lock();
        if !*held {
            return Ok(());
        }
        f()
    }
}

/// Say it. A banner with no words is a sound-only post — the focus rule's
/// output, and a Settings row's preview.
///
/// Errors are the spawn's own (no such program) and the terminal's; whether
/// the notification was actually SEEN is not ours to know, the same reason
/// the board says `opening …` and never `opened`. A rung that writes an
/// escape and finds the terminal handed over says nothing and reports no
/// error: not raising a banner is not a failure to report.
pub fn post(ch: &Channels, p: &Post, console: &Console, how: Delivery) -> std::io::Result<()> {
    post_with(ch, p, console, how, |argv| launch(argv, None), write_stdout)
}

/// kitty's notification id (T-676): one per banner, so a new one never
/// lands on top of the chunks of the last.
static KITTY_ID: AtomicU32 = AtomicU32::new(1);

fn post_with(
    ch: &Channels,
    p: &Post,
    console: &Console,
    how: Delivery,
    launch: impl Fn(&[String]) -> std::io::Result<()>,
    emit: impl Fn(&str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut icon_error = None;
    if !p.body.is_empty() && how.by_terminal && ch.banner != Banner::Off {
        // The terminal posts it, or nobody does (T-676): no helper is built
        // or launched on this road, a terminal without an escape included.
        if let Some(poster) = ch.poster {
            let f = Fields::of(p);
            let id = KITTY_ID.fetch_add(1, Ordering::Relaxed);
            console.write_if_held(|| emit(&poster.escape(&f, id)))?;
        }
    } else if !p.body.is_empty() {
        let f = Fields::of(p);
        match ch.banner.argv(ch.group.as_deref(), ch.click.as_ref(), &f) {
            Some(mut argv) => {
                if let Some(dir) = ch.icon_dir.as_ref() {
                    let mut branded = false;
                    if cfg!(target_os = "macos") && ch.banner == Banner::TerminalNotifier {
                        if let Some(source) = ch.notifier_app.as_ref() {
                            match crate::notification_app::prepare(dir, source) {
                                Ok(program) => {
                                    argv[0] = program.to_string_lossy().into_owned();
                                    branded = true;
                                }
                                Err(e) => icon_error = Some(io_error_without_icon(e)),
                            }
                        }
                    }
                    // No terminal-notifier: `osascript`'s banner would wear
                    // Script Editor's icon (T-605). The same words go to a
                    // Mesimon applet; a failed build keeps `osascript`.
                    if cfg!(target_os = "macos") && ch.banner == Banner::Osascript {
                        match crate::notification_app::prepare_applet(dir) {
                            Ok(program) => argv = applet_argv(&program, &f),
                            Err(e) => icon_error = Some(io_error_without_icon(e)),
                        }
                    }
                    // The app icon is stable. Only an attention post needs
                    // an extra amber image; Linux and unbranded helpers keep
                    // the per-post PNG on both kinds of banner.
                    if matches!(ch.banner, Banner::TerminalNotifier | Banner::NotifySend)
                        && (!branded || p.needs_you)
                    {
                        match crate::mascot::icon(dir, p.needs_you) {
                            Ok(path) => argv = with_icon(&ch.banner, argv, &path),
                            Err(e) => icon_error = Some(io_error_without_icon(e)),
                        }
                    }
                }
                launch(&argv)?;
            }
            None if ch.banner == Banner::Osc => {
                console.write_if_held(|| emit(&osc9(&f.title, &f.folded)))?
            }
            None if ch.banner == Banner::Kitty => {
                console.write_if_held(|| emit(&osc99(&f.title, &f.folded)))?
            }
            None => {}
        }
    }
    // iTerm2's dock bounce (T-492): once, on a post that carries an
    // attention event. The same lock and the same silence off screen as
    // the escape rungs; every other terminal ignores the sequence, and
    // `bounce` is already false anywhere but iTerm2 direct.
    if how.bounce && p.needs_you {
        console.write_if_held(|| emit(BOUNCE))?;
    }
    if !p.sound.is_off() {
        match ch.player.argv(p.sound) {
            Some(argv) => launch(&argv)?,
            None if ch.player == Player::Off => {}
            None => console.write_if_held(|| emit(BELL))?,
        }
    }
    match icon_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// The applet's argv: the `osascript` rung's words in the same order, each
/// behind the `=` that `notification_app::APPLET_SCRIPT` strips.
fn applet_argv(program: &std::path::Path, f: &Fields) -> Vec<String> {
    let mut argv = vec![program.to_string_lossy().into_owned(), format!("={}", f.title)];
    if f.subtitle.is_empty() {
        argv.push(format!("={}", f.folded));
    } else {
        argv.push(format!("={}", f.subtitle));
        argv.push(format!("={}", f.body));
    }
    argv
}

fn io_error_without_icon(error: std::io::Error) -> std::io::Error {
    std::io::Error::other(format!(
        "Mesimon app icon unavailable; using the helper's banner: {error}"
    ))
}

/// A post's three fields, each safe to hand to another process or to write
/// inside an escape sequence — plus the one line a rung with a single field
/// gets. Built once per post so the scrubbing happens once and the two
/// shapes cannot disagree about what was said.
struct Fields {
    title: String,
    subtitle: String,
    body: String,
    /// Subtitle and body as one line. Folded from the SCRUBBED halves, so it
    /// is bounded by construction and needs no cap of its own.
    folded: String,
}

impl Fields {
    fn of(p: &Post) -> Fields {
        let scrubbed = Post {
            needs_you: p.needs_you,
            title: field(&p.title),
            subtitle: field(&p.subtitle),
            body: field(&p.body),
            sound: p.sound,
        };
        // The wording of the fold belongs to the module that owns mesimon's
        // voice, so it is asked for rather than spelled again here.
        let folded = scrubbed.folded();
        Fields { title: scrubbed.title, subtitle: scrubbed.subtitle, body: scrubbed.body, folded }
    }
}

/// One field of a notification, safe to hand to another process or to write
/// inside an escape sequence.
fn field(raw: &str) -> String {
    text::cap_bytes(&text::scrub_text(raw), MAX_FIELD).to_string()
}

/// An escape rung's bytes, written straight to stdout the way
/// `osc::copy_to_clipboard` writes OSC 52: between draws, no `execute!`, and
/// nothing on screen moves, so no redraw is owed. The one writer `post`
/// hands `post_with`; a test hands it a recorder.
fn write_stdout(bytes: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    out.write_all(bytes.as_bytes())?;
    out.flush()
}

/// OSC 9 has one text, not three fields — so they are joined with the
/// separator every other line of mesimon uses.
fn osc9(title: &str, body: &str) -> String {
    if title.is_empty() {
        format!("\x1b]9;{body}\x07")
    } else {
        format!("\x1b]9;{title} ∙ {body}\x07")
    }
}

/// kitty's OSC 99, the simplest documented form: empty metadata, and the
/// payload is the title. ST-terminated, as kitty's own examples are; the
/// click action defaults to focusing the window.
fn osc99(title: &str, body: &str) -> String {
    if title.is_empty() {
        format!("\x1b]99;;{body}\x1b\\")
    } else {
        format!("\x1b]99;;{title} ∙ {body}\x1b\\")
    }
}

/// iTerm2's `RequestAttention=once`: one dock bounce, nothing to cancel.
const BOUNCE: &str = "\x1b]1337;RequestAttention=once\x07";

const BELL: &str = "\x07";

/// Whether a click will do anything, for the line below. `Some` only where
/// the answer is about the rung that ANSWERED: promising a click on a rung
/// that cannot deliver one is the failure this sentence exists to prevent,
/// and on Linux a `TERM_PROGRAM` of `vscode` or `WezTerm` would otherwise
/// resolve a macOS bundle id and make the line lie.
fn click_words(banner: &Banner, click: Option<&Click>) -> Option<String> {
    match (banner, click) {
        (Banner::TerminalNotifier, Some(c)) => Some(match &c.reveal {
            Some(r) => format!("click raises {} {}", c.app, r.word()),
            // Honest about the half it has: the application will come
            // forward showing whatever was in front of it, which is exactly
            // what somebody reads this line to find out (T-301).
            None => format!(
                "click raises {}, not this tab ∙ MESIMON_TERM_REVEAL names a program that can",
                c.app
            ),
        }),
        (Banner::TerminalNotifier, None) => {
            Some("click does nothing ∙ MESIMON_TERM_BUNDLE names your terminal's bundle id".into())
        }
        (Banner::Osascript, _) => {
            Some("this rung's banner cannot be clicked — terminal-notifier's can".into())
        }
        _ => None,
    }
}

/// Who posts the banner, as `doctor` says it (T-676): the ladder's rung
/// under `Delivered by: mesimon`, the terminal's escape under `your
/// terminal` — or that nothing will, where the terminal has none.
fn banner_words(ch: &Channels, by_terminal: bool) -> String {
    match (by_terminal, ch.poster) {
        (false, _) => ch.banner.word(),
        (true, _) if ch.banner == Banner::Off => Banner::Off.word(),
        (true, Some(poster)) => format!("your terminal, {}", poster.word()),
        (true, None) => "your terminal, which cannot post banners — none will show \
                         (Settings ∙ Notifications ∙ Delivered by)"
            .into(),
    }
}

/// What `mesimon doctor` says: whether it is on, which rungs answered, and
/// both sounds. The rungs are named even while it is off, because "would it
/// work if I turned it on" is the question somebody reads this line to ask.
pub fn doctor_line() -> String {
    let p = &crate::prefs::load_home().prefs;
    let ch = find();
    let by_terminal = p.notify_via == crate::prefs::NotifyVia::Terminal;
    let banner = banner_words(&ch, by_terminal);
    // Asked once and said in BOTH arms: "would it work if I turned it on" is
    // the whole reason the off arm names its rungs at all. A click is the
    // helper's gift, so the terminal's road promises none.
    let click = click_words(&ch.banner, ch.click.as_ref()).filter(|_| !by_terminal);
    if !p.notify {
        let mut off = format!(
            "off (Settings ∙ Notifications turns it on) — would use {banner} and {}",
            ch.player.word()
        );
        if let Some(c) = click {
            off.push_str(" ∙ ");
            off.push_str(&c);
        }
        return off;
    }
    let mut parts = vec![format!("on ∙ {banner}")];
    parts.push(format!("needs-you {}", p.notify_sound_needs_you.name()));
    if p.notify_done {
        parts.push(format!("finished {}", p.notify_sound_done.name()));
    } else {
        parts.push("nothing when a turn finishes".into());
    }
    if !p.notify_sound_needs_you.is_off() || !p.notify_sound_done.is_off() {
        parts.push(format!("through {}", ch.player.word()));
    }
    parts.push(if p.notify_focused {
        "shown even while the board is focused".into()
    } else {
        "quiet while the board is focused".into()
    });
    // What a banner will actually contain (T-292) — the question somebody
    // reads this line to ask on a machine other people can see.
    parts.push(if p.notify_words {
        "quoting the agent's own line".into()
    } else {
        "naming the ticket, never quoting the agent".into()
    });
    parts.push(if p.notify_in_pane {
        "said even inside the agent's own pane".into()
    } else {
        "silent inside the agent's own pane".into()
    });
    parts.push(if p.notify_crown {
        "the crown's agents said to you too".into()
    } else {
        "the crown's agents told to the crown".into()
    });
    // The one thing a user cannot read off the rung's name: this rung writes
    // to the board's own terminal, so it alone goes quiet while that terminal
    // belongs to an attached pane or an editor (T-291). Every other rung
    // spawns a program with null stdio and speaks through a handover. Said
    // apart from the in-pane PREFERENCE above, which is a choice; this is a
    // property of the rung that answered.
    if (by_terminal && ch.poster.is_some()) || matches!(ch.banner, Banner::Osc | Banner::Kitty) {
        parts.push("this rung cannot reach you mid-handover — a helper program can".into());
    }
    if p.notify_dock_bounce {
        parts.push(if crate::title::iterm2_direct() {
            "the dock bounces once when an agent needs you".into()
        } else {
            "dock bounce on, but this is not iTerm2 — nothing bounces".into()
        });
    }
    if let Some(c) = click {
        parts.push(c);
    }
    parts.join(" ∙ ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascot_arguments_preserve_clicks_fields_and_custom_helpers() {
        let f = fields("-board", "T-1", "needs you");
        let path = std::path::Path::new("/private/state with spaces/shin.png");
        let c = click(ITERM2_ID);
        let original = Banner::TerminalNotifier.argv(Some("mesimon-1"), Some(&c), &f).unwrap();
        let mut argv = with_icon(&Banner::TerminalNotifier, original.clone(), path);
        assert_eq!(&argv[1..3], &["-contentImage", path.to_str().unwrap()]);
        assert!(!argv.iter().any(|a| a == "-appIcon" || a == "-sender"));
        argv.drain(1..3);
        assert_eq!(argv, original, "the click and fields remain intact");
        let linux = Banner::NotifySend.argv(None, None, &f).unwrap();
        let argv = with_icon(&Banner::NotifySend, linux, path);
        assert_eq!(&argv[..5], &["notify-send", "--icon", path.to_str().unwrap(), "--", "-board"]);
        for rung in [Banner::Custom("ding".into()), Banner::Osascript] {
            let argv = rung.argv(None, None, &f).unwrap();
            assert_eq!(with_icon(&rung, argv.clone(), path), argv);
        }
    }

    #[test]
    fn silent_and_disabled_banners_do_not_materialize_icons() {
        let dir = std::env::temp_dir().join(format!("msmn-no-icon-{}", uuid::Uuid::new_v4()));
        let mut ch = Channels {
            banner: Banner::TerminalNotifier,
            player: Player::Off,
            group: None,
            click: None,
            icon_dir: Some(dir.clone()),
            notifier_app: None,
            poster: None,
        };
        post(&ch, &Post::sound_only(Sound::Off), &Console::default(), Delivery::default()).unwrap();
        ch.banner = Banner::Off;
        let p = Post {
            needs_you: true,
            title: "board".into(),
            subtitle: String::new(),
            body: "needs you".into(),
            sound: Sound::Off,
        };
        post(&ch, &p, &Console::default(), Delivery::default()).unwrap();
        assert!(!dir.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn failed_app_setup_still_launches_the_banner_and_reports_the_icon() {
        let root =
            std::env::temp_dir().join(format!("msmn-notify-fallback-{}", uuid::Uuid::new_v4()));
        let source = root.join("broken.app");
        std::fs::create_dir_all(&source).unwrap();
        let ch = Channels {
            banner: Banner::TerminalNotifier,
            player: Player::Off,
            group: Some("mesimon-fallback".into()),
            click: Some(click(ITERM2_ID)),
            icon_dir: Some(root.join("notifications")),
            notifier_app: Some(source),
            poster: None,
        };
        let p = Post {
            needs_you: false,
            title: "board".into(),
            subtitle: "T-1".into(),
            body: "finished".into(),
            sound: Sound::Off,
        };
        let calls = std::cell::RefCell::new(Vec::new());
        let error = post_with(
            &ch,
            &p,
            &Console::default(),
            Delivery::default(),
            |argv| {
                calls.borrow_mut().push(argv.to_vec());
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("app icon unavailable"));
        let calls = calls.into_inner();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0], "terminal-notifier");
        assert!(calls[0].windows(2).any(|pair| pair == ["-group", "mesimon-fallback"]));
        assert!(calls[0].windows(2).any(|pair| pair == ["-activate", ITERM2_ID]));
        assert!(calls[0].iter().any(|arg| arg == "-contentImage"));
        assert!(!calls[0].iter().any(|arg| arg == "-appIcon"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_applet_takes_the_osascript_words_and_none_can_read_as_a_flag() {
        let program = std::path::Path::new("/s/Mesimon.app/Contents/MacOS/applet");
        let f = fields("-board", "-T-1", "-needs you");
        assert_eq!(
            applet_argv(program, &f),
            vec![program.to_str().unwrap(), "=-board", "=-T-1", "=-needs you"]
        );
        let f = plain("board", "2 agents finished");
        assert_eq!(
            applet_argv(program, &f),
            vec![program.to_str().unwrap(), "=board", "=2 agents finished"]
        );
        // The applet's words are the osascript rung's, in its order.
        for f in [fields("b", "T-1", "done"), plain("b", "done")] {
            let osa = Banner::Osascript.argv(None, None, &f).unwrap();
            let words: Vec<String> = osa[7..].iter().map(|w| format!("={w}")).collect();
            assert_eq!(applet_argv(program, &f)[1..], words[..]);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn without_terminal_notifier_the_banner_comes_from_the_mesimon_applet() {
        let root =
            std::env::temp_dir().join(format!("msmn-notify-applet-{}", uuid::Uuid::new_v4()));
        let ch = Channels {
            banner: Banner::Osascript,
            player: Player::Off,
            group: None,
            click: None,
            icon_dir: Some(root.join("notifications")),
            notifier_app: None,
            poster: None,
        };
        let p = Post {
            needs_you: true,
            title: "board".into(),
            subtitle: "T-1".into(),
            body: "needs you".into(),
            sound: Sound::Off,
        };
        let calls = std::cell::RefCell::new(Vec::new());
        post_with(
            &ch,
            &p,
            &Console::default(),
            Delivery::default(),
            |argv| {
                calls.borrow_mut().push(argv.to_vec());
                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();
        let calls = calls.into_inner();
        assert_eq!(calls.len(), 1);
        assert!(calls[0][0].ends_with("Mesimon.app/Contents/MacOS/applet"), "{:?}", calls[0]);
        assert_eq!(calls[0][1..], ["=board", "=T-1", "=needs you"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    /// The three fields a rung is handed, as `post` builds them.
    fn fields(title: &str, subtitle: &str, body: &str) -> Fields {
        Fields::of(&Post {
            needs_you: false,
            title: title.into(),
            subtitle: subtitle.into(),
            body: body.into(),
            sound: Sound::Glass,
        })
    }

    /// A post with no subtitle — the aggregate's shape, and the shape every
    /// rung had before T-292.
    fn plain(title: &str, body: &str) -> Fields {
        fields(title, "", body)
    }
    fn all(n: &str) -> Option<PathBuf> {
        Some(PathBuf::from(format!("/usr/bin/{n}")))
    }

    /// A click that raises an application and nothing finer — what every
    /// test written before the tab existed means by "a click".
    fn click(app: &str) -> Click {
        Click { app: app.into(), reveal: None }
    }

    #[test]
    fn the_env_wins_on_both_ladders() {
        assert_eq!(find_banner(Some(" off "), all, false), Banner::Off);
        assert_eq!(find_banner(Some("OSC"), all, false), Banner::Osc);
        assert_eq!(
            find_banner(Some("my-notifier"), all, false),
            Banner::Custom("my-notifier".into())
        );
        assert_eq!(find_banner(Some("   "), none, false), find_banner(None, none, false));
        assert_eq!(find_player(Some("off"), all), Player::Off);
        assert_eq!(find_player(Some(" aplay "), all), Player::Custom("aplay".into()));
    }

    /// kitty's rung (T-492): below terminal-notifier, above the two
    /// programs that cannot click, and only where the board runs in kitty
    /// directly — an outer tmux's `TERM_PROGRAM` is the veto.
    #[test]
    fn kitty_sits_below_terminal_notifier_and_never_behind_an_outer_tmux() {
        assert_eq!(find_banner(None, none, true), Banner::Kitty);
        assert_eq!(find_banner(None, all, true), Banner::TerminalNotifier);
        assert_eq!(find_banner(Some("kitty"), all, false), Banner::Kitty);
        assert!(in_kitty(Some("1"), None));
        assert!(!in_kitty(Some("1"), Some("tmux")));
        assert!(!in_kitty(None, None));
        assert!(!in_kitty(Some(""), None));
    }

    #[test]
    fn the_ladders_end_on_a_rung_that_always_exists() {
        // Nothing on PATH: the banner falls to the terminal's own escape and
        // the sound to the bell — neither can fail to resolve.
        if cfg!(target_os = "macos") {
            assert_eq!(find_banner(None, none, false), Banner::Osascript);
            assert_eq!(find_player(None, none), Player::Afplay);
        } else {
            assert_eq!(find_banner(None, none, false), Banner::Osc);
            assert_eq!(find_player(None, none), Player::Bell);
            assert_eq!(find_banner(None, all, false), Banner::TerminalNotifier);
            assert_eq!(find_player(None, all), Player::Paplay);
            let send =
                |n: &str| (n == "notify-send").then(|| PathBuf::from("/usr/bin/notify-send"));
            assert_eq!(find_banner(None, send, false), Banner::NotifySend);
        }
        // terminal-notifier outranks the platform's own on every platform.
        let tn = |n: &str| (n == "terminal-notifier").then(|| PathBuf::from("/usr/bin/x"));
        assert_eq!(find_banner(None, tn, false), Banner::TerminalNotifier);
    }

    #[test]
    fn the_words_ride_argv_and_never_the_script() {
        // Both scripts, since T-292: the two-field one and the three-field
        // one an event with a subtitle takes.
        for f in [
            plain("board", "\" & do shell script \"boom"),
            fields("board", "T-1 ∙ \" & do shell script \"boom", "needs you"),
        ] {
            let argv = Banner::Osascript.argv(None, None, &f).expect("argv");
            // Every `-e` fragment is a constant: none of them holds the words.
            for (i, a) in argv.iter().enumerate() {
                if a == "-e" {
                    let script = &argv[i + 1];
                    assert!(!script.contains("board"), "{script}");
                    assert!(!script.contains("boom"), "{script}");
                    assert!(!script.contains("T-1"), "{script}");
                }
            }
            assert!(argv.iter().any(|a| a.contains("boom")), "and the words are still sent");
        }
        assert_eq!(
            Banner::NotifySend.argv(None, None, &plain("-t", "b")).expect("argv"),
            vec!["notify-send", "--", "-t", "b"],
            "a title that starts with a dash is still a title"
        );
        assert_eq!(
            Banner::Custom("ding".into()).argv(None, None, &plain("t", "b")).expect("argv"),
            vec!["ding", "t", "b"]
        );
        assert!(Banner::Osc.argv(None, None, &plain("t", "b")).is_none());
        assert!(Banner::Off.argv(None, None, &plain("t", "b")).is_none());
    }

    /// T-292: the two rungs that have three fields take the subtitle on its
    /// own; the three that have one get it folded in front of the body, so a
    /// user's own program keeps the arity it was written against.
    #[test]
    fn a_subtitle_rides_its_own_field_or_folds_into_the_body() {
        let f = fields("board", "T-1 ∙ Add auth", "needs you ∙ PERMISSION");
        assert_eq!(
            Banner::TerminalNotifier.argv(None, None, &f).expect("argv"),
            vec![
                "terminal-notifier",
                "-title",
                "board",
                "-subtitle",
                "T-1 ∙ Add auth",
                "-message",
                "needs you ∙ PERMISSION"
            ]
        );
        let osa = Banner::Osascript.argv(None, None, &f).expect("argv");
        assert_eq!(&osa[osa.len() - 3..], ["board", "T-1 ∙ Add auth", "needs you ∙ PERMISSION"]);
        assert!(osa.iter().any(|a| a.contains("subtitle (item 2 of argv)")), "{osa:?}");
        // The one-field rungs, all folded the same way.
        let folded = "T-1 ∙ Add auth ∙ needs you ∙ PERMISSION";
        assert_eq!(
            Banner::NotifySend.argv(None, None, &f).expect("argv"),
            vec!["notify-send", "--", "board", folded]
        );
        assert_eq!(
            Banner::Custom("ding".into()).argv(None, None, &f).expect("argv"),
            vec!["ding", "board", folded],
            "a user's own program still takes two arguments"
        );
    }

    /// An aggregate has no one ticket to name, so it takes the argv that
    /// shipped before this — byte for byte, both scripts included.
    #[test]
    fn no_subtitle_is_the_argv_that_always_was() {
        let f = plain("board", "2 agents finished ∙ T-1 T-2");
        assert_eq!(
            Banner::TerminalNotifier.argv(None, None, &f).expect("argv"),
            vec!["terminal-notifier", "-title", "board", "-message", "2 agents finished ∙ T-1 T-2"]
        );
        let osa = Banner::Osascript.argv(None, None, &f).expect("argv");
        assert!(osa.iter().any(|a| a.contains("(item 2 of argv) with title")), "{osa:?}");
        assert!(!osa.iter().any(|a| a.contains("subtitle")), "{osa:?}");
        assert_eq!(osa.len(), 9, "seven script words and the two fields");
    }

    /// The click's terminal (T-293). Every input is a parameter, so this is
    /// the same ladder on every platform the suite runs on.
    #[test]
    fn the_terminal_that_gets_raised_is_named_by_the_env_first_and_the_os_second() {
        let itrm = "com.googlecode.iterm2";
        // The variable outranks everything, and `off` is a rung of its own.
        assert_eq!(
            find_activate(Some(" com.x.y "), Some(itrm), Some("iTerm.app")),
            Some("com.x.y".into())
        );
        assert_eq!(find_activate(Some("OFF"), Some(itrm), Some("iTerm.app")), None);
        assert_eq!(find_activate(Some("   "), Some(itrm), None), Some(itrm.into()));
        // macOS's own answer beats the table, and the table catches its loss.
        assert_eq!(find_activate(None, Some("com.x.y"), Some("iTerm.app")), Some("com.x.y".into()));
        assert_eq!(find_activate(None, None, Some("iTerm.app")), Some(itrm.into()));
        assert_eq!(
            find_activate(None, None, Some("Apple_Terminal")),
            Some("com.apple.Terminal".into())
        );
        // Nothing to go on, and a terminal the table has never heard of.
        assert_eq!(find_activate(None, None, None), None);
        assert_eq!(find_activate(None, None, Some("SomeNewTerm")), None);
    }

    /// The veto, and the reason it sits above `__CFBundleIdentifier`: inside
    /// an outer tmux that variable is INHERITED, so it names whatever started
    /// the server rather than the client attached now — and `-activate`
    /// launches an application that is not running, so a stale id opens a
    /// window of the wrong terminal instead of raising the right one.
    #[test]
    fn an_outer_tmux_names_no_terminal_because_nothing_it_can_see_is_fresh() {
        assert_eq!(find_activate(None, Some("com.apple.Terminal"), Some("tmux")), None);
        assert_eq!(find_activate(None, None, Some("tmux")), None);
        // Said explicitly, it is still honoured: the user can see their own
        // terminal even when mesimon cannot.
        assert_eq!(
            find_activate(Some("com.googlecode.iterm2"), None, Some("tmux")),
            Some("com.googlecode.iterm2".into())
        );
    }

    /// The tab (T-301), which is the other half of the same click: the
    /// variable first, then the terminal we are actually in.
    #[test]
    fn the_tab_is_named_by_the_env_first_and_the_terminal_second() {
        let sid = Some("w0t1p0:3D455141-E58B-4841-B5FD-1C4F99E53CD6");
        let uuid = "3D455141-E58B-4841-B5FD-1C4F99E53CD6";
        let tty = Some("/dev/ttys004");
        // The variable outranks both, and `off` is a rung of its own: the
        // application still comes forward, the tab is simply not asked for.
        assert_eq!(
            find_reveal(Some(" raise-my-term "), Some("iTerm.app"), sid, tty),
            Some(Reveal::Custom("raise-my-term".into()))
        );
        assert_eq!(find_reveal(Some("OFF"), Some("iTerm.app"), sid, tty), None);
        // A blank variable is no variable, as on both ladders above.
        assert_eq!(
            find_reveal(Some("   "), Some("iTerm.app"), sid, tty),
            find_reveal(None, Some("iTerm.app"), sid, tty)
        );
        // The two terminals that can be asked, each by its own key.
        assert_eq!(
            find_reveal(None, Some("iTerm.app"), sid, tty),
            Some(Reveal::ITerm2(uuid.into()))
        );
        assert_eq!(
            find_reveal(None, Some("Apple_Terminal"), sid, tty),
            Some(Reveal::AppleTerminal("/dev/ttys004".into()))
        );
        // A uuid with no coordinates in front of it is still a uuid.
        assert_eq!(
            find_reveal(None, Some("iTerm.app"), Some(uuid), None),
            Some(Reveal::ITerm2(uuid.into()))
        );
        // Nothing to go on: the key is missing, or the terminal is one no
        // rung fits — which is most of them, and costs only the tab.
        assert_eq!(find_reveal(None, Some("iTerm.app"), None, tty), None);
        assert_eq!(find_reveal(None, Some("Apple_Terminal"), sid, None), None);
        assert_eq!(find_reveal(None, Some("WezTerm"), sid, tty), None);
        assert_eq!(find_reveal(None, None, sid, tty), None);
    }

    /// The veto reaches the tab too, and by the same road: inside an outer
    /// tmux `ITERM_SESSION_ID` is INHERITED from whatever started the
    /// server, so it names a session that is not this one — and selecting
    /// the wrong tab is the bug, not the fix. `TERM_PROGRAM` is rewritten in
    /// every pane, so the table simply never answers there.
    #[test]
    fn an_outer_tmux_names_no_tab_either() {
        let sid = Some("w0t1p0:3D455141-E58B-4841-B5FD-1C4F99E53CD6");
        assert_eq!(find_reveal(None, Some("tmux"), sid, Some("/dev/ttys004")), None);
        assert_eq!(
            find_click(None, None, Some("com.googlecode.iterm2"), Some("tmux"), sid, None),
            None,
            "and the click as a whole is still refused"
        );
    }

    /// A script talks to the application the click raises, or it does not
    /// travel: a `MESIMON_TERM_BUNDLE` naming some other application would
    /// otherwise leave an iTerm2 script attached to a click that raises
    /// something else. The user's own program names no application and so
    /// has nothing to disagree with.
    #[test]
    fn a_reveal_scripts_the_application_the_click_raises() {
        let sid = Some("w0t1p0:3D455141-E58B-4841-B5FD-1C4F99E53CD6");
        let uuid = "3D455141-E58B-4841-B5FD-1C4F99E53CD6";
        let whole = find_click(None, None, None, Some("iTerm.app"), sid, None).expect("a click");
        assert_eq!(whole.app, ITERM2_ID);
        assert_eq!(whole.reveal, Some(Reveal::ITerm2(uuid.into())));
        // Said explicitly and agreeing: both halves stand.
        let same =
            find_click(Some(ITERM2_ID), None, None, Some("iTerm.app"), sid, None).expect("a click");
        assert_eq!(same.reveal, Some(Reveal::ITerm2(uuid.into())));
        // Said explicitly and disagreeing: the application is the user's
        // word, and the script that cannot be about it is dropped.
        let other =
            find_click(Some("com.x.y"), None, None, Some("iTerm.app"), sid, None).expect("a click");
        assert_eq!(other.app, "com.x.y");
        assert_eq!(other.reveal, None);
        // A program of the user's own survives the same disagreement.
        let custom =
            find_click(Some("com.x.y"), Some("raise-my-term"), None, Some("iTerm.app"), sid, None)
                .expect("a click");
        assert_eq!(custom.reveal, Some(Reveal::Custom("raise-my-term".into())));
        // No application, no click — and so no tab either.
        assert_eq!(find_click(None, None, None, Some("SomeNewTerm"), sid, None), None);
    }

    /// The command is a constant script and one validated id, single-quoted:
    /// argv's rule kept where `-execute` offers no argv. And every script
    /// asks whether its application is running before it says `tell`, since
    /// `tell` STARTS what is not running and a click that opens an empty
    /// terminal is the promise this whole rung exists to keep.
    #[test]
    fn the_click_runs_a_constant_script_and_never_a_composed_one() {
        for (script, id) in [(ITERM2_SCRIPT, ITERM2_ID), (TERMINAL_SCRIPT, TERMINAL_ID)] {
            assert!(script.contains(&format!("application id \"{id}\" is running")), "{script}");
            // The activate is INSIDE the guard, which is what keeps it from
            // launching a terminal that is not running.
            let guard = script.find("is running").expect("a guard");
            assert!(script[guard..].contains("\nactivate\n"), "{script}");
            assert!(script.starts_with("on run argv"), "{script}");
            assert!(script.contains("item 1 of argv"), "the key rides argv");
            assert!(!script.contains('\''), "a quote would end sh's quoting: {script}");
        }
        let cmd = Reveal::ITerm2("3D455141-E58B".into()).command().expect("a command");
        assert!(cmd.starts_with("'osascript' '-e' '"), "{cmd}");
        assert!(cmd.ends_with("'3D455141-E58B'"), "{cmd}");
        let term = Reveal::AppleTerminal("/dev/ttys004".into()).command().expect("a command");
        assert!(term.contains("'/dev/ttys004'"), "{term}");
        assert_eq!(
            Reveal::Custom("raise-my-term".into()).command(),
            Some("'raise-my-term'".into())
        );
        // Nothing this module refuses can reach the line, and a word holding
        // the one byte that could end the quoting is refused, not escaped.
        assert_eq!(sh_line(&["a", "b c"]), Some("'a' 'b c'".into()));
        assert_eq!(sh_line(&["it's"]), None);
        assert_eq!(sh_line(&[""]), None);
        assert_eq!(sh_line(&[]), None);
        assert_eq!(sh_word("it's"), None);
        assert_eq!(sh_word("say\nboom"), None);
        // A program, not a command line: quoted whole, a command line would
        // be one program name with spaces in it and would never run.
        assert_eq!(sh_word("raise-my-term --window 3"), None);
        assert_eq!(sh_word("  raise-my-term  "), Some("raise-my-term".into()));
        assert_eq!(find_reveal(Some("it's"), None, None, None), None);
    }

    /// The keys REJECT where `field` scrubs, for `bundle_id`'s reason: a
    /// repaired session id names a different tab, and landing in one is the
    /// bug being fixed.
    #[test]
    fn a_session_id_or_a_tty_that_is_not_one_is_refused_rather_than_scrubbed() {
        assert_eq!(session_uuid("w0t1p0:3D45-E58B"), Some("3D45-E58B".into()));
        assert_eq!(session_uuid(" 3D45-E58B \n"), Some("3D45-E58B".into()));
        assert_eq!(session_uuid(""), None);
        assert_eq!(session_uuid("w0t1p0:"), None);
        assert_eq!(session_uuid("-flag"), None);
        assert_eq!(session_uuid("w0t1p0:3D45 E58B"), None, "not repaired into 3D45E58B");
        assert_eq!(session_uuid("w0t1p0:a';rm -rf /"), None);
        assert_eq!(session_uuid(&"a".repeat(REVEAL_MAX + 1)), None);
        assert_eq!(tty_path("/dev/ttys004"), Some("/dev/ttys004".into()));
        assert_eq!(tty_path("/dev/pts/3"), Some("/dev/pts/3".into()));
        assert_eq!(tty_path("ttys004"), None, "a tab's tty is an absolute path");
        assert_eq!(tty_path("/dev/ttys004;boom"), None);
        assert_eq!(tty_path(&format!("/dev/{}", "a".repeat(REVEAL_MAX))), None);
    }

    /// The tab flag sits beside the application's, never instead of it: a
    /// terminal that refuses to be scripted leaves exactly the click T-293
    /// shipped, and no other rung grows either flag.
    #[test]
    fn the_tab_is_a_second_flag_on_the_one_rung_that_can_carry_a_click() {
        let f = fields("board", "T-1 ∙ Add auth", "needs you");
        let whole =
            Click { app: ITERM2_ID.into(), reveal: Some(Reveal::ITerm2("3D455141-E58B".into())) };
        let argv = Banner::TerminalNotifier.argv(None, Some(&whole), &f).expect("argv");
        let at = argv.iter().position(|a| a == "-activate").expect("the application");
        let ex = argv.iter().position(|a| a == "-execute").expect("the tab");
        assert_eq!(argv[at + 1], ITERM2_ID);
        assert!(at < ex, "terminal-notifier activates first, then runs the command");
        assert!(argv[ex + 1].contains("'3D455141-E58B'"), "{}", argv[ex + 1]);
        // Without a reveal it is the argv T-293 shipped, byte for byte.
        let half = Banner::TerminalNotifier.argv(None, Some(&click(ITERM2_ID)), &f).expect("argv");
        assert!(!half.iter().any(|a| a == "-execute"), "{half:?}");
        assert_eq!(half.len() + 2, argv.len(), "the flag is the only difference");
        // And the rungs that cannot carry a click grow neither flag.
        for rung in [Banner::Osascript, Banner::NotifySend, Banner::Custom("ding".into())] {
            assert_eq!(rung.argv(None, Some(&whole), &f), rung.argv(None, None, &f), "{rung:?}");
        }
    }

    /// This boundary REJECTS where `field` scrubs, and that is deliberate: a
    /// bundle id with a character dropped is a different, possibly real one,
    /// so a silent repair would raise the wrong application.
    #[test]
    fn a_bundle_id_that_is_not_one_is_refused_rather_than_scrubbed() {
        assert_eq!(bundle_id("com.googlecode.iterm2"), Some("com.googlecode.iterm2".into()));
        assert_eq!(bundle_id("  com.apple.Terminal\n"), Some("com.apple.Terminal".into()));
        assert_eq!(bundle_id("net.kovidgoyal.kitty"), Some("net.kovidgoyal.kitty".into()));
        // A leading dash would be read as another flag by terminal-notifier.
        assert_eq!(bundle_id("-sound"), None);
        assert_eq!(bundle_id(""), None);
        assert_eq!(bundle_id("   "), None);
        assert_eq!(bundle_id(".com.x"), None);
        // Not repaired into `com.xy` — refused.
        assert_eq!(bundle_id("com.x y"), None);
        assert_eq!(bundle_id("com.x;rm -rf /"), None);
        assert_eq!(bundle_id(&"a".repeat(BUNDLE_ID_MAX + 1)), None);
        // And every rung exits through it, so nothing reaches argv unchecked.
        assert_eq!(find_activate(Some("-sound"), None, None), None);
        assert_eq!(
            find_activate(None, Some("com.x y"), Some("iTerm.app")),
            Some("com.googlecode.iterm2".into())
        );
    }

    /// One rung can carry a click, so one rung gets the flag. The loop is the
    /// load-bearing half: `osascript` finds its words by POSITION, so an
    /// argument inserted there would shift the title, subtitle and body.
    #[test]
    fn only_the_terminal_notifier_rung_grows_a_flag_for_the_click() {
        let f = fields("board", "T-1 ∙ Add auth", "needs you");
        let argv = Banner::TerminalNotifier.argv(None, Some(&click("com.x.y")), &f).expect("argv");
        let at = argv.iter().position(|a| a == "-activate").expect("the flag");
        assert_eq!(argv[at + 1], "com.x.y");
        // `-group` is still last, whether or not the click is asked for.
        let both = Banner::TerminalNotifier
            .argv(Some("mesimon-abc"), Some(&click("com.x.y")), &f)
            .expect("argv");
        assert_eq!(&both[both.len() - 2..], ["-group", "mesimon-abc"]);
        for rung in [Banner::Osascript, Banner::NotifySend, Banner::Custom("ding".into())] {
            assert_eq!(
                rung.argv(None, Some(&click("com.x.y")), &f),
                rung.argv(None, None, &f),
                "{rung:?} grew an argument for a click it cannot carry"
            );
        }
        // And with no id, the one rung that CAN carry a click is the argv
        // that shipped before — no empty flag, no placeholder.
        let bare = Banner::TerminalNotifier.argv(None, None, &f).expect("argv");
        assert!(!bare.iter().any(|a| a == "-activate"), "{bare:?}");
        assert_eq!(bare.len() + 2, argv.len(), "the flag is the only difference");
    }

    /// `doctor` promises a click only on the rung that can deliver one — on
    /// Linux a `TERM_PROGRAM` of `vscode` resolves a macOS bundle id, and the
    /// line must not say a notify-send banner is clickable because of it.
    #[test]
    fn the_click_is_promised_only_on_the_rung_that_can_deliver_one() {
        let said = click_words(&Banner::TerminalNotifier, Some(&click("com.googlecode.iterm2")))
            .expect("a word");
        assert!(said.contains("com.googlecode.iterm2"), "{said}");
        let missing = click_words(&Banner::TerminalNotifier, None).expect("a word");
        assert!(missing.contains("MESIMON_TERM_BUNDLE"), "{missing}");
        assert!(click_words(&Banner::Osascript, Some(&click("com.x.y"))).is_some());
        for rung in [Banner::NotifySend, Banner::Osc, Banner::Off, Banner::Custom("ding".into())] {
            assert_eq!(click_words(&rung, Some(&click("com.x.y"))), None, "{rung:?}");
        }
        // The application alone is said as the half it is, and the tab is
        // said as the whole when there is one (T-301).
        let half = click_words(&Banner::TerminalNotifier, Some(&click("com.x.y"))).expect("a word");
        assert!(half.contains("not this tab") && half.contains("MESIMON_TERM_REVEAL"), "{half}");
        let whole = click_words(
            &Banner::TerminalNotifier,
            Some(&Click {
                app: ITERM2_ID.into(),
                reveal: Some(Reveal::ITerm2("3D455141-E58B".into())),
            }),
        )
        .expect("a word");
        assert!(whole.contains("own tab") && !whole.contains("not this tab"), "{whole}");
    }

    /// One group per board (T-292), so a new banner replaces the last rather
    /// than stacking — and only the rung that has the concept gets it.
    #[test]
    fn a_board_groups_its_own_banners() {
        let f = fields("board", "T-1 ∙ Add auth", "needs you");
        let argv = Banner::TerminalNotifier.argv(Some("mesimon-abc123"), None, &f).expect("argv");
        assert_eq!(&argv[argv.len() - 2..], ["-group", "mesimon-abc123"]);
        for rung in [Banner::Osascript, Banner::NotifySend, Banner::Custom("ding".into())] {
            let argv = rung.argv(Some("mesimon-abc123"), None, &f).expect("argv");
            assert!(!argv.iter().any(|a| a == "mesimon-abc123"), "{argv:?}");
        }
        // Keyed off the repo, so two checkouts are two groups.
        let here = group_for(std::path::Path::new(".")).expect("a group for this repo");
        assert!(here.starts_with("mesimon-"), "{here}");
        assert_ne!(here, group_for(std::path::Path::new("/")).expect("a group for /"));
    }

    #[test]
    fn a_field_is_scrubbed_and_bounded() {
        // The two characters that would close an OSC sequence early, and the
        // one that would end a line in an argument.
        let dirty = "T-1 \x1b]9;evil\x07 \n needs you";
        let out = field(dirty);
        assert!(!out.contains('\x1b'), "{out}");
        assert!(!out.contains('\x07'), "{out}");
        assert!(!out.contains('\n'), "{out}");
        assert!(field(&"x".repeat(1000)).len() <= MAX_FIELD);
        assert_eq!(field("board ∙ name"), "board ∙ name", "ordinary words survive");
        // All three fields cross it, the subtitle included (T-292), and the
        // folded line is built from the scrubbed halves.
        let f = fields("b", "T-1 ∙ \x1b]9;evil\x07", "needs you\n now");
        for part in [&f.title, &f.subtitle, &f.body, &f.folded] {
            assert!(!part.contains('\x1b') && !part.contains('\x07') && !part.contains('\n'));
        }
        assert_eq!(f.folded, format!("{} ∙ {}", f.subtitle, f.body));
    }

    #[test]
    fn off_means_off_on_each_channel_alone() {
        assert!(Player::Off.argv(Sound::Glass).is_none());
        assert!(Player::Afplay.argv(Sound::Off).is_none());
        assert!(Player::Bell.argv(Sound::Glass).is_none(), "the bell has no argv, it is a write");
        assert_eq!(
            Player::Canberra.argv(Sound::Glass).expect("argv"),
            vec!["canberra-gtk-play", "-i", "message"]
        );
        assert_eq!(
            Player::Custom("ding".into()).argv(Sound::Tink).expect("argv"),
            vec!["ding", "Tink"]
        );
    }

    #[test]
    fn a_silent_post_runs_nothing() {
        // Off on both channels with a real body: `post` must not try to spawn,
        // and `Off` is the one rung that writes no escape either.
        let ch = Channels {
            banner: Banner::Off,
            player: Player::Off,
            group: None,
            click: None,
            icon_dir: None,
            notifier_app: None,
            poster: None,
        };
        let p = Post {
            needs_you: false,
            title: "t".into(),
            subtitle: "T-1 ∙ a ticket".into(),
            body: "b".into(),
            sound: Sound::Glass,
        };
        let console = Console::default();
        assert!(post(&ch, &p, &console, Delivery::default()).is_ok());
        assert!(post(&ch, &Post::sound_only(Sound::Off), &console, Delivery::default()).is_ok());
    }

    /// T-291's honest limit, as a rule rather than a sentence: an escape rung
    /// writes only while the board holds the terminal. Nothing here writes to
    /// the real stdout — the closure stands in for the escape, which is the
    /// only thing `write_if_held` decides about.
    #[test]
    fn an_escape_rung_is_silent_while_the_terminal_is_handed_over() {
        let console = Console::default();
        let mut wrote = 0;
        let go = |c: &Console, wrote: &mut i32| {
            c.write_if_held(|| {
                *wrote += 1;
                Ok(())
            })
            .expect("no error either way")
        };
        go(&console, &mut wrote);
        assert_eq!(wrote, 1, "a board holding its terminal writes");
        console.set_held(false);
        go(&console, &mut wrote);
        assert_eq!(wrote, 1, "a handover takes the escape rungs away");
        console.set_held(true);
        go(&console, &mut wrote);
        assert_eq!(wrote, 2, "and the return gives them back");
    }

    /// What one post under `Delivered by: your terminal` (T-676) launched
    /// and wrote, on a ladder whose helper would otherwise answer.
    fn by_terminal(poster: Option<Poster>, banner: Banner, p: &Post) -> (Vec<Vec<String>>, String) {
        let ch = Channels {
            banner,
            player: Player::Custom("ding".into()),
            group: Some("mesimon-1".into()),
            click: Some(click(ITERM2_ID)),
            icon_dir: Some(PathBuf::from("/nonexistent/msmn-notify-terminal")),
            notifier_app: Some(PathBuf::from("/nonexistent/terminal-notifier.app")),
            poster,
        };
        let launched = std::cell::RefCell::new(Vec::new());
        let wrote = std::cell::RefCell::new(String::new());
        post_with(
            &ch,
            p,
            &Console::default(),
            Delivery { bounce: false, by_terminal: true },
            |argv| {
                launched.borrow_mut().push(argv.to_vec());
                Ok(())
            },
            |bytes| {
                wrote.borrow_mut().push_str(bytes);
                Ok(())
            },
        )
        .expect("nothing on this road can fail");
        (launched.into_inner(), wrote.into_inner())
    }

    fn banner_post() -> Post {
        Post {
            needs_you: true,
            title: "mesimon - simbly".into(),
            subtitle: "T-1 ∙ Add auth".into(),
            body: "needs you".into(),
            sound: Sound::Off,
        }
    }

    /// Each terminal gets its own escape, built from the scrubbed fields,
    /// and no program of ours is launched — not the helper, not the applet.
    #[test]
    fn the_terminal_posts_its_own_escape_and_nothing_is_launched() {
        let p = banner_post();
        let (launched, wrote) = by_terminal(Some(Poster::Osc9), Banner::TerminalNotifier, &p);
        assert!(launched.is_empty(), "{launched:?}");
        assert_eq!(wrote, "\x1b]9;mesimon - simbly ∙ T-1 ∙ Add auth ∙ needs you\x07");

        let (launched, wrote) = by_terminal(Some(Poster::Osc99), Banner::Osascript, &p);
        assert!(launched.is_empty(), "{launched:?}");
        let (title, body) = wrote.split_once("\x1b\\").expect("two chunks");
        let id = title.strip_prefix("\x1b]99;i=").and_then(|t| t.split_once(':')).expect("an id").0;
        assert_eq!(title, format!("\x1b]99;i={id}:d=0;mesimon - simbly"));
        assert_eq!(body, format!("\x1b]99;i={id}:p=body;T-1 ∙ Add auth ∙ needs you\x1b\\"));

        let (launched, wrote) = by_terminal(Some(Poster::Osc777), Banner::NotifySend, &p);
        assert!(launched.is_empty(), "{launched:?}");
        assert_eq!(wrote, "\x1b]777;notify;mesimon - simbly;T-1 ∙ Add auth ∙ needs you\x1b\\");
    }

    /// A word from a payload cannot close the sequence early, and a `;` in a
    /// board's name cannot move OSC 777's title into its body.
    #[test]
    fn a_terminal_escape_carries_only_scrubbed_words() {
        let p = Post {
            title: "mesimon - a;b".into(),
            subtitle: "T-1 ∙ \x1b]9;evil\x07".into(),
            body: "needs you\x1b\\ now".into(),
            ..banner_post()
        };
        for poster in [Poster::Osc9, Poster::Osc99, Poster::Osc777] {
            let (_, wrote) = by_terminal(Some(poster), Banner::Osascript, &p);
            let closers = wrote.matches('\x07').count() + wrote.matches("\x1b\\").count();
            let chunks = if poster == Poster::Osc99 { 2 } else { 1 };
            assert_eq!(closers, chunks, "{poster:?}: {wrote:?}");
            let escapes = match poster {
                Poster::Osc9 => 1,
                Poster::Osc99 => 4,
                Poster::Osc777 => 2,
            };
            assert_eq!(wrote.matches('\x1b').count(), escapes, "{poster:?}: {wrote:?}");
        }
        let (_, wrote) = by_terminal(Some(Poster::Osc777), Banner::Osascript, &p);
        assert!(wrote.starts_with("\x1b]777;notify;mesimon - a,b;"), "{wrote:?}");
    }

    /// No fallback: a terminal that cannot post shows nothing and launches
    /// nothing, `MESIMON_NOTIFY=off` still silences the road, and the sound
    /// goes its own way on either.
    #[test]
    fn a_terminal_that_cannot_post_shows_nothing_and_the_sound_still_plays() {
        let p = banner_post();
        let (launched, wrote) = by_terminal(None, Banner::TerminalNotifier, &p);
        assert!(launched.is_empty() && wrote.is_empty(), "{launched:?} {wrote:?}");
        let (launched, wrote) = by_terminal(Some(Poster::Osc9), Banner::Off, &p);
        assert!(launched.is_empty() && wrote.is_empty(), "{launched:?} {wrote:?}");
        let chime = Post { sound: Sound::Glass, ..banner_post() };
        let (launched, wrote) = by_terminal(None, Banner::TerminalNotifier, &chime);
        assert_eq!(launched, vec![vec!["ding".to_string(), "Glass".to_string()]]);
        assert!(wrote.is_empty());
    }

    /// `doctor` names who posts it, and says so when nobody will.
    #[test]
    fn doctor_names_the_terminal_or_says_none_will_show() {
        let mut ch = Channels {
            banner: Banner::TerminalNotifier,
            player: Player::Off,
            group: None,
            click: None,
            icon_dir: None,
            notifier_app: None,
            poster: Some(Poster::Osc99),
        };
        assert_eq!(banner_words(&ch, false), "terminal-notifier");
        assert_eq!(banner_words(&ch, true), "your terminal, kitty (OSC 99)");
        ch.poster = None;
        assert!(banner_words(&ch, true).contains("none will show"));
        ch.banner = Banner::Off;
        assert_eq!(banner_words(&ch, true), "off");
    }
}
