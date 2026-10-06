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
use crate::hook_settings::{self, mesimon_bin, HookSet};

pub struct Claude;

/// Read only; preserve the same config-home precedence as the census.
/// Every snapshot asks, so the parse is kept until the file's stamp moves.
pub fn user_default_mode() -> Option<String> {
    type Stamp = (std::path::PathBuf, Option<std::time::SystemTime>, u64);
    static SEEN: std::sync::Mutex<Option<(Stamp, Option<String>)>> = std::sync::Mutex::new(None);
    let path = crate::census::claude_home().join("settings.json");
    let meta = std::fs::metadata(&path).ok();
    let stamp = (path, meta.as_ref().and_then(|m| m.modified().ok()), meta.map_or(0, |m| m.len()));
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((was, mode)) = seen.as_ref() {
        if *was == stamp {
            return mode.clone();
        }
    }
    let mode = read_default_mode(&stamp.0);
    *seen = Some((stamp, mode.clone()));
    mode
}

fn read_default_mode(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
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
    // On the mod road the mod registers the tools itself (T-577): no MCP
    // server is named, and a registered tool goes through no permission
    // check, so there is no allow rule to pass either.
    let shim = context.road != mesimon_core::road::Road::Mod;
    if context.tools != AgentTools::Off {
        if shim {
            argv.extend([
                "--mcp-config".into(),
                hook_settings::mcp_config_json(
                    context.paths,
                    &mesimon_bin(),
                    context.session,
                    context.tools,
                ),
            ]);
        }
        if context.brief {
            argv.extend([mesimon_core::brief::FLAG.into(), mesimon_core::brief::TEXT.into()]);
        }
        // The read tools run unasked (T-362): without an allow rule Claude
        // Code prompts for every MCP tool, so plan mode asked for
        // `get_ticket` on every turn. Argv, never a settings file — promise 2.
        let allowed = mesimon_core::mcp::allowed_tool_names(context.tools);
        if shim && !allowed.is_empty() {
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

/// The hook set's `--settings` pair, written for this session: the whole
/// set on the hook set's road, and the permission entries alone beside a
/// native mod (T-658), where Claude Code keeps the hook events from the mod
/// and the mod reports from its own. On the mod road otherwise (T-577) the
/// mod relays every event the set reported, holds the gate and runs
/// `mesimon approve`, so no settings file is written and none is passed.
fn hook_set(context: &LaunchContext<'_>) -> Result<Vec<String>, String> {
    let (paths, session, bin) = (context.paths, context.session, mesimon_bin());
    let settings = match context.hook_set {
        HookSet::None => return Ok(Vec::new()),
        HookSet::Full => hook_settings::write_settings(paths, session, &bin),
        HookSet::PermissionOnly => hook_settings::write_permission_settings(paths, session, &bin),
    }
    .map_err(|e| format!("hook settings: {e}"))?;
    Ok(vec!["--settings".into(), settings.display().to_string()])
}

fn launch(
    context: &LaunchContext<'_>,
    identity_flag: &str,
    identity: &str,
) -> Result<Vec<String>, String> {
    let executable = std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
    let mut argv = vec![executable];
    argv.extend(hook_set(context)?);
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
        } else {
            // The native road names no path (T-657): its `SessionStart`
            // carries the id, and the file is found under Claude's projects
            // root the way the census walks it (T-669). Claude Code writes
            // nothing before the first prompt, so the walk repeats on each
            // frame until the file is there. A new id (`/clear`, an
            // in-session `/resume`) is a new conversation: the path the
            // record knew was the last one's.
            if let Some(identity) = hooks::identity_of(frame) {
                let learned = (identity != record.id).then_some(identity);
                if record.claude_session_id != learned {
                    record.claude_session_id = learned;
                    record.transcript_path = None;
                    result.metadata_changed = true;
                }
            }
            if record.kind == mesimon_core::board::SessionKind::Claude
                && record.transcript_path.is_none()
            {
                let identity = record.claude_session_id.unwrap_or(record.id);
                let projects = crate::census::claude_home().join("projects");
                if let Some(found) = history::locate(identity, &projects) {
                    record.transcript_path = Some(found.display().to_string());
                    result.metadata_changed = true;
                }
            }
        }
        result.signal = hooks::signal_with_background(frame, &mut record.background_tasks);
        result.detail = hooks::detail_of(frame);
        result.plan = hooks::plan_of(frame);
        // A native failed turn names no class (T-659): the transcript's
        // error row does. Not flushed yet, `unknown` stands and the
        // recovery's re-read corrects it.
        if hooks::failure_unsaid(frame) {
            if let Some(error) = record
                .transcript_path
                .as_deref()
                .and_then(|p| tail::api_error(std::path::Path::new(p)))
            {
                result.signal = Some(mesimon_core::attention::Signal::StopFailure {
                    class: hooks::failure_class(Some(&error.class)),
                });
                result.detail = error.text.as_deref().and_then(hooks::failure_detail);
            }
        }
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

    fn rows(
        &self,
        at: u64,
        record: &serde_json::Value,
    ) -> Vec<mesimon_core::mesophon::TranscriptRow> {
        history::rows(at, record)
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
        // The hook set's pair is one (T-577): a wake re-decides the road, so
        // one launched on the hook set may wake on the mod and the reverse.
        let owned = [
            "--settings",
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
        let at = argv.len().min(1);
        let mut ours = hook_set(context)?;
        ours.extend(flags(context));
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
            road: mesimon_core::road::Road::Hooks,
            hook_set: HookSet::Full,
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
            // T-584: the person's words are the crown's to read and never
            // reach argv, whatever they say.
            description: "--dangerously-skip-permissions".into(),
        };
        let started = flags(&context(&paths, tier("opus", Effort::Xhigh)));
        assert!(!started.iter().any(|a| a.contains("dangerously")), "{started:?}");
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

    /// T-577: on the mod road the tools are the mod's registration, so no
    /// MCP server and no allow rule ride argv; the opt-in brief still does.
    /// A wake that lands on the mod drops a hook-road launch's pair.
    #[test]
    fn the_mod_road_carries_no_mcp_config_and_no_allow_rule() {
        use mesimon_core::road::Road;
        let dir = std::env::temp_dir().join(format!("msmn-claude-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = crate::paths::Paths::for_repo(&dir).unwrap();
        let on = |road| LaunchContext {
            tools: AgentTools::Full,
            brief: true,
            road,
            hook_set: if road == Road::Hooks { HookSet::Full } else { HookSet::None },
            ..context(&paths, Tier::builtin(AgentProvider::ClaudeCode))
        };
        let hooks = flags(&on(Road::Hooks));
        assert_eq!(pair(&hooks, "--mcp-config").len(), 1, "{hooks:?}");
        assert_eq!(pair(&hooks, "--allowedTools").len(), 1, "{hooks:?}");
        let modded = flags(&on(Road::Mod));
        assert!(pair(&modded, "--mcp-config").is_empty(), "{modded:?}");
        assert!(pair(&modded, "--allowedTools").is_empty(), "{modded:?}");
        assert_eq!(pair(&modded, mesimon_core::brief::FLAG), [mesimon_core::brief::TEXT]);

        let mut argv = vec!["claude".to_string()];
        argv.extend(hooks);
        argv.extend(["--session-id".to_string(), uuid::Uuid::from_u128(1).to_string()]);
        let rec = SessionRecord::new(
            uuid::Uuid::from_u128(1),
            SessionKind::Claude,
            ulid::Ulid(1),
            argv,
            "/".into(),
            SessionState::Sleeping,
        );
        let woke = Claude.resume(&on(Road::Mod), &rec).unwrap();
        for flag in ["--mcp-config", "--allowedTools", "--settings"] {
            assert!(pair(&woke.argv, flag).is_empty(), "{flag}: {:?}", woke.argv);
        }
        // T-658: where Claude Code keeps the hook events from the mod, the
        // permission entries alone ride beside it, and the tools stay the
        // mod's. The file is rewritten for the road each launch takes.
        let read = |argv: &[String]| -> serde_json::Value {
            let path = pair(argv, "--settings")[0];
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let events = |v: &serde_json::Value| v["hooks"].as_object().unwrap().len();
        let native = LaunchContext {
            hook_set: HookSet::PermissionOnly,
            mod_dir: Some("/m".into()),
            ..on(Road::Mod)
        };
        let both = Claude.resume(&native, &rec).unwrap();
        assert_eq!(pair(&both.argv, "--settings").len(), 1, "{:?}", both.argv);
        assert_eq!(pair(&both.argv, "--plugin-dir"), ["/m"]);
        for flag in ["--mcp-config", "--allowedTools"] {
            assert!(pair(&both.argv, flag).is_empty(), "{flag}: {:?}", both.argv);
        }
        let file = read(&both.argv);
        assert_eq!(events(&file), 1, "{file}");
        assert!(file["hooks"].get("PermissionRequest").is_some(), "{file}");
        let whole = Claude.resume(&on(Road::Hooks), &rec).unwrap();
        assert_eq!(pair(&whole.argv, "--settings"), pair(&both.argv, "--settings"));
        assert!(events(&read(&whole.argv)) > 1, "the hook set's road gets the whole set");
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

#[cfg(test)]
mod failure_tests {
    use super::*;
    use mesimon_core::attention::{Signal, StopFailureClass};
    use mesimon_core::board::{SessionKind, SessionState};

    fn stop_failure(payload: serde_json::Value, reason: &str) -> crate::ingest::HookFrame {
        let header =
            serde_json::json!({"v": 1, "session": "s", "event": "StopFailure", "reason": reason});
        let bytes = format!("{header}\n{payload}");
        crate::ingest::parse_frame(bytes.as_bytes()).unwrap()
    }

    fn native() -> crate::ingest::HookFrame {
        stop_failure(
            serde_json::json!({"session_id": "x", "error": "unknown", "native": true}),
            "unknown",
        )
    }

    fn record_on(rows: &[serde_json::Value]) -> (SessionRecord, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("msmn-failure-{}.jsonl", uuid::Uuid::new_v4()));
        let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let mut record = SessionRecord::new(
            uuid::Uuid::new_v4(),
            SessionKind::Claude,
            ulid::Ulid(1),
            vec!["claude".into()],
            "/repo".into(),
            SessionState::Running,
        );
        record.transcript_path = Some(path.display().to_string());
        (record, path)
    }

    fn prompt() -> serde_json::Value {
        serde_json::json!({"uuid": "p", "type": "user", "message": {"content": "go"}})
    }

    fn session_start(source: &str, session_id: uuid::Uuid) -> crate::ingest::HookFrame {
        let header = serde_json::json!({"v": 1, "session": "s", "event": "SessionStart",
            "reason": source});
        let payload = serde_json::json!({"session_id": session_id.to_string(), "cwd": "/repo",
            "source": source});
        let bytes = format!("{header}\n{payload}");
        crate::ingest::parse_frame(bytes.as_bytes()).unwrap()
    }

    /// T-669: a native `SessionStart` names no path, only the id. The
    /// launch's own id teaches nothing; a `/clear`'s new id is a new
    /// conversation, so the record learns it and drops the path it knew
    /// (the walk for the new file is the projects root's, `history::locate`).
    #[test]
    fn a_native_session_start_teaches_its_id_and_a_new_id_drops_the_known_path() {
        let (mut record, path) = record_on(&[prompt()]);
        let known = record.transcript_path.clone();
        let seen = Claude.parse_hook(&session_start("startup", record.id), &mut record);
        assert!(!seen.metadata_changed, "the launch's own id is known");
        assert_eq!(record.claude_session_id, None);
        assert_eq!(record.transcript_path, known, "the path stands");

        let cleared = uuid::Uuid::new_v4();
        let seen = Claude.parse_hook(&session_start("clear", cleared), &mut record);
        assert!(seen.metadata_changed);
        assert_eq!(record.claude_session_id, Some(cleared));
        assert_eq!(record.transcript_path, None, "the known file was the last conversation's");

        let seen = Claude.parse_hook(&session_start("compact", cleared), &mut record);
        assert!(!seen.metadata_changed, "the same id again changes nothing");
        assert_eq!(record.claude_session_id, Some(cleared));
        let _ = std::fs::remove_file(path);
    }

    /// T-659: every class the hook set's matcher names is read off the
    /// error row in the same word, with its text for the card.
    #[test]
    fn a_native_failure_reads_its_class_and_message_off_the_transcript() {
        let classes = [
            ("rate_limit", StopFailureClass::RateLimit),
            ("overloaded", StopFailureClass::Overloaded),
            ("authentication_failed", StopFailureClass::AuthenticationFailed),
            ("oauth_org_not_allowed", StopFailureClass::OauthOrgNotAllowed),
            ("billing_error", StopFailureClass::BillingError),
            ("invalid_request", StopFailureClass::InvalidRequest),
            ("model_not_found", StopFailureClass::ModelNotFound),
            ("max_output_tokens", StopFailureClass::MaxOutputTokens),
            ("server_error", StopFailureClass::ServerError),
            ("unknown", StopFailureClass::Unknown),
        ];
        for (word, class) in classes {
            let text = format!("API Error: {word}");
            let (mut record, path) = record_on(&[prompt(), tail::error_row(word, &text)]);
            let seen = Claude.parse_hook(&native(), &mut record);
            assert_eq!(seen.signal, Some(Signal::StopFailure { class }), "{word}");
            assert_eq!(seen.detail.as_deref(), Some(text.as_str()), "{word}");
            let _ = std::fs::remove_file(path);
        }
    }

    /// Not flushed yet: `unknown` stands (the recovery's re-read corrects
    /// it). A frame naming its class — the hook set's — is never re-read,
    /// whatever the transcript says.
    #[test]
    fn an_unwritten_row_keeps_unknown_and_a_said_class_is_untouched() {
        let (mut record, path) = record_on(&[prompt()]);
        let seen = Claude.parse_hook(&native(), &mut record);
        assert_eq!(seen.signal, Some(Signal::StopFailure { class: StopFailureClass::Unknown }));
        assert_eq!(seen.detail.as_deref(), Some("unknown"));
        let _ = std::fs::remove_file(path);

        let (mut record, path) =
            record_on(&[prompt(), tail::error_row("server_error", "API Error: 500")]);
        let hooked = stop_failure(
            serde_json::json!({"session_id": "x", "error": "rate_limit",
                "last_assistant_message": "API Error: Rate limit reached"}),
            "rate_limit",
        );
        let seen = Claude.parse_hook(&hooked, &mut record);
        assert_eq!(seen.signal, Some(Signal::StopFailure { class: StopFailureClass::RateLimit }));
        assert_eq!(seen.detail.as_deref(), Some("API Error: Rate limit reached"));
        let _ = std::fs::remove_file(path);
    }
}
