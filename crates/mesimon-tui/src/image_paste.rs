//! Desktop clipboard reads and the image portion of an editor draft.
use anyhow::{Context, Result};
use mesimon_core::attachment::{self, DRAFT_MAX_BYTES, MAX_BYTES, MAX_PIXELS};
use std::io::Cursor;
use std::sync::{mpsc, Arc};
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq)]
pub struct DraftImage {
    pub id: Ulid,
    pub number: usize,
    pub png: Option<Arc<[u8]>>,
}

pub enum Pasted {
    Text(String),
    Image(Vec<u8>),
}

pub struct Pending {
    pub started: std::time::Instant,
    pub timed_out: bool,
    pub editor: Ulid,
    pub body: crate::text::TextArea,
    pub result: mpsc::Receiver<Result<Pasted, String>>,
}

pub fn start(editor: Ulid, body: crate::text::TextArea) -> Result<Pending> {
    anyhow::ensure!(
        !["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
            .iter()
            .any(|key| std::env::var_os(key).is_some()),
        "image paste is unavailable over SSH; terminal text paste still works"
    );
    anyhow::ensure!(
        !crate::opener::is_wsl(),
        "image paste is unavailable under WSL; terminal text paste still works"
    );
    let (tx, result) = mpsc::channel();
    std::thread::Builder::new().name("mesimon-paste".into()).spawn(move || {
        let _ = tx.send(read().map_err(|e| format!("{e:#}")));
    })?;
    Ok(Pending { editor, body, result, started: std::time::Instant::now(), timed_out: false })
}

fn read() -> Result<Pasted> {
    let mut clipboard = arboard::Clipboard::new().context("desktop clipboard unavailable")?;
    match clipboard.get_image() {
        Ok(image) => Ok(Pasted::Image(encode(image)?)),
        Err(arboard::Error::ContentNotAvailable) => {
            let text =
                clipboard.get_text().context("clipboard has no supported picture or text")?;
            anyhow::ensure!(!text.is_empty(), "clipboard is empty");
            Ok(Pasted::Text(text))
        }
        Err(e) => Err(e).context("could not read clipboard picture"),
    }
}

fn encode(image: arboard::ImageData<'_>) -> Result<Vec<u8>> {
    let width = u32::try_from(image.width)?;
    let height = u32::try_from(image.height)?;
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "picture exceeds 25 megapixels"
    );
    let rgba = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .context("invalid clipboard pixels")?;
    let mut png = Cursor::new(Vec::new());
    rgba.write_to(&mut png, image::ImageFormat::Png)?;
    let png = png.into_inner();
    anyhow::ensure!(png.len() <= MAX_BYTES, "picture exceeds 10 MiB");
    Ok(png)
}

/// Keep only the compact label in the text editor; IDs ride the draft.
/// Other Markdown is untouched, including labels a user chose themselves.
pub fn unpack(text: &str) -> (String, Vec<DraftImage>) {
    let mut body = text.to_string();
    let mut images = Vec::new();
    for link in mesimon_core::links::extract(text) {
        let mesimon_core::links::Found::Attachment(id) = link.target else { continue };
        let Some(label) = link.label else { continue };
        let Some(number) = label.strip_prefix("Image #").and_then(|n| n.parse().ok()) else {
            continue;
        };
        let marker = format!("[{label}]");
        body = body.replace(&format!("{marker}({})", attachment::target(id)), &marker);
        images.push(DraftImage { id, number, png: None });
    }
    (body, images)
}

pub fn marker(image: &DraftImage) -> String {
    format!("[Image #{}]", image.number)
}

pub fn pack(body: &str, images: &[DraftImage]) -> String {
    let mut text = body.to_string();
    for image in images {
        let marker = marker(image);
        let mut expanded = String::new();
        let mut end = 0;
        for (at, _) in text.match_indices(&marker) {
            let after = at + marker.len();
            expanded.push_str(&text[end..after]);
            // A user can paste Markdown too. Its explicit target remains
            // authoritative, even when its label matches a draft picture.
            if !text[after..].starts_with('(') {
                expanded.push_str(&format!("({})", attachment::target(image.id)));
            }
            end = after;
        }
        expanded.push_str(&text[end..]);
        text = expanded;
    }
    text
}

pub fn check_draft(images: &[DraftImage], bytes: usize) -> Result<()> {
    let total: usize = images.iter().filter_map(|i| i.png.as_ref()).map(|p| p.len()).sum();
    anyhow::ensure!(
        bytes <= MAX_BYTES && total + bytes <= DRAFT_MAX_BYTES,
        "pictures exceed the 50 MiB draft limit (10 MiB per picture)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packing_never_overwrites_an_explicit_markdown_target() {
        let image = DraftImage { id: Ulid::new(), number: 1, png: None };
        let text = "[Image #1](https://example.org/picture.png) and [Image #1]";
        assert_eq!(
            pack(text, std::slice::from_ref(&image)),
            format!(
                "[Image #1](https://example.org/picture.png) and [Image #1]({})",
                attachment::target(image.id)
            )
        );
    }

    #[test]
    fn editor_hides_ids_but_save_round_trips_them() {
        let id = Ulid::new();
        let markdown = format!("Before [Image #3]({}) after", attachment::target(id));
        let (plain, images) = unpack(&markdown);
        assert_eq!(plain, "Before [Image #3] after");
        assert_eq!(pack(&plain, &images), markdown);
        assert_eq!(images[0].id, id);
        assert!(images[0].png.is_none());
    }

    #[test]
    fn clipboard_pixels_become_png_and_invalid_dimensions_are_refused() {
        let png = encode(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[1, 2, 3, 255]),
        })
        .unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.as_raw(), &[1, 2, 3, 255]);
        assert!(encode(arboard::ImageData {
            width: 100_000,
            height: 100_000,
            bytes: std::borrow::Cow::Borrowed(&[])
        })
        .is_err());
        assert!(encode(arboard::ImageData {
            width: 1,
            height: 1,
            bytes: std::borrow::Cow::Borrowed(&[])
        })
        .is_err());
        assert!(check_draft(&[], MAX_BYTES + 1).is_err());
    }
}
