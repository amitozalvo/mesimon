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
//! Three lifecycle hooks (`SessionStart`, `PreInvocation`, `Stop`), the
//! `PreToolUse` observer, and the title command run `mesimon hook`, which prints
//! nothing. That is the answer agy takes as "no opinion" on every event:
//! **a hook that prints `{}` or `{"decision":""}` on `PreToolUse` denies the
//! tool call**, so no hook of mesimon's ever prints an empty object, and
//! `mesimon hook` never answers `allow` for a person. `PreToolUse` also runs
//! `mesimon gate --provider agy` over agy's structured write tools (T-1238),
//! printing nothing on pass and `{"decision":"deny","reason":"…"}` on refusal;
//! the other hook that prints is the brief's, on `SessionStart` (T-1232).
//!
//! Approvals and questions (T-1235): `PreToolUse` (registered in agy's grouped
//! `{"matcher":"*","hooks":[…]}` form after the write gate) carries
//! `toolCall {id, name, args}` right before agy opens a confirmation or
//! `ask_question` dialog. The phone's permission card and the phone's and
//! crown's question cards bind to that call and walk agy's numbered dialog
//! (`> 1. …`) by keys, never selecting the option that persists to the
//! person's `settings.json`; the title feed's return to work is the edge that
//! confirms the dialog closed.
//!
//! The board's tools (T-1232) are `.agents/mcp_config.json` beside the hooks,
//! the shim [`hook_settings::mcp_config_json`] renders for Claude, and an
//! allow rule in the settings file, so a board tool does not ask the person
//! each time: the daemon's tier check is the gate, as on the mod road. agy
//! exposes the brief's `get_ticket` directly. Other tools use its lazy
//! `call_mcp_tool` after a schema read under its home.
//!
//! Words go into the pane by the paste road Claude's hook set takes
//! (T-1233): bracketed paste into agy's box (`claude::composer::AGY`), then
//! a separate Enter, acked by the first `PreInvocation`.
//!
//! The column's `agy mode` (T-1234) rides the launch: `--mode` for agy's
//! three words, and Default in the session's settings, which `--mode` has
//! no word for.
//!
//! The transcript (T-1237) is the hooks' `transcriptPath`, read by
//! [`history`]; the title feed's own `transcript_path` names a folder the
//! CLI never writes, so it is never kept. The title feed also carries the
//! context window ([`context_window`]) and the plan's quota
//! (`usage::parse_agy`), and its `email`, which nothing reads.

mod history;

use std::path::{Path, PathBuf};

use mesimon_core::attention::{Signal, TurnOutcome};
use mesimon_core::board::{
    AgentProvider, AgentTools, AgyStop, FailReason, Reason, SessionRecord, SessionState,
    UnknownReason,
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
pub const PRE_TOOL_USE: &str = "AgyPreToolUse";
pub const STOP: &str = "AgyStop";
pub const TITLE: &str = "AgyTitle";

/// The flat lifecycle hooks registered, as `(agy's event, the frame's name)`;
/// `PreToolUse` is registered beside them in agy's grouped `matcher` form
/// ([`render_hooks`]: [`GATE_MATCHER`] for T-1238's write gate, and `"*"` for
/// T-1235's pending-call observer). `PostToolUse` is not registered: the title
/// feed says when a confirmation or question is answered.
const HOOKS: [(&str, &str); 3] =
    [("SessionStart", SESSION_START), ("PreInvocation", PRE_INVOCATION), ("Stop", STOP)];

/// agy's structured file-writing tools (T-1238), matched on `PreToolUse` for
/// `mesimon gate --provider agy`. Cascade models use `write_to_file`,
/// `replace_file_content` and `multi_replace_file_content`; direct-tool models
/// (such as Gemini 4 Carbon) use `write_file` and `edit_file` (`claude/tools.go`
/// in agy 1.5.13 maps Claude's `Write`/`Edit`/`NotebookEdit` to both sets).
pub const GATE_MATCHER: &str = "write_to_file|write_file|replace_file_content|\
multi_replace_file_content|edit_file|sed_file|notebook_edit|edit_notebook|delete_file";

/// `mesimon hook` aborts itself at 500 ms; agy's own bound is seconds.
const HOOK_TIMEOUT_SECS: u32 = 5;

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

fn trusts_checkout(trusted: &[String], checkout: &Path) -> bool {
    trusted.iter().any(|t| same_folder(Path::new(t), checkout))
}

/// Read personal trust only on entry to the worktree's own dialog.
pub(crate) fn trust_detail(
    checkout: &Path,
    worktree: Option<&Path>,
    folder: &Path,
    home: &Path,
) -> String {
    let question = mesimon_core::board::AGY_FOLDER_TRUST;
    if worktree.is_some_and(|tree| same_folder(tree, folder))
        && !trusts_checkout(&trusted_by_person(home), checkout)
    {
        let path = mesimon_core::text::scrub_cells(&checkout.to_string_lossy(), false);
        return format!("{question} ∙ Trusting {path} once in agy covers this board's worktrees");
    }
    question.into()
}

/// `agy models`, as the daemon last read it (T-1234): one file beside
/// `tiers.toml` that every board's daemon writes and the tier page reads,
/// `<model>\t<label>` lines. A network call, so it is never run where a
/// page waits for it.
pub fn models_file() -> PathBuf {
    crate::paths::state_root().unwrap_or_else(|_| std::env::temp_dir()).join("agy-models.tsv")
}

/// How old the models file may be before a daemon reads `agy models` again.
pub const MODELS_TTL_MS: u64 = 6 * 60 * 60 * 1000;

/// `agy models` takes a second or two on this machine; past this it is not
/// answering.
pub const MODELS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether the models file is missing or older than [`MODELS_TTL_MS`].
pub fn models_due(file: &Path, now_ms: u64) -> bool {
    let modified = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(mesimon_core::clock::epoch_ms);
    modified.is_none_or(|at| now_ms.saturating_sub(at) >= MODELS_TTL_MS)
}

/// The models file's text for what `agy models` printed: the lines
/// [`mesimon_core::tier::agy_variants`] reads, one per variant, written back
/// in its words, so the file holds nothing agy said beyond them; the page
/// groups them as it reads. `None` when it listed no model, which leaves
/// the last good file in place.
pub fn models_text(printed: &str) -> Option<String> {
    let list = mesimon_core::tier::agy_variants(printed)?;
    Some(list.iter().map(|m| format!("{}\t{}\n", m.model, m.name)).collect())
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

/// One `mesimon gate --provider agy` command line as agy runs it, through `sh -c`.
fn gate_command(
    bin: &Path,
    sock: &Path,
    board_dir: &Path,
    state_dir: &Path,
    session: uuid::Uuid,
) -> String {
    use mesimon_core::text::sh_quote;
    let allow = state_dir.join(crate::paths::WORKTREES_DIR);
    format!(
        "{} gate --provider agy --session {session} --sock {} --deny-board {} --deny-state {} --allow {}",
        sh_quote(&bin.display().to_string()),
        sh_quote(&sock.display().to_string()),
        sh_quote(&board_dir.display().to_string()),
        sh_quote(&state_dir.display().to_string()),
        sh_quote(&allow.display().to_string()),
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
/// lifecycle event and the grouped `matcher` form on `PreToolUse` (the write
/// gate first, T-1238, and the silent pending-call observer second, T-1235).
/// With the brief on, `SessionStart` runs a second handler after the observer,
/// the one that prints; `mesimon hook` prints nothing.
fn render_hooks(
    bin: &Path,
    sock: &Path,
    board_dir: &Path,
    state_dir: &Path,
    session: uuid::Uuid,
    brief: bool,
) -> Value {
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
    events.insert(
        "PreToolUse".into(),
        json!([
            {
                "matcher": GATE_MATCHER,
                "hooks": [{
                    "type": "command",
                    "command": gate_command(bin, sock, board_dir, state_dir, session),
                    "timeout": HOOK_TIMEOUT_SECS,
                }],
            },
            {
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": hook_command(bin, sock, session, PRE_TOOL_USE),
                    "timeout": HOOK_TIMEOUT_SECS,
                }],
            },
        ]),
    );
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
    if context.managed_git_worktree && trusts_checkout(trusted, checkout) {
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
    // agy's Default mode (T-1234): `--mode` refuses the word, and the
    // settings' empty `agentMode` is how agy itself saves Default.
    if !context.plan && context.column.agy_mode == mesimon_core::board::AgyMode::Default {
        settings["agentMode"] = json!("");
    }
    settings
}

fn write_private(path: &Path, value: &Value) -> Result<(), String> {
    hook_settings::write_private(path, value).map_err(|e| format!("{}: {e}", path.display()))
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
    let hooks = render_hooks(
        &bin,
        &paths.hook_sock(),
        &paths.board_dir,
        &paths.state_dir,
        session,
        context.brief && tools,
    );
    write_private(&agents.join("hooks.json"), &hooks)?;
    // Rewritten or removed on every launch: a wake follows the column's tier
    // as it stands then.
    let mcp = agents.join("mcp_config.json");
    if tools {
        let mut config = hook_settings::mcp_config(paths, &bin, session, context.tools);
        if context.brief {
            // The first turn needs this tool before it has read any schema.
            config["mcpServers"][mesimon_core::mcp::SERVER_NAME]["tools"] =
                json!({"get_ticket": {"eager": true}});
        }
        write_private(&mcp, &config)?;
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

/// The conversation a hook or title frame names, if it names a valid one.
pub(crate) fn frame_conversation(frame: &HookFrame) -> Option<&str> {
    frame
        .payload
        .get("conversationId")
        .or_else(|| frame.payload.get("conversation_id"))
        .and_then(Value::as_str)
        .and_then(conversation_id)
}

/// The pending call an agy `PreToolUse` frame names (`toolCall {id, name,
/// args}`, T-1235), within the bounds Remote Control takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

pub(crate) fn parse_tool_call(payload: &Value) -> Option<ToolCall> {
    if serde_json::to_vec(payload).map_or(true, |b| b.len() > 16 * 1024) {
        return None;
    }
    let call = payload.get("toolCall")?;
    let id = call.get("id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256)?;
    let name =
        call.get("name").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 256)?;
    let args = call.get("args").filter(|a| a.is_object())?.clone();
    Some(ToolCall { id: id.into(), name: name.into(), args })
}

/// The context window a title report names for its model
/// (`context_window.context_window_size`): 0 until the model is known.
pub fn context_window(feed: &Value) -> Option<u64> {
    feed.get("context_window")?.get("context_window_size")?.as_u64().filter(|w| *w > 0)
}

/// A transcript path as a hook names it: absolute, a JSONL file, one line
/// of no control characters. A frame is any same-uid process's to send, and
/// the path is only ever read.
fn transcript_path(word: &str) -> bool {
    word.len() <= 4096
        && word.starts_with('/')
        && word.ends_with(".jsonl")
        && !word.chars().any(char::is_control)
}

/// Older boards saved the lead id beside whichever child last reported.
/// Correct that known CLI layout on load, even for a seat that stays asleep.
pub(crate) fn repair_saved_transcript(record: &mut SessionRecord) {
    let (Some(lead), Some(path)) = (&record.agy_conversation_id, &record.transcript_path) else {
        return;
    };
    let brain = home().join("brain");
    let Ok(relative) = Path::new(path).strip_prefix(&brain) else { return };
    let mut parts = relative.iter();
    let Some(owner) = parts.next().and_then(|name| name.to_str()) else { return };
    let log = Path::new(".system_generated/logs/transcript_full.jsonl");
    if owner != lead
        && parts.as_path() == log
        && uuid::Uuid::parse_str(owner).is_ok()
        && uuid::Uuid::parse_str(lead).is_ok()
    {
        record.transcript_path = Some(brain.join(lead).join(log).display().to_string());
    }
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
    let mut argv = vec![
        super::cli_bin(AgentProvider::Agy),
        "--add-dir".into(),
        dir.display().to_string(),
        "--settings".into(),
        settings.display().to_string(),
    ];
    // Plan mode for this launch (T-434) outranks the column's word
    // (T-1234), which outranks the person's own default: no flag at all.
    // Default rides the settings file instead (`render_settings`).
    let mode = if context.plan { Some("plan") } else { context.column.agy_mode.flag_word() };
    if let Some(mode) = mode {
        argv.extend(["--mode".into(), mode.into()]);
    }
    // A model carries its level in agy's own variant id (T-1326); only a
    // tier with no model names a level alone, for agy's own model.
    if context.tier.provider == AgentProvider::Agy {
        match context.tier.agy_model_arg() {
            Some(model) => argv.extend(["--model".into(), model]),
            None => {
                if let Some(effort) = context.tier.effort_arg() {
                    argv.extend(["--effort".into(), effort.into()]);
                }
            }
        }
    }
    if let Some(id) = conversation.and_then(conversation_id) {
        argv.extend(["--conversation".into(), id.into()]);
    }
    Ok(argv)
}

/// What a `Stop` says. `fullyIdle: false` is a turn whose background tasks
/// still run: the seat stays working, and the stop that follows them ends
/// it.
fn stop_signal(payload: &Value, record: &mut SessionRecord) -> Option<Signal> {
    record.agy_stop = AgyStop::Reported;
    if payload.get("fullyIdle").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let outcome = match payload.get("terminationReason").and_then(Value::as_str) {
        Some("USER_CANCELED") => TurnOutcome::Interrupted,
        Some(
            "ERROR" | "MAX_INVOCATIONS" | "MAX_FORCED_INVOCATIONS" | "MAX_TOKEN_BUDGET_EXCEEDED",
        ) => TurnOutcome::Failed(FailReason::Unknown),
        _ if record.agy_background_running => {
            record.agy_stop = AgyStop::WaitingForIdle;
            return None;
        }
        _ => {
            record.agy_stop = AgyStop::Completed;
            TurnOutcome::Completed
        }
    };
    Some(Signal::TurnEnded { outcome })
}

/// What a title report says, against the state the record holds. A pending
/// confirmation is agy asking the person; its answer shows as the agent at
/// work again. `idle` is the prompt box of a pane that just came up (agy's
/// `SessionStart` waits for the first prompt). A turn's end is `Stop`'s to
/// say, but for one: **an Esc fires no `Stop`** (1.5.9, T-1233: the turn
/// cut, "Interrupted" on screen, and the feed alone going `idle`), so an
/// `idle` on a running or asking turn with no background work or `Stop` is that
/// interrupt. A completed turn's `Stop` comes ~300 ms ahead of its `idle`
/// (measured), inside the leave's settle, so the record still reads the turn
/// running then: the flag, not the state, says the end was stated.
fn title_signal(payload: &Value, record: &mut SessionRecord) -> Option<Signal> {
    let previous_background = record.agy_background_running;
    record.agy_background_running = payload["subagents"].as_array().is_some_and(|agents| {
        agents.iter().any(|agent| agent["status"].as_str() == Some("running"))
    }) || payload["task_count"].as_u64().unwrap_or(0) > 0;
    if record.agy_background_running && record.agy_stop == AgyStop::Completed {
        record.agy_stop = AgyStop::WaitingForIdle;
    }
    if payload.get("tool_confirmation_pending").and_then(Value::as_bool) == Some(true) {
        return Some(Signal::Attention { reason: Reason::Permission });
    }
    let state = &record.state;
    let coming_up = matches!(
        state,
        SessionState::Spawning
            | SessionState::Unknown {
                reason: UnknownReason::DaemonRestarted | UnknownReason::ObservationLost
            }
    );
    let asking_permission =
        matches!(state, SessionState::RequiresAction { reason: Reason::Permission });
    let asking_question =
        matches!(state, SessionState::RequiresAction { reason: Reason::Question });
    let asked = asking_permission || asking_question;
    let turning = *state == SessionState::Running || asked;
    let agent_state = payload.get("agent_state").and_then(Value::as_str);
    if record.agy_background_running {
        // The lead can be idle while its children or background tasks work.
        // The first such report also cancels a leave still settling.
        // `ask_question` fires `agent_state: "tool_use"` ~280 ms after its
        // `PreToolUse` while the dialog stands (measured on 1.5.13, T-1235),
        // so only `"working" | "thinking"` resumes a question stop.
        let resumed_question =
            asking_question && matches!(agent_state, Some("working" | "thinking"));
        return (coming_up
            || asking_permission
            || resumed_question
            || matches!(state, SessionState::Idle { .. })
            || (*state == SessionState::Running && !previous_background))
            .then_some(Signal::TurnStarted);
    }
    match agent_state? {
        "idle" | "reviewing" if coming_up => {
            record.agy_stop = AgyStop::None;
            Some(Signal::Ready)
        }
        "idle" if turning && record.agy_stop == AgyStop::WaitingForIdle => {
            record.agy_stop = AgyStop::Completed;
            Some(Signal::TurnEnded { outcome: TurnOutcome::Completed })
        }
        "idle"
            if turning && matches!(record.agy_stop, AgyStop::None | AgyStop::InferredInterrupt) =>
        {
            record.agy_stop = AgyStop::InferredInterrupt;
            Some(Signal::TurnEnded { outcome: TurnOutcome::Interrupted })
        }
        "working" | "thinking" | "tool_use"
            if record.agy_stop == AgyStop::InferredInterrupt
                && (turning || matches!(state, SessionState::Idle { .. })) =>
        {
            record.agy_stop = AgyStop::None;
            Some(Signal::TurnStarted)
        }
        "working" | "thinking"
            if coming_up
                || asked
                || (matches!(state, SessionState::Idle { .. })
                    && record.agy_stop == AgyStop::None) =>
        {
            Some(Signal::TurnStarted)
        }
        "tool_use"
            if coming_up
                || asking_permission
                || (matches!(state, SessionState::Idle { .. })
                    && record.agy_stop == AgyStop::None) =>
        {
            Some(Signal::TurnStarted)
        }
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
        let named = frame_conversation(frame);
        if let Some(id) = named {
            if record.agy_conversation_id.as_deref() != Some(id) {
                // Children inherit the seat's hooks. Only the terminal's title
                // can switch an established lead conversation (/clear, /resume).
                if record.agy_conversation_id.is_some() {
                    if frame.event != TITLE {
                        return result;
                    }
                    record.transcript_path = None;
                    record.agy_stop = AgyStop::None;
                    record.agy_background_running = false;
                }
                record.agy_conversation_id = Some(id.to_string());
                result.metadata_changed = true;
            }
        }
        // The transcript from a hook alone: the title feed's path is wrong.
        if frame.event != TITLE {
            if let Some(path) = payload.get("transcriptPath").and_then(Value::as_str) {
                if transcript_path(path) && record.transcript_path.as_deref() != Some(path) {
                    record.transcript_path = Some(path.to_string());
                    result.metadata_changed = true;
                }
            }
        }
        match frame.event.as_str() {
            // The first model call of a turn is its prompt taken; a later
            // one is the same turn going on.
            PRE_INVOCATION => {
                let first = payload.get("invocationNum").and_then(Value::as_u64).unwrap_or(0) == 0;
                record.agy_stop = AgyStop::None;
                result.signal =
                    Some(if first { Signal::UserPromptSubmit } else { Signal::TurnStarted });
            }
            // `ask_question` sets no `tool_confirmation_pending` on the title
            // feed (measured on 1.5.13, T-1235): `PreToolUse` is its entry,
            // and the feed's return to `working` is its answer.
            PRE_TOOL_USE => {
                if parse_tool_call(payload).is_some_and(|c| c.name == "ask_question") {
                    result.signal = Some(Signal::Attention { reason: Reason::Question });
                }
            }
            STOP => {
                result.signal = stop_signal(payload, record);
                if matches!(
                    result.signal,
                    Some(Signal::TurnEnded { outcome: TurnOutcome::Failed(_) })
                ) {
                    result.detail = payload
                        .get("error")
                        .and_then(Value::as_str)
                        .and_then(super::detail_text)
                        .or_else(|| {
                            payload
                                .get("terminationReason")
                                .and_then(Value::as_str)
                                .and_then(super::detail_text)
                        });
                }
            }
            TITLE => result.signal = title_signal(payload, record),
            _ => {}
        }
        result
    }

    fn preview(&self, path: &Path) -> Option<AgentPreview> {
        history::preview(path)
    }

    fn rows(&self, at: u64, record: &Value) -> Vec<mesimon_core::mesophon::TranscriptRow> {
        history::rows(at, record)
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

    /// Measured on agy 1.5.13 (`presence/presence.go`, `presence/lock_unix.go`,
    /// T-1238): `presence/<conversation>.lock` under agy's home is an empty
    /// BSD `flock(LOCK_EX | LOCK_NB)` file with no PID written inside it.
    /// Probing it takes the exclusive lock (racing agy's own `presence.Acquire`
    /// and touching `~/.gemini`), cannot distinguish `own_pane` from another
    /// process without an owning PID, and agy itself refuses a second writer
    /// at `--conversation` startup (`presence: held by another process`).
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

    /// A repository and a worktree folder beside it, with the board's state
    /// under the same temporary root: `(root, paths, tree)`.
    fn bed() -> (tempfile::TempDir, crate::paths::Paths, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let (repo, tree) = (root.path().join("repo"), root.path().join("tree"));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        let mut paths = crate::paths::Paths::for_repo(&repo).unwrap();
        paths.state_dir = root.path().join("state");
        std::fs::create_dir_all(&paths.state_dir).unwrap();
        (root, paths, tree)
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    const ID: &str = "e491708f-fd93-4aec-9a23-1ad87af7c910";

    #[test]
    fn agy_recovers_a_sleeping_leads_transcript_on_board_load() {
        let (_root, paths, _) = bed();
        std::fs::create_dir_all(paths.board_dir.join("board")).unwrap();
        let mut seat = record(SessionState::Sleeping);
        let brain = home().join("brain");
        let log = ".system_generated/logs/transcript_full.jsonl";
        seat.agy_conversation_id = Some(ID.into());
        seat.transcript_path =
            Some(brain.join(uuid::Uuid::nil().to_string()).join(log).display().to_string());
        let mut board = mesimon_core::board::Board::with_default_columns();
        board.sessions.push(seat);
        crate::store::save_sessions(&paths, &board).unwrap();
        let loaded = crate::store::load(&paths).unwrap();
        assert_eq!(
            loaded.board.sessions[0].transcript_path.as_deref(),
            brain.join(ID).join(log).to_str()
        );
    }

    #[test]
    fn agy_resuming_before_an_inferred_interrupt_settles_keeps_the_turn() {
        use mesimon_core::attention::{Machine, SETTLE_MS};
        let mut rec = record(SessionState::Running);
        let mut machine = Machine::new(SessionState::Running, 0);
        for (at, state) in [(1, "idle"), (301, "working")] {
            let title = frame(TITLE, json!({"conversation_id": ID, "agent_state": state}));
            if let Some(signal) = Agy.parse_hook(&title, &mut rec).signal {
                machine.apply(&signal, at);
            }
        }
        assert!(machine.tick(1 + SETTLE_MS).is_none());
        assert_eq!(machine.state(), &SessionState::Running);
    }

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
        // A partial stop and the lead's idle while background work runs
        // neither end the seat nor report an Esc.
        let busy = frame(STOP, json!({"terminationReason": "NO_TOOL_CALL", "fullyIdle": false}));
        let _ = Agy.parse_hook(&invoked, &mut rec);
        assert_eq!(Agy.parse_hook(&busy, &mut rec).signal, None);
        assert_eq!(Agy.parse_hook(&idle, &mut rec).signal, None);
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
    fn only_the_leads_stop_and_a_quiet_roster_finish_the_seat() {
        let parent = "/tmp/lead.jsonl";
        let mut rec = record(SessionState::Running);
        let main = frame(PRE_INVOCATION, json!({"conversationId": ID, "transcriptPath": parent}));
        let _ = Agy.parse_hook(&main, &mut rec);
        let title = |status| {
            frame(
                TITLE,
                json!({
                    "conversation_id": ID, "agent_state": "idle",
                    "subagents": [{"status": "completed"}, {"status": status}]
                }),
            )
        };
        let busy = title("running");
        assert_eq!(Agy.parse_hook(&busy, &mut rec).signal, Some(Signal::TurnStarted));
        let child = json!({"conversationId": "abc123", "transcriptPath": "/tmp/child.jsonl",
                           "fullyIdle": true, "terminationReason": "NO_TOOL_CALL"});
        for event in [SESSION_START, PRE_INVOCATION, STOP] {
            let child = Agy.parse_hook(&frame(event, child.clone()), &mut rec);
            assert_eq!(child.signal, None);
            assert!(!child.metadata_changed);
        }
        assert_eq!(rec.agy_conversation_id.as_deref(), Some(ID));
        assert_eq!(rec.transcript_path.as_deref(), Some(parent));
        assert_eq!(Agy.parse_hook(&busy, &mut rec).signal, None);
        let stop = frame(STOP, json!({"conversationId": ID, "fullyIdle": true}));
        assert_eq!(Agy.parse_hook(&stop, &mut rec).signal, None);
        let quiet = title("completed");
        assert_eq!(
            Agy.parse_hook(&quiet, &mut rec).signal,
            Some(Signal::TurnEnded { outcome: TurnOutcome::Completed })
        );
        assert_eq!(Agy.parse_hook(&quiet, &mut rec).signal, None);

        // The title can expose a background task just after a successful
        // stop. Its running signal cancels the settling leave. A new lead
        // invocation then supersedes that stop before the task drains.
        let task = frame(TITLE, json!({"agent_state": "idle", "task_count": 1}));
        assert_eq!(Agy.parse_hook(&task, &mut rec).signal, Some(Signal::TurnStarted));
        let _ = Agy.parse_hook(&main, &mut rec);
        let working = frame(TITLE, json!({"agent_state": "working"}));
        assert_eq!(Agy.parse_hook(&working, &mut rec).signal, None);
        assert_eq!(rec.agy_stop, AgyStop::None);

        // An in-pane conversation switch is confirmed by the title even
        // when the new conversation's first invocation beat that report.
        rec.state = SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn };
        let switched = frame(TITLE, json!({"agent_state": "working", "conversation_id": "fed456"}));
        assert_eq!(Agy.parse_hook(&switched, &mut rec).signal, Some(Signal::TurnStarted));
        assert_eq!(rec.agy_conversation_id.as_deref(), Some("fed456"));
        assert_eq!(rec.transcript_path, None);
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

    /// T-1237: the transcript is the hooks' path, recorded on 1.5.9; the
    /// title feed's names a folder the CLI never writes and is never kept.
    #[test]
    fn the_transcript_is_the_hooks_path_and_never_the_title_feeds() {
        let start: Value =
            serde_json::from_str(include_str!("agy/fixtures/sessionstart.json")).unwrap();
        let title: Value = serde_json::from_str(include_str!("agy/fixtures/title.json")).unwrap();
        let mut rec = record(SessionState::Spawning);
        let seen = Agy.parse_hook(&frame(TITLE, title.clone()), &mut rec);
        assert_eq!(rec.transcript_path, None, "the feed's path is wrong for the CLI");
        assert!(seen.metadata_changed, "its conversation id is learned");
        let seen = Agy.parse_hook(&frame(SESSION_START, start.clone()), &mut rec);
        let want = start["transcriptPath"].as_str().unwrap().to_string();
        let want = want.as_str();
        assert!(want.ends_with("/.system_generated/logs/transcript_full.jsonl"));
        assert_eq!(rec.transcript_path.as_deref(), Some(want));
        assert!(seen.metadata_changed);
        // The same path again changes nothing; a later title report leaves it.
        let again = Agy.parse_hook(&frame(SESSION_START, start), &mut rec);
        assert!(!again.metadata_changed);
        let _ = Agy.parse_hook(&frame(TITLE, title), &mut rec);
        assert_eq!(rec.transcript_path.as_deref(), Some(want));
        // A path that is not an absolute JSONL file is never kept.
        for bad in ["relative/t.jsonl", "/tmp/t.json", "/tmp/a\nb.jsonl"] {
            let mut fresh = record(SessionState::Spawning);
            let _ = Agy.parse_hook(&frame(STOP, json!({"transcriptPath": bad})), &mut fresh);
            assert_eq!(fresh.transcript_path, None, "{bad}");
        }
    }

    /// T-1237: the title feed, recorded on 1.5.9 and scrubbed, names the
    /// window and carries the plan's quota as shares used; a report before
    /// the model is known names no window and no quota.
    #[test]
    fn the_title_feed_names_the_window_and_the_plans_quota() {
        use mesimon_core::usage::{parse_agy, Severity, WindowKind};
        let title: Value = serde_json::from_str(include_str!("agy/fixtures/title.json")).unwrap();
        assert!(title.get("email").is_none(), "the fixture holds no sign-in");
        assert_eq!(context_window(&title), Some(1_048_576));
        let now = mesimon_core::usage::rfc3339_ms("2026-11-14T14:33:42Z").unwrap();
        let r = parse_agy(&title, now).unwrap();
        assert_eq!(r.plan.as_deref(), Some("Google AI Pro"));
        let got: Vec<(WindowKind, &str, String)> =
            r.windows.iter().map(|w| (w.kind, w.label.as_str(), w.percent_word())).collect();
        assert_eq!(
            got,
            vec![
                (WindowKind::Session, "5h", "1%".to_string()),
                (WindowKind::Weekly, "week", "0%".to_string()),
                (WindowKind::Model, "3p 5h", "0%".to_string()),
                (WindowKind::Model, "3p", "0%".to_string()),
            ]
        );
        assert!(r.windows[0].headline, "the fullest is the headline");
        assert_eq!(
            r.windows[0].resets_at_ms,
            mesimon_core::usage::rfc3339_ms("2026-11-14T18:42:35Z")
        );
        assert!(r.windows.iter().all(|w| w.severity == Severity::Normal));
        let mut full = title.clone();
        full["quota"]["gemini-5h"]["remaining_fraction"] = json!(0.03);
        let hot = parse_agy(&full, now).unwrap();
        assert_eq!(hot.windows[0].severity, Severity::Critical);

        let early: Value =
            serde_json::from_str(include_str!("agy/fixtures/title-signing-in.json")).unwrap();
        assert_eq!(context_window(&early), None);
        assert_eq!(parse_agy(&early, now), None);
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
        let (_root, paths, tree) = bed();
        let tier = Tier {
            id: "A".into(),
            name: "flash".into(),
            provider: AgentProvider::Agy,
            model: "gemini-3.8-flash".into(),
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
        // T-1326: the level rides agy's variant id, never `--effort` beside it.
        assert_eq!(pair(&started, "--model"), ["gemini-3.8-flash-high"]);
        assert!(pair(&started, "--effort").is_empty(), "{started:?}");
        assert!(pair(&started, "--conversation").is_empty(), "{started:?}");
        assert!(!started.iter().any(|a| a.contains("dangerously")), "{started:?}");

        let hooks = read_json(&dir.join(".agents/hooks.json"));
        let ours = hooks["mesimon"].as_object().unwrap();
        assert_eq!(ours.len(), 4, "{hooks}");
        for (event, name) in HOOKS {
            let command = ours[event][0]["command"].as_str().unwrap();
            assert!(command.ends_with(&format!("--session {} --event {name}", ctx.session)));
            assert!(command.contains(" hook --sock '"), "{command}");
        }
        // `PreToolUse` carries the write gate (T-1238) and the silent observer
        // (T-1235) in agy's grouped `matcher` form; `PostToolUse` is absent.
        assert!(ours.get("PostToolUse").is_none());
        let pre = ours["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "{hooks}");
        assert_eq!(pre[0]["matcher"], GATE_MATCHER);
        let gate_cmd = pre[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(gate_cmd.contains(" gate --provider agy "), "{gate_cmd}");
        assert!(gate_cmd.contains(&format!("--session {}", ctx.session)), "{gate_cmd}");
        assert!(
            gate_cmd.contains(&format!("--deny-board '{}'", paths.board_dir.display())),
            "{gate_cmd}"
        );
        assert!(
            gate_cmd.contains(&format!("--deny-state '{}'", paths.state_dir.display())),
            "{gate_cmd}"
        );
        assert!(
            gate_cmd.contains(&format!(
                "--allow '{}'",
                paths.state_dir.join(crate::paths::WORKTREES_DIR).display()
            )),
            "{gate_cmd}"
        );
        assert_eq!(pre[1]["matcher"], "*");
        let pre_cmd = pre[1]["hooks"][0]["command"].as_str().unwrap();
        assert!(pre_cmd.ends_with(&format!("--session {} --event {PRE_TOOL_USE}", ctx.session)));
        assert!(pre_cmd.contains(" hook --sock '"), "{pre_cmd}");
        // A column without the board's tools: no server, no rule, no brief.
        assert_eq!(ours["SessionStart"].as_array().unwrap().len(), 1, "{hooks}");
        assert!(!dir.join(".agents/mcp_config.json").exists());
        let file = read_json(&settings);
        assert!(file["title"]["command"].as_str().unwrap().ends_with(TITLE));
        assert!(file.get("permissions").is_none(), "{file}");
        // Nobody trusted the checkout: only mesimon's own folder is named.
        assert_eq!(file["trustedWorkspaces"], json!([dir.display().to_string()]));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // No model: the level goes alone, for agy's own model.
        let own = Tier { effort: Effort::Low, ..Tier::builtin(AgentProvider::Agy) };
        let started = launch_with(&context(&paths, &tree, own), None, &[]).unwrap();
        assert!(pair(&started, "--model").is_empty(), "{started:?}");
        assert_eq!(pair(&started, "--effort"), ["low"]);

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
    }

    /// The board's tools and the brief (T-1232): the shim's server beside
    /// the hooks, one allow rule for it, and the brief as a second
    /// `SessionStart` handler; a wake into a column without tools takes all
    /// three away again.
    #[test]
    fn a_launch_with_the_board_s_tools_writes_the_server_the_rule_and_the_brief() {
        let (_root, paths, _tree) = bed();
        let repo = paths.repo_root.clone();
        let tier = Tier::builtin(AgentProvider::Agy);
        let ctx = LaunchContext {
            tools: AgentTools::Full,
            brief: true,
            ..context(&paths, &repo, tier.clone())
        };
        launch_with(&ctx, None, &[]).unwrap();
        let dir = session_dir(&paths, ctx.session);
        let config = read_json(&dir.join(".agents/mcp_config.json"));
        let server = &config["mcpServers"][mesimon_core::mcp::SERVER_NAME];
        let args: Vec<String> = server["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap().to_string())
            .collect();
        assert_eq!(args[0], "mcp");
        assert_eq!(pair(&args, "--sock"), [paths.orch_sock().display().to_string()]);
        assert_eq!(pair(&args, "--session"), [ctx.session.to_string()]);
        assert_eq!(pair(&args, "--tools"), [AgentTools::Full.word()]);
        assert_eq!(server["tools"], json!({"get_ticket": {"eager": true}}));
        use std::os::unix::fs::PermissionsExt;
        let mode =
            std::fs::metadata(dir.join(".agents/mcp_config.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        let settings = read_json(&settings_file(&paths, ctx.session));
        assert_eq!(settings["permissions"]["allow"], json!(["mcp(mesimon/*)"]));

        let hooks = read_json(&dir.join(".agents/hooks.json"));
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
        let hooks = read_json(&dir.join(".agents/hooks.json"));
        assert_eq!(hooks["mesimon"]["SessionStart"].as_array().unwrap().len(), 1);
        let config = read_json(&dir.join(".agents/mcp_config.json"));
        assert!(config["mcpServers"][mesimon_core::mcp::SERVER_NAME].get("tools").is_none());

        // A wake into a column without the tools: no server, no rule, no brief.
        let off =
            LaunchContext { tools: AgentTools::Off, brief: true, ..context(&paths, &repo, tier) };
        launch_with(&off, Some(ID), &[]).unwrap();
        assert!(!dir.join(".agents/mcp_config.json").exists());
        assert!(read_json(&settings_file(&paths, ctx.session)).get("permissions").is_none());
        let hooks = read_json(&dir.join(".agents/hooks.json"));
        assert_eq!(hooks["mesimon"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    /// T-1234: the column's agy mode rides `--mode` for agy's three words;
    /// Default, which `--mode` refuses, rides the session's settings as
    /// agy saves it; inherit passes nothing; a launch in plan mode outranks
    /// the column. Claude's mode is never read.
    #[test]
    fn the_columns_agy_mode_rides_the_flag_or_the_settings() {
        use mesimon_core::board::{AgyMode, ClaudeMode};
        let (_root, paths, tree) = bed();
        let launched = |agy_mode: AgyMode, plan: bool| {
            let column =
                ColumnSettings { agy_mode, claude_mode: ClaudeMode::Plan, ..Default::default() };
            let ctx = LaunchContext {
                column,
                plan,
                ..context(&paths, &tree, Tier::builtin(AgentProvider::Agy))
            };
            let argv = launch_with(&ctx, None, &[]).unwrap();
            let file = read_json(&settings_file(&paths, ctx.session));
            let mode: Vec<String> = pair(&argv, "--mode").into_iter().map(str::to_string).collect();
            (mode, file.get("agentMode").cloned())
        };
        assert_eq!(launched(AgyMode::Inherit, false), (vec![], None), "Claude's plan is not agy's");
        assert_eq!(launched(AgyMode::Default, false), (vec![], Some(json!(""))));
        assert_eq!(launched(AgyMode::AcceptEdits, false), (vec!["accept-edits".into()], None));
        assert_eq!(launched(AgyMode::Auto, false), (vec!["auto".into()], None));
        assert_eq!(launched(AgyMode::Plan, false), (vec!["plan".into()], None));
        assert_eq!(launched(AgyMode::Default, true), (vec!["plan".into()], None));
        assert_eq!(launched(AgyMode::Auto, true), (vec!["plan".into()], None));
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

    /// T-1234: the models file keeps the lines agy listed, in its words, and
    /// is due when missing or past its TTL.
    #[test]
    fn the_models_file_is_agys_list_and_comes_due() {
        let printed =
            "Fetching available models...\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n";
        assert_eq!(
            models_text(printed).as_deref(),
            Some("gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n")
        );
        assert_eq!(models_text("Fetching available models...\n"), None);
        let file = std::env::temp_dir().join(format!("msmn-agy-models-{}", uuid::Uuid::new_v4()));
        let now = mesimon_core::clock::epoch_ms(std::time::SystemTime::now()).unwrap();
        assert!(models_due(&file, now), "missing");
        std::fs::write(&file, "x\ty\n").unwrap();
        assert!(!models_due(&file, now));
        assert!(models_due(&file, now + MODELS_TTL_MS + 60_000), "the write is after `now`");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_worktree_trust_stop_names_only_the_missing_checkout_approval() {
        let home = tempfile::tempdir().unwrap();
        let checkout = Path::new("/project/main");
        let tree = Path::new("/state/worktrees/T-1");
        let stop = |folder| trust_detail(checkout, Some(tree), folder, home.path());
        let generic = "agy asks to trust this folder";
        let put_trust = |path: &Path| {
            std::fs::write(
                home.path().join("settings.json"),
                json!({"trustedWorkspaces": [path]}).to_string(),
            )
            .unwrap();
        };
        put_trust(checkout);
        assert_eq!(stop(tree), generic);
        put_trust(checkout.parent().unwrap());
        assert_eq!(stop(tree), "agy asks to trust this folder ∙ Trusting /project/main once in agy covers this board's worktrees");
        assert_eq!(stop(Path::new("/other")), generic);
        assert_eq!(trust_detail(checkout, None, tree, home.path()), generic);
    }

    /// Decision 3: a ticket's worktree is passed as trusted only where the
    /// person already trusts its main checkout, by exact path, and the
    /// person's own list is carried whole.
    #[test]
    fn a_worktree_is_trusted_only_where_the_person_trusts_its_checkout() {
        let (root, paths, tree) = bed();
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
        let parent = root.path().display().to_string();
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
    }

    /// T-1235: `ask_question` enters `Reason::Question` on `PreToolUse`, holds
    /// through the `tool_use` title report ~280 ms later (even with background
    /// tasks running), resumes on `working`, and interrupts on an `idle` with
    /// no `Stop`; a child subagent's `PreToolUse` is ignored.
    #[test]
    fn ask_question_holds_through_tool_use_and_resumes_on_working() {
        let mut rec = record(SessionState::Running);
        rec.agy_conversation_id = Some(ID.into());
        let ask = |conv: &str, name: &str| {
            frame(
                PRE_TOOL_USE,
                json!({
                    "conversationId": conv,
                    "hookEventName": "PreToolUse",
                    "toolCall": {
                        "id": "call_1",
                        "name": name,
                        "args": {"questions": [{"question": "Which?", "options": ["A", "B"]}]},
                    },
                }),
            )
        };
        assert_eq!(Agy.parse_hook(&ask("abc123", "ask_question"), &mut rec).signal, None);
        assert_eq!(Agy.parse_hook(&ask(ID, "run_command"), &mut rec).signal, None);
        assert_eq!(
            Agy.parse_hook(&ask(ID, "ask_question"), &mut rec).signal,
            Some(Signal::Attention { reason: Reason::Question })
        );
        rec.state = SessionState::RequiresAction { reason: Reason::Question };
        // `tool_use` arrives right after `PreToolUse` while the dialog stands.
        let tool_use = frame(TITLE, json!({"conversation_id": ID, "agent_state": "tool_use"}));
        assert_eq!(Agy.parse_hook(&tool_use, &mut rec).signal, None);
        let bg_tool_use = frame(
            TITLE,
            json!({"conversation_id": ID, "agent_state": "tool_use", "task_count": 1}),
        );
        assert_eq!(Agy.parse_hook(&bg_tool_use, &mut rec).signal, None);
        // Answering returns the feed to `working` (with or without tasks).
        let bg_working =
            frame(TITLE, json!({"conversation_id": ID, "agent_state": "working", "task_count": 1}));
        assert_eq!(Agy.parse_hook(&bg_working, &mut rec).signal, Some(Signal::TurnStarted));
        let working = frame(TITLE, json!({"conversation_id": ID, "agent_state": "working"}));
        assert_eq!(Agy.parse_hook(&working, &mut rec).signal, Some(Signal::TurnStarted));
        // An interrupt to `idle` with no `Stop` ends the turn as interrupted.
        let idle = frame(TITLE, json!({"conversation_id": ID, "agent_state": "idle"}));
        assert_eq!(
            Agy.parse_hook(&idle, &mut rec).signal,
            Some(Signal::TurnEnded { outcome: TurnOutcome::Interrupted })
        );
    }
}
