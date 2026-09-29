//! The shin's embedded notification assets, rendered from its pixels by
//! `creature.rs`'s goldens.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const RESTING: &[u8] = include_bytes!("../../../assets/mascot/resting.png");
const NEEDS_YOU: &[u8] = include_bytes!("../../../assets/mascot/needs-you.png");
/// The tab's needs-you icon (T-492): the pose as a dark silhouette on the
/// attention-colour tile, because a tab's sixteen pixels lose the "!".
const TAB_NEEDS_YOU: &[u8] = include_bytes!("../../../assets/mascot/tab-needs-you.png");

/// A 256px PNG representation in an ICNS container. This is the app's
/// stable identity; needs-you remains a per-notification attachment.
pub(crate) fn app_icon() -> Vec<u8> {
    let mut icon = Vec::with_capacity(RESTING.len() + 16);
    icon.extend_from_slice(b"icns");
    icon.extend_from_slice(&((RESTING.len() + 16) as u32).to_be_bytes());
    icon.extend_from_slice(b"ic08");
    icon.extend_from_slice(&((RESTING.len() + 8) as u32).to_be_bytes());
    icon.extend_from_slice(RESTING);
    icon
}

/// Materialize only when an image-capable banner is actually sent. Doctor,
/// sound previews and disabled notifications never write an asset. A digest
/// gives each drawing an immutable name, including across binary upgrades;
/// atomic publication keeps simultaneous boards from exposing half a PNG.
pub(crate) fn icon(dir: &Path, needs_you: bool) -> std::io::Result<PathBuf> {
    publish(dir, if needs_you { NEEDS_YOU } else { RESTING })
}

/// The tab's icon (T-492): resting is the notification's own file; the
/// needs-you pose is the tab-sized one, on the attention colour.
pub(crate) fn tab_icon(dir: &Path, needs_you: bool) -> std::io::Result<PathBuf> {
    publish(dir, if needs_you { TAB_NEEDS_YOU } else { RESTING })
}

fn publish(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    mesimon_daemon::paths::own_private_dir(dir).map_err(std::io::Error::other)?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    let path = dir.join(format!("shin-{digest}.png"));
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file())
        && std::fs::read(&path).is_ok_and(|existing| existing == bytes)
    {
        return Ok(path);
    }
    let temp = dir.join(format!(".shin-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file =
            std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp)?;
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&temp, &path)?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_distinct_reusable_and_do_not_follow_a_file_symlink() {
        let root = std::env::temp_dir().join(format!("msmn-mascot-{}", uuid::Uuid::new_v4()));
        let dir = root.join("icons");
        let resting = icon(&dir, false).unwrap();
        let waiting = icon(&dir, true).unwrap();
        assert_ne!(resting, waiting);
        assert_eq!(std::fs::read(&resting).unwrap(), RESTING);
        assert_eq!(std::fs::read(&waiting).unwrap(), NEEDS_YOU);
        let modified = std::fs::metadata(&resting).unwrap().modified().unwrap();
        assert_eq!(icon(&dir, false).unwrap(), resting);
        assert_eq!(std::fs::metadata(&resting).unwrap().modified().unwrap(), modified);
        let outside = root.join("untouched");
        std::fs::write(&outside, "user data").unwrap();
        std::fs::remove_file(&resting).unwrap();
        std::os::unix::fs::symlink(&outside, &resting).unwrap();
        icon(&dir, false).unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "user data");
        assert!(!std::fs::symlink_metadata(&resting).unwrap().file_type().is_symlink());
        std::fs::remove_dir_all(root).unwrap();
    }
}
