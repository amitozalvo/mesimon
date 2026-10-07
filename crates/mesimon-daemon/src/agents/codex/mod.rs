//! Native Codex integration: protocol observation and lifecycle stay here.

pub mod discovery;
pub mod observation;
mod recovery;
pub use recovery::{
    recovery_endpoint_absent, recovery_launch_target, recovery_owner_absent,
    restore_unverified_cleanup, retain_unverified_cleanup, RecoveryLaunchTarget,
};
pub mod rpc;
pub mod runtime;

use mesimon_core::board::SessionState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{AgentAdapter, ExternalOwner, LaunchContext, LaunchSpec};
use crate::hook_settings::mesimon_bin;

pub struct Codex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub session: uuid::Uuid,
    pub generation: u64,
    pub cwd: PathBuf,
    pub executable: String,
    pub upstream_socket: PathBuf,
    pub proxy_socket: PathBuf,
    pub snapshot_path: PathBuf,
    pub preview_path: PathBuf,
    pub resume: Option<String>,
    pub config_flags: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn snapshot_path(paths: &crate::paths::Paths, id: uuid::Uuid) -> PathBuf {
    paths.rt_dir.join(format!("cdx-{}.json", &id.simple().to_string()[..16]))
}

pub fn preview_path(paths: &crate::paths::Paths, id: uuid::Uuid) -> PathBuf {
    paths.hooks_dir().join(format!("{id}.preview.json"))
}

/// Input readiness is independent of the configurable status line. Require
/// the native text cursor in the composer; menus hide it, and tmux copy mode
/// has no application cursor. Structured idle observation gates delivery too.
pub fn input_ready(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let Some((x, y)) = screen.cursor else { return false };
    let Some(above) = screen.lines.get(..=y) else { return false };
    let Some(prompt) = above.iter().rposition(|line| {
        line.strip_prefix('›').is_some_and(|text| text.is_empty() || text.starts_with(' '))
    }) else {
        return false;
    };
    // Wrapped/multiline input keeps the composer's two-column indentation.
    // Nothing below the cursor (including any status line) participates.
    x >= 2 && above[prompt + 1..].iter().all(|line| line.is_empty() || line.starts_with("  "))
}

/// The native implementation dialog is local UI, not an app-server request.
/// Require its choice list as well as the heading to reject conversational text.
pub fn plan_dialog(screen: &mesimon_backend_tmux::InputScreen) -> bool {
    let lines = &screen.lines;
    let Some(index) = lines.iter().rposition(|line| line.trim() == "Implement this plan?") else {
        return false;
    };
    let tail = &lines[index..];
    tail.iter().any(|line| line.contains("Yes, implement this plan"))
        && tail.iter().any(|line| line.contains("No, stay in Plan mode"))
        && !input_ready(screen)
}

pub fn startup_attention(
    screen: &mesimon_backend_tmux::InputScreen,
) -> Option<mesimon_core::board::Reason> {
    use mesimon_core::board::Reason;
    if input_ready(screen) {
        return None;
    }
    let text = screen.lines.join("\n");
    if text.contains("Hooks need review") || text.contains("Do you trust") {
        Some(Reason::Trust)
    } else if text.contains("Sign in with ChatGPT") || text.contains("Sign in with an API key") {
        Some(Reason::Auth)
    } else {
        None
    }
}

fn policy_flags(column: &mesimon_core::board::ColumnSettings) -> Vec<String> {
    use mesimon_core::board::{CodexApproval, CodexSandbox};
    let mut flags = Vec::new();
    let sandbox = match column.codex_sandbox {
        CodexSandbox::Inherit => None,
        CodexSandbox::ReadOnly => Some("read-only"),
        CodexSandbox::WorkspaceWrite => Some("workspace-write"),
    };
    let approval = match column.codex_approval {
        CodexApproval::Inherit => None,
        CodexApproval::OnRequest => Some("on-request"),
        CodexApproval::Never => Some("never"),
    };
    for (key, value) in [("sandbox_mode", sandbox), ("approval_policy", approval)] {
        if let Some(value) = value {
            flags.extend(["-c".into(), format!("{key}=\"{value}\"")]);
        }
    }
    flags
}

/// The ticket's tier as Codex config (T-443): `model` and
/// `model_reasoning_effort`, the keys `~/.codex/config.toml` itself uses.
/// `-c` rather than `-m`, because `config_flags` reach both the app server
/// and the native client, and a resumed thread takes them too. The model is
/// `tier::check_model`'s whitelist, so it needs no TOML escaping.
fn tier_flags(tier: &mesimon_core::tier::Tier) -> Vec<String> {
    let mut flags = Vec::new();
    if tier.provider != mesimon_core::board::AgentProvider::Codex {
        return flags;
    }
    if let Some(model) = tier.model_arg() {
        flags.extend(["-c".into(), format!("model=\"{model}\"")]);
    }
    if let Some(effort) = tier.effort_arg() {
        flags.extend(["-c".into(), format!("model_reasoning_effort=\"{effort}\"")]);
    }
    flags
}

fn prepare(context: &LaunchContext<'_>, resume: Option<String>) -> Result<LaunchSpec, String> {
    let id = context.session;
    let generation = (uuid::Uuid::new_v4().as_u128() as u64).max(1);
    let stem = format!("cdx-{}-{:08x}", &id.simple().to_string()[..16], generation as u32);
    let mut config_flags = policy_flags(&context.column);
    config_flags.extend(tier_flags(&context.tier));
    // Stable definitions let native /hooks trust survive ticket changes.
    // Identity and guarded roots are invocation environment, not hook text.
    // Codex 0.153.4 preserves user hooks from both config.toml and hooks.json
    // beside these per-launch entries (bounded runtime probe).
    let binary = mesimon_bin().display().to_string();
    let quoted = format!("'{}'", binary.replace('\'', "'\\''"));
    let gate = serde_json::to_string(&format!("{quoted} gate --provider codex --from-env"))
        .map_err(|e| e.to_string())?;
    config_flags.extend(["-c".into(), format!("hooks.PreToolUse=[{{matcher=\"^apply_patch$\",hooks=[{{type=\"command\",command={gate},timeout=2}}]}}]")]);
    let brief = context.brief && context.tools != mesimon_core::board::AgentTools::Off;
    if brief {
        let command =
            serde_json::to_string(&format!("{quoted} agent-brief")).map_err(|e| e.to_string())?;
        config_flags.extend([
            "-c".into(),
            format!(
                "hooks.SessionStart=[{{hooks=[{{type=\"command\",command={command},timeout=2}}]}}]"
            ),
        ]);
    }
    if context.tools != mesimon_core::board::AgentTools::Off {
        let rendered =
            crate::hook_settings::mcp_config_json(context.paths, &mesimon_bin(), id, context.tools);
        let value: serde_json::Value =
            serde_json::from_str(&rendered).map_err(|e| e.to_string())?;
        let server = &value["mcpServers"]["mesimon"];
        config_flags.extend([
            "-c".into(),
            format!(
                "mcp_servers.mesimon={{command={},args={}}}",
                server["command"], server["args"],
            ),
        ]);
    }
    let config = RuntimeConfig {
        session: id,
        generation,
        cwd: context.cwd.to_path_buf(),
        executable: std::env::var("MESIMON_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
        upstream_socket: context.paths.rt_dir.join(format!("{stem}-up.sock")),
        proxy_socket: context.paths.rt_dir.join(format!("{stem}-ui.sock")),
        snapshot_path: snapshot_path(context.paths, id),
        preview_path: preview_path(context.paths, id),
        resume,
        config_flags,
        env: vec![
            ("MESIMON_AGENT_BRIEF".into(), if brief { "1" } else { "0" }.into()),
            ("MESIMON_SESSION".into(), id.to_string()),
            ("MESIMON_GATE_BOARD".into(), context.paths.board_dir.display().to_string()),
            ("MESIMON_GATE_STATE".into(), context.paths.state_dir.display().to_string()),
            ("MESIMON_GATE_ALLOW".into(), context.paths.worktrees_root().display().to_string()),
            ("MESIMON_HOOK_SOCK".into(), context.paths.hook_sock().display().to_string()),
        ],
    };
    let path = context.paths.hooks_dir().join(format!("{id}.codex.json"));
    write_json(&path, &config).map_err(|e| format!("Codex runtime configuration: {e}"))?;
    Ok(LaunchSpec {
        argv: vec![
            std::env::var("MESIMON_CODEX_RUNTIME_BIN")
                .unwrap_or_else(|_| mesimon_bin().display().to_string()),
            "agent-runtime".into(),
            "--config".into(),
            path.display().to_string(),
        ],
        generation: Some(generation),
    })
}

impl AgentAdapter for Codex {
    fn discover(
        &self,
        roots: &[PathBuf],
        known: &dyn Fn(&str) -> bool,
    ) -> Vec<mesimon_core::command::ExternalItem> {
        discovery::scan(&discovery::codex_home(), roots, known)
    }

    fn capabilities(&self) -> super::AgentCapabilities {
        super::AgentCapabilities {
            observation: super::ObservationMode::Structured,
            resume: super::ResumePolicy::ExactOnly,
        }
    }

    fn preview(&self, path: &Path) -> Option<super::AgentPreview> {
        super::read_preview_artifact(path).or_else(|| discovery::read_preview(path))
    }

    fn rows(
        &self,
        at: u64,
        record: &serde_json::Value,
    ) -> Vec<mesimon_core::mesophon::TranscriptRow> {
        discovery::rows(at, record)
    }

    fn conversation_key(&self, record: &mesimon_core::board::SessionRecord) -> Option<String> {
        record.codex_thread_id.clone().filter(|id| !id.is_empty())
    }

    fn history_missing(&self, _record: &mesimon_core::board::SessionRecord) -> bool {
        // Native Codex owns exact-resume validation; never substitute a fresh conversation.
        false
    }

    fn external_owner(&self, record: &mesimon_core::board::SessionRecord) -> Option<ExternalOwner> {
        let identity = record.codex_thread_id.as_deref()?;
        let path =
            record.transcript_path.as_deref().map(Path::new).unwrap_or_else(|| Path::new(""));
        match discovery::external_owner(identity, Path::new(&record.cwd), path) {
            discovery::Ownership::Live(pid) => {
                Some(ExternalOwner { pid: Some(pid), label: format!("Codex process {pid}") })
            }
            discovery::Ownership::Unknown if record.argv.is_empty() => {
                Some(ExternalOwner { pid: None, label: "Codex ownership unverified".into() })
            }
            // An owned session has durable cleanup evidence from its former
            // runtime. Unknown unrelated servers do not erase that evidence;
            // a positively matched external writer still refuses above.
            _ => None,
        }
    }

    fn start(&self, context: &LaunchContext<'_>, _identity: &str) -> Result<LaunchSpec, String> {
        prepare(context, None)
    }

    fn resume(
        &self,
        context: &LaunchContext<'_>,
        record: &mesimon_core::board::SessionRecord,
    ) -> Result<LaunchSpec, String> {
        let identity = record
            .codex_thread_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or("Codex thread identity is unavailable; cannot resume this conversation")?;
        prepare(context, Some(identity))
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Missing legacy evidence never proves that no conversation was created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPhase {
    #[default]
    Unknown,
    BeforeSelection,
    SelectionPending,
    Selected,
}

/// The supervised runtime's current observation, independent of board state.
/// Only the daemon writer applies this evidence to sessions and automation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: uuid::Uuid,
    pub generation: u64,
    pub sequence: u64,
    pub heartbeat_ms: u64,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub launch_phase: LaunchPhase,
    pub turn_id: Option<String>,
    pub state: SessionState,
    pub observation_hold: bool,
    pub history_path: Option<String>,
    /// Native foreground thread name, independent of terminal OSC titles.
    #[serde(default)]
    pub title: Option<String>,
    /// Complete provider plan item, never a stream delta or an approval claim.
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub plan_key: Option<String>,
    /// Durable evidence that this generation and its owned server have stopped.
    #[serde(default)]
    pub stopped: bool,
    /// The app-server's latest `account/rateLimits/updated` params (T-327),
    /// verbatim and bounded, and when they arrived: the session's own quota
    /// report, which spares the daemon a probe while Codex is working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<serde_json::Value>,
    #[serde(default)]
    pub rate_limits_at_ms: u64,
}

/// What a snapshot file looked like when it was last parsed: length, mtime
/// and inode. `write_json` renames a fresh file in, so a rewrite always
/// moves the inode, whatever the filesystem's mtime grain.
type Stamp = (u64, Option<std::time::SystemTime>, u64);

fn stamp_of(path: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok(), meta.ino()))
}

/// The snapshot file at `path`, or `None` where it is missing, oversized or
/// not a snapshot. Lost evidence is the daemon's to judge.
fn read_snapshot(path: &Path) -> Option<Snapshot> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.take(65537).read_to_end(&mut data).ok()?;
    if data.len() > 65536 {
        return None;
    }
    serde_json::from_slice(&data).ok()
}

/// The observer: one worker for the daemon's life (T-688), asked each tick
/// for the live runtimes' snapshot files and answering each ask through
/// `report`. A file is parsed again only when its `Stamp` moved; until then
/// the last parse is reported again, and the daemon judges it by the
/// heartbeat it carries, as it judged every re-parse before. The worker ends
/// with its asker or with the first report nobody takes.
pub fn spawn_observer(
    report: impl Fn(Vec<(uuid::Uuid, Option<Snapshot>)>) -> bool + Send + 'static,
) -> std::sync::mpsc::Sender<Vec<(uuid::Uuid, PathBuf)>> {
    let (ask, asks) = std::sync::mpsc::channel::<Vec<(uuid::Uuid, PathBuf)>>();
    std::thread::spawn(move || {
        let mut seen: std::collections::HashMap<PathBuf, (Stamp, Option<Snapshot>)> =
            std::collections::HashMap::new();
        while let Ok(paths) = asks.recv() {
            seen.retain(|p, _| paths.iter().any(|(_, q)| q == p));
            let snapshots = paths
                .into_iter()
                .map(|(id, path)| {
                    let snapshot = match (stamp_of(&path), seen.get(&path)) {
                        (Some(stamp), Some((was, kept))) if *was == stamp => kept.clone(),
                        (Some(stamp), _) => {
                            let fresh = read_snapshot(&path);
                            seen.insert(path, (stamp, fresh.clone()));
                            fresh
                        }
                        (None, _) => {
                            seen.remove(&path);
                            None
                        }
                    };
                    (id, snapshot)
                })
                .collect();
            if !report(snapshots) {
                break;
            }
        }
    });
    ask
}

/// Why a socket probe could not be made: a path no `sockaddr_un` holds,
/// or the socket or its nonblocking flag refused.
pub(crate) enum ProbeFail {
    Path,
    Socket(std::io::Error),
    NonBlock(std::io::Error),
}

/// A nonblocking connect to the Unix socket at `path`, sending nothing: the
/// inner result is the connect's, so a full accept backlog is never waited
/// on. Both of Codex's liveness checks ask through this one `unsafe` block.
pub(crate) fn connect_probe(path: &Path) -> Result<std::io::Result<()>, ProbeFail> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(ProbeFail::Path);
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of_val(&address) as u8;
    }
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(ProbeFail::Socket(std::io::Error::last_os_error()));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        return Err(ProbeFail::NonBlock(std::io::Error::last_os_error()));
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    Ok(if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) })
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use mesimon_backend_tmux::InputScreen;

    fn screen(text: &str, cursor: Option<(usize, usize)>) -> InputScreen {
        InputScreen { lines: text.lines().map(str::to_string).collect(), cursor }
    }

    #[test]
    fn input_cursor_is_independent_of_every_status_line() {
        for footer in [
            "",
            "? for shortcuts",
            "100% context left",
            "my arbitrary status",
            "mesimon · gpt-6-astra high · Context 0% used · weekly 42% left",
            "gpt-6-astra high · /repo · main · Ready",
            "› custom status",
            "Hooks need review",
        ] {
            assert!(
                input_ready(&screen(
                    &format!("› Ask Codex to do anything\n{footer}"),
                    Some((2, 0))
                )),
                "{footer}"
            );
        }
        assert!(input_ready(&screen("› multiline\n  continuation\n\n  last line", Some((11, 3)))));
    }

    #[test]
    fn input_requires_the_active_cursor_inside_the_composer() {
        assert!(!input_ready(&screen("› text\n? for shortcuts", None)));
        assert!(!input_ready(&screen("› text", Some((0, 0)))));
        assert!(!input_ready(&screen("› text\nloading", Some((7, 1)))));
        assert!(!input_ready(&screen("› text", Some((2, 5)))));
        assert!(!input_ready(&screen("loading", Some((2, 0)))));
        // A stale composer above a different text field is not its cursor.
        assert!(!input_ready(&screen("› old prompt\nSearch sessions\n  query", Some((7, 2)))));
        for modal in
            ["Hooks need review", "Do you trust", "Select a model", "An unknown future dialog"]
        {
            assert!(!input_ready(&screen(&format!("› text\n{modal}\n› 1. Yes"), None)));
        }
    }

    #[test]
    fn plan_modal_is_distinct_from_plan_text_and_native_composer() {
        let modal = screen("Implement this plan?\n› 1. Yes, implement this plan\n  2. Yes, clear context and implement\n  3. No, stay in Plan mode\nPress enter to confirm or esc to go back", None);
        assert!(plan_dialog(&modal));
        assert!(!input_ready(&modal));
        let composer = screen("Implement this plan?\n› 1. Yes, implement this plan\n  3. No, stay in Plan mode\n› Continue planning", Some((2, 3)));
        assert!(!plan_dialog(&composer));
        assert!(input_ready(&composer));
        assert!(!plan_dialog(&screen("The answer is: Implement this plan?", None)));
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use mesimon_core::board::{ClaudeMode, CodexApproval, CodexSandbox, ColumnSettings};

    #[test]
    fn inherited_codex_policy_emits_nothing_even_with_claude_plan_mode() {
        let column = ColumnSettings { claude_mode: ClaudeMode::Plan, ..Default::default() };
        assert!(policy_flags(&column).is_empty());
    }

    #[test]
    fn explicit_policies_use_native_config_and_preserve_independence() {
        let mut column = ColumnSettings {
            codex_sandbox: CodexSandbox::ReadOnly,
            codex_approval: CodexApproval::OnRequest,
            ..Default::default()
        };
        assert_eq!(
            policy_flags(&column),
            ["-c", "sandbox_mode=\"read-only\"", "-c", "approval_policy=\"on-request\""]
        );
        column.codex_sandbox = CodexSandbox::WorkspaceWrite;
        column.codex_approval = CodexApproval::Never;
        assert_eq!(
            policy_flags(&column),
            ["-c", "sandbox_mode=\"workspace-write\"", "-c", "approval_policy=\"never\""]
        );
        column.codex_sandbox = CodexSandbox::Inherit;
        assert_eq!(policy_flags(&column), ["-c", "approval_policy=\"never\""]);
    }

    /// T-443: a Codex tier is two config keys; a Claude tier, and a
    /// built-in, are none.
    #[test]
    fn a_codex_tier_is_model_and_reasoning_effort_config() {
        use mesimon_core::board::AgentProvider;
        use mesimon_core::tier::{Effort, Tier};
        let reviewer = Tier {
            id: "A".into(),
            name: "reviewer".into(),
            provider: AgentProvider::Codex,
            model: "gpt-6-astra".into(),
            effort: Effort::High,
            description: String::new(),
        };
        assert_eq!(
            tier_flags(&reviewer),
            ["-c", "model=\"gpt-6-astra\"", "-c", "model_reasoning_effort=\"high\""]
        );
        assert!(tier_flags(&Tier::builtin(AgentProvider::Codex)).is_empty());
        let claude = Tier { provider: AgentProvider::ClaudeCode, ..reviewer };
        assert!(tier_flags(&claude).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(sequence: u64) -> Snapshot {
        Snapshot {
            session: uuid::Uuid::nil(),
            generation: 1,
            sequence,
            heartbeat_ms: 1_000 * sequence,
            thread_id: None,
            launch_phase: LaunchPhase::default(),
            turn_id: None,
            state: SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown },
            observation_hold: false,
            history_path: None,
            title: None,
            plan: None,
            plan_key: None,
            stopped: false,
            rate_limits: None,
            rate_limits_at_ms: 0,
        }
    }

    /// T-688: an unchanged file is reported from the last parse, a rewrite
    /// is read again, and a file that went is `None`.
    #[test]
    fn observer_reparses_only_a_moved_snapshot() {
        let dir = std::env::temp_dir().join(format!("msmn-cdx-observer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cdx.json");
        let id = uuid::Uuid::new_v4();
        write_json(&path, &snapshot(1)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let ask = spawn_observer(move |s| tx.send(s).is_ok());
        let round = || {
            ask.send(vec![(id, path.clone())]).unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().remove(0).1
        };
        assert_eq!(round().map(|s| s.sequence), Some(1));
        // Same bytes, same stamp: the parse is kept.
        assert_eq!(round().map(|s| s.sequence), Some(1));
        // A rewrite of the same length moves the inode: read again.
        write_json(&path, &snapshot(2)).unwrap();
        assert_eq!(round().map(|s| s.sequence), Some(2));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(round(), None);
        write_json(&path, &snapshot(3)).unwrap();
        assert_eq!(round().map(|s| s.sequence), Some(3));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
