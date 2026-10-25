//! Shared test-only supervisor client; included by process-owning test crates.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Control {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}

impl Control {
    fn receive(&mut self) -> Value {
        let mut line = String::new();
        self.output.read_line(&mut line).expect("read fixture supervisor");
        let reply: Value =
            serde_json::from_str(&line).expect("fixture supervisor exited or timed out");
        assert!(reply.get("error").is_none(), "fixture supervisor: {reply}");
        reply["ok"].clone()
    }

    fn request(&mut self, value: Value) -> Value {
        writeln!(self.input.as_mut().expect("open control pipe"), "{value}")
            .expect("write fixture supervisor");
        self.receive()
    }

    fn finish(&mut self) -> Result<(), String> {
        if let Some(mut input) = self.input.take() {
            let _ = writeln!(input, "{}", json!({"op": "finish"}));
            drop(input);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("fixture cleanup failed: {status}")),
                Err(e) => return Err(e.to_string()),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => {
                    return Err(
                        "fixture supervisor exceeded cleanup deadline; manifest retained".into()
                    )
                }
            }
        }
    }
}

pub struct Fixture {
    pub dir: PathBuf,
    control: Arc<Mutex<Control>>,
    finished: bool,
    tmux: String,
    socks: Mutex<Vec<PathBuf>>,
}

impl Fixture {
    pub fn new(name: &str, tmux: &str) -> Self {
        // This repository, or — compiled into the relay's tests from the
        // sibling checkout — the one its `.cargo/config.toml` names.
        let core = option_env!("MESIMON_CORE_DIR")
            .unwrap_or(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let script = Path::new(core).join("ci/test_guard.py");
        let mut child = Command::new("python3")
            .arg("-u")
            .arg(script)
            .args(["--name", name, "--tmux", tmux])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("python3 is required for process-owning tests");
        let mut control = Control {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        };
        let dir = PathBuf::from(control.receive().as_str().expect("fixture root"));
        Self {
            dir,
            control: Arc::new(Mutex::new(control)),
            finished: false,
            tmux: tmux.to_string(),
            socks: Mutex::new(Vec::new()),
        }
    }

    pub fn register(&self, repo: &Path, state: Option<&Path>, runtime: Option<&Path>, sock: &Path) {
        self.socks.lock().unwrap_or_else(|e| e.into_inner()).push(sock.to_path_buf());
        self.control.lock().unwrap().request(json!({
            "op": "register", "repo": repo, "state": state, "runtime": runtime, "sock": sock,
        }));
    }

    pub fn spawn(
        &self,
        argv: Vec<String>,
        env: std::collections::BTreeMap<String, String>,
    ) -> TestProcess {
        let pid = self
            .control
            .lock()
            .unwrap()
            .request(json!({"op": "spawn", "argv": argv, "env": env}))
            .as_u64()
            .expect("child pid") as u32;
        TestProcess { pid, control: self.control.clone() }
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.control.lock().unwrap_or_else(|e| e.into_inner()).finish()
    }
}

impl Fixture {
    /// A supervised child's stdout and stderr go to `child-N.log` under the
    /// fixture root, which the supervisor removes on cleanup. A FAILING test
    /// echoes them here first, so the daemon's side of the story lands in the
    /// captured output beside the assertion, where the in-process daemon's
    /// used to. Long logs keep their tail. The daemon's own journal and feed
    /// (`daemon.log`, `activity.jsonl`, anywhere under the root) follow
    /// them (T-937): a wait that timed out with an empty child log said
    /// nothing of what the daemon did meanwhile.
    fn echo_child_logs(&self) {
        const KEEP: usize = 200;
        let echo = |log: &Path, keep: usize| {
            let body = std::fs::read_to_string(log).unwrap_or_default();
            let lines: Vec<&str> = body.lines().collect();
            let skip = lines.len().saturating_sub(keep);
            let cut = if skip > 0 { format!(", last {keep}") } else { String::new() };
            eprintln!("--- {} ({} lines{cut}) ---", log.display(), lines.len());
            for line in &lines[skip..] {
                eprintln!("{line}");
            }
        };
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut logs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("child-") && n.ends_with(".log"))
            })
            .collect();
        logs.sort();
        for log in logs {
            echo(&log, KEEP);
        }
        let mut daemon_logs = Vec::new();
        let mut dirs = vec![self.dir.clone()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for path in entries.flatten().map(|e| e.path()) {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if path.is_dir() && !path.is_symlink() && name != ".git" {
                    dirs.push(path);
                } else if name == "daemon.log" || name == "activity.jsonl" {
                    daemon_logs.push(path);
                }
            }
        }
        daemon_logs.sort();
        for log in daemon_logs {
            echo(&log, KEEP);
        }
        self.echo_panes();
    }

    /// Every pane of the fixture's tmux servers as the daemon reads it, on a
    /// failing test (T-1019): the screen, the cursor and the modes, so a
    /// wait for words that never went in shows the box the daemon judged.
    fn echo_panes(&self) {
        let socks = self.socks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let tmux = |sock: &Path, args: &[&str]| {
            Command::new(&self.tmux)
                .arg("-S")
                .arg(sock)
                .args(args)
                .stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        };
        for sock in socks.iter().filter(|s| s.exists()) {
            let format = "#{pane_id} #{session_name} dead=#{pane_dead} \
                          cmd=#{pane_current_command} cursor=#{cursor_flag}:#{cursor_x},#{cursor_y} \
                          mode=#{pane_in_mode} size=#{pane_width}x#{pane_height} tty=#{pane_tty}";
            let panes = tmux(sock, &["list-panes", "-a", "-F", format]);
            eprintln!("--- {} panes ({}) ---", sock.display(), tmux(sock, &["-V"]).trim());
            for row in panes.lines() {
                eprintln!("{row}");
                let Some(pane) = row.split(' ').next() else { continue };
                for line in tmux(sock, &["capture-pane", "-p", "-t", pane]).lines() {
                    eprintln!("  |{line}");
                }
                let Some(tty) = row.rsplit_once("tty=").map(|(_, t)| t.trim()) else { continue };
                let tty_name = tty.trim_start_matches("/dev/");
                let ps = Command::new("ps")
                    .args(["-o", "pid,ppid,pgid,stat,wchan,command", "-t", tty_name])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default();
                for line in ps.lines() {
                    eprintln!("  ps {}", line.chars().take(120).collect::<String>());
                }
                let flag = if cfg!(target_os = "macos") { "-f" } else { "-F" };
                let stty = Command::new("stty")
                    .args(["-a", flag, tty])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default();
                for line in stty.lines() {
                    eprintln!("  stty {line}");
                }
            }
        }
        // The files a stub writes beside itself (`got.txt` and its kin).
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut notes: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "txt"))
            .collect();
        notes.sort();
        for note in notes {
            let body = std::fs::read_to_string(&note).unwrap_or_default();
            eprintln!("--- {} ---\n{body:?}", note.display());
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.echo_child_logs();
        }
        if let Err(error) = self.finish() {
            if std::thread::panicking() {
                eprintln!("{error}");
            } else {
                panic!("{error}");
            }
        }
    }
}

pub struct TestProcess {
    pid: u32,
    control: Arc<Mutex<Control>>,
}

impl TestProcess {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&self) -> Result<Option<i32>, String> {
        let value = self
            .control
            .lock()
            .map_err(|e| e.to_string())?
            .request(json!({"op": "poll", "pid": self.pid}));
        Ok(value.as_i64().map(|code| code as i32))
    }

    pub fn join(self) -> Result<(), String> {
        let code = self.wait()?;
        if code == 0 {
            Ok(())
        } else {
            Err(format!("test daemon exited {code}"))
        }
    }

    pub fn wait(&self) -> Result<i32, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(code) = self.try_wait()? {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err("test daemon did not exit in 10 seconds".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
