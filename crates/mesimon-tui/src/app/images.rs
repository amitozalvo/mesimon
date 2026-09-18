//! Clipboard completion and save-time upload of a draft's pictures.
use super::*;
use crate::image_paste::{self, DraftImage, Pasted};
use base64::{engine::general_purpose::STANDARD, Engine};
use mesimon_core::attachment;

impl App {
    pub(super) fn poll_image_paste(&mut self) -> bool {
        let Some(pending) = &mut self.pending_paste else { return false };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if !pending.timed_out && pending.started.elapsed() > Duration::from_secs(3) {
                    pending.timed_out = true;
                    if matches!(&self.mode, Mode::Editor(ed) if ed.draft_id == pending.editor) {
                        self.status =
                            "clipboard read timed out ∙ you can keep editing or save".into();
                        return true;
                    }
                }
                return false;
            }
            Err(_) => Err("clipboard reader stopped".into()),
        };
        let pending = self.pending_paste.take().expect("pending clipboard read");
        if pending.timed_out {
            return false;
        }
        let Mode::Editor(ed) = &mut self.mode else { return false };
        if ed.draft_id != pending.editor {
            return false;
        }
        if ed.body != pending.body || ed.focus != Field::Body {
            self.status = "note changed while reading clipboard ∙ paste again".into();
            return true;
        }
        match result {
            Err(message) => self.status = message,
            Ok(Pasted::Text(text)) => {
                let pasted = ed.body.paste(&text);
                self.status = if pasted.trimmed {
                    "paste trimmed ∙ note text limit reached"
                } else {
                    "text pasted"
                }
                .into();
            }
            Ok(Pasted::Image(png)) => {
                if let Err(e) = image_paste::check_draft(&ed.images, png.len()) {
                    self.status = e.to_string();
                    return true;
                }
                let number = attachment::next_number(ed.body.as_str())
                    .max(ed.images.iter().map(|i| i.number).max().unwrap_or(0).saturating_add(1));
                let image = DraftImage { id: ulid::Ulid::new(), number, png: Some(png.into()) };
                let marker = image_paste::marker(&image);
                let mut body = ed.body.clone();
                if body.paste(&marker).trimmed {
                    self.status =
                        "no room for a picture reference ∙ note text limit reached".into();
                    return true;
                }
                let mut images = ed.images.clone();
                images.push(image);
                if image_paste::pack(body.as_str(), &images).len()
                    > mesimon_core::board::NOTE_MAX_BYTES
                {
                    self.status =
                        "no room for a picture reference ∙ note text limit reached".into();
                    return true;
                }
                ed.body = body;
                ed.images = images;
                self.status = format!("{marker} pasted ∙ save keeps the picture");
            }
        }
        ed.esc_armed = false;
        ed.delete_armed = false;
        true
    }

    pub(super) fn upload_draft(
        &mut self,
        body: &str,
        draft: &[DraftImage],
    ) -> Result<(String, Vec<DraftImage>, Vec<ulid::Ulid>)> {
        let mut images: Vec<_> =
            draft.iter().filter(|i| body.contains(&image_paste::marker(i))).cloned().collect();
        anyhow::ensure!(
            image_paste::pack(body, &images).len() <= mesimon_core::board::NOTE_MAX_BYTES,
            "note text including image references exceeds 32 KiB"
        );
        let mut uploads = Vec::new();
        for image in &mut images {
            let Some(png) = &image.png else { continue };
            let mut upload = None;
            for (index, bytes) in png.chunks(attachment::CHUNK_BYTES).enumerate() {
                let offset = index * attachment::CHUNK_BYTES;
                match self.req(Command::UploadAttachment {
                    upload,
                    offset,
                    data: STANDARD.encode(bytes),
                    complete: offset + bytes.len() == png.len(),
                }) {
                    Response::AttachmentUploaded { upload: id } => {
                        if upload.is_none() {
                            uploads.push(id);
                        }
                        upload = Some(id);
                    }
                    response => {
                        let _ = self.req(Command::DiscardAttachmentUploads { uploads });
                        let message = match response {
                            Response::Err { message } => message,
                            _ => "daemon did not accept the picture upload".into(),
                        };
                        anyhow::bail!(message);
                    }
                }
            }
            image.id = upload.ok_or_else(|| anyhow::anyhow!("empty picture"))?;
            image.png = None;
        }
        Ok((image_paste::pack(body, &images), images, uploads))
    }
}
