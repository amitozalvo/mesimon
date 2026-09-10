//! Native Codex integration: protocol observation and lifecycle stay here.

pub mod discovery;
pub mod observation;
pub mod rpc;
pub mod runtime;

use mesimon_core::board::SessionState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{AgentAdapter, LaunchContext, LaunchSpec};
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

/// Input readiness is separate from turn state. A thread can exist while
/// native startup trust is still on screen. Only the provider examines its
/// native footer, and never presses Enter into an observed modal.
pub fn input_ready(lines: &[String]) -> bool {
    let Some(prompt) = lines.iter().rposition(|line| line.trim_start().starts_with('›')) else {
        return false;
    };
    let footer = lines
        .iter()
        .enumerate()
        .skip(prompt + 1)
        .find(|(_, line)| {
            line.contains("? for shortcuts") || line.contains("% context left")
            // Native user-configured status lines can replace the default
            // footer. The model/cwd/status layout is also an input footer.
            || (line.contains(" · ") && line.contains('/') && !line.trim_start().starts_with('›'))
        })
        .map(|(index, _)| index);
    let blocked = [
        "Press enter to continue",
        "Hooks need review",
        "Would you like",
        "Sign in with ChatGPT",
        "Do you trust",
        "Select a model",
        "Review hooks",
    ];
    footer.is_some_and(|footer| {
        !lines.iter().skip(footer).any(|line| blocked.iter().any(|marker| line.contains(marker)))
    })
}

pub fn startup_attention(lines: &[String]) -> Option<mesimon_core::board::Reason> {
    use mesimon_core::board::Reason;
    if input_ready(lines) {
        return None;
    }
    let text = lines.join("\n");
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

fn prepare(context: &LaunchContext<'_>, resume: Option<String>) -> Result<LaunchSpec, String> {
    let id = context.session;
    let generation = (uuid::Uuid::new_v4().as_u128() as u64).max(1);
    let stem = format!("cdx-{}-{:08x}", &id.simple().to_string()[..16], generation as u32);
    let mut config_flags = policy_flags(&context.column);
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

    fn conversation_key(&self, record: &mesimon_core::board::SessionRecord) -> Option<String> {
        record.codex_thread_id.clone().filter(|id| !id.is_empty())
    }

    fn history_missing(&self, _record: &mesimon_core::board::SessionRecord) -> bool {
        // Native Codex owns exact-resume validation; never substitute a fresh conversation.
        false
    }

    fn external_owner(&self, record: &mesimon_core::board::SessionRecord) -> Option<String> {
        let identity = record.codex_thread_id.as_deref()?;
        let path =
            record.transcript_path.as_deref().map(Path::new).unwrap_or_else(|| Path::new(""));
        match discovery::external_owner(identity, Path::new(&record.cwd), path) {
            discovery::Ownership::Live(pid) => Some(format!("Codex process {pid}")),
            discovery::Ownership::Unknown if record.argv.is_empty() => {
                Some("Codex ownership unverified".into())
            }
            // An owned session has durable cleanup evidence from its former
            // runtime. Unknown unrelated servers do not erase that evidence;
            // a positively matched external writer still refuses above.
            _ => None,
        }
    }

    fn normalize_title(&self, title: &str) -> String {
        mesimon_core::text::scrub_cells(title, false).chars().take(80).collect()
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

/// The supervised runtime's current observation, independent of board state.
/// Only the daemon writer applies this evidence to sessions and automation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: uuid::Uuid,
    pub generation: u64,
    pub sequence: u64,
    pub heartbeat_ms: u64,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub state: SessionState,
    pub observation_hold: bool,
    pub history_path: Option<String>,
    /// Complete provider plan item, never a stream delta or an approval claim.
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub plan_key: Option<String>,
    /// Durable evidence that this generation and its owned server have stopped.
    #[serde(default)]
    pub stopped: bool,
}

#[cfg(test)]
mod input_tests {
    use super::*;
    fn screen(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }
    #[test]
    fn input_footer_accepts_default_and_native_custom_status_but_not_modals() {
        assert!(input_ready(&screen("› Ask Codex to do anything\n\n? for shortcuts")));
        assert!(input_ready(&screen("› hello\n\ngpt-6-astra high · /repo · main · Ready")));
        assert!(!input_ready(&screen(
            "› Ask Codex to do anything\n? for shortcuts\nHooks need review\n› Review hooks"
        )));
        assert!(!input_ready(&screen("Select a model\n› gpt-6-astra\nPress enter to continue")));
        assert!(!input_ready(&screen("Would you like to run this command?\n› 1. Yes")));
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
}
