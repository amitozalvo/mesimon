//! Per-session Claude settings generator (11 §11.2.1/§11.2.3, D33b paths).
//!
//! Everything mesimon injects into a session it spawns lives here: the hook
//! settings file, and (T-84) the MCP server config that rides on argv.
//!
//! One JSON file per spawned Claude session, mode 0600, absolute paths only,
//! passed as `--settings`. Hooks MERGE with the user's own. The generator
//! encodes 11 §11.2.3's silent-failure traps as hard rules, each unit-tested:
//! `if` is never emitted (silently disables non-tool events), matchers only on
//! events that support them, `async: true` only where it cannot block, and
//! `timeout: 2` for observers and the gate; a remote decision holds as long as
//! the dialog stands, within `PERMISSION_HOLD_SECS` (T-632).
//!
//! D15's attention hooks
//! are pure observers: they exec `mesimon hook`, which never writes stdout and
//! always exits 0, and they can only ever report. The static deciding hook execs
//! `mesimon gate`, which may answer "deny" and nothing else — a separate
//! subcommand precisely so the observer's never-writes-stdout invariant stays
//! true and stays testable. They share the `PreToolUse` key with disjoint
//! matchers; no tool matches both. `mesimon approve` separately bridges a
//! one-shot paired-human PermissionRequest answer; it never installs rules.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::paths::Paths;
use mesimon_core::mesophon::PERMISSION_HOLD_SECS;

const SESSION_START_SOURCES: [&str; 5] = ["startup", "resume", "clear", "compact", "fork"];
const SESSION_END_REASONS: [&str; 5] = ["clear", "resume", "logout", "prompt_input_exit", "other"];
const STOP_FAILURE_MATCHERS: [&str; 10] = [
    "rate_limit",
    "overloaded",
    "authentication_failed",
    "oauth_org_not_allowed",
    "billing_error",
    "invalid_request",
    "model_not_found",
    "server_error",
    "max_output_tokens",
    "unknown",
];
/// Matcherless single entries. The verbose tier (PostToolBatch,
/// MessageDisplay, broad PreToolUse) is deliberately absent (11 §11.2.5);
/// broad PostToolUse is the one exception — it is the permission-accept
/// clear path (STALE-MAP deviation, dogfood 2026-08-30).
const SINGLE_EVENTS: [&str; 12] = [
    "UserPromptSubmit",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "TeammateIdle",
    "PermissionRequest",
    "PermissionDenied",
    "Notification", // one entry, NO matcher — daemon discriminates on notification_type
    "Elicitation",
    "ElicitationResult",
    "PreCompact",
    "PostCompact",
];
/// `async: true` only where the hook can never need to block (11 §11.2.3).
const ASYNC_EVENTS: [&str; 4] = ["SessionStart", "SessionEnd", "Stop", "SubagentStop"];

fn entry(
    hook_bin: &Path,
    hook_sock: &Path,
    session: uuid::Uuid,
    event: &str,
    matcher: Option<&str>,
    reason: Option<&str>,
) -> Value {
    // Exec form (no shell): `command` is the executable, `args` its arguments
    // — verified against the live 2.1.251 settings validator; docs/11's
    // bare-`args` example is wrong (STALE-MAP).
    let mut args = vec![
        "hook".to_string(),
        "--sock".into(),
        hook_sock.display().to_string(),
        "--session".into(),
        session.to_string(),
        "--event".into(),
        event.to_string(),
    ];
    if let Some(r) = reason {
        args.push("--reason".into());
        args.push(r.to_string());
    }
    let mut hook = json!({
        "type": "command",
        "command": hook_bin.display().to_string(),
        "args": args,
        "timeout": 2,
    });
    if ASYNC_EVENTS.contains(&event) {
        hook["async"] = json!(true);
    }
    match matcher {
        Some(m) => json!({ "matcher": m, "hooks": [hook] }),
        None => json!({ "hooks": [hook] }),
    }
}

/// The deciding hook (T-84): structured writes into paths mesimon owns.
///
/// `Edit`/`Write`/`NotebookEdit` carry `file_path` as a real argument, so the
/// check is exact and cannot be dodged by quoting. `Bash` is deliberately NOT
/// matched: a command-shape pre-filter is a documented evasion hole, and
/// hooking it would put a blocking round trip on every shell call the agent
/// makes. That the shell path stays open is written down in `docs/USING.md` rather
/// than papered over.
///
/// Synchronous (no `async`) — an async hook cannot return a decision.
fn gate_entry(
    gate_bin: &Path,
    hook_sock: &Path,
    board_dir: &Path,
    state_dir: &Path,
    session: uuid::Uuid,
) -> Value {
    // One flag per rule rather than a repeatable `--deny-under`, so the
    // argv names which `RuleId` fired instead of encoding it as position.
    let args = vec![
        "gate".to_string(),
        "--session".into(),
        session.to_string(),
        // Denials are reported to the daemon so they land in the activity
        // feed. Best-effort and after the fact: the decision itself is local
        // and static, so a dead daemon cannot make the gate fail open.
        "--sock".into(),
        hook_sock.display().to_string(),
        "--deny-board".into(),
        board_dir.display().to_string(),
        "--deny-state".into(),
        state_dir.display().to_string(),
        // Ticket worktrees sit under the state dir and are the agent's own.
        "--allow".into(),
        state_dir.join(crate::paths::WORKTREES_DIR).display().to_string(),
    ];
    json!({
        "matcher": "Edit,Write,NotebookEdit",
        "hooks": [{
            "type": "command",
            "command": gate_bin.display().to_string(),
            "args": args,
            "timeout": 2,
        }],
    })
}

/// The 36-entry set: 34 observers, the static gate, and remote human decisions.
pub fn render_settings(
    hook_bin: &Path,
    hook_sock: &Path,
    session: uuid::Uuid,
    board_dir: &Path,
    state_dir: &Path,
) -> Value {
    let e = |event: &str, matcher: Option<&str>, reason: Option<&str>| {
        entry(hook_bin, hook_sock, session, event, matcher, reason)
    };
    let mut hooks = serde_json::Map::new();
    // The reason travels in argv — which registration fired — because the
    // payload field is `source`, not `session_start_reason` (11 §11.2.3).
    hooks.insert(
        "SessionStart".into(),
        Value::Array(
            SESSION_START_SOURCES.iter().map(|m| e("SessionStart", Some(m), Some(m))).collect(),
        ),
    );
    hooks.insert(
        "SessionEnd".into(),
        Value::Array(
            SESSION_END_REASONS.iter().map(|m| e("SessionEnd", Some(m), Some(m))).collect(),
        ),
    );
    // The matcher IS the error class (spike S-A).
    hooks.insert(
        "StopFailure".into(),
        Value::Array(
            STOP_FAILURE_MATCHERS.iter().map(|m| e("StopFailure", Some(m), Some(m))).collect(),
        ),
    );
    // Two PreToolUse entries with disjoint matchers, and no broad one
    // (11 §11.2.5): the narrow observer, and the deciding gate. A tool name
    // matches at most one of them.
    hooks.insert(
        "PreToolUse".into(),
        Value::Array(vec![
            e("PreToolUse", Some("AskUserQuestion,ExitPlanMode"), None),
            gate_entry(hook_bin, hook_sock, board_dir, state_dir, session),
        ]),
    );
    // PostToolUse is BROAD (deviation from 11 §11.2.5's verbose-tier ban,
    // recorded in STALE-MAP): completion is the ONLY mid-turn signal that
    // clears RequiresAction, and that holds for generic permissions too —
    // dogfood 2026-08-30, an ACCEPTED tool stayed needs-you until end of
    // turn, because there is no "permission answered" event (11 §11.7.3).
    // One entry, star matcher; the daemon discriminates on tool_name.
    hooks.insert("PostToolUse".into(), Value::Array(vec![e("PostToolUse", Some("*"), None)]));
    for ev in SINGLE_EVENTS {
        hooks.insert(ev.into(), Value::Array(vec![e(ev, None, None)]));
    }
    hooks.insert("PermissionRequest".into(), permission_entries(hook_bin, hook_sock, session));
    json!({ "hooks": hooks })
}

/// `PermissionRequest`'s two entries: the observer, and `mesimon approve`,
/// which holds the dialog for a paired person's answer (T-632). The same in
/// the whole set and in the permission-only one.
fn permission_entries(hook_bin: &Path, hook_sock: &Path, session: uuid::Uuid) -> Value {
    Value::Array(vec![
        entry(hook_bin, hook_sock, session, "PermissionRequest", None, None),
        json!({
            "hooks": [{"type": "command", "command": hook_bin.display().to_string(),
                "args": ["approve", "--sock", hook_sock.display().to_string(),
                    "--session", session.to_string(),
                    "--hold", PERMISSION_HOLD_SECS.to_string()],
                "timeout": PERMISSION_HOLD_SECS + 10}]
        }),
    ])
}

/// The permission-only set (T-658): `PermissionRequest`'s two entries and
/// nothing else, for a native mod launch. Where Claude Code keeps the
/// classic hook events from a person's plugins, the mod reports from its
/// own events, and a permission dialog is the one moment those do not give
/// it (`tool.check` resolving `ask` holds the dialog undrawn, T-651).
pub fn render_permission_settings(hook_bin: &Path, hook_sock: &Path, session: uuid::Uuid) -> Value {
    json!({ "hooks": { "PermissionRequest": permission_entries(hook_bin, hook_sock, session) } })
}

/// Which generated hook file a Claude launch passes as `--settings`
/// (`Daemon::launch_road`): the whole set on the hook set's road, none on a
/// mod that hears the hook events (T-577), the permission entries alone
/// beside a native one (T-658).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSet {
    Full,
    PermissionOnly,
    None,
}

/// Write `state_dir/hooks/<session>.json`, mode 0600. Returns the abs path.
pub fn write_settings(paths: &Paths, session: uuid::Uuid, hook_bin: &Path) -> Result<PathBuf> {
    let value =
        render_settings(hook_bin, &paths.hook_sock(), session, &paths.board_dir, &paths.state_dir);
    write_file(paths, session, &value)
}

/// The permission-only set at the same path, mode 0600: a wake rewrites the
/// file for the road it takes.
pub fn write_permission_settings(
    paths: &Paths,
    session: uuid::Uuid,
    hook_bin: &Path,
) -> Result<PathBuf> {
    let value = render_permission_settings(hook_bin, &paths.hook_sock(), session);
    write_file(paths, session, &value)
}

fn write_file(paths: &Paths, session: uuid::Uuid, value: &Value) -> Result<PathBuf> {
    let dir = paths.hooks_dir();
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{session}.json"));
    std::fs::write(&file, serde_json::to_string_pretty(value)?)?;
    std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

/// The mesimon binary a spawned session should exec back into — for the hook
/// on every tool event, for the write gate, and for the MCP shim. One ladder,
/// because they are all the same binary and a test seam that moved only one of
/// them would be a trap.
pub fn mesimon_bin() -> PathBuf {
    std::env::var("MESIMON_HOOK_BIN")
        .map(PathBuf::from)
        .or_else(|_| mesimon_core::exe::current_exe())
        .unwrap_or_else(|_| PathBuf::from("mesimon"))
}

/// The MCP server mesimon injects into every Claude session it spawns.
///
/// Three properties, and each one is a promise rather than a detail:
///
/// * **It is installed nowhere.** The config travels as an inline argv string
///   — `--mcp-config` takes JSON strings, not only files (verified on 2.1.251)
///   — so mesimon writes no `.mcp.json`, no `~/.claude.json`, no
///   `settings.local.json`, and no plugin marketplace entry. A session mesimon
///   did not start cannot see these tools, and revoking them is "stop passing
///   the flag".
/// * **The transport is the socket that already exists.** stdio to a shim that
///   talks to `orch.sock`. No TCP port: a loopback listener is reachable by
///   every process on the machine and by any browser page, and it would drag
///   in Origin validation, DNS-rebinding defence and a 60 s per-request timer
///   that stdio does not have.
/// * **There is no bearer token, deliberately.** It would be theatre —
///   `orch.sock` already accepts `Principal::Local` with the full command set
///   from any same-uid process, and a token in the tmux session environment is
///   readable from every other pane (`tmux show-environment`). The honest
///   boundary is the one the hook socket already committed to: the 0700
///   runtime directory. Identity is the session uuid, validated against a live
///   record mesimon spawned.
///
/// `--strict-mcp-config` is deliberately NOT passed: dropping the user's own
/// MCP servers from their own agent is subtractive magic (D7).
pub fn mcp_config_json(
    paths: &Paths,
    mesimon_bin: &Path,
    session: uuid::Uuid,
    tier: mesimon_core::board::AgentTools,
) -> String {
    // Built through serde, never by formatting: a repo path containing a quote
    // would otherwise break the blob open.
    json!({
        "mcpServers": {
            mesimon_core::mcp::SERVER_NAME: {
                // A `url` with no `type` is read as stdio and fails; being
                // explicit costs nothing and removes the trap entirely.
                "type": "stdio",
                "command": mesimon_bin.display().to_string(),
                "args": [
                    "mcp",
                    "--sock", paths.orch_sock().display().to_string(),
                    "--session", session.to_string(),
                    // The column's tier at spawn (T-117): what the shim
                    // LISTS. The daemon enforces at every call regardless,
                    // against the ticket's column as it stands then.
                    "--tools", tier.word(),
                ],
            }
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `const NAME = [ 'a', 'b' ]` list out of the mod's source.
    fn ts_list(name: &str) -> Vec<String> {
        let src = crate::modroad::FILES[2].1;
        let at = src.find(&format!("const {name} = [")).unwrap_or_else(|| panic!("{name}"));
        let body = &src[at..];
        let body = &body[body.find('[').unwrap() + 1..body.find(']').unwrap()];
        body.split(',')
            .map(|w| w.trim().trim_matches('\'').to_string())
            .filter(|w| !w.is_empty())
            .collect()
    }

    /// The mod relays the hook set's events by the hook set's names and
    /// matchers (T-574): on the mod road it is the only road (T-577), so an
    /// event it missed is an event the board never hears. One list in each
    /// language, held together here.
    #[test]
    fn the_mod_relays_exactly_what_the_hook_set_reports() {
        let words = |list: &[&str]| list.iter().map(|w| w.to_string()).collect::<Vec<_>>();
        assert_eq!(ts_list("SESSION_START_SOURCES"), words(&SESSION_START_SOURCES));
        assert_eq!(ts_list("SESSION_END_REASONS"), words(&SESSION_END_REASONS));
        assert_eq!(ts_list("STOP_FAILURE_MATCHERS"), words(&STOP_FAILURE_MATCHERS));
        assert_eq!(ts_list("PRE_TOOL_USE_TOOLS").join(","), "AskUserQuestion,ExitPlanMode");
        let rendered = rendered();
        let mut events: Vec<&str> =
            rendered["hooks"].as_object().unwrap().keys().map(String::as_str).collect();
        events.sort_unstable();
        let mut relayed = mesimon_core::road::RELAYED_EVENTS.to_vec();
        relayed.sort_unstable();
        assert_eq!(events, relayed);
        let src = crate::modroad::FILES[2].1;
        for event in mesimon_core::road::RELAYED_EVENTS {
            assert!(src.contains(&format!("on('classic.{event}',")), "{event} is not relayed");
        }
        assert!(!src.contains("on('classic.*'"), "the wildcard carries the verbose tier");
        assert!(src.contains(&format!(
            "const BRIDGE_REFUSED_EXIT = {}",
            mesimon_core::road::BRIDGE_REFUSED_EXIT
        )));
        // What it says it speaks, and the reports the daemon reads (T-575,
        // T-576), by the daemon's own names.
        assert_eq!(ts_list("SPEAKS"), words(&mesimon_core::road::SPEAKS));
        for (event, reason) in [
            (mesimon_core::road::MOD_SUBMIT, "id"),
            (mesimon_core::road::MOD_FILL, "id"),
            (mesimon_core::road::MOD_ANSWER, "'answered'"),
            (mesimon_core::road::MOD_ANSWER, "'declined'"),
            (mesimon_core::road::MOD_ANSWER, "'nothing_held'"),
            (mesimon_core::road::MOD_LOAD_FAILED, "'recovered'"),
            (mesimon_core::road::MOD_USAGE, "String(e?.reason ?? 'answer')"),
        ] {
            assert!(src.contains(&format!("relay($, '{event}', {reason},")), "{event} {reason}");
        }
        // The pane variables the daemon sets are the ones the mod reads.
        for var in [
            "MESIMON_MOD_BIN",
            "MESIMON_MOD_HOOK_SOCK",
            "MESIMON_MOD_ORCH_SOCK",
            "MESIMON_MOD_SESSION",
            "MESIMON_MOD_GATE_BOARD",
            "MESIMON_MOD_GATE_STATE",
            "MESIMON_MOD_GATE_ALLOW",
            "MESIMON_MOD_TOOLS",
            "MESIMON_MOD_NATIVE",
        ] {
            assert!(src.contains(&format!("$.env.get('{var}')")), "{var}");
        }
        // The native road (T-657) builds each of its events from an engine
        // event and relays it by the hook set's name, spelled as a literal at
        // the call site; the rest of the relayed set has no native source
        // and is relayed by nothing there.
        for event in mesimon_core::road::NATIVE_EVENTS {
            assert!(mesimon_core::road::RELAYED_EVENTS.contains(&event), "{event}");
            assert!(
                src.contains(&format!("relay($, '{event}',")),
                "{event} is not relayed natively"
            );
        }
        for event in mesimon_core::road::RELAYED_EVENTS {
            if mesimon_core::road::NATIVE_EVENTS.contains(&event) {
                continue;
            }
            assert!(!src.contains(&format!("relay($, '{event}',")), "{event} has no native source");
        }
        for hook in [
            "session.start",
            "session.end",
            "turn.start",
            "turn.complete",
            "tool.call",
            "agent.spawn",
            "session.receive",
            "session.compact",
        ] {
            assert!(src.contains(&format!("on('{hook}',")), "{hook}");
        }
    }

    /// The mod's gate (T-577) is `mesimon gate`'s: the same tools, the
    /// same rules by the same tags, and the model reads the same words.
    #[test]
    fn the_mod_gate_says_what_mesimon_gate_says() {
        use mesimon_core::verdict::RuleId;
        let src = crate::modroad::FILES[2].1;
        let gate = gate(&rendered());
        let tools: Vec<String> =
            gate["matcher"].as_str().unwrap().split(',').map(str::to_string).collect();
        let mut theirs = ts_list("GATE_TOOLS");
        theirs.sort_unstable();
        let mut ours = tools;
        ours.sort_unstable();
        assert_eq!(theirs, ours);
        for (rule, name) in [(RuleId::BoardDir, "BOARD"), (RuleId::StateDir, "STATE")] {
            assert!(src.contains(&format!("const RULE_{name} = '{}'", rule.tag())), "{name}");
            let literal = rule.reason().replace('\\', "\\\\").replace('\'', "\\'");
            assert!(
                src.contains(&format!("const REASON_{name} =\n  '{literal}'")),
                "REASON_{name} is not RuleId::reason: {literal}"
            );
        }
        assert!(src.contains(&format!("relay($, '{}', rule,", mesimon_core::road::GATE_DENIED)));
    }

    /// README promise 3, as far as a source scan can hold it: the mod never
    /// reaches for what puts words in front of the model.
    #[test]
    fn the_mod_spells_nothing_on_the_never_list() {
        let src = crate::modroad::FILES[2].1;
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for banned in [
            "$.session.append",
            "$.session.send",
            "$.model.",
            "'prompt.compose'",
            "'prompt.context'",
            "'prompt.section'",
            "'prompt.submit'",
            "'tool.check'",
            "context:",
            // A rewrite of an event's input is a rewrite of the model's
            // arguments or the person's words.
            "next({",
            // Deny or nothing (T-577): the mod decides no allow and no ask
            // of its own, in any position.
            "'allow'",
            "\"allow\"",
            "'ask'",
            "\"ask\"",
        ] {
            assert!(!code.contains(banned), "the mod spells {banned}");
        }
        // A deny's text reaches the model: one is the gate's, in `mesimon
        // gate`'s words (`the_mod_gate_says_what_mesimon_gate_says`), the
        // other a board tool's refusal in the daemon's, as the shim's error
        // result carried them (T-577).
        assert_eq!(code.matches("deny:").count(), 2, "the gate's and a tool's refusal");
        assert!(code.contains("return { deny: denial(rule) }"));
        assert!(code.contains("if (out?.isError === true) return { deny: text ||"));
        // The `$.prompt` calls are the turn road's submit (T-575) and the
        // send-now's fill (T-601), each the person's words, bare: the submit
        // `asUser: true`, the fill the whole text over an EMPTY box, read
        // first, so a person's draft is never replaced and nothing is added.
        let calls: Vec<&str> = code.matches("$.prompt.").collect();
        assert_eq!(calls.len(), 3, "the submit, and the fill with its read");
        assert!(code.contains("$.prompt.submit({ text, asUser: true })"), "a submit is asUser");
        assert!(code.contains("$.prompt.fill({ text, mode: 'replace' })"), "a fill is the text");
        assert!(code.contains("const box: any = await $.prompt.read()"));
        assert!(code.contains("box.text.trim() !== ''"), "a draft is never filled over");
        // The one allow is the consented one-shot (T-581): a permission
        // dialog's decision is returned in one place, and it is what `mesimon
        // approve` printed, which only a person's answer from Remote Control
        // fills (`PermissionDecision::hook_output`). The mod spells no
        // behavior of its own.
        assert_eq!(code.matches("decision:").count(), 1, "one decision, the one-shot's");
        assert!(code.contains("return { decision: ours } as any"));
        assert!(code.contains("const ours = await approve($, e)"));
        assert!(code
            .contains("JSON.parse(String(out?.stdout || 'null'))?.hookSpecificOutput?.decision"));
        assert!(!code.contains("behavior"), "the mod spells no behavior");
        // The mod's rounds are the core's numbers (T-632), each inside the
        // ten minutes a mod's process may live, together the whole hold.
        use mesimon_core::mesophon::{PERMISSION_RENEW_EXIT, PERMISSION_ROUND_SECS};
        let timeout_ms = (PERMISSION_ROUND_SECS + 30) * 1000;
        assert!(timeout_ms <= 600_000);
        for line in [
            format!("const APPROVE_ROUND_SECS = {PERMISSION_ROUND_SECS}"),
            format!("const APPROVE_TIMEOUT_MS = {timeout_ms}"),
            format!("const APPROVE_RENEW_EXIT = {PERMISSION_RENEW_EXIT}"),
            format!("const APPROVE_ROUNDS = {}", PERMISSION_HOLD_SECS / PERMISSION_ROUND_SECS),
        ] {
            assert!(code.contains(&line), "{line}");
        }
    }

    fn rendered() -> Value {
        render_settings(
            Path::new("/abs/mesimon"),
            Path::new("/tmp/mesimon-1/abcd/hook.sock"),
            uuid::Uuid::nil(),
            Path::new("/repo/.mesimon"),
            Path::new("/state/abcd"),
        )
    }

    /// The gate, isolated from the observers by its subcommand.
    fn gate(v: &Value) -> Value {
        v["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["hooks"][0]["args"][0] == json!("gate"))
            .cloned()
            .unwrap()
    }

    fn observers(v: &Value) -> Vec<(String, &Value)> {
        entries(v).into_iter().filter(|(_, e)| e["hooks"][0]["args"][0] == json!("hook")).collect()
    }

    fn entries(v: &Value) -> Vec<(String, &Value)> {
        v["hooks"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(ev, arr)| arr.as_array().unwrap().iter().map(move |e| (ev.clone(), e)))
            .collect()
    }

    #[test]
    fn thirty_six_entries() {
        // 34 observers + the deny-only gate + the remote permission bridge.
        assert_eq!(entries(&rendered()).len(), 36);
        assert_eq!(observers(&rendered()).len(), 34);
    }

    /// Observers and deciders have separate subcommands, not conditional stdout.
    #[test]
    fn observers_and_deciders_have_distinct_subcommands() {
        let v = rendered();
        for (ev, e) in observers(&v) {
            for h in e["hooks"].as_array().unwrap() {
                assert_eq!(h["args"][0], json!("hook"), "{ev} must be an observer");
            }
        }
        assert_eq!(gate(&v)["hooks"][0]["args"][0], json!("gate"));
    }

    /// An async hook cannot return a decision — it is fire-and-forget by
    /// definition, so a deciding hook that carried `async` would silently
    /// permit everything it was written to refuse.
    #[test]
    fn the_gate_is_synchronous() {
        assert!(gate(&rendered())["hooks"][0].get("async").is_none());
    }

    /// `Bash` is deliberately absent: a command-shape pre-filter is an evasion
    /// hole, and hooking it would block every shell call the agent makes.
    #[test]
    fn the_gate_matches_structured_writes_only() {
        assert_eq!(gate(&rendered())["matcher"], json!("Edit,Write,NotebookEdit"));
    }

    #[test]
    fn the_gate_names_every_guarded_root_absolutely() {
        let g = gate(&rendered());
        let args: Vec<&str> =
            g["hooks"][0]["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
        let val = |k: &str| args[args.iter().position(|a| *a == k).expect(k) + 1];
        assert_eq!(val("--deny-board"), "/repo/.mesimon");
        assert_eq!(val("--deny-state"), "/state/abcd");
        assert_eq!(val("--allow"), "/state/abcd/worktrees", "worktrees are the agent's own");
        assert!(val("--sock").starts_with('/'), "sock abs");
        for k in ["--deny-board", "--deny-state", "--allow"] {
            assert!(val(k).starts_with('/'), "guarded root must be absolute: {k}");
        }
    }

    #[test]
    fn never_emits_if() {
        // `if` is evaluated only on tool events; anywhere else it silently
        // disables the hook (11 §11.2.3). We simply never emit it.
        let v = rendered();
        for (ev, e) in entries(&v) {
            for h in e["hooks"].as_array().unwrap() {
                assert!(h.get("if").is_none(), "`if` on {ev}");
            }
            assert!(e.get("if").is_none(), "`if` on {ev} entry");
        }
    }

    #[test]
    fn matchers_only_where_supported() {
        let allowed = ["SessionStart", "SessionEnd", "StopFailure", "PreToolUse", "PostToolUse"];
        for (ev, e) in entries(&rendered()) {
            if e.get("matcher").is_some() {
                assert!(allowed.contains(&ev.as_str()), "matcher on {ev}");
            } else {
                assert!(!allowed.contains(&ev.as_str()), "{ev} should carry a matcher");
            }
        }
    }

    #[test]
    fn async_only_on_the_safe_four() {
        for (ev, e) in entries(&rendered()) {
            for h in e["hooks"].as_array().unwrap() {
                let is_async = h.get("async").and_then(Value::as_bool).unwrap_or(false);
                assert_eq!(is_async, ASYNC_EVENTS.contains(&ev.as_str()), "async on {ev}");
            }
        }
    }

    #[test]
    fn bounded_timeouts_and_paths_absolute() {
        let v = rendered();
        for (ev, e) in entries(&v) {
            for h in e["hooks"].as_array().unwrap() {
                assert_eq!(
                    h["timeout"],
                    if h["args"][0] == "approve" {
                        json!(PERMISSION_HOLD_SECS + 10)
                    } else {
                        json!(2)
                    },
                    "timeout on {ev}"
                );
                assert_eq!(h["type"], json!("command"));
                assert!(h["command"].as_str().unwrap().starts_with('/'), "hook bin abs");
            }
        }
        for (ev, e) in observers(&v) {
            for h in e["hooks"].as_array().unwrap() {
                let args = h["args"].as_array().unwrap();
                assert_eq!(args[0], json!("hook"), "subcommand first on {ev}");
                let sock_pos = args.iter().position(|a| a == "--sock").unwrap();
                assert!(args[sock_pos + 1].as_str().unwrap().starts_with('/'), "sock abs");
            }
        }
    }

    fn permission_only() -> Value {
        render_permission_settings(
            Path::new("/abs/mesimon"),
            Path::new("/tmp/mesimon-1/abcd/hook.sock"),
            uuid::Uuid::nil(),
        )
    }

    /// T-658: the native launch's file is `PermissionRequest` alone, its
    /// two entries spelled exactly as the whole set spells them, under the
    /// same traps: the exec form, no `if`, no matcher (the event takes
    /// none), no `async` (the approve holds a decision), bounded timeouts.
    #[test]
    fn the_permission_only_set_is_the_whole_sets_permission_entries() {
        let v = permission_only();
        let events: Vec<&String> = v["hooks"].as_object().unwrap().keys().collect();
        assert_eq!(events, ["PermissionRequest"]);
        assert_eq!(v["hooks"]["PermissionRequest"], rendered()["hooks"]["PermissionRequest"]);
        let all = entries(&v);
        assert_eq!(all.len(), 2);
        let subcommands: Vec<&Value> = all.iter().map(|(_, e)| &e["hooks"][0]["args"][0]).collect();
        assert_eq!(subcommands, [&json!("hook"), &json!("approve")]);
        for (ev, e) in all {
            assert!(e.get("matcher").is_none(), "matcher on {ev}");
            assert!(e.get("if").is_none(), "`if` on {ev}");
            for h in e["hooks"].as_array().unwrap() {
                assert_eq!(h["type"], json!("command"));
                assert!(h["command"].as_str().unwrap().starts_with('/'), "hook bin abs");
                assert!(h["args"].is_array(), "the exec form: command plus args");
                assert!(h.get("if").is_none() && h.get("async").is_none(), "{h}");
                assert!(h["timeout"].as_u64().is_some_and(|t| t <= PERMISSION_HOLD_SECS + 10));
            }
        }
        assert_eq!(permission_only(), permission_only());
    }

    #[test]
    fn deterministic() {
        assert_eq!(rendered(), rendered());
    }

    #[test]
    fn notification_has_no_matcher() {
        let v = rendered();
        let n = v["hooks"]["Notification"].as_array().unwrap();
        assert_eq!(n.len(), 1);
        assert!(n[0].get("matcher").is_none());
    }

    #[test]
    fn no_verbose_tier_events() {
        let v = rendered();
        let hooks = v["hooks"].as_object().unwrap();
        for banned in ["PostToolUseFailure", "PostToolBatch", "MessageDisplay"] {
            assert!(!hooks.contains_key(banned), "{banned} is the verbose tier");
        }
        // PreToolUse stays the narrow two-tool matcher (broad pre IS the
        // banned verbose tier). PostToolUse is deliberately broad — the
        // permission-accept clear path (STALE-MAP deviation).
        let p = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(p.len(), 2, "PreToolUse: the narrow observer and the gate");
        assert_eq!(p[0]["matcher"], json!("AskUserQuestion,ExitPlanMode"));
        assert_eq!(p[1]["matcher"], json!("Edit,Write,NotebookEdit"));
        // Disjoint: a tool name matches at most one entry, so the observer
        // never has to return a decision and the gate never has to observe.
        let a: Vec<&str> = p[0]["matcher"].as_str().unwrap().split(',').collect();
        let b: Vec<&str> = p[1]["matcher"].as_str().unwrap().split(',').collect();
        assert!(a.iter().all(|t| !b.contains(t)), "PreToolUse matchers overlap");
        let p = v["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(p.len(), 1, "PostToolUse single entry");
        assert_eq!(p[0]["matcher"], json!("*"));
    }
}
