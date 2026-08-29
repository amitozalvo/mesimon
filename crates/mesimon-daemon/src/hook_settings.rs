//! Per-session Claude settings generator (11 §11.2.1/§11.2.3, D33b paths).
//!
//! One JSON file per spawned Claude session, mode 0600, absolute paths only,
//! passed as `--settings`. Hooks MERGE with the user's own. The generator
//! encodes 11 §11.2.3's silent-failure traps as hard rules, each unit-tested:
//! `if` is never emitted (silently disables non-tool events), matchers only on
//! events that support them, `async: true` only where it cannot block, and
//! `timeout: 2` everywhere.

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
/// Matcherless single entries. The verbose tier (PostToolUse, PostToolBatch,
/// MessageDisplay, broad PreToolUse) is deliberately absent (11 §11.2.5).
const SINGLE_EVENTS: [&str; 9] = [
    "UserPromptSubmit",
    "Stop",
    "SubagentStop",
    "TeammateIdle",
    "PermissionRequest",
    "PermissionDenied",
    "Notification", // one entry, NO matcher — daemon discriminates on notification_type
    "Elicitation",
    "ElicitationResult",
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

/// The 30-entry registered set for one session.
pub fn render_settings(hook_bin: &Path, hook_sock: &Path, session: uuid::Uuid) -> Value {
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
    // The ONLY PreToolUse entry — narrow matcher, never broad (11 §11.2.5).
    hooks.insert(
        "PreToolUse".into(),
        Value::Array(vec![e("PreToolUse", Some("AskUserQuestion,ExitPlanMode"), None)]),
    );
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
    let value = render_settings(hook_bin, &paths.hook_sock(), session);
    std::fs::write(&file, serde_json::to_string_pretty(&value)?)?;
    std::fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> Value {
        render_settings(
            Path::new("/abs/mesimon"),
            Path::new("/tmp/mesimon-1/abcd/hook.sock"),
            uuid::Uuid::nil(),
        )
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
    fn thirty_entries() {
        assert_eq!(entries(&rendered()).len(), 30);
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
        let allowed = ["SessionStart", "SessionEnd", "StopFailure", "PreToolUse"];
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
        for (ev, e) in entries(&rendered()) {
            for h in e["hooks"].as_array().unwrap() {
                assert_eq!(h["timeout"], json!(2), "timeout on {ev}");
                assert_eq!(h["type"], json!("command"));
                assert!(h["command"].as_str().unwrap().starts_with('/'), "hook bin abs");
                let args = h["args"].as_array().unwrap();
                assert_eq!(args[0], json!("hook"), "subcommand first");
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
        for banned in ["PostToolUse", "PostToolUseFailure", "PostToolBatch", "MessageDisplay"] {
            assert!(!hooks.contains_key(banned), "{banned} is the verbose tier");
        }
        // The one PreToolUse entry is the narrow two-tool matcher.
        let p = v["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(p[0]["matcher"], json!("AskUserQuestion,ExitPlanMode"));
    }
}
