//! Bounded, connection-owned uploads and ticket-local immutable PNG files.
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::{store, Paths};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use mesimon_core::attachment::{Attachment, CHUNK_BYTES, DRAFT_MAX_BYTES, MAX_BYTES, MAX_PIXELS};
use ulid::Ulid;

/// Who may add to an upload and commit it: the desk's connection, gone with
/// its socket, or a paired browser's grant (T-629), which outlives the
/// browser's reconnects and is let go by the ten-minute idle rule or a revoke.
#[derive(Clone)]
pub(crate) enum Owner {
    Stream(Weak<Mutex<UnixStream>>),
    Grant(String),
}

impl Owner {
    pub fn stream(stream: &Arc<Mutex<UnixStream>>) -> Self {
        Self::Stream(Arc::downgrade(stream))
    }

    fn alive(&self) -> bool {
        match self {
            Self::Stream(s) => s.strong_count() > 0,
            Self::Grant(_) => true,
        }
    }

    fn is(&self, other: &Owner) -> bool {
        match (self, other) {
            (Self::Stream(a), Self::Stream(b)) => a.ptr_eq(b),
            (Self::Grant(a), Self::Grant(b)) => a == b,
            _ => false,
        }
    }
}

struct Upload {
    owner: Owner,
    bytes: Vec<u8>,
    meta: Option<Attachment>,
    touched: Instant,
}

#[derive(Default)]
pub(crate) struct Uploads(HashMap<Ulid, Upload>);

impl Uploads {
    pub fn prune(&mut self) {
        self.0.retain(|_, u| u.owner.alive() && u.touched.elapsed() < Duration::from_secs(600));
    }

    pub fn chunk(
        &mut self,
        owner: &Owner,
        id: Option<Ulid>,
        offset: usize,
        data: &str,
        complete: bool,
    ) -> Result<Ulid> {
        self.prune();
        anyhow::ensure!(data.len() <= CHUNK_BYTES.div_ceil(3) * 4, "attachment chunk too large");
        let bytes = STANDARD.decode(data).context("invalid attachment base64")?;
        anyhow::ensure!(
            !bytes.is_empty() && bytes.len() <= CHUNK_BYTES,
            "invalid attachment chunk size"
        );
        anyhow::ensure!(
            self.0.values().map(|u| u.bytes.len()).sum::<usize>() + bytes.len() <= DRAFT_MAX_BYTES,
            "attachment staging is full; retry after other uploads finish"
        );
        let id = match id {
            Some(id) => id,
            None => {
                anyhow::ensure!(offset == 0, "first attachment chunk must start at zero");
                anyhow::ensure!(self.0.len() < 128, "too many pending attachments");
                let id = Ulid::new();
                self.0.insert(
                    id,
                    Upload {
                        owner: owner.clone(),
                        bytes: Vec::new(),
                        meta: None,
                        touched: Instant::now(),
                    },
                );
                id
            }
        };
        let u = self.0.get_mut(&id).context("attachment upload expired; retry the save")?;
        anyhow::ensure!(u.owner.is(owner), "attachment upload belongs to another connection");
        anyhow::ensure!(
            u.meta.is_none() && offset == u.bytes.len(),
            "attachment upload offset mismatch"
        );
        anyhow::ensure!(offset + bytes.len() <= MAX_BYTES, "picture exceeds 10 MiB");
        u.bytes.extend(bytes);
        u.touched = Instant::now();
        if complete {
            match validate(id, &u.bytes) {
                Ok(meta) => u.meta = Some(meta),
                Err(e) => {
                    self.0.remove(&id);
                    return Err(e);
                }
            }
        }
        Ok(id)
    }

    pub fn prepare(
        &self,
        owner: &Owner,
        ids: &[Ulid],
        text: &str,
    ) -> Result<Vec<(Attachment, Vec<u8>)>> {
        let refs = mesimon_core::attachment::references(text);
        let mut result = Vec::new();
        for id in ids {
            anyhow::ensure!(refs.contains(id), "attachment upload has no note reference");
            anyhow::ensure!(
                !result.iter().any(|(m, _): &(Attachment, Vec<u8>)| m.id == *id),
                "duplicate attachment handle"
            );
            let u = self.0.get(id).context("attachment upload expired; retry the save")?;
            anyhow::ensure!(u.owner.is(owner), "attachment upload belongs to another connection");
            result.push((
                u.meta.clone().context("attachment upload is incomplete")?,
                u.bytes.clone(),
            ));
        }
        Ok(result)
    }

    pub fn discard(&mut self, owner: &Owner, ids: &[Ulid]) {
        for id in ids {
            if self.0.get(id).is_some_and(|u| u.owner.is(owner)) {
                self.0.remove(id);
            }
        }
    }

    /// Every upload `owner` holds: a revoked grant's.
    pub fn discard_all(&mut self, owner: &Owner) {
        self.0.retain(|_, u| !u.owner.is(owner));
    }

    pub fn committed(&mut self, ids: &[Ulid]) {
        for id in ids {
            self.0.remove(id);
        }
    }
}

pub fn validate(id: Ulid, bytes: &[u8]) -> Result<Attachment> {
    anyhow::ensure!(bytes.len() <= MAX_BYTES, "picture exceeds 10 MiB");
    let dimensions = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png)
        .into_dimensions()?;
    let (width, height) = dimensions;
    anyhow::ensure!(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "picture exceeds 25 megapixels"
    );
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PIXELS * 8);
    reader.limits(limits);
    reader.decode().context("invalid PNG picture")?;
    Ok(Attachment { id, width, height, bytes: bytes.len() })
}

pub fn directory(paths: &Paths, key: &str) -> PathBuf {
    paths.board_dir.join("board/tickets").join(key).join("attachments")
}

fn regular(path: &Path) -> Result<()> {
    anyhow::ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "attachment is not a regular file"
    );
    Ok(())
}

fn bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    regular(path)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= limit, "attachment file too large");
    Ok(bytes)
}

fn check_dir(dir: &Path) -> Result<()> {
    anyhow::ensure!(
        std::fs::symlink_metadata(dir)?.file_type().is_dir(),
        "attachment directory is not a directory"
    );
    anyhow::ensure!(
        std::fs::symlink_metadata(dir.parent().context("missing ticket directory")?)?
            .file_type()
            .is_dir(),
        "ticket directory is not a directory"
    );
    Ok(())
}

pub fn save(paths: &Paths, key: &str, meta: &Attachment, bytes: &[u8]) -> Result<()> {
    let dir = directory(paths, key);
    std::fs::create_dir_all(&dir)?;
    check_dir(&dir)?;
    store::write_atomic_bytes(&dir.join(format!("{}.png", meta.id)), bytes, 0o644)?;
    store::write_atomic(
        &dir.join(format!("{}.json", meta.id)),
        &serde_json::to_string(meta)?,
        0o644,
    )
}

/// Roll back only IDs from a validated, still-owned upload batch. These IDs
/// cannot belong to a previously committed attachment.
pub(crate) fn discard_files(paths: &Paths, key: &str, ids: &[Ulid]) -> Result<()> {
    let dir = directory(paths, key);
    if !dir.exists() {
        return Ok(());
    }
    check_dir(&dir)?;
    for id in ids {
        for extension in ["png", "json", "tmp"] {
            match std::fs::remove_file(dir.join(format!("{id}.{extension}"))) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(())
}

pub fn read(paths: &Paths, key: &str, id: Ulid) -> Result<(Attachment, Vec<u8>)> {
    let dir = directory(paths, key);
    check_dir(&dir).context("image unavailable on this machine")?;
    let meta: Attachment = serde_json::from_slice(
        &bounded(&dir.join(format!("{id}.json")), 4096)
            .context("image unavailable on this machine")?,
    )?;
    anyhow::ensure!(meta.id == id, "attachment metadata mismatch");
    let bytes = bounded(&dir.join(format!("{id}.png")), MAX_BYTES)?;
    anyhow::ensure!(meta.bytes == bytes.len(), "attachment size mismatch");
    Ok((meta, bytes))
}

pub fn collect(paths: &Paths, key: &str) -> Result<Vec<(Attachment, Vec<u8>)>> {
    let dir = directory(paths, key);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    check_dir(&dir)?;
    let mut images = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok())
            else {
                bail!("invalid attachment filename")
            };
            images.push(read(paths, key, id)?);
        }
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn stream() -> Arc<Mutex<UnixStream>> {
        let (a, _b) = UnixStream::pair().unwrap();
        Arc::new(Mutex::new(a))
    }

    #[test]
    fn upload_is_ordered_owned_and_validated_before_it_can_be_saved() {
        let mut uploads = Uploads::default();
        let stream = stream();
        let owner = Owner::stream(&stream);
        let bytes = png();
        let id = uploads.chunk(&owner, None, 0, &STANDARD.encode(&bytes[..12]), false).unwrap();
        let text = format!("[Image #1]({})", mesimon_core::attachment::target(id));
        assert!(uploads.prepare(&owner, &[id], &text).is_err());
        assert!(uploads.chunk(&owner, Some(id), 0, "AA==", false).is_err());
        let other_stream = self::stream();
        let other = Owner::stream(&other_stream);
        assert!(uploads.chunk(&other, Some(id), 12, "AA==", false).is_err());
        uploads.discard(&other, &[id]);
        assert!(uploads.0.contains_key(&id));
        uploads.chunk(&owner, Some(id), 12, &STANDARD.encode(&bytes[12..]), true).unwrap();
        let images = uploads.prepare(&owner, &[id], &text).unwrap();
        assert_eq!((images[0].0.width, images[0].0.height), (2, 3));
        assert!(uploads.prepare(&other, &[id], &text).is_err());
        assert!(uploads.prepare(&owner, &[id], "no reference").is_err());
        assert!(uploads.prepare(&owner, &[id, id], &text).is_err());
        uploads.committed(&[id]);
        assert!(uploads.0.is_empty());
    }

    #[test]
    fn malformed_oversized_abandoned_and_expired_uploads_are_bounded() {
        let mut uploads = Uploads::default();
        let stream = stream();
        let owner = Owner::stream(&stream);
        assert!(uploads.chunk(&owner, None, 0, "not base64", true).is_err());
        assert!(uploads.chunk(&owner, None, 0, "AA==", true).is_err());
        assert!(uploads.0.is_empty());
        assert!(uploads
            .chunk(&owner, None, 0, &STANDARD.encode(vec![0; CHUNK_BYTES + 1]), false)
            .is_err());
        let id = uploads.chunk(&owner, None, 0, "AA==", false).unwrap();
        uploads.0.get_mut(&id).unwrap().touched -= Duration::from_secs(601);
        uploads.prune();
        assert!(uploads.0.is_empty());
        uploads.chunk(&owner, None, 0, "AA==", false).unwrap();
        drop(stream);
        uploads.prune();
        assert!(uploads.0.is_empty());
        assert!(validate(Ulid::new(), &vec![0; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn a_grant_owns_its_uploads_across_reconnects_until_revoked() {
        let mut uploads = Uploads::default();
        let phone = Owner::Grant("aa".into());
        let other = Owner::Grant("bb".into());
        let desk_stream = stream();
        let desk = Owner::stream(&desk_stream);
        let bytes = png();
        let id = uploads.chunk(&phone, None, 0, &STANDARD.encode(&bytes[..12]), false).unwrap();
        uploads.prune();
        assert!(uploads.chunk(&other, Some(id), 12, "AA==", false).is_err());
        assert!(uploads.chunk(&desk, Some(id), 12, "AA==", false).is_err());
        uploads.chunk(&phone, Some(id), 12, &STANDARD.encode(&bytes[12..]), true).unwrap();
        let text = format!("[Image #1]({})", mesimon_core::attachment::target(id));
        assert!(uploads.prepare(&other, &[id], &text).is_err());
        assert_eq!(uploads.prepare(&phone, &[id], &text).unwrap().len(), 1);
        uploads.discard_all(&other);
        assert!(uploads.0.contains_key(&id));
        uploads.discard_all(&phone);
        assert!(uploads.0.is_empty());
    }

    #[test]
    fn files_survive_reload_and_never_resolve_another_tickets_picture() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::for_repo(root.path()).unwrap();
        let bytes = png();
        let meta = validate(Ulid::new(), &bytes).unwrap();
        save(&paths, "T-1", &meta, &bytes).unwrap();
        let fresh = Paths::for_repo(root.path()).unwrap();
        assert_eq!(read(&fresh, "T-1", meta.id).unwrap(), (meta.clone(), bytes.clone()));
        assert!(read(&fresh, "T-2", meta.id).is_err());
        assert_eq!(collect(&fresh, "T-1").unwrap().len(), 1);
        let file = directory(&paths, "T-1").join(format!("{}.png", meta.id));
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(root.path().join("secret"), &file).unwrap();
        assert!(read(&paths, "T-1", meta.id).is_err());
    }
}
