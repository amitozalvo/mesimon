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
    // How a snoozed ticket returns (T-74) — the third preference in the file.
    records.push(rec(Level::Note, "snooze", mesimon_tui::snooze_status()));
    // The merge train (2026-09-04): the one standing consent for mesimon to
    // prompt an agent with no per-press gesture, so doctor says when it is on.
    records.push(rec(Level::Note, "merge train", mesimon_tui::train_status()));

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

fn agents(repo: &Path, verbose: bool) -> Section {
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

    // The agent tool surface, and whether the repo tells a session to use it
    // (T-217). Both read the board's own files; neither writes one.
    if let Ok(paths) = mesimon_daemon::Paths::for_repo(repo) {
        let on = mesimon_daemon::store::read_mcp_tools(&paths);
        if on {
            records.push(rec(Level::Ok, "agent tools", "on for this repo"));
        } else {
            records.push(
                rec(Level::Note, "agent tools", "off for this repo").advice(
                    "Sessions spawn without --mcp-config, so none of them can see which ticket it is on. The Esc menu's Settings > Agent tools row turns them back on; a running session picks it up when you sleep and wake it.",
                ),
            );
        }

        // The agent brief (T-224): the one line mesimon puts in the system
        // prompt of the sessions it starts, opt-in. Printed VERBATIM either
        // way — on, so the user can see what every agent of theirs is told;
        // off, so the offer is never a surprise.
        let brief = mesimon_daemon::store::read_system_prompt(&paths);
        if brief && on {
            records.push(
                rec(
                    Level::Ok,
                    "agent brief",
                    "on ∙ in the system prompt of every claude mesimon starts here",
                )
                .advice(format!("The line, verbatim:\n\n{}\n", mesimon_core::brief::TEXT)),
            );
        } else if brief {
            records.push(rec(Level::Note, "agent brief", "on, but inert while the agent tools are off").advice(
                "The brief tells claude to call get_ticket, so it rides the argv only beside the tools. Turn the tools back on and the next spawn or wake carries both.",
            ));
        } else {
            records.push(
                rec(Level::Note, "agent brief", "off ∙ sessions get no system-prompt line").advice(format!(
                    "A spawned session is often handed only the ticket's TITLE; its description lives in a note that only the get_ticket tool reaches, so agents skip it. The Esc menu's Settings > Agent brief row puts this line — and only this line — in the system prompt of the claude sessions mesimon starts in this repo:\n\n{}\n",
                    mesimon_core::brief::TEXT,
                )),
            );
        }

        // And the CLAUDE.md road, for a user who would rather keep the words
        // in their own file. Always printed with the snippet when it is
        // missing — including on a board that answered the offer with "never
        // ask again". Doctor is deliberately the one door that stamp does
        // not close, which is what makes "never" a safe thing to press.
        let mut sampler = mesimon_daemon::claudemd::Sampler::default();
        sampler.refresh(&paths.repo_root);
        let md = sampler.status();
        if md.present {
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
                        "The alternative to the agent brief, if you would rather keep the words in your own file (it reaches every claude in the repo, not only mesimon's). Add this to {}:\n\n{}",
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
                 together. Worktree tickets are not offered on a workspace yet: a worktree of \
                 the root would hold none of the code.",
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
            &[environment(false), install(false), git_section(std::path::Path::new("."), false)],
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
