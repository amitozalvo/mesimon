//! mesimon — me-si-MON. The task instrument.

mod doctor;
mod exec;
mod gate;
mod hook;
mod join;
mod mcp;
mod state;

use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // First: the hot path. Runs inside the user's agent turn (≤5 ms p99,
        // docs/14 §1.7) and must touch nothing else in the binary.
        Some("hook") => hook::run(&args[1..]),
        // Also inside the turn, and also before anything else is touched:
        // `gate` decides a PreToolUse, `mcp` is the board's tool server. Both
        // are spawned by Claude Code, never by a person.
        Some("gate") => gate::run(&args[1..]),
        Some("mcp") => mcp::run(&args[1..]),
        // Separate opt-in context handler: the observer remains silent and
        // the write gate remains deny-only. Native Codex owns hook trust.
        Some("agent-brief") => {
            if std::env::var("MESIMON_AGENT_BRIEF").as_deref() == Ok("1") {
                println!(
                    "{}",
                    serde_json::json!({"hookSpecificOutput": {
                        "hookEventName": "SessionStart",
                        "additionalContext": mesimon_core::brief::TEXT,
                    }})
                );
            }
            Ok(())
        }
        // The pane launcher: applies the captured environment and execs.
        Some("exec") => exec::run(&args[1..]),
        Some("agent-runtime") => {
            let path = arg_value(&args, "--config")
                .ok_or_else(|| anyhow::anyhow!("agent-runtime needs --config"))?;
            mesimon_daemon::agents::codex::runtime::run(std::path::Path::new(&path))
        }
        Some("daemon") => {
            let repo =
                arg_value(&args, "--repo").map(PathBuf::from).unwrap_or(std::env::current_dir()?);
            mesimon_daemon::install_sigterm_handler();
            mesimon_daemon::run_foreground(&repo)
        }
        // A board at an explicit root: a joined team board lives under the
        // state dir, not in a checkout (T-215).
        Some("open") => {
            let root = args
                .get(1)
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("open needs a directory"))?;
            mesimon_tui::run(&root)
        }
        Some("doctor") => doctor::run(&args[1..]),
        // Redeem an invite code from the shell (T-335): the current
        // directory's daemon joins, and the joined board's root is printed.
        Some("join") if cfg!(debug_assertions) || std::env::var_os("MESIMON_TEAMS").is_some() => {
            join::run(&args[1..])
        }
        Some("state") => state::run(&args[1..]),
        Some("--version" | "-V") => {
            println!("mesimon {}", version_line());
            Ok(())
        }
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            print_help();
            std::process::exit(2);
        }
        None => {
            let cwd = std::env::current_dir()?;
            mesimon_tui::run(&cwd)
        }
    }
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1).cloned())
}

/// `<version> (<git sha>, <build date>)` — the string a bug report should carry
/// (16 §8.1).
pub fn version_line() -> String {
    format!(
        "{} ({}, {})",
        env!("CARGO_PKG_VERSION"),
        env!("MESIMON_GIT_SHA"),
        env!("MESIMON_BUILD_DATE"),
    )
}

fn print_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    "mesimon (me-si-MON) — a terminal kanban that orchestrates coding-agent sessions\n\n\
         usage:\n  mesimon              open the board for the current directory\n  \
         mesimon doctor [section]       diagnose the environment; prints fixes, applies none\n  \
         mesimon doctor --mcp           print everything mesimon adds to a session's model input\n  \
         mesimon daemon --repo <path>   run the daemon in the foreground\n  \
         mesimon open <dir>             open the board at a directory (a joined team board)\n  \
         mesimon join <code>            redeem a team invite code; prints the board's directory\n  \
         mesimon state explain [session]   explain observed state and movement\n  \
         mesimon state replay <files...>  replay offline state scenarios\n  \
         mesimon state compatibility [claude|codex] <version>   show measured Claude coverage\n  \
         mesimon --version\n\n\
         spawned by Claude Code inside a mesimon session, never run by hand:\n  \
         mesimon hook   observer; reports one event, writes no stdout, exits 0\n  \
         mesimon gate   PreToolUse decider; refuses writes into paths mesimon owns\n  \
         mesimon mcp    the board's scoped MCP server; doctor --mcp shows current tools\n  \
         mesimon exec   pane launcher; applies the captured shell environment, then execs"
}

#[cfg(test)]
mod tests {
    use super::help_text;

    #[test]
    fn help_points_to_the_canonical_mcp_disclosure() {
        let help = help_text();
        assert!(help.contains("mesimon doctor --mcp"));
        assert!(help.contains("doctor --mcp shows current tools"));
        for tool in mesimon_core::mcp::tools() {
            let name = tool["name"].as_str().expect("every tool has a name");
            assert!(!help.contains(name), "help duplicated the registered tool {name}");
        }
    }
}
