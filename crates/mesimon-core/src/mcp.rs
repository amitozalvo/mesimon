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
            "description": "Returns the mesimon ticket this session is attached to: key, \
                            title, current column, workspace mode, branch, merge state, \
                            the column names move_ticket accepts, the tags it wears, \
                            every tag the board knows (allowed_tags), the description (its \
                            first note) and the id, name and author of every note. The \
                            prompt that starts a session is often the ticket's title \
                            alone; the description and notes here are the rest of the \
                            brief, so this is the first call of a session.",
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
        json!({
            "name": "read_note",
            "description": "Returns the full markdown text of one note on this session's \
                            ticket. Note ids are listed by get_ticket.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "A note id from get_ticket." },
                },
                "required": ["note"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "write_note",
            "description": "Creates a markdown note on this session's ticket, or replaces \
                            the whole text of an existing one. The first note is the \
                            ticket's description. Empty text deletes an existing note.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "note": {
                        "type": "string",
                        "description": "Optional. The id of the note to replace; omitted \
                                        creates a new note.",
                    },
                    "text": { "type": "string", "description": "The note's whole markdown text." },
                },
                "required": ["text"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "create_ticket",
            "description": "Creates a ticket on the mesimon board and returns its key. It \
                            has no session; this session stays on its own. For work found \
                            outside this ticket's scope.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "One line, the card's text." },
                    // A plain string, NOT an enum: see the module header.
                    "column": {
                        "type": "string",
                        "description": "Optional. A column name from list_board; omitted \
                                        means the board's default column.",
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional markdown, the first note.",
                    },
                    // Names, NOT the registry: see the module header on enums.
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional. Names from allowed_tags (get_ticket), one \
                                        per group.",
                    },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Optional. Repeating a call with the same key replays \
                                        the first result.",
                    },
                },
                "required": ["title"],
                "additionalProperties": false,
            },
        }),
        json!({
            "name": "tag_ticket",
            "description": "Puts one of the board's existing tags on this session's ticket, \
                            or takes one off. Tags come in groups and a ticket wears at most \
                            one per group, so a tag replaces its groupmate. The names \
                            accepted are listed by get_ticket as allowed_tags; new tags \
                            are created on the board, not here.",
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
    MoveTicket {
        to_column: String,
        idempotency_key: Option<String>,
    },
    ReadNote {
        note: ulid::Ulid,
    },
    WriteNote {
        note: Option<ulid::Ulid>,
        text: String,
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
    },
    RaiseHand {
        reason: String,
    },
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
        "read_note" => Ok(ToolCall::ReadNote {
            note: note_id(args, true)?.ok_or("read_note requires a note id")?,
        }),
        "write_note" => {
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or("write_note requires a text string")?
                .to_string();
            Ok(ToolCall::WriteNote { note: note_id(args, false)?, text })
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
            Ok(ToolCall::TagTicket { name, group, remove })
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
        // The tier. Eight tools, eight commands.
        Command::AgentGetTicket
        | Command::AgentListBoard
        | Command::AgentMoveTicket { .. }
        // T1 ANNOTATE: notes on the caller's OWN ticket. Unlike a tag, a
        // note is what D10 enumerated a tier for, and it is the one channel
        // through which the ticket's description reaches the agent without
        // a token entering its conversation.
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
        | Command::DropQueuedAsk { .. }
        | Command::SetAutomation { .. }
        // The same delivery with mesimon's words: still one session's turn
        // being steered, still a human's gesture only.
        | Command::NoteToAgent { .. }
        // The local forms carry a ticket id; the agent forms above are the
        // same operations bound to the session's own ticket.
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
        // Whether a branch lands on its own is the person's call, never the
        // agent's whose branch it is.
        | Command::SetManualMerge { .. }
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
        | Command::SetDefaultColumn { .. }
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
        Command::AgentGetTicket | Command::AgentListBoard | Command::AgentReadNote { .. } => {
            AgentTools::Read
        }
        Command::AgentWriteNote { .. }
        | Command::AgentTagTicket { .. }
        | Command::AgentRaiseHand { .. } => AgentTools::Annotate,
        Command::AgentMoveTicket { .. } | Command::AgentCreateTicket { .. } => AgentTools::Full,
        _ => return None,
    })
}

/// The same table by tool NAME, for the shim's `tools/list`.
pub fn tier_needed_by_tool(name: &str) -> Option<AgentTools> {
    Some(match name {
        "get_ticket" | "list_board" | "read_note" => AgentTools::Read,
        "write_note" | "tag_ticket" | "raise_hand" => AgentTools::Annotate,
        "move_ticket" | "create_ticket" => AgentTools::Full,
        _ => return None,
    })
}

/// Whether `tier` admits `cmd`. `Off` admits nothing.
pub fn tier_admits(tier: AgentTools, cmd: &Command) -> bool {
    tier_needed_by(cmd).is_some_and(|need| tier >= need)
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
        assert_eq!(names(AgentTools::Read), ["get_ticket", "list_board", "read_note"]);
        assert_eq!(
            names(AgentTools::Annotate),
            ["get_ticket", "list_board", "read_note", "write_note", "tag_ticket", "raise_hand"]
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
            (Command::AgentListBoard, "list_board"),
            (Command::AgentReadNote { note: ulid::Ulid::nil() }, "read_note"),
            (Command::AgentWriteNote { note: None, text: "x".into() }, "write_note"),
            (
                Command::AgentTagTicket { name: "x".into(), group: None, remove: false },
                "tag_ticket",
            ),
            (Command::AgentRaiseHand { reason: "x".into() }, "raise_hand"),
            (
                Command::AgentMoveTicket { to_column: "X".into(), idempotency_key: None },
                "move_ticket",
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

    #[test]
    fn exactly_eight_tools() {
        let t = tools();
        assert_eq!(t.len(), 8);
        let names: Vec<&str> = t.iter().filter_map(|v| v["name"].as_str()).collect();
        assert_eq!(
            names,
            [
                "get_ticket",
                "list_board",
                "move_ticket",
                "read_note",
                "write_note",
                "create_ticket",
                "tag_ticket",
                "raise_hand"
            ]
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
            Ok(ToolCall::TagTicket { name: "bug".into(), group: None, remove: false })
        );
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": 3, "remove": true })),
            Ok(ToolCall::TagTicket { name: "bug".into(), group: Some(3), remove: true })
        );
        // Null optionals are absent, not errors.
        assert_eq!(
            parse_tool_call("tag_ticket", &json!({ "name": "bug", "group": null, "remove": null })),
            Ok(ToolCall::TagTicket { name: "bug".into(), group: None, remove: false })
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
            Ok(ToolCall::ReadNote { note: id })
        );
        assert!(parse_tool_call("read_note", &json!({})).is_err());
        assert!(parse_tool_call("read_note", &json!({ "note": "nope" })).is_err());
        assert_eq!(
            parse_tool_call("write_note", &json!({ "text": "hi" })),
            Ok(ToolCall::WriteNote { note: None, text: "hi".into() })
        );
        assert_eq!(
            parse_tool_call("write_note", &json!({ "note": id.to_string(), "text": "" })),
            Ok(ToolCall::WriteNote { note: Some(id), text: String::new() })
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
    fn the_tier_is_exactly_eight_commands() {
        let allowed = [
            Command::AgentGetTicket,
            Command::AgentListBoard,
            Command::AgentMoveTicket { to_column: "X".into(), idempotency_key: None },
            Command::AgentReadNote { note: ulid::Ulid::nil() },
            Command::AgentWriteNote { note: None, text: "x".into() },
            Command::AgentCreateTicket {
                title: "x".into(),
                column: None,
                description: None,
                tags: vec![],
                idempotency_key: None,
            },
            Command::AgentTagTicket { name: "x".into(), group: None, remove: false },
            Command::AgentRaiseHand { reason: "x".into() },
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
            Command::SetAgentProvider { provider: crate::board::AgentProvider::Codex },
            Command::Hello { version: 1, client: "x".into() },
            Command::Snapshot,
            Command::Subscribe,
            Command::CreateTicket { column: "TODO".into(), title: "t".into(), workspace: None },
            Command::RenameTicket { id: t, title: "t".into() },
            Command::DeleteTicket { id: t, discard_worktree: true },
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
            Command::SetManualMerge { id: t, on: true },
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
            // One agent steering another agent's turn is the sharpest thing
            // the never-tier exists to stop.
            Command::PromptSession { ticket: t, text: "do the thing".into(), queued: false },
            Command::DropQueuedAsk { ticket: t },
            Command::SetAutomation { merge_train: true, merge_notice: true },
            Command::NoteToAgent { ticket: t, note: t },
            // The ticket-addressed forms; the agent forms are the tier.
            Command::ReadNote { ticket: t, note: t },
            Command::WriteNote { ticket: t, note: None, text: "x".into() },
        ];
        for c in &denied {
            assert!(!agent_allows(c), "{c:?} must stay out of the tier");
        }
    }
}
