//! agy, Google's Antigravity CLI (T-1225): its launch files, its command
//! line and what its hooks say. Measured on agy 1.5.8.
//!
//! agy reads lifecycle hooks from `.agents/hooks.json` in any workspace
//! folder, one added with `--add-dir` included, and session-only settings
//! from `--settings <file>`. So a launch writes one folder and one file under
//! mesimon's state dir and names both on argv: nothing is written into the
//! checkout or into `~/.gemini`, and a session mesimon did not start reports
//! nothing.
//!
//! Three hooks and the title command run `mesimon hook`, which prints
//! nothing. That is the answer agy takes as "no opinion" on every event:
//! **a hook that prints `{}` or `{"decision":""}` on `PreToolUse` denies the
//! tool call**, so no hook of mesimon's ever prints an empty object. The one
//! hook that prints is the brief's, on `SessionStart` (T-1232).
//!
//! The board's tools (T-1232) are `.agents/mcp_config.json` beside the hooks,
//! the shim [`hook_settings::mcp_config_json`] renders for Claude, and an
//! allow rule in the settings file, so a board tool does not ask the person
//! each time: the daemon's tier check is the gate, as on the mod road. agy
//! reaches them through its own `call_mcp_tool`, after reading a schema file
//! it keeps under its home, never as native tools.
//!
//! Words go into the pane by the paste road Claude's hook set takes
//! (T-1233): bracketed paste into agy's box (`claude::composer::AGY`), then
//! a separate Enter, acked by the first `PreInvocation`.
//!
//! Not built yet, each its own ticket: approvals and plan mode from the
//! board, the transcript reader, cost and quota.

use std::path::{Path, PathBuf};

use mesimon_core::attention::{Signal, TurnOutcome};
use mesimon_core::board::{
    AgentProvider, AgentTools, FailReason, Reason, SessionRecord, SessionState, UnknownReason,
};
use serde_json::{json, Value};

use super::{
    AgentAdapter, AgentCapabilities, AgentPreview, ExternalOwner, HookObservation, LaunchContext,
    LaunchSpec, ObservationMode, ResumePolicy,
};
use crate::hook_settings::{self, mesimon_bin};
use crate::ingest::HookFrame;

pub struct Agy;

/// The frame names agy's reports travel under: its own, so nothing that
/// reads a Claude hook by name reads one of these.
pub const SESSION_START: &str = "AgySessionStart";
pub const PRE_INVOCATION: &str = "AgyPreInvocation";
pub const STOP: &str = "AgyStop";
pub const TITLE: &str = "AgyTitle";

/// The hooks registered, as `(agy's event, the frame's name)`. `PreToolUse`
/// and `PostToolUse` are not among them: the title feed says when a
/// confirmation is pending and when it is answered, and a hook on every tool
/// call is a process per call.
const HOOKS: [(&str, &str); 3] =
    [("SessionStart", SESSION_START), ("PreInvocation", PRE_INVOCATION), ("Stop", STOP)];

/// `mesimon hook` aborts itself at 500 ms; agy's own bound is seconds.
const HOOK_TIMEOUT_SECS: u32 = 5;

const DETAIL_MAX: usize = 200;

/// Whether this process may start an agy session: `MESIMON_AGY=1`
/// ([`mesimon_core::board::AGY_SWITCH`]). Off, nothing of agy's launches,
/// whatever a board or a tier names.
pub fn enabled() -> bool {
    std::env::var(mesimon_core::board::AGY_SWITCH).as_deref() == Ok("1")
}

/// agy's own data folder, read and never written: `MESIMON_AGY_HOME` (the
/// test seam), else `~/.gemini/antigravity-cli`.
pub fn home() -> PathBuf {
    std::env::var_os("MESIMON_AGY_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(".gemini")
            .join("antigravity-cli")
    })
}

/// The folders the person told agy to trust: `trustedWorkspaces` in agy's
/// own `settings.json`. Empty when the file is absent or reads as anything
/// else.
fn trusted_by_person(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("trustedWorkspaces").and_then(Value::as_array).cloned())
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}

/// The session's folder, handed to agy with `--add-dir`: it holds
/// `.agents/hooks.json` and, where the column gives the board's tools,
/// `.agents/mcp_config.json`, and nothing else.
pub fn session_dir(paths: &crate::paths::Paths, session: uuid::Uuid) -> PathBuf {
    paths.state_dir.join("agy").join(session.to_string())
}

/// The session-only settings file, handed to agy with `--settings`.
pub fn settings_file(paths: &crate::paths::Paths, session: uuid::Uuid) -> PathBuf {
    paths.state_dir.join("agy").join(format!("{session}.json"))
}

/// One `mesimon hook` command line as agy runs it, through `sh -c`.
fn hook_command(bin: &Path, sock: &Path, session: uuid::Uuid, event: &str) -> String {
    use mesimon_core::text::sh_quote;
    format!(
        "{} hook --sock {} --session {session} --event {event}",
        sh_quote(&bin.display().to_string()),
        sh_quote(&sock.display().to_string()),
    )
}

/// The brief's word to `mesimon agent-brief`: answer in agy's shape.
pub const BRIEF_PROVIDER: &str = "agy";

/// What the brief's `SessionStart` hook prints: the brief as one system
/// message, which agy keeps in the conversation, so a wake (no
/// `SessionStart`) still has it.
pub fn brief_answer() -> Value {
    json!({ "injectSteps": [{ "systemMessage": { "systemMessage": mesimon_core::brief::TEXT } }] })
}

/// `hooks.json`: one named hook, `mesimon`, with a flat handler list per
/// event (agy takes the grouped `matcher` form on tool events alone). With
/// the brief on, `SessionStart` runs a second handler after the observer,
/// the one that prints.
fn render_hooks(bin: &Path, sock: &Path, session: uuid::Uuid, brief: bool) -> Value {
    let mut events = serde_json::Map::new();
    for (event, name) in HOOKS {
        let mut handlers = vec![json!({
            "type": "command",
            "command": hook_command(bin, sock, session, name),
            "timeout": HOOK_TIMEOUT_SECS,
        })];
        if brief && event == "SessionStart" {
            handlers.push(json!({
                "type": "command",
                "command": format!(
                    "{} agent-brief --provider {BRIEF_PROVIDER}",
                    mesimon_core::text::sh_quote(&bin.display().to_string()),
                ),
                "timeout": HOOK_TIMEOUT_SECS,
            }));
        }
        events.insert(event.into(), Value::Array(handlers));
    }
    json!({ "mesimon": events })
}

/// The allow rule for every board tool the shim lists: agy asks before each
/// MCP call without one. Which tools a column admits is the shim's list
/// (`--tools`) and the daemon's check at every call.
fn allow_rule() -> String {
    format!("mcp({}/*)", mesimon_core::mcp::SERVER_NAME)
}

/// The session's settings. `trustedWorkspaces` is the person's own list
/// with the session folder added, which mesimon wrote, and the ticket's
/// worktree **only where the person already trusts its main checkout**: agy
/// trusts a folder by its exact path, so a worktree cut from a trusted
/// checkout would otherwise ask again on every ticket, and a folder nobody
/// trusted still gets agy's own dialog. The title command is the state feed:
/// agy runs it on every state change with the session's state on stdin.
fn render_settings(
    context: &LaunchContext<'_>,
    dir: &Path,
    bin: &Path,
    trusted: &[String],
) -> Value {
    let mut list: Vec<String> = trusted.to_vec();
    let mut add = |path: &Path| {
        let word = path.display().to_string();
        if !list.contains(&word) {
            list.push(word);
        }
    };
    add(dir);
    let checkout = &context.paths.repo_root;
    if context.managed_git_worktree && trusted.iter().any(|t| same_folder(Path::new(t), checkout)) {
        add(context.cwd);
    }
    let mut settings = json!({
        "trustedWorkspaces": list,
        "title": {
            "type": "command",
            "command": hook_command(bin, &context.paths.hook_sock(), context.session, TITLE),
        },
    });
    if context.tools != AgentTools::Off {
        settings["permissions"] = json!({ "allow": [allow_rule()] });
    }
    settings
}

fn write_private(path: &Path, value: &Value) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Write the session's folder and settings file, rewritten on every launch
/// (a wake's binary or socket may have moved). Returns `(folder, settings)`.
fn prepare(context: &LaunchContext<'_>, trusted: &[String]) -> Result<(PathBuf, PathBuf), String> {
    let (paths, session, bin) = (context.paths, context.session, mesimon_bin());
    let dir = session_dir(paths, session);
    let agents = dir.join(".agents");
    std::fs::create_dir_all(&agents).map_err(|e| format!("{}: {e}", agents.display()))?;
    if let Some(root) = dir.parent() {
        crate::paths::own_private_dir(root).map_err(|e| e.to_string())?;
    }
    // The brief rides with the board's tools, as Codex's does: it names
    // `get_ticket`, which an `Off` column has not got.
    let tools = context.tools != AgentTools::Off;
    let hooks = render_hooks(&bin, &paths.hook_sock(), session, context.brief && tools);
    write_private(&agents.join("hooks.json"), &hooks)?;
    // Rewritten or removed on every launch: a wake follows the column's tier
    // as it stands then.
    let mcp = agents.join("mcp_config.json");
    if tools {
        let text = hook_settings::mcp_config_json(paths, &bin, session, context.tools);
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        write_private(&mcp, &value)?;
    } else if let Err(e) = std::fs::remove_file(&mcp) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("{}: {e}", mcp.display()));
        }
    }
    let settings = settings_file(paths, session);
    write_private(&settings, &render_settings(context, &dir, &bin, trusted))?;
    Ok((dir, settings))
}

/// A conversation id as it may ride argv: agy's are uuids, and a frame is
/// any same-uid process's to send.
fn conversation_id(word: &str) -> Option<&str> {
    (!word.is_empty()
        && word.len() <= 64
        && word.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && !word.starts_with('-'))
    .then_some(word)
}

fn launch(context: &LaunchContext<'_>, conversation: Option<&str>) -> Result<Vec<String>, String> {
    launch_with(context, conversation, &trusted_by_person(&home()))
}

/// [`launch`] against a given trust list: the person's own, as read.
fn launch_with(
    context: &LaunchContext<'_>,
    conversation: Option<&str>,
    trusted: &[String],
) -> Result<Vec<String>, String> {
    let (dir, settings) =
        prepare(context, trusted).map_err(|e| format!("agy launch files: {e}"))?;
    let executable = std::env::var("MESIMON_AGY_BIN").unwrap_or_else(|_| "agy".into());
    let mut argv = vec![
        executable,
        "--add-dir".into(),
        dir.display().to_string(),
        "--settings".into(),
        settings.display().to_string(),
    ];
    // Plan mode for this launch (T-434). No column word is read yet: without
    // the flag agy starts in the person's own default mode.
    if context.plan {
        argv.extend(["--mode".into(), "plan".into()]);
    }
    if context.tier.provider == AgentProvider::Agy {
        if let Some(model) = context.tier.model_arg() {
            argv.extend(["--model".into(), model.into()]);
        }
        if let Some(effort) = context.tier.effort_arg() {
            argv.extend(["--effort".into(), effort.into()]);
        }
    }
    if let Some(id) = conversation.and_then(conversation_id) {
        argv.extend(["--conversation".into(), id.into()]);
    }
    Ok(argv)
}

fn sanitize(text: &str) -> Option<String> {
    let cleaned = mesimon_core::text::scrub_cells(text, false);
    let cleaned: String = cleaned.trim().chars().take(DETAIL_MAX).collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// What a `Stop` says. `fullyIdle: false` is a turn whose background tasks
/// still run: the seat stays working, and the stop that follows them ends
/// it.
fn stop_signal(payload: &Value) -> Option<Signal> {
    if payload.get("fullyIdle").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let outcome = match payload.get("terminationReason").and_then(Value::as_str) {
        Some("USER_CANCELED") => TurnOutcome::Interrupted,
        Some(
            "ERROR" | "MAX_INVOCATIONS" | "MAX_FORCED_INVOCATIONS" | "MAX_TOKEN_BUDGET_EXCEEDED",
        ) => TurnOutcome::Failed(FailReason::Unknown),
        _ => TurnOutcome::Completed,
    };
    Some(Signal::TurnEnded { outcome })
}

/// What a title report says, against the state the record holds. A pending
/// confirmation is agy asking the person; its answer shows as the agent at
/// work again. `idle` is the prompt box of a pane that just came up (agy's
/// `SessionStart` waits for the first prompt). A turn's end is `Stop`'s to
/// say, but for one: **an Esc fires no `Stop`** (1.5.9, T-1233: the turn
/// cut, "Interrupted" on screen, and the feed alone going `idle`), so an
/// `idle` on a running or asking turn that had no `Stop` (`stopped`) is that
/// interrupt. A completed turn's `Stop` comes ~300 ms ahead of its `idle`
/// (measured), inside the leave's settle, so the record still reads the turn
/// running then: the flag, not the state, says the end was stated.
fn title_signal(payload: &Value, state: &SessionState, stopped: bool) -> Option<Signal> {
    if payload.get("tool_confirmation_pending").and_then(Value::as_bool) == Some(true) {
        return Some(Signal::Attention { reason: Reason::Permission });
    }
    let coming_up = matches!(
        state,
        SessionState::Spawning
            | SessionState::Unknown {
                reason: UnknownReason::DaemonRestarted | UnknownReason::ObservationLost
            }
    );
    let asked = matches!(state, SessionState::RequiresAction { reason: Reason::Permission });
    let turning = *state == SessionState::Running || asked;
    match payload.get("agent_state").and_then(Value::as_str)? {
        "idle" | "reviewing" if coming_up => Some(Signal::Ready),
        "idle" if turning && !stopped => {
            Some(Signal::TurnEnded { outcome: TurnOutcome::Interrupted })
        }
        "working" | "thinking" | "tool_use" if coming_up || asked => Some(Signal::TurnStarted),
        _ => None,
    }
}

impl AgentAdapter for Agy {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            observation: ObservationMode::Hooks,
            resume: ResumePolicy::FreshWhenHistoryMissing,
        }
    }

    fn parse_hook(&self, frame: &HookFrame, record: &mut SessionRecord) -> HookObservation {
        let mut result = HookObservation::default();
        let payload = &frame.payload;
        // Every hook names the conversation (`conversationId`), and the
        // title feed does from the moment there is one (`conversation_id`,
        // empty before the first prompt): a wake learns it with no
        // `SessionStart`, and a `/clear` or an in-app `/resume` moves it.
        let named = payload
            .get("conversationId")
            .or_else(|| payload.get("conversation_id"))
            .and_then(Value::as_str)
            .and_then(conversation_id);
        if let Some(id) = named {
            if record.agy_conversation_id.as_deref() != Some(id) {
                record.agy_conversation_id = Some(id.to_string());
                result.metadata_changed = true;
            }
        }
        match frame.event.as_str() {
            // The first model call of a turn is its prompt taken; a later
            // one is the same turn going on.
            PRE_INVOCATION => {
                let first = payload.get("invocationNum").and_then(Value::as_u64).unwrap_or(0) == 0;
                record.agy_turn_stopped = false;
                result.signal =
                    Some(if first { Signal::UserPromptSubmit } else { Signal::TurnStarted });
            }
            STOP => {
                result.signal = stop_signal(payload);
                record.agy_turn_stopped = result.signal.is_some();
                if matches!(
                    result.signal,
                    Some(Signal::TurnEnded { outcome: TurnOutcome::Failed(_) })
                ) {
                    result.detail =
                        payload.get("error").and_then(Value::as_str).and_then(sanitize).or_else(
                            || {
                                payload
                                    .get("terminationReason")
                                    .and_then(Value::as_str)
                                    .and_then(sanitize)
                            },
                        );
                }
            }
            TITLE => result.signal = title_signal(payload, &record.state, record.agy_turn_stopped),
            _ => {}
        }
        result
    }

    /// No transcript reader yet: a card shows the state and no words.
    fn preview(&self, _path: &Path) -> Option<AgentPreview> {
        None
    }

    fn rows(&self, _at: u64, _record: &Value) -> Vec<mesimon_core::mesophon::TranscriptRow> {
        Vec::new()
    }

    fn conversation_key(&self, record: &SessionRecord) -> Option<String> {
        record.agy_conversation_id.clone()
    }

    /// No id is no conversation: agy mints one at the first prompt. A known
    /// id is missing only where agy's own store is there to read and holds
    /// nothing under it.
    fn history_missing(&self, record: &SessionRecord) -> bool {
        let Some(id) = record.agy_conversation_id.as_deref() else { return true };
        let store = home().join("conversations");
        match std::fs::read_dir(&store) {
            Ok(entries) => !entries
                .flatten()
                .any(|e| e.file_name().to_str().is_some_and(|name| name.starts_with(id))),
            Err(_) => false,
        }
    }

    /// Not read yet: agy keeps a lock per open conversation, and nothing
    /// here has measured it.
    fn external_owner(
        &self,
        _record: &SessionRecord,
        _own_pane: Option<i32>,
    ) -> Option<ExternalOwner> {
        None
    }

    /// agy takes no id at launch, so `identity` names nothing here.
    fn start(&self, context: &LaunchContext<'_>, _identity: &str) -> Result<LaunchSpec, String> {
        launch(context, None).map(LaunchSpec::plain)
    }

    fn resume(
        &self,
        context: &LaunchContext<'_>,
        record: &SessionRecord,
    ) -> Result<LaunchSpec, String> {
        launch(context, record.agy_conversation_id.as_deref()).map(LaunchSpec::plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_settings::HookSet;
    use mesimon_core::board::{ColumnSettings, SessionKind};
    use mesimon_core::tier::{Effort, Tier};

    fn frame(event: &str, payload: Value) -> HookFrame {
        let header = json!({"v": 1, "session": "s", "event": event});
        crate::ingest::parse_frame(format!("{header}\n{payload}").as_bytes()).unwrap()
    }

    fn record(state: SessionState) -> SessionRecord {
        SessionRecord::new(
            uuid::Uuid::from_u128(7),
            SessionKind::Agy,
            ulid::Ulid(1),
            vec!["agy".into()],
            "/repo".into(),
            state,
        )
    }

    fn context<'a>(paths: &'a crate::paths::Paths, cwd: &'a Path, tier: Tier) -> LaunchContext<'a> {
        LaunchContext {
            paths,
            cwd,
            managed_git_worktree: false,
            session: uuid::Uuid::from_u128(7),
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

    const ID: &str = "e491708f-fd93-4aec-9a23-1ad87af7c910";

    /// What agy 1.5.8 sent through one turn with a confirmation in it, in
    /// order: the seat comes up idle, the prompt starts the turn, the dialog
    /// asks, its answer resumes, and `Stop` ends it.
    #[test]
    fn a_turn_reads_as_ready_prompt_ask_resume_and_end() {
        let mut rec = record(SessionState::Spawning);
        let title = |state: &str, pending: bool| {
            let mut p = json!({"agent_state": state, "conversation_id": ""});
            if pending {
                p["tool_confirmation_pending"] = true.into();
            }
            frame(TITLE, p)
        };
        // Signing in says nothing; the prompt box says the seat is up.
        assert_eq!(Agy.parse_hook(&title("authenticating", false), &mut rec).signal, None);
        assert_eq!(Agy.parse_hook(&title("idle", false), &mut rec).signal, Some(Signal::Ready));
        assert_eq!(rec.agy_conversation_id, None, "an empty id names no conversation");

        rec.state = SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Unknown };
        let hook = |event: &str, extra: Value| {
            let mut p = json!({"conversationId": ID, "hookEventName": "x"});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            frame(event, p)
        };
        let started = Agy.parse_hook(&hook(SESSION_START, json!({})), &mut rec);
        assert_eq!(started.signal, None);
        assert!(started.metadata_changed);
        assert_eq!(rec.agy_conversation_id.as_deref(), Some(ID));
        let first = Agy.parse_hook(&hook(PRE_INVOCATION, json!({"invocationNum": 0})), &mut rec);
        assert_eq!(first.signal, Some(Signal::UserPromptSubmit));
        assert!(!first.metadata_changed, "the same id again changes nothing");

        rec.state = SessionState::Running;
        // A working report mid-turn is no news; a pending confirmation is.
        assert_eq!(Agy.parse_hook(&title("working", false), &mut rec).signal, None);
        assert_eq!(
            Agy.parse_hook(&title("tool_use", true), &mut rec).signal,
            Some(Signal::Attention { reason: Reason::Permission })
        );
        rec.state = SessionState::RequiresAction { reason: Reason::Permission };
        assert_eq!(
            Agy.parse_hook(&title("working", false), &mut rec).signal,
            Some(Signal::TurnStarted)
        );
        rec.state = SessionState::Running;
        assert_eq!(
            Agy.parse_hook(&hook(PRE_INVOCATION, json!({"invocationNum": 2})), &mut rec).signal,
            Some(Signal::TurnStarted)
        );
        let stop = json!({"terminationReason": "NO_TOOL_CALL", "fullyIdle": true});
        assert_eq!(
            Agy.parse_hook(&hook(STOP, stop), &mut rec).signal,
            Some(Signal::TurnEnded { outcome: TurnOutcome::Completed })
        );
        // The idle report that follows a stop never restates the turn's end.
        rec.state = SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
        assert_eq!(Agy.parse_hook(&title("idle", false), &mut rec).signal, None);
    }

    /// An Esc mid-turn (1.5.9, T-1233): no `Stop`, the feed goes `working`
    /// then `idle`. That idle, on a running or asking turn, is the
    /// interrupt; behind the turn's `Stop`, whose leave is still settling
    /// while the record reads `Running`, it says nothing.
    #[test]
    fn an_idle_report_with_no_stop_before_it_is_the_interrupt() {
        use mesimon_core::board::StopReason;
        let idle = frame(TITLE, json!({"agent_state": "idle", "conversation_id": ID}));
        let invoked = frame(PRE_INVOCATION, json!({"invocationNum": 0}));
        let interrupted = Some(Signal::TurnEnded { outcome: TurnOutcome::Interrupted });
        let mut rec = record(SessionState::Running);
        let _ = Agy.parse_hook(&invoked, &mut rec);
        assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, interrupted);
        rec.state = SessionState::RequiresAction { reason: Reason::Permission };
        assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, interrupted);
        // A completed turn: its `Stop`, then the idle, the record still
        // `Running` inside the settle.
        rec.state = SessionState::Running;
        let stop = frame(STOP, json!({"terminationReason": "NO_TOOL_CALL", "fullyIdle": true}));
        assert!(Agy.parse_hook(&stop, &mut rec).signal.is_some());
        assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, None);
        // A stop that ends nothing (background tasks) states no end.
        let busy = frame(STOP, json!({"terminationReason": "NO_TOOL_CALL", "fullyIdle": false}));
        let _ = Agy.parse_hook(&invoked, &mut rec);
        let _ = Agy.parse_hook(&busy, &mut rec);
        assert!(!rec.agy_turn_stopped);
        // The next turn's first call clears the stop: its Esc is read again.
        let _ = Agy.parse_hook(&stop, &mut rec);
        let _ = Agy.parse_hook(&invoked, &mut rec);
        assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, interrupted);
        // Not on a seat at its trust dialog, nor one idle.
        for state in [
            SessionState::RequiresAction { reason: Reason::Trust },
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        ] {
            rec.state = state;
            assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, None);
        }
    }

    #[test]
    fn a_stop_says_how_the_turn_ended_and_a_busy_one_ends_nothing() {
        let mut rec = record(SessionState::Running);
        let stop = |reason: &str, idle: bool, error: &str| {
            frame(STOP, json!({"terminationReason": reason, "fullyIdle": idle, "error": error}))
        };
        let seen = Agy.parse_hook(&stop("USER_CANCELED", true, ""), &mut rec);
        assert_eq!(seen.signal, Some(Signal::TurnEnded { outcome: TurnOutcome::Interrupted }));
        let failed = Agy.parse_hook(&stop("ERROR", true, "quota\u{7} exhausted"), &mut rec);
        assert_eq!(
            failed.signal,
            Some(Signal::TurnEnded { outcome: TurnOutcome::Failed(FailReason::Unknown) })
        );
        assert_eq!(failed.detail.as_deref(), Some("quota exhausted"), "scrubbed for a cell");
        let named = Agy.parse_hook(&stop("MAX_INVOCATIONS", true, ""), &mut rec);
        assert_eq!(named.detail.as_deref(), Some("MAX_INVOCATIONS"));
        // Background tasks still run: the seat is working until their stop.
        assert_eq!(Agy.parse_hook(&stop("NO_TOOL_CALL", false, ""), &mut rec).signal, None);
    }

    /// After a daemon restart the title feed re-derives the seat, and a
    /// wake learns the conversation it resumed from the feed alone.
    #[test]
    fn the_title_feed_re_derives_a_seat_and_names_a_resumed_conversation() {
        let mut rec = record(SessionState::Unknown { reason: UnknownReason::DaemonRestarted });
        let working = frame(TITLE, json!({"agent_state": "working", "conversation_id": ID}));
        let seen = Agy.parse_hook(&working, &mut rec);
        assert_eq!(seen.signal, Some(Signal::TurnStarted));
        assert!(seen.metadata_changed);
        assert_eq!(rec.agy_conversation_id.as_deref(), Some(ID));
        // An id that could not ride argv is never kept.
        for bad in ["--dangerously-skip-permissions", "a b", &"f".repeat(65), "x; rm"] {
            let mut fresh = record(SessionState::Spawning);
            let _ = Agy.parse_hook(
                &frame(TITLE, json!({"agent_state": "idle", "conversation_id": bad})),
                &mut fresh,
            );
            assert_eq!(fresh.agy_conversation_id, None, "{bad}");
        }
        let reviewing = frame(TITLE, json!({"agent_state": "reviewing", "conversation_id": ID}));
        let mut woke = record(SessionState::Spawning);
        assert_eq!(Agy.parse_hook(&reviewing, &mut woke).signal, Some(Signal::Ready));
    }

    /// The launch writes one folder and one file under the state dir, names
    /// both on argv, and resumes by the conversation the record learned.
    #[test]
    fn a_launch_writes_its_folder_and_settings_and_a_wake_names_the_conversation() {
        let root = std::env::temp_dir().join(format!("msmn-agy-{}", uuid::Uuid::new_v4()));
        let repo = root.join("repo");
        let tree = root.join("tree");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        let paths = crate::paths::Paths::for_repo(&repo).unwrap();
        let tier = Tier {
            id: "A".into(),
            name: "flash".into(),
            provider: AgentProvider::Agy,
            model: "gemini-3.8-flash-high".into(),
            effort: Effort::High,
            description: String::new(),
        };
        let ctx = LaunchContext { plan: true, ..context(&paths, &tree, tier) };
        let started = launch_with(&ctx, None, &[]).unwrap();
        let dir = session_dir(&paths, ctx.session);
        let settings = settings_file(&paths, ctx.session);
        assert!(dir.starts_with(&paths.state_dir) && settings.starts_with(&paths.state_dir));
        assert_eq!(pair(&started, "--add-dir"), [dir.display().to_string()]);
        assert_eq!(pair(&started, "--settings"), [settings.display().to_string()]);
        assert_eq!(pair(&started, "--mode"), ["plan"]);
        assert_eq!(pair(&started, "--model"), ["gemini-3.8-flash-high"]);
        assert_eq!(pair(&started, "--effort"), ["high"]);
        assert!(pair(&started, "--conversation").is_empty(), "{started:?}");
        assert!(!started.iter().any(|a| a.contains("dangerously")), "{started:?}");

        let hooks: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(".agents/hooks.json")).unwrap())
                .unwrap();
        let ours = hooks["mesimon"].as_object().unwrap();
        assert_eq!(ours.len(), 3, "{hooks}");
        for (event, name) in HOOKS {
            let command = ours[event][0]["command"].as_str().unwrap();
            assert!(command.ends_with(&format!("--session {} --event {name}", ctx.session)));
            assert!(command.contains(" hook --sock '"), "{command}");
        }
        // No tool hook: an empty answer there would deny every call.
        assert!(ours.get("PreToolUse").is_none() && ours.get("PostToolUse").is_none());
        // A column without the board's tools: no server, no rule, no brief.
        assert_eq!(ours["SessionStart"].as_array().unwrap().len(), 1, "{hooks}");
        assert!(!dir.join(".agents/mcp_config.json").exists());
        let file: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(file["title"]["command"].as_str().unwrap().ends_with(TITLE));
        assert!(file.get("permissions").is_none(), "{file}");
        // Nobody trusted the checkout: only mesimon's own folder is named.
        assert_eq!(file["trustedWorkspaces"], json!([dir.display().to_string()]));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // Another provider's tier passes nothing, and a wake names the
        // conversation the record learned.
        let mut rec = record(SessionState::Sleeping);
        assert!(Agy.history_missing(&rec), "no id is no conversation");
        rec.agy_conversation_id = Some(ID.into());
        let plain = context(&paths, &tree, Tier::builtin(AgentProvider::ClaudeCode));
        let woke = launch_with(&plain, rec.agy_conversation_id.as_deref(), &[]).unwrap();
        assert_eq!(pair(&woke, "--conversation"), [ID]);
        for flag in ["--model", "--effort", "--mode"] {
            assert!(pair(&woke, flag).is_empty(), "{flag}: {woke:?}");
        }
        assert_eq!(Agy.conversation_key(&rec).as_deref(), Some(ID));
        let _ = std::fs::remove_dir_all(&paths.state_dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The board's tools and the brief (T-1232): the shim's server beside
    /// the hooks, one allow rule for it, and the brief as a second
    /// `SessionStart` handler; a wake into a column without tools takes all
    /// three away again.
    #[test]
    fn a_launch_with_the_board_s_tools_writes_the_server_the_rule_and_the_brief() {
        let root = std::env::temp_dir().join(format!("msmn-agy-mcp-{}", uuid::Uuid::new_v4()));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let paths = crate::paths::Paths::for_repo(&repo).unwrap();
        let tier = Tier::builtin(AgentProvider::Agy);
        let ctx = LaunchContext {
            tools: AgentTools::Full,
            brief: true,
            ..context(&paths, &repo, tier.clone())
        };
        launch_with(&ctx, None, &[]).unwrap();
        let dir = session_dir(&paths, ctx.session);
        let read = |path: &Path| -> Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let config = read(&dir.join(".agents/mcp_config.json"));
        let server = &config["mcpServers"][mesimon_core::mcp::SERVER_NAME];
        let args: Vec<&str> =
            server["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        assert_eq!(args[0], "mcp");
        assert_eq!(pair_of(&args, "--sock"), paths.orch_sock().display().to_string());
        assert_eq!(pair_of(&args, "--session"), ctx.session.to_string());
        assert_eq!(pair_of(&args, "--tools"), AgentTools::Full.word());
        use std::os::unix::fs::PermissionsExt;
        let mode =
            std::fs::metadata(dir.join(".agents/mcp_config.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let settings = read(&settings_file(&paths, ctx.session));
        assert_eq!(settings["permissions"]["allow"], json!(["mcp(mesimon/*)"]));

        let hooks = read(&dir.join(".agents/hooks.json"));
        let start = hooks["mesimon"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2, "{hooks}");
        // The observer first, the brief second; nothing else gains a handler.
        assert!(start[0]["command"].as_str().unwrap().ends_with(SESSION_START));
        assert!(start[1]["command"].as_str().unwrap().ends_with(" agent-brief --provider agy"));
        for event in ["PreInvocation", "Stop"] {
            assert_eq!(hooks["mesimon"][event].as_array().unwrap().len(), 1, "{event}");
        }

        // The brief switched off keeps the tools and drops the handler.
        let quiet = LaunchContext { brief: false, ..ctx };
        launch_with(&quiet, None, &[]).unwrap();
        let hooks = read(&dir.join(".agents/hooks.json"));
        assert_eq!(hooks["mesimon"]["SessionStart"].as_array().unwrap().len(), 1);
        assert!(dir.join(".agents/mcp_config.json").exists());

        // A wake into a column without the tools: no server, no rule, no brief.
        let off =
            LaunchContext { tools: AgentTools::Off, brief: true, ..context(&paths, &repo, tier) };
        launch_with(&off, Some(ID), &[]).unwrap();
        assert!(!dir.join(".agents/mcp_config.json").exists());
        assert!(read(&settings_file(&paths, ctx.session)).get("permissions").is_none());
        let hooks = read(&dir.join(".agents/hooks.json"));
        assert_eq!(hooks["mesimon"]["SessionStart"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&paths.state_dir);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// What the brief's hook prints is agy's `SessionStart` answer, the
    /// brief verbatim as one system message, and never an empty object.
    #[test]
    fn the_brief_answer_is_one_system_message_with_the_brief() {
        let answer = brief_answer();
        assert_eq!(answer["injectSteps"].as_array().unwrap().len(), 1);
        assert_eq!(
            answer["injectSteps"][0]["systemMessage"]["systemMessage"],
            mesimon_core::brief::TEXT
        );
        assert_eq!(answer.as_object().unwrap().len(), 1, "{answer}");
    }

    fn pair_of(args: &[&str], flag: &str) -> String {
        args.windows(2).find(|w| w[0] == flag).map(|w| w[1].to_string()).unwrap()
    }

    /// Decision 3: a ticket's worktree is passed as trusted only where the
    /// person already trusts its main checkout, by exact path, and the
    /// person's own list is carried whole.
    #[test]
    fn a_worktree_is_trusted_only_where_the_person_trusts_its_checkout() {
        let root = std::env::temp_dir().join(format!("msmn-agy-trust-{}", uuid::Uuid::new_v4()));
        let repo = root.join("repo");
        let tree = root.join("tree");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        let paths = crate::paths::Paths::for_repo(&repo).unwrap();
        let builtin = || Tier::builtin(AgentProvider::Agy);
        let dir = PathBuf::from("/state/agy/s");
        let bin = Path::new("/bin/mesimon");
        let list = |ctx: &LaunchContext<'_>, trusted: &[String]| -> Vec<String> {
            render_settings(ctx, &dir, bin, trusted)["trustedWorkspaces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        };
        let checkout = paths.repo_root.display().to_string();
        let parent = root.display().to_string();
        let tree_word = tree.display().to_string();
        let managed =
            LaunchContext { managed_git_worktree: true, ..context(&paths, &tree, builtin()) };
        // The checkout is trusted: the worktree rides beside the person's list.
        let trusted = list(&managed, std::slice::from_ref(&checkout));
        assert_eq!(trusted, [checkout.clone(), "/state/agy/s".into(), tree_word.clone()]);
        // Only a parent is trusted: agy asks by exact path, and so does this.
        assert!(!list(&managed, std::slice::from_ref(&parent)).contains(&tree_word));
        assert!(!list(&managed, &[]).contains(&tree_word));
        // A folder that is no worktree of mesimon's is never added.
        let shared = context(&paths, &tree, builtin());
        assert!(!list(&shared, std::slice::from_ref(&checkout)).contains(&tree_word));
        let _ = std::fs::remove_dir_all(&root);
    }
}
