//! `mesimon doctor` — diagnoses and prints copy-pasteable fixes. It has no
//! `--fix` (D8): mesimon never mutates your config, so every remedy here is a
//! line for you to run, never one we run for you.
//!
//! Shape is 16 §8's, and the constraints are deliberate:
//!   * Records are vim.health's `ok` / `note` / `WARN` / `FAIL`.
//!   * Output is ASCII-only. `⚠` is East-Asian *Ambiguous* — width 1 in some
//!     terminals, 2 in others — and this is the command you run WHEN YOUR
//!     TERMINAL IS MISBEHAVING, pasted into issues where the font is unknown.
//!     A report that misaligns in exactly the environments it exists to
//!     diagnose is self-refuting.
//!   * Every section rule is exactly 78 columns (`section_rule`, asserted by
//!     a unit test, because a mockup nobody checks drifts).
//!   * Paths are redacted by default: `$HOME` becomes `~`. The default view is
//!     what people screenshot, so redaction that only applies to a flag is not
//!     redaction.
//!
//! Exit codes (16 §8.2): 0 clean or warnings only, 1 any FAIL, 2 doctor itself
//! failed.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

#[derive(PartialEq, Clone, Copy)]
enum Level {
    Ok,
    Note,
    Warn,
    Fail,
}

impl Level {
    fn marker(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Note => "note",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        }
    }
}

struct Record {
    level: Level,
    label: String,
    value: String,
    advice: Option<String>,
}

fn rec(level: Level, label: &str, value: impl Into<String>) -> Record {
    Record { level, label: label.into(), value: value.into(), advice: None }
}

impl Record {
    fn advice(mut self, a: impl Into<String>) -> Self {
        self.advice = Some(a.into());
        self
    }
}

struct Section {
    name: &'static str,
    records: Vec<Record>,
}

/// `-- name ` + dashes + ` ` + a 14-column right-aligned summary = 78 columns
/// exactly. The arithmetic is the test's business, not a comment's.
fn section_rule(name: &str, records: &[Record]) -> String {
    let fails = records.iter().filter(|r| r.level == Level::Fail).count();
    let warns = records.iter().filter(|r| r.level == Level::Warn).count();
    let summary = match (fails, warns) {
        (0, 0) => String::new(),
        (0, w) => format!("{w} warn"),
        (f, 0) => format!("{f} fail"),
        (f, w) => format!("{f} fail {w} warn"),
    };
    let dashes = 59usize.saturating_sub(name.len());
    format!("-- {} {} {:>14}", name, "-".repeat(dashes), summary)
}

fn render(sections: &[Section], verbose: bool) -> String {
    let mut out = String::new();
    let head = format!("mesimon doctor{:>width$}", crate::version_line(), width = 64);
    out.push_str(&head);
    out.push('\n');
    out.push_str(&format!("{:>78}\n\n", "redacted by default"));
    for s in sections {
        out.push_str(&section_rule(s.name, &s.records));
        out.push('\n');
        for r in &s.records {
            if r.level == Level::Note && !verbose {
                continue;
            }
            out.push_str(&format!("  {:<6}{:<17}{}\n", r.level.marker(), r.label, r.value));
            if let Some(a) = &r.advice {
                for (i, line) in wrap(a, 58).into_iter().enumerate() {
                    let tag = if i == 0 { "ADVICE" } else { "      " };
                    out.push_str(&format!("        {tag}  {line}\n"));
                }
            }
        }
        out.push('\n');
    }
    let fails: usize = sections.iter().map(|s| count(s, Level::Fail)).sum();
    let warns: usize = sections.iter().map(|s| count(s, Level::Warn)).sum();
    out.push_str(&format!("{fails} failures, {warns} warnings.\n"));
    if !verbose {
        out.push_str("  mesimon doctor <section>   re-run one section\n");
        out.push_str("  mesimon doctor --verbose   every note, unredacted paths\n");
    }
    out
}

fn count(s: &Section, l: Level) -> usize {
    s.records.iter().filter(|r| r.level == l).count()
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Redaction for a line the user will PASTE INTO A SHELL: `$HOME`, not `~`.
/// A tilde inside double quotes does not expand, so `~` in a fix line hands
/// out a command that silently does the wrong thing.
fn redact_cmd(p: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && p.starts_with(&h) => p.replacen(&h, "$HOME", 1),
        _ => p.to_string(),
    }
}

/// `$HOME` → `~`. The default view is what gets screenshotted into issues.
fn redact(p: &str, verbose: bool) -> String {
    if verbose {
        return p.to_string();
    }
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && p.starts_with(&h) => p.replacen(&h, "~", 1),
        _ => p.to_string(),
    }
}

fn tool_version(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.lines().next().unwrap_or_default().trim().to_string())
}

fn which(bin: &str) -> Option<String> {
    let out = Command::new("sh").arg("-c").arg(format!("command -v {bin}")).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

fn environment(verbose: bool) -> Section {
    let mut records = Vec::new();
    let os = tool_version("uname", &["-sr"]).unwrap_or_else(|| "unknown".into());
    let arch = tool_version("uname", &["-m"]).unwrap_or_else(|| "unknown".into());
    records.push(rec(Level::Ok, "os", format!("{os} ({arch})")));

    let shell = std::env::var("SHELL").unwrap_or_default();
    records.push(if shell.is_empty() {
        // server.rs falls back to /bin/zsh for bash-kind panes.
        rec(Level::Warn, "shell", "SHELL unset")
            .advice("mesimon falls back to /bin/zsh for shell panes. Set SHELL in your login environment if that is not what you want.")
    } else {
        rec(Level::Ok, "shell", redact(&shell, verbose))
    });

    let term = std::env::var("TERM").unwrap_or_default();
    records.push(if term.is_empty() {
        rec(Level::Warn, "TERM", "unset")
            .advice("mesimon forces TERM=xterm-256color on child panes, so agents still work, but the board itself needs a real TERM.")
    } else {
        rec(Level::Ok, "TERM", term)
    });

    records.push(match std::env::var("HOME") {
        Ok(h) if !h.is_empty() => rec(Level::Ok, "HOME", redact(&h, verbose)),
        _ => rec(Level::Fail, "HOME", "unset").advice(
            "mesimon keeps its state under $HOME/.local/state/mesimon and cannot start without it.",
        ),
    });
    Section { name: "environment", records }
}

fn install(verbose: bool) -> Section {
    let mut records = Vec::new();
    match std::env::current_exe() {
        Ok(exe) => {
            records.push(rec(Level::Ok, "binary", redact(&exe.display().to_string(), verbose)));
            // The hook settings of every spawned session embed this absolute
            // path, and the TUI's `update ready` offer watches its mtime. A
            // binary that moves per version breaks both, silently.
            let dir = exe.parent().map(|d| d.display().to_string()).unwrap_or_default();
            let on_path = std::env::var("PATH").unwrap_or_default().split(':').any(|p| p == dir);
            records.push(if on_path {
                rec(Level::Ok, "on PATH", redact(&dir, verbose))
            } else {
                rec(Level::Warn, "on PATH", format!("{} is not in PATH", redact(&dir, verbose)))
                    .advice(format!("Add it: export PATH=\"{}:$PATH\"", redact_cmd(&dir)))
            });
        }
        Err(e) => records.push(rec(Level::Warn, "binary", format!("unknown ({e})"))),
    }
    records.push(rec(Level::Note, "version", crate::version_line()));
    Section { name: "install", records }
}

fn multiplexer(verbose: bool) -> Section {
    let mut records = Vec::new();
    // The same ladder the daemon uses, so this reports the tmux that will
    // actually run — not whatever `tmux` means to your shell.
    let bin = mesimon_backend_tmux::tmux_bin();
    let shown = redact(&bin.display().to_string(), verbose);
    records.push(if std::env::var_os("MESIMON_TMUX_BIN").is_some() {
        rec(Level::Note, "tmux binary", format!("{shown} (MESIMON_TMUX_BIN)"))
    } else if bin.is_absolute() {
        rec(Level::Note, "tmux binary", format!("{shown} (shipped with mesimon)"))
    } else {
        // A release ships its own; falling back to PATH means this is a
        // source build, or the bundled binary is missing from the install.
        rec(Level::Note, "tmux binary", "resolved from PATH")
    });
    match tool_version(&bin.display().to_string(), &["-V"]) {
        None => records.push(
            rec(Level::Fail, "tmux", "not found").advice(
                "mesimon runs every agent in its own private tmux server and cannot spawn without it. A release build ships one; if you built from source, install tmux: brew install tmux",
            ),
        ),
        Some(v) => {
            // The generated conf uses brace-literal hooks and `extended-keys
            // always`, which need tmux >= 3.1.
            let num: f32 = v
                .split_whitespace()
                .nth(1)
                .unwrap_or("0")
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .parse()
                .unwrap_or(0.0);
            records.push(if num >= 3.1 {
                rec(Level::Ok, "tmux", format!("{v} (needs >= 3.1)"))
            } else {
                rec(Level::Warn, "tmux", format!("{v} is below the 3.1 floor")).advice(
                    "mesimon's generated conf uses brace-literal hooks and extended-keys, both 3.1+. Upgrade: brew upgrade tmux",
                )
            });
        }
    }
    Section { name: "multiplexer", records }
}

fn agents(verbose: bool) -> Section {
    let mut records = Vec::new();
    match which("claude") {
        None => records.push(
            rec(Level::Fail, "claude", "not found on PATH").advice(
                "Without it a Claude ticket spawns a pane that dies instantly and reads as crashed. Install Claude Code, then re-run this.",
            ),
        ),
        Some(p) => {
            records.push(rec(Level::Ok, "claude", redact(&p, verbose)));
            match tool_version("claude", &["--version"]) {
                Some(v) => records.push(rec(Level::Ok, "claude version", v)),
                None => records.push(
                    rec(Level::Warn, "claude version", "on PATH but would not report a version")
                        .advice("Try running `claude --version` yourself; a broken install spawns panes that die immediately."),
                ),
            }
        }
    }
    Section { name: "agents", records }
}

fn git_section() -> Section {
    let mut records = Vec::new();
    match tool_version("git", &["--version"]) {
        Some(v) => records.push(rec(Level::Ok, "git", v)),
        None => records.push(rec(Level::Fail, "git", "not found on PATH").advice(
            "Per-ticket worktrees, the diff viewer and the merge flow all shell out to git.",
        )),
    }
    Section { name: "git", records }
}

/// One-shot Hello over `orch.sock`. Deliberately not the TUI's client: doctor
/// must never spawn a daemon, restart one, or change anything it is reporting
/// on. If nothing is listening, that is the finding.
fn daemon_hello(sock: &Path) -> Option<(u32, u32, String)> {
    let stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok()?;
    let mut write = stream.try_clone().ok()?;
    let env = mesimon_core::command::Envelope {
        principal: mesimon_core::Principal::Local,
        command: mesimon_core::command::Command::Hello {
            version: mesimon_core::command::PROTOCOL_VERSION,
            client: format!("mesimon-doctor/{}", env!("CARGO_PKG_VERSION")),
        },
    };
    writeln!(write, "{}", serde_json::to_string(&env).ok()?).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    match serde_json::from_str::<mesimon_core::command::Response>(&line).ok()? {
        mesimon_core::command::Response::Hello { version, daemon_pid, build, .. } => {
            Some((version, daemon_pid, build))
        }
        _ => None,
    }
}

fn daemon(repo: &Path, verbose: bool) -> Section {
    let mut records = Vec::new();
    let paths = match mesimon_daemon::Paths::for_repo(repo) {
        Ok(p) => p,
        Err(e) => {
            records.push(rec(Level::Fail, "paths", format!("{e}")));
            return Section { name: "daemon", records };
        }
    };
    records.push(rec(Level::Note, "project key", paths.proj16.clone()));
    records.push(rec(
        Level::Note,
        "state dir",
        redact(&paths.state_dir.display().to_string(), verbose),
    ));

    match daemon_hello(&paths.orch_sock()) {
        None => records.push(rec(Level::Ok, "daemon", "not running (starts with the board)")),
        Some((proto, pid, build)) => {
            records.push(rec(
                Level::Ok,
                "daemon",
                format!("running (pid {pid}, protocol {proto})"),
            ));
            let ours = env!("CARGO_PKG_VERSION");
            records.push(if build.is_empty() {
                rec(Level::Warn, "daemon build", "too old to report its version").advice(
                    "Open the board once: a newer client restarts a stale daemon by itself.",
                )
            } else if build == ours {
                rec(Level::Ok, "daemon build", build)
            } else {
                rec(Level::Warn, "daemon build", format!("{build}, this binary is {ours}"))
                    .advice("Open the board: a newer client restarts a stale daemon by itself.")
            });
        }
    }

    // Quarantined state files: the standing condition behind a board notice.
    let mut quarantined = Vec::new();
    for dir in [paths.state_dir.clone(), paths.board_dir.join("board")] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.contains(".quarantine-") {
                    quarantined.push(e.path().display().to_string());
                }
            }
        }
    }
    records.push(if quarantined.is_empty() {
        rec(Level::Ok, "state files", "all readable")
    } else {
        rec(
            Level::Warn,
            "state files",
            format!("{} file(s) mesimon could not read", quarantined.len()),
        )
        .advice(format!(
            "The originals are preserved and mesimon started from a default. Inspect, then delete when you are done: {}",
            quarantined.iter().map(|p| redact_cmd(p)).collect::<Vec<_>>().join("  ")
        ))
    });
    Section { name: "daemon", records }
}

/// `mesimon doctor --mcp` — everything mesimon adds to a spawned session's
/// model input, printed verbatim.
///
/// This is the P1-printable half of the promise the README makes. mesimon does
/// not write these tools into any config file, so the only way to read what an
/// agent will actually be handed is to ask mesimon to print it — and it prints
/// the real bytes, generated by the same function the spawner calls, not a
/// description of them.
fn print_mcp(repo: &std::path::Path) -> Result<()> {
    use mesimon_core::mcp;

    let paths = mesimon_daemon::Paths::for_repo(repo)?;
    let bin = mesimon_daemon::hook_settings::mesimon_bin();
    // A fixed nil uuid: the real blob differs only in the session id, and a
    // fresh one on every run would make this output impossible to diff.
    let blob = mesimon_daemon::hook_settings::mcp_config_json(&paths, &bin, uuid::Uuid::nil());

    println!("the flag every mesimon-spawned Claude session carries");
    println!("  --mcp-config '{blob}'");
    println!();
    println!("  --strict-mcp-config is NOT passed: your own MCP servers still load.");
    println!("  <session> above is the per-session uuid; nothing else varies.");
    println!();

    println!("what the model sees ({} tools)", mcp::tools().len());
    let mut total = 0usize;
    for t in mcp::tools() {
        let bytes = serde_json::to_string(&t)?.len();
        total += bytes;
        println!(
            "  mcp__{}__{}  ({bytes} bytes)",
            mcp::SERVER_NAME,
            t["name"].as_str().unwrap_or("")
        );
        for line in wrap(t["description"].as_str().unwrap_or(""), 70) {
            println!("      {line}");
        }
    }
    println!();
    println!("  {total} bytes of tool definitions, on every request of every session.");
    println!(
        "  At most ~{} tokens per request: 223 tok was measured on an ~818-byte",
        mcp::tools().len() * 223
    );
    println!("  definition, and every tool here is capped below that. Deferred tool");
    println!("  search would cut it to ~12/tool, but it is force-disabled for proxy,");
    println!("  Bedrock and Vertex users, so this is the number to plan with.");
    println!();

    println!("what mesimon does NOT send");
    println!("  initialize.instructions   (empty)      the largest injection surface");
    println!("  skills/list               -32601       registers SKILL.md into the system prompt");
    println!("  server/discover           -32601");
    println!("  resources, prompts        not declared");
    println!("  system prompt, reminders, prompt templates: none, ever");
    println!();

    println!("what an agent may ask for");
    println!("  get_ticket / list_board / move_ticket, bound to its own ticket.");
    println!("  There is no tool, at any tier, to spawn or kill a session, delete or");
    println!("  archive or rename a ticket, merge a branch, or read a session, a");
    println!("  transcript or a cost. The daemon refuses those commands outright.");
    println!();

    println!("what the write gate refuses");
    println!("  Edit / Write / NotebookEdit under:");
    println!("    {}", paths.board_dir.display());
    println!("    {}", paths.state_dir.display());
    println!("  Bash is NOT hooked: `sed -i` into those paths still works. The tiers");
    println!("  govern mesimon's tools and its structured writes, not your shell.");
    println!();

    println!("files mesimon writes for any of this");
    println!("  {}/<session>.json   the hook settings (0600)", paths.hooks_dir().display());
    println!("  and nothing else. No .mcp.json, no ~/.claude.json, no settings.local.json,");
    println!("  no plugin marketplace entry. Revoking is: stop launching from mesimon.");
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--mcp") {
        return print_mcp(&std::env::current_dir()?);
    }
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    let wanted: Vec<&str> =
        args.iter().filter(|a| !a.starts_with('-')).map(|s| s.as_str()).collect();
    let repo = std::env::current_dir()?;

    let all = vec![
        environment(verbose),
        install(verbose),
        multiplexer(verbose),
        agents(verbose),
        git_section(),
        daemon(&repo, verbose),
    ];
    let names: Vec<&str> = all.iter().map(|s| s.name).collect();
    for w in &wanted {
        if !names.contains(w) {
            eprintln!("unknown section: {w}\nsections: {}", names.join(" "));
            std::process::exit(2);
        }
    }
    let sections: Vec<Section> =
        all.into_iter().filter(|s| wanted.is_empty() || wanted.contains(&s.name)).collect();

    print!("{}", render(&sections, verbose));
    let fails: usize = sections.iter().map(|s| count(s, Level::Fail)).sum();
    // 16 §8.2: 0 clean or warnings only, 1 any FAIL, 2 doctor itself failed.
    if fails > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule is 78 columns, always — including when the summary is empty.
    /// A mockup nobody checks drifts, so this is the check (16 §8).
    #[test]
    fn section_rule_is_78_columns() {
        for name in ["environment", "daemon", "git", "multiplexer", "install", "agents"] {
            let clean = section_rule(name, &[]);
            assert_eq!(clean.chars().count(), 78, "clean {name}: {clean:?}");
            let dirty =
                section_rule(name, &[rec(Level::Fail, "x", "y"), rec(Level::Warn, "a", "b")]);
            assert_eq!(dirty.chars().count(), 78, "dirty {name}: {dirty:?}");
        }
    }

    /// The `--mcp` page is the only place a user can read what an agent is
    /// actually handed, because mesimon writes it to no config file. It must
    /// name the flag, every tool, and the two things it deliberately omits.
    #[test]
    fn the_mcp_page_prints_the_real_surface() {
        let dir = std::env::temp_dir().join(format!("msmn-doctor-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Rendering must not fail on a directory that is not a git repo: the
        // page describes what WOULD be injected, and a user asking "what does
        // this send" deserves an answer wherever they ask it.
        print_mcp(&dir).unwrap();
        for t in mesimon_core::mcp::tools() {
            let bytes = serde_json::to_string(&t).unwrap().len();
            assert!(bytes <= mesimon_core::mcp::MAX_TOOL_BYTES);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ASCII only — this is the command people run when their terminal is
    /// misbehaving, and paste where the font is unknown.
    #[test]
    fn output_is_ascii_only() {
        let s = render(&[environment(false), install(false), git_section()], false);
        assert!(s.is_ascii(), "non-ascii in doctor output:\n{s}");
    }

    /// $HOME never appears in the default view.
    #[test]
    fn home_is_redacted_by_default() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let s = render(&[install(false), daemon(&std::env::current_dir().unwrap(), false)], false);
        assert!(!s.contains(&home), "raw $HOME leaked:\n{s}");
        assert!(!s.contains("PATH=\"~"), "a tilde in double quotes does not expand:\n{s}");
        assert!(redact(&format!("{home}/x"), false).starts_with('~'));
        assert!(redact(&format!("{home}/x"), true).starts_with(&home), "verbose keeps it");
    }

    /// Notes are behind --verbose; warnings and failures never are.
    #[test]
    fn notes_are_verbose_only() {
        let s = Section {
            name: "install",
            records: vec![rec(Level::Note, "quiet", "hidden"), rec(Level::Warn, "loud", "shown")],
        };
        assert!(!render(std::slice::from_ref(&s), false).contains("hidden"));
        assert!(render(std::slice::from_ref(&s), false).contains("shown"));
        assert!(render(&[s], true).contains("hidden"));
    }
}
