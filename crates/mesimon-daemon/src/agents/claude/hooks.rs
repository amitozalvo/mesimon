//! Claude Code hook payload normalization.

use mesimon_core::attention::{
    is_teammate_task, AttentionTool, EndKind, NotificationKind, Signal, StartSource,
    StopFailureClass,
};
use mesimon_core::background::{
    classify, is_live_status, is_monitor_kind, Liveness, Registry, Transition,
};
use mesimon_core::board::SessionRecord;
use serde_json::Value;

/// Cards get an excerpt, never a transcript (D11). Hard cap.
const DETAIL_MAX: usize = 200;

#[cfg(test)]
use crate::ingest::parse_frame;
use crate::ingest::HookFrame;

/// Stateful task evidence is shared by live ingestion and offline replay.
/// This registry is deliberately absent from persisted session records.
pub fn signal_with_background(frame: &HookFrame, tasks: &mut Registry) -> Option<Signal> {
    let before = tasks.liveness();
    let owner = frame.payload.get("agent_id").and_then(Value::as_str);
    if frame.event == "SessionStart"
        && owner.is_none()
        && frame.reason.as_deref().or_else(|| frame.payload.get("source").and_then(Value::as_str))
            != Some("compact")
        || frame.event == "SessionEnd"
    {
        tasks.clear();
    }
    if matches!(frame.event.as_str(), "Stop" | "SubagentStop")
        && frame.payload.get("background_tasks").is_some_and(Value::is_array)
    {
        let rows: Vec<_> = background_tasks(frame).collect();
        let ids: Vec<_> = rows.iter().filter_map(|t| t.get("id").and_then(Value::as_str)).collect();
        if owner.is_none() {
            tasks.retain_snapshot(None, &ids);
        }
        for task in rows {
            if let Some(id) = task.get("id").and_then(Value::as_str) {
                let kind = task.get("type").and_then(Value::as_str);
                if kind.is_some_and(is_teammate_task) {
                    // Teammates belong exclusively to the idle-notice ledger,
                    // even if an earlier start looked like an ordinary agent.
                    tasks.record(id, kind, None, Transition::Completed, owner);
                } else {
                    tasks.record(
                        id,
                        kind,
                        task.get("status").and_then(Value::as_str),
                        Transition::Listed,
                        owner,
                    );
                }
            }
        }
    }
    if matches!(frame.event.as_str(), "SubagentStart" | "SubagentStop") {
        if let Some(id) = owner {
            // agent_id identifies the agent itself on these two hooks.
            if frame.event == "SubagentStop" {
                tasks.record(id, Some("subagent"), None, Transition::Completed, None);
            } else {
                // Ownership is not in this hook. Keep the agent independent
                // until its own Stop or Agent result supplies better evidence.
                tasks.record(id, Some("subagent"), None, Transition::Started, Some(id));
            }
        }
    }
    if frame.event == "PostToolUse" {
        let response = &frame.payload["tool_response"];
        match frame.payload.get("tool_name").and_then(Value::as_str) {
            Some("Agent" | "Task") => {
                if let Some(id) = response.get("agentId").and_then(Value::as_str) {
                    let status = response.get("status").and_then(Value::as_str);
                    tasks.record(id, Some("subagent"), status, Transition::Updated, owner);
                }
            }
            Some("Monitor") => {
                if let Some(id) = response.get("taskId").and_then(Value::as_str) {
                    tasks.record(id, Some("monitor"), None, Transition::Started, owner);
                }
            }
            Some("TaskStop") => {
                if let Some(id) =
                    frame.payload.pointer("/tool_input/task_id").and_then(Value::as_str)
                {
                    tasks.record(id, None, None, Transition::Completed, owner);
                }
            }
            Some("TaskOutput") => {
                if let Some(id) = response.pointer("/task/task_id").and_then(Value::as_str) {
                    tasks.record(
                        id,
                        response.pointer("/task/task_type").and_then(Value::as_str),
                        response.pointer("/task/status").and_then(Value::as_str),
                        Transition::Updated,
                        owner,
                    );
                }
            }
            _ => {}
        }
    }
    let mut signal = signal_of(frame);
    if let Some(Signal::Stop { has_agent_id: false, blocking_tasks, monitoring_tasks, .. }) =
        &mut signal
    {
        *blocking_tasks |= tasks.liveness() == Liveness::Working;
        *monitoring_tasks |= tasks.liveness() == Liveness::Monitoring;
    } else if tasks.liveness() != before
        && (matches!(frame.event.as_str(), "SubagentStart" | "SubagentStop")
            || (owner.is_some() && matches!(frame.event.as_str(), "Stop" | "PostToolUse")))
    {
        signal = Some(Signal::BackgroundChanged { liveness: tasks.liveness() });
    }
    signal
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
        "PreCompact" | "PostCompact" if !has_agent_id(frame) => {
            // Missing/unknown triggers stay on the automatic continuation path.
            let manual = frame.payload.get("trigger").and_then(Value::as_str) == Some("manual");
            Some(if frame.event == "PreCompact" {
                Signal::PreCompact { manual }
            } else {
                Signal::PostCompact { manual }
            })
        }
        "Stop" => Some(Signal::Stop {
            stop_hook_active: frame
                .payload
                .get("stop_hook_active")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            has_agent_id: has_agent_id(frame),
            blocking_tasks: background_tasks(frame).any(|t| {
                live_task(t)
                    && !t.get("type").and_then(Value::as_str).is_some_and(is_teammate_task)
                    && classify(t.get("type").and_then(Value::as_str)) == Liveness::Working
            }),
            // A `monitor` row is decided by the registry's provenance, never
            // by the payload alone (`background::is_monitor_kind`).
            monitoring_tasks: background_tasks(frame).any(|t| {
                let kind = t.get("type").and_then(Value::as_str);
                live_task(t) && classify(kind) == Liveness::Monitoring && !is_monitor_kind(kind)
            }),
            // ...except a teammate, which is counted (T-135): it reads
            // `running` idle or busy, so the machine weighs the count against
            // the `TeammateIdle` frames instead.
            teammates: background_tasks(frame)
                .filter(|t| {
                    live_task(t)
                        && t.get("type").and_then(Value::as_str).is_some_and(is_teammate_task)
                })
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

fn live_task(task: &Value) -> bool {
    task.get("status").and_then(Value::as_str).is_none_or(is_live_status)
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

/// The one exception to D24: the file MOVED. Claude Code homes a transcript
/// under a project dir derived from the process cwd, and its `EnterWorktree`
/// tool changes that cwd mid-session — the file is re-homed on the spot and
/// every later frame names the new path (T-433, 2026-09-23: T-245's agent did
/// exactly that and its card read "nothing to read in its transcript" for a
/// day while the conversation sat 1.6 MB deep under
/// `…-mesimon--claude-worktrees-t245-pane-id/`). A `cd` in the Bash tool does
/// not do this: 213 local transcripts changed `cwd` that way and none moved.
///
/// Identity still never travels (D24): the new path is taken only when its
/// file stem is the uuid the record already knows, it differs from the
/// recorded path, and the recorded file is gone — a frame that names another
/// session, or a copy beside a still-present original, changes nothing.
pub fn transcript_moved(frame: &HookFrame, record: &SessionRecord) -> Option<String> {
    let path = frame.payload.get("transcript_path").and_then(Value::as_str)?;
    let known = record.transcript_path.as_deref()?;
    if path == known {
        return None;
    }
    let identity = record.claude_session_id.unwrap_or(record.id);
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str())?;
    if stem.parse::<uuid::Uuid>().ok()? != identity {
        return None;
    }
    if std::path::Path::new(known).is_file() {
        return None;
    }
    Some(path.to_string())
}

/// The plan an approved `ExitPlanMode` carries, whole (2026-09-03).
///
/// `PostToolUse` fires once the tool has RETURNED, and for this tool that is
/// once the user approved the plan — a rejection is a tool error and fires
/// nothing mesimon hooks — so the frame is the approval. WHERE the plan rides
/// moved under us: 2.1.251–2.1.258 put the markdown in `tool_input.plan`;
/// 2.1.259 injects `plan`/`planFilePath` into the input from the plan file,
/// strips both again right before the call, and hands the plan back in the
/// RESULT — `tool_response: {plan, isAgent, filePath, hasTaskTool}` — so the
/// approval frame sees `tool_input: {}` and only the frames BEFORE approval
/// (`PreToolUse`, `PermissionRequest`) still carry the input form. The
/// response is read first and the input second, so both builds land. A
/// subagent's plan (`agent_id` set, or the response saying `isAgent`) is not
/// the session's.
pub fn plan_of(frame: &HookFrame) -> Option<String> {
    if frame.event != "PostToolUse" || has_agent_id(frame) {
        return None;
    }
    if frame.payload.get("tool_name").and_then(Value::as_str) != Some("ExitPlanMode") {
        return None;
    }
    let response = frame.payload.get("tool_response");
    if response.and_then(|r| r.get("isAgent")).and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let plan = response
        .and_then(|r| r.get("plan"))
        .and_then(Value::as_str)
        .or_else(|| frame.payload.get("tool_input")?.get("plan")?.as_str())?;
    (!plan.trim().is_empty()).then(|| plan.to_string())
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
                monitoring_tasks: false,
                teammates: 0
            })
        );
    }

    /// Watches and shells are monitoring; unknown tasks conservatively work.
    #[test]
    fn background_tasks_block_end_turn_by_type_not_emptiness() {
        let blocking = |body: &str| match signal_of(&frame("Stop", None, body)) {
            Some(Signal::Stop { blocking_tasks, .. }) => blocking_tasks,
            other => panic!("expected Stop, got {other:?}"),
        };
        assert!(!blocking(r#"{"background_tasks":[{"type":"monitor"}]}"#));
        assert!(!blocking(r#"{"background_tasks":[{"type":"shell"}]}"#));
        // Mixed watches remain monitoring.
        assert!(!blocking(r#"{"background_tasks":[{"type":"monitor"},{"type":"shell"}]}"#));
        assert!(blocking(r#"{"background_tasks":[{"description":"?"}]}"#));
        assert!(!blocking(r#"{"background_tasks":[]}"#));
        assert!(!blocking("{}"));
    }

    #[test]
    fn teammate_snapshot_replaces_an_earlier_agent_classification() {
        let mut tasks = Registry::default();
        signal_with_background(&frame("SubagentStart", None, r#"{"agent_id":"mate"}"#), &mut tasks);
        assert!(matches!(
            signal_with_background(
                &frame(
                    "Stop",
                    None,
                    r#"{"background_tasks":[{"id":"mate","type":"teammate","status":"running"}]}"#
                ),
                &mut tasks
            ),
            Some(Signal::Stop { blocking_tasks: false, teammates: 1, .. })
        ));
        assert_eq!(tasks.liveness(), Liveness::None);
    }

    #[test]
    fn stop_ignores_idle_and_terminal_tasks() {
        for status in ["idle", "completed", "failed", "stopped", "cancelled", "interrupted"] {
            let body = serde_json::json!({"background_tasks": [
                {"id":"agent", "type":"subagent", "status":status},
                {"id":"watch", "type":"monitor", "status":status},
                {"id":"mate", "type":"teammate", "status":status}
            ]})
            .to_string();
            assert!(matches!(
                signal_with_background(&frame("Stop", None, &body), &mut Registry::default()),
                Some(Signal::Stop {
                    blocking_tasks: false,
                    monitoring_tasks: false,
                    teammates: 0,
                    ..
                })
            ));
        }
    }

    #[test]
    fn nested_agent_survives_its_parent_and_shells_are_monitoring() {
        let mut tasks = Registry::default();
        signal_with_background(
            &frame(
                "PostToolUse",
                None,
                r#"{"agent_id":"parent","tool_name":"Agent","tool_response":{"agentId":"child","status":"async_launched"}}"#,
            ),
            &mut tasks,
        );
        signal_with_background(
            &frame("SubagentStop", None, r#"{"agent_id":"parent","background_tasks":[]}"#),
            &mut tasks,
        );
        // A parent's Stop is not a statement that its child finished.
        assert_eq!(tasks.liveness(), Liveness::Working);
        assert!(matches!(
            signal_with_background(
                &frame("Stop", None, r#"{"background_tasks":[{"id":"watch","type":"shell"}]}"#),
                &mut tasks
            ),
            Some(Signal::Stop { blocking_tasks: true, .. })
        ));
        assert_eq!(
            signal_with_background(
                &frame("SubagentStop", None, r#"{"agent_id":"child"}"#),
                &mut tasks
            ),
            Some(Signal::BackgroundChanged { liveness: Liveness::Monitoring })
        );
    }

    /// The Stop payload of a session that published an artifact (2.1.278):
    /// Claude Code keeps an ambient websocket watch on it for the session's
    /// whole life and lists it as a running `monitor`, `ambient` flag dropped.
    /// Without a Monitor tool result naming that id it is housekeeping, and
    /// the turn ended (T-408). The same row after the tool armed it parks.
    #[test]
    fn an_ambient_artifact_watch_is_not_a_park() {
        let stop = r#"{"background_tasks":[{"id":"sk3m9x2qp","type":"monitor","status":"running","description":"live updates for artifact plan (comments)"}]}"#;
        let mut tasks = Registry::default();
        assert!(matches!(
            signal_with_background(&frame("Stop", None, stop), &mut tasks),
            Some(Signal::Stop { blocking_tasks: false, monitoring_tasks: false, .. })
        ));
        assert_eq!(tasks.liveness(), Liveness::None);
        signal_with_background(
            &frame(
                "PostToolUse",
                None,
                r#"{"tool_name":"Monitor","tool_response":{"taskId":"sk3m9x2qp","timeoutMs":0,"persistent":true}}"#,
            ),
            &mut tasks,
        );
        assert!(matches!(
            signal_with_background(&frame("Stop", None, stop), &mut tasks),
            Some(Signal::Stop { blocking_tasks: false, monitoring_tasks: true, .. })
        ));
        // A daemon restart empties the registry: the armed watch is then
        // indistinguishable from an ambient one and reads as done — the
        // narrow hole the provenance rule accepts.
        let mut fresh = Registry::default();
        assert!(matches!(
            signal_with_background(&frame("Stop", None, stop), &mut fresh),
            Some(Signal::Stop { monitoring_tasks: false, .. })
        ));
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
            Some(Signal::Stop { blocking_tasks, monitoring_tasks, teammates, .. }) => {
                assert!(!blocking_tasks);
                // The `comments` watch is an ambient artifact watch (T-408).
                assert!(!monitoring_tasks);
                assert_eq!(teammates, 2);
            }
            other => panic!("expected Stop, got {other:?}"),
        }
        // A shell beside them does not count as active agent work.
        let body = r#"{"background_tasks":[{"type":"teammate"},{"type":"shell"}]}"#;
        match signal_of(&frame("Stop", None, body)) {
            Some(Signal::Stop { blocking_tasks, teammates, .. }) => {
                assert!(!blocking_tasks);
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
    fn a_moved_transcript_is_followed_once_the_old_file_is_gone() {
        use mesimon_core::board::{SessionKind, SessionState};
        let dir = std::env::temp_dir().join(format!("msmn-moved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = uuid::Uuid::new_v4();
        let old = dir.join(format!("{id}.jsonl"));
        let new = dir.join("elsewhere").join(format!("{id}.jsonl"));
        let mut rec = SessionRecord::new(
            id,
            SessionKind::Claude,
            ulid::Ulid::new(),
            vec![],
            "/r".into(),
            SessionState::Running,
        );
        rec.transcript_path = Some(old.to_string_lossy().into_owned());
        let f = |p: &std::path::Path| {
            frame(
                "PostToolUse",
                Some("Bash"),
                &format!(r#"{{"session_id":"x","transcript_path":"{}","cwd":"/w"}}"#, p.display()),
            )
        };
        // The recorded file still exists: a differing path is a copy, not a move.
        std::fs::write(&old, "").unwrap();
        assert_eq!(transcript_moved(&f(&new), &rec), None);
        // Same path: nothing to do.
        std::fs::remove_file(&old).unwrap();
        assert_eq!(transcript_moved(&f(&old), &rec), None);
        // Another session's file at the new place: identity never travels.
        let other = dir.join(format!("{}.jsonl", uuid::Uuid::new_v4()));
        assert_eq!(transcript_moved(&f(&other), &rec), None);
        // Gone here, named there, same uuid: follow it.
        assert_eq!(transcript_moved(&f(&new), &rec), Some(new.to_string_lossy().into_owned()));
        // A record with no path yet is SessionStart's to fill, not this road's.
        rec.transcript_path = None;
        assert_eq!(transcript_moved(&f(&new), &rec), None);
        std::fs::remove_dir_all(&dir).ok();
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
                monitoring_tasks: false,
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

    /// The plan rides the PostToolUse frame and only there: the approval
    /// dialog's frames come before the approval, another tool's `plan` key is
    /// not a plan, and a subagent's is not the session's. WHERE on the frame
    /// moved: 2.1.251–2.1.258 carried it in `tool_input.plan` beside a
    /// one-sentence response; 2.1.259 strips the input before the call and
    /// returns `{plan, isAgent, filePath, hasTaskTool}` (captured live
    /// 2026-09-03 — the shape on which a day of approvals landed nothing).
    #[test]
    fn an_approved_plan_is_read_off_post_tool_use_only() {
        let approved = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# A\n\n1. look"},"tool_response":"User has approved your plan."}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, approved)), Some("# A\n\n1. look".into()));
        assert_eq!(plan_of(&frame("PreToolUse", None, approved)), None);
        assert_eq!(plan_of(&frame("PermissionRequest", None, approved)), None);
        let v259 = r##"{"tool_name":"ExitPlanMode","tool_input":{},"tool_response":{"plan":"# Repro plan\n- touch nothing\n","isAgent":false,"filePath":"/h/.claude/plans/x.md","hasTaskTool":true}}"##;
        assert_eq!(
            plan_of(&frame("PostToolUse", None, v259)),
            Some("# Repro plan\n- touch nothing\n".into())
        );
        let both = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"old"},"tool_response":{"plan":"new"}}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, both)), Some("new".into()));
        let sub = r##"{"tool_name":"ExitPlanMode","tool_input":{},"tool_response":{"plan":"# A","isAgent":true}}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, sub)), None);
        let bash = r##"{"tool_name":"Bash","tool_input":{"plan":"# A"}}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, bash)), None);
        let nested =
            r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# A"},"agent_id":"a1"}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, nested)), None);
        let blank = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"  \n"}}"##;
        assert_eq!(plan_of(&frame("PostToolUse", None, blank)), None);
        assert_eq!(plan_of(&frame("PostToolUse", None, "{truncated")), None);
    }
}
