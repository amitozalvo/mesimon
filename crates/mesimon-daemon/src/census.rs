//! External-session census (19 §4 tier 1) — the I/O half of adoption.
//!
//! Lazy by design: runs only on `RescanExternal` (the drawer opening), never
//! at startup, never on a timer. Reads are bounded — the head and tail of
//! each transcript, never whole files (667 MB observed under one
//! `~/.claude/projects/`). Everything here is best-effort: an unreadable
//! file is a skipped candidate, never an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{assistant_text, cwd_matches, HeadScan, SessionsPidFile, TranscriptHead};
use mesimon_core::command::ExternalItem;

/// Identity scan budget, from the start of the file. Read record by record and
/// stopped at the first `sessionId` + `cwd` pair, so the usual cost is one
/// buffered read and the cap only bounds a head that never yields. T-425: a
/// fixed 8 KiB window cut the first real record mid-line on 230 of 550
/// transcripts measured, because the latches before it carry no `cwd` and the
/// first record that does can carry a pasted image or prompt — 787 KB was the
/// largest head measured, 20 of 550 sat past 256 KiB.
const HEAD_BYTES_MAX: u64 = 4 * 1024 * 1024;
/// Preview scan windows, back from EOF. Real transcripts bury the last
/// assistant text under tool results, stop_hook_summary and latch records —
/// often several KB, sometimes one record alone exceeds 4 KB (measured on the
/// author's tree) — so a small first window escalates once before giving up.
const TAIL_BYTES: u64 = 32 * 1024;
const TAIL_BYTES_MAX: u64 = 256 * 1024;
pub const PREVIEW_MAX: usize = 160;

/// Where Claude keeps its per-user tree. The env override order is a test
/// seam first (`MESIMON_CLAUDE_HOME`), the real Claude relocation knob second.
pub fn claude_home() -> PathBuf {
    if let Ok(p) = std::env::var("MESIMON_CLAUDE_HOME") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".claude")
}

/// The repo root plus its worktree roots — the cwd-match universe. Tolerates
/// git being absent (plain root only).
pub fn repo_roots(repo_root: &Path) -> Vec<PathBuf> {
    let mut roots = vec![repo_root.to_path_buf()];
    if let Ok(out) = crate::git::git(repo_root).args(["worktree", "list", "--porcelain"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    let p = PathBuf::from(p);
                    if !roots.contains(&p) {
                        roots.push(p);
                    }
                }
            }
        }
    }
    roots
}

pub fn scan(
    home: &Path,
    roots: &[PathBuf],
    known: &dyn Fn(uuid::Uuid) -> bool,
) -> Vec<ExternalItem> {
    let pid_files = read_pid_files(&home.join("sessions"));
    let mut by_session: HashMap<uuid::Uuid, ExternalItem> = HashMap::new();

    let projects = home.join("projects");
    let Ok(dirs) = std::fs::read_dir(&projects) else { return Vec::new() };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(item) = candidate(&path, roots, &pid_files) else { continue };
            if known(item.id) {
                continue;
            }
            // The same session can leave transcripts in more than one project
            // dir (cross-directory resume) — keep the freshest.
            match by_session.get(&item.id) {
                Some(prev) if prev.mtime_ms >= item.mtime_ms => {}
                _ => {
                    by_session.insert(item.id, item);
                }
            }
        }
    }
    let mut v: Vec<ExternalItem> = by_session.into_values().collect();
    v.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then(a.id.cmp(&b.id)));
    v
}

/// Fresh single-session liveness check for the double-resume guard (09 §9):
/// a live pid claiming this sessionId right now. Best-effort — pid reuse can
/// false-positive, which is why the guard is confirm-overridable.
pub fn running_pid_for(home: &Path, session_id: uuid::Uuid) -> Option<i32> {
    live_pid_file(home, session_id).map(|(_, pid)| pid)
}

/// The pid file a LIVE process keeps for this sessionId — the file whose
/// `status` the interrupt probe reads (`Daemon::probe_status_files`). Live
/// pid required: a resumed conversation can leave an older process's file
/// beside the current one, and only the running one's status means anything.
pub fn status_file_for(home: &Path, session_id: uuid::Uuid) -> Option<PathBuf> {
    live_pid_file(home, session_id).map(|(path, _)| path)
}

fn live_pid_file(home: &Path, session_id: uuid::Uuid) -> Option<(PathBuf, i32)> {
    let dir = home.join("sessions");
    let files = std::fs::read_dir(dir).ok()?;
    for f in files.flatten() {
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        let Ok(pf) = serde_json::from_str::<SessionsPidFile>(&text) else { continue };
        if pf.session_id != Some(session_id) {
            continue;
        }
        if let Some(pid) = pf.pid {
            if pid > 0 && unsafe { libc::kill(pid, 0) } == 0 {
                return Some((f.path(), pid));
            }
        }
    }
    None
}

struct PidEntry {
    name: Option<String>,
    alive: bool,
}

/// `~/.claude/sessions/<pid>.json` — enrichment only, joined on the file's
/// `sessionId`, never the filename pid (11 §11.3).
fn read_pid_files(dir: &Path) -> HashMap<uuid::Uuid, PidEntry> {
    let mut map = HashMap::new();
    let Ok(files) = std::fs::read_dir(dir) else { return map };
    for f in files.flatten() {
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        let Ok(pf) = serde_json::from_str::<SessionsPidFile>(&text) else { continue };
        let alive = pf.pid.is_some_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0);
        merge_pid_file(&mut map, pf, alive);
    }
    map
}

/// A resume leaves the dead process's file beside the live one's under the
/// same `sessionId`, and `read_dir` orders them however it likes: a live file
/// outranks a dead one, and between equals the later read wins (T-425).
fn merge_pid_file(map: &mut HashMap<uuid::Uuid, PidEntry>, pf: SessionsPidFile, alive: bool) {
    let Some(sid) = pf.session_id else { return };
    if let Some(have) = map.get(&sid) {
        if have.alive && !alive {
            return;
        }
    }
    map.insert(sid, PidEntry { name: pf.name, alive });
}

fn candidate(
    path: &Path,
    roots: &[PathBuf],
    pid_files: &HashMap<uuid::Uuid, PidEntry>,
) -> Option<ExternalItem> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = mesimon_core::clock::epoch_ms(meta.modified().ok()?)?;

    let head = read_head(path)?;
    if !cwd_matches(&head.cwd, roots) || head.agent_sdk_run() {
        return None;
    }

    let tail = read_tail_info(path, meta.len());
    // The user's own words are an honest preview when the session's final
    // stretch holds no assistant text (measured: one real session ended with
    // 6.8 MB of attachments/snapshots after the last assistant turn).
    let preview = tail.assistant.or_else(|| tail.last_prompt.map(|p| sanitize(&format!("> {p}"))));
    let pid = pid_files.get(&head.session_id);
    let name = pid
        .and_then(|p| p.name.clone())
        .or(tail.custom_title)
        .or(tail.ai_title)
        .map(|n| sanitize(&n));
    Some(ExternalItem {
        id: head.session_id,
        provider: mesimon_core::board::AgentProvider::ClaudeCode,
        conversation_id: head.session_id.to_string(),
        cwd: head.cwd,
        transcript_path: path.display().to_string(),
        mtime_ms,
        preview,
        name,
        running_elsewhere: pid.is_some_and(|p| p.alive),
    })
}

/// The transcript's identity, read record by record under `HEAD_BYTES_MAX`.
/// A record cut by the budget fails to parse and ends the scan.
fn read_head(path: &Path) -> Option<TranscriptHead> {
    use std::io::{BufRead, BufReader, Read};
    let f = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(f.take(HEAD_BYTES_MAX));
    let mut scan = HeadScan::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if let Some(head) = scan.feed(&String::from_utf8_lossy(&line)) {
            return Some(head);
        }
    }
}

/// What the tail scan can offer the drawer: the last assistant text (already
/// sanitized), plus the uuid-less latches that ride near EOF (09 §4.2 —
/// last-write-wins state records: `last-prompt`, `ai-title`, `custom-title`).
#[derive(Default)]
struct TailInfo {
    assistant: Option<String>,
    last_prompt: Option<String>,
    ai_title: Option<String>,
    custom_title: Option<String>,
}

impl TailInfo {
    fn merge_missing(&mut self, other: TailInfo) {
        self.assistant = self.assistant.take().or(other.assistant);
        self.last_prompt = self.last_prompt.take().or(other.last_prompt);
        self.ai_title = self.ai_title.take().or(other.ai_title);
        self.custom_title = self.custom_title.take().or(other.custom_title);
    }
}

fn read_tail_info(path: &Path, len: u64) -> TailInfo {
    let mut info = scan_tail_window(path, len, TAIL_BYTES);
    // Latches live at EOF; only the assistant text warrants the deep window.
    if info.assistant.is_none() && len > TAIL_BYTES {
        info.merge_missing(scan_tail_window(path, len, TAIL_BYTES_MAX));
    }
    info
}

fn scan_tail_window(path: &Path, len: u64, window: u64) -> TailInfo {
    use std::io::{Read, Seek, SeekFrom};
    let mut info = TailInfo::default();
    let Ok(mut f) = std::fs::File::open(path) else { return info };
    let start = len.saturating_sub(window);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return info;
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return info;
    }
    // The seek can land mid-record and mid-UTF-8 — lossy, never fatal.
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // the seek landed mid-line
    }
    // Reversed: the first hit of each kind is the latest write.
    for line in lines.iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("uuid").is_none() {
            let s = |key: &str| v.get(key).and_then(serde_json::Value::as_str).map(str::to_string);
            match v.get("type").and_then(serde_json::Value::as_str) {
                Some("last-prompt") if info.last_prompt.is_none() => {
                    info.last_prompt = s("lastPrompt");
                }
                Some("ai-title") if info.ai_title.is_none() => info.ai_title = s("aiTitle"),
                Some("custom-title") if info.custom_title.is_none() => {
                    info.custom_title = s("customTitle");
                }
                _ => {}
            }
            continue;
        }
        if info.assistant.is_none() {
            if let Some(text) = assistant_text(&v) {
                info.assistant = Some(sanitize(text));
            }
        }
    }
    info
}

/// D29: foreign text entering chrome is sanitized — control characters and
/// direction overrides out, length capped.
pub fn sanitize(text: &str) -> String {
    // A control is a word break here ("my\u{7}session" is two words), then
    // the shared hazard list, then one space between words.
    let spaced: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let cleaned = mesimon_core::text::scrub_cells(&spaced, false);
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for ch in cleaned.chars() {
        if out.chars().count() >= PREVIEW_MAX {
            out.push('~');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID_A: &str = "aaaaaaaa-1111-4e6f-8b1a-2c3d4e5f6a7b";
    const SID_B: &str = "bbbbbbbb-2222-4e6f-8b1a-2c3d4e5f6a7b";
    const SID_C: &str = "cccccccc-3333-4e6f-8b1a-2c3d4e5f6a7b";

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("msmn-census-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_transcript(
        home: &Path,
        slug: &str,
        file: &str,
        sid: &str,
        cwd: &str,
        tail: &str,
    ) -> PathBuf {
        let dir = home.join("projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(file);
        let head = format!(
            "{{\"sessionId\":\"{sid}\",\"cwd\":\"{cwd}\",\"type\":\"user\",\"uuid\":\"u0\"}}\n"
        );
        std::fs::write(&p, format!("{head}{tail}")).unwrap();
        p
    }

    #[test]
    fn scan_finds_matching_cwd_only_and_previews() {
        let home = tmp("scan");
        let repo = "/repo/x";
        let tail = r#"{"uuid":"u1","type":"assistant","message":{"content":[{"type":"text","text":"did the thing"}]}}
{"mode":"plan"}
"#;
        write_transcript(&home, "-gibberish-slug", "a.jsonl", SID_A, repo, tail);
        write_transcript(&home, "-other", "b.jsonl", SID_B, "/elsewhere", "");
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id.to_string(), SID_A);
        assert_eq!(items[0].preview.as_deref(), Some("did the thing"));
        assert!(!items[0].running_elsewhere);
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn scan_skips_known_and_joins_pid_files_on_session_id() {
        let home = tmp("pid");
        let repo = "/repo/y";
        write_transcript(&home, "-s", "a.jsonl", SID_A, repo, "");
        write_transcript(&home, "-s", "c.jsonl", SID_C, repo, "");
        // pid-file: filename pid is garbage on purpose — join is on sessionId.
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let me = std::process::id();
        std::fs::write(
            sessions.join("999999.json"),
            format!("{{\"sessionId\":\"{SID_A}\",\"pid\":{me},\"name\":\"my\\u0007session\"}}"),
        )
        .unwrap();
        let known: uuid::Uuid = SID_C.parse().unwrap();
        let items = scan(&home, &[PathBuf::from(repo)], &|id| id == known);
        assert_eq!(items.len(), 1);
        assert!(items[0].running_elsewhere, "own pid is alive");
        assert_eq!(items[0].name.as_deref(), Some("my session"));
        std::fs::remove_dir_all(home).ok();
    }

    /// A program's conversation is not a session a person could take over
    /// (T-441); the CLI driven by an app is, and so is a head that names no
    /// entrypoint at all.
    #[test]
    fn scan_skips_agent_sdk_runs() {
        let home = tmp("sdk");
        let repo = "/repo/s";
        let dir = home.join("projects").join("-s");
        std::fs::create_dir_all(&dir).unwrap();
        for (sid, entrypoint) in [(SID_A, "sdk-py"), (SID_B, "sdk-cli"), (SID_C, "cli")] {
            std::fs::write(
                dir.join(format!("{sid}.jsonl")),
                format!(
                    "{{\"type\":\"queue-operation\",\"sessionId\":\"{sid}\"}}\n\
                     {{\"sessionId\":\"{sid}\",\"cwd\":\"{repo}\",\"entrypoint\":\"{entrypoint}\",\"type\":\"user\",\"uuid\":\"u0\"}}\n"
                ),
            )
            .unwrap();
        }
        let bare = "dddddddd-4444-4e6f-8b1a-2c3d4e5f6a7b";
        write_transcript(&home, "-s", "d.jsonl", bare, repo, "");
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        let mut ids: Vec<String> = items.iter().map(|i| i.id.to_string()).collect();
        ids.sort();
        assert_eq!(ids, [SID_B, SID_C, bare]);
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn dead_pid_is_not_running_elsewhere() {
        let home = tmp("dead");
        let repo = "/repo/z";
        write_transcript(&home, "-s", "a.jsonl", SID_A, repo, "");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        // PID 99999999 exceeds macOS/Linux defaults — reliably dead.
        std::fs::write(
            sessions.join("1.json"),
            format!("{{\"sessionId\":\"{SID_A}\",\"pid\":99999999}}"),
        )
        .unwrap();
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        assert_eq!(items.len(), 1);
        assert!(!items[0].running_elsewhere);
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn preview_found_when_buried_under_kilobytes_of_tail_noise() {
        // Real shape: assistant text, then tool results / stop_hook_summary /
        // uuid-less latches piling up well past the old 4 KB window.
        let home = tmp("buried");
        let repo = "/repo/b";
        let noise: String = (0..40)
            .map(|i| {
                format!(
                    "{{\"uuid\":\"n{i}\",\"type\":\"system\",\"subtype\":\"stop_hook_summary\",\"pad\":\"{}\"}}\n{{\"cost-state\":\"{}\"}}\n",
                    "x".repeat(300),
                    "y".repeat(200)
                )
            })
            .collect();
        let tail = format!(
            "{{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"buried preview\"}}]}}}}\n{noise}"
        );
        assert!(tail.len() > 4 * 1024, "fixture must exceed the old window");
        write_transcript(&home, "-s", "a.jsonl", SID_A, repo, &tail);
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        assert_eq!(items[0].preview.as_deref(), Some("buried preview"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn preview_falls_back_to_last_prompt_and_name_to_ai_title() {
        // No assistant text anywhere near EOF — the latches carry the story.
        let home = tmp("latch");
        let repo = "/repo/l";
        let tail = format!(
            "{{\"uuid\":\"u1\",\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"do it\"}}]}}}}\n\
             {{\"type\":\"last-prompt\",\"lastPrompt\":\"ill push, find it\",\"sessionId\":\"{SID_A}\"}}\n\
             {{\"type\":\"ai-title\",\"aiTitle\":\"Fix widget animation\",\"sessionId\":\"{SID_A}\"}}\n"
        );
        write_transcript(&home, "-s", "a.jsonl", SID_A, repo, &tail);
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        assert_eq!(items[0].preview.as_deref(), Some("> ill push, find it"));
        assert_eq!(items[0].name.as_deref(), Some("Fix widget animation"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn head_reads_past_latches_and_a_record_larger_than_the_old_window() {
        // Measured shape (T-425): latch records carrying `sessionId` and no
        // `cwd`, then a 22 KB attachment as the first real record. An 8 KiB
        // window cut that record mid-line and lost the session.
        let home = tmp("bighead");
        let repo = "/repo/h";
        let dir = home.join("projects").join("-s");
        std::fs::create_dir_all(&dir).unwrap();
        let latches = format!(
            "{{\"type\":\"last-prompt\",\"lastPrompt\":\"hi\",\"sessionId\":\"{SID_A}\"}}\n\
             {{\"type\":\"mode\",\"mode\":\"default\",\"sessionId\":\"{SID_A}\"}}\n\
             {{\"type\":\"permission-mode\",\"permissionMode\":\"default\",\"sessionId\":\"{SID_A}\"}}\n\
             {{\"type\":\"atis-latch\",\"sessionId\":\"{SID_A}\"}}\n"
        );
        let big = format!(
            "{{\"uuid\":\"u0\",\"type\":\"attachment\",\"attachment\":{{\"body\":\"{}\"}},\"sessionId\":\"{SID_A}\",\"cwd\":\"{repo}\"}}\n",
            "a".repeat(22 * 1024)
        );
        assert!(latches.len() + big.len() > 8 * 1024, "fixture must exceed the old window");
        let tail = "{\"uuid\":\"u1\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"after the attachment\"}]}}\n";
        std::fs::write(dir.join("a.jsonl"), format!("{latches}{big}{tail}")).unwrap();
        let items = scan(&home, &[PathBuf::from(repo)], &|_| false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id.to_string(), SID_A);
        assert_eq!(items[0].cwd, repo);
        assert_eq!(items[0].preview.as_deref(), Some("after the attachment"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn head_scan_stops_at_its_budget() {
        // A head that never yields a cwd inside the budget is a skipped
        // candidate, and the read never goes past `HEAD_BYTES_MAX`.
        let home = tmp("nohead");
        let dir = home.join("projects").join("-s");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.jsonl");
        let latch = format!("{{\"type\":\"atis-latch\",\"sessionId\":\"{SID_A}\"}}\n");
        let filler =
            format!("{{\"uuid\":\"u0\",\"pad\":\"{}\"}}\n", "p".repeat(HEAD_BYTES_MAX as usize));
        let late =
            format!("{{\"uuid\":\"u1\",\"cwd\":\"/repo/late\",\"sessionId\":\"{SID_A}\"}}\n");
        std::fs::write(&p, format!("{latch}{filler}{late}")).unwrap();
        assert!(read_head(&p).is_none());
        // The same file with the pad under budget parses — the cap is the
        // only thing in the way.
        let filler = format!("{{\"uuid\":\"u0\",\"pad\":\"{}\"}}\n", "p".repeat(100 * 1024));
        std::fs::write(&p, format!("{latch}{filler}{late}")).unwrap();
        assert_eq!(read_head(&p).map(|h| h.cwd).as_deref(), Some("/repo/late"));
        std::fs::remove_dir_all(home).ok();
    }

    #[test]
    fn a_live_pid_file_outranks_a_dead_one_under_the_same_session_in_either_order() {
        let sid: uuid::Uuid = SID_A.parse().unwrap();
        let file = |name: &str| SessionsPidFile {
            session_id: Some(sid),
            name: Some(name.to_string()),
            ..Default::default()
        };
        for live_first in [true, false] {
            let mut map = HashMap::new();
            let order: Vec<(SessionsPidFile, bool)> = if live_first {
                vec![(file("live"), true), (file("dead"), false)]
            } else {
                vec![(file("dead"), false), (file("live"), true)]
            };
            for (pf, alive) in order {
                merge_pid_file(&mut map, pf, alive);
            }
            let e = &map[&sid];
            assert!(e.alive, "live_first={live_first}");
            assert_eq!(e.name.as_deref(), Some("live"), "live_first={live_first}");
        }
        // Two dead files: the later read wins, as before.
        let mut map = HashMap::new();
        merge_pid_file(&mut map, file("first"), false);
        merge_pid_file(&mut map, file("second"), false);
        assert!(!map[&sid].alive);
        assert_eq!(map[&sid].name.as_deref(), Some("second"));
    }

    #[test]
    fn sanitize_strips_controls_and_caps() {
        assert_eq!(sanitize("a\x1b[31mb\u{202e}c"), "a [31mbc");
        let long = "x".repeat(300);
        let s = sanitize(&long);
        assert!(s.chars().count() <= PREVIEW_MAX + 1);
        assert!(s.ends_with('~'));
    }
}
