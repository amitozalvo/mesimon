//! Focus handover (docs/19 §2): fully restore the terminal, exec the tmux attach
//! as a child, wait, drain stale query replies from stdin (spike T-4), resume.

use std::io::Read;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Run outside raw mode / alt screen — the caller restores the terminal first
/// and re-initializes after. Returns when the user detaches (Ctrl+]).
/// `cwd` is for the `!` shell-in-worktree handover (M4b); attach argvs pass None,
/// and so does the note editor's `^g` (`external.rs`), which rides the same
/// road and wants the exit status judged.
///
/// Returns as soon as the child exits — the caller must re-enter the alt
/// screen immediately (the primary screen shows stale shell output) and then
/// call `drain_stdin`.
pub fn run(argv: &[String], cwd: Option<&std::path::Path>) -> Result<()> {
    let Some((prog, rest)) = argv.split_first() else {
        bail!("empty attach argv");
    };
    let mut cmd = Command::new(prog);
    cmd.args(rest);
    let shell = cwd.is_some();
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    // system(3) semantics: raw mode is off, so ISIG is live again, and until
    // the child takes the terminal (tmux client / zsh job control) a Ctrl+C
    // lands on OUR process group too — default SIGINT then kills the TUI
    // silently while the interactive child survives, stranding the user
    // inside it (dogfood 2026-08-30: `!` shell appeared to "crash mesimon").
    // Ignore INT/QUIT for exactly the wait, restore after. drain_stdin stays
    // the caller's, after the alt screen is back up.
    let (old_int, old_quit) = unsafe {
        (libc::signal(libc::SIGINT, libc::SIG_IGN), libc::signal(libc::SIGQUIT, libc::SIG_IGN))
    };
    let status = cmd.status().context("attach child");
    unsafe {
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
    }
    let status = status?;
    // An interactive shell exits with its LAST command's status — meaningless
    // here, never an attach failure. Only the attach path reports non-zero.
    if !shell && !status.success() {
        bail!("exited with {status}");
    }
    Ok(())
}

/// Spike T-4: tmux queries DA1/DA2/OSC 10/11 at attach; on a fast detach the
/// terminal's replies can land in our stdin. Drain before the event loop
/// reads keys. Called AFTER the alt screen is back up: the 50 ms settle would
/// otherwise flash the primary screen's stale output at the user.
pub fn drain_stdin() {
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
