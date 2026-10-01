//! The ratatui board client. Owns its own terminal and the staged restore
//! discipline (docs/05 §13) — exercised on every focus handover (docs/19 §2).

mod app;
mod appearance;
mod caffeine;
mod caffeine_watch;
mod clipboard;
mod creature;
mod image_paste;
mod keys;
// Public so an integration test can drive the real connect path (the
// build-skew daemon restart lives in it); the TUI itself uses it internally.
pub mod client;
mod detach_guide;
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
mod qr;
mod quiet;
mod release;
mod rich;
mod strike;
mod tags;
mod text;
mod theme;
mod title;
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
pub use detach_guide::run as run_detach_guide;
/// What `mesimon doctor` says about the note editor's `^g`: which program
/// opens, and which variable named it.
pub use external::doctor_line as editor_status;
/// What `mesimon doctor` says about notifications (T-282): whether they
/// are on, which rungs of the two ladders answered, and both sounds.
pub use notify::doctor_line as notify_status;
/// What `mesimon doctor` says about the link opener (`^k`, T-256).
pub use opener::doctor_line as opener_status;
pub use prefs::board_doctor_line as board_prefs_status;
/// What `mesimon doctor` says about the theme picks (`prefs.rs`).
pub use prefs::doctor_line as theme_status;
/// What `mesimon doctor` says about the board's reply row (T-365).
pub use prefs::peek_doctor_line as peek_status;
/// What `mesimon doctor` says about how a snoozed ticket comes back (T-74).
pub use prefs::snooze_doctor_line as snooze_status;
pub use prefs::status_line_doctor_line as status_line_status;
/// What `mesimon doctor` says about the terminal's own tab (T-492).
pub use prefs::tab_title_doctor_line as terminal_status;
pub use prefs::train_doctor_line as train_status;
/// What `mesimon doctor` says about release checks — whether they are on, and
/// when they last answered. Exported because the checker lives here, beside
/// the offer it raises, and the doctor must not carry a second copy of the
/// rules for when it runs.
pub use release::doctor_line as update_check_status;
/// `mesimon update` — the same checker and installer, asked from a shell.
pub use release::update_command;

pub fn run(repo_root: &Path) -> Result<()> {
    // The two slots are loaded HERE and never in `App::new`, so no test app
    // ever reads the developer's own file.
    let prefs_path = prefs::prefs_path();
    let loaded = prefs_path.as_deref().map(prefs::load).unwrap_or_else(|| prefs::Loaded {
        prefs: Default::default(),
        write_barred: false,
        notice: None,
    });
    // This board's overrides (T-361), the sparse file under the repo's
    // state dir. Same rule: loaded here, never in `App::new`.
    let board_prefs_path = prefs::board_prefs_path(repo_root);
    let board = board_prefs_path.as_deref().map(prefs::load_board).unwrap_or_default();
    let resolved = loaded.prefs.overlay(&board.prefs);
    // Capability detection runs exactly once, before raw mode and before any
    // PTY exists (06 §2.9 query hygiene; handovers reuse the cached answers).
    // A board that follows the OS appearance (T-485) asks the OS first — a
    // subprocess, no tty — and with an answer the terminal is not asked at
    // all. Nothing re-asks the terminal later, ever: `appearance::Watch`
    // is the live half, and it asks the OS on a thread of its own.
    let known = resolved.follow_os.then(appearance::probe).flatten();
    let detected = detect::detect(known);
    // The pin outranks the slot; the slot is the ground's.
    let flavor = detected.forced.unwrap_or(resolved.for_ground(detected.ground));
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
    app.ground = detected.ground;
    app.forced = detected.forced;
    // The OS probe, handed over here and never in `App::new`, so no test
    // app spawns a subprocess; `resolve_prefs` below is what arms the watch.
    app.appearance_probe = Some(appearance::probe);
    app.machine_prefs = loaded.prefs;
    app.prefs_path = prefs_path;
    app.prefs_write_barred = loaded.write_barred;
    app.board_prefs = board.prefs;
    app.board_prefs_path = board_prefs_path;
    app.board_prefs_write_barred = board.write_barred;
    app.resolve_prefs();
    app.arm_appearance();
    // The merge train preference reaches the daemon now, not on the first
    // event: an armed board that sits quiet would otherwise never say so.
    app.reconcile_train();
    // `reconcile_train` pushes only an ON, so one board never disarms
    // another's — but a board that SETS the train, off included, is a choice
    // about this repo's own daemon, and a daemon this board armed last
    // session must hear the off (T-361).
    if app.board_prefs.is_set(mesimon_core::prefs::PrefKey::MergeTrain) {
        app.push_automation();
    }
    // Same for the status line's side (T-264): a daemon that outlived the
    // last board holds bottom until a board says top.
    app.reconcile_status_line();
    let notices: Vec<String> = [loaded.notice, board.notice].into_iter().flatten().collect();
    if !notices.is_empty() {
        app.status = notices.join(" ∙ ");
    }

    let mut terminal = init_terminal()?;
    // Set AFTER init_terminal, which is what runs (and caches) the probe —
    // asking before raw mode is on gets a false negative. This is the single
    // gate on every `Key::ShiftEnter` binding.
    app.rich_keys = kitty_keyboard_supported();
    // Which terminal the tab belongs to (T-492) — set here and never in
    // `App::new`, so no test app reads a developer's terminal.
    app.terminal = title::terminal();
    // The word the note editor's `^g` hint wears — set here and never in
    // `App::new`, so no test app ever reads the developer's `$EDITOR`.
    app.editor_word = external::word();
    // What `^k` opens a URL with — same rule, same reason.
    app.opener = opener::find();
    // Whether a ticket may grow its own shell (T-300) — same rule again.
    app.ticket_shells = ticket_shells();
    // Board sharing (T-335): a development build offers it; a release build
    // only with `MESIMON_TEAMS=1`, until it ships.
    // Mesophon follows the same development-build default as Teams.
    app.mesophon_available =
        cfg!(debug_assertions) || std::env::var("MESIMON_MESOPHON").as_deref() == Ok("1");
    app.teams = app.teams
        || app.mesophon_available
        || std::env::var_os("MESIMON_TEAMS").is_some_and(|v| v == "1");
    // The board's outward voice (T-282), on a thread of its own since T-291:
    // it owns a second daemon connection and keeps speaking through a
    // handover, when this loop is stopped inside `cmd.status()`. Started
    // here and never in `App::new` — the rule the two ladders and `opener`
    // already follow — so no test app and no golden raises a banner, makes a
    // sound, or opens a second connection.
    app.notifier = Some(notifier::Notifier::start(repo_root, notify::find(), (&app.prefs).into()));
    // What holds the machine awake while an agent is mid-turn (T-288) —
    // resolved here and never in `App::new`, the same rule again, so no test
    // app takes a power assertion or forks a holder. Its observer keeps
    // tracking activity while the terminal is handed to an agent pane.
    app.caffeine = Some(caffeine_watch::Monitor::start(
        repo_root,
        caffeine::find(),
        app.prefs.keep_awake,
        &app.board,
        &app.pending,
    ));
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
    if let (Ok(()), Some(root)) = (&result, app.pending_switch.take()) {
        return switch_board(&root);
    }
    result
}

/// Enter on a team board (T-335): this process becomes `mesimon open
/// <root>` — a board is one process per root, and the joined board's daemon
/// is a different one, which the new client's connect spawns as usual.
/// This board's daemon stays up; nothing here asked it to stop.
fn switch_board(root: &Path) -> Result<()> {
    let _ = blank_primary_screen();
    eprintln!("mesimon: opening {}…", root.display());
    use std::os::unix::process::CommandExt;
    let exe = mesimon_core::exe::current_exe()?;
    let err = std::process::Command::new(exe).arg("open").arg(root).exec();
    Err(anyhow::anyhow!("exec of mesimon open failed: {err}"))
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

fn event_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    // The notification thread writes an escape to this same stdout on its
    // two escape rungs, so the draw and the write take one lock (T-291).
    // Cloned out of `App` here: the guard has to outlive the borrow the draw
    // takes.
    let console = app.notifier.as_ref().map(crate::notifier::Notifier::console);
    // The terminal's own tab (T-492): named and marked after the board
    // while its rows are on, under the same lock as the draw, so no escape
    // lands inside a frame or a banner. `finish` before every exit and
    // every suspend gives the terminal its own state back.
    let mut tab = title::Tab::default();
    loop {
        {
            let _held = console.as_deref().map(crate::notify::Console::drawing);
            // Every tick renders, and only a frame that changed writes — an
            // idle board sends the tty nothing, so a background iTerm2 tab
            // does not spin its activity indicator (T-496).
            quiet::draw(terminal, |f| ui::draw(f, app))?;
            let mut out = std::io::stdout();
            // The shin PNGs, on the first frame the icon row wants them:
            // a write under the state dir, so it happens here and not in
            // `App`. A failure leaves the row inert rather than the loop.
            if app.prefs.tab_icon && app.icons.is_none() {
                app.icons = title::shin_icons(&app.repo_root).ok();
            }
            tab.sync(&mut out, &app.tab_frame(false))?;
        }

        app.tick()?;

        // U on a ready update: fall out to `run`, which execs the new
        // binary once the terminal is restored.
        if app.pending_reexec {
            tab.finish(&mut std::io::stdout())?;
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
            // The shell's prompt is about to own the title: pop ours, and
            // the next frame's `sync` pushes it again on the way back.
            tab.finish(&mut std::io::stdout())?;
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
            // The pane's own title never reaches the tab — the private
            // server keeps `set-titles` off — so the tab reads the ticket
            // for as long as the user is in there (T-492).
            tab.sync(&mut std::io::stdout(), &app.tab_frame(true))?;
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
            tab.finish(&mut std::io::stdout())?;
            return Ok(());
        }
    }
}

type Term = ratatui::Terminal<quiet::Quiet<std::io::Stdout>>;

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
    let mut t = ratatui::Terminal::new(quiet::Quiet::new(stdout))?;
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
        settle_key_reports();
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

/// The pop is written, not yet obeyed, and the key that started this handover
/// is often still held: under the flags its RELEASE is reported as
/// `CSI code;mods:3 u`, and it lands on stdin after the loop has stopped
/// reading — where the tmux client inherits it. tmux 3.6a's CSI-u parser
/// (`tty_keys_extended_key`) stops at the `:`, so the report is not a key but
/// text, and `49;2:3u` was typed into the `!` shell on a machine where the
/// daemon's spawn took longer than a keypress (T-383). A cursor-position
/// query is the fence: the terminal answers it only after it has processed
/// the pop, so a report queued before the answer was sent under the flags and
/// is dropped here, and one after it is never sent. Nothing pending is
/// typeahead worth keeping — `handover::drain_stdin` discards the same on the
/// way back. Kitty-capable terminals answer CPR; crossterm gives up after 2 s
/// on one that does not, and the handover goes on.
fn settle_key_reports() {
    use ratatui::crossterm::event::{poll, read};
    // CSI 6 n. Every event crossterm reads past on the way to the answer is
    // kept in its queue, which the loop below empties.
    let _ = ratatui::crossterm::cursor::position();
    while matches!(poll(std::time::Duration::ZERO), Ok(true)) {
        if read().is_err() {
            break;
        }
    }
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
