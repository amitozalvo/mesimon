//! The MCP surface, as pure data: what the server advertises, what it accepts,
//! and what an agent principal is allowed to ask the daemon for.
//!
//! Everything here is testable without a socket. The shim
//! (`crates/mesimon/src/mcp.rs`) is I/O only and holds no policy — it runs
//! inside the agent's own process tree, so nothing it decides can be trusted.
//!
//! Three rules this module exists to enforce:
//!
//! 1. **`initialize.result.instructions` is never set.** It reaches the model
//!    verbatim with no tool call — the single largest injection surface an MCP
//!    server has. `initialize_result` cannot emit the key and a test says so.
//! 2. **Tool text is permanent context**, paid on every request of every
//!    session. `lint_tool_text` refuses anything that reads as an instruction
//!    rather than a description, and `MAX_TOOL_BYTES` caps the bill.
//! 3. **Column names never enter a schema.** `to_column` is a plain string
//!    validated server-side. An `enum` would put the *user's* column names into
//!    every request forever — a column called `DO_NOT_SELF_APPROVE` would
//!    become an instruction nobody wrote and no lint could see.

use serde_json::{json, Value};

use crate::command::Command;

/// The server name, and therefore the `mcp__mesimon__*` tool prefix the model
/// sees. Changing it renames every tool.
pub const SERVER_NAME: &str = "mesimon";

/// MCP revisions this server speaks, newest first. `initialize` echoes the
/// client's requested revision when it is one of these, and otherwise answers
/// with the first — which is what the spec asks for, and what makes Claude
/// Code's `2026-07-28` probe fall back cleanly instead of failing.
pub const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// Per-tool ceiling on the serialized definition, in bytes.
///
/// 223 tokens was measured on an ~818-byte tool definition (docs/15 §1.4), so
/// this cap is what keeps that measurement a ceiling rather than an average the
/// budget merely hopes for. Three tools ≈ 670 tok on every request of every
/// session, forever — deferred tool search is force-disabled for proxy and
/// Bedrock/Vertex users, so 223/tool is the planning number, not 12.
pub const MAX_TOOL_BYTES: usize = 820;

/// JSON-RPC: the method is not implemented. Used for every method mesimon
/// deliberately does not answer.
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// Methods mesimon answers. Everything else — including `skills/list`,
/// `server/discover`, `resources/list`, `prompts/list` and
/// `sampling/createMessage` — gets `-32601`.
///
/// `skills/list` is not in any MCP spec: Claude Code calls it when a server
/// declares `capabilities.resources`, fetches each SKILL.md by `resources/read`
/// and registers it as a real skill, name and description into the system
/// prompt. It is the one injection surface that is invisible in `tools/list`.
/// mesimon declares no resources capability *and* refuses the method.
pub const HANDLED_METHODS: [&str; 5] =
    ["initialize", "notifications/initialized", "ping", "tools/list", "tools/call"];

// ---------------------------------------------------------------- tool text

/// Words that turn a description into an instruction. Tool text is injected
/// into every request; it may describe, and it may not tell the model what to
/// do or address it directly.
const BANNED_SUBSTRINGS: [&str; 14] = [
    "you ",
    "your ",
    "yours",
    "must ",
    "always ",
    "never ",
    "should ",
    "do not",
    "don't",
    "make sure",
    "remember",
    "important:",
    "note that",
    "be sure",
];

/// Reject text that reads as an instruction rather than a description.
///
/// Deliberately crude and deliberately noisy: a false positive costs one
/// rewrite at compile time, and a false negative ships a sentence into every
/// request of every session for the life of the product.
pub fn lint_tool_text(s: &str) -> Result<(), String> {
    let lower = s.to_lowercase();
    for bad in BANNED_SUBSTRINGS {
        if lower.contains(bad) {
            return Err(format!("tool text reads as an instruction: {bad:?} in {s:?}"));
        }
    }
    // Shouting is an instruction too. Acronyms up to 3 letters are fine (MCP).
    for word in s.split(|c: char| !c.is_ascii_alphabetic()) {
        if word.len() >= 4 && word.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(format!("tool text shouts: {word:?} in {s:?}"));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------- tools

/// The complete tool surface. Three tools, and there is deliberately no tool
/// to spawn a session, kill a session, delete a ticket, archive a ticket,
/// rename a ticket, change a workspace, merge a branch, read a transcript, read
/// a cost, or grant anything. A tool that does not exist cannot be granted by
/// accident at 11pm, and cannot be talked into firing by injected ticket text.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "get_ticket",
            "description": "Returns the mesimon ticket this session is attached to: key, \
                            title, current column, workspace mode, branch, merge state, and \
                            the column names move_ticket accepts.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
        json!({
            "name": "list_board",
            "description": "Returns the mesimon board: every column in order, and every \
                            ticket's key, title and column. Session and process information \
                            is excluded.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        }),
        json!({
            "name": "move_ticket",
            "description": "Moves this session's ticket to another column of the mesimon \
                            board. Accepted destinations are listed by get_ticket as \
                            allowed_columns; a name outside that set is refused.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    // A plain string, NOT an enum: see the module header.
                    "to_column": {
                        "type": "string",
                        "description": "Destination column name, as spelled in allowed_columns.",
                    },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Optional. Repeating a call with the same key replays \
                                        the first result instead of moving twice.",
                    },
                },
                "required": ["to_column"],
                "additionalProperties": false,
            },
        }),
    ]
}

/// `initialize`'s result.
///
/// Declares `tools` and nothing else — no `resources` (which is what makes
/// Claude Code probe the non-spec `skills/list`), no `prompts`, no `logging`
/// (a dead channel: a `notifications/message` during a tool call never reaches
/// the model), no `subscribe`, no `listChanged`.
///
/// There is no `instructions` key, and there is no code path that can add one.
pub fn initialize_result(client_protocol: Option<&str>) -> Value {
    let protocol = client_protocol
        .filter(|p| SUPPORTED_PROTOCOLS.contains(p))
        .unwrap_or(SUPPORTED_PROTOCOLS[0]);
    json!({
        "protocolVersion": protocol,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
    })
}

/// One parsed `tools/call`. The ticket is never a parameter: it comes from the
/// session the connection is bound to, so an agent cannot address another
/// ticket even by guessing an id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCall {
    GetTicket,
    ListBoard,
    MoveTicket { to_column: String, idempotency_key: Option<String> },
}

/// Parse a `tools/call` into a `ToolCall`, or produce the message the agent
/// sees. Unknown tool names are a plain error: the model sometimes holds a
/// `tool_reference` for a tool that no longer exists.
pub fn parse_tool_call(name: &str, args: &Value) -> Result<ToolCall, String> {
    match name {
        "get_ticket" => Ok(ToolCall::GetTicket),
        "list_board" => Ok(ToolCall::ListBoard),
        "move_ticket" => {
            let to_column = args
                .get("to_column")
                .and_then(Value::as_str)
                .ok_or("move_ticket requires a to_column string")?
                .trim()
                .to_string();
            if to_column.is_empty() {
                return Err("to_column is empty".into());
            }
            Ok(ToolCall::MoveTicket {
                to_column,
                idempotency_key: args
                    .get("idempotency_key")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            })
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

// ------------------------------------------------------------- the tier

/// May a `Principal::Agent` ask for this command?
///
/// The match is **exhaustive with no `_` arm on purpose**. Adding a command to
/// the wire protocol will not compile until someone comes here and decides,
/// deliberately, whether an agent may call it. That is the whole enforcement
/// mechanism for D10's never-tier: it is not a list somebody has to remember to
/// update, it is a compile error.
pub fn agent_allows(cmd: &Command) -> bool {
    match cmd {
        // The tier. Three tools, three commands.
        Command::AgentGetTicket | Command::AgentListBoard | Command::AgentMoveTicket { .. } => true,

        // Everything below is the never-tier. An agent may not spawn or kill a
        // session, delete or archive or rename a ticket, change a workspace,
        // merge a branch, read a diff, take the focus token, or stop the
        // daemon — and there is no tool that would let it try.
        //
        // Tags (T-83) join it, all five. Four of them mutate the REGISTRY,
        // which is board-wide state: `ForgetTag` strips a tag from every
        // ticket wearing it, `RenameTag` rewrites it everywhere, and both
        // are precisely the "an agent cannot change the board itself" rule
        // `authorize` already states. `SetTag` touches only the caller's own
        // ticket and is the arguable one, but it stays out for the reason
        // docs/15 §1.4 gave before tags existed: D10 defines its tiers by
        // enumeration and none of them has a home for structured non-column
        // state, tagging is a human curation act with a one-key path (`^t`),
        // and a tool nobody asked for still costs ~223 tokens on every
        // request of every session, forever.
        Command::SetTag { .. }
        | Command::ForgetTag { .. }
        | Command::RegisterTag { .. }
        | Command::RenameTag { .. }
        | Command::SetTagColor { .. }
        | Command::Hello { .. }
        | Command::Snapshot
        | Command::Subscribe
        | Command::CreateTicket { .. }
        | Command::RenameTicket { .. }
        | Command::DeleteTicket { .. }
        | Command::SetWorkspace { .. }
        | Command::MergeTicket { .. }
        | Command::MergeToAgent { .. }
        | Command::RestoreTicket { .. }
        | Command::ArchiveTicket { .. }
        | Command::UnarchiveTicket { .. }
        | Command::ArchiveAll
        // The environment every future pane gets, board-wide and shared by
        // every session. An agent asking to re-read the user's rc files would
        // be running the user's shell on its own say-so.
        | Command::ReloadShellEnv
        | Command::MoveTicket { .. }
        | Command::SpawnSession { .. }
        | Command::KillSession { .. }
        | Command::RescanExternal
        | Command::AttachExternal { .. }
        | Command::ResumeExternal { .. }
        | Command::ResumeSession { .. }
        | Command::SleepSession { .. }
        | Command::WakeSession { .. }
        | Command::ReclaimAll
        | Command::PinAwake { .. }
        | Command::FocusStart { .. }
        | Command::FocusEnd { .. }
        | Command::GateStatus
        | Command::GatePassed
        | Command::Shutdown
        | Command::DiffList { .. }
        | Command::DiffFile { .. }
        // A pane's contents are the session itself. `authorize` already
        // denies an agent `Resource::Session` at every action, and this is
        // the same rule stated where a new command has to walk past it.
        | Command::PaneTail { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_three_tools() {
        let t = tools();
        assert_eq!(t.len(), 3);
        let names: Vec<&str> = t.iter().filter_map(|v| v["name"].as_str()).collect();
        assert_eq!(names, ["get_ticket", "list_board", "move_ticket"]);
    }

    /// The token bill, enforced. Every byte here is paid on every request of
    /// every session for as long as the product exists.
    #[test]
    fn every_tool_fits_the_budget() {
        for t in tools() {
            let bytes = serde_json::to_string(&t).unwrap().len();
            let name = t["name"].as_str().unwrap();
            assert!(bytes <= MAX_TOOL_BYTES, "{name} is {bytes} bytes, cap is {MAX_TOOL_BYTES}");
        }
    }

    /// Descriptions describe; they do not instruct. This walks the tool
    /// definitions the way the model reads them — the top-level description and
    /// every parameter description.
    #[test]
    fn all_shipped_tool_text_passes_the_lint() {
        for t in tools() {
            lint_tool_text(t["description"].as_str().unwrap()).unwrap();
            if let Some(props) = t["inputSchema"]["properties"].as_object() {
                for (k, v) in props {
                    if let Some(d) = v["description"].as_str() {
                        lint_tool_text(d).unwrap_or_else(|e| panic!("{k}: {e}"));
                    }
                }
            }
        }
    }

    #[test]
    fn the_lint_catches_what_it_is_for() {
        assert!(lint_tool_text("You must always move the ticket.").is_err());
        assert!(lint_tool_text("Never call this tool twice.").is_err());
        assert!(lint_tool_text("IMPORTANT: read this.").is_err());
        assert!(lint_tool_text("Returns the ticket this session is attached to.").is_ok());
    }

    /// The single largest available injection surface. There is no code path
    /// that sets it, and this is the test that keeps it that way.
    #[test]
    fn initialize_never_carries_instructions() {
        for p in [None, Some("2025-11-25"), Some("2026-07-28"), Some("nonsense")] {
            let v = initialize_result(p);
            assert!(v.get("instructions").is_none());
            assert!(!serde_json::to_string(&v).unwrap().contains("instructions"));
        }
    }

    /// Declaring `resources` is what makes Claude Code probe `skills/list`,
    /// which registers SKILL.md bodies into the system prompt. mesimon
    /// declares tools and nothing else.
    #[test]
    fn only_the_tools_capability_is_declared() {
        let caps = initialize_result(None)["capabilities"].clone();
        assert_eq!(caps, json!({ "tools": {} }));
    }

    #[test]
    fn protocol_is_echoed_when_known_and_pinned_when_not() {
        assert_eq!(initialize_result(Some("2025-06-18"))["protocolVersion"], "2025-06-18");
        assert_eq!(initialize_result(Some("2026-07-28"))["protocolVersion"], "2025-11-25");
        assert_eq!(initialize_result(None)["protocolVersion"], "2025-11-25");
    }

    /// A board with a column called `DO_NOT_SELF_APPROVE` would otherwise
    /// inject that string into every request of every session.
    #[test]
    fn no_column_name_can_reach_a_schema() {
        let blob = serde_json::to_string(&tools()).unwrap();
        assert!(!blob.contains("\"enum\""), "an enum in a schema is model-visible prose");
    }

    #[test]
    fn move_ticket_parses_and_refuses() {
        assert_eq!(
            parse_tool_call("move_ticket", &json!({"to_column": "REVIEW"})).unwrap(),
            ToolCall::MoveTicket { to_column: "REVIEW".into(), idempotency_key: None }
        );
        assert_eq!(
            parse_tool_call(
                "move_ticket",
                &json!({"to_column": " REVIEW ", "idempotency_key": "k"})
            )
            .unwrap(),
            ToolCall::MoveTicket { to_column: "REVIEW".into(), idempotency_key: Some("k".into()) }
        );
        assert!(parse_tool_call("move_ticket", &json!({})).is_err());
        assert!(parse_tool_call("move_ticket", &json!({"to_column": "  "})).is_err());
        assert!(parse_tool_call("get_session", &json!({})).is_err());
    }

    #[test]
    fn no_argument_tools_ignore_arguments() {
        assert_eq!(
            parse_tool_call("get_ticket", &json!({"ticket": "OTHER-9"})).unwrap(),
            ToolCall::GetTicket
        );
        assert_eq!(parse_tool_call("list_board", &json!(null)).unwrap(), ToolCall::ListBoard);
    }

    /// Every tool has a command, and every allowed command has a tool. A
    /// command an agent may send that no tool can reach would be a hole nobody
    /// is looking at.
    #[test]
    fn the_tier_is_exactly_three_commands() {
        let allowed = [
            Command::AgentGetTicket,
            Command::AgentListBoard,
            Command::AgentMoveTicket { to_column: "X".into(), idempotency_key: None },
        ];
        for c in &allowed {
            assert!(agent_allows(c), "{c:?} should be in the tier");
        }
        assert_eq!(allowed.len(), tools().len());
    }

    /// The never-tier, named one command at a time. This is the list a reader
    /// checks when they ask "can an agent do X".
    #[test]
    fn the_never_tier_holds() {
        let t = ulid::Ulid::nil();
        let s = uuid::Uuid::nil();
        let denied = vec![
            Command::Hello { version: 1, client: "x".into() },
            Command::Snapshot,
            Command::Subscribe,
            Command::CreateTicket { column: "TODO".into(), title: "t".into() },
            Command::RenameTicket { id: t, title: "t".into() },
            Command::DeleteTicket { id: t, discard_worktree: true },
            Command::SetWorkspace { id: t, workspace: None },
            Command::SetTag { id: t, group: 1, name: Some("urgent".into()) },
            Command::ForgetTag { group: 1, name: "urgent".into() },
            Command::RegisterTag { group: 1, name: "urgent".into() },
            Command::RenameTag { group: 1, from: "a".into(), to: "b".into() },
            Command::SetTagColor { group: 1, name: "urgent".into(), color: 2 },
            Command::MergeTicket { id: t },
            Command::RestoreTicket { id: t },
            Command::ArchiveTicket { id: t },
            Command::UnarchiveTicket { id: t },
            Command::ArchiveAll,
            Command::MoveTicket { id: t, column: "DONE".into(), before: None },
            Command::SpawnSession {
                ticket: t,
                kind: crate::board::SessionKind::Claude,
                submit_prompt: true,
            },
            Command::KillSession { id: s },
            Command::RescanExternal,
            Command::AttachExternal { claude_session_id: s, ticket: None },
            Command::ResumeExternal { claude_session_id: s, ticket: None, confirm: true },
            Command::ResumeSession { id: s, confirm: true },
            Command::SleepSession { id: s },
            Command::WakeSession { id: s },
            Command::ReclaimAll,
            Command::PinAwake { id: s, pinned: true },
            Command::FocusStart { session: s },
            Command::FocusEnd { session: s },
            Command::GateStatus,
            Command::GatePassed,
            Command::Shutdown,
            Command::DiffList { ticket: t },
            Command::DiffFile { ticket: t, path: "a".into(), context: 3 },
            Command::PaneTail { session: s, lines: 20 },
        ];
        for c in &denied {
            assert!(!agent_allows(c), "{c:?} must stay out of the tier");
        }
    }
}
