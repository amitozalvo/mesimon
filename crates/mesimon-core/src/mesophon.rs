//! The bounded owner-control surface. No paths, native argv, or local envelopes.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// File a ticket (T-497). It lands quietly: no agent starts, whatever
    /// the column says. The description becomes `notes[0]`; a tag must be
    /// one the board already has.
    Create {
        title: String,
        #[serde(default)]
        description: String,
        /// `None` lands it in the board's default column.
        #[serde(default)]
        column: Option<String>,
        #[serde(default)]
        tags: Vec<TagPick>,
    },
    /// Start an agent on a ticket that has none, or wake the one asleep
    /// on it (T-498, T-510): the board's Shift+Enter from a phone. The
    /// provider is the one the board's tiers give the ticket; `prompt` is
    /// the first turn's words, and blank it is the ticket's title and
    /// description on an empty seat, or a plain wake on a sleeping one.
    /// Answered `starting` (or `provisioning` while a worktree is cut),
    /// and the receipt turns `started` once the session runs.
    Start {
        ticket: String,
        #[serde(default)]
        prompt: Option<String>,
    },
    /// Retitle a ticket (T-530): the board's rename from a phone. A blank
    /// title is refused; the host scrubs and caps it as it does the desk's.
    Rename {
        ticket: String,
        title: String,
    },
    /// Move a ticket to a column (T-530), before the ticket `before` names,
    /// or at the column's end without one. In its own column it is a
    /// reorder. The board's own gates apply: a column that needs the work
    /// merged refuses an unmerged ticket.
    Move {
        ticket: String,
        column: String,
        #[serde(default)]
        before: Option<String>,
    },
    /// Put one of the board's tags on a ticket, replacing the one it wore on
    /// that group, or with no `name` take the group's tag off (T-530). A tag
    /// the board does not have is refused: a phone never adds a word.
    Tag {
        ticket: String,
        group: u8,
        #[serde(default)]
        name: Option<String>,
    },
}

/// A ticket a paired browser sealed for the host's mailbox (T-497): the
/// create op's fields and when it was written. Unknown fields are ignored,
/// not refused: a host may be older than the page that wrote the ticket.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailTicket {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub tags: Vec<TagPick>,
    /// When it was written, in the browser's clock; shown, never trusted.
    #[serde(default)]
    pub written_at: u64,
}

/// A tag a new ticket wears, spelled as the board's vocabulary spells it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagPick {
    pub group: u8,
    pub name: String,
}

/// One tag of the board's vocabulary, as the New ticket sheet offers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagOption {
    pub group: u8,
    pub name: String,
    /// Index into the tag tint ramp (`board::TAG_TINTS`), the TUI's colour.
    pub tint: u8,
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
    /// The tags the ticket wears, each with the TUI's tint (T-497).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagOption>,
    /// A ticket a paired browser filed, once it was picked up at the desk
    /// (T-497): the browser that filed it turns its ticks teal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<Picked>,
}

/// How a phone's ticket was picked up, and when: `by` is `desk` (its page was
/// opened in the TUI) or `agent` (an agent started on it). A word, never an
/// enum, so a newer host's word does not fail an older browser's board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picked {
    pub by: String,
    /// Milliseconds since the epoch, the host's clock.
    pub at: u64,
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
    /// When the agent entered `state`: milliseconds since the epoch, the
    /// host's clock (T-497).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// The step a working turn is on, one line: a tool call, or `thinking`.
    /// Read from the transcript, as the TUI's card reads it. Live only: a
    /// browser never keeps it with the remembered board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doing: Option<String>,
    /// The first line of the agent's latest reply. Live only, like `doing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
}

/// The page a pairing QR opens (T-497): the relay's browser origin with the
/// code in the fragment, which a browser never sends to the relay. The page
/// fills the code in and waits for Connect.
pub fn pair_link(origin: &str, code: &str) -> String {
    format!("{}/#pair={code}", origin.trim_end_matches('/'))
}

/// The longest line of an agent's words a phone is sent (T-497): one row.
pub const LINE_MAX_BYTES: usize = 200;

/// A reply's first line for a phone (T-497): the first line that says
/// something, without its heading, quote or bullet marker or its emphasis,
/// scrubbed where it leaves for another process and capped.
pub fn reply_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_start_matches(['#', '>']).trim_start();
    // A bullet needs its space: `*emphasis*` is not one.
    let line = ["- ", "* ", "+ "].iter().find_map(|b| line.strip_prefix(b)).unwrap_or(line);
    step_line(&line.replace("**", "").replace('`', ""))
}

/// A step's words on one row (T-497): scrubbed and capped, as `reply_line`.
pub fn step_line(step: &str) -> Option<String> {
    let flat = crate::text::scrub_text(&step.replace(['\n', '\t'], " "));
    crate::text::nonblank(crate::text::cap_bytes(&flat, LINE_MAX_BYTES))
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
        /// Where a ticket lands when it names no column (T-279).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default_column: Option<String>,
        /// What each column is for, in the owner's words (T-467).
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        column_descriptions: BTreeMap<String, String>,
        /// The board's tag vocabulary: every tag a new ticket may wear.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_tags: Vec<TagOption>,
    },
    Preview {
        lines: Vec<String>,
        /// The pane's width in cells (T-506), so the browser draws the lines
        /// as the screen they came from. Absent from an older host.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cols: Option<u16>,
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
    /// A filed ticket is on the board: its id, its key and where it landed.
    Created {
        ticket: String,
        key: String,
        column: String,
    },
    /// A rename, move or tag took (T-530); the board that follows shows it.
    Edited {
        ticket: String,
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

    /// A filed ticket names only a title; everything else takes the board's
    /// default, and a field the host does not know is refused, not dropped.
    #[test]
    fn a_filed_ticket_needs_a_title_and_defaults_the_rest() {
        let Request::Create { title, description, column, tags } =
            serde_json::from_str(r#"{"op":"create","title":"Fix it"}"#).unwrap()
        else {
            panic!("create")
        };
        assert_eq!(
            (title.as_str(), description.as_str(), column, tags),
            ("Fix it", "", None, vec![])
        );
        let full = r#"{"op":"create","title":"t","description":"d","column":"TODO","tags":[{"group":1,"name":"BUG"}]}"#;
        let Request::Create { column, tags, .. } = serde_json::from_str(full).unwrap() else {
            panic!("create")
        };
        assert_eq!(column.as_deref(), Some("TODO"));
        assert_eq!(tags, vec![TagPick { group: 1, name: "BUG".into() }]);
        for bad in [
            r#"{"op":"create"}"#,
            r#"{"op":"create","title":"t","autorun":true}"#,
            r#"{"op":"create","title":"t","tags":[{"group":1,"name":"BUG","tint":3}]}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    /// The sheet's facts ride the board reply beside what an older browser
    /// already reads, and a board without them still parses.
    #[test]
    fn the_board_reply_carries_the_sheet_facts_only_when_there_are_any() {
        let bare = Reply::Board {
            title: "b".into(),
            columns: vec!["TODO".into()],
            tickets: vec![],
            default_column: None,
            column_descriptions: BTreeMap::new(),
            allowed_tags: vec![],
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"result":"board","title":"b","columns":["TODO"],"tickets":[]})
        );
        let Reply::Board { default_column, allowed_tags, .. } =
            serde_json::from_value(json).unwrap()
        else {
            panic!("board")
        };
        assert!(default_column.is_none() && allowed_tags.is_empty());
        let created = serde_json::to_value(Reply::Created {
            ticket: "01J".into(),
            key: "T-7".into(),
            column: "TODO".into(),
        })
        .unwrap();
        assert_eq!(
            created,
            serde_json::json!({"result":"created","ticket":"01J","key":"T-7","column":"TODO"})
        );
    }

    /// The projection's newer facts are absent unless there is something to
    /// say, so an older browser reads the same bytes as before; and a reply
    /// without them, from an older host, still parses.
    #[test]
    fn a_ticket_carries_tags_pickup_and_the_agent_s_step_only_when_known() {
        let bare = Ticket {
            queued: None,
            id: "01J".into(),
            key: "T-1".into(),
            title: "t".into(),
            column: "TODO".into(),
            agent: Some(Agent {
                permission: None,
                dialog: None,
                session: "s".into(),
                provider: "claude".into(),
                state: "working".into(),
                promptable: true,
                since: None,
                doing: None,
                said: None,
            }),
            tags: vec![],
            picked: None,
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"queued":null,"id":"01J","key":"T-1","title":"t","column":"TODO",
                "agent":{"permission":null,"dialog":null,"session":"s","provider":"claude",
                "state":"working","promptable":true}})
        );
        let full = Ticket {
            tags: vec![TagOption { group: 1, name: "BUG".into(), tint: 3 }],
            picked: Some(Picked { by: "desk".into(), at: 1_790_000_000_000 }),
            agent: bare.agent.clone().map(|a| Agent {
                since: Some(1_790_000_000_000),
                doing: Some("Bash(cargo test)".into()),
                said: Some("Fixed.".into()),
                ..a
            }),
            ..bare
        };
        let back: Ticket = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back.tags, full.tags);
        assert_eq!(back.picked, full.picked);
        let agent = back.agent.unwrap();
        assert_eq!(
            (agent.since, agent.doing.as_deref(), agent.said.as_deref()),
            (Some(1_790_000_000_000), Some("Bash(cargo test)"), Some("Fixed."))
        );
    }

    /// A phone gets one plain row of a reply: markers, emphasis and hazards
    /// go, a long line is cut on a character, and silence stays silence.
    #[test]
    fn a_reply_reaches_a_phone_as_its_first_plain_line() {
        assert_eq!(
            reply_line("\n\n## Fixed, and three tests pass.\nmore").as_deref(),
            Some("Fixed, and three tests pass.")
        );
        assert_eq!(
            reply_line("- **Updated** `CHANGELOG.md`").as_deref(),
            Some("Updated CHANGELOG.md")
        );
        assert_eq!(reply_line("*emphasis* stays").as_deref(), Some("*emphasis* stays"));
        assert_eq!(reply_line("> quoted").as_deref(), Some("quoted"));
        assert_eq!(reply_line("a\u{202e}b\x07c").as_deref(), Some("abc"));
        assert_eq!(reply_line("  \n \n"), None);
        assert_eq!(reply_line("##"), None);
        let long = "é".repeat(150);
        let cut = reply_line(&long).unwrap();
        assert!(cut.len() <= LINE_MAX_BYTES && cut.chars().all(|c| c == 'é'));
        assert_eq!(step_line("Bash(cargo\ntest)").as_deref(), Some("Bash(cargo test)"));
    }

    /// The QR's page keeps the code off the wire: it rides the fragment.
    #[test]
    fn the_pairing_link_puts_the_code_in_the_fragment() {
        let code = "7K2M-QX4P-0B9D-RT6W-HN3C-5VJE-8FGA-1YSZ";
        for origin in ["https://remote.mesimon.dev", "https://remote.mesimon.dev/"] {
            assert_eq!(pair_link(origin, code), format!("https://remote.mesimon.dev/#pair={code}"));
        }
        assert_eq!(pair_link("http://localhost:8444", "C"), "http://localhost:8444/#pair=C");
    }

    /// A start names a ticket and, since T-510, the first turn's words;
    /// the provider and the mode are the host's, so a field that tries to
    /// pick one is refused. A page that sends no `prompt` (T-498's) still
    /// parses: the words are then the ticket's own.
    #[test]
    fn a_start_names_its_ticket_and_at_most_a_prompt() {
        let Request::Start { ticket, prompt } =
            serde_json::from_str(r#"{"op":"start","ticket":"01J"}"#).unwrap()
        else {
            panic!("start")
        };
        assert_eq!(ticket, "01J");
        assert_eq!(prompt, None);
        let Request::Start { prompt, .. } =
            serde_json::from_str(r#"{"op":"start","ticket":"01J","prompt":"fix the test"}"#)
                .unwrap()
        else {
            panic!("start")
        };
        assert_eq!(prompt.as_deref(), Some("fix the test"));
        for bad in [
            r#"{"op":"start"}"#,
            r#"{"op":"start","ticket":"01J","provider":"codex"}"#,
            r#"{"op":"start","ticket":"01J","plan":true}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
    }

    /// The card edits (T-530) name the ticket and what changes, nothing
    /// else; a move without `before` lands at the column's end, and a tag
    /// without `name` takes the group's tag off.
    #[test]
    fn a_card_edit_names_its_ticket_and_only_what_changes() {
        let Request::Move { ticket, column, before } =
            serde_json::from_str(r#"{"op":"move","ticket":"01J","column":"DONE"}"#).unwrap()
        else {
            panic!("move")
        };
        assert_eq!((ticket.as_str(), column.as_str(), before), ("01J", "DONE", None));
        let Request::Move { before, .. } =
            serde_json::from_str(r#"{"op":"move","ticket":"01J","column":"DONE","before":"01K"}"#)
                .unwrap()
        else {
            panic!("move")
        };
        assert_eq!(before.as_deref(), Some("01K"));
        let Request::Tag { group, name, .. } =
            serde_json::from_str(r#"{"op":"tag","ticket":"01J","group":2}"#).unwrap()
        else {
            panic!("tag")
        };
        assert_eq!((group, name), (2, None));
        let Request::Rename { title, .. } =
            serde_json::from_str(r#"{"op":"rename","ticket":"01J","title":"Fix it"}"#).unwrap()
        else {
            panic!("rename")
        };
        assert_eq!(title, "Fix it");
        for bad in [
            r#"{"op":"rename","ticket":"01J"}"#,
            r#"{"op":"move","ticket":"01J"}"#,
            r#"{"op":"move","ticket":"01J","column":"DONE","force":true}"#,
            r#"{"op":"tag","ticket":"01J","group":1,"name":"NEW","register":true}"#,
        ] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
        assert_eq!(
            serde_json::to_value(Reply::Edited { ticket: "01J".into() }).unwrap(),
            serde_json::json!({"result":"edited","ticket":"01J"})
        );
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
