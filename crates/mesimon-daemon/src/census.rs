//! External-session census (19 §4 tier 1) — the I/O half of adoption.
//!
//! Lazy by design: runs only on `RescanExternal` (the drawer opening), never
//! at startup, never on a timer. Reads are bounded — the head and tail of
//! each transcript, never whole files (667 MB observed under one
//! `~/.claude/projects/`). Everything here is best-effort: an unreadable
//! file is a skipped candidate, never an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mesimon_core::adopt::{
    classify_tail_record, cwd_matches, parse_transcript_head, SessionsPidFile, TailEvent,
};
use mesimon_core::command::ExternalItem;

const HEAD_BYTES: usize = 8 * 1024;
const TAIL_BYTES: u64 = 4 * 1024;
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
    if let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
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

pub fn scan(home: &Path, roots: &[PathBuf], known: &dyn Fn(uuid::Uuid) -> bool) -> Vec<ExternalItem> {
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
            if known(item.claude_session_id) {
                continue;
            }
            // The same session can leave transcripts in more than one project
            // dir (cross-directory resume) — keep the freshest.
            match by_session.get(&item.claude_session_id) {
                Some(prev) if prev.mtime_ms >= item.mtime_ms => {}
                _ => {
                    by_session.insert(item.claude_session_id, item);
                }
            }
        }
    }
    let mut v: Vec<ExternalItem> = by_session.into_values().collect();
    v.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then(a.claude_session_id.cmp(&b.claude_session_id)));
    v
}

/// Fresh single-session liveness check for the double-resume guard (09 §9):
/// a live pid claiming this sessionId right now. Best-effort — pid reuse can
/// false-positive, which is why the guard is confirm-overridable.
pub fn running_pid_for(home: &Path, session_id: uuid::Uuid) -> Option<i32> {
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
                return Some(pid);
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
        let Some(sid) = pf.session_id else { continue };
        let alive = pf.pid.is_some_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0);
        map.insert(sid, PidEntry { name: pf.name, alive });
    }
    map
}

fn candidate(
    path: &Path,
    roots: &[PathBuf],
    pid_files: &HashMap<uuid::Uuid, PidEntry>,
) -> Option<ExternalItem> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)?;

    let head = read_head(path)?;
    let head = parse_transcript_head(&head)?;
    if !cwd_matches(&head.cwd, roots) {
        return None;
    }

    let preview = read_tail_preview(path, meta.len());
    let pid = pid_files.get(&head.session_id);
    Some(ExternalItem {
        claude_session_id: head.session_id,
        cwd: head.cwd,
        transcript_path: path.display().to_string(),
        mtime_ms,
        preview,
        name: pid.and_then(|p| p.name.clone()).map(|n| sanitize(&n)),
        running_elsewhere: pid.is_some_and(|p| p.alive),
    })
}

fn read_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD_BYTES];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Last assistant text from the file's tail — the drawer's one-line preview.
fn read_tail_preview(path: &Path, len: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    let mut lines: Vec<&str> = buf.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // the seek landed mid-line
    }
    for line in lines.iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if let TailEvent::AssistantText { text } = classify_tail_record(&v) {
            return Some(sanitize(&text));
        }
    }
    None
}

/// D29: foreign text entering chrome is sanitized — control characters and
/// direction overrides out, length capped.
pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { c })
        .collect();
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

    fn write_transcript(home: &Path, slug: &str, file: &str, sid: &str, cwd: &str, tail: &str) -> PathBuf {
        let dir = home.join("projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(file);
        let head = format!("{{\"sessionId\":\"{sid}\",\"cwd\":\"{cwd}\",\"type\":\"user\",\"uuid\":\"u0\"}}\n");
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
        assert_eq!(items[0].claude_session_id.to_string(), SID_A);
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
    fn sanitize_strips_controls_and_caps() {
        assert_eq!(sanitize("a\x1b[31mb\u{202e}c"), "a [31mb c");
        let long = "x".repeat(300);
        let s = sanitize(&long);
        assert!(s.chars().count() <= PREVIEW_MAX + 1);
        assert!(s.ends_with('~'));
    }
}
