//! Attachment commands run on the board's single writer, after authorization.
use super::*;
use crate::attachments;
use base64::{engine::general_purpose::STANDARD, Engine};

impl Daemon {
    /// One picture, whole. A failed read is told apart by the ticket's own
    /// notes (T-609): an id one of them links is a picture whose bytes are
    /// on another machine (a team board joined without them), and any other
    /// id is simply not this ticket's.
    pub(super) fn read_attachment(&self, ticket: ulid::Ulid, attachment: ulid::Ulid) -> Response {
        let Some(t) = self.board.ticket(ticket) else { return no_such_ticket() };
        match attachments::read(&self.paths, &t.short_key, attachment) {
            Ok((meta, bytes)) => Response::Attachment { meta, data: STANDARD.encode(bytes) },
            Err(_) if !self.links_attachment(t, attachment) => {
                Response::Err { message: "no picture with that id on this ticket".into() }
            }
            Err(e) => Response::Err { message: format!("could not read picture: {e:#}") },
        }
    }

    /// Whether any note on `t` links the picture `attachment`. A note file
    /// that cannot be read links nothing.
    fn links_attachment(&self, t: &Ticket, attachment: ulid::Ulid) -> bool {
        t.notes.iter().any(|n| {
            crate::store::read_note(&self.paths, &t.short_key, n.id)
                .is_ok_and(|text| mesimon_core::attachment::references(&text).contains(&attachment))
        })
    }

    /// Commit `owner`'s finished uploads and then the note that links them:
    /// the desk's save, and a paired browser's (T-629) as `by`.
    pub(super) fn save_note_with_attachments(
        &mut self,
        owner: &attachments::Owner,
        ticket: ulid::Ulid,
        note: Option<ulid::Ulid>,
        text: String,
        uploads: Vec<ulid::Ulid>,
        by: &Principal,
    ) -> Response {
        let prepared = (|| -> Result<_> {
            let t = self.board.ticket(ticket).ok_or_else(|| anyhow::anyhow!("no such ticket"))?;
            anyhow::ensure!(note.is_none_or(|id| t.note(id).is_some()), "no such note");
            anyhow::ensure!(
                mesimon_core::board::sanitize_note(&text) == text,
                "note must fit within its text limit"
            );
            Ok((t.short_key.clone(), self.uploads.prepare(owner, &uploads, &text)?))
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
            response = self.write_note(ticket, note, text, None, by);
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
}
