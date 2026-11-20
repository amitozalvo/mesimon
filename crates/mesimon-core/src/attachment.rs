//! Ticket-local pictures. Pure limits, metadata and Markdown references.
mod file_type;
pub use file_type::FileType;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// The maximum raw file bytes one tool result can carry.
pub const READ_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePart {
    pub offset: usize,
    pub next_offset: usize,
    pub encoding: String,
    pub data: String,
}

pub const MAX_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 25_000_000;
pub const DRAFT_MAX_BYTES: usize = 50 * 1024 * 1024;
pub const CHUNK_BYTES: usize = 256 * 1024;
pub const PREFIX: &str = "mesimon-attachment:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUpload {
    pub ticket: Ulid,
    pub name: String,
}

/// A basename is display data. Only the daemon's ID becomes a filesystem path.
pub fn valid_basename(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 255
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
        && crate::text::scrub_text(name) == name
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Png,
    File {
        name: String,
        #[serde(default)]
        mime_type: FileType,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(default)]
    pub kind: Kind,
    pub id: Ulid,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}

pub fn target(id: Ulid) -> String {
    format!("{PREFIX}{id}")
}

pub fn parse_target(text: &str) -> Option<Ulid> {
    text.strip_prefix(PREFIX)?.parse().ok()
}

pub fn references(text: &str) -> Vec<Ulid> {
    crate::links::words(text)
        .into_iter()
        .filter_map(|link| match link.target {
            crate::links::Found::Attachment(id) => Some(id),
            _ => None,
        })
        .collect()
}

pub fn next_number(text: &str) -> usize {
    text.split("[Image #")
        .skip(1)
        .filter_map(|part| part.split_once(']')?.0.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_links_are_ticket_local_ids_and_numbers_survive_gaps() {
        let id = Ulid::new();
        let body = format!("Before [Image #1]({}) and [Image #4]({}).", target(id), target(id));
        assert_eq!(references(&body), vec![id]);
        assert_eq!(next_number(&body), 5);
        assert_eq!(parse_target("mesimon-attachment:../../other"), None);
        assert_eq!(parse_target("https://example.org/image.png"), None);
    }
}
