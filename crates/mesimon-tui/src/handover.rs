//! Focus handover (docs/19 §2): fully restore the terminal, exec the tmux attach
//! as a child, wait, drain stale query replies from stdin (spike T-4), resume.

use std::io::Read;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Run outside raw mode / alt screen — the caller restores the terminal first
/// and re-initializes after. Returns when the user detaches (Ctrl+]).
/// `cwd` is for the `!` shell-in-worktree handover (M4b); attach argvs pass None.
pub fn run(argv: &[String], cwd: Option<&std::path::Path>) -> Result<()> {
    let Some((prog, rest)) = argv.split_first() else {
        bail!("empty attach argv");
    };
    let mut cmd = Command::new(prog);
    cmd.args(rest);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let status = cmd.status().context("attach child")?;
    drain_stdin();
    if !status.success() {
        bail!("attach exited with {status}");
    }
    Ok(())
}

/// Spike T-4: tmux queries DA1/DA2/OSC 10/11 at attach; on a fast detach the
/// terminal's replies can land in our stdin. Drain before re-entering raw mode.
fn drain_stdin() {
    unsafe {
        let fd = 0;
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return;
        }
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        let mut buf = [0u8; 4096];
        // A short settle so late replies arrive before we drain.
        std::thread::sleep(std::time::Duration::from_millis(50));
        while let Ok(n) = std::io::stdin().read(&mut buf) {
            if n == 0 {
                break;
            }
        }
        libc::fcntl(fd, libc::F_SETFL, flags);
    }
}
