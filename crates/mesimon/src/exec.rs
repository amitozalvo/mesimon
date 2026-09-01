//! `mesimon exec` — the pane launcher.
//!
//! Every pane mesimon spawns starts here: `mesimon exec --env <file>
//! [--set K=V]... -- <argv>`. It reads the captured shell environment from a
//! 0600 file in the runtime dir, applies mesimon's own per-session variables
//! over it, and `exec`s the real command in place — so the pane's pid IS the
//! agent's, as it was when tmux exec'd it directly, and no shell ever runs.
//!
//! It exists because the previous road was `new-session -e K=V` for every
//! variable: the user's whole environment, API keys included, spelled on a
//! tmux command line. On macOS any user on the machine can read another
//! process's arguments, so for the lifetime of that spawn the environment was
//! public. A terminal-started agent never has that problem, because a shell
//! passes its environment through `execve`, not through argv. This is the
//! same property, restored: the values travel inside the pane only. The
//! `--set` variables stay on the command line on purpose — a ticket key is
//! not a secret, and `ps` showing which ticket a pane belongs to is useful.
//!
//! A missing or unreadable file is a warning on the pane and the command
//! still runs with the tmux server's own environment (what every spawn got
//! before the capture existed), never a dead pane.

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub fn run(args: &[String]) -> ! {
    let plan = match Plan::parse(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("mesimon exec: {e}");
            std::process::exit(2);
        }
    };
    let mut cmd = Command::new(&plan.argv[0]);
    cmd.args(&plan.argv[1..]);
    if let Some(file) = &plan.env_file {
        match std::fs::read(file) {
            Ok(bytes) => {
                for (k, v) in mesimon_core::shellenv::parse_env0(&bytes) {
                    cmd.env(k, v);
                }
            }
            Err(e) => eprintln!(
                "mesimon exec: {}: {e}; this pane keeps the tmux server's environment",
                file.display()
            ),
        }
    }
    // mesimon's own last, so nothing captured from a shell can shadow them.
    for (k, v) in &plan.sets {
        cmd.env(k, v);
    }
    // `exec` only returns on failure.
    let err = cmd.exec();
    eprintln!("mesimon exec: {}: {err}", plan.argv[0]);
    std::process::exit(127);
}

/// The parsed command line: what to load, what to set, what to run.
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub env_file: Option<PathBuf>,
    pub sets: Vec<(String, String)>,
    pub argv: Vec<String>,
}

impl Plan {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut env_file = None;
        let mut sets = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--env" => {
                    env_file = Some(PathBuf::from(args.get(i + 1).ok_or("--env needs a path")?));
                    i += 2;
                }
                "--set" => {
                    let kv = args.get(i + 1).ok_or("--set needs NAME=value")?;
                    let (k, v) = kv.split_once('=').ok_or_else(|| format!("--set {kv}: no `=`"))?;
                    if k.is_empty() {
                        return Err(format!("--set {kv}: empty name"));
                    }
                    sets.push((k.to_string(), v.to_string()));
                    i += 2;
                }
                "--" => {
                    let argv: Vec<String> = args[i + 1..].to_vec();
                    if argv.is_empty() {
                        return Err("no command after --".into());
                    }
                    return Ok(Self { env_file, sets, argv });
                }
                other => return Err(format!("unknown flag {other}")),
            }
        }
        Err("no command: expected `-- <argv>`".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_plan_reads_flags_then_everything_after_the_dashes() {
        let p =
            Plan::parse(&s(&["--env", "/f", "--set", "A=1", "--set", "B=x=y", "--", "cl", "--x"]))
                .unwrap();
        assert_eq!(p.env_file, Some(PathBuf::from("/f")));
        assert_eq!(p.sets, vec![("A".into(), "1".into()), ("B".into(), "x=y".into())]);
        assert_eq!(p.argv, s(&["cl", "--x"]));
    }

    #[test]
    fn a_command_is_required_and_flags_after_the_dashes_are_the_commands() {
        assert!(Plan::parse(&s(&["--env", "/f"])).is_err());
        assert!(Plan::parse(&s(&["--"])).is_err());
        assert!(Plan::parse(&s(&["--bogus", "--", "x"])).is_err());
        assert!(Plan::parse(&s(&["--set", "novalue", "--", "x"])).is_err());
        let p = Plan::parse(&s(&["--", "x", "--env", "/not-a-flag"])).unwrap();
        assert_eq!(p.argv, s(&["x", "--env", "/not-a-flag"]));
        assert_eq!(p.env_file, None);
    }
}
