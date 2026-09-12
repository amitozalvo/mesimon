//! `mesimon join CODE` (T-335): redeem an invite code from the shell, the
//! way the TUI's team boards dialog does, and print where the joined board
//! lives. The daemon for the current directory does the work — it holds the
//! relay identity — and the board root it creates is printed with the
//! command that opens it. Nothing here touches the relay directly.
use anyhow::{bail, Context, Result};
use mesimon_core::command::{Command, Response};
use mesimon_tui::client::{Client, Transport};
use std::path::Path;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(20);

pub fn run(args: &[String]) -> Result<()> {
    let Some(code) = args.first().filter(|a| !a.starts_with('-')) else {
        bail!("usage: mesimon join XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX");
    };
    let cwd = std::env::current_dir()?;
    let mut client = Client::connect(&cwd).context("connect to the daemon")?;
    let before = match client.request(Command::Snapshot)? {
        Response::Board { team, .. } => {
            if !team.device.as_ref().is_some_and(|d| d.registered) {
                bail!("sign in first: open mesimon, Esc › Settings › Team");
            }
            team.boards.iter().map(|b| b.board.clone()).collect::<Vec<_>>()
        }
        other => bail!("unexpected answer to a snapshot: {other:?}"),
    };
    match client.request(Command::JoinBoard { code: code.clone() })? {
        Response::Ok => {}
        Response::Err { message } => bail!("{message}"),
        other => bail!("unexpected answer to a join: {other:?}"),
    }
    let start = Instant::now();
    loop {
        let Response::Board { team, .. } = client.request(Command::Snapshot)? else {
            bail!("the daemon stopped answering");
        };
        if let Some(e) = team.error.as_deref().filter(|e| e.starts_with("joining")) {
            bail!("{e}");
        }
        let fresh = team.boards.iter().find(|b| !before.contains(&b.board) && b.root.is_some());
        if let Some(b) = fresh {
            let root = b.root.clone().unwrap_or_default();
            println!("joined {}'s board as a {}", b.owner_name, b.role);
            println!("{}", root.display());
            println!("open it with: mesimon open {}", shell_word(&root));
            return Ok(());
        }
        if start.elapsed() > WAIT {
            bail!("the relay did not answer the join in {}s", WAIT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A path as a shell would need it typed: quoted when it holds a space.
fn shell_word(path: &Path) -> String {
    let text = path.display().to_string();
    if text.chars().any(char::is_whitespace) {
        format!("'{}'", text.replace('\'', "'\\''"))
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_a_space_is_quoted() {
        assert_eq!(shell_word(Path::new("/a/b")), "/a/b");
        assert_eq!(shell_word(Path::new("/a b/c")), "'/a b/c'");
    }
}
