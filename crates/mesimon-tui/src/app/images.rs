//! Clipboard completion and save-time upload of a draft's pictures.
use super::*;
use crate::image_paste::{self, DraftImage, Landing, Pasted};
use base64::{engine::general_purpose::STANDARD, Engine};
use mesimon_core::attachment;

/// The two texts a clipboard read lands in: the room's body keeps its
/// lines, and so does the ask field since T-1191.
trait Lands: Clone {
    fn put(&mut self, text: &str) -> crate::text::Pasted;
    fn text(&self) -> &str;
}

impl Lands for crate::text::TextArea {
    fn put(&mut self, text: &str) -> crate::text::Pasted {
        self.paste(text)
    }
    fn text(&self) -> &str {
        self.as_str()
    }
}

impl Lands for EditBuffer {
    fn put(&mut self, text: &str) -> crate::text::Pasted {
        self.paste_lines(text)
    }
    fn text(&self) -> &str {
        self.as_str()
    }
}

/// A read's answer into `body` at its cursor: text as text, a picture as
/// `[Image #N]` with the picture held in `images` until `keeps` (a note's
/// save, an ask's send) puts it up. What it did is the status line.
fn land<B: Lands>(
    body: &mut B,
    images: &mut Vec<DraftImage>,
    limit: usize,
    keeps: &str,
    result: Result<Pasted, String>,
) -> String {
    match result {
        Err(message) => message,
        Ok(Pasted::Text(text)) => if body.put(&text).trimmed {
            "paste trimmed ∙ text limit reached"
        } else {
            "text pasted"
        }
        .into(),
        Ok(Pasted::Image(png)) => {
            if let Err(e) = image_paste::check_draft(images, png.len()) {
                return e.to_string();
            }
            let number = attachment::next_number(body.text())
                .max(images.iter().map(|i| i.number).max().unwrap_or(0).saturating_add(1));
            let image = DraftImage { id: ulid::Ulid::new(), number, png: Some(png.into()) };
            let marker = image_paste::marker(&image);
            let mut grown = body.clone();
            let mut held = images.clone();
            held.push(image);
            if grown.put(&marker).trimmed || image_paste::pack(grown.text(), &held).len() > limit {
                return "no room for a picture reference ∙ text limit reached".into();
            }
            *body = grown;
            *images = held;
            format!("{marker} pasted ∙ {keeps} keeps the picture")
        }
    }
}

impl App {
    pub(super) fn poll_image_paste(&mut self) -> bool {
        let Some(pending) = &mut self.pending_paste else { return false };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if !pending.timed_out && pending.started.elapsed() > Duration::from_secs(3) {
                    pending.timed_out = true;
                    if landing_open(&self.mode, &pending.landing) {
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
        if pending.timed_out || !landing_open(&self.mode, &pending.landing) {
            return false;
        }
        const CHANGED: &str = "text changed while reading clipboard ∙ paste again";
        let one_line = self.board_field();
        self.status = match (&mut self.mode, pending.landing) {
            (Mode::Editor(ed), Landing::Editor { body, .. }) => {
                if ed.body != body || ed.focus != Field::Body {
                    CHANGED.into()
                } else {
                    let limit = ed.purpose.body_limit();
                    // What keeps the picture: a note's save, or an ask's send (T-853).
                    let keeps = if ed.asking() { "send" } else { "save" };
                    ed.esc_armed = false;
                    ed.delete_armed = false;
                    land(&mut ed.body, &mut ed.images, limit, keeps, result)
                }
            }
            (
                Mode::Input { purpose: InputPurpose::Prompt { images, walk, .. }, buffer },
                Landing::Field(was),
            ) => {
                if *buffer != was {
                    CHANGED.into()
                } else {
                    let limit = buffer.limit();
                    let status = land(buffer, images, limit, "send", result);
                    // The board's field is one line (T-1212).
                    if one_line {
                        buffer.flatten_lines();
                    }
                    // The words are the person's own now, not a recalled ask.
                    *walk = None;
                    status
                }
            }
            _ => return false,
        };
        true
    }

    /// `^v` in the ask field (T-1210): the clipboard read the room's `^v`
    /// starts, landing in the field itself — a picture as `[Image #N]`,
    /// text as text — so a picture never grows the field into the room.
    pub(super) fn read_clipboard_into_field(&mut self, buffer: &EditBuffer) {
        if self.pending_paste.is_some() {
            self.status = "clipboard read already pending".into();
            return;
        }
        match image_paste::start(Landing::Field(buffer.clone())) {
            Ok(pending) => {
                self.pending_paste = Some(pending);
                self.status = "reading clipboard".into();
            }
            Err(e) => self.status = e.to_string(),
        }
    }

    /// The ask field's send (T-1210): the pictures its words still name go
    /// up first, as the room's `^s` sends them (T-853), and the words go
    /// linked to them. A refusal puts the field back with its pictures.
    pub(super) fn send_field(&mut self, mut purpose: InputPurpose, text: String) -> Result<()> {
        let images = match &mut purpose {
            InputPurpose::Prompt { images, .. } => std::mem::take(images),
            _ => Vec::new(),
        };
        if images.is_empty() {
            return self.commit_prompt(purpose, text, Vec::new());
        }
        match self.upload_draft(&text, &images, mesimon_core::command::PROMPT_MAX_BYTES) {
            Ok((linked, _, uploads)) => {
                let sent = self.commit_prompt(purpose, linked, uploads.clone());
                if !uploads.is_empty() {
                    let _ = self.req(Command::DiscardAttachmentUploads { uploads });
                }
                sent
            }
            Err(e) => {
                self.status = format!("could not send pictures: {e}");
                if let InputPurpose::Prompt { images: held, .. } = &mut purpose {
                    *held = images;
                }
                self.mode = Mode::Input { purpose, buffer: history_field(&text, self.board_field()) };
                Ok(())
            }
        }
    }

    /// `limit` is the most bytes the linked text may hold: a note's, or a
    /// prompt's for an ask (T-853).
    pub(super) fn upload_draft(
        &mut self,
        body: &str,
        draft: &[DraftImage],
        limit: usize,
    ) -> Result<(String, Vec<DraftImage>, Vec<ulid::Ulid>)> {
        let mut images: Vec<_> =
            draft.iter().filter(|i| body.contains(&image_paste::marker(i))).cloned().collect();
        anyhow::ensure!(
            image_paste::pack(body, &images).len() <= limit,
            "text including image references exceeds {} KiB",
            limit / 1024
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

/// Whether the place a read was started for is still on screen.
fn landing_open(mode: &Mode, landing: &Landing) -> bool {
    match (mode, landing) {
        (Mode::Editor(ed), Landing::Editor { draft, .. }) => ed.draft_id == *draft,
        (Mode::Input { purpose: InputPurpose::Prompt { .. }, .. }, Landing::Field(_)) => true,
        _ => false,
    }
}
