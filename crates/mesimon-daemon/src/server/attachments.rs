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
}
