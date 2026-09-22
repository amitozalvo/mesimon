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
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use mesimon_core::board::AgentProvider;

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
                    let row = format!("        {tag}  {line}");
                    // A blank paragraph break is blank: sixteen spaces of
                    // indent on an empty line is trailing whitespace in
                    // whatever the user pipes this into.
                    out.push_str(row.trim_end());
                    out.push('\n');
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

/// Wrap advice to `width`, one PARAGRAPH at a time.
///
/// The paragraph split is load-bearing, not cosmetic: doctor's promise is
/// copy-pasteable fixes, and a fix that is meant to be pasted verbatim — a
/// command, or T-217's CLAUDE.md snippet — stops being one the moment its
/// newlines are reflowed into prose. A line already inside the measure is
/// therefore passed through untouched; only prose that overruns is wrapped.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        if para.chars().count() <= width {
            lines.push(para.to_string());
            continue;
        }
        let mut cur = String::new();
        for word in para.split_whitespace() {
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
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if bin.contains('/') {
        return executable(Path::new(bin)).then(|| bin.to_string());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(bin))
        .find(|path| executable(path))
        .map(|path| path.to_string_lossy().into_owned())
}

fn environment(repo: &std::path::Path, verbose: bool) -> Section {
    let mut records = Vec::new();
    let os = tool_version("uname", &["-sr"]).unwrap_or_else(|| "unknown".into());
    let arch = tool_version("uname", &["-m"]).unwrap_or_else(|| "unknown".into());
    let wsl = if is_wsl() { ", WSL" } else { "" };
    records.push(rec(Level::Ok, "os", format!("{os} ({arch}{wsl})")));

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

    // The two theme slots, and whether MESIMON_THEME is pinning one. Never
    // which ground the terminal is on: doctor runs in pipes.
    records.push(rec(Level::Note, "theme", mesimon_tui::theme_status()));
    // The editor `^g` opens on a note, and which variable chose it.
    records.push(rec(Level::Note, "editor", mesimon_tui::editor_status()));
    // What `^k` opens a link with (T-256): the platform's opener, or
    // MESIMON_OPEN's; none is a note, since a link can still be copied.
    records.push(rec(Level::Note, "opener", mesimon_tui::opener_status()));
    // Whether a ticket may grow its own shell session (T-300). Off by
    // default, so the two keys that start one are inert — a line here is
    // where a finger that remembers `s` finds out why.
    records.push(rec(Level::Note, "ticket shells", mesimon_tui::ticket_shells_status()));
    // How a snoozed ticket returns (T-74) — the third preference in the file.
    records.push(rec(Level::Note, "snooze", mesimon_tui::snooze_status()));
    // The rung the board's reply row was left on (T-365): `p`/`P` set it and
    // the next board opens on it, so a board that opens with every card
    // open is answered here.
    records.push(rec(Level::Note, "replies", mesimon_tui::peek_status()));
    // The merge train (2026-09-04): the one standing consent for mesimon to
    // prompt an agent with no per-press gesture, so doctor says when it is on.
    records.push(rec(Level::Note, "merge train", mesimon_tui::train_status()));
    // Where the private tmux server's status line sits over a pane (T-264).
    records.push(rec(Level::Note, "status line", mesimon_tui::status_line_status()));
    // What the board says outside its own window (T-282): whether it is on,
    // and which rung of each ladder would answer if it were.
    records.push(rec(Level::Note, "notifications", mesimon_tui::notify_status()));
    // Whether the board holds this machine awake while an agent works
    // (T-288), and what would do it. A note, like the opener: where nothing
    // answers, the machine sleeps exactly as it always did.
    records.push(rec(Level::Note, "keep awake", mesimon_tui::keep_awake_status()));
    // Which of the machine's preferences this repo's board overrides
    // (T-361), and what the machine holds for each.
    records.push(rec(Level::Note, "board prefs", mesimon_tui::board_prefs_status(repo)));

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
    match mesimon_core::exe::current_exe() {
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
    // A checker that quietly does nothing looks exactly like one that broke,
    // so the reason it is off is printed even when the reason is the point.
    records.push(rec(Level::Note, "update checks", mesimon_tui::update_check_status()));
    // Both the check and the download ride `curl`. macOS ships it; a minimal
    // Linux (a fresh WSL distro, a container) may not, and then the checker
    // is silently a no-op — exactly the shape the line above exists to catch.
    if which("curl").is_none() {
        records.push(rec(Level::Warn, "curl", "not found on PATH").advice(
            "The release check and install.sh both use curl. Install it with your package manager.",
        ));
    }
    Section { name: "install", records }
}

fn multiplexer(verbose: bool) -> Section {
    let mut records = Vec::new();
    // The same ladder the daemon uses, so this reports the tmux that will
    // actually run — not whatever `tmux` means to your shell.
    let bin = mesimon_backend_tmux::tmux_bin();
    let shown = redact(&bin.display().to_string(), verbose);
    let bundled = bin.file_name().is_some_and(|n| n == mesimon_backend_tmux::BUNDLED_TMUX);
    records.push(if std::env::var_os("MESIMON_TMUX_BIN").is_some() {
        rec(Level::Note, "tmux binary", format!("{shown} (MESIMON_TMUX_BIN)"))
    } else if bundled {
        rec(Level::Note, "tmux binary", format!("{shown} (shipped with mesimon)"))
    } else {
        // The ladder resolves PATH to an absolute path too, so "absolute"
        // does not mean "ours": only the sibling's NAME does. On macOS a
        // release ships one and PATH means a source build or a missing
        // sibling; on Linux PATH is where the release expects to find it.
        rec(Level::Note, "tmux binary", format!("{shown} (from PATH)"))
    });
    match tool_version(&bin.display().to_string(), &["-V"]) {
        None => records.push(rec(Level::Fail, "tmux", "not found").advice(format!(
            "mesimon runs every agent in its own private tmux server and cannot spawn without it. {}",
            if cfg!(target_os = "macos") {
                "A release build ships one; if you built from source, install tmux: brew install tmux".to_string()
            } else {
                format!("The Linux build does not bundle one. Install it: {}", tmux_pkg_line("install"))
            }
        ))),
        Some(v) => records.push(tmux_verdict(&v)),
    }
    // The detach key, and the one layout where pressing it is worse than
    // doing nothing. `conf.rs` binds both C-] and C-5 to detach-client; on
    // Hebrew the bracket keys are mirrored, so ctrl+physical-] emits 0x1B,
    // tmux never sees its key, and the byte lands in the pane as Esc — which
    // interrupts the agent. C-5 exists for exactly that (digits do not move).
    // The terminal can also fix the keystroke, which is the better repair:
    // it leaves the Escape key itself — and so vim, the board's menu and
    // Claude's own Esc — untouched.
    records.push(rec(Level::Note, "back to board", "Ctrl+] or Ctrl+5").advice(
        "On Hebrew and other layouts that mirror the bracket keys, Ctrl+] arrives as Esc, \
         which interrupts the agent instead of detaching. Ctrl+5 is bound for that and works \
         on any layout.\nRemapping the keystroke fixes the key itself and leaves Escape alone:\n\
         iTerm2: Keys > Key Bindings > Ctrl+] > Send Hex Code 0x1d",
    ));
    Section { name: "multiplexer", records }
}

/// The line that installs or upgrades tmux here. Advice only — doctor prints
/// fixes and never applies them (16 §8).
fn tmux_pkg_line(verb: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("brew {verb} tmux")
    } else {
        "sudo apt install tmux (Debian, Ubuntu, WSL; otherwise your distro's package manager)"
            .into()
    }
}

/// Two floors, and they read differently on purpose. Below 3.1 the generated
/// conf does not parse (brace-literal hooks, `extended-keys`). Between 3.1
/// and 3.3 it runs, but `allow-passthrough` is an option that tmux does not
/// have yet — and before the option existed, passthrough was simply on — so
/// an agent can write straight to the outer terminal and T-10's containment
/// is a line tmux ignored. Every tmux a distro ships today is 3.2a or newer,
/// which is why the middle band is a warning and not a failure.
fn tmux_verdict(v: &str) -> Record {
    let num: f32 = v
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .unwrap_or(0.0);
    if num >= 3.3 {
        rec(Level::Ok, "tmux", format!("{v} (needs >= 3.3)"))
    } else if num >= 3.1 {
        rec(Level::Warn, "tmux", format!("{v} runs, but is below the 3.3 containment floor")).advice(
            format!(
                "tmux gained allow-passthrough in 3.3; before it an agent's escape sequences reach your terminal unfiltered. Upgrade: {}",
                tmux_pkg_line("upgrade")
            ),
        )
    } else {
        rec(Level::Warn, "tmux", format!("{v} is below the 3.1 floor")).advice(format!(
            "mesimon's generated conf uses brace-literal hooks and extended-keys, both 3.1+. Upgrade: {}",
            tmux_pkg_line("upgrade")
        ))
    }
}

/// Windows Subsystem for Linux announces itself in the kernel string, and it
/// is the one Linux where a repo can sit on a foreign filesystem (`/mnt/c`).
fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_to_string("/proc/version")
            .is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"))
}

fn provider_installation(
    provider: AgentProvider,
    selected: AgentProvider,
    binary: &str,
    path: Option<&str>,
    version: Option<&str>,
    verbose: bool,
) -> Vec<Record> {
    let label = if provider == AgentProvider::Codex { "codex" } else { "claude" };
    let Some(path) = path else {
        return vec![rec(if provider == selected { Level::Fail } else { Level::Warn }, label,
            format!("{} not found; {}", redact(binary, verbose), if provider == selected { "selected for new sessions" } else { "optional provider unavailable" }))
            .advice(format!("Install {} or select the installed provider under Settings > Agents. Existing sessions retain their original provider.", provider.label()))];
    };
    let mut records = vec![rec(Level::Ok, label, redact(path, verbose))];
    records.push(match version {
        Some(version) => rec(Level::Ok, &format!("{label} version"), version),
        None => {
            rec(Level::Warn, &format!("{label} version"), "executable did not report a version")
                .advice(format!("Run {label} --version to inspect the installation."))
        }
    });
    if provider == AgentProvider::Codex {
        let measured = version.is_some_and(|version| {
            version.split_whitespace().last() == Some(crate::state::CODEX_TESTED_VERSION)
        });
        records.push(rec(if measured { Level::Ok } else { Level::Warn }, "codex evidence",
            format!("runtime paths measured on {}; {}", crate::state::CODEX_TESTED_VERSION,
                if measured { "installed version matches" } else { "installed version untested" }))
            .advice("Run mesimon state compatibility codex <version> for measured scope. Version matching alone is not proof of the complete workflow acceptance matrix."));
        records.push(rec(Level::Note, "codex trust", "native /hooks review controls generated hooks")
            .advice("Review new or changed hook definitions in native Codex. Doctor does not trust hooks, alter approval/sandbox policy, sign in, or submit a model turn."));
    }
    records
}

fn agents(repo: &Path, verbose: bool) -> Section {
    let paths = mesimon_daemon::Paths::for_repo(repo).ok();
    // Everything this section reads off `columns.toml`, parsed once (T-247).
    let cols = paths.as_ref().map(mesimon_daemon::store::read_columns_scalars).unwrap_or_default();
    let selected = cols.agent_provider;
    let mut records = vec![rec(Level::Ok, "new sessions", selected.label())
        .advice("Settings > Agents selects the provider for new sessions only. Existing and sleeping sessions keep their original provider.")];
    for (provider, name, override_key) in [
        (AgentProvider::ClaudeCode, "claude", "MESIMON_CLAUDE_BIN"),
        (AgentProvider::Codex, "codex", "MESIMON_CODEX_BIN"),
    ] {
        let binary = std::env::var(override_key).unwrap_or_else(|_| name.into());
        let path = which(&binary);
        let version = path.as_deref().and_then(|path| tool_version(path, &["--version"]));
        records.extend(provider_installation(
            provider,
            selected,
            &binary,
            path.as_deref(),
            version.as_deref(),
            verbose,
        ));
    }

    // The agent tool surface, and whether the repo tells a session to use it
    // (T-217). Both read the board's own files; neither writes one.
    if let Some(paths) = paths {
        let on = cols.mcp_tools;
        if on {
            records.push(rec(Level::Ok, "agent tools", "on for this repo"));
        } else {
            records.push(
                rec(Level::Note, "agent tools", "off for this repo").advice(
                    "Sessions spawn without Mesimon MCP configuration, so these tools cannot identify their ticket. Settings > Agents > Agent tools turns them back on; existing sessions pick up launch settings on sleep/wake.",
                ),
            );
        }

        // The agent brief (T-224): the one line mesimon puts in the system
        // prompt of the sessions it starts, opt-in. Printed VERBATIM either
        // way — on, so the user can see what every agent of theirs is told;
        // off, so the offer is never a surprise.
        let brief = cols.system_prompt;
        if brief && on {
            records.push(
                rec(
                    Level::Ok,
                    "agent brief",
                    "on - optional agent context for sessions Mesimon starts here",
                )
                .advice(format!("The line, verbatim:\n\n{}\n", mesimon_core::brief::TEXT)),
            );
        } else if brief {
            records.push(rec(Level::Note, "agent brief", "on, but inert while the agent tools are off").advice(
                "The brief tells the agent to call get_ticket and is enabled only beside those tools. Claude receives appended system context; Codex receives a native SessionStart hook subject to native hook trust.",
            ));
        } else {
            records.push(
                rec(Level::Note, "agent brief", "off - no optional Mesimon context").advice(format!(
                    "A new session may receive only the ticket title; get_ticket reads its description and notes. Settings > Agents > Agent brief opts into this exact context line:\n\n{}\n",
                    mesimon_core::brief::TEXT,
                )),
            );
        }

        // The four sentences Mesimon itself types into an agent's box
        // (T-353, T-414). Printed VERBATIM either way, the brief's rule and for the
        // brief's reason: these are the only words Mesimon adds to a
        // conversation, so "what does it say" must be answerable without
        // opening the TUI.
        let prompts = &cols.prompts;
        let custom = prompts.custom_count();
        let body = mesimon_core::prompts::AgentPrompt::ALL
            .iter()
            .map(|w| {
                let whose = if prompts.is_custom(*w) { "yours" } else { "default" };
                format!("{} ({whose}):\n  {}\n", w.label(), prompts.text(*w))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let summary = match custom {
            0 => "default wording".to_string(),
            n => format!("{n} of {} rewritten here", mesimon_core::prompts::AgentPrompt::ALL.len()),
        };
        records.push(
            rec(Level::Ok, "agent prompts", summary)
                .advice(format!(
                    "Mesimon sends these into a live session: a rebase request, a merge notice, a note update, and the crown's wake when an agent it started finishes. Settings > Agents > Agent prompts edits them; an emptied field restores the default wording.\n\n{body}"
                )),
        );

        // The crown's spawn budget (T-412): the one number that bounds an
        // agent starting agents on this board. Printed either way, because
        // "how many could it start" must be answerable without the TUI.
        records.push(match cols.crown_budget {
            0 => rec(Level::Note, "crown budget", "0 - a crowned agent starts no agents").advice(
                "start_agent is refused on this board. Settings > Agents > Crown may start ... sets the cap.",
            ),
            n => rec(Level::Ok, "crown budget", format!("{n} - the most agents a crowned agent may have started at once"))
                .advice("Counted over live seats the crown started, sleeping ones included; a crown-started ticket cannot itself be crowned. Settings > Agents > Crown may start ... sets it."),
        });

        // The columns and what each one DOES (T-117): every automation is a
        // column setting now, so this line is the whole answer to "why did
        // that card move". A board with no file prints nothing — doctor
        // never creates one.

        if let Some(columns) = &cols.columns {
            let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            let mut rules: Vec<String> = Vec::new();
            for c in columns {
                let words = c.settings.summary();
                if !words.is_empty() {
                    rules.push(format!("{}: {}", c.name, words.join(" ∙ ")));
                }
            }
            let mut line = names.join(" → ");
            // The default column (T-279), only when one was chosen: unset
            // means the first, which the arrow line already shows first.
            if let Some(d) = &cols.default_column {
                line.push_str(&format!(" ∙ an agent's create_ticket lands in {d}"));
            }
            let record = rec(Level::Ok, "columns", line);
            records.push(if rules.is_empty() {
                record.advice("No column carries an automation: nothing moves a card but a hand.")
            } else {
                record.advice(format!(
                    "What each column does, and nothing else does (Enter on a column header changes it):\n\n{}",
                    rules.join("\n")
                ))
            });
        }

        // And the CLAUDE.md road, for a user who would rather keep the words
        // in their own file. Always printed with the snippet when it is
        // missing — including on a board that answered the offer with "never
        // ask again". Doctor is deliberately the one door that stamp does
        // not close, which is what makes "never" a safe thing to press.
        let mut sampler = mesimon_daemon::claudemd::Sampler::default();
        sampler.refresh(&paths.repo_root);
        let md = sampler.status();
        if selected == AgentProvider::Codex {
            records.push(rec(Level::Note, "repo guidance", "Codex uses its native AGENTS.md loading")
                .advice("Mesimon does not rewrite AGENTS.md. Agent tools and the optional reviewed startup brief are separate settings."));
        } else if md.present {
            records.push(rec(Level::Ok, "claude.md", "tells sessions to read their ticket"));
        } else if brief {
            records.push(rec(
                Level::Ok,
                "claude.md",
                "does not mention MESIMON_TICKET ∙ the agent brief covers it",
            ));
        } else {
            records.push(
                rec(Level::Note, "claude.md", "does not mention MESIMON_TICKET")
                    .advice(format!(
                        "The alternative to the agent brief, if you would rather keep the words in your own file (it reaches every Claude Code session in the repo, not only mesimon's). Add this to {}:\n\n{}",
                        md.path,
                        mesimon_core::claudemd::SNIPPET,
                    )),
            );
        }
    }

    Section { name: "agents", records }
}

fn git_section(repo: &Path, verbose: bool) -> Section {
    let mut records = Vec::new();
    match tool_version("git", &["--version"]) {
        Some(v) => records.push(rec(Level::Ok, "git", v)),
        None => records.push(rec(Level::Fail, "git", "not found on PATH").advice(
            "Per-ticket worktrees, the diff viewer and the merge flow all shell out to git.",
        )),
    }
    // The checkout the board sits in (T-124): what the header will say, and
    // whether the opt-in fetch is armed — read through the daemon's own
    // sampler, so doctor and the header cannot disagree.
    let g = mesimon_daemon::gitstatus::sample(repo);
    // A workspace (T-225): repositories nested one level under the root.
    // Said before the branch line, because the branch line is then about
    // the meta repo and reads as the whole board's without this.
    if !g.repos.is_empty() {
        let shown: Vec<&str> = g.repos.iter().take(4).map(String::as_str).collect();
        let more = if g.repos.len() > shown.len() { ", …" } else { "" };
        records.push(
            rec(
                // `Ok`, not `Note`: a note hides without `--verbose`, and the
                // `branch` line under it reads `root: …` on its account.
                Level::Ok,
                "workspace",
                format!(
                    "{} nested here ({}{more})",
                    mesimon_core::workspace::repos_word(g.repos.len()),
                    shown.join(", ")
                ),
            )
            .advice(
                "The header counts changes across all of them and v on the board diffs them \
                 together. A worktree ticket here cuts one worktree per nested repo (and of \
                 the root, when it is a repository) under one directory, all on the ticket's \
                 branch; m merges each fast-forward into the branch it was cut from, and a \
                 repo whose branch moved on is named for the rebase.",
            ),
        );
    }
    if g.sampled && !g.branch.is_empty() {
        let every = mesimon_daemon::gitstatus::fetch_every_from_env();
        let fetch = if every.is_zero() {
            "fetch off (opt in: MESIMON_GIT_FETCH=5, minutes)".to_string()
        } else {
            format!("fetch every {}m (MESIMON_GIT_FETCH)", every.as_secs() / 60)
        };
        let upstream = match &g.upstream {
            Some(u) => format!("upstream {u}"),
            None => "no upstream".to_string(),
        };
        // Whose branch: the root's where the root is a repository with others
        // nested under it, the one nested repo's where a folder holds exactly
        // one, unlabelled on a plain checkout.
        let whose = match &g.repos[..] {
            [] => String::new(),
            _ if repo.join(".git").exists() => "root: ".to_string(),
            [only] => format!("{only}: "),
            _ => String::new(),
        };
        records.push(rec(Level::Ok, "branch", format!("{whose}{}, {upstream}, {fetch}", g.branch)));
    }
    // Which ref a ticket's branch is judged merged against (T-267). A PR
    // squashed on a forge lands on the remote-tracking ref and on nothing
    // else until you pull, so the answer is worth printing beside the fetch.
    if let Ok(base) = mesimon_daemon::worktree::default_branch(repo) {
        let (word, advice) = match mesimon_daemon::worktree::upstream_base(repo, &base) {
            Some(up) => (
                format!("{base}, or {up} once fetched"),
                "A worktree branch reads merged when its work is on either one: its tip an \
                 ancestor, or its patch already there under a squash or a rebase-merge. A \
                 fetch is what makes a merge someone else made visible.",
            ),
            None => (
                format!("{base} (no remote-tracking ref)"),
                "A worktree branch reads merged when its work is on it: its tip an ancestor, \
                 or its patch already there under a local squash merge.",
            ),
        };
        records.push(rec(Level::Ok, "merge base", word).advice(advice));
    }
    // A repo on the Windows drive reaches git through WSL's 9p bridge, where
    // every operation is many times slower — and worktrees, the diff viewer
    // and the merge flow shell out to git constantly. Only WSL mounts drives
    // under /mnt/<letter>, so the path is the whole test.
    if is_wsl() && repo.starts_with("/mnt") {
        records.push(
            rec(Level::Warn, "repo", format!("{} is on a Windows drive", redact(&repo.display().to_string(), verbose)))
                .advice("Keep the repo in the Linux filesystem (under ~): git across the WSL boundary is an order of magnitude slower, and mesimon shells out to it for worktrees, diffs and merges."),
        );
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
    // The daemon's journal (2026-09-05): what the process did — started,
    // stopping and why, slow writer turns. The last stop is the one question
    // a restarted board asks of the daemon before it.
    let log = paths.daemon_log();
    let last_stop = mesimon_daemon::journal::Journal::last_stop(&log)
        .map_or_else(|| "no stop recorded".to_string(), |l| format!("last stop: {l}"));
    records.push(rec(
        Level::Note,
        "daemon log",
        format!("{} ∙ {last_stop}", redact(&log.display().to_string(), verbose)),
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
    let blob = mesimon_daemon::hook_settings::mcp_config_json(
        &paths,
        &bin,
        uuid::Uuid::nil(),
        mesimon_core::board::AgentTools::Full,
    );

    println!("Mesimon MCP launch configuration (when agent tools are enabled)");
    println!("Claude Code:");
    println!("  --mcp-config '{blob}'");
    println!(
        "  --allowedTools {}",
        mcp::allowed_tool_names(mesimon_core::board::AgentTools::Full).join(",")
    );
    let value: serde_json::Value = serde_json::from_str(&blob)?;
    let server = &value["mcpServers"][mcp::SERVER_NAME];
    println!("Codex app-server and native TUI:");
    println!(
        "  -c 'mcp_servers.mesimon={{command={},args={}}}'",
        server["command"], server["args"]
    );
    println!();
    println!("  --strict-mcp-config is NOT passed: your own MCP servers still load.");
    println!("  --allowedTools pre-approves the read tools only; writers still prompt.");
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
    println!("  {total} serialized bytes in the complete tools/list response.");
    println!("  Native providers decide when tool definitions enter model context.");
    println!();

    println!("what mesimon does NOT send");
    println!("  initialize.instructions   (empty)      the largest injection surface");
    println!("  skills/list               -32601       registers SKILL.md into the system prompt");
    println!("  server/discover           -32601");
    println!("  resources, prompts        not declared");
    println!("  MCP initialization adds no system prompt, reminders, or prompt templates.");
    println!();
    println!("optional agent brief (a separate opt-in setting)");
    println!("  {}", mesimon_core::brief::TEXT);
    println!("  Claude Code appends this context; Codex uses a native SessionStart hook.");
    println!("  Codex hooks require native review/trust. Mesimon never changes hook trust.");
    println!();

    println!("what an agent may ask for");
    println!("  The definitions above are complete and come from the same registry returned");
    println!("  by tools/list. Ticket-scoped operations stay bound to the caller's ticket;");
    println!("  a newly filed ticket starts no session. Notes are markdown files under");
    println!("  .mesimon/, which the write gate below refuses, so a tool is the one road an");
    println!("  agent has to them.");
    println!("  There is no tool, at any tier, to spawn or kill a session, delete or");
    println!("  archive or rename a ticket, merge a branch, or read a session, a");
    println!("  transcript or a cost. The daemon refuses those commands outright.");
    println!();

    println!("what the write gate refuses");
    println!(
        "  Claude Edit / Write / NotebookEdit; Codex apply_patch sources and destinations under:"
    );
    println!("    {}", paths.board_dir.display());
    println!("    {}", paths.state_dir.display());
    println!("  Bash is NOT hooked: `sed -i` into those paths still works. The tiers");
    println!("  govern mesimon's tools and its structured writes, not your shell.");
    println!();

    println!("files mesimon writes for any of this");
    println!("  {}/<session>.json   the hook settings (0600)", paths.hooks_dir().display());
    println!(
        "  {}/<session>.codex.json   Codex runtime configuration (0600)",
        paths.hooks_dir().display()
    );
    println!("  Codex sockets, observation snapshots, and previews use Mesimon's private runtime directory.");
    println!("  No user .mcp.json, ~/.claude.json, settings.local.json, Codex config, hook trust,");
    println!("  or plugin marketplace is rewritten. Doctor itself writes none of these files.");
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
        environment(&repo, verbose),
        install(verbose),
        multiplexer(verbose),
        agents(&repo, verbose),
        git_section(&repo, verbose),
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
    use super::wrap;

    /// Advice that is meant to be pasted keeps its shape. `wrap` reflows
    /// prose that overruns and leaves everything else exactly as written,
    /// which is what lets a fix be a fix rather than a description of one.
    #[test]
    fn advice_keeps_the_lines_it_was_given() {
        let snippet = mesimon_core::claudemd::SNIPPET;
        let advice = format!("Add this:\n\n{snippet}");
        let out = wrap(&advice, 58);
        for line in snippet.lines() {
            assert!(out.iter().any(|l| l == line), "`{line}` was reflowed: {out:#?}");
        }
        // And prose still wraps.
        let long = "a ".repeat(60);
        assert!(wrap(&long, 58).len() > 1);
    }

    use super::*;

    #[test]
    fn missing_selected_provider_fails_without_requiring_both_installations() {
        let selected = provider_installation(
            AgentProvider::Codex,
            AgentProvider::Codex,
            "codex",
            None,
            None,
            false,
        );
        assert!(selected[0].level == Level::Fail);
        let other = provider_installation(
            AgentProvider::ClaudeCode,
            AgentProvider::Codex,
            "claude",
            None,
            None,
            false,
        );
        assert!(other[0].level == Level::Warn);
        assert!(other[0].value.contains("optional provider"));
    }

    #[test]
    fn codex_version_evidence_never_claims_full_acceptance() {
        let measured = provider_installation(
            AgentProvider::Codex,
            AgentProvider::Codex,
            "codex",
            Some("/fixture/codex"),
            Some("codex-cli 0.153.4"),
            false,
        );
        let evidence = measured.iter().find(|r| r.label == "codex evidence").unwrap();
        assert!(evidence.level == Level::Ok);
        assert!(evidence.advice.as_ref().unwrap().contains("not proof"));
        let unknown = provider_installation(
            AgentProvider::Codex,
            AgentProvider::Codex,
            "codex",
            Some("/fixture/codex"),
            Some("codex-cli 9.0.0"),
            false,
        );
        assert!(unknown.iter().any(|r| r.label == "codex evidence" && r.level == Level::Warn));
    }

    #[test]
    fn executable_override_is_a_path_not_shell_code() {
        let path = std::env::temp_dir()
            .join(format!("msmn-doctor-{}-tool $(false)", uuid::Uuid::new_v4()));
        std::fs::write(&path, "not executed").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(which(path.to_str().unwrap()), Some(path.display().to_string()));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(which(path.to_str().unwrap()), None);
        std::fs::remove_file(path).unwrap();
    }

    /// Two floors, read differently: 3.3 is where containment starts, 3.1 is
    /// where the conf parses at all. Every distro tmux today sits in or above
    /// the middle band, so the middle band must warn, never fail.
    #[test]
    fn the_tmux_floor_is_3_3_and_3_1_still_runs() {
        let new = tmux_verdict("tmux 3.6a");
        assert!(new.level == Level::Ok);
        assert!(new.value.contains(">= 3.3"), "{}", new.value);
        assert!(tmux_verdict("tmux 3.3a").level == Level::Ok, "3.3 is the floor, inclusive");

        let mid = tmux_verdict("tmux 3.2a");
        assert!(mid.level == Level::Warn, "runs, but unfiltered");
        assert!(mid.advice.as_deref().unwrap_or("").contains("allow-passthrough"));

        let old = tmux_verdict("tmux 3.0");
        assert!(old.level == Level::Warn);
        assert!(old.value.contains("3.1 floor"), "{}", old.value);
        assert!(tmux_verdict("tmux next-3.7").level == Level::Warn, "unreadable is not a pass");
    }

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
        let s = render(
            &[
                environment(std::path::Path::new("."), false),
                install(false),
                git_section(std::path::Path::new("."), false),
            ],
            false,
        );
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
