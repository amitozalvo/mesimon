//! mesimond — the per-repo daemon. Single writer of board state (D22);
//! owns session lifecycle through the tmux backend (docs/19).

pub mod paths;
pub mod server;
pub mod store;

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
    let exe = std::env::current_exe()?;
    let paths = Paths::for_repo(repo_root)?;
    paths.ensure_dirs()?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.daemon_log())?;
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
    cmd.spawn()?;
    Ok(())
}
