//! Claude Code flags, settings and exact conversation selection.

pub mod composer;
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
    // One launch in plan mode (T-434) outranks the column's word, which
    // outranks the user's own default — the same flag either way, and the
    // TUI reads it back off the argv as the one fact it holds about a
    // session's mode (`ticket_planning`).
    let mode = if context.plan {
        Some("plan".to_string())
    } else {
        context.column.claude_mode.flag_word().map(str::to_string).or_else(user_default_mode)
    };
    if let Some(mode) = mode {
        argv.extend(["--permission-mode".into(), mode]);
    }
    // T-573's research seam: a mod of function hooks, by flag alone (promise
    // 1: no config is written for it). A wake re-reads the seam like a tier.
    if let Some(dir) = &context.mod_dir {
        argv.extend(["--plugin-dir".into(), dir.display().to_string()]);
    }
    // The ticket's tier (T-443). Only a Claude tier's words reach here —
    // `Book::launch` hands another provider's pick this provider's built-in,
    // which passes nothing — and only words the tier's own checks accept.
    if context.tier.provider == mesimon_core::board::AgentProvider::ClaudeCode {
        if let Some(model) = context.tier.model_arg() {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = context.tier.effort_arg() {
            argv.extend(["--effort".into(), effort.into()]);
        }
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
            // The tier's pair (T-443): a wake re-reads the ticket's tier, so
            // a switch lands and a replayed argv never carries two.
            "--model",
            "--effort",
            "--plugin-dir",
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

#[cfg(test)]
mod tier_tests {
    use super::*;
    use mesimon_core::board::{AgentProvider, ColumnSettings, SessionKind, SessionState};
    use mesimon_core::tier::{Effort, Tier};

    fn context(paths: &crate::paths::Paths, tier: Tier) -> LaunchContext<'_> {
        LaunchContext {
            paths,
            cwd: std::path::Path::new("/"),
            session: uuid::Uuid::from_u128(1),
            tools: AgentTools::Off,
            brief: false,
            column: ColumnSettings::default(),
            plan: false,
            tier,
            mod_dir: None,
        }
    }

    fn pair<'a>(argv: &'a [String], flag: &str) -> Vec<&'a str> {
        argv.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
    }

    /// T-443: a tier's model and effort ride argv, a wake re-reads the
    /// ticket's tier instead of replaying the old pair, and the built-in
    /// passes nothing at all.
    #[test]
    fn a_wake_carries_the_tier_it_is_woken_on_and_never_two() {
        let dir = std::env::temp_dir().join(format!("msmn-claude-tier-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = crate::paths::Paths::for_repo(&dir).unwrap();
        let tier = |model: &str, effort| Tier {
            id: "A".into(),
            name: "coder".into(),
            provider: AgentProvider::ClaudeCode,
            model: model.into(),
            effort,
        };
        let started = flags(&context(&paths, tier("opus", Effort::Xhigh)));
        assert_eq!(pair(&started, "--model"), ["opus"]);
        assert_eq!(pair(&started, "--effort"), ["xhigh"]);

        let mut rec = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            ["claude", "--settings", "/s.json", "--model", "opus", "--effort", "xhigh"]
                .into_iter()
                .map(String::from)
                .chain(["--session-id".to_string(), uuid::Uuid::from_u128(1).to_string()])
                .collect(),
            "/".into(),
            SessionState::Sleeping,
        );
        let woke = Claude.resume(&context(&paths, tier("sonnet", Effort::High)), &rec).unwrap();
        assert_eq!(pair(&woke.argv, "--model"), ["sonnet"]);
        assert_eq!(pair(&woke.argv, "--effort"), ["high"]);
        assert_eq!(pair(&woke.argv, "--resume"), [uuid::Uuid::from_u128(1).to_string()]);

        rec.argv = woke.argv;
        let plain = Claude
            .resume(&context(&paths, Tier::builtin(AgentProvider::ClaudeCode)), &rec)
            .unwrap();
        assert!(!plain.argv.iter().any(|a| a == "--model" || a == "--effort"), "{:?}", plain.argv);

        // Another provider's tier never reaches a claude's argv.
        let codex = Tier { provider: AgentProvider::Codex, ..tier("gpt-6-astra", Effort::Ultra) };
        let argv = flags(&context(&paths, codex));
        assert!(!argv.iter().any(|a| a == "--model" || a == "--effort"), "{argv:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// T-573: the research seam rides argv only while it is set, and a wake
    /// drops a stale `--plugin-dir` pair the way it drops a stale tier.
    #[test]
    fn the_mod_seam_is_off_by_default_and_re_read_on_a_wake() {
        let dir = std::env::temp_dir().join(format!("msmn-claude-mod-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = crate::paths::Paths::for_repo(&dir).unwrap();
        let builtin = || Tier::builtin(AgentProvider::ClaudeCode);
        let off = flags(&context(&paths, builtin()));
        assert!(!off.iter().any(|a| a == "--plugin-dir"), "{off:?}");
        let on = flags(&LaunchContext {
            mod_dir: Some("/state/mod".into()),
            ..context(&paths, builtin())
        });
        assert_eq!(pair(&on, "--plugin-dir"), ["/state/mod"]);

        let rec = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            ["claude", "--settings", "/s.json", "--plugin-dir", "/old/mod"]
                .into_iter()
                .map(String::from)
                .chain(["--session-id".to_string(), uuid::Uuid::from_u128(1).to_string()])
                .collect(),
            "/".into(),
            SessionState::Sleeping,
        );
        let woke = Claude.resume(&context(&paths, builtin()), &rec).unwrap();
        assert!(
            !woke.argv.iter().any(|a| a == "--plugin-dir" || a == "/old/mod"),
            "{:?}",
            woke.argv
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
