//! `costs.json` (T-327): the tokens each ticket's agents have spent, read off
//! their transcripts as they grow.
//!
//! A ticket's account outlives its sessions: a record forgets a conversation
//! at `/clear` and goes when the ticket's agent is replaced, so the ledger
//! keeps every transcript a ticket's sessions ever pointed at (`learn`), with
//! how far it has been read, and the tokens by hour and model. The hour is
//! what lets the Usage dialog say the last 24 hours, 7 days and 30 days
//! without a time zone. Claude's subagent transcripts
//! (`<session>/subagents/agent-*.jsonl`) are found beside the session's own
//! and counted to the same ticket. What a ticket held before this file
//! existed is counted from the transcripts its live records still name.
//!
//! The reading runs on a worker (`scan`): a transcript is megabytes, and the
//! writer only folds the counts back in. The file follows the other state
//! files' contract: its own `schema_version`, a newer build's bytes left
//! untouched with writes barred, an unparseable file quarantined.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use mesimon_core::command::Notice;
use mesimon_core::cost::{claude_turn, codex_line, delta, CodexLine, TicketCost, Tokens};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

pub const COSTS_SCHEMA: u32 = 1;

/// The most a pass reads of one transcript; the rest waits for the next.
const READ_CAP: u64 = 32 << 20;
/// Message ids remembered per conversation: a message's lines are adjacent.
const RECENT: usize = 64;

/// How far one transcript has been read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cursor {
    /// A Codex rollout, rather than a Claude transcript.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub codex: bool,
    #[serde(default)]
    pub offset: u64,
    /// The last Claude message ids counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<String>,
    /// Codex: the running totals last read, and the model the turns run on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TicketLedger {
    /// Every transcript the ticket's agents held, by path.
    #[serde(default)]
    pub conversations: BTreeMap<String, Cursor>,
    /// Tokens by unix hour, then model key.
    #[serde(default)]
    pub hours: BTreeMap<u64, BTreeMap<String, Tokens>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub tickets: BTreeMap<ulid::Ulid, TicketLedger>,
}

/// One transcript to read on the worker.
#[derive(Debug, Clone)]
pub struct Job {
    pub ticket: ulid::Ulid,
    pub path: String,
    pub cursor: Cursor,
}

/// What a read found: the cursor after it and the tokens it added.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub ticket: ulid::Ulid,
    pub path: String,
    pub cursor: Cursor,
    pub added: Vec<(u64, String, Tokens)>,
}

impl Ledger {
    /// A transcript a ticket's session points at. True when it is new.
    pub fn learn(&mut self, ticket: ulid::Ulid, path: &str, codex: bool) -> bool {
        let t = self.tickets.entry(ticket).or_default();
        if t.conversations.contains_key(path) {
            return false;
        }
        t.conversations.insert(path.to_string(), Cursor { codex, ..Cursor::default() });
        true
    }

    /// Every known transcript, for the worker to read what grew.
    pub fn jobs(&self) -> Vec<Job> {
        self.tickets
            .iter()
            .flat_map(|(ticket, t)| {
                t.conversations.iter().map(|(path, cursor)| Job {
                    ticket: *ticket,
                    path: path.clone(),
                    cursor: cursor.clone(),
                })
            })
            .collect()
    }

    /// Fold a pass back in. True when anything moved.
    pub fn apply(&mut self, done: Vec<Done>) -> bool {
        let mut moved = false;
        for d in done {
            let t = self.tickets.entry(d.ticket).or_default();
            if t.conversations.get(&d.path) != Some(&d.cursor) {
                t.conversations.insert(d.path, d.cursor);
                moved = true;
            }
            for (hour, model, tokens) in d.added {
                if tokens.is_empty() {
                    continue;
                }
                t.hours.entry(hour).or_default().entry(model).or_default().add(&tokens);
                moved = true;
            }
        }
        moved
    }

    /// Drop the tickets the board no longer has. True when any went.
    pub fn keep(&mut self, live: &HashSet<ulid::Ulid>) -> bool {
        let before = self.tickets.len();
        self.tickets.retain(|id, _| live.contains(id));
        self.tickets.len() != before
    }

    /// Each ticket's account as of `now_ms`, for the snapshot.
    pub fn view(&self, now_ms: u64) -> Vec<TicketCost> {
        self.tickets
            .iter()
            .filter_map(|(ticket, t)| {
                let mut c = TicketCost { ticket: *ticket, ..TicketCost::default() };
                for (hour, models) in &t.hours {
                    for (model, tokens) in models {
                        c.fold(model, tokens, *hour, now_ms);
                    }
                }
                (c.tokens > 0).then_some(c)
            })
            .collect()
    }
}

/// Claude's subagent transcripts for a session transcript: `<session>.jsonl`
/// keeps them in `<session>/subagents/`.
fn subagents(path: &str) -> Vec<String> {
    if path.contains("/subagents/") {
        return Vec::new();
    }
    let Some(stem) = path.strip_suffix(".jsonl") else { return Vec::new() };
    let Ok(dir) = std::fs::read_dir(Path::new(stem).join("subagents")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| p.display().to_string())
        .collect();
    out.sort();
    out
}

/// Read what grew in every transcript, and the subagent transcripts beside
/// them. Runs on a worker; `now_ms` dates a line that carries no stamp.
pub fn scan(jobs: Vec<Job>, now_ms: u64) -> Vec<Done> {
    let known: HashSet<String> = jobs.iter().map(|j| j.path.clone()).collect();
    let mut all = jobs;
    let found: Vec<Job> = all
        .iter()
        .filter(|j| !j.cursor.codex)
        .flat_map(|j| {
            subagents(&j.path).into_iter().map(|path| Job {
                ticket: j.ticket,
                path,
                cursor: Cursor::default(),
            })
        })
        .filter(|j| !known.contains(&j.path))
        .collect();
    all.extend(found);
    all.into_iter().filter_map(|j| read(j, now_ms)).collect()
}

/// One transcript from its cursor to the last whole line, or `None` when it
/// has not grown. A file shorter than the cursor was rewritten: it is read
/// again from the start.
fn read(job: Job, now_ms: u64) -> Option<Done> {
    let len = std::fs::metadata(&job.path).ok()?.len();
    let mut cursor = job.cursor;
    let new = cursor.offset == 0 && cursor.recent.is_empty() && cursor.totals.is_none();
    if len == cursor.offset && !new {
        return None;
    }
    if len < cursor.offset {
        cursor = Cursor { codex: cursor.codex, ..Cursor::default() };
    }
    let mut file = std::fs::File::open(&job.path).ok()?;
    file.seek(SeekFrom::Start(cursor.offset)).ok()?;
    let mut buf = Vec::new();
    file.take(READ_CAP.min(len - cursor.offset)).read_to_end(&mut buf).ok()?;
    let Some(end) = buf.iter().rposition(|b| *b == b'\n') else {
        // Nothing whole yet; a new transcript is still worth recording.
        return new.then_some(Done {
            ticket: job.ticket,
            path: job.path,
            cursor,
            added: Vec::new(),
        });
    };
    let text = String::from_utf8_lossy(&buf[..end]);
    let mut added = Vec::new();
    for line in text.lines() {
        if cursor.codex {
            match codex_line(line) {
                Some(CodexLine::Model(m)) => cursor.model = Some(m),
                Some(CodexLine::Totals { at_ms, tokens }) => {
                    let grew = delta(&cursor.totals.unwrap_or_default(), &tokens);
                    cursor.totals = Some(tokens);
                    let model = cursor.model.clone().unwrap_or_else(|| "codex".into());
                    added.push((hour_of(at_ms, now_ms), model, grew));
                }
                None => {}
            }
        } else if let Some(turn) = claude_turn(line) {
            if cursor.recent.contains(&turn.id) {
                continue;
            }
            cursor.recent.push(turn.id);
            if cursor.recent.len() > RECENT {
                cursor.recent.remove(0);
            }
            added.push((hour_of(turn.at_ms, now_ms), turn.model, turn.tokens));
        }
    }
    cursor.offset += end as u64 + 1;
    Some(Done { ticket: job.ticket, path: job.path, cursor, added })
}

fn hour_of(at_ms: u64, now_ms: u64) -> u64 {
    (if at_ms == 0 { now_ms } else { at_ms }) / 3_600_000
}

/// `Err(Some(v))` is a file from a NEWER mesimon; `Err(None)` is unparseable.
fn parse(text: &str) -> std::result::Result<Ledger, (Option<u32>, String)> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| (None, e.to_string()))?;
    let found = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(1) as u32;
    if found > COSTS_SCHEMA {
        return Err((Some(found), format!("schema {found}")));
    }
    serde_json::from_value::<Ledger>(v).map_err(|e| (None, e.to_string()))
}

/// Startup loader: the ledger, any notices, and whether writes are barred.
pub fn load_or_recover(paths: &Paths) -> (Ledger, Vec<Notice>, bool) {
    let f = paths.costs_file();
    let mut notices = Vec::new();
    if !f.is_file() {
        return (Ledger::default(), notices, false);
    }
    let text = match std::fs::read_to_string(&f) {
        Ok(t) => t,
        Err(e) => {
            notices.push(
                Notice::new(
                    "quarantined",
                    "the tickets' costs could not be opened — not written to",
                )
                .with_path(f.display())
                .with_detail(e.to_string()),
            );
            return (Ledger::default(), notices, true);
        }
    };
    let detail = match parse(&text) {
        Ok(ledger) => return (ledger, notices, false),
        Err((Some(found), _)) => {
            notices.push(
                Notice::new(
                    "future_version",
                    format!(
                        "costs.json was written by a newer mesimon (schema {found}, this build \
                         reads {COSTS_SCHEMA}) — left untouched and not written to"
                    ),
                )
                .with_path(f.display()),
            );
            return (Ledger::default(), notices, true);
        }
        Err((None, detail)) => detail,
    };
    let moved = crate::store::quarantine(&f);
    notices.push(
        Notice::new(
            "quarantined",
            match &moved {
                Some(dest) => format!(
                    "the tickets' costs could not be read — the file was set aside as {}, and \
                     counting starts again",
                    dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
                None => "the tickets' costs could not be read — not written to".to_string(),
            },
        )
        .with_path(f.display())
        .with_detail(detail),
    );
    (Ledger::default(), notices, moved.is_none())
}

pub fn save(paths: &Paths, ledger: &Ledger) -> Result<()> {
    let mut body = ledger.clone();
    body.schema_version = COSTS_SCHEMA;
    crate::store::write_atomic(
        &paths.costs_file(),
        &serde_json::to_string(&body)?,
        crate::store::PRIVATE,
    )
}

/// Where a session's transcript is, if it is one this ledger reads.
pub fn transcript_of(record: &mesimon_core::board::SessionRecord) -> Option<(PathBuf, bool)> {
    use mesimon_core::board::SessionKind;
    let path = PathBuf::from(record.transcript_path.as_ref()?);
    match record.kind {
        SessionKind::Claude => Some((path, false)),
        SessionKind::Codex => Some((path, true)),
        SessionKind::Bash => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_870_640_000;

    fn line(id: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"r","timestamp":"2026-10-01T16:04:00.000Z","message":{{"id":"{id}","model":"claude-opus-5-5","usage":{{"input_tokens":1,"output_tokens":{out}}}}}}}"#
        )
    }

    #[test]
    fn a_transcript_is_read_once_a_message_and_only_what_grew() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let t = ulid::Ulid::new();
        // Two lines of one message (two content blocks), one user line, and a
        // half-written line the pass must leave for the next.
        let body = format!(
            "{}\n{}\n{{\"type\":\"user\"}}\n{}\n{{\"type\":\"assis",
            line("m1", 100),
            line("m1", 100),
            line("m2", 50)
        );
        std::fs::write(&path, &body).unwrap();
        let mut ledger = Ledger::default();
        assert!(ledger.learn(t, &path.display().to_string(), false));
        assert!(!ledger.learn(t, &path.display().to_string(), false));
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        let c = &ledger.view(NOW)[0];
        assert_eq!(c.tokens, 152, "m1 once, m2 once");
        assert!((c.usd - (2.0 * 4.0 + 150.0 * 20.0) / 1e6).abs() < 1e-12);
        // Nothing grew: nothing read.
        assert!(scan(ledger.jobs(), NOW).is_empty());
        // The half line completes, and a third message lands.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        writeln!(f, "tant\"}}").unwrap();
        writeln!(f, "{}", line("m3", 10)).unwrap();
        assert!(ledger.apply(scan(ledger.jobs(), NOW)));
        assert_eq!(ledger.view(NOW)[0].tokens, 163);
    }

    #[test]
    fn subagents_count_to_the_ticket_and_codex_counts_growth() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("abc.jsonl");
        std::fs::write(&main, format!("{}\n", line("m1", 10))).unwrap();
        std::fs::create_dir_all(dir.path().join("abc/subagents")).unwrap();
        std::fs::write(
            dir.path().join("abc/subagents/agent-1.jsonl"),
            format!("{}\n", line("s1", 20)),
        )
        .unwrap();
        let rollout = dir.path().join("rollout.jsonl");
        std::fs::write(
            &rollout,
            concat!(
                r#"{"timestamp":"2026-10-01T16:00:00Z","type":"turn_context","payload":{"model":"gpt-6-astra"}}"#,
                "\n",
                r#"{"timestamp":"2026-10-01T16:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":60,"output_tokens":10}}}}"#,
                "\n",
                r#"{"timestamp":"2026-10-01T16:02:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":90,"output_tokens":20}}}}"#,
                "\n"
            ),
        )
        .unwrap();
        let (a, b) = (ulid::Ulid::new(), ulid::Ulid::new());
        let mut ledger = Ledger::default();
        ledger.learn(a, &main.display().to_string(), false);
        ledger.learn(b, &rollout.display().to_string(), true);
        ledger.apply(scan(ledger.jobs(), NOW));
        let view = ledger.view(NOW);
        let of = |t| view.iter().find(|c| c.ticket == t).unwrap();
        assert_eq!(of(a).tokens, 2 + 30, "the subagent's tokens are the ticket's");
        assert_eq!(ledger.tickets[&a].conversations.len(), 2, "and its transcript is learned");
        assert_eq!((of(b).tokens, of(b).unpriced, of(b).usd), (170, 170, 0.0), "counted, unpriced");
        // The ledger round-trips, and a board without the ticket drops it.
        let paths = {
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let mut p = Paths::for_repo(&repo).unwrap();
            p.state_dir = dir.path().join("state");
            std::fs::create_dir_all(&p.state_dir).unwrap();
            p
        };
        save(&paths, &ledger).unwrap();
        let (back, notices, barred) = load_or_recover(&paths);
        assert!(notices.is_empty() && !barred);
        assert_eq!(back.tickets, ledger.tickets);
        let mut kept = back;
        assert!(kept.keep(&[a].into_iter().collect()));
        assert_eq!(kept.tickets.len(), 1);
        // A newer file is left alone and barred; a broken one is set aside.
        std::fs::write(paths.costs_file(), r#"{"schema_version":9}"#).unwrap();
        let (_, notices, barred) = load_or_recover(&paths);
        assert!(barred && notices[0].text.contains("newer mesimon"));
        std::fs::write(paths.costs_file(), "{not json").unwrap();
        let (fresh, notices, barred) = load_or_recover(&paths);
        assert!(!barred && fresh.tickets.is_empty() && notices[0].text.contains("set aside"));
    }
}
