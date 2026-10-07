//! `SessionBackend` over a private tmux server (docs/19-tmux-backend-v01.md).
//! Private socket + `-f` config, never the user's tmux. Every fact this crate
//! relies on was verified in docs/spikes/T-*.md.

pub mod conf;

use std::ffi::OsString;
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

    /// The environment every tmux invocation runs with. The server inherits
    /// it on first launch — scrubbed (D29, spike T-2): the allowlist is a
    /// floor rather than the whole story, because the daemon hands the
    /// user's captured shell environment to each pane through `mesimon exec`,
    /// and only PATH has to be here, since only PATH is read off the client
    /// (see `set_path`). One list, because on macOS the server is started by
    /// `posix_spawn` (`start_server`) and everywhere else by `Command`, and
    /// the two must not disagree.
    fn env_pairs(&self) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        let mut set = |k: &str, v: OsString| {
            env.retain(|(name, _)| name != k);
            env.push((OsString::from(k), v));
        };
        for k in ENV_ALLOWLIST {
            if let Some(v) = std::env::var_os(k) {
                set(k, v);
            }
        }
        // Unit tests own this socket directory. Do not read personal shell
        // startup files; integration tests get the same isolation from their
        // daemon subprocess environment. Production behavior is unchanged.
        #[cfg(test)]
        {
            set("HOME", self.sock.parent().expect("test socket directory").into());
            set("SHELL", "/bin/sh".into());
        }
        if let Some(path) = &self.path {
            set("PATH", path.into());
        }
        set("TERM", "xterm-256color".into());
        env
    }

    fn tmux(&self) -> Command {
        let mut c = Command::new(tmux_bin());
        c.arg("-S").arg(&self.sock).arg("-f").arg(&self.conf);
        c.env_clear();
        c.envs(self.env_pairs());
        c
    }

    /// Start the private server before the first pane, on macOS, as a
    /// process responsible for itself (T-690). Everywhere else the first
    /// `new-session` forks one, as it always did.
    ///
    /// macOS attributes a process's access to Documents, Desktop and
    /// Downloads to its *responsible process* — the app that launched the
    /// tree, inherited at fork. A server tmux forks for itself inherits the
    /// daemon's, which is the terminal app that opened the board, and once
    /// that app quits every pane under the server points at a dead pid. The
    /// kernel keeps a cached allow for a while (measured: a week), then every
    /// `git` and `claude` in a pane reads `Operation not permitted` on the
    /// checkout, agents die at launch, and nothing in the panes says why.
    ///
    /// `posix_spawn` with `responsibility_spawnattrs_setdisclaim` makes the
    /// spawned process responsible for itself, and `tmux -D` runs the server
    /// in that very process instead of forking one and exiting. Every pane
    /// and `run-shell` under it is then attributed to the server, alive as
    /// long as the panes are, and TCC's identity for the prompt and the
    /// stored grant is the tmux binary: macOS asks once, "`mesimon-tmux`
    /// would like to access files in your Documents folder", and keeps the
    /// answer by path and code hash (an ad-hoc signed binary asks again when
    /// its bytes change; a platform binary such as `/bin/ls` is refused with
    /// no prompt at all — measured 2026-10-07, STALE-MAP T-690).
    ///
    /// `-D` also turns `exit-empty` off, so this server stays up with no
    /// session where the forked one exited with its last; `kill_server`
    /// ends it, and a daemon that stops leaves it running as before.
    ///
    /// Best effort: a tmux without `-D` (before 3.2) or a spawn failure is
    /// said on stderr (the daemon's journal) and the pane's `new-session`
    /// starts the server the old way.
    fn ensure_server(&self) {
        #[cfg(target_os = "macos")]
        if !self.server_alive() {
            if let Err(e) = self.start_server() {
                eprintln!(
                    "mesimon: the private tmux server did not start detached ({e:#}); \
                     starting it with the first pane"
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn start_server(&self) -> Result<()> {
        let argv: Vec<OsString> = vec![
            tmux_bin().into_os_string(),
            "-S".into(),
            self.sock.clone().into_os_string(),
            "-f".into(),
            self.conf.clone().into_os_string(),
            "-D".into(),
        ];
        let pid = disclaim::spawn(&argv, &self.env_pairs()).context("posix_spawn of tmux")?;
        // Reap: the server is a child now, where the forked one was launchd's.
        std::thread::spawn(move || unsafe {
            let mut status = 0;
            libc::waitpid(pid, &mut status, 0);
        });
        let deadline = std::time::Instant::now() + SERVER_START_WAIT;
        while std::time::Instant::now() < deadline {
            if self.server_alive() {
                return Ok(());
            }
            // Reaped already: a tmux that refused `-D` or the socket.
            if unsafe { libc::kill(pid, 0) } != 0 {
                bail!("tmux (pid {pid}) exited before answering on the socket");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        bail!("no server on {} after {:?}", self.sock.display(), SERVER_START_WAIT)
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

    /// Whether a server answers on the socket. `list-sessions`, not
    /// `has-session`: the latter wants a current session and fails on a
    /// server with none, which a `-D` server is between its sessions
    /// (`exit-empty` off) — and read as dead, it was started twice (T-690).
    pub fn server_alive(&self) -> bool {
        self.tmux().args(["list-sessions"]).output().map(|o| o.status.success()).unwrap_or(false)
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
        self.ensure_server();
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

    /// Claude Code's send-now (`mesimon_core::road::SEND_NOW_KEYS`, T-601)
    /// into a session's pane: the words in its composer go to the model at
    /// once, a running tool call moved to the background. A SEPARATE call
    /// after the words went in, as `send_enter` is.
    pub fn send_now(&self, sid16: &str) -> Result<()> {
        let [first, second] = mesimon_core::road::SEND_NOW_KEYS;
        self.run(&["send-keys", "-t", sid16, first, second])?;
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
        Ok(self.panes()?.unwrap_or_default())
    }

    /// `snapshot`, keeping the difference between a server that answered
    /// with no pane and no server at all: `None` is the latter. A caller
    /// that reads an empty list as "the pane is gone" needs the first and
    /// must not take the second for it — and a `-D` server lives on between
    /// its sessions (T-690), so an empty answer is now an everyday one.
    pub fn panes(&self) -> Result<Option<Vec<PaneSnapshot>>> {
        if !self.server_alive() {
            return Ok(None);
        }
        let out =
            self.list_panes(&["session_name", "pane_pid", "pane_dead", "pane_dead_status"])?;
        Ok(Some(parse_snapshot(&out)))
    }

    /// `list-panes -a -F <names>` over every session, and nothing — not an
    /// error — on a server with none. A `-D` server lives on between its
    /// sessions (T-690) and answers `list-panes -a` there with `no current
    /// target`, which read as a failure told the Codex cleanup it could not
    /// verify a pane was gone. `list-sessions` tells that server from a dead
    /// one: it answers with nothing on the first and not at all on the second.
    fn list_panes(&self, names: &[&str]) -> Result<String> {
        match self.run(&["list-panes", "-a", "-F", &fields(names)]) {
            Ok(out) => Ok(out),
            Err(e) => match self.tmux().args(["list-sessions"]).output() {
                Ok(o) if o.status.success() && o.stdout.is_empty() => Ok(String::new()),
                _ => Err(e),
            },
        }
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
        let out = self.list_panes(&["session_name", "window_activity"])?;
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
        let out =
            self.list_panes(&["session_name", "pane_dead", "pane_current_command", "pane_title"])?;
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

    /// Whether a pane under this server may read `dir` — see [`folder_access`].
    pub fn folder_access(&self, dir: &Path) -> Access {
        folder_access(&self.sock, dir)
    }
}

/// How long `start_server` waits for the server it spawned to answer.
#[cfg(target_os = "macos")]
const SERVER_START_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// `posix_spawn` with the responsibility disclaimed (macOS, T-690): the
/// child is its own responsible process, so macOS keys its folder access
/// to the child's binary and its own lifetime, not to whatever terminal app
/// the daemon descends from.
#[cfg(target_os = "macos")]
mod disclaim {
    use std::ffi::{CString, OsString};
    use std::os::unix::ffi::OsStrExt;

    use anyhow::{bail, Result};
    use libc::{c_char, c_int, c_short, pid_t, posix_spawn_file_actions_t, posix_spawnattr_t};

    extern "C" {
        /// libSystem, private but stable since 10.14: Chromium, Emacs and
        /// Alacritty each spawn with it, for the same reason.
        fn responsibility_spawnattrs_setdisclaim(
            attr: *mut posix_spawnattr_t,
            disclaim: c_int,
        ) -> c_int;
        /// `<spawn.h>`: the one descriptor `POSIX_SPAWN_CLOEXEC_DEFAULT`
        /// leaves open. Not in the libc crate.
        fn posix_spawn_file_actions_addinherit_np(
            actions: *mut posix_spawn_file_actions_t,
            fd: c_int,
        ) -> c_int;
        /// `<spawn.h>` since 10.15. Not in the libc crate.
        fn posix_spawn_file_actions_addchdir_np(
            actions: *mut posix_spawn_file_actions_t,
            path: *const c_char,
        ) -> c_int;
    }

    /// `<spawn.h>`, not in the libc crate: the child is a session leader,
    /// as `daemon()` would have made the forked server.
    const POSIX_SPAWN_SETSID: c_short = 0x0400;

    /// Spawn `argv` with `env` as its whole environment, in `/`: stdin and
    /// stdout on `/dev/null`, stderr the caller's own (the daemon's journal,
    /// so a server that refuses its conf is heard), every other descriptor
    /// closed. `/` because the child is now what macOS asks about: a server
    /// started in the checkout reads its own cwd at startup, and a checkout
    /// under Documents would raise the prompt before any pane needed the
    /// folder. Every pane names its directory (`new-session -c`), so the
    /// server's own is read by nothing.
    pub fn spawn(argv: &[OsString], env: &[(OsString, OsString)]) -> Result<pid_t> {
        let c_argv: Vec<CString> = argv
            .iter()
            .map(|a| CString::new(a.as_bytes()))
            .collect::<std::result::Result<_, _>>()?;
        let c_env: Vec<CString> = env
            .iter()
            .map(|(k, v)| {
                let mut kv = k.as_bytes().to_vec();
                kv.push(b'=');
                kv.extend_from_slice(v.as_bytes());
                CString::new(kv)
            })
            .collect::<std::result::Result<_, _>>()?;
        let mut argv_p: Vec<*mut c_char> = c_argv.iter().map(|s| s.as_ptr().cast_mut()).collect();
        argv_p.push(std::ptr::null_mut());
        let mut env_p: Vec<*mut c_char> = c_env.iter().map(|s| s.as_ptr().cast_mut()).collect();
        env_p.push(std::ptr::null_mut());
        let devnull = CString::new("/dev/null")?;
        let root = CString::new("/")?;
        // SAFETY: every pointer handed to libc outlives the calls (the
        // CStrings and the null-terminated arrays live to the end of this
        // function), the attr and the file actions are initialised before
        // use and destroyed after, and nothing is read back from the child.
        unsafe {
            let mut attr: posix_spawnattr_t = std::mem::zeroed();
            check(libc::posix_spawnattr_init(&mut attr), "posix_spawnattr_init")?;
            let mut actions: posix_spawn_file_actions_t = std::mem::zeroed();
            if let Err(e) = check(
                libc::posix_spawn_file_actions_init(&mut actions),
                "posix_spawn_file_actions_init",
            ) {
                libc::posix_spawnattr_destroy(&mut attr);
                return Err(e);
            }
            let spawned = (|| {
                check(
                    responsibility_spawnattrs_setdisclaim(&mut attr, 1),
                    "responsibility_spawnattrs_setdisclaim",
                )?;
                let flags = POSIX_SPAWN_SETSID | libc::POSIX_SPAWN_CLOEXEC_DEFAULT as c_short;
                check(
                    libc::posix_spawnattr_setflags(&mut attr, flags),
                    "posix_spawnattr_setflags",
                )?;
                check(
                    libc::posix_spawn_file_actions_addopen(
                        &mut actions,
                        0,
                        devnull.as_ptr(),
                        libc::O_RDONLY,
                        0,
                    ),
                    "stdin on /dev/null",
                )?;
                check(
                    libc::posix_spawn_file_actions_addopen(
                        &mut actions,
                        1,
                        devnull.as_ptr(),
                        libc::O_WRONLY,
                        0,
                    ),
                    "stdout on /dev/null",
                )?;
                check(posix_spawn_file_actions_addinherit_np(&mut actions, 2), "stderr inherited")?;
                check(
                    posix_spawn_file_actions_addchdir_np(&mut actions, root.as_ptr()),
                    "chdir /",
                )?;
                let mut pid: pid_t = 0;
                check(
                    libc::posix_spawn(
                        &mut pid,
                        argv_p[0],
                        &actions,
                        &attr,
                        argv_p.as_ptr(),
                        env_p.as_ptr(),
                    ),
                    "posix_spawn",
                )?;
                Ok(pid)
            })();
            libc::posix_spawn_file_actions_destroy(&mut actions);
            libc::posix_spawnattr_destroy(&mut attr);
            spawned
        }
    }

    fn check(rc: c_int, what: &str) -> Result<()> {
        if rc == 0 {
            Ok(())
        } else {
            bail!("{what}: {}", std::io::Error::from_raw_os_error(rc))
        }
    }
}

/// What a process under the private server may read (T-690).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// No server on the socket: nothing is cut off, the next spawn starts one.
    NoServer,
    Readable,
    /// `ls` under the server refused the directory, in the system's words
    /// (`Operation not permitted`).
    Denied(String),
}

/// Whether a process under the private server on `sock` may list `dir`.
///
/// `ls` through `run-shell`, which runs under the server and so under the
/// server's responsible process, exactly as a pane does: the one way to see
/// macOS's cut-off from outside a pane. `doctor` prints it and the daemon
/// asks it when a spawn dies at launch (T-690).
pub fn folder_access(sock: &Path, dir: &Path) -> Access {
    let client = || {
        let mut c = Command::new(tmux_bin());
        c.arg("-S").arg(sock);
        c
    };
    // `list-sessions`, as `server_alive` — an empty server answers it.
    if !client().arg("list-sessions").output().is_ok_and(|o| o.status.success()) {
        return Access::NoServer;
    }
    // The path rides the server's environment, never the command: argv in,
    // `"$MESIMON_PROBE_DIR"` out, and the command is a constant. `run-shell`
    // runs under the server's `default-shell` — the person's own shell, by
    // tmux's default — so no quoting written for `sh` is a promise here.
    let set = client().args(["set-environment", "-g", PROBE_DIR_VAR]).arg(dir).output();
    if !set.is_ok_and(|o| o.status.success()) {
        return Access::Denied("tmux set-environment failed".into());
    }
    let out = client().args(["run-shell", PROBE_CMD]).output();
    let _ = client().args(["set-environment", "-gu", PROBE_DIR_VAR]).output();
    match out {
        Ok(out) => {
            match parse_access(out.status.success(), &String::from_utf8_lossy(&out.stdout)) {
                Ok(()) => Access::Readable,
                Err(why) => Access::Denied(why),
            }
        }
        Err(e) => Access::Denied(format!("tmux run-shell: {e}")),
    }
}

/// The pid of the server on `sock`, if one answers.
pub fn server_pid(sock: &Path) -> Option<u32> {
    let out = Command::new(tmux_bin())
        .arg("-S")
        .arg(sock)
        .args(["display-message", "-p", "#{pid}"])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().parse().ok())?
}

/// `run-shell`'s exit status and output into the probe's answer: the
/// system's words after `ls: <dir>: `, without tmux's own `'…' returned 1`
/// line, which `run-shell` appends to a failing command.
fn parse_access(ok: bool, out: &str) -> Result<(), String> {
    if ok {
        return Ok(());
    }
    let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let why =
        first.strip_prefix("ls: ").and_then(|r| r.rsplit_once(": ")).map_or(first, |(_, w)| w);
    Err(if why.is_empty() { "ls failed".to_string() } else { why.to_string() })
}

/// The server's environment variable the probe's directory rides in: set
/// with `set-environment -g` (a tmux argv, no shell), read by the shell as
/// `"$MESIMON_PROBE_DIR"`, unset after. The checkout's path — which a
/// person may have named anything — is never parsed as shell.
const PROBE_DIR_VAR: &str = "MESIMON_PROBE_DIR";
/// stderr onto stdout, stdout away: the output is the error or nothing.
const PROBE_CMD: &str = "ls \"$MESIMON_PROBE_DIR\" 2>&1 >/dev/null";

/// macOS's word for the cut-off: `ls` under the server got EPERM
/// (`Operation not permitted`) on the directory — TCC's refusal, where a
/// plain permissions problem is EACCES (`Permission denied`) and a shell
/// that is not POSIX fails the probe in its own words. Only this raises the
/// board's notice and doctor's macOS advice; any other refusal is said as
/// what it is.
pub fn is_cut_off(why: &str) -> bool {
    why == "Operation not permitted"
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
    fn the_access_probe_reads_the_system_words_off_run_shell() {
        assert_eq!(parse_access(true, ""), Ok(()));
        assert_eq!(
            parse_access(
                false,
                "ls: /Users/me/Documents/code/app: Operation not permitted\n'ls …' returned 1\n"
            ),
            Err("Operation not permitted".to_string())
        );
        // A path with a colon in it: the LAST `: ` is the one before the words.
        assert_eq!(
            parse_access(false, "ls: /Users/me/Documents/a: b: No such file or directory\n"),
            Err("No such file or directory".to_string())
        );
        assert_eq!(parse_access(false, "\n"), Err("ls failed".to_string()));
        assert!(is_cut_off("Operation not permitted"));
        assert!(!is_cut_off("Permission denied"));
        assert!(!is_cut_off("No such file or directory"));
        // No byte of the directory reaches the command: it is a constant,
        // and the path rides the server's environment.
        assert!(!PROBE_CMD.contains('\''), "nothing to quote in a constant");
    }

    /// The mechanism T-690 rests on, asserted where it can be: the server a
    /// spawn starts on macOS is its own responsible process, so the folder
    /// access macOS grants it outlives the terminal that opened the board.
    /// TCC itself cannot be simulated; the manual check is in STALE-MAP.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_private_server_answers_to_macos_for_itself() {
        extern "C" {
            fn responsibility_get_pid_responsible_for_pid(pid: libc::pid_t) -> libc::pid_t;
        }
        let f = fixture("backend-responsible", "t.sock");
        let be = TmuxBackend::new(f.dir.join("t.sock"), &f.dir, None).unwrap();
        be.spawn("resp", &PathBuf::from("/tmp"), &["sleep".into(), "60".into()]).unwrap();
        let pid = server_pid(&f.dir.join("t.sock")).expect("a server answers on the socket");
        // SAFETY: a pid in, a pid out; libSystem reads nothing of ours.
        let responsible = unsafe { responsibility_get_pid_responsible_for_pid(pid as libc::pid_t) };
        assert_eq!(
            responsible, pid as libc::pid_t,
            "the server must be responsible for itself, not for the terminal this test runs in"
        );
        // And the probe reads through it: the fixture dir is nobody's
        // protected folder — and a directory named to break a shell is
        // read all the same, since its path never meets one. The variable
        // is gone afterwards.
        assert_eq!(be.folder_access(&f.dir), Access::Readable);
        let odd = f.dir.join("it's $(odd) `here`; \"x\"");
        std::fs::create_dir_all(&odd).unwrap();
        assert_eq!(be.folder_access(&odd), Access::Readable);
        assert_eq!(
            be.folder_access(&f.dir.join("absent")),
            Access::Denied("No such file or directory".into())
        );
        assert!(
            be.run(&["show-environment", "-g", PROBE_DIR_VAR]).is_err(),
            "unset after the probe"
        );
        // `-D` keeps the server up with no session, where the forked one
        // exited with its last; a wake into an empty server is then a plain
        // `new-session`, with no `%0` reuse.
        be.kill_session("resp").unwrap();
        assert!(be.server_alive(), "a -D server outlives its last session");
        // And an empty server reads as empty, never as a failure: the
        // Codex cleanup's "cannot verify private pane absence" was this.
        assert_eq!(be.snapshot().unwrap(), Vec::new());
        assert_eq!(be.panes().unwrap(), Some(Vec::new()), "answered, with no pane");
        assert_eq!(be.pane_facts().unwrap(), Vec::new());
        assert_eq!(be.activity().unwrap(), Vec::new());
        be.kill_server().unwrap();
        assert_eq!(folder_access(&f.dir.join("t.sock"), &f.dir), Access::NoServer);
        assert_eq!(be.panes().unwrap(), None, "no server answered");
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
