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

use crate::board::AgentTools;
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
/// budget merely hopes for. `mesimon doctor --mcp` computes the current registry's
/// total at 223 tokens per tool; deferred tool search is force-disabled for proxy
/// and Bedrock/Vertex users, so that is the planning number, not 12.
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

/// What a crowned agent is told about the board's wake (T-414, T-537), in
/// the `start_agent` and `ask_agent` receipts and on its own `get_ticket`
/// view: transient result data, never tool text, so it may instruct. It is
/// said where the crown would otherwise go looking — a crown on another
/// board armed a git monitor to learn what its workers did, not knowing the
/// board would tell it, and that monitor made it read as busy, which held
/// the very wake it was waiting for (`session_idle`).
pub const CROWN_WAKES: &str = "The board wakes this session on its own: when an agent the crown \
                               started delivers, is merged, answers the crown's ask or raises \
                               its hand, one sentence naming the ticket and what changed arrives \
                               as this session's next prompt, once it is idle. Nothing needs \
                               polling. A background task or monitor left running makes this \
                               session read as busy, and the wake and every queued word wait \
                               until it ends. A worker whose branch is merged is finished: \
                               sleep_agent parks it, which frees its seat in the crown's \
                               budget, and archive_ticket then takes its ticket off the board \
                               and reclaims a merged worktree. That is the crown's to do, not a \
                               person's to be asked for; a person's own agent is the one the \
                               crown may not park.";

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

/// The complete tool surface. Eight tools, and there is deliberately no tool
/// to spawn a session, kill a session, delete a ticket, archive a ticket,
/// rename a ticket, change a workspace, merge a branch, read a transcript, read
/// a cost, or grant anything. A tool that does not exist cannot be granted by
/// accident at 11pm, and cannot be talked into firing by injected ticket text.
///
/// The two note tools are D10's T1 ANNOTATE tier — the one tier that names a
/// home for text an agent writes about its own ticket, which is the argument
/// tags never had (see `agent_allows`). They cost what every tool costs, and
/// they are what lets a description reach the agent without a single token
/// being injected into its conversation.
///
/// `create_ticket` is the one tool that touches a ticket other than the
/// caller's — by making it. It is what an agent does with work it found and
/// was not asked for: file it where a human will see it, instead of doing it
/// unasked or dropping it. The new card has no session and no tool starts
/// one; the human decides what happens next, on the board.
///
/// `tag_ticket` (T-164) puts one of the board's EXISTING tags on the caller's
/// ticket, or takes it off. The vocabulary stays the human's: an agent may
/// pick from the registry and may not add to it — a name it invents is
/// refused, not registered — so ten tickets tagged by ten agents still speak
/// one language, and nothing an agent does fills an axis or lands on the
/// wrong one. Which names exist, what each axis means and what colour each
/// wears are the user's, through the picker.
///
/// `raise_hand` (T-107) is the one tool that reaches the LOUD register — the
/// `!` mark, the one saturated colour, the header's count, the tmux status
/// line — and it reaches exactly one card: the caller's own. It exists
/// because the board could not tell "I finished the refactor" from "I cannot
/// proceed until somebody chooses an auth provider": both end a turn, both
/// land in REVIEW, and only one of them is waiting on a person. Claude Code's
/// own `AskUserQuestion` already lights a card, but it FREEZES the turn on a
/// modal in the pane; this is the same message with the turn over and the
/// answer owed whenever the user likes.
///
/// The `reason` is required rather than optional on purpose. A mark with no
/// words makes the user open the ticket to learn anything at all, which is
/// the cost the board exists to remove — and requiring it is also the one
/// honest way to ask the model whether it has something to say, given that
/// tool text may describe and may never instruct.
pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "get_ticket",
            "description": "Returns the mesimon ticket this session is attached to, or with \
                            key another ticket (crown only): key, title, column, workspace, \
                            branch, merge state (per repo on a workspace), the column names \
                            move_ticket accepts and what each column is for, tags, \
                            every tag the board knows (allowed_tags), the description (first \
                            note), every note's id, name and author, the agent's state word \
                            and a seen stamp keyed edits require. The prompt that starts a \
                            session is often the ticket's title alone; the description and \
                            notes are the rest of the brief, so this is a session's first call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Optional. Another ticket's key; crown only." },
                },
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": "list_board",
            "description": "Returns the mesimon board: every column in order, what each \
                            column is for in the user's words (column_descriptions), and every \
                            ticket's key, title and column. Session and process information \
                            is excluded. Title comparison here is the pre-check for \
                            create_ticket: work extending a ticket in todo, in progress \
                            or review belongs on that ticket as scope, not as a sibling. \
                            A near-duplicate title means the older ticket wins.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": "move_ticket",
            // At the byte cap: every clause here paid for itself by dropping
            // one elsewhere (T-376 bought the automove sentence).
            "description": "Moves this session's ticket, or with key another's (crown only), \
                            to a column in get_ticket's allowed_columns; others are refused. \
                            The board itself moves it (get_ticket's automove) when a turn \
                            starts or ends. Position is priority; the same column reorders.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    // A plain string, NOT an enum: see the module header.
                    "to_column": {
                        "type": "string",
                        "description": "A name from allowed_columns.",
                    },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Optional. Same key replays result.",
                    },
                    "key": { "type": "string", "description": "Optional. Another ticket's key; crown only." },
                    "before": {
                        "type": "string",
                        "description": "Optional. Land above this key; default top.",
                    },
                    "seen": {
                        "type": "string",
                        "description": "With key: get_ticket's seen stamp; stale refuses.",
                    },
                },
                "required": ["to_column"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "read_note",
            "description": "Returns the full markdown text of one note on this session's \
                            ticket, or with key another ticket's (crown only). Note ids are \
                            listed by get_ticket.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "A note id from get_ticket." },
                    "key": { "type": "string", "description": "Optional. Another ticket's key; crown only." },
                },
                "required": ["note"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": "read_attachment",
            "description": "Returns one PNG picture on this session's ticket as image content. Attachment ids appear in mesimon-attachment links in the description and notes.",
            "inputSchema": { "type": "object", "properties": {
                "attachment": { "type": "string", "description": "An attachment id from a note's image link." }
            }, "required": ["attachment"], "additionalProperties": false },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": "write_note",
            // The plan clause (T-459): agents saved their approved plan here
            // by hand, a second copy of the note `record_plan` had already
            // written off the same approval.
            "description": "Creates a markdown note on this session's ticket, or with key \
                            another ticket's (crown only), or replaces the whole text of an \
                            existing one. The first note is the ticket's description. Empty \
                            text deletes an existing note. Text past 32 KiB is refused, not \
                            cut; the refusal names both sizes. A plan from plan mode is \
                            already a note, saved by the board and revised on each re-plan.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": {
                        "type": "string",
                        "description": "Optional. The id of the note to replace; omitted \
                                        creates a new note.",
                    },
                    "text": {
                        "type": "string",
                        "description": "The note's whole markdown text, at most 32 KiB.",
                    },
                    "key": { "type": "string", "description": "Optional. Another ticket's key; crown only." },
                },
                "required": ["text"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "create_ticket",
            // At the byte cap: the first clause says WHO works a filed ticket
            // (T-415 — an agent offered to build a sibling ticket in its own
            // session), paid for by "returns key" (the result carries it) and
            // two schema descriptions losing a word.
            "description": "Creates a ticket for a session of its own, not this one. \
                            One ticket is a work unit to pick up, not an idea/list \
                            row; findings on one surface share a ticket with a list. \
                            list_board checks scope/duplicates first. Research belongs \
                            in this ticket's notes; the user chooses tickets. Agents \
                            cannot delete tickets; cleanup costs the user.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    // A plain string, NOT an enum: see the module header.
                    "column": {
                        "type": "string",
                        "description": "list_board column; default if omitted.",
                    },
                    "description": {
                        "type": "string",
                        "description": "First note (markdown).",
                    },
                    // Names, NOT the registry: see the module header on enums.
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "allowed_tags; one per group.",
                    },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Same key replays.",
                    },
                },
                "required": ["title"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "tag_ticket",
            "description": "Puts one of the board's existing tags on this session's ticket, \
                            or with key another ticket (crown only), or takes one off. A \
                            ticket wears one tag per group, so a tag replaces its groupmate. \
                            Names accepted are get_ticket's allowed_tags; new tags are made \
                            on the board.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    // A plain string, NOT an enum: see the module header. A tag
                    // name is the user's word exactly as a column name is.
                    "name": {
                        "type": "string",
                        "description": "A tag name, as spelled in allowed_tags.",
                    },
                    "group": {
                        "type": "integer",
                        "description": "Optional, 1-10. Needed only when the same name exists \
                                        in more than one group.",
                    },
                    "remove": {
                        "type": "boolean",
                        "description": "Optional. True takes the tag off instead of putting it on.",
                    },
                    "key": { "type": "string", "description": "Optional. Another ticket's key; crown only." },
                },
                "required": ["name"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "raise_hand",
            "description": "Marks this session's ticket as waiting on a person: the card \
                            lights on the mesimon board and the board's attention count \
                            includes it until the ticket is opened or its next prompt \
                            arrives. For a turn that ends on a question, a decision, or a \
                            blocker a person has to settle; an ordinary finished turn \
                            already reads as done.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "One line, the words shown on the card.",
                    },
                },
                "required": ["reason"],
                "additionalProperties": false,
            },
        }),
        // The three that exist only for the crown (T-411). They take a key
        // and a `seen` stamp; the daemon refuses every call from a ticket
        // that does not wear the crown, and the refusal says how a person
        // grants one.
        json!({
            "name": "rename_ticket",
            "description": "Retitles another mesimon ticket (crown only). The seen stamp \
                            get_ticket returned for it is required, and a ticket that changed \
                            since it was read is refused with its current state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    "title": { "type": "string", "description": "The new title, one line." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp for this ticket." },
                },
                "required": ["key", "title", "seen"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "set_workspace",
            "description": "Chooses another mesimon ticket's workspace (crown only): a \
                            worktree of its own or the shared checkout. Refused once the \
                            ticket has an agent pane or a worktree, the same lock a person \
                            meets, so it is for a ticket nobody has started.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    // A plain string: `no_column_name_can_reach_a_schema`
                    // refuses every enum, and these two are words anyway.
                    "workspace": { "type": "string", "description": "worktree or shared_checkout." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp for this ticket." },
                },
                "required": ["key", "workspace", "seen"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "archive_ticket",
            "description": "Archives another mesimon ticket (crown only), or with restore \
                            brings an archived one back to its column. Refused while a \
                            session on it is awake; sleep_agent parks one the crown started. \
                            Agents cannot delete a ticket; this is the reversible form, and \
                            the person can restore it too.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    "restore": { "type": "boolean", "description": "Optional. True restores instead." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp for this ticket." },
                },
                "required": ["key", "seen"],
                "additionalProperties": false,
            },
        }),
        // The crown's one start (T-412). The daemon spawns; this only asks,
        // behind the board's spawn budget and the one-agent-per-ticket rule.
        json!({
            "name": "start_agent",
            "description": "Starts the board's agent on another mesimon ticket (crown only): \
                            the title and description are its first prompt. Refused, as an \
                            error, on a ticket that has an agent, on this session's own \
                            ticket, and past the crown's budget. The receipt's status is \
                            started, or waiting_for_worktree until its worktree is cut; \
                            budget_left is the starts left. The board then wakes this session \
                            when that agent delivers, merges, answers or raises its hand, so \
                            nothing is polled.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp." },
                    "plan": { "type": "boolean", "description": "Optional. True: plan mode." },
                },
                "required": ["key", "seen"],
                "additionalProperties": false,
            },
        }),
        // The crown's sleep (T-539): `x` on a card the crown started. Kill
        // stays in the never-tier; a park keeps the conversation and a
        // person's `c` undoes it. Scoped to `started_by` so a person's own
        // agent — the one they may be mid-conversation with — is never
        // parked under them by an agent.
        json!({
            "name": "sleep_agent",
            "description": "Parks another mesimon ticket's agent (crown only), as x on its \
                            card does: the conversation is kept and a person's c wakes it. \
                            Only an agent the crown started, and only once it is idle; a \
                            working agent, a person's agent and this session's own are \
                            refused in words. A parked agent holds no seat in the crown's \
                            budget until it is woken, so parking a finished worker frees its \
                            seat; archive_ticket afterwards reclaims a merged worktree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp." },
                },
                "required": ["key", "seen"],
                "additionalProperties": false,
            },
        }),
        // The crown's ask (T-413): words for another ticket's agent, held on
        // its card until a person sends them. The direct prompt stays in the
        // never-tier; this is its road through a person.
        json!({
            "name": "ask_agent",
            "description": "Queues words for another mesimon ticket's agent (crown only). \
                            They wait on that ticket's card, marked as this agent's, until \
                            a person sends them (^y) or takes them back (^u); nothing \
                            reaches the agent before that. One ask per ticket: a second \
                            replaces the first. Refused on a ticket with no agent and on this \
                            session's own ticket. The turn that takes them wakes this session \
                            when it ends.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The ticket's key, from list_board." },
                    "text": { "type": "string", "description": "The words, as a person types them." },
                    "seen": { "type": "string", "description": "get_ticket's seen stamp." },
                    "plan": { "type": "boolean", "description": "Optional. True: plan mode." },
                },
                "required": ["key", "text", "seen"],
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
    /// `key` names another ticket (T-411): the crown's road, refused by the
    /// daemon on every other ticket.
    GetTicket {
        key: Option<String>,
    },
    ListBoard,
    MoveTicket {
        to_column: String,
        idempotency_key: Option<String>,
        key: Option<String>,
        before: Option<String>,
        seen: Option<String>,
    },
    ReadAttachment {
        attachment: ulid::Ulid,
    },
    ReadNote {
        note: ulid::Ulid,
        key: Option<String>,
    },
    WriteNote {
        note: Option<ulid::Ulid>,
        text: String,
        key: Option<String>,
    },
    RenameTicket {
        key: String,
        title: String,
        seen: String,
    },
    SetWorkspace {
        key: String,
        workspace: String,
        seen: String,
    },
    ArchiveTicket {
        key: String,
        restore: bool,
        seen: String,
    },
    StartAgent {
        key: String,
        seen: String,
        plan: bool,
    },
    SleepAgent {
        key: String,
        seen: String,
    },
    AskAgent {
        key: String,
        text: String,
        seen: String,
        plan: bool,
    },
    CreateTicket {
        title: String,
        column: Option<String>,
        description: Option<String>,
        /// Tag NAMES, resolved against the registry by the daemon. Empty when
        /// the argument was absent.
        tags: Vec<String>,
        idempotency_key: Option<String>,
    },
    TagTicket {
        name: String,
        group: Option<u8>,
        remove: bool,
        key: Option<String>,
    },
    RaiseHand {
        reason: String,
    },
}

/// An optional string argument, trimmed: absent, null or blank is `None`;
/// any other shape is an error the model can read. The keyed tools (T-411)
/// take their `key`, `before` and `seen` through this.
/// An optional boolean argument: absent or null is `false`, anything but a
/// boolean is refused by name (the `restore` rule, shared since T-434).
fn flag(args: &Value, name: &str) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| format!("{name} must be a boolean, not {v}")),
    }
}

fn opt_word(args: &Value, name: &str) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string()).filter(|s| !s.is_empty())),
        Some(other) => Err(format!("{name} must be a string, not {other}")),
    }
}

/// A required string argument, trimmed and non-empty.
fn word(args: &Value, name: &str) -> Result<String, String> {
    opt_word(args, name)?.ok_or_else(|| format!("{name} is required"))
}

/// Parse a `tools/call` into a `ToolCall`, or produce the message the agent
/// sees. Unknown tool names are a plain error: the model sometimes holds a
/// `tool_reference` for a tool that no longer exists.
pub fn parse_tool_call(name: &str, args: &Value) -> Result<ToolCall, String> {
    match name {
        "get_ticket" => Ok(ToolCall::GetTicket { key: opt_word(args, "key")? }),
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
                key: opt_word(args, "key")?,
                before: opt_word(args, "before")?,
                seen: opt_word(args, "seen")?,
            })
        }
        "rename_ticket" => Ok(ToolCall::RenameTicket {
            key: word(args, "key")?,
            title: word(args, "title")?,
            seen: word(args, "seen")?,
        }),
        "set_workspace" => Ok(ToolCall::SetWorkspace {
            key: word(args, "key")?,
            workspace: word(args, "workspace")?,
            seen: word(args, "seen")?,
        }),
        "archive_ticket" => Ok(ToolCall::ArchiveTicket {
            key: word(args, "key")?,
            restore: match args.get("restore") {
                None | Some(Value::Null) => false,
                Some(v) => {
                    v.as_bool().ok_or_else(|| format!("restore must be a boolean, not {v}"))?
                }
            },
            seen: word(args, "seen")?,
        }),
        "start_agent" => Ok(ToolCall::StartAgent {
            key: word(args, "key")?,
            seen: word(args, "seen")?,
            plan: flag(args, "plan")?,
        }),
        "sleep_agent" => {
            Ok(ToolCall::SleepAgent { key: word(args, "key")?, seen: word(args, "seen")? })
        }
        "ask_agent" => Ok(ToolCall::AskAgent {
            key: word(args, "key")?,
            text: args
                .get("text")
                .and_then(Value::as_str)
                .ok_or("ask_agent requires a text string")?
                .to_string(),
            seen: word(args, "seen")?,
            plan: flag(args, "plan")?,
        }),
        "read_attachment" => Ok(ToolCall::ReadAttachment {
            attachment: args
                .get("attachment")
                .and_then(Value::as_str)
                .ok_or("read_attachment requires an attachment id")?
                .parse()
                .map_err(|_| "not an attachment id")?,
        }),
        "read_note" => Ok(ToolCall::ReadNote {
            note: note_id(args, true)?.ok_or("read_note requires a note id")?,
            key: opt_word(args, "key")?,
        }),
        "write_note" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or("write_note requires a text string")?
                .to_string();
            Ok(ToolCall::WriteNote {
                note: note_id(args, false)?,
                text,
                key: opt_word(args, "key")?,
            })
        }
        "create_ticket" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or("create_ticket requires a title string")?
                .trim()
                .to_string();
            if title.is_empty() {
                return Err("title is empty".into());
            }
            let word = |k: &str| {
                args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
            };
            // Absent is no tags; present must be an array of strings, so a
            // model that sends `"tags": "BUG"` reads why nothing was worn
            // instead of getting a bare card.
            let tags = match args.get("tags") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|v| v.as_str().ok_or("tags must be an array of strings"))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                Some(_) => return Err("tags must be an array of strings".into()),
            };
            Ok(ToolCall::CreateTicket {
                title,
                column: word("column").map(str::to_string),
                // Untrimmed: markdown's leading whitespace can mean something.
                description: args
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string),
                tags,
                idempotency_key: word("idempotency_key").map(str::to_string),
            })
        }
        "tag_ticket" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("tag_ticket requires a name string")?
                .trim()
                .to_string();
            if name.is_empty() {
                return Err("name is empty".into());
            }
            // An out-of-range group is an error the model can read, not a
            // silent `None` that would turn "the one on axis 3" into "any".
            let group = match args.get("group") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_u64()
                        .filter(|g| (1..=10).contains(g))
                        .map(|g| g as u8)
                        .ok_or_else(|| format!("group must be 1-10, not {v}"))?,
                ),
            };
            let remove = match args.get("remove") {
                None | Some(Value::Null) => false,
                Some(v) => {
                    v.as_bool().ok_or_else(|| format!("remove must be a boolean, not {v}"))?
                }
            };
            Ok(ToolCall::TagTicket { name, group, remove, key: opt_word(args, "key")? })
        }
        "raise_hand" => {
            // A blank reason is an error the model can read, never a silent
            // success: a mark with no words is the thing this tool exists to
            // improve on, and the daemon would refuse it a moment later.
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .ok_or("raise_hand requires a reason string")?
                .trim()
                .to_string();
            if reason.is_empty() {
                return Err("reason is empty".into());
            }
            Ok(ToolCall::RaiseHand { reason })
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// The `note` argument as an id. A malformed id is an error the model can
/// read, never a silent `None` that would turn "replace this note" into
/// "create another".
fn note_id(args: &Value, required: bool) -> Result<Option<ulid::Ulid>, String> {
    match args.get("note").and_then(Value::as_str).map(str::trim) {
        None | Some("") if !required => Ok(None),
        None | Some("") => Err("note id is empty".into()),
        Some(s) => s.parse::<ulid::Ulid>().map(Some).map_err(|_| format!("not a note id: {s}")),
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
        // The tier. Fifteen tools, sixteen commands (`get_ticket` with a
        // key is its own command on the wire).
        Command::AgentGetTicket
        | Command::AgentListBoard
        | Command::AgentMoveTicket { .. }
        // The crown's road (T-411): the keyed read and the three keyed
        // writers. Admitted HERE for every agent — the crown is not a tier,
        // it is a binding the daemon checks on every call against
        // `Board::crown`, and an uncrowned caller reads a refusal that says
        // how a person grants one. What stays out is deciding who wears it.
        | Command::AgentReadTicket { .. }
        | Command::AgentRenameTicket { .. }
        | Command::AgentSetWorkspace { .. }
        | Command::AgentArchiveTicket { .. }
        // The crown's start (T-412): an ASK to spawn, judged by the daemon
        // against the spawn budget and the seat rule. `SpawnSession` itself
        // stays below, in the never-tier — no agent names a kind, a prompt
        // or a session id.
        | Command::AgentStartTicket { .. }
        // The crown's sleep (T-539): `x` on an agent the crown started, the
        // one reversible stop. `KillSession` and `SleepSession` themselves
        // stay below — no agent names a session id, and a person's agent is
        // parked by nobody but the person.
        | Command::AgentSleepTicket { .. }
        // The crown's ask (T-413): words HELD on another ticket's card until
        // a person sends them. `PromptSession` itself stays below: the
        // person's send is the road, and there is no other.
        | Command::AgentAskTicket { .. }
        // T1 ANNOTATE: notes on the caller's OWN ticket. Unlike a tag, a
        // note is what D10 enumerated a tier for, and it is the one channel
        // through which the ticket's description reaches the agent without
        // a token entering its conversation.
        | Command::AgentReadAttachment { .. }
        | Command::AgentReadNote { .. }
        | Command::AgentWriteNote { .. }
        // Minting a ticket. It is a MUTATE on a column, not on the board
        // (`authorize` draws that line): the registry, the column list and
        // every existing card are untouched, one card is appended, and the
        // human sees it where every new ticket lands. Deleting, renaming or
        // archiving what was made stays out — nothing an agent files can be
        // unfiled by an agent.
        | Command::AgentCreateTicket { .. }
        // Wearing a tag the board already has. A MUTATE on the caller's own
        // ticket and on nothing else: the registry — the vocabulary, the axes,
        // the colours — is never written on this path, which is what keeps
        // `SetTag` below in the never-tier while this is in. See there.
        | Command::AgentTagTicket { .. }
        // Asking for a person on the caller's own ticket (T-107). A MUTATE on
        // that ticket and nothing else, and the only channel an agent has
        // into the loud register — one card, its own. What it CANNOT do is
        // take the mark off: a hand is lowered by the person it was raised
        // for (`LowerHand`, below), which is what keeps `!N` a number the
        // user can trust.
        | Command::AgentRaiseHand { .. } => true,

        // Everything below is the never-tier. An agent may not spawn or kill a
        // session, delete or archive or rename a ticket, change a workspace,
        // merge a branch, read a diff, take the focus token, or stop the
        // daemon — and there is no tool that would let it try.
        //
        // The six human tag commands (T-83) stay out, even now that
        // `AgentTagTicket` is in. Five of them mutate the REGISTRY, which is
        // board-wide state: `ForgetTag` strips a tag from every ticket
        // wearing it, `RenameTag` rewrites it everywhere, `MoveTag` changes
        // what an axis means, and all of that is precisely the "an agent
        // cannot change the board itself" rule `authorize` states. `SetTag`
        // carries a ticket id AND registers on the fly — using a name is what
        // creates it — so an agent holding it could put a word of its own in
        // the user's picker forever. The agent form above is the same wear/
        // unwear bound to the caller's ticket, with the registry read and
        // never written.
        Command::SetTag { .. }
        | Command::ForgetTag { .. }
        | Command::RegisterTag { .. }
        | Command::RenameTag { .. }
        | Command::SetTagColor { .. }
        | Command::MoveTag { .. }
        | Command::Hello { .. }
        | Command::Snapshot
        | Command::Subscribe
        | Command::CreateTicket { .. }
        | Command::ImportTicket { .. }
        // Board sharing is the person's: who to sign in as, what to share
        // with whom, who to let in and who to remove (T-215).
        | Command::Mesophon { .. }
        | Command::TeamSignIn { .. }
        | Command::TeamSignOut
        | Command::RedeemCode { .. }
        | Command::ShareBoard { .. }
        | Command::UnshareBoard
        | Command::MintInvite { .. }
        | Command::RevokeMember { .. }
        | Command::JoinBoard { .. }
        | Command::LeaveBoard
        | Command::TeamRefresh
        | Command::DuplicateTicket { .. }
        | Command::RenameTicket { .. }
        | Command::DeleteTicket { .. }
        | Command::SetWorkspace { .. }
        | Command::MergeTicket { .. }
        | Command::MergeToAgent { .. }
        // Typing into another agent's input box is the purest form of the
        // thing D10's never-tier exists to stop: one session steering
        // another's turn, with no human in between. The command carries a
        // ticket id, so an agent could not even address its own without
        // guessing one — and it must not address its own either.
        | Command::PromptSession { .. }
        // N input boxes at once (T-378): the same thing, a column wide.
        | Command::PromptColumn { .. }
        | Command::TakeQueuedAsk { .. }
        | Command::SendQueuedAsk { .. }
        | Command::DropQueuedAsk { .. }
        | Command::SetAutomation { .. }
        // The same delivery with mesimon's words: still one session's turn
        // being steered, still a human's gesture only.
        | Command::NoteToAgent { .. }
        // The local forms carry a ticket id; the agent forms above are the
        // same operations bound to the session's own ticket.
        | Command::ReadAttachment { .. }
        | Command::DiscardAttachmentUploads { .. }
        | Command::UploadAttachment { .. }
        | Command::SaveNoteWithAttachments { .. }
        | Command::CreateTicketWithNote { .. }
        | Command::ReadNote { .. }
        | Command::WriteNote { .. }
        | Command::RestoreTicket { .. }
        | Command::ArchiveTicket { .. }
        | Command::UnarchiveTicket { .. }
        // A snooze is an archive with a deadline, and a woke mark is the
        // user's to clear: both are the person's gestures.
        | Command::SnoozeTicket { .. }
        | Command::SeenTicket { .. }
        // The other half of `AgentRaiseHand`. An agent that could lower its
        // own hand could raise one on every turn and take it down before
        // anybody looked; more simply, being answered is not something the
        // asker gets to declare.
        | Command::LowerHand { .. }
        // Picked up at the desk is a person opening the page (T-497): an
        // agent's own start already says it started.
        | Command::OpenedTicket { .. }
        // Whether a branch lands on its own is the person's call, never the
        // agent's whose branch it is.
        | Command::SetManualMerge { .. }
        // Who wears the crown (T-411) is the person's call and nobody
        // else's: an agent that could crown itself, or a friend, would be
        // writing its own tier — the one thing this match exists to stop.
        | Command::CrownTicket { .. }
        | Command::Uncrown
        | Command::ArchiveAll
        // The environment every future pane gets, board-wide and shared by
        // every session. An agent asking to re-read the user's rc files would
        // be running the user's shell on its own say-so.
        | Command::ReloadShellEnv
        // The network and the repo's remote-tracking refs, on an agent's
        // say-so: never. The board shows an agent nothing of the fetch either.
        | Command::GitFetch
        // The tool surface itself. An agent that could switch its own tools
        // off — or back on for a board whose user took them away — would be
        // deciding its own tier, which is the one thing this match exists to
        // stop. It cannot see the flag either: no tool reports it.
        | Command::SetMcpTools { .. }
        | Command::SetAgentProvider { .. }
        // Agent tiers (T-443): which model an agent runs on is money and
        // capability, the person's to choose — an agent that could pick its
        // own tier, or define one, would be writing its own budget.
        | Command::SetTicketTier { .. }
        | Command::SaveTier { .. }
        | Command::DeleteTier { .. }
        | Command::SetDefaultTier { .. }
        | Command::SetParkAfterMinutes { .. }
        | Command::SetCrownBudget { .. }
        // Where the status line sits over the user's own panes: chrome, and
        // theirs. An agent moving it would be redecorating a screen it is
        // not looking at.
        | Command::SetStatusLine { .. }
        // Writes a file the user tracks in git, and stamps a board-wide
        // "never ask again". The dialog that shows the bytes is a person's;
        // this is not a road an agent gets a share of.
        | Command::SetSystemPrompt { .. }
        | Command::IgnoreBriefOffer
        // Where an agent's own `create_ticket` lands by default (T-279): an
        // agent choosing where its cards go would be choosing what the user
        // sees first. It names a column per call instead, in the open.
        | Command::SetFollowUpMode { .. }
        | Command::SetDefaultColumn { .. }
        // The sentences mesimon types into an agent's box (T-353). An agent
        // that could rewrite the rebase ask would be writing the prompt that
        // starts its own next turn — the one thing this tier exists to keep
        // in the user's hands.
        | Command::SetAgentPrompt { .. }
        // The column lifecycle (T-117): a tier that could add a column,
        // rename its own, or rewrite its own column's `agent_tools` would be
        // writing its own tier.
        | Command::AddColumn { .. }
        | Command::RenameColumn { .. }
        | Command::DeleteColumn { .. }
        | Command::ReorderColumn { .. }
        | Command::SetColumnSettings { .. }
        | Command::SortColumn { .. }
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
        | Command::FocusStart { .. }
        | Command::FocusEnd { .. }
        | Command::OpenTerminal { .. }
        | Command::TerminalEnd
        | Command::GateStatus
        | Command::GatePassed
        | Command::Shutdown
        | Command::DiffList { .. }
        | Command::DiffFile { .. }
        // A pane's contents are the session itself. `authorize` already
        // denies an agent `Resource::Session` at every action, and this is
        // the same rule stated where a new command has to walk past it.
        | Command::PaneTail { .. }
        | Command::TerminalTail { .. }
        // The terminal is the person's own shell; adopting it grows a
        // session, and no agent spawns anything.
        | Command::AdoptTerminal { .. }
        // Who is sitting at the user's terminal, and how recently they
        // touched it (T-299). A session read by the same rule as the line
        // above, and a fact about the PERSON besides — the notification
        // thread is the only caller it was built for.
        | Command::FocusQuiet => false,
    }
}

/// The tier each tool needs (T-117): a column's `agent_tools` says how far
/// up this ladder a claude on a ticket there may reach. `Read` is the three
/// that look, `Annotate` adds the three that write on the caller's own ticket
/// — a note, a tag, a raised hand — `Full` the two that touch the board: a
/// move, a new card. `None` is a
/// command no tier ever admits, which `agent_allows` refuses first anyway.
pub fn tier_needed_by(cmd: &Command) -> Option<AgentTools> {
    Some(match cmd {
        Command::AgentGetTicket
        | Command::AgentReadTicket { .. }
        | Command::AgentListBoard
        | Command::AgentReadNote { .. }
        | Command::AgentReadAttachment { .. } => AgentTools::Read,
        Command::AgentWriteNote { .. }
        | Command::AgentTagTicket { .. }
        | Command::AgentRaiseHand { .. } => AgentTools::Annotate,
        // The crown's three writers (T-411) sit on the top rung beside the
        // move: a column that narrowed its agents below `full` narrows the
        // crown with them.
        Command::AgentMoveTicket { .. }
        | Command::AgentCreateTicket { .. }
        | Command::AgentRenameTicket { .. }
        | Command::AgentSetWorkspace { .. }
        | Command::AgentArchiveTicket { .. }
        | Command::AgentStartTicket { .. }
        | Command::AgentSleepTicket { .. }
        | Command::AgentAskTicket { .. } => AgentTools::Full,
        _ => return None,
    })
}

/// The same table by tool NAME, for the shim's `tools/list`.
pub fn tier_needed_by_tool(name: &str) -> Option<AgentTools> {
    Some(match name {
        "get_ticket" | "list_board" | "read_note" | "read_attachment" => AgentTools::Read,
        "write_note" | "tag_ticket" | "raise_hand" => AgentTools::Annotate,
        "move_ticket" | "create_ticket" | "rename_ticket" | "set_workspace" | "archive_ticket"
        | "start_agent" | "sleep_agent" | "ask_agent" => AgentTools::Full,
        _ => return None,
    })
}

/// Whether `tier` admits `cmd`. `Off` admits nothing.
pub fn tier_admits(tier: AgentTools, cmd: &Command) -> bool {
    tier_needed_by(cmd).is_some_and(|need| tier >= need)
}

/// The read-tier tools `tier` admits, spelled as Claude Code's
/// `mcp__<server>__<tool>` permission names — the `--allowedTools` value a
/// spawn carries (T-362). Measured on Claude Code 2.1.270: default and auto
/// mode prompt for every MCP tool without an allow rule and ignore the MCP
/// `readOnlyHint`; plan mode ignores allow rules and admits exactly the tools
/// whose `annotations.readOnlyHint` is true, refusing the rest outright. So
/// the read rung carries both — this flag for default mode, the annotation
/// in `tools()` for plan mode — and `read_rung_is_hinted_read_only` keeps
/// the two lists the same one. Writers are never here: `write_note` or
/// `move_ticket` still prompt wherever the mode prompts. Empty at `Off`, so
/// the flag is omitted.
pub fn allowed_tool_names(tier: AgentTools) -> Vec<String> {
    tools_for(tier)
        .iter()
        .filter_map(|t| t["name"].as_str())
        .filter(|name| tier_needed_by_tool(name) == Some(AgentTools::Read))
        .map(|name| format!("mcp__{SERVER_NAME}__{name}"))
        .collect()
}

/// `tools()` narrowed to what `tier` admits — what the shim lists, so a
/// session in a `read` column is never shown a `move_ticket` it would be
/// refused. `Full` is `tools()` whole.
pub fn tools_for(tier: AgentTools) -> Vec<Value> {
    tools()
        .into_iter()
        .filter(|t| {
            t["name"].as_str().and_then(tier_needed_by_tool).is_some_and(|need| tier >= need)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tier table (T-117): every tool sits on exactly one rung, `Off`
    /// lists and admits nothing, and each rung admits by name exactly what
    /// it admits by command.
    #[test]
    fn the_tier_table_covers_every_tool_once() {
        let names = |tier| -> Vec<String> {
            tools_for(tier).iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
        };
        assert!(names(AgentTools::Off).is_empty());
        assert_eq!(
            names(AgentTools::Read),
            ["get_ticket", "list_board", "read_note", "read_attachment"]
        );
        assert_eq!(
            names(AgentTools::Annotate),
            [
                "get_ticket",
                "list_board",
                "read_note",
                "read_attachment",
                "write_note",
                "tag_ticket",
                "raise_hand"
            ]
        );
        assert_eq!(tools_for(AgentTools::Full).len(), tools().len(), "full is everything");
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            assert!(tier_needed_by_tool(name).is_some(), "{name} is on no rung");
        }
        assert_eq!(tier_needed_by_tool("nope"), None);
        // By name and by command agree, for every command an agent may send.
        let calls = [
            (Command::AgentGetTicket, "get_ticket"),
            // `get_ticket` with a key is its own command on the wire (T-411)
            // and sits on the same rung as the tool it rides.
            (Command::AgentReadTicket { key: "T-1".into() }, "get_ticket"),
            (Command::AgentListBoard, "list_board"),
            (Command::AgentReadNote { note: ulid::Ulid::nil(), key: None }, "read_note"),
            (Command::AgentReadAttachment { attachment: ulid::Ulid::nil() }, "read_attachment"),
            (Command::AgentWriteNote { note: None, text: "x".into(), key: None }, "write_note"),
            (
                Command::AgentTagTicket { name: "x".into(), group: None, remove: false, key: None },
                "tag_ticket",
            ),
            (Command::AgentRaiseHand { reason: "x".into() }, "raise_hand"),
            (
                Command::AgentMoveTicket {
                    to_column: "X".into(),
                    idempotency_key: None,
                    key: None,
                    before: None,
                    seen: None,
                },
                "move_ticket",
            ),
            (
                Command::AgentRenameTicket { key: "T-1".into(), title: "x".into(), seen: None },
                "rename_ticket",
            ),
            (
                Command::AgentSetWorkspace {
                    key: "T-1".into(),
                    workspace: "worktree".into(),
                    seen: None,
                },
                "set_workspace",
            ),
            (
                Command::AgentArchiveTicket { key: "T-1".into(), restore: false, seen: None },
                "archive_ticket",
            ),
            (
                Command::AgentStartTicket { key: "T-1".into(), seen: None, plan: false },
                "start_agent",
            ),
            (Command::AgentSleepTicket { key: "T-1".into(), seen: None }, "sleep_agent"),
            (
                Command::AgentAskTicket {
                    key: "T-1".into(),
                    text: "x".into(),
                    seen: None,
                    plan: false,
                },
                "ask_agent",
            ),
            (
                Command::AgentCreateTicket {
                    title: "x".into(),
                    column: None,
                    description: None,
                    tags: vec![],
                    idempotency_key: None,
                },
                "create_ticket",
            ),
        ];
        for (cmd, name) in &calls {
            assert_eq!(tier_needed_by(cmd), tier_needed_by_tool(name), "{name}");
            for tier in [AgentTools::Off, AgentTools::Read, AgentTools::Annotate, AgentTools::Full]
            {
                assert_eq!(
                    tier_admits(tier, cmd),
                    names(tier).iter().any(|n| n == name),
                    "{name} at {tier:?}"
                );
            }
        }
        assert!(!tier_admits(AgentTools::Full, &Command::Snapshot), "never-tier stays never");
    }

    /// The pre-approved set is the read rung and only the read rung, at every
    /// tier that lists it (T-362).
    #[test]
    fn allowed_tools_are_the_read_rung_only() {
        let read = [
            "mcp__mesimon__get_ticket",
            "mcp__mesimon__list_board",
            "mcp__mesimon__read_note",
            "mcp__mesimon__read_attachment",
        ];
        assert!(allowed_tool_names(AgentTools::Off).is_empty());
        for tier in [AgentTools::Read, AgentTools::Annotate, AgentTools::Full] {
            assert_eq!(allowed_tool_names(tier), read, "{tier:?}");
        }
    }

    /// Plan mode reads `annotations.readOnlyHint` and nothing else (T-362,
    /// measured on 2.1.270), so the read rung carries it and no writer does —
    /// the annotation and `allowed_tool_names` are one list, two spellings.
    #[test]
    fn read_rung_is_hinted_read_only() {
        for t in tools() {
            let name = t["name"].as_str().unwrap();
            let hinted = t["annotations"]["readOnlyHint"] == json!(true);
            let read = tier_needed_by_tool(name) == Some(AgentTools::Read);
            assert_eq!(hinted, read, "{name}: readOnlyHint {hinted}, read rung {read}");
            assert!(
                t["annotations"]
                    .as_object()
                    .is_none_or(|a| a.len() == 1 && a.contains_key("readOnlyHint")),
                "{name}: readOnlyHint is the only annotation; title is text the model reads"
            );
        }
    }

    #[test]
    fn exactly_fifteen_tools() {
        let t = tools();
        assert_eq!(t.len(), 15);
        let names: Vec<&str> = t.iter().filter_map(|v| v["name"].as_str()).collect();
        assert_eq!(
            names,
            [
                "get_ticket",
                "list_board",
                "move_ticket",
                "read_note",
                "read_attachment",
                "write_note",
                "create_ticket",
                "tag_ticket",
                "raise_hand",
                "rename_ticket",
                "set_workspace",
                "archive_ticket",
                "start_agent",
                "sleep_agent",
                "ask_agent"
            ]
        );
    }

    /// The crown's arguments (T-411): a key is a trimmed word, `seen` is
    /// required on the three keyed writers, and a wrongly shaped argument
    /// is an answer the model can read rather than a silent own-ticket call.
    #[test]
    fn keyed_tools_parse_and_refuse() {
        assert_eq!(
            parse_tool_call("get_ticket", &json!({ "key": " T-4 " })),
            Ok(ToolCall::GetTicket { key: Some("T-4".into()) })
        );
        assert_eq!(
            parse_tool_call("get_ticket", &json!({ "key": "" })),
            Ok(ToolCall::GetTicket { key: None })
        );
        assert!(parse_tool_call("get_ticket", &json!({ "key": 4 })).is_err());
        assert_eq!(
            parse_tool_call(
                "move_ticket",
                &json!({ "to_column": "TODO", "key": "T-4", "before": "T-2", "seen": "abc" })
            ),
            Ok(ToolCall::MoveTicket {
                to_column: "TODO".into(),
                idempotency_key: None,
                key: Some("T-4".into()),
                before: Some("T-2".into()),
                seen: Some("abc".into()),
            })
        );
        assert_eq!(
            parse_tool_call(
                "rename_ticket",
                &json!({ "key": "T-4", "title": " new ", "seen": "abc" })
            ),
            Ok(ToolCall::RenameTicket {
                key: "T-4".into(),
                title: "new".into(),
                seen: "abc".into()
            })
        );
        assert!(parse_tool_call("rename_ticket", &json!({ "key": "T-4", "title": "x" })).is_err());
        assert!(parse_tool_call("rename_ticket", &json!({ "title": "x", "seen": "a" })).is_err());
        assert_eq!(
            parse_tool_call(
                "set_workspace",
                &json!({ "key": "T-4", "workspace": "worktree", "seen": "abc" })
            ),
            Ok(ToolCall::SetWorkspace {
                key: "T-4".into(),
                workspace: "worktree".into(),
                seen: "abc".into()
            })
        );
        assert_eq!(
            parse_tool_call("archive_ticket", &json!({ "key": "T-4", "seen": "abc" })),
            Ok(ToolCall::ArchiveTicket { key: "T-4".into(), restore: false, seen: "abc".into() })
        );
        assert_eq!(
            parse_tool_call("start_agent", &json!({ "key": " T-4 ", "seen": "abc" })),
            Ok(ToolCall::StartAgent { key: "T-4".into(), seen: "abc".into(), plan: false })
        );
        // The plan flag (T-434): absent is false, a boolean is itself, a
        // non-boolean is refused by name.
        assert_eq!(
            parse_tool_call("start_agent", &json!({ "key": "T-4", "seen": "abc", "plan": true })),
            Ok(ToolCall::StartAgent { key: "T-4".into(), seen: "abc".into(), plan: true })
        );
        assert!(parse_tool_call(
            "start_agent",
            &json!({ "key": "T-4", "seen": "abc", "plan": "yes" })
        )
        .unwrap_err()
        .contains("plan must be a boolean"));
        assert!(parse_tool_call("start_agent", &json!({ "key": "T-4" })).is_err());
        assert!(parse_tool_call("start_agent", &json!({ "seen": "abc" })).is_err());
        assert_eq!(
            parse_tool_call("ask_agent", &json!({ "key": " T-4 ", "text": "go", "seen": "abc" })),
            Ok(ToolCall::AskAgent {
                key: "T-4".into(),
                text: "go".into(),
                seen: "abc".into(),
                plan: false
            })
        );
        assert_eq!(
            parse_tool_call(
                "ask_agent",
                &json!({ "key": "T-4", "text": "go", "seen": "abc", "plan": true })
            ),
            Ok(ToolCall::AskAgent {
                key: "T-4".into(),
                text: "go".into(),
                seen: "abc".into(),
                plan: true
            })
        );
        assert!(parse_tool_call("ask_agent", &json!({ "key": "T-4", "text": "go" })).is_err());
        assert_eq!(
            parse_tool_call("sleep_agent", &json!({ "key": " T-4 ", "seen": "abc" })),
            Ok(ToolCall::SleepAgent { key: "T-4".into(), seen: "abc".into() })
        );
        assert!(parse_tool_call("sleep_agent", &json!({ "key": "T-4" })).is_err());
        assert!(parse_tool_call("sleep_agent", &json!({ "seen": "abc" })).is_err());
        assert!(parse_tool_call("ask_agent", &json!({ "key": "T-4", "seen": "abc" })).is_err());
        assert!(parse_tool_call("ask_agent", &json!({ "text": "go", "seen": "abc" })).is_err());
        assert_eq!(
            parse_tool_call(
                "archive_ticket",
                &json!({ "key": "T-4", "restore": true, "seen": "abc" })
            ),
            Ok(ToolCall::ArchiveTicket { key: "T-4".into(), restore: true, seen: "abc".into() })
        );
        assert!(parse_tool_call(
            "archive_ticket",
            &json!({ "key": "T-4", "restore": "yes", "seen": "a" })
        )
        .is_err());
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": "bug", "key": "T-4" })),
            Ok(ToolCall::TagTicket {
                name: "bug".into(),
                group: None,
                remove: false,
                key: Some("T-4".into())
            })
        );
    }

    /// The words are the whole point (T-107), so a call without them is an
    /// error the model can read rather than a mark with nothing under it.
    #[test]
    fn raise_hand_parses_and_refuses() {
        assert_eq!(
            parse_tool_call("raise_hand", &json!({ "reason": "  which auth provider?  " })),
            Ok(ToolCall::RaiseHand { reason: "which auth provider?".into() })
        );
        assert!(parse_tool_call("raise_hand", &json!({})).is_err(), "reason is required");
        assert!(parse_tool_call("raise_hand", &json!({ "reason": "   " })).is_err());
        assert!(parse_tool_call("raise_hand", &json!({ "reason": 7 })).is_err());
    }

    #[test]
    fn tag_ticket_parses_and_refuses() {
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": " bug " })),
            Ok(ToolCall::TagTicket { name: "bug".into(), group: None, remove: false, key: None })
        );
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": 3, "remove": true })),
            Ok(ToolCall::TagTicket { name: "bug".into(), group: Some(3), remove: true, key: None })
        );
        // Null optionals are absent, not errors.
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": null, "remove": null })),
            Ok(ToolCall::TagTicket { name: "bug".into(), group: None, remove: false, key: None })
        );
        assert!(parse_tool_call("tag_ticket", &json!({})).is_err());
        assert!(parse_tool_call("tag_ticket", &json!({ "name": "  " })).is_err());
        // A group outside the axes, or a non-boolean remove, is a legible
        // refusal — never silently widened to "any group" or "put on".
        assert!(parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": 0 })).is_err());
        assert!(parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": 11 })).is_err());
        assert!(parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": "3" })).is_err());
        assert!(parse_tool_call("tag_ticket", &json!({ "name": "bug", "remove": "yes" })).is_err());
    }

    #[test]
    fn create_ticket_parses_and_refuses() {
        assert_eq!(
            parse_tool_call("create_ticket", &json!({ "title": " fix the thing " })),
            Ok(ToolCall::CreateTicket {
                title: "fix the thing".into(),
                column: None,
                description: None,
                tags: vec![],
                idempotency_key: None,
            })
        );
        assert_eq!(
            parse_tool_call(
                "create_ticket",
                &json!({ "title": "t", "column": " REVIEW ", "description": "  # why\n\nbecause",
                         "tags": [" BUG ", "", "P1"], "idempotency_key": "k" })
            ),
            Ok(ToolCall::CreateTicket {
                title: "t".into(),
                column: Some("REVIEW".into()),
                description: Some("  # why\n\nbecause".into()),
                tags: vec!["BUG".into(), "P1".into()],
                idempotency_key: Some("k".into()),
            })
        );
        // Blank optionals are absent, not empty strings the daemon must judge.
        assert_eq!(
            parse_tool_call(
                "create_ticket",
                &json!({ "title": "t", "column": "", "description": " \n ", "tags": [] })
            ),
            Ok(ToolCall::CreateTicket {
                title: "t".into(),
                column: None,
                description: None,
                tags: vec![],
                idempotency_key: None,
            })
        );
        assert!(parse_tool_call("create_ticket", &json!({})).is_err());
        assert!(parse_tool_call("create_ticket", &json!({ "title": "   " })).is_err());
        // A wrongly shaped `tags` is an answer, never a silently bare card.
        assert!(parse_tool_call("create_ticket", &json!({ "title": "t", "tags": "BUG" })).is_err());
        assert!(parse_tool_call("create_ticket", &json!({ "title": "t", "tags": [1] })).is_err());
    }

    #[test]
    fn note_tools_parse_and_refuse() {
        let id = ulid::Ulid::nil();
        assert_eq!(
            parse_tool_call("read_note", &json!({ "note": id.to_string() })),
            Ok(ToolCall::ReadNote { note: id, key: None })
        );
        assert!(parse_tool_call("read_note", &json!({})).is_err());
        assert!(parse_tool_call("read_note", &json!({ "note": "nope" })).is_err());
        assert_eq!(
            parse_tool_call("write_note", &json!({ "text": "hi" })),
            Ok(ToolCall::WriteNote { note: None, text: "hi".into(), key: None })
        );
        assert_eq!(
            parse_tool_call("write_note", &json!({ "note": id.to_string(), "text": "" })),
            Ok(ToolCall::WriteNote { note: Some(id), text: String::new(), key: None })
        );
        // A malformed id must not silently become "create another".
        assert!(parse_tool_call("write_note", &json!({ "note": "x", "text": "hi" })).is_err());
        assert!(parse_tool_call("write_note", &json!({})).is_err());
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

    #[test]
    fn ticket_creation_guidance_is_bounded_and_descriptive() {
        let registry = tools();
        for (name, concepts) in [
            (
                "create_ticket",
                vec![
                    "for a session of its own, not this one",
                    "work unit to pick up, not an idea/list row",
                    "findings on one surface share a ticket with a list",
                    "list_board checks scope/duplicates first",
                    "Research belongs in this ticket's notes; the user chooses tickets",
                    "Agents cannot delete tickets; cleanup costs the user",
                ],
            ),
            (
                "list_board",
                vec![
                    // T-467: where a new ticket's column is chosen from.
                    "what each column is for in the user's words (column_descriptions)",
                    "Title comparison here is the pre-check for create_ticket",
                    "todo, in progress or review belongs on that ticket as scope, not as a sibling",
                    "A near-duplicate title means the older ticket wins",
                ],
            ),
            (
                "get_ticket",
                vec!["the column names move_ticket accepts and what each column is for"],
            ),
            // T-537: a crown that is not told the board wakes it goes
            // looking on its own, with a monitor that holds the wake.
            ("start_agent", vec!["The board then wakes this session", "so nothing is polled"]),
            ("ask_agent", vec!["The turn that takes them wakes this session"]),
        ] {
            let tool = registry.iter().find(|t| t["name"] == name).unwrap();
            let description = tool["description"].as_str().unwrap();
            for concept in concepts {
                assert!(description.contains(concept), "{name} lost guidance: {concept}");
            }
            lint_tool_text(description).unwrap();
            let bytes = serde_json::to_vec(tool).unwrap().len();
            assert!(bytes <= MAX_TOOL_BYTES, "{name} is {bytes} bytes, cap is {MAX_TOOL_BYTES}");
        }
    }

    /// T-459: an agent that does not know the board saved its approved plan
    /// saves it again by hand. The tool it would do that with is where it
    /// learns the note already exists.
    #[test]
    fn write_note_says_the_plan_is_already_a_note() {
        let registry = tools();
        let tool = registry.iter().find(|t| t["name"] == "write_note").unwrap();
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("A plan from plan mode is already a note, saved by the board"),
            "{description}"
        );
        assert!(description.contains("revised on each re-plan"), "{description}");
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
            ToolCall::MoveTicket {
                to_column: "REVIEW".into(),
                idempotency_key: None,
                key: None,
                before: None,
                seen: None
            }
        );
        assert_eq!(
            parse_tool_call(
                "move_ticket",
                &json!({"to_column": " REVIEW ", "idempotency_key": "k"})
            )
            .unwrap(),
            ToolCall::MoveTicket {
                to_column: "REVIEW".into(),
                idempotency_key: Some("k".into()),
                key: None,
                before: None,
                seen: None
            }
        );
        assert!(parse_tool_call("move_ticket", &json!({})).is_err());
        assert!(parse_tool_call("move_ticket", &json!({"to_column": "  "})).is_err());
        assert!(parse_tool_call("get_session", &json!({})).is_err());
    }

    #[test]
    fn no_argument_tools_ignore_arguments() {
        // `ticket` was never the argument's name: a session cannot address
        // another ticket by guessing a field. `key` is (T-411), and the
        // daemon judges it against the crown.
        assert_eq!(
            parse_tool_call("get_ticket", &json!({"ticket": "OTHER-9"})).unwrap(),
            ToolCall::GetTicket { key: None }
        );
        assert_eq!(parse_tool_call("list_board", &json!(null)).unwrap(), ToolCall::ListBoard);
    }

    /// Every tool has a command, and every allowed command has a tool. A
    /// command an agent may send that no tool can reach would be a hole nobody
    /// is looking at. Sixteen commands for fifteen tools: `get_ticket` with
    /// a key rides its own command (T-411).
    #[test]
    fn the_tier_is_exactly_sixteen_commands() {
        let allowed = [
            Command::AgentGetTicket,
            Command::AgentReadTicket { key: "T-1".into() },
            Command::AgentListBoard,
            Command::AgentMoveTicket {
                to_column: "X".into(),
                idempotency_key: None,
                key: None,
                before: None,
                seen: None,
            },
            Command::AgentReadNote { note: ulid::Ulid::nil(), key: None },
            Command::AgentReadAttachment { attachment: ulid::Ulid::nil() },
            Command::AgentWriteNote { note: None, text: "x".into(), key: None },
            Command::AgentCreateTicket {
                title: "x".into(),
                column: None,
                description: None,
                tags: vec![],
                idempotency_key: None,
            },
            Command::AgentTagTicket { name: "x".into(), group: None, remove: false, key: None },
            Command::AgentRaiseHand { reason: "x".into() },
            Command::AgentRenameTicket { key: "T-1".into(), title: "x".into(), seen: None },
            Command::AgentSetWorkspace {
                key: "T-1".into(),
                workspace: "worktree".into(),
                seen: None,
            },
            Command::AgentArchiveTicket { key: "T-1".into(), restore: false, seen: None },
            Command::AgentStartTicket { key: "T-1".into(), seen: None, plan: false },
            Command::AgentSleepTicket { key: "T-1".into(), seen: None },
            Command::AgentAskTicket {
                key: "T-1".into(),
                text: "x".into(),
                seen: None,
                plan: false,
            },
        ];
        for c in &allowed {
            assert!(agent_allows(c), "{c:?} should be in the tier");
        }
        assert_eq!(allowed.len(), tools().len() + 1);
    }

    /// The never-tier, named one command at a time. This is the list a reader
    /// checks when they ask "can an agent do X".
    #[test]
    fn the_never_tier_holds() {
        let t = ulid::Ulid::nil();
        let s = uuid::Uuid::nil();
        let denied = vec![
            Command::UploadAttachment {
                upload: None,
                offset: 0,
                data: "AA==".into(),
                complete: false,
            },
            Command::DiscardAttachmentUploads { uploads: vec![t] },
            Command::ReadAttachment { ticket: t, attachment: t },
            Command::SaveNoteWithAttachments {
                ticket: t,
                note: None,
                text: "x".into(),
                uploads: vec![],
            },
            Command::CreateTicketWithNote {
                column: "TODO".into(),
                title: "x".into(),
                workspace: None,
                text: "x".into(),
                uploads: vec![],
                tags: vec![],
                tier: None,
            },
            Command::SetAgentProvider { provider: crate::board::AgentProvider::Codex },
            Command::SetParkAfterMinutes { minutes: 30 },
            Command::SetCrownBudget { budget: 3 },
            Command::SetTicketTier { id: t, tier: Some("claude".into()) },
            Command::SaveTier {
                scope: crate::tier::TierScope::Machine,
                tier: crate::tier::Tier::builtin(crate::board::AgentProvider::Codex),
            },
            Command::DeleteTier { scope: crate::tier::TierScope::Board, id: "x".into() },
            Command::SetDefaultTier { scope: crate::tier::TierScope::Board, id: None },
            Command::Hello { version: 1, client: "x".into() },
            Command::Snapshot,
            Command::Subscribe,
            Command::CreateTicket {
                column: "TODO".into(),
                title: "t".into(),
                workspace: None,
                tier: None,
            },
            Command::ImportTicket {
                column: "TODO".into(),
                content: crate::content::TicketContent { title: "incoming".into(), notes: vec![] },
                origin: crate::content::ImportOrigin { source: t, item: t },
            },
            Command::RenameTicket { id: t, title: "t".into() },
            Command::DeleteTicket { id: t, discard_worktree: true },
            Command::TeamSignIn {
                relay: "relay.example".into(),
                display_name: "Dana".into(),
                code: None,
            },
            Command::TeamSignOut,
            Command::RedeemCode { code: "x".into() },
            Command::Mesophon { action: crate::mesophon::LocalAction::Status },
            Command::Mesophon { action: crate::mesophon::LocalAction::Enable },
            Command::ShareBoard { notes: true },
            Command::UnshareBoard,
            Command::MintInvite { role: "viewer".into() },
            Command::RevokeMember { device: "00".repeat(16) },
            Command::JoinBoard { code: "x".into() },
            Command::LeaveBoard,
            Command::TeamRefresh,
            Command::SetWorkspace { id: t, workspace: None },
            Command::SetTag { id: t, group: 1, name: Some("urgent".into()) },
            Command::ForgetTag { group: 1, name: "urgent".into() },
            Command::RegisterTag { group: 1, name: "urgent".into() },
            Command::RenameTag { group: 1, from: "a".into(), to: "b".into() },
            Command::SetTagColor { group: 1, name: "urgent".into(), color: 2 },
            Command::MoveTag { group: 1, name: "urgent".into(), to_group: 2, to_index: 0 },
            Command::MergeTicket { id: t },
            Command::RestoreTicket { id: t },
            Command::ArchiveTicket { id: t },
            Command::UnarchiveTicket { id: t },
            Command::SnoozeTicket { id: t, until: 1, needs_you: true },
            Command::SeenTicket { id: t },
            Command::OpenedTicket { id: t },
            Command::SetManualMerge { id: t, on: true },
            // The crown is the person's to give (T-411).
            Command::CrownTicket { id: t },
            Command::Uncrown,
            Command::ArchiveAll,
            Command::AddColumn { name: "QA".into(), after: None },
            Command::RenameColumn { name: "QA".into(), to: "QC".into() },
            Command::DeleteColumn { name: "QA".into() },
            Command::ReorderColumn { name: "QA".into(), before: None },
            Command::SetColumnSettings { name: "QA".into(), settings: Default::default() },
            Command::SortColumn { column: "QA".into(), by: crate::board::SortBy::Key },
            Command::SetDefaultColumn { column: Some("QA".into()) },
            Command::MoveTicket { id: t, column: "DONE".into(), before: None },
            Command::SpawnSession {
                ticket: t,
                kind: crate::board::SessionKind::Claude,
                submit_prompt: true,
                plan: false,
            },
            Command::KillSession { id: s },
            Command::RescanExternal,
            Command::AttachExternal { claude_session_id: s, ticket: None },
            Command::ResumeExternal { claude_session_id: s, ticket: None, confirm: true },
            Command::ResumeSession { id: s, confirm: true },
            Command::SleepSession { id: s },
            Command::WakeSession { id: s },
            Command::ReclaimAll,
            Command::FocusStart { session: s },
            Command::FocusEnd { session: s },
            Command::OpenTerminal { ticket: Some(t) },
            Command::TerminalEnd,
            Command::GateStatus,
            Command::GatePassed,
            Command::Shutdown,
            Command::DiffList { target: crate::command::DiffTarget::Ticket { id: t } },
            Command::DiffFile {
                target: crate::command::DiffTarget::Checkout,
                path: "a".into(),
                context: 3,
            },
            Command::PaneTail { session: s, lines: 20 },
            Command::TerminalTail { ticket: Some(t), lines: 20 },
            Command::AdoptTerminal { ticket: t },
            // One agent steering another agent's turn is the sharpest thing
            // the never-tier exists to stop.
            Command::PromptSession {
                ticket: t,
                text: "do the thing".into(),
                queued: false,
                accept_plan: false,
                plan: false,
                tier: None,
            },
            Command::PromptColumn {
                column: "TODO".into(),
                text: "do the thing".into(),
                queued: true,
                accept_plan: false,
            },
            Command::DropQueuedAsk { ticket: t },
            Command::TakeQueuedAsk { ticket: t },
            Command::SendQueuedAsk { ticket: t },
            Command::SetFollowUpMode { mode: crate::board::FollowUpMode::Steer },
            Command::SetAutomation { merge_train: true, merge_notice: true },
            Command::NoteToAgent { ticket: t, note: t },
            // The ticket-addressed forms; the agent forms are the tier.
            Command::ReadNote { ticket: t, note: t },
            Command::WriteNote { ticket: t, note: None, text: "x".into() },
        ];
        // The tier's other-ticket writers (T-411) are admitted here and
        // judged by the crown in the daemon; the id-addressed forms above
        // stay out, key or no key.
        assert!(agent_allows(&Command::AgentRenameTicket {
            key: "T-1".into(),
            title: "x".into(),
            seen: None
        }));
        for c in &denied {
            assert!(!agent_allows(c), "{c:?} must stay out of the tier");
        }
    }
}
