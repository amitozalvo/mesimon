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
}

impl Fixture {
    pub fn new(name: &str, tmux: &str) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/test_guard.py");
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
        Self { dir, control: Arc::new(Mutex::new(control)), finished: false }
    }

    pub fn register(&self, repo: &Path, state: Option<&Path>, runtime: Option<&Path>, sock: &Path) {
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
    /// used to. Long logs keep their tail.
    fn echo_child_logs(&self) {
        const KEEP: usize = 200;
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
            let body = std::fs::read_to_string(&log).unwrap_or_default();
            let lines: Vec<&str> = body.lines().collect();
            let skip = lines.len().saturating_sub(KEEP);
            let cut = if skip > 0 { format!(", last {KEEP}") } else { String::new() };
            eprintln!("--- {} ({} lines{cut}) ---", log.display(), lines.len());
            for line in &lines[skip..] {
                eprintln!("{line}");
            }
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
