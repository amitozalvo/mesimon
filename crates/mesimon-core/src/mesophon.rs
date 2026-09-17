//! The bounded owner-control surface. No paths, native argv, or local envelopes.
use serde::{Deserialize, Serialize};

/// An explicit one-shot human answer, never a policy or input rewrite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
}
impl PermissionDecision {
    pub fn hook_output(self) -> serde_json::Value {
        serde_json::json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest", "decision": {"behavior": self}
        }})
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Permission {
    pub request: String,
    pub tool: String,
    pub input: serde_json::Value,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Info {
    pub enabled: bool,
    pub connected: bool,
    pub origin: String,
    pub code: Option<String>,
    pub error: Option<String>,
    pub devices: Vec<Device>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub grant: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalAction {
    Status,
    Enable,
    Disable,
    Pair,
    Revoke { grant: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Snapshot,
    Foreground {
        ticket: Option<String>,
    },
    Dialog {
        ticket: String,
        session: String,
        request: String,
        response: DialogAnswer,
    },
    Permission {
        ticket: String,
        session: String,
        request: String,
        decision: PermissionDecision,
    },
    Preview {
        ticket: String,
        session: String,
    },
    Prompt {
        ticket: String,
        session: String,
        text: String,
        #[serde(default = "queue_by_default")]
        queued: bool,
    },
    SendNow {
        ticket: String,
        session: String,
    },
    TakeBack {
        ticket: String,
        session: String,
    },
    Status {
        command: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub incarnation: String,
    pub id: u64,
    pub request: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ticket {
    #[serde(default)]
    pub queued: Option<String>,
    pub id: String,
    pub key: String,
    pub title: String,
    pub column: String,
    pub agent: Option<Agent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    #[serde(default)]
    pub permission: Option<Permission>,
    #[serde(default)]
    pub dialog: Option<Dialog>,
    pub session: String,
    pub provider: String,
    pub state: String,
    pub promptable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    Ready {
        incarnation: String,
        next: u64,
        #[serde(default)]
        features: Vec<String>,
    },
    Board {
        title: String,
        columns: Vec<String>,
        tickets: Vec<Ticket>,
    },
    Preview {
        lines: Vec<String>,
    },
    Delivery {
        status: String,
    },
    Rejected {
        message: String,
    },
    TakenBack {
        text: String,
    },
    Awareness {
        ticket: String,
        awareness: Awareness,
        alert: bool,
    },
    Changed,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Answer {
    pub id: u64,
    pub reply: Reply,
}

fn queue_by_default() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_hosts_have_no_m2_features() {
        let Reply::Ready { features, .. } =
            serde_json::from_str::<Reply>(r#"{"result":"ready","incarnation":"old","next":1}"#)
                .unwrap()
        else {
            panic!("ready")
        };
        assert!(features.is_empty());
    }

    #[test]
    fn approval_is_a_one_shot_native_decision_without_rules_or_input_changes() {
        for (decision, word) in
            [(PermissionDecision::Allow, "allow"), (PermissionDecision::Deny, "deny")]
        {
            assert_eq!(
                decision.hook_output(),
                serde_json::json!({"hookSpecificOutput": {
                    "hookEventName": "PermissionRequest", "decision": {"behavior": word}
                }})
            );
        }
        assert!(serde_json::from_str::<PermissionDecision>("\"ask\"").is_err());
    }

    #[test]
    fn awareness_requires_real_completion_and_preserves_attention_ranks() {
        use crate::board::{Reason as R, SessionState as S, StopReason as E};
        assert_eq!(Phase::of(&S::Idle { stop_reason: E::EndTurn }), Phase::Completed);
        for stop_reason in [E::Interrupted, E::Unknown] {
            assert_eq!(Phase::of(&S::Idle { stop_reason }), Phase::Stale);
        }
        for reason in [R::Permission, R::Plan, R::Trust] {
            assert_eq!(Phase::of(&S::RequiresAction { reason }), Phase::WaitingForApproval);
        }
        for reason in [
            R::Question,
            R::Secret,
            R::Elicitation,
            R::Auth,
            R::QuotaResume,
            R::StartupModal,
            R::ResumeDialog,
        ] {
            assert_eq!(Phase::of(&S::RequiresAction { reason }), Phase::WaitingForInput);
        }
        assert_eq!(Phase::of(&S::Running), Phase::Running);
        assert_eq!(Phase::of(&S::Spawning), Phase::Starting);
        assert_eq!(Phase::of(&S::Sleeping), Phase::Stale);
    }

    #[test]
    fn phone_composers_queue_unless_they_explicitly_steer() {
        let request = r#"{"op":"prompt","ticket":"t","session":"s","text":"next"}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(request).unwrap(),
            Request::Prompt { queued: true, .. }
        ));
        let request = r#"{"op":"prompt","ticket":"t","session":"s","text":"next","queued":false}"#;
        assert!(matches!(
            serde_json::from_str::<Request>(request).unwrap(),
            Request::Prompt { queued: false, .. }
        ));
    }
}

/// A small daemon-computed notification payload. No terminal or board contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Starting,
    Running,
    WaitingForApproval,
    WaitingForInput,
    Completed,
    Failed,
    Stale,
}
impl Phase {
    pub fn of(state: &crate::board::SessionState) -> Self {
        use crate::board::{Reason, SessionState as S, StopReason};
        match state {
            S::Spawning => Self::Starting,
            S::Running
            | S::Idle { stop_reason: StopReason::Background | StopReason::Monitoring } => {
                Self::Running
            }
            S::RequiresAction { reason: Reason::Permission | Reason::Plan | Reason::Trust } => {
                Self::WaitingForApproval
            }
            S::RequiresAction { .. } => Self::WaitingForInput,
            S::Idle { stop_reason: StopReason::EndTurn } => Self::Completed,
            S::Failed { .. } => Self::Failed,
            _ => Self::Stale,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Awareness {
    pub phase: Phase,
    pub headline: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(rename = "deepLink")]
    pub deep_link: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dialog {
    pub request: String,
    #[serde(flatten)]
    pub content: DialogContent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DialogContent {
    Questions { questions: Vec<Question> },
    Plan { markdown: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    #[serde(rename = "multiSelect", default)]
    pub multi_select: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case", deny_unknown_fields)]
pub enum DialogAnswer {
    Choice { index: usize },
    Text { text: String },
    Accept,
    Reject,
}
