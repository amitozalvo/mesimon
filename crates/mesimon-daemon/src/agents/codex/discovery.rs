//! Bounded native Codex rollout discovery. History provides previews and identity,
//! never sufficient evidence to release a checkout or declare a live turn idle.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::AgentProvider;
use mesimon_core::command::ExternalItem;
use serde_json::Value;
use sha2::{Digest, Sha256};

const HEAD_BYTES: u64 = 128 * 1024;
const TAIL_BYTES: u64 = 256 * 1024;
const SCAN_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;
const MAX_FILES: usize = 2048;
const PROCESS_BYTES: usize = 1024 * 1024;

pub fn codex_home() -> PathBuf {
    std::env::var_os("MESIMON_CODEX_HOME")
        .or_else(|| std::env::var_os("CODEX_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        })
}

/// An unavailable process inventory is different from an empty inventory.
/// Callers must require explicit confirmation for both Live and Unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ownership {
    Clear,
    Live(i32),
    Unknown,
}

impl Ownership {
    pub fn hold_message(&self) -> Option<String> {
        match self {
            Self::Clear => None,
            Self::Live(pid) => Some(format!("Codex process {pid} may own this conversation")),
            Self::Unknown => Some("Codex conversation ownership could not be established".into()),
        }
    }
}

#[derive(Debug)]
struct Head {
    id: String,
    cwd: String,
}

fn head(bytes: &[u8]) -> Option<Head> {
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(value) = serde_json::from_slice::<Value>(line) else { continue };
        if value["type"] != "session_meta" {
            continue;
        }
        if value["payload"]["thread_source"] == "system" {
            return None;
        }
        let id = value["payload"]["id"].as_str()?;
        let cwd = value["payload"]["cwd"].as_str()?;
        if id.is_empty()
            || id.len() > 1024
            || id.chars().any(char::is_control)
            || value["payload"]["session_id"].as_str().is_some_and(|alias| alias != id)
            || !Path::new(cwd).is_absolute()
        {
            return None;
        }
        return Some(Head { id: id.into(), cwd: cwd.into() });
    }
    None
}

fn selector(id: &str) -> uuid::Uuid {
    let mut hash = Sha256::new();
    hash.update(b"mesimon:external:codex\0");
    hash.update(id.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    // UUID version 8 reserves application-specific deterministic identifiers.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

fn window(path: &Path, tail: bool, budget: &mut u64) -> Option<Vec<u8>> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let limit = if tail { TAIL_BYTES } else { HEAD_BYTES }.min(*budget);
    if limit == 0 {
        return None;
    }
    let start = if tail { len.saturating_sub(limit) } else { 0 };
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    *budget = budget.saturating_sub(bytes.len() as u64);
    if start > 0 {
        // Never interpret a partial first JSON record after seeking into a tail.
        let end = bytes.iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| p + 1);
        bytes.drain(..end);
    }
    Some(bytes)
}

fn display(text: &str, max: usize) -> String {
    mesimon_core::text::scrub_cells(text, false).chars().take(max).collect()
}

#[derive(Default)]
struct Tail {
    assistant: Option<String>,
    user: Option<String>,
    key: Option<u64>,
}

fn tail(bytes: &[u8]) -> Tail {
    let mut result = Tail::default();
    for line in bytes.split(|b| *b == b'\n').rev() {
        let Ok(value) = serde_json::from_slice::<Value>(line) else { continue };
        let payload = &value["payload"];
        if value["type"] == "response_item" && payload["type"] == "message" {
            let role = payload["role"].as_str().unwrap_or_default();
            if role != "assistant" && role != "user" {
                continue;
            }
            let Some(content) = payload["content"].as_array() else { continue };
            let text = content
                .iter()
                .filter(|part| part["type"] == "output_text" || part["type"] == "input_text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                continue;
            }
            if role == "assistant" && result.assistant.is_none() {
                result.assistant = Some(display(&text, 4096));
                result.key = Some(u64::from_le_bytes(
                    Sha256::digest(line)[..8].try_into().expect("hash length"),
                ));
            } else if role == "user" && result.user.is_none() {
                result.user = Some(display(&text, 160));
            }
        }
        if result.assistant.is_some() && result.user.is_some() {
            break;
        }
    }
    result
}

/// Read-only native history preview. No transcript event changes board state.
pub fn read_preview(path: &Path) -> Option<super::super::AgentPreview> {
    let mut budget = HEAD_BYTES + TAIL_BYTES;
    head(&window(path, false, &mut budget)?)?;
    let parsed = tail(&window(path, true, &mut budget)?);
    Some(super::super::AgentPreview {
        text: parsed.assistant,
        activity: None,
        reply_key: parsed.key,
    })
}

/// Scan only native sessions, at the documented year/month/day depth. Symlinks,
/// unreadable records, oversized heads and unrelated repository roots are skipped.
pub fn scan(home: &Path, roots: &[PathBuf], known: &dyn Fn(&str) -> bool) -> Vec<ExternalItem> {
    let mut paths = Vec::new();
    let mut stack = vec![(home.join("sessions"), 0)];
    let mut entries = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(children) = std::fs::read_dir(dir) else { continue };
        for child in children.flatten() {
            entries += 1;
            if entries > MAX_ENTRIES || paths.len() >= MAX_FILES {
                break;
            }
            let Ok(kind) = child.file_type() else { continue };
            if kind.is_dir() && depth < 3 {
                stack.push((child.path(), depth + 1));
            } else if kind.is_file() && child.path().extension().is_some_and(|ext| ext == "jsonl") {
                paths.push(child.path());
            }
        }
        if entries > MAX_ENTRIES || paths.len() >= MAX_FILES {
            break;
        }
    }
    let inventory = ProcessInventory::read();
    scan_paths(paths, roots, known, &inventory)
}

fn scan_paths(
    paths: Vec<PathBuf>,
    roots: &[PathBuf],
    known: &dyn Fn(&str) -> bool,
    inventory: &ProcessInventory,
) -> Vec<ExternalItem> {
    let mut budget = SCAN_BYTES;
    let mut found: HashMap<String, ExternalItem> = HashMap::new();
    for path in paths {
        let Some(parsed) = window(&path, false, &mut budget).and_then(|bytes| head(&bytes)) else {
            continue;
        };
        if known(&parsed.id) || !mesimon_core::adopt::cwd_matches(&parsed.cwd, roots) {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        let Some(mtime_ms) = meta.modified().ok().and_then(mesimon_core::clock::epoch_ms) else {
            continue;
        };
        if found.get(&parsed.id).is_some_and(|old| old.mtime_ms >= mtime_ms) {
            continue;
        }
        let tail = window(&path, true, &mut budget).map(|bytes| tail(&bytes)).unwrap_or_default();
        let preview = tail
            .assistant
            .map(|text| display(&text, 160))
            .or_else(|| tail.user.map(|text| display(&format!("> {text}"), 160)));
        let ownership = inventory.owner(&parsed.id, Path::new(&parsed.cwd), &path);
        found.insert(
            parsed.id.clone(),
            ExternalItem {
                id: selector(&parsed.id),
                provider: AgentProvider::Codex,
                conversation_id: parsed.id,
                cwd: parsed.cwd,
                transcript_path: path.to_string_lossy().into_owned(),
                mtime_ms,
                preview,
                name: None,
                running_elsewhere: ownership != Ownership::Clear,
            },
        );
        if budget == 0 {
            break;
        }
    }
    let mut items: Vec<_> = found.into_values().collect();
    items.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then(a.id.cmp(&b.id)));
    items
}

/// Fresh inventory for takeover. The result is only a duplicate-writer guard,
/// never state evidence and never authority to kill an external process.
pub fn external_owner(conversation: &str, cwd: &Path, transcript: &Path) -> Ownership {
    ProcessInventory::read().owner(conversation, cwd, transcript)
}

#[derive(Default)]
struct ProcessInventory {
    complete: bool,
    processes: Vec<Process>,
}

#[derive(Default)]
struct Process {
    pid: i32,
    args: String,
    cwd: Option<PathBuf>,
    files: Vec<PathBuf>,
    inspected: bool,
}

impl ProcessInventory {
    fn read() -> Self {
        let Some(output) = bounded_output("ps", &["-axo", "pid=,comm="]) else {
            return Self::default();
        };
        let configured = std::env::var_os("MESIMON_CODEX_BIN").map(PathBuf::from);
        let configured_name = configured.as_deref().and_then(Path::file_name);
        let mut processes = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            let Some((pid, command)) = line.split_once(char::is_whitespace) else { continue };
            let Ok(pid) = pid.parse::<i32>() else { continue };
            let name = Path::new(command.trim()).file_name();
            if name.is_some_and(|name| name == "codex" || Some(name) == configured_name) {
                processes.push(Process { pid, ..Process::default() });
            }
        }
        if processes.is_empty() {
            return Self { complete: true, processes };
        }
        if processes.len() > 256 {
            return Self::default();
        }
        let pids =
            processes.iter().map(|process| process.pid.to_string()).collect::<Vec<_>>().join(",");
        let Some(args) = bounded_output("ps", &["-ww", "-p", &pids, "-o", "pid=,args="]) else {
            return Self::default();
        };
        for line in args.lines() {
            let Some((pid, args)) = line.trim().split_once(char::is_whitespace) else { continue };
            if let Some(process) =
                pid.parse::<i32>().ok().and_then(|pid| processes.iter_mut().find(|p| p.pid == pid))
            {
                process.args = args.trim().into();
            }
        }
        let Some(files) = bounded_output("lsof", &["-nP", "-a", "-p", &pids, "-F", "pfn"]) else {
            return Self { complete: false, processes };
        };
        parse_lsof(&files, &mut processes);
        Self { complete: true, processes }
    }

    fn owner(&self, conversation: &str, cwd: &Path, transcript: &Path) -> Ownership {
        let mut uncertain = !self.complete;
        for process in &self.processes {
            if process.files.iter().any(|file| same_path(file, transcript))
                || process.args.split_whitespace().any(|arg| arg == conversation)
            {
                return Ownership::Live(process.pid);
            }
            // An app-server can retain threads whose rollouts are currently closed.
            // A different cwd cannot prove that it does not own this conversation.
            // A shared cwd likewise identifies only a possible owner: another
            // conversation can legitimately work in that same checkout.
            if !process.inspected
                || process.cwd.is_none()
                || process.cwd.as_deref().is_some_and(|path| same_path(path, cwd))
                || process.args.is_empty()
                || process
                    .args
                    .split_whitespace()
                    .any(|arg| arg == "app-server" || arg == "--remote")
            {
                uncertain = true;
            }
        }
        if uncertain {
            Ownership::Unknown
        } else {
            Ownership::Clear
        }
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || left.canonicalize().ok().zip(right.canonicalize().ok()).is_some_and(|(a, b)| a == b)
}

fn parse_lsof(text: &str, processes: &mut [Process]) {
    let mut current = None;
    let mut cwd = false;
    for line in text.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            current =
                pid.parse::<i32>().ok().and_then(|pid| processes.iter().position(|p| p.pid == pid));
        } else if let Some(fd) = line.strip_prefix('f') {
            cwd = fd == "cwd";
        } else if let (Some(name), Some(index)) = (line.strip_prefix('n'), current) {
            if cwd {
                processes[index].cwd = Some(name.into());
            } else {
                processes[index].files.push(name.into());
            }
            processes[index].inspected = true;
        }
    }
}

/// Read-only process commands have a hard deadline and output limit. Nonblocking
/// reads avoid pipe saturation; failed/denied/timed-out commands mean Unknown.
fn bounded_output(program: &str, args: &[&str]) -> Option<String> {
    use std::os::fd::AsRawFd;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let deadline = Instant::now() + Duration::from_millis(750);
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    let result = loop {
        match stdout.read(&mut chunk) {
            Ok(0) => match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) => {}
                Err(_) => break false,
            },
            Ok(size) => {
                bytes.extend_from_slice(&chunk[..size]);
                if bytes.len() > PROCESS_BYTES {
                    break false;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break false,
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if !result {
        let _ = child.kill();
    }
    let _ = child.wait();
    result.then(|| String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(id: &str, cwd: &str) -> String {
        json!({"timestamp":"2026-09-10T00:00:00Z","type":"session_meta",
            "payload":{"id":id,"session_id":id,"cwd":cwd,"source":"vscode",
                "thread_source":"user","originator":"codex-tui","history_mode":"paginated",
                "cli_version":"0.153.4"}})
        .to_string()
    }
    fn reply(role: &str, text: &str) -> String {
        json!({"timestamp":"2026-09-10T00:01:00Z","type":"response_item",
            "payload":{"type":"message","role":role,"content":[{"type":if role == "assistant" {"output_text"} else {"input_text"},"text":text}]}}).to_string()
    }

    #[test]
    fn opaque_identity_comes_from_metadata_not_filename() {
        let parsed = head(record("opaque-session:alpha", "/fixture/repo").as_bytes()).unwrap();
        assert_eq!(parsed.id, "opaque-session:alpha");
        assert_eq!(selector(&parsed.id), selector("opaque-session:alpha"));
        assert_ne!(selector(&parsed.id), selector("opaque-session:beta"));
        assert!(head(record("", "/fixture/repo").as_bytes()).is_none());
        assert!(head(record("valid", "relative/repo").as_bytes()).is_none());
        let mut system: Value = serde_json::from_str(&record("system", "/fixture/repo")).unwrap();
        system["payload"]["thread_source"] = "system".into();
        assert!(head(system.to_string().as_bytes()).is_none());
        system["payload"]["thread_source"] = "user".into();
        system["payload"]["session_id"] = "conflicting-id".into();
        assert!(head(system.to_string().as_bytes()).is_none());
    }

    #[test]
    fn preview_ignores_tool_output_and_malformed_tail_and_scrubs_controls() {
        let text = format!(
            "{}\n{}\n{}\n{{broken",
            reply("user", "Synthetic request"),
            reply("assistant", "Fixture reply\u{1b}[31m red"),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"Fake later reply"}})
        );
        let parsed = tail(text.as_bytes());
        assert_eq!(parsed.assistant.as_deref(), Some("Fixture reply[31m red"));
        assert!(!parsed.assistant.as_ref().unwrap().contains('\u{1b}'));
        assert_eq!(parsed.user.as_deref(), Some("Synthetic request"));
        assert!(parsed.key.is_some());
    }

    #[test]
    fn uncertain_process_inventory_cannot_authorize_takeover() {
        let cwd = Path::new("/fixture/repo");
        let rollout = Path::new("/fixture/history.jsonl");
        assert_eq!(ProcessInventory::default().owner("opaque", cwd, rollout), Ownership::Unknown);
        let mut inventory = ProcessInventory { complete: true, processes: vec![] };
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Clear);
        inventory.processes.push(Process {
            pid: 12,
            args: "codex app-server".into(),
            cwd: Some("/other/repo".into()),
            inspected: true,
            files: vec![],
        });
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Unknown);
        inventory.processes[0].files.push(rollout.into());
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Live(12));
        inventory.processes[0].files.clear();
        inventory.processes[0].args = "codex resume opaque".into();
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Live(12));
        inventory.processes[0].args = "codex resume opaque-prefix".into();
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Clear);
        inventory.processes[0].cwd = Some(cwd.into());
        assert_eq!(
            inventory.owner("opaque", cwd, rollout),
            Ownership::Unknown,
            "a different conversation in the same checkout is only a possible owner"
        );
        inventory.processes[0].files.push(rollout.into());
        assert_eq!(inventory.owner("opaque", cwd, rollout), Ownership::Live(12));
    }

    #[test]
    fn lsof_is_parsed_by_pid_and_descriptor_not_display_command() {
        let mut processes = vec![
            Process { pid: 12, ..Process::default() },
            Process { pid: 13, ..Process::default() },
        ];
        parse_lsof("p12\nfcwd\nn/fixture/repo with spaces\nf5\nn/fixture/session.jsonl\np13\nfcwd\nn/other/repo\n", &mut processes);
        assert_eq!(processes[0].cwd.as_deref(), Some(Path::new("/fixture/repo with spaces")));
        assert_eq!(processes[0].files, vec![PathBuf::from("/fixture/session.jsonl")]);
        assert_eq!(processes[1].cwd.as_deref(), Some(Path::new("/other/repo")));
    }

    #[test]
    fn scan_matches_repository_filters_known_and_deduplicates() {
        let root =
            std::env::temp_dir().join(format!("mesimon-codex-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("unrelated-name.jsonl");
        let b = root.join("duplicate.jsonl");
        let foreign = root.join("foreign.jsonl");
        let content = format!(
            "{}\n{}\n",
            record("opaque-session", "/fixture/repo/subdir"),
            reply("assistant", "Synthetic reply")
        );
        std::fs::write(&a, &content).unwrap();
        std::fs::write(&b, &content).unwrap();
        std::fs::write(&foreign, record("foreign", "/fixture/repo-other")).unwrap();
        let paths = vec![a.clone(), b, foreign];
        let roots = vec![PathBuf::from("/fixture/repo")];
        let items = scan_paths(paths.clone(), &roots, &|_| false, &ProcessInventory::default());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].provider, AgentProvider::Codex);
        assert_eq!(items[0].conversation_id, "opaque-session");
        assert!(items[0].running_elsewhere);
        assert_eq!(items[0].preview.as_deref(), Some("Synthetic reply"));
        assert!(scan_paths(
            paths,
            &roots,
            &|id| id == "opaque-session",
            &ProcessInventory::default()
        )
        .is_empty());
        assert_eq!(read_preview(&a).unwrap().text.as_deref(), Some("Synthetic reply"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
