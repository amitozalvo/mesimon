//! Hook-frame parsing and payload → `Signal` distillation (11 §11.2.3).
//!
//! Tolerant by design: a truncated or malformed body degrades to
//! metadata-only — the argv header alone carries enough for every transition
//! except detail text. Unknown event names map to `None` (forward-safe:
//! Claude silently ignores unknown registrations, and so do we).

use mesimon_core::attention::{
    is_teammate_task, task_blocks_end_turn, AttentionTool, EndKind, NotificationKind, Signal,
    StartSource, StopFailureClass,
};
use serde_json::Value;

/// Cards get an excerpt, never a transcript (D11). Hard cap.
const DETAIL_MAX: usize = 200;

#[derive(Debug, Clone)]
pub struct HookFrame {
    /// A session UUID (Claude hooks) or a sid16 (the tmux pane-died hook).
    pub session: String,
    pub event: String,
    /// Which registration fired — the matcher travels in argv, not payload
    /// (11 §11.2.3: `SessionStart` has `source`, not `session_start_reason`).
    pub reason: Option<String>,
    pub payload: Value,
}

/// One header line + `\n` + raw payload bytes (may be absent or malformed).
pub fn parse_frame(bytes: &[u8]) -> Option<HookFrame> {
    let nl = bytes.iter().position(|b| *b == b'\n').unwrap_or(bytes.len());
    let header: Value = serde_json::from_slice(&bytes[..nl]).ok()?;
    let session = header.get("session")?.as_str()?.to_string();
    let event = header.get("event")?.as_str()?.to_string();
    let reason = header.get("reason").and_then(Value::as_str).map(str::to_string);
    let payload = bytes
        .get(nl + 1..)
        .filter(|rest| !rest.is_empty())
        .and_then(|rest| serde_json::from_slice(rest).ok())
        .unwrap_or(Value::Null);
    Some(HookFrame { session, event, reason, payload })
}

pub fn signal_of(frame: &HookFrame) -> Option<Signal> {
    let reason = frame.reason.as_deref();
    match frame.event.as_str() {
        "SessionStart" => Some(Signal::SessionStart {
            source: match reason.or_else(|| frame.payload.get("source").and_then(Value::as_str)) {
                Some("resume") => StartSource::Resume,
                Some("clear") => StartSource::Clear,
                Some("compact") => StartSource::Compact,
                Some("fork") => StartSource::Fork,
                _ => StartSource::Startup,
            },
        }),
        "SessionEnd" => Some(Signal::SessionEnd {
            kind: match reason.or_else(|| frame.payload.get("reason").and_then(Value::as_str)) {
                Some("clear") => EndKind::Clear,
                Some("resume") => EndKind::Resume,
                Some("logout") => EndKind::Logout,
                Some("prompt_input_exit") => EndKind::PromptInputExit,
                _ => EndKind::Other,
            },
        }),
        "UserPromptSubmit" => Some(Signal::UserPromptSubmit),
        "Stop" => Some(Signal::Stop {
            stop_hook_active: frame
                .payload
                .get("stop_hook_active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            has_agent_id: has_agent_id(frame),
            // Classified, not counted: a dormant `monitor` would otherwise
            // suppress every Stop for the rest of the session (T-72). An entry
            // with no readable `.type` counts as blocking — the safe read.
            blocking_tasks: background_tasks(frame)
                .any(|t| t.get("type").and_then(Value::as_str).is_none_or(task_blocks_end_turn)),
            // ...except a teammate, which is counted (T-135): it reads
            // `running` idle or busy, so the machine weighs the count against
            // the `TeammateIdle` frames instead.
            teammates: background_tasks(frame)
                .filter(|t| t.get("type").and_then(Value::as_str).is_some_and(is_teammate_task))
                .count(),
        }),
        "SubagentStop" => Some(Signal::SubagentStop),
        "TeammateIdle" => Some(Signal::TeammateIdle {
            name: frame.payload.get("teammate_name").and_then(Value::as_str).map(str::to_string),
        }),
        "StopFailure" => Some(Signal::StopFailure {
            // The matcher IS the error class (spike S-A); `error` is the
            // payload cross-check when the argv reason is missing.
            class: match reason.or_else(|| frame.payload.get("error").and_then(Value::as_str)) {
                Some("rate_limit") => StopFailureClass::RateLimit,
                Some("overloaded") => StopFailureClass::Overloaded,
                Some("authentication_failed") => StopFailureClass::AuthenticationFailed,
                Some("oauth_org_not_allowed") => StopFailureClass::OauthOrgNotAllowed,
                Some("billing_error") => StopFailureClass::BillingError,
                Some("invalid_request") => StopFailureClass::InvalidRequest,
                Some("model_not_found") => StopFailureClass::ModelNotFound,
                Some("max_output_tokens") => StopFailureClass::MaxOutputTokens,
                Some("server_error") => StopFailureClass::ServerError,
                _ => StopFailureClass::Unknown,
            },
        }),
        // The approval dialog for the two interaction tools IS the plan/
        // question moment — a generic Permission here would clobber the
        // sharper reason PreToolUse just set (dogfood 2026-08-30: plan
        // dialogs read as `permission`, never `plan`).
        "PermissionRequest" => match frame.payload.get("tool_name").and_then(Value::as_str) {
            Some("AskUserQuestion") => {
                Some(Signal::PreToolUse { tool: AttentionTool::AskUserQuestion })
            }
            Some("ExitPlanMode") => Some(Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }),
            _ => Some(Signal::PermissionRequest),
        },
        "PermissionDenied" => Some(Signal::PermissionDenied),
        "PreToolUse" => match frame.payload.get("tool_name").and_then(Value::as_str) {
            Some("AskUserQuestion") => {
                Some(Signal::PreToolUse { tool: AttentionTool::AskUserQuestion })
            }
            Some("ExitPlanMode") => Some(Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }),
            _ => None,
        },
        // Broad since dogfood 2026-08-30 (an ACCEPTED permission stayed
        // needs-you until end of turn): the interaction pair keeps its sharp
        // signal; any other completion is the generic permission-accept path
        // — there is no "permission answered" event (11 §11.7.3), a tool
        // finishing is the only proof the dialog resolved.
        "PostToolUse" => match frame.payload.get("tool_name").and_then(Value::as_str) {
            Some("AskUserQuestion") => {
                Some(Signal::PostToolUse { tool: AttentionTool::AskUserQuestion })
            }
            Some("ExitPlanMode") => Some(Signal::PostToolUse { tool: AttentionTool::ExitPlanMode }),
            // A message to a teammate wakes it, so it is working again
            // whatever it last reported (T-135). `to` is the addressee.
            Some("SendMessage") => match frame
                .payload
                .get("tool_input")
                .and_then(|i| i.get("to"))
                .and_then(Value::as_str)
            {
                Some(name) => Some(Signal::TeammateMessaged { name: name.to_string() }),
                None => Some(Signal::ToolCompleted { nested: has_agent_id(frame) }),
            },
            // `agent_id` marks a subagent's or teammate's tool, not the
            // session's own (measured 2026-09-01).
            _ => Some(Signal::ToolCompleted { nested: has_agent_id(frame) }),
        },
        // One registration, no matcher — discriminate here (11 §11.2.3: the
        // documented matcher list is shorter than the shipping enum).
        "Notification" => Some(Signal::Notification {
            kind: match frame.payload.get("notification_type").and_then(Value::as_str) {
                Some("permission_prompt") => NotificationKind::PermissionPrompt,
                Some("quota_auto_resume_fired") => NotificationKind::QuotaFired,
                Some("quota_auto_resume_stale") => NotificationKind::QuotaStale,
                Some("quota_auto_resume_disabled") => NotificationKind::QuotaDisabled,
                _ => NotificationKind::Other,
            },
        }),
        "Elicitation" => Some(Signal::Elicitation),
        "ElicitationResult" => Some(Signal::ElicitationResult),
        "PaneDied" => Some(Signal::PaneDied { status: reason.and_then(|r| r.parse().ok()) }),
        _ => None,
    }
}

fn background_tasks(frame: &HookFrame) -> impl Iterator<Item = &Value> {
    frame
        .payload
        .get("background_tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flat_map(|a| a.iter())
}

fn has_agent_id(frame: &HookFrame) -> bool {
    frame.payload.get("agent_id").is_some_and(|v| !v.is_null())
}

/// `SessionStart` is the only authoritative source of `transcript_path` (D24).
pub fn transcript_of(frame: &HookFrame) -> Option<String> {
    if frame.event != "SessionStart" {
        return None;
    }
    frame.payload.get("transcript_path").and_then(Value::as_str).map(str::to_string)
}

/// The card excerpt for this event, if it carries one worth showing.
pub fn detail_of(frame: &HookFrame) -> Option<String> {
    let text = match frame.event.as_str() {
        // The rendered API error string, e.g. "API Error: Rate limit reached".
        "StopFailure" => frame
            .payload
            .get("last_assistant_message")
            .and_then(Value::as_str)
            .or_else(|| frame.payload.get("error").and_then(Value::as_str))
            .map(str::to_string),
        "PermissionRequest" => {
            let tool = frame.payload.get("tool_name").and_then(Value::as_str)?;
            if tool == "ExitPlanMode" {
                // The dialog is "approve this plan?", not a tool permission —
                // the raw tool name reads as noise on the card.
                return Some("plan ready for review".to_string());
            }
            let arg = frame
                .payload
                .get("tool_input")
                .and_then(|i| {
                    i.get("command")
                        .or_else(|| i.get("file_path"))
                        .or_else(|| i.get("url"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("");
            Some(if arg.is_empty() { tool.to_string() } else { format!("{tool}({arg})") })
        }
        "PreToolUse" => match frame.payload.get("tool_name").and_then(Value::as_str) {
            Some("AskUserQuestion") => frame
                .payload
                .get("tool_input")
                .and_then(|i| i.get("questions"))
                .and_then(|q| q.get(0))
                .and_then(|q| q.get("question"))
                .and_then(Value::as_str)
                .map(str::to_string),
            Some("ExitPlanMode") => Some("plan ready for review".to_string()),
            _ => None,
        },
        "Notification" => frame.payload.get("message").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }?;
    let text = sanitize(&text);
    if text.is_empty() {
        return None;
    }
    Some(text)
}

/// The most of a hook frame either side keeps. A frame is a one-line header
/// and a few JSON keys the daemon reads; a `Write` payload carries the whole
/// file after them and is not needed. Any same-uid process can open
/// `hook.sock`, so what arrives is bounded on both sides of it.
pub const HOOK_FRAME_MAX_BYTES: u64 = 1 << 20;

/// Control bytes stripped, hard length cap — every string here originates in
/// a pane an agent controls (07 §18 rule 5).
fn sanitize(s: &str) -> String {
    let cleaned = mesimon_core::text::scrub_cells(s, false);
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= DETAIL_MAX {
        return cleaned.to_string();
    }
    cleaned.chars().take(DETAIL_MAX).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: &str, reason: Option<&str>, payload: &str) -> HookFrame {
        let header = serde_json::json!({
            "v": 1, "session": "s", "event": event, "reason": reason,
        });
        let mut bytes = serde_json::to_vec(&header).unwrap();
        bytes.push(b'\n');
        bytes.extend_from_slice(payload.as_bytes());
        parse_frame(&bytes).unwrap()
    }

    #[test]
    fn parses_header_and_body() {
        let f = frame("Stop", None, r#"{"stop_hook_active": false, "background_tasks": []}"#);
        assert_eq!(f.event, "Stop");
        assert_eq!(
            signal_of(&f),
            Some(Signal::Stop {
                stop_hook_active: false,
                has_agent_id: false,
                blocking_tasks: false,
                teammates: 0
            })
        );
    }

    /// `background_tasks` is classified, not counted: a live artifact-comment
    /// monitor is dormant and must not hold the turn open (T-72); a background
    /// shell must. A typeless entry reads as blocking.
    #[test]
    fn background_tasks_block_end_turn_by_type_not_emptiness() {
        let blocking = |body: &str| match signal_of(&frame("Stop", None, body)) {
            Some(Signal::Stop { blocking_tasks, .. }) => blocking_tasks,
            other => panic!("expected Stop, got {other:?}"),
        };
        assert!(!blocking(r#"{"background_tasks":[{"type":"monitor"}]}"#));
        assert!(blocking(r#"{"background_tasks":[{"type":"shell"}]}"#));
        // Mixed: the shell still holds it open.
        assert!(blocking(r#"{"background_tasks":[{"type":"monitor"},{"type":"shell"}]}"#));
        assert!(blocking(r#"{"background_tasks":[{"description":"?"}]}"#));
        assert!(!blocking(r#"{"background_tasks":[]}"#));
        assert!(!blocking("{}"));
    }

    /// A teammate is counted, not classed (T-135): the payload lists one as
    /// `running` for its whole life, so the machine weighs the count against
    /// the idle notices. The shape is the one captured on the wire.
    #[test]
    fn teammates_are_counted_not_blocking() {
        let body = r#"{"background_tasks":[
            {"id":"t1","type":"teammate","status":"running","description":"Reuse review"},
            {"id":"t2","type":"teammate","status":"running","description":"Altitude review"},
            {"id":"m1","type":"monitor","status":"running","description":"comments"}]}"#;
        match signal_of(&frame("Stop", None, body)) {
            Some(Signal::Stop { blocking_tasks, teammates, .. }) => {
                assert!(!blocking_tasks);
                assert_eq!(teammates, 2);
            }
            other => panic!("expected Stop, got {other:?}"),
        }
        // A shell beside them still holds the turn open on its own.
        let body = r#"{"background_tasks":[{"type":"teammate"},{"type":"shell"}]}"#;
        match signal_of(&frame("Stop", None, body)) {
            Some(Signal::Stop { blocking_tasks, teammates, .. }) => {
                assert!(blocking_tasks);
                assert_eq!(teammates, 1);
            }
            other => panic!("expected Stop, got {other:?}"),
        }
    }

    /// The teammate bookkeeping frames: an idle notice carries the name, a
    /// `SendMessage` names its addressee, and a subagent's tool completion is
    /// marked nested by its `agent_id` (all three captured 2026-09-01).
    #[test]
    fn teammate_frames_carry_names_and_nesting() {
        let f = frame("TeammateIdle", None, r#"{"teammate_name":"reuse"}"#);
        assert_eq!(signal_of(&f), Some(Signal::TeammateIdle { name: Some("reuse".into()) }));
        let f = frame("TeammateIdle", None, "{}");
        assert_eq!(signal_of(&f), Some(Signal::TeammateIdle { name: None }));
        let f = frame(
            "PostToolUse",
            None,
            r#"{"tool_name":"SendMessage","tool_input":{"to":"reuse","message":"go on"}}"#,
        );
        assert_eq!(signal_of(&f), Some(Signal::TeammateMessaged { name: "reuse".into() }));
        let f = frame("PostToolUse", None, r#"{"tool_name":"SendMessage","tool_input":{}}"#);
        assert_eq!(signal_of(&f), Some(Signal::ToolCompleted { nested: false }));
        let f = frame(
            "PostToolUse",
            None,
            r#"{"tool_name":"Bash","agent_id":"a47da72ea574bf9f0","agent_type":"general-purpose"}"#,
        );
        assert_eq!(signal_of(&f), Some(Signal::ToolCompleted { nested: true }));
        let f = frame("PostToolUse", None, r#"{"tool_name":"Bash","agent_id":null}"#);
        assert_eq!(signal_of(&f), Some(Signal::ToolCompleted { nested: false }));
    }

    #[test]
    fn malformed_body_degrades_to_metadata_only() {
        let f = frame("PermissionRequest", None, "{truncated");
        assert_eq!(f.payload, Value::Null);
        assert_eq!(signal_of(&f), Some(Signal::PermissionRequest));
        assert_eq!(detail_of(&f), None);
    }

    #[test]
    fn missing_body_is_fine() {
        let bytes = br#"{"v":1,"session":"abc","event":"PaneDied","reason":"7"}"#;
        let f = parse_frame(bytes).unwrap();
        assert_eq!(signal_of(&f), Some(Signal::PaneDied { status: Some(7) }));
    }

    #[test]
    fn session_start_source_from_argv_reason() {
        // Captured shape (spike S-A): source in payload too, argv wins.
        let f = frame(
            "SessionStart",
            Some("resume"),
            r#"{"session_id":"x","transcript_path":"/tmp/t.jsonl","cwd":"/r","source":"startup"}"#,
        );
        assert_eq!(signal_of(&f), Some(Signal::SessionStart { source: StartSource::Resume }));
        assert_eq!(transcript_of(&f), Some("/tmp/t.jsonl".into()));
    }

    #[test]
    fn stop_failure_matcher_is_the_class() {
        // Captured payload (spike S-A, verbatim shape).
        let f = frame(
            "StopFailure",
            Some("authentication_failed"),
            r#"{"session_id":"x","error":"authentication_failed","last_assistant_message":"Not logged in · Please run /login"}"#,
        );
        assert_eq!(
            signal_of(&f),
            Some(Signal::StopFailure { class: StopFailureClass::AuthenticationFailed })
        );
        assert_eq!(detail_of(&f), Some("Not logged in · Please run /login".into()));
    }

    #[test]
    fn stop_with_agent_id_flags_nested() {
        let f = frame("Stop", None, r#"{"agent_id":"a1","stop_hook_active":false}"#);
        assert_eq!(
            signal_of(&f),
            Some(Signal::Stop {
                stop_hook_active: false,
                has_agent_id: true,
                blocking_tasks: false,
                teammates: 0
            })
        );
    }

    #[test]
    fn notification_discriminates_on_type() {
        let f = frame(
            "Notification",
            None,
            r#"{"notification_type":"quota_auto_resume_stale","message":"press Enter"}"#,
        );
        assert_eq!(
            signal_of(&f),
            Some(Signal::Notification { kind: NotificationKind::QuotaStale })
        );
        let f = frame("Notification", None, r#"{"notification_type":"idle_prompt"}"#);
        assert_eq!(signal_of(&f), Some(Signal::Notification { kind: NotificationKind::Other }));
    }

    #[test]
    fn permission_request_detail_names_the_call() {
        let f = frame(
            "PermissionRequest",
            None,
            r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf node_modules"},"prompt_id":"p1","permission_mode":"default"}"#,
        );
        assert_eq!(signal_of(&f), Some(Signal::PermissionRequest));
        assert_eq!(detail_of(&f), Some("Bash(rm -rf node_modules)".into()));
        // The interaction tools' approval dialogs keep their sharp reason —
        // never the generic Permission (see signal_of).
        let f = frame(
            "PermissionRequest",
            None,
            r#"{"tool_name":"ExitPlanMode","tool_input":{"plan":"p"}}"#,
        );
        assert_eq!(signal_of(&f), Some(Signal::PreToolUse { tool: AttentionTool::ExitPlanMode }));
        assert_eq!(detail_of(&f), Some("plan ready for review".into()));
        let f = frame("PermissionRequest", None, r#"{"tool_name":"AskUserQuestion"}"#);
        assert_eq!(
            signal_of(&f),
            Some(Signal::PreToolUse { tool: AttentionTool::AskUserQuestion })
        );
    }

    #[test]
    fn pretooluse_only_maps_the_two_tools() {
        let f = frame(
            "PreToolUse",
            None,
            r#"{"tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Keep the 301?"}]}}"#,
        );
        assert_eq!(
            signal_of(&f),
            Some(Signal::PreToolUse { tool: AttentionTool::AskUserQuestion })
        );
        assert_eq!(detail_of(&f), Some("Keep the 301?".into()));
        let f = frame("PreToolUse", None, r#"{"tool_name":"Bash"}"#);
        assert_eq!(signal_of(&f), None);
        let f = frame("PostToolUse", None, r#"{"tool_name":"AskUserQuestion","tool_response":{}}"#);
        assert_eq!(
            signal_of(&f),
            Some(Signal::PostToolUse { tool: AttentionTool::AskUserQuestion })
        );
        // Any other completion is the generic permission-accept path.
        let f = frame("PostToolUse", None, r#"{"tool_name":"Bash"}"#);
        assert_eq!(signal_of(&f), Some(Signal::ToolCompleted { nested: false }));
        // Missing/truncated tool_name still counts as a completion.
        let f = frame("PostToolUse", None, "{truncated");
        assert_eq!(signal_of(&f), Some(Signal::ToolCompleted { nested: false }));
    }

    #[test]
    fn unknown_events_are_forward_safe() {
        let f = frame("TotallyFakeEvent", None, "{}");
        assert_eq!(signal_of(&f), None);
    }

    #[test]
    fn detail_is_sanitized_and_capped() {
        let long = format!("{}\u{1b}[31mx", "a".repeat(300));
        let f = frame(
            "Notification",
            None,
            &serde_json::json!({"notification_type":"other_thing","message": long}).to_string(),
        );
        let d = detail_of(&f).unwrap();
        assert!(d.chars().count() <= 200);
        assert!(!d.contains('\u{1b}'));
    }
}
