//! The bounded owner-control surface. No paths, native argv, or local envelopes.
use serde::{Deserialize, Serialize};

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
    pub session: String,
    pub provider: String,
    pub state: String,
    pub promptable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    Ready { incarnation: String, next: u64 },
    Board { title: String, columns: Vec<String>, tickets: Vec<Ticket> },
    Preview { lines: Vec<String> },
    Delivery { status: String },
    Rejected { message: String },
    TakenBack { text: String },
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
