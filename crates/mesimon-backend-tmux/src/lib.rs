//! `SessionBackend` over a private tmux server (docs/19-tmux-backend-v01.md).
//! Private socket + `-f` config, never the user's tmux. Every fact this crate
//! relies on was verified in docs/spikes/T-*.md.

pub mod conf;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use mesimon_core::reconcile::PaneSnapshot;

/// D29: the child environment is built from an allowlist, never inherited.
const ENV_ALLOWLIST: &[&str] =
    &["HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "PATH"];

/// Which tmux to run.
///
/// mesimon treats tmux as a private implementation detail — its own server, its
/// own socket, its own generated conf, never the user's tmux — so it ships one
/// and prefers it over whatever happens to be on PATH. That removes the class
/// of bug where a tester's tmux version behaves differently from the author's,
/// and it means a fresh machine needs nothing installed.
///
/// Ladder: an explicit override, then a `mesimon-tmux` sitting beside our own
/// executable (how the release lays it out), then PATH.
///
/// The sibling is deliberately NOT named `tmux`. Installing under that name
/// would put it on the user's PATH and shadow their own tmux — the exact
/// trespass mesimon promises never to commit — and a bare `tmux` sibling would
/// also mean installing mesimon into, say, /opt/homebrew/bin silently
/// "bundles" whatever tmux already lives there.
pub const BUNDLED_TMUX: &str = "mesimon-tmux";

/// Bounded dialog navigation; remote clients never supply tmux key names.
pub enum DialogKey {
    Up,
    Down,
    Escape,
    /// Ticks or unticks a several-choice row (T-571).
    Space,
}

pub fn tmux_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("MESIMON_TMUX_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = mesimon_core::exe::current_exe() {
        if let Some(sibling) = exe.parent().map(|d| d.join(BUNDLED_TMUX)) {
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    // Resolve against OUR OWN PATH, and return the absolute result. The two
    // rungs above are already absolute; this one has to become absolute for the
    // same reason, because `TmuxBackend::set_path` runs tmux with the USER's
    // captured PATH and a bare name would then be looked up there. A user whose
    // PATH does not happen to include tmux's directory would lose the backend
    // entirely — a spawn failing with ENOENT on the one binary mesimon cannot
    // do without, caused by a PATH change that has nothing to do with tmux.
    which_on_path("tmux").unwrap_or_else(|| PathBuf::from("tmux"))
}

/// First executable named `name` on the current process's `PATH`.
pub(crate) fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| std::fs::metadata(cand).is_ok_and(|m| m.is_file()))
}

/// The field separator in every `-F` format this backend reads back, and it
/// is a PRINTABLE character on purpose.
///
/// Every tmux from 3.2a through 3.5a rewrites a control character in format
/// OUTPUT as `_` (measured 2026-09-01 on Ubuntu 22.04 and 24.04, Debian 12
/// and 13); only 3.6 passes a tab through, and 3.6 was the one tmux this code
/// had ever run against. With `\t` here the parse yielded nothing on every
/// distro tmux — an empty snapshot reads every session as crashed at
/// reconcile, and an empty activity list is an interrupt probe that never
/// fires — while the panes sat there alive. `|` cannot occur in a session
/// name (sid16 is hex) or in a pid, and the one free-text field, `pane_title`,
/// is always last, where `split_once` leaves it whole.
const SEP: char = '|';

/// `#{a}|#{b}|…` — the format string for `fields`, built from the same
/// separator the parsers split on, so the two cannot drift apart.
fn fields(names: &[&str]) -> String {
    let parts: Vec<String> = names.iter().map(|n| format!("#{{{n}}}")).collect();
    parts.join(&SEP.to_string())
}

/// `list-panes -a` output into snapshots: `session|pid|dead|status`, one per
/// line. A line with fewer than three fields is not a pane row and is skipped.
fn parse_snapshot(out: &str) -> Vec<PaneSnapshot> {
    let mut v = Vec::new();
    for line in out.lines() {
        let mut f = line.split(SEP);
        let (Some(name), Some(pid), Some(dead), status) = (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        v.push(PaneSnapshot {
            session_name: name.to_string(),
            pane_pid: pid.parse().unwrap_or(0),
            pane_dead: dead == "1",
            dead_status: status.and_then(|s| s.parse().ok()),
        });
    }
    v
}

/// `session|value` rows. The split is on the FIRST separator, so a value that
/// itself contains one (a pane title can) comes back intact.
fn pairs(out: &str) -> impl Iterator<Item = (&str, &str)> {
    out.lines().filter_map(|l| l.split_once(SEP))
}

/// What one pane is doing, from the poll-bucket fork (`pane_facts`): its
/// name, whether it is a `remain-on-exit` corpse, the foreground process's
/// name (`#{pane_current_command}` — the shell's own at a prompt, the
/// command's while one runs) and its OSC-0 title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneFacts {
    pub session_name: String,
    pub pane_dead: bool,
    pub current_command: String,
    pub title: String,
}

/// `session|dead|command|title` rows. Three splits, so the title — the one
/// free-text field, last — keeps any separator of its own.
fn parse_facts(out: &str) -> Vec<PaneFacts> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, SEP);
            let (Some(name), Some(dead), Some(cmd), Some(title)) =
                (f.next(), f.next(), f.next(), f.next())
            else {
                return None;
            };
            Some(PaneFacts {
                session_name: name.to_string(),
                pane_dead: dead == "1",
                current_command: cmd.trim().to_string(),
                title: title.trim().to_string(),
            })
        })
        .collect()
}

pub struct TmuxBackend {
    sock: PathBuf,
    conf: PathBuf,
    /// The `PATH` every tmux invocation runs with, when the daemon has
    /// captured a fresher one than its own — see [`TmuxBackend::set_path`].
    path: Option<String>,
    /// Kept so the conf can be re-rendered when a rendered preference moves
    /// (see [`TmuxBackend::set_status_position`]).
    pane_died_cmd: Option<String>,
    /// Where the status line sits (T-264): the Settings preference as the
    /// daemon last heard it. Bottom until told otherwise — tmux's default,
    /// so a daemon that was never told changes nothing.
    status_top: bool,
}

/// Visible physical rows and the application's active text cursor. A hidden
/// cursor, dead pane or tmux copy mode has no application input cursor.
#[derive(Debug)]
pub struct InputScreen {
    pub lines: Vec<String>,
    pub cursor: Option<(usize, usize)>,
}

impl TmuxBackend {
    /// `sock` must be short enough for `sun_path` (checked); `conf_dir` holds the
    /// generated tmux.conf (state dir — long paths fine). `pane_died_cmd` (from
    /// `conf::pane_died_cmd`) lands in the conf for FRESH servers only — a
    /// running server never re-reads `-f`, so callers also issue
    /// `install_pane_died_hook` against a live one.
    pub fn new(sock: PathBuf, conf_dir: &Path, pane_died_cmd: Option<&str>) -> Result<Self> {
        if sock.as_os_str().len() > 100 {
            bail!("tmux socket path too long for sun_path: {}", sock.display());
        }
        std::fs::create_dir_all(conf_dir)?;
        let conf = conf_dir.join("tmux.conf");
        std::fs::write(&conf, conf::render(pane_died_cmd, false))?;
        Ok(Self {
            sock,
            conf,
            path: None,
            pane_died_cmd: pane_died_cmd.map(str::to_string),
            status_top: false,
        })
    }

    /// Where the status line sits, as last set.
    pub fn status_top(&self) -> bool {
        self.status_top
    }

    /// Move the status line to the top or the bottom of every pane (T-264).
    ///
    /// Both roads at once, because a running server never re-reads its conf:
    /// the conf is re-rendered so the NEXT server comes up on the right side,
    /// and a live server gets the `set-option`. No server is not a failure —
    /// the conf carries the word until one starts — and the value is held
    /// either way, so the snapshot reports what was asked, not what tmux
    /// happened to be running.
    pub fn set_status_position(&mut self, top: bool) -> Result<()> {
        self.status_top = top;
        std::fs::write(&self.conf, conf::render(self.pane_died_cmd.as_deref(), top))?;
        if self.server_alive() {
            self.run(&["set-option", "-g", "status-position", conf::status_position(top)])?;
        }
        Ok(())
    }

    /// Point every subsequent tmux invocation at a different `PATH`.
    ///
    /// This is the ONLY way to change what a pane's `PATH` will be, and the
    /// reason is a tmux behaviour that is easy to get backwards. tmux takes a
    /// pane's `PATH` from the environment of the *client process issuing the
    /// spawn* — so that a command given by bare name can be resolved — and it
    /// does so in preference to `new-session -e PATH=…`, which lands in the
    /// session's environment table where the child never reads it. Measured
    /// 2026-09-01 against tmux 3.6a: a pane spawned with `-e PATH=/EPATH/bin`
    /// from a client holding `PATH=/CLIENTPATH/bin` came up with the client's,
    /// while `show-environment` on that session reported `/EPATH/bin`; a bare
    /// command present only on the `-e` PATH exited 127.
    ///
    /// That is also why this needs no server restart. The server's own global
    /// environment is frozen at its first launch and `update-environment ""`
    /// keeps it that way, but it is not where a pane's `PATH` comes from.
    ///
    /// `None` restores the daemon's own `PATH`.
    pub fn set_path(&mut self, path: Option<String>) {
        self.path = path.filter(|p| !p.trim().is_empty());
    }

    /// Write `PATH` into the LIVE server's global environment as well.
    ///
    /// Cosmetic for panes — [`TmuxBackend::set_path`] is what they actually
    /// follow — but not cosmetic for a person debugging: `show-environment -g`
    /// is the first thing anyone reads when asking why a pane has the wrong
    /// `PATH`, and a server answering with a two-day-old one sends them down
    /// the wrong road. It is also what the server's own `run-shell` hooks see.
    /// Best-effort: a dead server is not an error here.
    pub fn publish_path(&self) -> Result<()> {
        let Some(path) = self.path.clone() else { return Ok(()) };
        if self.server_alive() {
            self.run(&["set-environment", "-g", "PATH", &path])?;
        }
        Ok(())
    }

    /// (Re-)install the pane-died hook on a live server — idempotent, and how
    /// a server that outlived a daemon restart learns the new binary path.
    pub fn install_pane_died_hook(&self, cmd: &str) -> Result<()> {
        self.run(&["set-hook", "-g", "pane-died", cmd])?;
        Ok(())
    }

    /// (Re-)install the copy-pipe bindings on a live server — same reason as
    /// the hook: a running server never re-reads the conf.
    pub fn install_copy_bindings(&self) -> Result<()> {
        for (table, key, pipe) in conf::copy_pipe_bindings() {
            self.run(&[
                "bind-key",
                "-T",
                table,
                key,
                "send-keys",
                "-X",
                "copy-pipe-and-cancel",
                &pipe,
            ])?;
        }
        Ok(())
    }

    /// Apply the scroll step to servers that survived a daemon reload.
    pub fn install_scroll_bindings(&self) -> Result<()> {
        for (table, key, command) in conf::SCROLL_BINDINGS {
            self.run(&["bind-key", "-T", table, key, command])?;
        }
        Ok(())
    }

    fn tmux(&self) -> Command {
        let mut c = Command::new(tmux_bin());
        c.arg("-S").arg(&self.sock).arg("-f").arg(&self.conf);
        // The server inherits this env on first launch — scrub it (D29, spike T-2).
        // The allowlist is now a floor rather than the whole story: the daemon
        // passes the user's captured shell environment per session through
        // `-e`, and only PATH has to be here, because only PATH is read off the
        // client (see `set_path`).
        c.env_clear();
        for k in ENV_ALLOWLIST {
            if let Ok(v) = std::env::var(k) {
                c.env(k, v);
            }
        }
        // Unit tests own this socket directory. Do not read personal shell
        // startup files; integration tests get the same isolation from their
        // daemon subprocess environment. Production behavior is unchanged.
        #[cfg(test)]
        c.env("HOME", self.sock.parent().expect("test socket directory")).env("SHELL", "/bin/sh");
        if let Some(path) = &self.path {
            c.env("PATH", path);
        }
        c.env("TERM", "xterm-256color");
        c
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.tmux().args(args).output().context("spawning tmux")?;
        if !out.status.success() {
            bail!(
                "tmux {:?} failed: {}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn server_alive(&self) -> bool {
        self.tmux().args(["has-session"]).output().map(|o| o.status.success()).unwrap_or(false)
    }

    /// Spawn a session: tmux session name = sid16, running `argv` in `cwd`.
    ///
    /// No `-e`, deliberately. A pane's environment is the launcher's job
    /// (`mesimon exec --env`): values on a `new-session` command line are
    /// readable by every user on the machine for as long as the spawn runs,
    /// and this argv is exactly where the user's exported secrets used to be.
    ///
    /// Returns the new pane's key (`conf::PANE_KEY`, via `-P -F`): the
    /// identity a death frame is matched against, because a wake reuses the
    /// session NAME (T-245).
    pub fn spawn(&self, sid16: &str, cwd: &Path, argv: &[String]) -> Result<String> {
        let mut args: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            conf::PANE_KEY.into(),
            "-s".into(),
            sid16.into(),
            "-c".into(),
            cwd.display().to_string(),
        ];
        args.extend(argv.iter().cloned());
        let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(self.run(&argrefs)?.trim().to_string())
    }

    /// The key (`conf::PANE_KEY`) of a session's pane — for a pane that
    /// already exists (the adopt road), where `spawn`'s return is not on hand.
    pub fn pane_key(&self, sid16: &str) -> Result<String> {
        Ok(self.run(&["display-message", "-p", "-t", sid16, conf::PANE_KEY])?.trim().to_string())
    }

    /// Type literal text into a session's pane WITHOUT pressing Enter — a
    /// prefill for the agent's input box. `-l` disables key-name lookup so
    /// the text lands verbatim; `--` guards text starting with `-`. Keys
    /// sent before the agent's TUI is ready sit in the pty buffer, so no
    /// readiness wait is needed.
    pub fn send_text(&self, sid16: &str, text: &str) -> Result<()> {
        self.run(&["send-keys", "-t", sid16, "-l", "--", text])?;
        Ok(())
    }

    /// Press Enter in a session's pane, and nothing else — the second half of
    /// a `send_text` prefill the user asked to have submitted. It is a
    /// SEPARATE tmux call issued at a SEPARATE time for the same reason
    /// `paste_text` splits its Enter out: a CR arriving in the same byte burst
    /// as the text is absorbed as pasted content and never submits (T-5's
    /// negative test, reconfirmed 2026-08-31 against a fresh Claude pane).
    /// The caller owns the timing — see `Daemon::arm_owed` and `settle_owed`.
    pub fn send_enter(&self, sid16: &str) -> Result<()> {
        self.run(&["send-keys", "-t", sid16, "Enter"])?;
        Ok(())
    }

    /// One Ctrl+C into a session's pane: Claude Code clears a composer that
    /// holds text on it (T-570). The caller sends exactly one, and only into
    /// a composer it has seen holding text — a second press, or one into an
    /// empty box followed by another, exits Claude.
    pub fn clear_input(&self, sid16: &str) -> Result<()> {
        self.run(&["send-keys", "-t", sid16, "C-c"])?;
        Ok(())
    }

    pub fn dialog_key(&self, sid16: &str, key: DialogKey) -> Result<()> {
        let key = match key {
            DialogKey::Up => "Up",
            DialogKey::Down => "Down",
            DialogKey::Escape => "Escape",
            DialogKey::Space => "Space",
        };
        self.run(&["send-keys", "-t", sid16, key])?;
        Ok(())
    }

    /// Deliver a full prompt and submit it (spike T-5 / 19 §6): `load-buffer -`
    /// from stdin → `paste-buffer -p` (bracketed paste) → a SEPARATE
    /// `send-keys Enter`. A single send-keys call truncated 3696→630 bytes and
    /// ate the Enter; `;`-joined tmux commands split — three forks is the shape.
    pub fn paste_text(&self, sid16: &str, text: &str) -> Result<()> {
        self.paste_input(sid16, text)?;
        self.send_enter(sid16)
    }

    /// Bracketed input without submission. The provider owns readiness and
    /// the delay before Enter; native Codex paste detection needs that gap.
    pub fn paste_input(&self, sid16: &str, text: &str) -> Result<()> {
        use std::io::Write as _;
        use std::process::Stdio;
        let mut child = self
            .tmux()
            .args(["load-buffer", "-b", "msmn-paste", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn tmux load-buffer")?;
        child
            .stdin
            .take()
            .context("load-buffer stdin")?
            .write_all(text.as_bytes())
            .context("write paste buffer")?;
        let st = child.wait().context("wait load-buffer")?;
        if !st.success() {
            bail!("tmux load-buffer failed");
        }
        self.run(&["paste-buffer", "-p", "-b", "msmn-paste", "-d", "-t", sid16])?;
        Ok(())
    }

    /// Discovery snapshot for `reconcile()` after a daemon restart.
    pub fn snapshot(&self) -> Result<Vec<PaneSnapshot>> {
        if !self.server_alive() {
            return Ok(Vec::new());
        }
        let out = self.run(&[
            "list-panes",
            "-a",
            "-F",
            &fields(&["session_name", "pane_pid", "pane_dead", "pane_dead_status"]),
        ])?;
        Ok(parse_snapshot(&out))
    }

    /// Per-pane last-output time, epoch seconds (`#{window_activity}`; tmux
    /// tracks it server-side, attached or not). The working/idle
    /// discriminator for the Esc-interrupt probe (spike S-E): a turn in
    /// flight repaints continuously, an idle prompt emits only sparse
    /// statusline bursts.
    pub fn activity(&self) -> Result<Vec<(String, u64)>> {
        if !self.server_alive() {
            return Ok(Vec::new());
        }
        let out =
            self.run(&["list-panes", "-a", "-F", &fields(&["session_name", "window_activity"])])?;
        Ok(pairs(&out).filter_map(|(name, t)| Some((name.to_string(), t.parse().ok()?))).collect())
    }

    /// Every pane's liveness, foreground command and OSC-0 title in one fork
    /// (the title: tmux reports the hostname when the app never set one —
    /// callers filter, same rule as `pane_title`). The title is the last
    /// field because it is the one that may contain the separator.
    pub fn pane_facts(&self) -> Result<Vec<PaneFacts>> {
        if !self.server_alive() {
            return Ok(Vec::new());
        }
        let out = self.run(&[
            "list-panes",
            "-a",
            "-F",
            &fields(&["session_name", "pane_dead", "pane_current_command", "pane_title"]),
        ])?;
        Ok(parse_facts(&out))
    }

    /// Give a session a new name (T-366: adopting the `!` terminal renames
    /// its pane to the record's `sid16`, so every sid16-keyed road — capture,
    /// kill, the pane-died hook, reconcile — finds it with no other change).
    ///
    /// `-t` PREFIX-matches when no session has the exact name, and
    /// `msmn-term` is a prefix of every `msmn-term-<ulid>`: a caller must
    /// have seen `old` exactly in a snapshot before asking, or a miss here
    /// would rename a neighbour.
    pub fn rename_session(&self, old: &str, new: &str) -> Result<()> {
        self.run(&["rename-session", "-t", old, new])?;
        Ok(())
    }

    /// Kill ladder rung 1: SIGTERM the pane's process group; caller escalates to
    /// `kill_pane` after grace (D23 — never SIGKILL mid-turn from here).
    pub fn signal_session(&self, sid16: &str) -> Result<()> {
        let out = self.run(&["display-message", "-p", "-t", sid16, "#{pane_pid}"])?;
        let pid: i32 = out.trim().parse().context("pane_pid parse")?;
        // Negative pid = process group (tmux setsid's the pane command).
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        Ok(())
    }

    pub fn kill_session(&self, sid16: &str) -> Result<()> {
        self.run(&["kill-session", "-t", sid16])?;
        Ok(())
    }

    /// Replace the focused status line's left text (the breadcrumb). Issued
    /// as a live command — a running server never re-reads its conf, and a
    /// server predating the 120-length conf still holds the old cap, so the
    /// length rides along.
    pub fn set_status_left(&self, text: &str) -> Result<()> {
        self.run(&["set-option", "-g", "status-left-length", "120"])?;
        self.run(&["set-option", "-g", "status-left", text])?;
        // Live servers predate conf wording changes; keep the right side in step.
        self.run(&["set-option", "-g", "status-right", conf::STATUS_RIGHT])?;
        // And the side (T-264): a server that outlived the daemon holding the
        // preference converges on what this daemon holds, on the first focus.
        self.run(&["set-option", "-g", "status-position", conf::status_position(self.status_top)])?;
        // Live servers also predate the C-5 bind (extended-keys makes Ctrl+5 a
        // distinct key, so the C-] bind alone doesn't catch it).
        self.run(&["bind-key", "-T", "root", "C-5", "detach-client"])?;
        // And predate `extended-keys on` with its Shift+Enter bind (T-488):
        // converge, or a live server keeps typing iTerm2's XTVERSION reply
        // into nvim until it is restarted.
        self.run(&["set-option", "-g", "extended-keys", "on"])?;
        let mut bind = vec!["bind-key", "-T", "root", "S-Enter", "send-keys", "-H"];
        bind.extend(conf::SHIFT_ENTER_HEX.split(' '));
        self.run(&bind)?;
        Ok(())
    }

    /// The pane's OSC-0 title (tmux tracks it in `#{pane_title}`). Defaults to
    /// the hostname when the app never set one — callers match app-specific
    /// content, not emptiness (startup-modal probe, 11 §11.5.3).
    pub fn pane_title(&self, sid16: &str) -> Result<String> {
        Ok(self.run(&["display-message", "-p", "-t", sid16, "#{pane_title}"])?.trim().to_string())
    }

    /// Last N logical lines of a pane, already SGR-free (spike T-9: no `-e`).
    pub fn capture_tail(&self, sid16: &str, lines: usize) -> Result<Vec<String>> {
        Ok(self.capture_tail_sized(sid16, lines)?.0)
    }

    /// `capture_tail` plus the pane's width in cells, read in the same tmux
    /// command queue (T-506): a remote screen that knows the width can put a
    /// joined line back where the pane wrapped it, and size its type so the
    /// whole pane fits. 0 when tmux did not say.
    pub fn capture_tail_sized(&self, sid16: &str, lines: usize) -> Result<(Vec<String>, u16)> {
        let out = self.run(&[
            "capture-pane",
            "-p",
            "-J",
            "-t",
            sid16,
            ";",
            "display-message",
            "-p",
            "-t",
            sid16,
            "#{pane_width}",
        ])?;
        let mut rows: Vec<&str> = out.lines().collect();
        let cols = rows.pop().and_then(|w| w.trim().parse().ok()).unwrap_or(0);
        let tail = rows
            .into_iter()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(lines)
            .map(str::to_string)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        Ok((tail, cols))
    }

    pub fn capture_input_screen(&self, sid16: &str) -> Result<InputScreen> {
        // Keep physical rows (no -J and no blank-line filtering): cursor_y
        // indexes this screen. Read screen and cursor in one tmux command queue.
        let format = fields(&["cursor_flag", "cursor_x", "cursor_y", "pane_in_mode", "pane_dead"]);
        let out = self.run(&[
            "capture-pane",
            "-p",
            "-t",
            sid16,
            ";",
            "display-message",
            "-p",
            "-t",
            sid16,
            &format,
        ])?;
        let mut lines: Vec<String> = out.lines().map(str::to_owned).collect();
        let metadata = lines.pop().context("missing input cursor metadata")?;
        let parts: Vec<_> = metadata.split(SEP).collect();
        let cursor = match parts.as_slice() {
            ["1", x, y, "0", "0"] => {
                let x = x.parse::<usize>().context("invalid input cursor column")?;
                let y = y.parse::<usize>().context("invalid input cursor row")?;
                (y < lines.len()).then_some((x, y))
            }
            [_, _, _, _, _] => None,
            _ => bail!("invalid input cursor metadata"),
        };
        Ok(InputScreen { lines, cursor })
    }

    /// How long every client attached to this session has been quiet, in
    /// SECONDS — the freshest one wins, and `None` means nobody is attached
    /// (T-299).
    ///
    /// `#{client_activity}` is the last time tmux read input from that
    /// client, which is the only evidence anywhere that the person is still
    /// at the terminal while the board is handed over: focus reporting is
    /// off for the duration of an attach and the keys go to tmux, not to
    /// the TUI. Seconds, because that is tmux's own resolution here — a
    /// `time_t` on the wire, measured against the same clock this process
    /// reads, so no timezone or format is involved.
    ///
    /// An empty listing is nobody attached, which is not an error: a client
    /// detaching is the ordinary end of every handover. A timestamp in the
    /// FUTURE (a clock stepped back under us) saturates to zero rather than
    /// wrapping into "quiet since the Bronze Age".
    pub fn client_quiet_secs(&self, sid16: &str) -> Result<Option<u64>> {
        let out = self.run(&["list-clients", "-t", sid16, "-F", "#{client_activity}"])?;
        let now = mesimon_core::clock::now_secs();
        Ok(out
            .lines()
            .filter_map(|l| l.trim().parse::<u64>().ok())
            .map(|at| now.saturating_sub(at))
            .min())
    }

    /// argv for the focus handover (docs/19 §2): the TUI execs this as a child.
    pub fn attach_argv(&self, sid16: &str) -> Vec<String> {
        vec![
            // The same binary that started the server, not whatever `tmux`
            // resolves to for the user: a client and server from different
            // tmux builds refuse each other over protocol version, and this
            // argv is exec'd by the TUI for the focus handover.
            tmux_bin().display().to_string(),
            "-S".into(),
            self.sock.display().to_string(),
            "-f".into(),
            self.conf.display().to_string(),
            "attach".into(),
            "-t".into(),
            sid16.into(),
        ]
    }

    pub fn detach_all_clients(&self, sid16: &str) -> Result<()> {
        self.run(&["detach-client", "-s", sid16])?;
        Ok(())
    }

    pub fn kill_server(&self) -> Result<()> {
        if self.server_alive() {
            self.run(&["kill-server"])?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../ci/test_support.rs"]
mod test_support;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str, sock: &str) -> test_support::Fixture {
        let f = test_support::Fixture::new(name, &tmux_bin().to_string_lossy());
        f.register(&f.dir, None, None, &f.dir.join(sock));
        f
    }

    /// The separator is printable, because every tmux before 3.6 rewrites a
    /// control character in format output as `_` — and `spawn_snapshot_kill_roundtrip`
    /// below is the live half of this: on Debian's tmux 3.3a it failed with
    /// the pane alive, which is how the tab was found.
    #[test]
    fn the_format_separator_is_printable_and_a_title_may_contain_it() {
        assert!(!SEP.is_control(), "tmux < 3.6 turns a control character in -F output into `_`");
        assert!(!SEP.is_whitespace(), "a title is trimmed, a whitespace separator would vanish");
        assert_eq!(fields(&["session_name", "pane_pid"]), "#{session_name}|#{pane_pid}");

        let snap = parse_snapshot("abc123|42|0|\ndead1|43|1|7\nnot a pane row\n");
        assert_eq!(snap.len(), 2);
        assert_eq!(
            (snap[0].session_name.as_str(), snap[0].pane_pid, snap[0].pane_dead),
            ("abc123", 42, false)
        );
        assert_eq!(snap[0].dead_status, None);
        assert_eq!((snap[1].pane_dead, snap[1].dead_status), (true, Some(7)));

        let titled: Vec<_> = pairs("abc123|claude | T-12 fix\nbare\n").collect();
        assert_eq!(
            titled,
            [("abc123", "claude | T-12 fix")],
            "split once: the title keeps its own bar"
        );

        let facts = parse_facts("abc123|0|cargo|claude | T-12 fix\nmsmn-term-x|1|zsh|host\nbare\n");
        assert_eq!(
            facts,
            [
                PaneFacts {
                    session_name: "abc123".into(),
                    pane_dead: false,
                    current_command: "cargo".into(),
                    title: "claude | T-12 fix".into(),
                },
                PaneFacts {
                    session_name: "msmn-term-x".into(),
                    pane_dead: true,
                    current_command: "zsh".into(),
                    title: "host".into(),
                },
            ],
            "three splits: the title, last, keeps its own bar"
        );
    }

    #[test]
    fn input_screen_preserves_cursor_rows_and_excludes_hidden_and_copy_mode_cursors() {
        let f = fixture("backend-input-cursor", "t.sock");
        let be = TmuxBackend::new(f.dir.join("t.sock"), &f.dir, None).unwrap();
        let code = "import os,tty\ntty.setraw(0)\nos.write(1,b'\\x1b[2J\\x1b[Hheading\\r\\n> prompt\\r\\n\\r\\ncustom footer\\x1b[2;3H\\x1b[?25h')\nwhile True:\n b=os.read(0,1)\n os.write(1,b'\\x1b[?25l' if b==b'h' else b'\\x1b[?25h')\n";
        be.spawn("cursor", &f.dir, &["python3".into(), "-c".into(), code.into()]).unwrap();
        let wait = |expected| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let screen = be.capture_input_screen("cursor").unwrap();
                if screen.cursor == expected && screen.lines[0] == "heading" {
                    return screen;
                }
                assert!(std::time::Instant::now() < deadline, "{screen:?}");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        };
        let screen = wait(Some((2, 1)));
        assert_eq!(&screen.lines[..4], &["heading", "> prompt", "", "custom footer"]);
        be.send_text("cursor", "h").unwrap();
        wait(None);
        be.send_text("cursor", "s").unwrap();
        wait(Some((2, 1)));
        be.run(&["copy-mode", "-t", "cursor"]).unwrap();
        assert!(be.capture_input_screen("cursor").unwrap().cursor.is_none());
        be.run(&["send-keys", "-t", "cursor", "-X", "cancel"]).unwrap();
        wait(Some((2, 1)));
        be.kill_server().unwrap();
    }

    #[test]
    fn spawn_snapshot_kill_roundtrip() {
        if Command::new("tmux").arg("-V").output().is_err() {
            eprintln!("tmux not installed; skipping");
            return;
        }
        let f = fixture("backend-roundtrip", "t.sock");
        let dir = f.dir.clone();
        let be = TmuxBackend::new(dir.join("t.sock"), &dir, None).unwrap();
        be.spawn("abc123", &PathBuf::from("/tmp"), &["sleep".into(), "60".into()]).unwrap();
        let snap = be.snapshot().unwrap();
        assert!(snap.iter().any(|p| p.session_name == "abc123" && !p.pane_dead));
        // Dead pane preserved by remain-on-exit:
        be.spawn("dead1", &PathBuf::from("/tmp"), &["sh".into(), "-c".into(), "exit 7".into()])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let snap = be.snapshot().unwrap();
        let d = snap.iter().find(|p| p.session_name == "dead1").unwrap();
        assert!(d.pane_dead);
        assert_eq!(d.dead_status, Some(7));
        be.kill_server().unwrap();
    }

    #[test]
    fn wheel_scrolls_one_line_on_fresh_and_surviving_servers() {
        let f = fixture("backend-scroll", "t.sock");
        let be = TmuxBackend::new(f.dir.join("t.sock"), &f.dir, None).unwrap();
        be.spawn("scroll", &f.dir, &["sleep".into(), "60".into()]).unwrap();
        let assert_bindings = || {
            for table in ["copy-mode", "copy-mode-vi"] {
                for (key, direction) in [("WheelUpPane", "up"), ("WheelDownPane", "down")] {
                    let binding = be.run(&["list-keys", "-T", table, key]).unwrap();
                    assert!(
                        binding.contains("select-pane")
                            && binding.contains("send-keys")
                            && binding.contains("-N 1 ")
                            && binding.contains("-X")
                            && binding.trim_end().ends_with(&format!("scroll-{direction}")),
                        "{binding}"
                    );
                }
            }
        };
        assert_bindings();
        // Simulate a server born with the old defaults, then reload twice:
        // the upgrade must cover both key tables and be idempotent.
        for table in ["copy-mode", "copy-mode-vi"] {
            for (key, direction) in [("WheelUpPane", "up"), ("WheelDownPane", "down")] {
                be.run(&[
                    "bind-key",
                    "-T",
                    table,
                    key,
                    &format!("select-pane; send-keys -N 5 -X scroll-{direction}"),
                ])
                .unwrap();
            }
        }
        be.install_scroll_bindings().unwrap();
        be.install_scroll_bindings().unwrap();
        assert_bindings();
        // Applications still receive their mouse events through tmux's
        // root binding, rather than being forced into history scrolling.
        assert!(be
            .run(&["list-keys", "-T", "root", "WheelUpPane"])
            .unwrap()
            .contains("send-keys -M"));
        be.kill_server().unwrap();
    }

    /// The measured tmux behaviour `set_path` rests on, pinned so a future
    /// tmux bump cannot change it silently: a pane's `PATH` comes from the
    /// CLIENT that spawned it. (The other half of the old measurement — that
    /// `-e PATH=…` never reaches the child — no longer matters: nothing
    /// travels through `-e` any more.)
    #[test]
    fn a_panes_path_is_the_clients() {
        if Command::new("tmux").arg("-V").output().is_err() {
            eprintln!("tmux not installed; skipping");
            return;
        }
        let f = fixture("backend-env", "t.sock");
        let dir = f.dir.clone();
        let mut be = TmuxBackend::new(dir.join("t.sock"), &dir, None).unwrap();
        be.set_path(Some("/CLIENT-SENTINEL/bin:/usr/bin:/bin".into()));
        be.spawn("envpr1", &PathBuf::from("/tmp"), &["/usr/bin/env".into()]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let pane = be.capture_tail("envpr1", 200).unwrap().join("\n");
        be.kill_server().unwrap();

        assert!(
            pane.contains("/CLIENT-SENTINEL/bin"),
            "the pane must take PATH from the client env:\n{pane}"
        );
    }

    #[test]
    fn an_empty_path_override_is_refused_rather_than_breaking_every_lookup() {
        // No tmux is started here, so no supervisor: a private dir is enough.
        let dir = std::env::temp_dir().join(format!("msmn-path-refusal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut be = TmuxBackend::new(dir.join("t2.sock"), &dir, None).unwrap();
        be.set_path(Some("   ".into()));
        assert!(be.path.is_none());
        be.set_path(Some("/a/bin".into()));
        assert_eq!(be.path.as_deref(), Some("/a/bin"));
        be.set_path(None);
        assert!(be.path.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
