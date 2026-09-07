//! The ratatui board client. Owns its own terminal and the staged restore
//! discipline (docs/05 §13) — exercised on every focus handover (docs/19 §2).

mod app;
mod caffeine;
mod keys;
// Public so an integration test can drive the real connect path (the
// build-skew daemon restart lives in it); the TUI itself uses it internally.
pub mod client;
mod detect;
mod external;
mod glyphs;
mod handover;
mod layout;
mod localtime;
mod mascot;
mod notification_app;
mod notifier;
mod notify;
mod opener;
mod osc;
mod peek;
mod prefs;
mod release;
mod rich;
mod tags;
mod text;
mod theme;
mod ui;
mod update;

use std::path::Path;

use anyhow::Result;
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use app::App;
use client::Client;

/// The words a snooze preset wears on the card row and in the status line —
/// the keymap's own hint for it, so the footer, the card and the status all
/// say one thing. The week start names the last rung's day.
pub(crate) fn snooze_words(
    p: mesimon_core::snooze::Preset,
    week_start: mesimon_core::snooze::Weekday,
) -> &'static str {
    mesimon_core::snooze::hint_for_label(p.label(week_start))
}

/// May a ticket grow its own shell session (T-300)? Off by default, and the
/// two keys that start one (`s` and `S` on the ticket page, `s` on the
/// board) are inert and unhinted while it is — the feature is whole
/// underneath, and this is the door. Read here and handed to `App` in `run`,
/// never in `App::new`, so no test app and no golden reads a developer's
/// environment.
pub(crate) fn ticket_shells() -> bool {
    std::env::var_os("MESIMON_TICKET_SHELLS").is_some_and(|v| v == "1")
}

/// What `mesimon doctor` says about that gate — a user who remembers the key
/// and finds it dead has one place to look.
pub fn ticket_shells_status() -> String {
    if ticket_shells() {
        "on ∙ MESIMON_TICKET_SHELLS=1 ∙ s and S start a shell on a ticket".into()
    } else {
        "off ∙ s and S are inert ∙ MESIMON_TICKET_SHELLS=1 offers them again".into()
    }
}

/// What `mesimon doctor` says about holding the machine awake (T-288):
/// whether it is on, and which rung would hold it — named even while it is
/// off, and named plainly when nothing here can.
pub use caffeine::doctor_line as keep_awake_status;
/// What `mesimon doctor` says about the note editor's `^g`: which program
/// opens, and which variable named it.
pub use external::doctor_line as editor_status;
/// What `mesimon doctor` says about notifications (T-282): whether they
/// are on, which rungs of the two ladders answered, and both sounds.
pub use notify::doctor_line as notify_status;
/// What `mesimon doctor` says about the link opener (`^k`, T-256).
pub use opener::doctor_line as opener_status;
/// What `mesimon doctor` says about the theme picks (`prefs.rs`).
pub use prefs::doctor_line as theme_status;
/// What `mesimon doctor` says about how a snoozed ticket comes back (T-74).
pub use prefs::snooze_doctor_line as snooze_status;
pub use prefs::status_line_doctor_line as status_line_status;
pub use prefs::train_doctor_line as train_status;
/// What `mesimon doctor` says about release checks — whether they are on, and
/// when they last answered. Exported because the checker lives here, beside
/// the offer it raises, and the doctor must not carry a second copy of the
/// rules for when it runs.
pub use release::doctor_line as update_check_status;

pub fn run(repo_root: &Path) -> Result<()> {
    // Capability detection runs exactly once, before raw mode and before any
    // PTY exists (06 §2.9 query hygiene; handovers reuse the cached answers).
    // Light/dark is the one rung that keeps asking — see `detect::GroundWatch`
    // — but only from inside the event loop, where nothing else owns stdin.
    let detected = detect::detect();
    // The two slots are loaded HERE and never in `App::new`, so no test app
    // ever reads the developer's own file.
    let prefs_path = prefs::prefs_path();
    let loaded = prefs_path.as_deref().map(prefs::load).unwrap_or_else(|| prefs::Loaded {
        prefs: Default::default(),
        write_barred: false,
        notice: None,
    });
    // The pin outranks the slot; the slot is the ground's.
    let flavor = detected.forced.unwrap_or(loaded.prefs.for_ground(detected.ground));
    let theme = theme::Theme::new(flavor, detected.profile);

    // Hard floor (07 §2.4): refuse to start below 60x20.
    if let Ok((w, h)) = ratatui::crossterm::terminal::size() {
        if w < layout::MIN_W || h < layout::MIN_H {
            eprintln!(
                "mesimon needs {}x{}; this terminal is {w}x{h}",
                layout::MIN_W,
                layout::MIN_H
            );
            std::process::exit(2);
        }
    }

    // From here to `init_terminal` the primary screen is what shows, and
    // after a `U` it holds whatever was last printed there (tmux's
    // `[detached …]` line, on a board that has been into a session). The
    // connect and the first snapshot are the whole wait; a second of it
    // earns a sentence (`client::LateWord`).
    let word = client::LateWord::new(
        std::time::Duration::from_secs(1),
        "mesimon: connecting to the daemon…",
    );
    let client = Client::connect(repo_root)?;
    let mut app = App::new(Box::new(client), repo_root.to_path_buf(), theme)?;
    drop(word);
    app.flavor_watch = detected.watch;
    app.ground = detected.ground;
    app.forced = detected.forced;
    app.prefs = loaded.prefs;
    app.prefs_path = prefs_path;
    app.prefs_write_barred = loaded.write_barred;
    // The merge train preference reaches the daemon now, not on the first
    // event: an armed board that sits quiet would otherwise never say so.
    app.reconcile_train();
    // Same for the status line's side (T-264): a daemon that outlived the
    // last board holds bottom until a board says top.
    app.reconcile_status_line();
    if let Some(notice) = loaded.notice {
        app.status = notice;
    }

    let mut terminal = init_terminal()?;
    // Set AFTER init_terminal, which is what runs (and caches) the probe —
    // asking before raw mode is on gets a false negative. This is the single
    // gate on every `Key::ShiftEnter` binding.
    app.rich_keys = kitty_keyboard_supported();
    // The word the note editor's `^g` hint wears — set here and never in
    // `App::new`, so no test app ever reads the developer's `$EDITOR`.
    app.editor_word = external::word();
    // What `^k` opens a URL with — same rule, same reason.
    app.opener = opener::find();
    // Whether a ticket may grow its own shell (T-300) — same rule again.
    app.ticket_shells = ticket_shells();
    // The board's outward voice (T-282), on a thread of its own since T-291:
    // it owns a second daemon connection and keeps speaking through a
    // handover, when this loop is stopped inside `cmd.status()`. Started
    // here and never in `App::new` — the rule the two ladders and `opener`
    // already follow — so no test app and no golden raises a banner, makes a
    // sound, or opens a second connection.
    app.notifier = Some(notifier::Notifier::start(repo_root, notify::find(), (&app.prefs).into()));
    // What holds the machine awake while an agent is mid-turn (T-288) —
    // resolved here and never in `App::new`, the same rule again, so no test
    // app takes a power assertion or forks a holder. `App::tick` drives it.
    app.caffeine = Some(caffeine::Caffeine::new(caffeine::find()));
    let result = event_loop(&mut terminal, &mut app);
    // The board is done with its terminal, so the notification thread's two
    // escape rungs stop writing to it NOW — taken under the same lock a
    // write takes, so nothing is half out — and then the thread itself goes.
    // A reload is why the order matters: `reexec` waits for the daemon it
    // just asked to stop, and a thread still holding a connection to it has
    // no business talking over that.
    app.saw_board(false);
    drop(app.notifier.take());
    // And the hold goes here, BEFORE `reexec`: the reload waits up to
    // `HANDOVER_MAX` for the daemon it asked to stop, and holding the
    // machine awake through that wait — or into the next image — is exactly
    // what a keep-awake feature must not do. (The pipe a spawned holder
    // reads closes on the `exec` anyway; this is the road that does not
    // depend on that.)
    drop(app.caffeine.take());
    restore_terminal()?;
    if result.is_ok() && app.pending_reexec {
        return reexec(repo_root);
    }
    result
}

/// U on `update ready`: swap this process for the new binary at our own
/// path. The daemon was asked to shut down first; wait for it to be gone so
/// the fresh TUI's connect-spawn doesn't race the old flock.
fn reexec(repo_root: &Path) -> Result<()> {
    // The primary screen is on display from here to the new client's first
    // draw, and it still holds whatever was last printed there — tmux's
    // `[detached (from session …)]` line, after any focus. Under a busy box
    // that line sat alone for a minute and read as a hang (dogfood
    // 2026-09-05). Blank it, as before an attach, and say what this is; the
    // new client adds its own word if the connect runs long (`run`).
    let _ = blank_primary_screen();
    eprintln!("mesimon: reloading…");
    // Socket unlinked AND lock released, up to `HANDOVER_MAX` — the fresh
    // client only tolerates a few seconds of no daemon, and a shutdown
    // (every pending settle committed through automove) is not bounded by
    // that. Two seconds of socket-polling here was what a `U` during a
    // parallel e2e run overran (2026-09-04).
    if !client::await_daemon_gone(repo_root) {
        eprintln!("mesimon: the daemon is still shutting down; starting the new client anyway");
    }
    use std::os::unix::process::CommandExt;
    let exe = mesimon_core::exe::current_exe()?;
    let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
    Err(anyhow::anyhow!("exec of the new binary failed: {err}"))
}

fn event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    // The notification thread writes an escape to this same stdout on its
    // two escape rungs, so the draw and the write take one lock (T-291).
    // Cloned out of `App` here: the guard has to outlive the borrow the draw
    // takes.
    let console = app.notifier.as_ref().map(crate::notifier::Notifier::console);
    loop {
        {
            let _held = console.as_deref().map(crate::notify::Console::drawing);
            terminal.draw(|f| ui::draw(f, app))?;
        }
        app.tick()?;

        // U on a ready update: fall out to `run`, which execs the new
        // binary once the terminal is restored.
        if app.pending_reexec {
            return Ok(());
        }

        // ^L (04 §2.2): throw away what we think is on screen and repaint
        // from nothing. This is the Terminal.app probe-garbage recovery, and
        // the answer to any stray write from a program that escaped its pane.
        if std::mem::take(&mut app.force_redraw) {
            terminal.clear()?;
        }

        // ^Z: suspend THIS client only. The terminal goes back the way we
        // found it, we stop, and on SIGCONT we take it again — the daemon and
        // every session run through all of it untouched.
        if std::mem::take(&mut app.pending_suspend) {
            app.saw_board(false);
            restore_terminal()?;
            // SAFETY: raising a signal at a point of our choosing, with the
            // terminal already restored, is the whole contract of ^Z.
            unsafe {
                libc::raise(libc::SIGTSTP);
            }
            *terminal = init_terminal()?;
            app.saw_board(true);
            terminal.clear()?;
        }

        // Focus handover: leave the terminal entirely, attach, come back (docs/19 §2).
        while let Some(argv) = app.pending_attach.take() {
            let cwd = app.pending_attach_cwd.take();
            // The board is about to stop being what the terminal shows, for
            // however long the user stays in that pane (T-291). Said before
            // the restore, so nothing writes an escape into the gap.
            app.saw_board(false);
            restore_terminal()?;
            blank_primary_screen()?;
            let ho = handover::run(&argv, cwd.as_deref());
            // Alt screen back up FIRST — the drain's settle sleep must not
            // leave the primary screen (stale logs) on display.
            *terminal = init_terminal()?;
            app.saw_board(true);
            handover::drain_stdin();
            if let Err(e) = ho {
                app.status = format!("focus failed: {e}");
                break;
            }
            app.after_handover()?; // may queue the post-GATE attach
        }

        // ^g in the note editor: the body goes to the user's own editor on
        // the terminal we give back for the duration (T-181) — the focus
        // handover's road, and the same blank-then-drain around it, since
        // vim leaves the alt screen the way tmux does.
        if let Some(req) = app.pending_external_edit.take() {
            app.saw_board(false);
            restore_terminal()?;
            blank_primary_screen()?;
            let out = external::edit_dir(&app.repo_root).and_then(|d| external::run(&req, &d));
            *terminal = init_terminal()?;
            app.saw_board(true);
            handover::drain_stdin();
            app.external_edit_done(out)?;
        }

        // A link going outside the terminal (T-256): detached, no handover,
        // the board stays up. Only a spawn that fails at once comes back.
        if let Some(argv) = app.pending_open.take() {
            if let Err(e) = opener::launch(&argv, None) {
                app.status = format!("could not run {}: {e}", argv[0]);
            }
        }

        if app.quit {
            return Ok(());
        }
    }
}

type Term = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>;

fn init_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // No EnableMouseCapture: we handle no mouse events, and capture steals the
    // terminal's native text selection.
    // Bracketed paste: the clipboard arrives as ONE `Event::Paste`, so a
    // multi-line paste into the composer is one title and a paste on the
    // board is inert, instead of both being typed as keystrokes (the
    // newline was an Enter, and it saved). `App::on_paste` takes it.
    // Focus reporting (DECSET 1004): the terminal says when somebody looks
    // away, which is what decides whether a notification is a banner or only
    // a sound (T-282). A terminal that ignores it sends nothing and the
    // presence rule falls back to the keyboard — never to silence.
    execute!(
        stdout,
        EnterAlternateScreen,
        SetCursorStyle::SteadyBar,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    // Kitty keyboard protocol distinguishes Shift+Enter and reports key
    // releases/repeats, so a fresh Up can leave the first ticket immediately.
    // The support probe is a terminal query, so it runs once per
    // process (query hygiene, 06 §2.9) — handovers reuse the cached answer.
    if kitty_keyboard_supported() {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
    }
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut t = ratatui::Terminal::new(backend)?;
    t.clear()?;
    Ok(t)
}

/// Cached kitty-protocol probe (crossterm writes CSI ? u and reads the reply,
/// so it needs raw mode — call only between enable/disable_raw_mode).
fn kitty_keyboard_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
    })
}

/// Staged restore (05 §13 baseline): reverse order of init, idempotent enough
/// to call around every handover.
fn restore_terminal() -> Result<()> {
    // Pop before leaving raw mode; terminals without the protocol ignore the
    // sequence, and a pop with nothing pushed is defined as a no-op.
    if kitty_keyboard_supported() {
        execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
    }
    disable_raw_mode()?;
    execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen,
        SetCursorStyle::DefaultUserShape
    )?;
    Ok(())
}

/// Handover-only: the primary screen holds pre-TUI output (build logs, shell
/// scrollback), and tmux's detach teardown restores it for at least one frame
/// before we can re-enter the alt screen (T-4 §4 — `?1049l` + `[detached …]`
/// land before the client exits). Push that content into scrollback and blank
/// the viewport so the unavoidable flash is empty, not a screenful of logs.
/// Scrolling (not `ED 2`) keeps the user's history reachable.
fn blank_primary_screen() -> Result<()> {
    use ratatui::crossterm::{cursor, style::Print};
    let (_, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    execute!(
        std::io::stdout(),
        cursor::MoveTo(0, rows.saturating_sub(1)),
        Print("\n".repeat(rows as usize)),
        cursor::MoveTo(0, 0),
    )?;
    Ok(())
}
