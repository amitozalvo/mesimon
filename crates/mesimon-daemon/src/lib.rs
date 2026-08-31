//! mesimond — the per-repo daemon. Single writer of board state (D22);
//! owns session lifecycle through the tmux backend (docs/19).

pub mod census;
pub mod diff;
pub mod feed;
pub mod hook_settings;
pub mod ingest;
pub mod movegate;
pub mod paths;
pub mod resources;
pub mod server;
pub mod store;
pub mod tail;
pub mod worktree;

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

pub use paths::Paths;

/// Run the daemon in the foreground (the `mesimon daemon` subcommand).
pub fn run_foreground(repo_root: &Path) -> Result<()> {
    let paths = Paths::for_repo(repo_root)?;
    server::run(paths)
}

/// Spawn a detached daemon for `repo_root` (called by a client that found no
/// socket). setsid so it survives the client's terminal (02 §3).
pub fn spawn_detached(repo_root: &Path) -> Result<()> {
    // MESIMON_DAEMON_BIN mirrors MESIMON_HOOK_BIN/MESIMON_CLAUDE_BIN: a test
    // driving the real client runs inside the TEST binary, whose current_exe()
    // has no `daemon` subcommand, so the respawn has to be pointed at the real
    // one. Unset in production, where the daemon is always this same binary.
    let exe = std::env::var_os("MESIMON_DAEMON_BIN")
        .map(std::path::PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)?;
    let paths = Paths::for_repo(repo_root)?;
    paths.ensure_dirs()?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(paths.daemon_log())?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon")
        .arg("--repo")
        .arg(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    // MESIMON_DETACHED marks a daemon mesimon started for itself. A newer
    // client may restart one of these; a human's foreground
    // `mesimon daemon --repo` carries no marker and is left alone.
    cmd.env("MESIMON_DETACHED", "1");
    let mut child = cmd.spawn()?;
    // Reap. setsid() detaches the session, not the parent-child link, so an
    // unwaited daemon stays a zombie for the client's whole lifetime — and
    // the reconnect path now spawns on a cadence, where every flock loser
    // exits within milliseconds. One blocked waitpid thread costs nothing.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// (mtime_ms, len) of this process's executable, or `None` when `current_exe`
/// or the stat fails. Both the daemon (at startup) and the TUI (at connect)
/// call this, so they cannot disagree about which fields they read; a `None`
/// on either side means "unknown" and never "changed" (D26 fails closed).
pub fn exe_stamp() -> Option<mesimon_core::command::ExeStamp> {
    let exe = std::env::current_exe().ok()?;
    let md = std::fs::metadata(exe).ok()?;
    let mtime_ms =
        md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
    Some(mesimon_core::command::ExeStamp { mtime_ms, len: md.len() })
}
