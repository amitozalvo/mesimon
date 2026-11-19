//! Headless key driving for daemon integration tests; no terminal or settings writes.
use crate::app::{App, Mode, RailRow, Screen};
use crate::client::Transport;
use crate::theme::{Flavor, Profile, Theme};
use anyhow::{bail, Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

/// Exercise the real preview reader and asynchronous Enter road without
/// taking the test runner's terminal. The resulting argv is the same one
/// the outer event loop hands to tmux.
pub fn preview_then_enter(
    client: Box<dyn Transport>,
    repo: PathBuf,
    sock: PathBuf,
    ticket: ulid::Ulid,
    from_board: bool,
    marker: &str,
) -> Result<Vec<String>> {
    use std::time::{Duration, Instant};
    let mut app = App::new(client, repo, Theme::new(Flavor::Graphite, Profile::Mono))?;
    app.refresh()?;
    app.tmux_sock = Some(sock);
    let column = &app.board.ticket(ticket).context("ticket missing")?.column;
    app.cursor_col = app.board.sorted_columns().iter().position(|c| &c.name == column).unwrap();
    app.cursor_row = Some(0);
    app.screen = Screen::Ticket { ticket, rail_idx: 0 };
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        terminal.draw(|frame| crate::ui::draw(frame, &app))?;
        app.pump()?;
        let frame = terminal.backend().buffer();
        let text: String = frame.content.iter().map(|cell| cell.symbol()).collect();
        if text.contains(marker) {
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "live preview never rendered {marker}");
        std::thread::sleep(Duration::from_millis(10));
    }
    if from_board {
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)?;
    }
    app.handle_key(KeyCode::Enter, KeyModifiers::NONE)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.pump()?;
        if app.pending_attach.is_some() && app.prepare_handover() {
            return Ok(app.pending_attach.take().expect("pending above"));
        }
        anyhow::ensure!(Instant::now() < deadline, "Enter never attached: {}", app.status);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Drives the actual ticket-row Shift+Enter and Esc against the supplied transport.
/// `None` selects the unadopted terminal, proving that cancellation leaves it unadopted.
pub fn capture_then_cancel(
    client: Box<dyn Transport>,
    repo: PathBuf,
    ticket: ulid::Ulid,
    shell: Option<uuid::Uuid>,
) -> Result<String> {
    capture(client, repo, ticket, shell, false)
}

/// Drives the preview's send gesture, including deferred adoption for a terminal.
pub fn capture_then_send(
    client: Box<dyn Transport>,
    repo: PathBuf,
    ticket: ulid::Ulid,
    shell: Option<uuid::Uuid>,
) -> Result<String> {
    capture(client, repo, ticket, shell, true)
}

fn capture(
    client: Box<dyn Transport>,
    repo: PathBuf,
    ticket: ulid::Ulid,
    shell: Option<uuid::Uuid>,
    send: bool,
) -> Result<String> {
    let mut app = App::new(client, repo, Theme::new(Flavor::Graphite, Profile::Mono))?;
    // The normal event loop refreshes before the first ticket-page interaction.
    app.refresh()?;
    let rail_idx = app
        .rail_rows(ticket)
        .iter()
        .position(|row| match (row, shell) {
            (RailRow::Session(rec), Some(id)) => rec.id == id,
            (RailRow::Terminal(_), None) => true,
            _ => false,
        })
        .context("shell row missing")?;
    app.screen = Screen::Ticket { ticket, rail_idx };
    app.rich_keys = true;
    app.handle_key(KeyCode::Enter, KeyModifiers::SHIFT)?;
    let Mode::Editor(ed) = &app.mode else { bail!("capture did not open: {}", app.status) };
    let text = ed.body.as_str().to_owned();
    if send {
        app.handle_key(KeyCode::Char('s'), KeyModifiers::CONTROL)?;
    } else {
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)?;
    }
    if !matches!(app.mode, Mode::Normal) {
        bail!("capture editor stayed open: {}", app.status)
    }
    Ok(text)
}
