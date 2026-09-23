//! Path layout (02 §2 + D33b): short-lived runtime under `/tmp/mesimon-<uid>/<proj16>/`
//! (sun_path budget), persisted state under `~/.local/state/mesimon/<repo-id>/`,
//! board data under `<repo>/.mesimon/` (uncommitted, D34.9).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// The one spelling of the worktrees directory name, shared by the path it
/// is created at and the gate exemption that names it.
pub const WORKTREES_DIR: &str = "worktrees";

#[derive(Clone)]
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

        let rt_dir = runtime_root().join(&proj16);
        let state_dir = state_root()?.join(&proj16);
        let board_dir = canon.join(".mesimon");

        Ok(Self { repo_root: canon, proj16, rt_dir, state_dir, board_dir })
    }

    pub fn orch_sock(&self) -> PathBuf {
        self.rt_dir.join("orch.sock")
    }
    pub fn tmux_sock(&self) -> PathBuf {
        self.rt_dir.join("tmux.sock")
    }
    /// Hook-ingest socket (0600 — filesystem permission IS the auth).
    pub fn hook_sock(&self) -> PathBuf {
        self.rt_dir.join("hook.sock")
    }
    /// Per-session generated Claude settings files (11 §11.2.1, D33b path).
    pub fn hooks_dir(&self) -> PathBuf {
        self.state_dir.join("hooks")
    }
    /// Append-only activity feed (D34.10: JSONL, not SQLite, for v0.1).
    pub fn activity_log(&self) -> PathBuf {
        self.state_dir.join("activity.jsonl")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.rt_dir.join("daemon.lock")
    }
    /// Where the shell-environment capture writes its `env -0` dump. In the
    /// 0700 runtime dir rather than the state dir on purpose: it is a copy of
    /// the user's whole environment, so it belongs somewhere unreadable by
    /// others and cleared by a reboot. `crate::shellenv::capture` deletes it
    /// either way — this is the path, not a file that persists.
    pub fn shell_env_dump(&self) -> PathBuf {
        self.rt_dir.join("shellenv.dump")
    }
    /// The captured shell environment a pane is launched with (`mesimon exec
    /// --env`): `K=V\0` entries, 0600, in the runtime dir for the same reason
    /// as the dump — it holds whatever secrets the user's rc files export.
    pub fn shell_env_file(&self) -> PathBuf {
        self.rt_dir.join("shellenv.env")
    }
    pub fn sessions_file(&self) -> PathBuf {
        self.state_dir.join("sessions.json")
    }
    /// The ask queue's starts and wakes (T-418), beside `sessions.json`;
    /// absent whenever the queue is empty.
    pub fn queue_file(&self) -> PathBuf {
        self.state_dir.join("queue.json")
    }
    /// Every conversation a session mesimon spawned has held (T-441): what
    /// the External drawer leaves out, kept past the record.
    pub fn started_file(&self) -> PathBuf {
        self.state_dir.join("started.json")
    }
    /// This board's overrides of the machine's `prefs.json` (T-361): the
    /// TUI's file, not the daemon's, under the state dir so it is private to
    /// the machine and never rides a team board.
    pub fn prefs_file(&self) -> PathBuf {
        self.state_dir.join("prefs.json")
    }
    pub fn gate_file(&self) -> PathBuf {
        self.state_dir.join("gate-passed")
    }
    /// Transcript copies made at sleep time (D23 step 3; B-A22).
    pub fn transcripts_dir(&self) -> PathBuf {
        self.state_dir.join("transcripts")
    }
    pub fn daemon_log(&self) -> PathBuf {
        self.state_dir.join("daemon.log")
    }
    /// Ticket worktrees, `<state>/worktrees/<KEY>-<slug>/`. Inside the state
    /// dir but the agent's OWN to write: the gate is told so (`--allow`).
    pub fn worktrees_root(&self) -> PathBuf {
        self.state_dir.join(WORKTREES_DIR)
    }
    /// This board's sharing state (T-215): keys, cursor, outbox. 0600, JSON,
    /// beside `sessions.json`.
    pub fn team_file(&self) -> PathBuf {
        self.state_dir.join("team.json")
    }
    /// Board sharing, per user rather than per repo: the device identity
    /// (`device.toml`, 0600) and the synthetic roots of joined boards under
    /// `boards/<board16>/`. Beside the per-project dirs, like
    /// `notifications/`.
    pub fn team_root() -> Result<PathBuf> {
        Ok(state_root()?.join("team"))
    }
    pub fn team_device_file() -> Result<PathBuf> {
        Ok(Self::team_root()?.join("device.toml"))
    }
    /// The root a joined board is opened at: a directory with no checkout,
    /// whose `.mesimon/` is the board and whose state dir is derived from it
    /// like any other root's.
    pub fn board_root_for(board16: &str) -> Result<PathBuf> {
        Ok(Self::team_root()?.join("boards").join(board16))
    }

    /// The 0700 directories ARE the trust boundary: `orch.sock` answers any
    /// same-uid caller and `hook.sock` admits any same-uid frame, so what
    /// keeps another user out is that they cannot reach either socket. Both
    /// trees are created here and verified here, every start — sticky `/tmp`
    /// lets anyone pre-plant `/tmp/mesimon-<uid>`, and a symlink or a
    /// directory someone else owns must be refused, not used.
    pub fn ensure_dirs(&self) -> Result<()> {
        if let Some(parent) = self.rt_dir.parent() {
            own_private_dir(parent)?;
        }
        own_private_dir(&self.rt_dir)?;
        if let Some(parent) = self.state_dir.parent() {
            own_private_dir(parent)?;
        }
        own_private_dir(&self.state_dir)?;
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

/// `/tmp/mesimon-<uid>`, the parent of every per-project runtime dir — and of
/// `mesimon update`'s staging dir, which has no project (T-445). Callers
/// verify it with [`own_private_dir`] before putting anything in it.
pub fn runtime_root() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/mesimon-{uid}"))
}

/// `~/.local/state/mesimon`, the parent of every per-project state dir.
pub fn state_root() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset")?;
    Ok(PathBuf::from(home).join(".local/state/mesimon"))
}

/// The machine's agent tiers (T-443), `tiers.toml` beside every board's
/// state dir: the one list every repo's daemon reads, like
/// `team/device.toml`, and writes only on a person's gesture.
pub fn machine_tiers_file() -> Result<PathBuf> {
    Ok(state_root()?.join("tiers.toml"))
}

/// Create `dir` if missing, then insist it is a real directory that this uid
/// owns, and close it to everyone else. Errors name the path.
pub fn own_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e).with_context(|| format!("create {}", dir.display())),
    }
    let meta = std::fs::symlink_metadata(dir).with_context(|| format!("stat {}", dir.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!("{} is a symlink; refusing to use it", dir.display());
    }
    if !meta.is_dir() {
        anyhow::bail!("{} is not a directory", dir.display());
    }
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid {
        anyhow::bail!("{} is owned by uid {}, not {uid}", dir.display(), meta.uid());
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, PermissionsExt::from_mode(0o700))
            .with_context(|| format!("chmod 0700 {}", dir.display()))?;
    }
    Ok(())
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
        assert!(p.hook_sock().as_os_str().len() <= 100, "{:?}", p.hook_sock());
        assert_eq!(p.proj16.len(), 16);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The boundary is checked, not assumed: a pre-planted symlink where a
    /// private dir should be is refused, and a lax mode is tightened.
    #[test]
    fn a_private_dir_is_ours_alone() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let base = std::env::temp_dir().join(format!("msmn-priv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let fresh = base.join("fresh/deeper");
        own_private_dir(&fresh).unwrap();
        assert_eq!(std::fs::metadata(&fresh).unwrap().mode() & 0o777, 0o700);

        let lax = base.join("lax");
        std::fs::create_dir(&lax).unwrap();
        std::fs::set_permissions(&lax, PermissionsExt::from_mode(0o755)).unwrap();
        own_private_dir(&lax).unwrap();
        assert_eq!(std::fs::metadata(&lax).unwrap().mode() & 0o777, 0o700);

        let target = base.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        let link = base.join("planted");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = own_private_dir(&link).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");

        std::fs::remove_dir_all(base).ok();
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
