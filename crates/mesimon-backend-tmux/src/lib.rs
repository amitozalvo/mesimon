//! `SessionBackend` over a private tmux server (docs/19-tmux-backend-v01.md).
//! Private socket + `-f` config, never the user's tmux. Every fact this crate
//! relies on was verified in docs/spikes/T-*.md.

pub mod conf;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use mesimon_core::reconcile::PaneSnapshot;

/// D29: the child environment is built from an allowlist, never inherited.
const ENV_ALLOWLIST: &[&str] = &[
    "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "PATH",
];

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

    fn tmux(&self) -> Command {
        let mut c = Command::new("tmux");
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
        self.tmux()
            .args(["has-session"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Spawn a session: tmux session name = sid16, running `argv` in `cwd`.
    /// Extra env vars go through `-e` (per-session, spike T-2).
    pub fn spawn(&self, sid16: &str, cwd: &Path, argv: &[String], env: &[(String, String)]) -> Result<()> {
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
            "tmux".into(),
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
        be.spawn("abc123", &PathBuf::from("/tmp"), &["sleep".into(), "60".into()], &[])
            .unwrap();
        let snap = be.snapshot().unwrap();
        assert!(snap.iter().any(|p| p.session_name == "abc123" && !p.pane_dead));
        // Dead pane preserved by remain-on-exit:
        be.spawn("dead1", &PathBuf::from("/tmp"), &["sh".into(), "-c".into(), "exit 7".into()], &[])
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
