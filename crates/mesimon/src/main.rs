//! mesimon — me-si-MON. The task instrument.

mod hook;

use std::path::PathBuf;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // First: the hot path. Runs inside the user's agent turn (≤5 ms p99,
        // docs/14 §1.7) and must touch nothing else in the binary.
        Some("hook") => hook::run(&args[1..]),
        Some("daemon") => {
            let repo = arg_value(&args, "--repo")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            mesimon_daemon::run_foreground(&repo)
        }
        Some("--version" | "-V") => {
            println!("mesimon {}", env!("CARGO_PKG_VERSION"));
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

fn print_help() {
    println!(
        "mesimon (me-si-MON) — a terminal kanban that orchestrates coding-agent sessions\n\n\
         usage:\n  mesimon              open the board for the current directory\n  \
         mesimon daemon --repo <path>   run the daemon in the foreground\n  \
         mesimon --version\n"
    );
}
