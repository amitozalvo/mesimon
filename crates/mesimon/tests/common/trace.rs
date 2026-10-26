//! Traces recorded from a real Claude Code (T-1014), replayed into a stub's
//! seat: `tests/traces/<name>.jsonl`, cut by `ci/rig.py --record` from the
//! daemon's `MESIMON_RECORD_TRACE` file. The first line is the header (the
//! rig case, the Claude Code, the road, the model); each line after is a
//! frame the daemon's hook socket took (`hook` by the hook set, `mod` by the
//! mod) or a line the session's transcript gained, `t` milliseconds after
//! the first, in the order the recorder saw them. A path is a placeholder
//! (`{transcript}`, `{cwd}`), the session is `{session}`, and a thinking
//! block keeps no words.
//!
//! A replay sends each frame through the real hook binary as `hook_send_with`
//! does (the mod's twin beside it, so both passes read the same) and appends
//! each transcript line to the bed's transcript, stamped now: the daemon
//! fences a result by its stamp against the held spell. The mod's own
//! reports (`ModUsage`, `ModSubmit`, …) and the pane's death are the board's
//! side of the road, not the engine's, and are not replayed.

use super::*;

pub struct Trace {
    pub name: String,
    pub header: Value,
    pub lines: Vec<Value>,
}

impl Trace {
    pub fn load(name: &str) -> Self {
        let path = format!("{}/tests/traces/{name}.jsonl", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut lines =
            text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap());
        let header: Value = lines.next().expect("a header line");
        assert!(header["claude"].is_string(), "{path}: the header names its Claude Code");
        Self { name: name.into(), header, lines: lines.collect() }
    }

    /// The frames, as `(event, reason, payload)`, in the recorded order.
    pub fn frames(&self) -> Vec<(&str, Option<&str>, &Value)> {
        self.lines
            .iter()
            .filter(|l| replayed(l))
            .map(|l| (l["event"].as_str().unwrap(), l["reason"].as_str(), &l["payload"]))
            .collect()
    }

    /// The events of the replayed frames, in order.
    pub fn events(&self) -> Vec<&str> {
        self.frames().into_iter().map(|(e, _, _)| e).collect()
    }

    /// Sends the first recorded frame `pick` holds for into `sid`'s seat,
    /// as a replay would, and returns it.
    pub fn send(
        &self,
        sock: &Path,
        sid: uuid::Uuid,
        transcript: &Path,
        pick: impl FnMut(&Value) -> bool,
    ) -> Value {
        self.send_with(sock, sid, transcript, pick, |_| {})
    }

    /// `send`, with `patch` applied to the frame's payload first: the
    /// engine's shape, the test's words (a prompt's text).
    pub fn send_with(
        &self,
        sock: &Path,
        sid: uuid::Uuid,
        transcript: &Path,
        mut pick: impl FnMut(&Value) -> bool,
        patch: impl FnOnce(&mut Value),
    ) -> Value {
        let mut line = self
            .lines
            .iter()
            .find(|l| replayed(l) && pick(l))
            .unwrap_or_else(|| panic!("{}: no such frame", self.name))
            .clone();
        patch(&mut line["payload"]);
        self.replay(sock, sid, transcript).play(&line);
        line
    }

    pub fn replay<'a>(&'a self, sock: &Path, sid: uuid::Uuid, transcript: &Path) -> Replay<'a> {
        Replay {
            trace: self,
            at: 0,
            sock: sock.to_path_buf(),
            sid,
            transcript: transcript.to_path_buf(),
            gap_cap: Duration::from_millis(300),
        }
    }
}

/// A frame of the engine's: a hook-set event, by either road.
fn replayed(line: &Value) -> bool {
    matches!(line["kind"].as_str(), Some("hook" | "mod"))
        && line["event"]
            .as_str()
            .is_some_and(|e| !e.starts_with("Mod") && e != "PaneDied" && e != "RemotePermission")
}

/// A transcript line's tool calls and results, by id: `("tool_use", id)` or
/// `("tool_result", id)`.
pub fn blocks(line: &Value) -> Vec<(&str, &str)> {
    line["line"]["message"]["content"]
        .as_array()
        .map(|c| {
            c.iter()
                .filter_map(|b| {
                    let kind = b["type"].as_str()?;
                    let id = match kind {
                        "tool_use" => b["id"].as_str()?,
                        "tool_result" => b["tool_use_id"].as_str()?,
                        _ => return None,
                    };
                    Some((kind, id))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct Replay<'a> {
    trace: &'a Trace,
    at: usize,
    sock: PathBuf,
    sid: uuid::Uuid,
    transcript: PathBuf,
    /// The longest wait between two lines: the recorded gap, up to this.
    pub gap_cap: Duration,
}

impl Replay<'_> {
    /// Plays lines up to the first that `stop` holds for, leaving it
    /// unplayed, and returns it; `None` once the trace is spent.
    pub fn until(&mut self, mut stop: impl FnMut(&Value) -> bool) -> Option<Value> {
        while let Some(line) = self.trace.lines.get(self.at) {
            if stop(line) {
                return Some(line.clone());
            }
            self.play_next();
        }
        None
    }

    /// Plays lines through the first that `stop` holds for, that one too.
    pub fn through(&mut self, stop: impl FnMut(&Value) -> bool) -> Value {
        let line = self.until(stop).unwrap_or_else(|| panic!("{}: no such line", self.trace.name));
        self.play_next();
        line
    }

    pub fn rest(&mut self) {
        while self.at < self.trace.lines.len() {
            self.play_next();
        }
    }

    fn play_next(&mut self) {
        let trace = self.trace;
        let line = &trace.lines[self.at];
        if self.at > 0 {
            let gap = line["t"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(trace.lines[self.at - 1]["t"].as_u64().unwrap_or(0));
            std::thread::sleep(Duration::from_millis(gap).min(self.gap_cap));
        }
        self.at += 1;
        self.play(line);
    }

    fn play(&self, line: &Value) {
        if line["kind"] == "transcript" {
            let mut record = self.fill(&line["line"]);
            if record.get("timestamp").is_some() {
                record["timestamp"] = json!(iso_now());
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.transcript)
                .unwrap();
            writeln!(f, "{record}").unwrap();
        } else if replayed(line) {
            let body = self.fill(&line["payload"]).to_string();
            let event = line["event"].as_str().unwrap();
            hook_send_with(
                &self.sock,
                &self.sid.to_string(),
                event,
                line["reason"].as_str(),
                &body,
            );
        }
    }

    /// `v` with its placeholders for this seat.
    fn fill(&self, v: &Value) -> Value {
        let text = v
            .to_string()
            .replace("{session}", &self.sid.to_string())
            .replace("{transcript}", self.transcript.to_str().unwrap())
            .replace("{sid16}", &self.sid.simple().to_string()[..16]);
        serde_json::from_str(&text).unwrap()
    }
}

/// Now, as Claude Code stamps a transcript record: UTC to the millisecond.
pub fn iso_now() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
        as u64;
    let (secs, milli) = (ms / 1000, ms % 1000);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant's algorithm), UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// A frame of `event`, of `tool`'s call when one is named.
pub fn frame(event: &'static str, tool: Option<&'static str>) -> impl Fn(&Value) -> bool {
    move |l| {
        replayed(l) && l["event"] == event && tool.is_none_or(|t| l["payload"]["tool_name"] == t)
    }
}

/// The transcript line that carries call `id`'s result.
pub fn result_of(id: &str) -> impl Fn(&Value) -> bool + '_ {
    move |l| l["kind"] == "transcript" && blocks(l).contains(&("tool_result", id))
}
