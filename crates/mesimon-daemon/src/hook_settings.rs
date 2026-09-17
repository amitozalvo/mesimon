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
//! `timeout: 2` everywhere.
//!
//! **Two disjoint hook sets live in this file (T-84).** D15's attention hooks
//! are pure observers: they exec `mesimon hook`, which never writes stdout and
//! always exits 0, and they can only ever report. The one *deciding* hook execs
//! `mesimon gate`, which may answer "deny" and nothing else — a separate
//! subcommand precisely so the observer's never-writes-stdout invariant stays
//! true and stays testable. They share the `PreToolUse` key with disjoint
//! matchers; no tool matches both.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::paths::Paths;

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
/// makes. That the shell path stays open is written down in the README rather
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

/// The 35-entry registered set for one session: 34 observers plus the gate.
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
    json!({ "hooks": hooks })
}

/// Write `state_dir/hooks/<session>.json`, mode 0600. Returns the abs path.
pub fn write_settings(paths: &Paths, session: uuid::Uuid, hook_bin: &Path) -> Result<PathBuf> {
    let dir = paths.hooks_dir();
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{session}.json"));
    let value =
        render_settings(hook_bin, &paths.hook_sock(), session, &paths.board_dir, &paths.state_dir);
    std::fs::write(&file, serde_json::to_string_pretty(&value)?)?;
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
        entries(v).into_iter().filter(|(_, e)| e["hooks"][0]["args"][0] != json!("gate")).collect()
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
    fn thirty_five_entries() {
        // 34 observers (D15) + 1 decider (D10).
        assert_eq!(entries(&rendered()).len(), 35);
        assert_eq!(observers(&rendered()).len(), 34);
    }

    /// The two sets are told apart by the binary they exec, not by a comment.
    /// `mesimon hook` never writes stdout; `mesimon gate` is the only thing
    /// that may answer a decision.
    #[test]
    fn only_the_gate_can_decide() {
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
    fn timeout_2_everywhere_and_paths_absolute() {
        let v = rendered();
        for (ev, e) in entries(&v) {
            for h in e["hooks"].as_array().unwrap() {
                assert_eq!(h["timeout"], json!(2), "timeout on {ev}");
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
