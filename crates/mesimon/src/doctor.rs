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
    // What the board does to the terminal's own tab (T-492).
    records.push(rec(Level::Note, "terminal", mesimon_tui::terminal_status()));
    // The subscription quota (T-327): what the line shows, and the last
    // reading each provider gave, from the machine's file — no probe here.
    records.push(rec(Level::Note, "usage", mesimon_tui::usage_status()));
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
            // By the directory, or by a PATH entry whose `mesimon` IS this
            // file: Homebrew on Linux runs us as `<prefix>/opt/mesimon/bin/
            // mesimon` and puts `<prefix>/bin`, a link to the same file, on
            // PATH (exe.rs).
            let same = |p: &str| {
                let found = exe.file_name().map(|n| Path::new(p).join(n));
                matches!(
                    (found.map(std::fs::canonicalize), std::fs::canonicalize(&exe)),
                    (Some(Ok(a)), Ok(b)) if a == b
                )
            };
            let path = std::env::var("PATH").unwrap_or_default();
            let on_path = path.split(':').find(|p| *p == dir || same(p));
            records.push(if let Some(on_path) = on_path {
                rec(Level::Ok, "on PATH", redact(on_path, verbose))
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

fn multiplexer(repo: &Path, verbose: bool) -> Section {
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
    // The private server, asked through the live one (T-690): whether a
    // process under it may read the checkout — which on macOS it may not,
    // once the terminal app that opened the board has quit. The one way to
    // see the cut-off from outside a pane, and the acceptance test of the
    // fix: a server mesimon starts now answers to macOS for itself.
    if let Ok(paths) = mesimon_daemon::Paths::for_repo(repo) {
        let sock = paths.tmux_sock();
        let access = mesimon_backend_tmux::folder_access(&sock, repo);
        let pid = mesimon_backend_tmux::server_pid(&sock);
        records.push(private_server(&access, pid, &bin, &sock));
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

/// The private server's record (T-690): `ok` while a process under it reads
/// the checkout, a note while there is none, and a FAIL with the two
/// repairs — the board's menu row, or the same kill by hand — when macOS
/// has cut it off. The kill line is the tmux the server runs on, never a
/// bare `tmux`: a client from another build refuses the server. It is
/// spelled over three lines with continuations because `wrap` reflows any
/// line past the measure, and a socket path alone is most of it.
fn private_server(
    access: &mesimon_backend_tmux::Access,
    pid: Option<u32>,
    bin: &Path,
    sock: &Path,
) -> Record {
    use mesimon_backend_tmux::Access;
    let running = pid.map_or_else(|| "running".to_string(), |p| format!("running (pid {p})"));
    match access {
        Access::NoServer => {
            rec(Level::Note, "private server", "not running (starts with the first session)")
        }
        Access::Readable => {
            rec(Level::Ok, "private server", format!("{running}, reads the checkout"))
        }
        Access::Denied(why) => {
            let name = bin
                .file_name()
                .map_or_else(|| "tmux".to_string(), |n| n.to_string_lossy().into_owned());
            rec(Level::Fail, "private server", format!("{running}, cannot read the checkout: {why}"))
                .advice(format!(
                    "macOS keys a folder permission (Documents, Desktop, Downloads) to the app that \
                     started this server, and that terminal has quit; every agent under it dies at \
                     launch. Restart the server: open the board and take `Restart the private tmux \
                     server` from the Esc menu, or run\n{} \\\n  -S {} \\\n  kill-server\nSessions \
                     park and wake back; `!` terminals close. If macOS then asks whether {name} may \
                     access the folder, allow it: the new server answers for itself and outlives \
                     any terminal.",
                    redact_cmd(&bin.display().to_string()),
                    redact_cmd(&sock.display().to_string()),
                ))
        }
    }
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

/// The tiers a person made, each with their words on when to use it
/// (T-584), which the crown reads in `list_board` to pick one. `None` while
/// nobody made a tier.
fn tiers_line(book: &mesimon_core::tier::Book) -> Option<String> {
    let names: Vec<String> = book
        .custom()
        .into_iter()
        .map(|(t, _)| match mesimon_core::tier::sanitize_description(&t.description) {
            words if words.is_empty() => t.name,
            words => format!("{} ({words})", t.name),
        })
        .collect();
    (!names.is_empty()).then(|| names.join(" ∙ "))
}

fn agents(repo: &Path, verbose: bool) -> Section {
    let paths = mesimon_daemon::Paths::for_repo(repo).ok();
    // Everything this section reads off `columns.toml`, parsed once (T-247).
    let cols = paths.as_ref().map(mesimon_daemon::store::read_columns_scalars).unwrap_or_default();
    // The default tier (T-443) over both layers — the machine's `tiers.toml`
    // read without touching it, and the board's own. Its provider is who a
    // ticket that picked nothing starts.
    let machine = mesimon_daemon::paths::machine_tiers_file()
        .ok()
        .and_then(|p| mesimon_daemon::store::read_machine_tiers(&p))
        .unwrap_or_default();
    let layer = mesimon_core::board::Board {
        agent_provider: cols.agent_provider,
        default_tier: cols.default_tier.clone(),
        tiers: cols.tiers.clone(),
        ..Default::default()
    };
    let book = mesimon_core::tier::Book::new(&machine, &layer);
    let default = book.default_tier();
    let selected = default.provider;
    let mut records = vec![rec(
        Level::Ok,
        "default tier",
        format!("{} ∙ {}", default.name, default.summary()),
    )
    .advice("Settings > Agents > Default tier picks what a ticket starts on; ^n on a ticket picks its own. Existing and sleeping sessions keep their original provider.")];
    if let Some(line) = tiers_line(&book) {
        records.push(rec(Level::Note, "tiers", line));
    }
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
        records.push(claude_road(&paths));
        records.push(cost_sources(&paths));
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
            n => rec(Level::Ok, "crown budget", format!("{n} - the most awake agents a crowned agent may have started at once"))
                .advice("Counted over the awake agents the crown started; a sleeping one frees its seat until it is woken. A crown-started ticket cannot itself be crowned. Settings > Agents > Crown may start ... sets it."),
        });

        // The crown's mode (T-610, over T-550's and T-569's switches):
        // whether its words reach another agent with no person between
        // them, and whether it answers a question in another agent's dialog
        // and accepts its plan, a decision made for the person. Printed
        // either way, beside the budget that bounds what its words can wake.
        records.push(if cols.crown_mode.sends() {
            rec(Level::Note, "crown mode", "autonomous - the crown's asks go to agents it started once they are idle, and it may answer their questions and accept their plans")
                .advice("An ask_agent to an agent the crown started is delivered by the queue without your ^y; words that would wake a sleeping agent need a free seat in the crown budget. A question or plan from such an agent wakes the crown; answer_agent types the answer into the dialog and accept_plan presses the plan dialog's default row, and the feed and the card say what it did. It raises its hand for a question or a plan that is yours to decide. An agent you started, and every permission, secret or form, still waits for you. Settings > Agents > Crown mode makes it supervised.")
        } else {
            rec(Level::Ok, "crown mode", "supervised - every ask_agent waits on its card for your ^y, and every question and plan an agent stops on waits for you")
                .advice("Settings > Agents > Crown mode makes it autonomous: the crown delivers to the agents it started, answers their questions and accepts their plans; an agent you started always waits for you.")
        });

        records.push(crown_archives(cols.crown_archives));

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

        // The worktree init script (T-614): the one file in the repository
        // mesimon reads before an agent starts in a fresh worktree, and never
        // writes. Printed either way, because "why does every worktree build
        // from nothing" is answered here, and so is "what runs before my
        // agent starts".
        let init_script = mesimon_core::workspace::INIT_SCRIPT;
        records.push(if mesimon_daemon::worktree::init_script(&paths.repo_root).is_some() {
            rec(Level::Ok, "worktree init", format!("{init_script} runs in each new worktree before its agent starts"))
                .advice("Read from this checkout, run in the new worktree through your login shell's environment, with MESIMON_CHECKOUT (this checkout), MESIMON_WORKTREE (the worktree) and MESIMON_TICKET set. Executable, it runs as itself; otherwise under sh. Its output goes to the daemon journal, the activity feed keeps its exit code and time, and a failure marks the ticket page and never holds the agent back. Ten minutes is the limit (MESIMON_WORKTREE_INIT_MS).")
        } else {
            rec(Level::Ok, "worktree init", "none - every worktree starts cold")
                .advice(format!("A {init_script} at the repository root runs once in each new worktree before its agent starts: seed a build cache (cp -c -R \"$MESIMON_CHECKOUT/target\" target on macOS), install dependencies (npm ci, uv sync), anything. Mesimon never creates it; docs/USING.md has the examples."))
        });

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

/// Whether the crown takes a card off the board (T-590): a person's gesture,
/// off by default, printed either way.
fn crown_archives(on: bool) -> Record {
    if on {
        rec(Level::Note, "crown archives", "on - the crown may archive and restore tickets")
            .advice("archive_ticket takes a ticket off the board and reclaims its merged worktree, and restores one; the card and the feed say what it did. It is refused while a session on the ticket is awake. Settings > Agents > Crown archives tickets turns it off.")
    } else {
        rec(Level::Ok, "crown archives", "off - the crown may not archive or restore tickets")
            .advice("A DONE ticket keeps its merged worktree until you archive it. Settings > Agents > Crown archives tickets lets the crown archive and restore tickets.")
    }
}

/// How Claude sessions report to this board (T-574): which road the last
/// Claude launch got and why (`<state>/mod/road.json`, because the daemon's
/// seam is not in doctor's environment). No setting chooses the road
/// (T-588), and since T-577 the mod carries a session alone.
fn claude_road(paths: &mesimon_daemon::Paths) -> Record {
    use mesimon_daemon::modroad::{read_verdict, Source};
    let advice = "mesimon loads its mod where Claude Code is 2.1.287 or newer, `claude plugin validate` passes on it and Claude Code loads mods: the mod reports the session's events, refuses writes to the board's files and serves the board's tools. Below that, the hook set and the MCP server mesimon generates. Each launch decides; `mesimon state ping <KEY>` times one session's mod.";
    let Some(v) = read_verdict(paths) else {
        return rec(Level::Note, "claude road", "auto ∙ no Claude launch yet").advice(advice);
    };
    let mut value = v.road.word().to_string();
    if v.source == Source::Seam {
        value.push_str(&format!(" ∙ MESIMON_CLAUDE_ROAD={}", v.setting));
    }
    if let Some(probe) = &v.probe {
        value.push_str(&format!(" ∙ {probe}"));
    }
    if let Some(e) = &v.lay_error {
        return rec(Level::Warn, "claude road", value)
            .advice(format!("The mod could not be laid, so launches take the hook set: {e}"));
    }
    // Claude Code turning mods off (T-598), or keeping the hook events from
    // them on a Team or Enterprise account (T-650), is its own call and
    // costs the board nothing: the hook set carries every session as it did
    // before 2.1.287, or the mod reports from Claude Code's own events with
    // one hook for permissions (T-658), and the probe asks again by itself.
    if v.mods_off {
        return rec(Level::Ok, "claude road", value).advice(format!(
            "Nothing is needed from you: Claude Code has mods turned off, or keeps hook events from a person's plugins (a Team or Enterprise account). With mods off, the hook set reports each session as it did before 2.1.287. On a Team or Enterprise account the mod still loads: it reports each session from Claude Code's own events and carries the prompts and the board's tools, and one hook beside it reports permission requests. mesimon asks again at its next start and every 6 hours. {advice}"
        ));
    }
    let level = if v.fallback { Level::Warn } else { Level::Ok };
    let advice = if v.fallback {
        format!("The mod stopped passing its checks after a Claude Code update (`claude plugin validate`, or `claude plugin test` on the load probe), so launches take the hook set. {advice}")
    } else {
        advice.into()
    };
    rec(level, "claude road", value).advice(advice)
}

/// Where the tickets' token counts come from (T-581): a Claude session
/// whose mod reports its turns is counted from those reports, and its
/// transcript is still read beside them. Where the two disagree, this line
/// says by how much; the board shows the mod's.
fn cost_sources(paths: &mesimon_daemon::Paths) -> Record {
    let advice = "A Claude session on the mod road is counted from Claude Code's own report of each turn; its transcript is read beside it as a check. The hook set's sessions and Codex are counted from their transcripts. A turn still running reads ahead on the transcript until it ends.";
    let Some(ledger) = mesimon_daemon::cost::read_only(paths) else {
        return rec(Level::Note, "costs", "nothing counted yet").advice(advice);
    };
    let by_mod = ledger.by_mod();
    if by_mod == 0 {
        return rec(Level::Note, "costs", "from the transcripts").advice(advice);
    }
    let apart = ledger.disagreements();
    let tickets = |n: usize| if n == 1 { "1 ticket".to_string() } else { format!("{n} tickets") };
    let mut value = format!("{} counted from the mod's reports", tickets(by_mod));
    let Some((_, m, t)) = apart.iter().max_by_key(|(_, m, t)| m.abs_diff(*t)) else {
        value.push_str(", each agreeing with its transcript");
        return rec(Level::Note, "costs", value).advice(advice);
    };
    value.push_str(&format!(
        "; {} disagree with the transcript, the most by {} tokens (mod {m}, transcript {t})",
        apart.len(),
        m.abs_diff(*t)
    ));
    rec(Level::Note, "costs", value).advice(advice)
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
    records.push(archived_trees(&paths));
    Section { name: "daemon", records }
}

/// How long `archived_trees` walks for a size before it settles for a floor.
const TREE_SIZE_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// The worktrees of archived tickets still on disk (T-481): the number that
/// would have said "126 GB" weeks before the disk filled. An archived
/// ticket's tree stays while its work has not landed, which is what keeps
/// the archive reversible, and the daemon tears it down once it has — so a
/// merged one still standing is a WARN, a reclaim that never reached it.
/// Shown as `ok` otherwise, never as a note: a note is hidden without
/// `--verbose`, and the size is the point.
fn archived_trees(paths: &mesimon_daemon::Paths) -> Record {
    use mesimon_daemon::worktree;
    let archived = mesimon_daemon::store::read_archived(paths);
    let bindings = match worktree::load_bindings(paths) {
        Ok(b) => b,
        Err(e) => return rec(Level::Warn, "archived trees", format!("unreadable: {e}")),
    };
    let standing: Vec<&worktree::Binding> = bindings
        .iter()
        .filter(|(id, b)| archived.contains(id) && b.path.is_dir())
        .map(|(_, b)| b)
        .collect();
    if standing.is_empty() {
        return rec(Level::Ok, "archived trees", "none on disk");
    }
    let repo = &paths.repo_root;
    let base = worktree::default_branch(repo).unwrap_or_default();
    let merged = standing
        .iter()
        .filter(|b| {
            !b.branch.is_empty()
                && b.legs(repo, &base)
                    .iter()
                    .all(|l| !l.base.is_empty() && worktree::is_merged(&l.repo, &b.branch, &l.base))
        })
        .count();
    let deadline = std::time::Instant::now() + TREE_SIZE_BUDGET;
    let mut bytes = 0;
    let mut whole = true;
    for b in &standing {
        whole &= mesimon_daemon::resources::tree_bytes(&b.path, Some(deadline), &mut bytes);
    }
    let value = format!(
        "{} on disk, {}{}, {merged} merged",
        standing.len(),
        if whole { "" } else { "at least " },
        gib(bytes)
    );
    if merged == 0 {
        return rec(Level::Ok, "archived trees", value);
    }
    rec(Level::Warn, "archived trees", value).advice(
        "An archived ticket whose work is on the base branch should not keep its worktree: \
         the daemon tears these down when it starts and each time it samples the worktrees. \
         Open the board, since a daemon older than this build restarts itself, then run \
         doctor again. One whose ticket has its terminal open waits for that terminal to \
         close. If the rest stay, `pkill -f \"mesimon daemon\"` stops the daemon (sessions \
         survive it) and the next board starts a fresh one.",
    )
}

fn gib(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MIB;
    if mib < 1024.0 {
        format!("{mib:.0} MiB")
    } else {
        format!("{:.1} GiB", mib / 1024.0)
    }
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
    println!("Claude Code 2.1.287 and newer, with mesimon's mod (see `claude road`):");
    println!("  no --mcp-config and no --allowedTools. The mod registers the same tools,");
    println!("  by the same names, descriptions and input schemas, from");
    println!("  `mesimon mcp --list --tools <tier>`, and serves each call through");
    println!("  `mesimon mcp --call`. Claude Code asks no permission for a tool a mod");
    println!("  registers, in any mode: mesimon checks the tier and the ticket at every");
    println!("  call, which is the same check the MCP server's calls get.");
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
    let registered: usize = mcp::registered_for(mesimon_core::board::AgentTools::Full)
        .iter()
        .map(|t| serde_json::to_string(t).map(|s| s.len()).unwrap_or(0))
        .sum();
    println!("  {registered} bytes registered by the mod (the same, less each readOnlyHint).");
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
        "  {}/   mesimon's mod under the mod road (0700/0600; Claude Code adds types/ and tsconfig.json there), its probe.json and road.json",
        paths.mod_root().display()
    );
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
        multiplexer(&repo, verbose),
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
    /// The private server line (T-690): the three states, and a kill line
    /// every piece of which sits inside doctor's measure, so `wrap` passes
    /// it through and it pastes as one command.
    #[test]
    fn the_private_server_line_names_the_cut_off_and_its_repairs() {
        use super::*;
        use mesimon_backend_tmux::Access;
        let home = std::env::var("HOME").unwrap();
        let bin = std::path::PathBuf::from(format!("{home}/.local/bin/mesimon-tmux"));
        let sock = Path::new("/tmp/mesimon-501/0123456789abcdef/tmux.sock");
        let r = private_server(&Access::NoServer, None, &bin, sock);
        assert!(r.level == Level::Note && r.value.contains("not running"), "{}", r.value);
        let r = private_server(&Access::Readable, Some(42), &bin, sock);
        assert!(r.level == Level::Ok, "{}", r.value);
        assert_eq!(r.value, "running (pid 42), reads the checkout");
        let r =
            private_server(&Access::Denied("Operation not permitted".into()), Some(42), &bin, sock);
        assert!(r.level == Level::Fail);
        assert_eq!(r.value, "running (pid 42), cannot read the checkout: Operation not permitted");
        let advice = r.advice.unwrap();
        assert!(advice.contains("`Restart the private tmux server` from the Esc menu"), "{advice}");
        assert!(advice.contains("whether mesimon-tmux may access"), "{advice}");
        let cmd = "$HOME/.local/bin/mesimon-tmux \\\n  -S /tmp/mesimon-501/0123456789abcdef/tmux.sock \\\n  kill-server";
        assert!(advice.contains(cmd), "{advice}");
        let lines = wrap(&advice, 58);
        for piece in cmd.split('\n') {
            assert!(
                lines.iter().any(|l| l == piece),
                "{piece:?} was reflowed:\n{}",
                lines.join("\n")
            );
        }
    }

    use super::wrap;

    /// T-588: the road line says which road the last Claude launch got and
    /// why, in the probe's own words, and names no settings row.
    #[test]
    fn the_road_line_says_which_road_and_why() {
        use mesimon_core::road::Road;
        use mesimon_daemon::modroad::{write_verdict, RoadVerdict, Source};
        let dir = std::env::temp_dir().join(format!("msmn-doctor-road-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut paths = mesimon_daemon::Paths::for_repo(&dir).unwrap();
        paths.state_dir = dir.join("state");
        let none = super::claude_road(&paths);
        assert_eq!(none.value, "auto ∙ no Claude launch yet");
        assert!(!none.advice.as_deref().unwrap_or_default().contains("Settings"));
        let line = |road, setting: &str, source, probe: Option<&str>| {
            let verdict = RoadVerdict {
                road,
                setting: setting.into(),
                source,
                probe: probe.map(Into::into),
                lay_error: None,
                fallback: false,
                mods_off: false,
            };
            write_verdict(&paths, &verdict);
            super::claude_road(&paths).value
        };
        let auto = Source::Default;
        assert_eq!(
            line(Road::Mod, "auto", auto, Some("claude 2.1.287, the mod validated")),
            "mod ∙ claude 2.1.287, the mod validated"
        );
        assert_eq!(
            line(Road::Hooks, "auto", auto, Some("claude 2.1.280 is older than 2.1.287")),
            "hooks ∙ claude 2.1.280 is older than 2.1.287"
        );
        assert_eq!(
            line(
                Road::Hooks,
                "auto",
                auto,
                Some("claude plugin validate failed on 2.1.290: hooks: no such event")
            ),
            "hooks ∙ claude plugin validate failed on 2.1.290: hooks: no such event"
        );
        assert_eq!(
            line(Road::Hooks, "hooks", Source::Seam, None),
            "hooks ∙ MESIMON_CLAUDE_ROAD=hooks"
        );
        // T-598: Claude Code turned mods off. The line says so in the
        // verdict's words, and the advice asks nothing of the person.
        let off =
            "claude 2.1.288: mods are off in this Claude Code (seen 02:00); the hook set is used";
        write_verdict(
            &paths,
            &RoadVerdict {
                road: Road::Hooks,
                setting: "auto".into(),
                source: auto,
                probe: Some(off.into()),
                lay_error: None,
                fallback: true,
                mods_off: true,
            },
        );
        let r = super::claude_road(&paths);
        assert_eq!(r.value, format!("hooks ∙ {off}"));
        assert!(matches!(r.level, super::Level::Ok), "nothing to fix");
        let advice = r.advice.unwrap_or_default();
        assert!(advice.starts_with("Nothing is needed from you"), "{advice}");
        // T-650: a Team or Enterprise account keeps the hook events from the
        // mod. The same flag, the verdict's own words, the same advice.
        let deaf = "claude 2.1.289: hook events do not reach the mod in this Claude Code (seen 11:44; an enterprise account); the plugin reports from its own events, with one hook for permissions";
        write_verdict(
            &paths,
            &RoadVerdict {
                road: Road::Mod,
                setting: "auto".into(),
                source: auto,
                probe: Some(deaf.into()),
                lay_error: None,
                fallback: true,
                mods_off: true,
            },
        );
        let r = super::claude_road(&paths);
        assert_eq!(r.value, format!("mod ∙ {deaf}"));
        assert!(matches!(r.level, super::Level::Ok), "nothing to fix");
        let advice = r.advice.unwrap_or_default();
        assert!(advice.starts_with("Nothing is needed from you"), "{advice}");
        assert!(advice.contains("Team or Enterprise"), "{advice}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-584: the tiers line names each tier with the person's words on when
    /// to use it, a board's version with the board's words.
    #[test]
    fn the_tiers_line_carries_each_tiers_words() {
        use mesimon_core::board::{AgentProvider, Board};
        use mesimon_core::tier::{Book, Effort, MachineTiers, Tier};
        let tier = |id: &str, name: &str, description: &str| Tier {
            id: id.into(),
            name: name.into(),
            provider: AgentProvider::ClaudeCode,
            model: String::new(),
            effort: Effort::Default,
            description: description.into(),
        };
        let none = MachineTiers::default();
        assert_eq!(super::tiers_line(&Book::new(&none, &Board::default())), None);
        let machine = MachineTiers {
            default_tier: None,
            tiers: vec![tier("A", "quick", "docs, renames"), tier("B", "deep", "refactors")],
        };
        let board = Board {
            tiers: vec![tier("B", "deep", "the daemon's writer"), tier("C", "plain", "")],
            ..Board::default()
        };
        assert_eq!(
            super::tiers_line(&Book::new(&machine, &board)).as_deref(),
            Some("quick (docs, renames) ∙ deep (the daemon's writer) ∙ plain")
        );
    }

    /// T-590: doctor prints the crown's archive switch either way, naming
    /// the row that turns it.
    #[test]
    fn the_crown_archives_line_names_the_row() {
        let off = super::crown_archives(false);
        assert_eq!(off.label, "crown archives");
        assert!(off.value.starts_with("off - "), "{}", off.value);
        let on = super::crown_archives(true);
        assert!(on.value.starts_with("on - "), "{}", on.value);
        for r in [off, on] {
            let advice = r.advice.unwrap();
            assert!(advice.contains("Settings > Agents > Crown archives tickets"), "{advice}");
        }
    }

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

    /// The size walk counts what is under the root, and a budget already
    /// spent says the figure is a floor rather than pretending it is whole.
    #[test]
    fn tree_bytes_sums_a_tree_and_owns_up_to_a_spent_budget() {
        let dir = std::env::temp_dir().join(format!("msmn-doctor-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(dir.join("a.txt"), vec![b'a'; 64 * 1024]).unwrap();
        std::fs::write(dir.join("target/debug/b.o"), vec![b'b'; 64 * 1024]).unwrap();
        let later = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut total = 0;
        assert!(mesimon_daemon::resources::tree_bytes(&dir, Some(later), &mut total));
        assert!(total >= 128 * 1024, "both files counted: {total}");
        let mut floor = 0;
        let now = Some(std::time::Instant::now());
        assert!(
            !mesimon_daemon::resources::tree_bytes(&dir, now, &mut floor),
            "the budget ran out"
        );
        assert!(floor <= total);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(gib(3 * 1024 * 1024), "3 MiB");
        assert_eq!(gib(126 * 1024 * 1024 * 1024), "126.0 GiB");
    }

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
