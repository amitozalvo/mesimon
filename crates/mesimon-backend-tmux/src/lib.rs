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

pub fn tmux_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("MESIMON_TMUX_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(sibling) = exe.parent().map(|d| d.join(BUNDLED_TMUX)) {
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    PathBuf::from("tmux")
}

pub struct TmuxBackend {
    sock: PathBuf,
    conf: PathBuf,
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
        std::fs::write(&conf, conf::render(pane_died_cmd))?;
        Ok(Self { sock, conf })
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
                pipe,
            ])?;
        }
        Ok(())
    }

    fn tmux(&self) -> Command {
        let mut c = Command::new(tmux_bin());
        c.arg("-S").arg(&self.sock).arg("-f").arg(&self.conf);
        // The server inherits this env on first launch — scrub it (D29, spike T-2).
        c.env_clear();
        for k in ENV_ALLOWLIST {
            if let Ok(v) = std::env::var(k) {
                c.env(k, v);
            }
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
    /// Extra env vars go through `-e` (per-session, spike T-2).
    pub fn spawn(
        &self,
        sid16: &str,
        cwd: &Path,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            sid16.into(),
            "-c".into(),
            cwd.display().to_string(),
        ];
        for (k, v) in env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        args.extend(argv.iter().cloned());
        let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&argrefs)?;
        Ok(())
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
    /// The caller owns the timing — see `Daemon::deliver_pending_submit`.
    pub fn send_enter(&self, sid16: &str) -> Result<()> {
        self.run(&["send-keys", "-t", sid16, "Enter"])?;
        Ok(())
    }

    /// Deliver a full prompt and submit it (spike T-5 / 19 §6): `load-buffer -`
    /// from stdin → `paste-buffer -p` (bracketed paste) → a SEPARATE
    /// `send-keys Enter`. A single send-keys call truncated 3696→630 bytes and
    /// ate the Enter; `;`-joined tmux commands split — three forks is the shape.
    pub fn paste_text(&self, sid16: &str, text: &str) -> Result<()> {
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
        self.run(&["send-keys", "-t", sid16, "Enter"])?;
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
            "#{session_name}\t#{pane_pid}\t#{pane_dead}\t#{pane_dead_status}",
        ])?;
        let mut v = Vec::new();
        for line in out.lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(pid), Some(dead), status) =
                (f.next(), f.next(), f.next(), f.next())
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
        Ok(v)
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
        let out = self.run(&["list-panes", "-a", "-F", "#{session_name}\t#{window_activity}"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (name, t) = l.split_once('\t')?;
                Some((name.to_string(), t.parse().ok()?))
            })
            .collect())
    }

    /// Every pane's OSC-0 title in one fork (`#{pane_title}`; tmux reports
    /// the hostname when the app never set one — callers filter, same rule
    /// as `pane_title`).
    pub fn titles(&self) -> Result<Vec<(String, String)>> {
        if !self.server_alive() {
            return Ok(Vec::new());
        }
        let out = self.run(&["list-panes", "-a", "-F", "#{session_name}\t#{pane_title}"])?;
        Ok(out
            .lines()
            .filter_map(|l| {
                let (name, t) = l.split_once('\t')?;
                Some((name.to_string(), t.trim().to_string()))
            })
            .collect())
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
        self.run(&["set-option", "-g", "status-right", " Ctrl+]/^5 back  "])?;
        // Live servers also predate the C-5 bind (extended-keys makes Ctrl+5 a
        // distinct key, so the C-] bind alone doesn't catch it).
        self.run(&["bind-key", "-T", "root", "C-5", "detach-client"])?;
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
        let out = self.run(&["capture-pane", "-p", "-J", "-t", sid16])?;
        Ok(out
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(lines)
            .map(str::to_string)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect())
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
mod tests {
    use super::*;

    fn shortdir() -> PathBuf {
        let d = PathBuf::from(format!("/tmp/msmn-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn spawn_snapshot_kill_roundtrip() {
        if Command::new("tmux").arg("-V").output().is_err() {
            eprintln!("tmux not installed; skipping");
            return;
        }
        let dir = shortdir();
        let be = TmuxBackend::new(dir.join("t.sock"), &dir, None).unwrap();
        be.spawn("abc123", &PathBuf::from("/tmp"), &["sleep".into(), "60".into()], &[]).unwrap();
        let snap = be.snapshot().unwrap();
        assert!(snap.iter().any(|p| p.session_name == "abc123" && !p.pane_dead));
        // Dead pane preserved by remain-on-exit:
        be.spawn(
            "dead1",
            &PathBuf::from("/tmp"),
            &["sh".into(), "-c".into(), "exit 7".into()],
            &[],
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let snap = be.snapshot().unwrap();
        let d = snap.iter().find(|p| p.session_name == "dead1").unwrap();
        assert!(d.pane_dead);
        assert_eq!(d.dead_status, Some(7));
        be.kill_server().unwrap();
        std::fs::remove_dir_all(dir).ok();
    }
}
