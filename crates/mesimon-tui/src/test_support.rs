//! Headless key driving for daemon integration tests; no terminal or settings writes.
use crate::app::{App, Mode, RailRow, Screen};
use crate::client::Transport;
use crate::theme::{Flavor, Profile, Theme};
use anyhow::{bail, Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

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
