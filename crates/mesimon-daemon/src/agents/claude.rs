//! Claude Code flags, settings and exact conversation selection.

mod history;
pub mod hooks;
mod recovery;
pub mod tail;

use mesimon_core::board::{AgentTools, SessionRecord};

use super::{
    AgentAdapter, AgentCapabilities, AgentPreview, ExternalOwner, LaunchContext, LaunchSpec,
    ObservationMode, ResumePolicy,
};
use crate::hook_settings::{self, mesimon_bin};

pub struct Claude;

/// Read only; preserve the same config-home precedence as the census.
pub fn user_default_mode() -> Option<String> {
    let home = crate::census::claude_home();
    let text = std::fs::read_to_string(home.join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("permissions")
        .and_then(|p| p.get("defaultMode"))
        .or_else(|| value.get("defaultMode"))
        .and_then(|m| m.as_str())
        .map(str::to_string)
}

fn flags(context: &LaunchContext<'_>) -> Vec<String> {
    let mut argv = Vec::new();
    if context.tools != AgentTools::Off {
        argv.extend([
            "--mcp-config".into(),
            hook_settings::mcp_config_json(
                context.paths,
                &mesimon_bin(),
                context.session,
                context.tools,
            ),
        ]);
        if context.brief {
            argv.extend([mesimon_core::brief::FLAG.into(), mesimon_core::brief::TEXT.into()]);
        }
        // The read tools run unasked (T-362): without an allow rule Claude
        // Code prompts for every MCP tool, so plan mode asked for
        // `get_ticket` on every turn. Argv, never a settings file — promise 2.
        let allowed = mesimon_core::mcp::allowed_tool_names(context.tools);
        if !allowed.is_empty() {
            argv.extend(["--allowedTools".into(), allowed.join(",")]);
        }
    }
    if let Some(mode) =
        context.column.claude_mode.flag_word().map(str::to_string).or_else(user_default_mode)
    {
        argv.extend(["--permission-mode".into(), mode]);
    }
    argv
}

fn launch(
    context: &LaunchContext<'_>,
    identity_flag: &str,
    identity: &str,
) -> Result<Vec<String>, String> {
    let settings = hook_settings::write_settings(context.paths, context.session, &mesimon_bin())
        .map_err(|e| format!("hook settings: {e}"))?;
    let executable = std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let mut argv = vec![executable, "--settings".into(), settings.display().to_string()];
    argv.extend(flags(context));
    argv.extend([identity_flag.into(), identity.into()]);
    Ok(argv)
}

impl AgentAdapter for Claude {
    fn recovery(&self) -> Box<dyn super::AgentRecovery> {
        Box::new(recovery::ClaudeRecovery::default())
    }

    fn discover(
        &self,
        roots: &[std::path::PathBuf],
        known: &dyn Fn(&str) -> bool,
    ) -> Vec<mesimon_core::command::ExternalItem> {
        crate::census::scan(&crate::census::claude_home(), roots, &|id| known(&id.to_string()))
    }

    fn parse_hook(
        &self,
        frame: &crate::ingest::HookFrame,
        record: &mut SessionRecord,
    ) -> super::HookObservation {
        let mut result = super::HookObservation::default();
        if let Some(path) = hooks::transcript_of(frame) {
            if record.transcript_path.as_ref() != Some(&path) {
                record.transcript_path = Some(path.clone());
                result.metadata_changed = true;
            }
            if let Some(identity) = std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<uuid::Uuid>().ok())
            {
                let learned = (identity != record.id).then_some(identity);
                if record.claude_session_id != learned {
                    record.claude_session_id = learned;
                    result.metadata_changed = true;
                }
            }
        } else if let Some(path) = hooks::transcript_moved(frame, record) {
            record.transcript_path = Some(path);
            result.metadata_changed = true;
        }
        result.signal = hooks::signal_with_background(frame, &mut record.background_tasks);
        result.detail = hooks::detail_of(frame);
        result.plan = hooks::plan_of(frame);
        result
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            observation: ObservationMode::Hooks,
            resume: ResumePolicy::FreshWhenHistoryMissing,
        }
    }

    fn preview(&self, path: &std::path::Path) -> Option<AgentPreview> {
        history::latest_preview(path)
    }

    fn conversation_key(&self, record: &SessionRecord) -> Option<String> {
        Some(record.claude_session_id.unwrap_or(record.id).to_string())
    }

    fn history_missing(&self, record: &SessionRecord) -> bool {
        history::missing(record, &crate::census::claude_home().join("projects"))
    }

    fn external_owner(&self, record: &SessionRecord) -> Option<ExternalOwner> {
        let identity = record.claude_session_id.unwrap_or(record.id);
        crate::census::running_pid_for(&crate::census::claude_home(), identity)
            .map(|pid| ExternalOwner { pid: Some(pid), label: format!("pid {pid}") })
    }

    fn normalize_title(&self, title: &str) -> String {
        let clean: String =
            mesimon_core::text::scrub_cells(title, false).chars().take(80).collect();
        clean
            .trim_start_matches(|c: char| {
                matches!(c, '✳' | '✻' | '✽' | '✶' | '✢' | '*' | '·') || c.is_whitespace()
            })
            .to_string()
    }

    fn start(&self, context: &LaunchContext<'_>, identity: &str) -> Result<LaunchSpec, String> {
        launch(context, "--session-id", identity).map(LaunchSpec::plain)
    }

    fn resume(
        &self,
        context: &LaunchContext<'_>,
        record: &SessionRecord,
    ) -> Result<LaunchSpec, String> {
        let identity = record.claude_session_id.unwrap_or(record.id).to_string();
        if record.argv.is_empty() {
            return launch(context, "--resume", &identity).map(LaunchSpec::plain);
        }
        // Retain user flags while refreshing every pair Mesimon owns. An
        // in-app /resume can change identity without changing the saved argv.
        let owned = [
            "--session-id",
            "--resume",
            "--mcp-config",
            "--allowedTools",
            mesimon_core::brief::FLAG,
            "--permission-mode",
        ];
        let mut argv = Vec::with_capacity(record.argv.len() + 2);
        let mut previous = record.argv.iter();
        while let Some(arg) = previous.next() {
            if owned.contains(&arg.as_str()) {
                let _ = previous.next();
            } else {
                argv.push(arg.clone());
            }
        }
        let at = argv
            .iter()
            .position(|a| a == "--settings")
            .map(|i| (i + 2).min(argv.len()))
            .unwrap_or(argv.len().min(1));
        let mut ours = flags(context);
        ours.extend(["--resume".into(), identity]);
        argv.splice(at..at, ours);
        Ok(LaunchSpec::plain(argv))
    }
}
