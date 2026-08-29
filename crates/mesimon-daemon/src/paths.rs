//! Path layout (02 §2 + D33b): short-lived runtime under `/tmp/mesimon-<uid>/<proj16>/`
//! (sun_path budget), persisted state under `~/.local/state/mesimon/<repo-id>/`,
//! board data under `<repo>/.mesimon/` (uncommitted, D34.9).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub struct Paths {
    pub repo_root: PathBuf,
    pub proj16: String,
    pub rt_dir: PathBuf,
    pub state_dir: PathBuf,
    pub board_dir: PathBuf,
}

impl Paths {
    pub fn for_repo(repo_root: &Path) -> Result<Self> {
        // Canonical path, not repo name — two checkouts must differ (D33b).
        let canon = repo_root.canonicalize().context("canonicalize repo root")?;
        let mut h = Sha256::new();
        h.update(canon.as_os_str().as_encoded_bytes());
        let proj16 = hex(&h.finalize())[..16].to_string();

        let uid = unsafe { libc::getuid() };
        let rt_dir = PathBuf::from(format!("/tmp/mesimon-{uid}/{proj16}"));
        let home = std::env::var("HOME").context("HOME unset")?;
        let state_dir = PathBuf::from(home).join(".local/state/mesimon").join(&proj16);
        let board_dir = canon.join(".mesimon");

        Ok(Self { repo_root: canon, proj16, rt_dir, state_dir, board_dir })
    }

    pub fn orch_sock(&self) -> PathBuf {
        self.rt_dir.join("orch.sock")
    }
    pub fn tmux_sock(&self) -> PathBuf {
        self.rt_dir.join("tmux.sock")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.rt_dir.join("daemon.lock")
    }
    pub fn sessions_file(&self) -> PathBuf {
        self.state_dir.join("sessions.json")
    }
    pub fn gate_file(&self) -> PathBuf {
        self.state_dir.join("gate-passed")
    }
    pub fn daemon_log(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        // /tmp/mesimon-<uid> must not be usable by others (02 §2).
        if let Some(parent) = self.rt_dir.parent() {
            std::fs::create_dir_all(parent)?;
            let perm = std::os::unix::fs::PermissionsExt::from_mode(0o700);
            std::fs::set_permissions(parent, perm)?;
        }
        std::fs::create_dir_all(&self.rt_dir)?;
        std::fs::create_dir_all(&self.state_dir)?;
        std::fs::create_dir_all(self.board_dir.join("board/tickets"))?;
        self.ensure_excluded()?;
        Ok(())
    }

    /// D34.9: `.mesimon/` is excluded via `$GIT_DIR/info/exclude` — the one
    /// permitted write to the repo's git metadata (D8/D33c). Never `.gitignore`.
    fn ensure_excluded(&self) -> Result<()> {
        let git_dir = self.repo_root.join(".git");
        if !git_dir.is_dir() {
            return Ok(()); // not a git repo — nothing to exclude
        }
        let info = git_dir.join("info");
        std::fs::create_dir_all(&info)?;
        let exclude = info.join("exclude");
        let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !existing.lines().any(|l| l.trim() == "/.mesimon/") {
            let mut content = existing;
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("/.mesimon/\n");
            std::fs::write(&exclude, content)?;
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_paths_fit_sun_path() {
        let dir = std::env::temp_dir().join(format!("msmn-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Paths::for_repo(&dir).unwrap();
        assert!(p.orch_sock().as_os_str().len() <= 100, "{:?}", p.orch_sock());
        assert!(p.tmux_sock().as_os_str().len() <= 100, "{:?}", p.tmux_sock());
        assert_eq!(p.proj16.len(), 16);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn two_checkouts_differ() {
        let base = std::env::temp_dir().join(format!("msmn-two-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let pa = Paths::for_repo(&a).unwrap();
        let pb = Paths::for_repo(&b).unwrap();
        assert_ne!(pa.proj16, pb.proj16);
        std::fs::remove_dir_all(base).ok();
    }
}
