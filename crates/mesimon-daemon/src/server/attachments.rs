//! Attachment commands run on the board's single writer, after authorization.
use super::*;
use crate::attachments;
use base64::{engine::general_purpose::STANDARD, Engine};

impl Daemon {
    pub(super) fn read_attachment(&self, ticket: ulid::Ulid, attachment: ulid::Ulid) -> Response {
        let Some(t) = self.board.ticket(ticket) else { return no_such_ticket() };
        match attachments::read(&self.paths, &t.short_key, attachment) {
            Ok((meta, bytes)) => Response::Attachment { meta, data: STANDARD.encode(bytes) },
            Err(e) => Response::Err { message: format!("could not read picture: {e:#}") },
        }
    }

    pub(super) fn save_note_with_attachments(
        &mut self,
        stream: &Arc<Mutex<UnixStream>>,
        ticket: ulid::Ulid,
        note: Option<ulid::Ulid>,
        text: String,
        uploads: Vec<ulid::Ulid>,
    ) -> Response {
        let prepared = (|| -> Result<_> {
            let t = self.board.ticket(ticket).ok_or_else(|| anyhow::anyhow!("no such ticket"))?;
            anyhow::ensure!(note.is_none_or(|id| t.note(id).is_some()), "no such note");
            anyhow::ensure!(
                mesimon_core::board::sanitize_note(&text) == text,
                "note must fit within its text limit"
            );
            Ok((t.short_key.clone(), self.uploads.prepare(stream, &uploads, &text)?))
        })();
        let (key, images) = match prepared {
            Ok(prepared) => prepared,
            Err(e) => return Response::Err { message: format!("could not save pictures: {e:#}") },
        };
        let mut response = Response::Ok;
        for (meta, bytes) in images {
            if let Err(e) = attachments::save(&self.paths, &key, &meta, &bytes) {
                response = Response::Err { message: format!("could not save picture: {e:#}") };
                break;
            }
        }
        if matches!(response, Response::Ok) {
            response = self.write_note(ticket, note, text, &Principal::Local);
        }
        if matches!(response, Response::NoteWritten { .. }) {
            self.uploads.committed(&uploads);
        } else if let Err(cleanup) = attachments::discard_files(&self.paths, &key, &uploads) {
            if let Response::Err { message } = &mut response {
                message.push_str(&format!("; picture cleanup failed: {cleanup:#}"));
            }
        }
        response
    }

    pub(super) fn create_ticket_with_note(
        &mut self,
        stream: &Arc<Mutex<UnixStream>>,
        column: String,
        title: String,
        workspace: Option<WorkspaceStrategy>,
        text: String,
        uploads: Vec<ulid::Ulid>,
    ) -> Response {
        let result = (|| -> Result<ulid::Ulid> {
            anyhow::ensure!(!self.columns_barred, "{}", self.barred_message("columns"));
            anyhow::ensure!(self.board.column(&column).is_some(), "no such column: {column}");
            let title = mesimon_core::board::sanitize_title(&title);
            anyhow::ensure!(!title.trim().is_empty(), "a ticket needs a title");
            anyhow::ensure!(
                mesimon_core::board::sanitize_note(&text) == text && !text.trim().is_empty(),
                "invalid note text"
            );
            let images = self.uploads.prepare(stream, &uploads, &text)?;
            self.board.next_key += 1;
            store::save_columns(&self.paths, &self.board)?;
            let workspace =
                workspace.or_else(|| self.board.column(&column).and_then(|c| c.settings.workspace));
            let last = self
                .board
                .column_tickets(&column)
                .last()
                .map(|t| t.order.clone())
                .unwrap_or_default();
            let now = now_iso();
            let note = mesimon_core::board::NoteMeta {
                id: ulid::Ulid::new(),
                name: mesimon_core::board::note_name(&text),
                rev: 1,
                created_at: now.clone(),
                edited_at: now.clone(),
                created_by: "local".into(),
                edited_by: "local".into(),
            };
            let ticket = Ticket {
                id: ulid::Ulid::new(),
                short_key: format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
                title,
                column,
                order: fracindex::between(&last, ""),
                created_at: now.clone(),
                created_by: "local".into(),
                created_from: None,
                entered_at: Some(now),
                previous_column: None,
                woke_at: None,
                manual_merge: false,
                execution_policy: Default::default(),
                import_origin: None,
                raised: None,
                workspace,
                tags: Vec::new(),
                notes: vec![note],
                archived: None,
            };
            let saved = (|| -> Result<()> {
                for (meta, bytes) in &images {
                    attachments::save(&self.paths, &ticket.short_key, meta, bytes)?;
                }
                store::save_note(&self.paths, &ticket.short_key, ticket.notes[0].id, &text)?;
                store::save_ticket(&self.paths, &ticket)
            })();
            if let Err(error) = saved {
                let cleanup = store::delete_ticket_dir(&self.paths, &ticket.short_key);
                return Err(error.context(format!("ticket save failed; cleanup: {cleanup:?}")));
            }
            let id = ticket.id;
            self.board.tickets.push(ticket);
            self.uploads.committed(&uploads);
            self.persist_and_notify();
            Ok(id)
        })();
        match result {
            Ok(id) => {
                let started = self.auto_run(id);
                Response::Created { id, started }
            }
            Err(e) => Response::Err { message: format!("could not create ticket: {e:#}") },
        }
    }
}
