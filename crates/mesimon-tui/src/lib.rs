//! The ratatui board client. Owns its own terminal and the staged restore
//! discipline (docs/05 §13) — exercised on every focus handover (docs/19 §2).

mod app;
mod client;
mod detect;
mod glyphs;
mod handover;
mod layout;
mod peek;
mod text;
mod theme;
mod ui;
mod update;

use std::path::Path;

use anyhow::Result;
use ratatui::crossterm::event::DisableMouseCapture;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use app::App;
use client::Client;

pub fn run(repo_root: &Path) -> Result<()> {
    // Capability + light/dark detection runs exactly once, before raw mode,
    // before any PTY exists, and never again for the process lifetime
    // (06 §2.9 query hygiene; handovers reuse the cached theme).
    let theme = detect::detect();

    // Hard floor (07 §2.4): refuse to start below 60x20.
    if let Ok((w, h)) = ratatui::crossterm::terminal::size() {
        if w < layout::MIN_W || h < layout::MIN_H {
            eprintln!("mesimon needs {}x{}; this terminal is {w}x{h}", layout::MIN_W, layout::MIN_H);
            std::process::exit(2);
        }
    }

    let client = Client::connect(repo_root)?;
    let mut app = App::new(Box::new(client), repo_root.to_path_buf(), theme)?;

    let mut terminal = init_terminal()?;
    let result = event_loop(&mut terminal, &mut app);
    restore_terminal()?;
    if result.is_ok() && app.pending_reexec {
        return reexec(repo_root);
    }
    result
}

/// U on `update ready`: swap this process for the new binary at our own
/// path. The daemon was asked to shut down first; wait for its socket to
/// vanish so the fresh TUI's connect-spawn doesn't race the old flock.
fn reexec(repo_root: &Path) -> Result<()> {
    if let Ok(paths) = mesimon_daemon::Paths::for_repo(repo_root) {
        let sock = paths.orch_sock();
        for _ in 0..20 {
            if !sock.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
    Err(anyhow::anyhow!("exec of the new binary failed: {err}"))
}

fn event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        app.tick()?;

        // U on a ready update: fall out to `run`, which execs the new
        // binary once the terminal is restored.
        if app.pending_reexec {
            return Ok(());
        }

        // Focus handover: leave the terminal entirely, attach, come back (docs/19 §2).
        while let Some(argv) = app.pending_attach.take() {
            restore_terminal()?;
            let ho = handover::run(&argv);
            *terminal = init_terminal()?;
            if let Err(e) = ho {
                app.status = format!("focus failed: {e}");
                break;
            }
            app.after_handover()?; // may queue the post-GATE attach
        }

        if app.quit {
            return Ok(());
        }
    }
}

fn init_terminal() -> Result<ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // No EnableMouseCapture: we handle no mouse events, and capture steals the
    // terminal's native text selection.
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut t = ratatui::Terminal::new(backend)?;
    t.clear()?;
    Ok(t)
}

/// Staged restore (05 §13 baseline): reverse order of init, idempotent enough
/// to call around every handover.
fn restore_terminal() -> Result<()> {
    disable_raw_mode()?;
    execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    Ok(())
}
