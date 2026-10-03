//! `mesimon mcp` — the stdio MCP server a mesimon-spawned Claude session talks
//! to (T-84).
//!
//! **This process is untrusted and holds no policy.** Claude spawns it, so it
//! runs inside the agent's own process tree; anything it decided could be
//! decided differently by an agent that simply ran the binary itself. Every
//! check — the command allowlist, the session binding, the tier, the move
//! guards — lives in the daemon, on the far side of `orch.sock`. What lives
//! here is translation: MCP in, one `Envelope` out, one `Response` back.
//!
//! Identity is the session uuid in argv, and there is no token. That is not an
//! oversight: `orch.sock` already accepts `Principal::Local` with the full
//! command set from any process running as this user, and a secret placed in
//! the tmux session environment is readable from every other pane on the
//! private server. The boundary mesimon actually has — and the one the hook
//! socket already committed to — is the 0700 runtime directory. What the
//! injected config *does* guarantee is the thing that was asked for: a session
//! mesimon did not spawn never receives these tools, because the config is
//! passed on argv and installed nowhere.
//!
//! On the mod road (T-577) no MCP server runs: the mod registers the same
//! tools with `$.tool.register` and serves each call from a `tool.call` hook
//! by running this binary once per call, `mesimon mcp --call <tool>` with the
//! arguments on stdin and the result on stdout, and lists them at the
//! session's start with `mesimon mcp --list`. The same translation, the same
//! daemon checks; only the transport is a process instead of a stdio server.
//!
//! One connection per call, deliberately. Tool calls happen at human scale, and
//! a fresh connect removes every stale-socket path on a daemon that gets
//! restarted many times a day during its own development.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use mesimon_core::board::AgentTools;
use mesimon_core::command::{AskRoad, Command, Envelope, Response};
use mesimon_core::mcp::{self, ToolCall};
use mesimon_core::Principal;
use serde_json::{json, Value};

/// Long enough that a writer thread busy with a provisioning burst still
/// answers, short enough that a wedged daemon does not hold the agent's turn.
const READ_TIMEOUT_SECS: u64 = 20;

/// How long `answer_agent`'s receipt is waited for (T-569, T-571): it comes
/// once the delivery settles, after the key walk (8 s for one question, up
/// to the daemon's 60 s `DIALOG_WALK_MAX` for a batch) and the 5 s hook
/// window, so this is that and a margin. `accept_plan`'s (T-582) waits
/// as long: at most the daemon's 30 s for a busy checkout and its 8 s for
/// the hook edge.
const ANSWER_WAIT_SECS: u64 = 75;

pub fn run(args: &[String]) -> ! {
    // The column's tier at spawn (T-117): what this process LISTS. Absent —
    // an argv persisted before the flag existed — lists everything; the
    // daemon decides at every call, so listing more never grants more.
    let tier = val(args, "--tools").and_then(AgentTools::parse).unwrap_or(AgentTools::Full);
    // The mod's registration (T-577): what `$.tool.register` takes, no
    // daemon asked.
    if args.iter().any(|a| a == "--list") {
        let list = serde_json::to_string(&mcp::registered_for(tier)).unwrap_or_default();
        println!("{list}");
        std::process::exit(0);
    }
    let Some(sock) = val(args, "--sock").map(PathBuf::from) else {
        eprintln!("mesimon mcp: --sock is required");
        std::process::exit(2);
    };
    let Some(session) = val(args, "--session").and_then(|s| s.parse::<uuid::Uuid>().ok()) else {
        eprintln!("mesimon mcp: --session must be a uuid");
        std::process::exit(2);
    };
    // One call for the mod (T-577): the model's arguments on stdin, the
    // result as `tools/call` answers it on stdout.
    if let Some(name) = val(args, "--call") {
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body);
        let arguments: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let mut params = json!({ "name": name, "arguments": arguments });
        if let Some(id) = val(args, "--tool-use-id") {
            params["_meta"] = json!({ "claudecode/toolUseId": id });
        }
        let result = tool_result(&params, &sock, session);
        let _ = writeln!(std::io::stdout(), "{result}");
        std::process::exit(0);
    }

    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle_line(&line, &sock, session, tier) else { continue };
        let Ok(s) = serde_json::to_string(&reply) else { continue };
        // stdout is the protocol. Nothing else may ever be written here —
        // diagnostics go to stderr, which Claude Code logs.
        if writeln!(out, "{s}").is_err() || out.flush().is_err() {
            break;
        }
    }
    std::process::exit(0)
}

/// One JSON-RPC message in, at most one out. `None` = a notification, which
/// takes no reply (answering one is a protocol error, not a courtesy).
fn handle_line(line: &str, sock: &PathBuf, session: uuid::Uuid, tier: AgentTools) -> Option<Value> {
    let msg: Value = serde_json::from_str(line).ok()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    // No id means a notification — `notifications/initialized` arrives on
    // every startup, and answering one is a protocol error, not a courtesy.
    let id = msg.get("id").cloned()?;

    match method {
        "initialize" => {
            let client_protocol = params.get("protocolVersion").and_then(Value::as_str);
            Some(ok(id, mcp::initialize_result(client_protocol)))
        }
        "ping" => Some(ok(id, json!({}))),
        "tools/list" => Some(ok(id, json!({ "tools": mcp::tools_for(tier) }))),
        "tools/call" => Some(call_tool(id, &params, sock, session)),
        // Everything else, including the three that would otherwise become
        // injection surfaces: `skills/list` (registers SKILL.md bodies into
        // the system prompt and is invisible in tools/list), `server/discover`
        // (a 2026-07-28 probe mesimon does not implement, so refusing is
        // conformant), and `resources/list` + `prompts/list`, which mesimon
        // declares no capability for.
        _ => Some(err(id, mcp::METHOD_NOT_FOUND, &format!("method not found: {method}"))),
    }
}

fn call_tool(id: Value, params: &Value, sock: &PathBuf, session: uuid::Uuid) -> Value {
    ok(id, tool_result(params, sock, session))
}

/// One `tools/call`'s `result`: the shim's and the mod's (T-577) alike.
fn tool_result(params: &Value, sock: &PathBuf, session: uuid::Uuid) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let call = match mcp::parse_tool_call(name, &args) {
        Ok(c) => c,
        Err(message) => return tool_error(&message),
    };
    // The client's own tool-use id, when the agent supplied no key of its own.
    // It is already on the wire in `_meta`, and it is stable across the retry
    // path that matters: a dropped connection after the move was persisted.
    let tool_use_id = params
        .get("_meta")
        .and_then(|m| m.get("claudecode/toolUseId"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let command = match call {
        // `get_ticket` with a key is its own command on the wire (T-411), so
        // a shim from before the crown still parses at the daemon.
        ToolCall::GetTicket { key: None } => Command::AgentGetTicket,
        ToolCall::GetTicket { key: Some(key) } => Command::AgentReadTicket { key },
        ToolCall::ListBoard => Command::AgentListBoard,
        ToolCall::MoveTicket { to_column, idempotency_key, key, before, seen } => {
            Command::AgentMoveTicket {
                to_column,
                idempotency_key: idempotency_key.or(tool_use_id),
                key,
                before,
                seen,
            }
        }
        ToolCall::ReadAttachment { attachment, key } => {
            Command::AgentReadAttachment { attachment, key }
        }
        ToolCall::ReadNote { note, key } => Command::AgentReadNote { note, key },
        ToolCall::WriteNote { note, text, key } => Command::AgentWriteNote { note, text, key },
        ToolCall::RenameTicket { key, title, seen } => {
            Command::AgentRenameTicket { key, title, seen: Some(seen) }
        }
        ToolCall::SetWorkspace { key, workspace, seen } => {
            Command::AgentSetWorkspace { key, workspace, seen: Some(seen) }
        }
        ToolCall::ArchiveTicket { key, restore, seen } => {
            Command::AgentArchiveTicket { key, restore, seen: Some(seen) }
        }
        ToolCall::StartAgent { key, seen, plan, tier, workspace } => Command::AgentStartTicket {
            key,
            seen: Some(seen),
            plan,
            tier,
            workspace: Some(workspace),
        },
        ToolCall::SleepAgent { key, seen } => Command::AgentSleepTicket { key, seen: Some(seen) },
        ToolCall::MergeTicket { key, seen } => Command::AgentMergeTicket { key, seen: Some(seen) },
        ToolCall::AskAgent { key, text, seen, plan, deliver } => {
            Command::AgentAskTicket { key, text, seen: Some(seen), plan, deliver }
        }
        ToolCall::AnswerAgent { key, seen, request, index, text, answers } => {
            Command::AgentAnswerTicket { key, seen: Some(seen), request, index, text, answers }
        }
        ToolCall::AcceptPlan { key, seen, request } => {
            Command::AgentAcceptPlan { key, seen: Some(seen), request }
        }
        ToolCall::CreateTicket {
            title,
            column,
            description,
            tags,
            idempotency_key,
            tier,
            workspace,
        } => Command::AgentCreateTicket {
            title,
            column,
            description,
            tags,
            idempotency_key: idempotency_key.or(tool_use_id),
            tier,
            workspace,
        },
        ToolCall::TagTicket { name, group, remove, key } => {
            Command::AgentTagTicket { name, group, remove, key }
        }
        ToolCall::RaiseHand { reason } => Command::AgentRaiseHand { reason },
    };
    let env = Envelope { principal: Principal::Agent { session }, command };
    match ask(sock, &env) {
        Ok(resp) => render(resp),
        Err(message) => tool_error(&message),
    }
}

/// A daemon `Response` as the model sees it.
///
/// A refusal is a tool *result* with `isError`, never a JSON-RPC error: the
/// model is meant to read "that column does not exist" or "this would undo a
/// move you just made" and act on it, and a transport-level error would be
/// rendered as a malfunction instead of an answer.
fn render(resp: Response) -> Value {
    match resp {
        Response::AgentTicket { ticket } => text(&ticket),
        Response::AgentBoard { board } => text(&board),
        Response::AgentMoved { column, board_version, replayed, seen } => {
            let mut body =
                json!({ "column": column, "board_version": board_version, "replayed": replayed });
            // A keyed move (T-411) hands back the target's fresh stamp so the
            // next edit needs no second read; an own-ticket move carries none.
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            text(&body)
        }
        // The workspace the ticket was filed with (T-583), so a crown that
        // will start it reads its own choice back.
        Response::AgentCreated { key, column, board_version, replayed, workspace } => {
            let mut body = json!({
                "key": key, "column": column, "board_version": board_version, "replayed": replayed
            });
            if !workspace.is_empty() {
                body["workspace"] = json!(workspace);
            }
            text(&body)
        }
        Response::AgentTagged { tags, replaced, board_version, seen } => {
            let mut body =
                json!({ "tags": tags, "replaced": replaced, "board_version": board_version });
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            text(&body)
        }
        // The words as the board KEPT them: scrubbed and capped, so a line
        // that came back short says so where the model can see it.
        Response::AgentRaised { reason, board_version } => {
            text(&json!({ "reason": reason, "board_version": board_version }))
        }
        // The crown's start (T-412): which ticket, whether its pane runs now
        // or the start waits on the worktree cut, and the seats left. A word,
        // not a bool (T-466): every refusal is `isError`, so a `false` here
        // read as "no" when it meant "not yet".
        // `wakes` (T-537) says the board will tell the crown what became of
        // the start, so it arms no monitor of its own — which would hold the
        // wake — and how it picks a tier (T-584). `tier` names the one the
        // agent launched on; a daemon from before it sends none. `woken`
        // (T-583) is the crown's own parked agent back in its conversation,
        // and `workspace` where it runs.
        Response::AgentStarted { key, session_started, budget_left, tier, woken, workspace } => {
            let mut body = json!({
                "key": key,
                "status": match (session_started, woken) {
                    (false, _) => "waiting_for_worktree",
                    (true, true) => "woken",
                    (true, false) => "started",
                },
                "budget_left": budget_left,
                "wakes": mcp::CROWN_WAKES
            });
            if !tier.is_empty() {
                body["tier"] = json!(tier);
            }
            if !workspace.is_empty() {
                body["workspace"] = json!(workspace);
            }
            text(&body)
        }
        // The crown's merge (T-613): the branch landed, in the merge's own
        // words, and what became of the merged notice to the worker — a
        // word, never a bool (T-466). Every refusal is `Err`.
        Response::AgentMerged { key, detail, notice, seen } => {
            let mut body =
                json!({ "key": key, "outcome": "merged", "detail": detail, "notice": notice });
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            text(&body)
        }
        // The crown's ask (T-413): held on the card until a person sends it,
        // or, where the board lets the crown send (T-550), queued to go once
        // the agent is idle, or sent at once with `deliver` now (T-600) or
        // immediately (T-601). `road` names which; `held_because` says why a
        // send was held.
        Response::AgentAsked { key, replaced, seen, held_for_person, held_because, road } => {
            let mut body = json!({
                "key": key,
                "replaced": replaced,
                "road": AskRoad::of(road, held_for_person).word(),
                "held_for_person": held_for_person,
                "wakes": mcp::CROWN_WAKES
            });
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            if let Some(why) = held_because {
                body["held_because"] = json!(why);
            }
            text(&body)
        }
        // The crown's answer (T-569), once its delivery settled: `answered`
        // only on the dialog's own hook edge, `input_sent` for keys that went
        // in unconfirmed, `unknown` with its reason when none could. The
        // receipt repeats which questions stay a person's.
        Response::AgentAnswered { key, outcome, reason, answer, seen } => {
            let mut body = json!({
                "key": key,
                "outcome": outcome,
                "answer": answer,
                "persons_questions": mcp::PERSONS_QUESTIONS,
            });
            if let Some(why) = reason {
                body["reason"] = json!(why);
            }
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            text(&body)
        }
        // The crown's plan accept (T-582), once its press settled:
        // `accepted` only on the plan's own hook edge, `input_sent` for an
        // Enter unconfirmed, `queued` while a busy checkout holds the press,
        // `unknown` with its reason when no Enter went. The receipt repeats
        // which plans stay a person's.
        Response::AgentPlanAccepted { key, outcome, reason, seen } => {
            let mut body = json!({
                "key": key,
                "outcome": outcome,
                "persons_plans": mcp::PERSONS_PLANS,
            });
            if let Some(why) = reason {
                body["reason"] = json!(why);
            }
            if let Some(seen) = seen {
                body["seen"] = json!(seen);
            }
            text(&body)
        }

        // The body as the text block itself: markdown inside a JSON string is
        // a worse read, and the metadata already travels in `get_ticket`.
        Response::Note { text: body, .. } => {
            json!({ "content": [{ "type": "text", "text": body }], "isError": false })
        }
        Response::Attachment { data, .. } => json!({
            "content": [{ "type": "image", "mimeType": "image/png", "data": data }],
            "isError": false
        }),
        Response::NoteWritten { note } => {
            text(&json!({ "note": note.map(|n| n.to_string()), "deleted": note.is_none() }))
        }
        Response::Err { message } => tool_error(&message),
        other => tool_error(&format!(
            "unexpected daemon response: {}",
            serde_json::to_string(&other).unwrap_or_else(|_| "?".into())
        )),
    }
}

/// One request, one connection, one line each way.
fn ask(sock: &PathBuf, env: &Envelope) -> Result<Response, String> {
    let stream = UnixStream::connect(sock)
        .map_err(|e| format!("mesimon daemon is not reachable ({e}); the board may be closed"))?;
    let wait = match env.command {
        Command::AgentAnswerTicket { .. } | Command::AgentAcceptPlan { .. } => ANSWER_WAIT_SECS,
        _ => READ_TIMEOUT_SECS,
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(wait)))
        .map_err(|e| e.to_string())?;
    let line = serde_json::to_string(env).map_err(|e| e.to_string())?;
    {
        let mut w = &stream;
        writeln!(w, "{line}").map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
    }
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    if reply.trim().is_empty() {
        return Err("mesimon daemon closed the connection without answering".into());
    }
    serde_json::from_str(&reply).map_err(|e| format!("unreadable daemon response: {e}"))
}

fn text<T: serde::Serialize>(v: &T) -> Value {
    let body = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    json!({ "content": [{ "type": "text", "text": body }], "isError": false })
}

fn tool_error(message: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn val<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(v: Value) -> Option<Value> {
        handle_line(
            &v.to_string(),
            &PathBuf::from("/nonexistent.sock"),
            uuid::Uuid::nil(),
            AgentTools::Full,
        )
    }

    #[test]
    fn initialize_answers_and_carries_no_instructions() {
        let r = line(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                            "params":{"protocolVersion":"2025-11-25"}}))
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
        assert!(r["result"].get("instructions").is_none());
        assert_eq!(r["result"]["serverInfo"]["name"], "mesimon");
    }

    #[test]
    fn tools_list_returns_the_whole_tier() {
        let r = line(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(
            r["result"]["tools"].as_array().unwrap().len(),
            mesimon_core::mcp::tools().len()
        );
    }

    /// The receipt is what the ticket wears now, so the model sees the
    /// groupmate its call displaced without a second round-trip.
    #[test]
    fn a_tag_result_names_what_is_worn_and_what_came_off() {
        let v = render(Response::AgentTagged {
            tags: vec![mesimon_core::command::AgentTagView { name: "feature".into(), group: 1 }],
            replaced: Some("bug".into()),
            board_version: 4,
            seen: None,
        });
        assert_eq!(v["isError"], false);
        let body: Value = serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["tags"][0]["name"], "feature");
        assert_eq!(body["tags"][0]["group"], 1);
        assert_eq!(body["replaced"], "bug");
    }

    #[test]
    fn a_create_result_names_the_new_key() {
        let v = render(Response::AgentCreated {
            key: "T-9".into(),
            column: "TODO".into(),
            board_version: 3,
            replayed: false,
            workspace: "worktree".into(),
        });
        assert_eq!(v["isError"], false);
        let body: Value = serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["key"], "T-9");
        assert_eq!(body["column"], "TODO");
        assert_eq!(body["replayed"], false);
        // T-583: the receipt echoes the workspace it was filed with.
        assert_eq!(body["workspace"], "worktree");
    }

    /// A note body is the text block itself, not JSON with a string in it.
    #[test]
    fn a_note_renders_as_its_own_text() {
        let meta = mesimon_core::board::NoteMeta {
            id: ulid::Ulid::nil(),
            name: "Why".into(),
            rev: 1,
            created_at: "@1".into(),
            created_by: "local".into(),
            edited_at: "@1".into(),
            edited_by: "local".into(),
        };
        let r = render(Response::Note { text: "# Why\n\nbecause".into(), meta });
        assert_eq!(r["content"][0]["text"], "# Why\n\nbecause");
        assert_eq!(r["isError"], false);
        let r = render(Response::NoteWritten { note: None });
        assert!(r["content"][0]["text"].as_str().unwrap().contains("\"deleted\": true"));
    }

    /// Answering a notification is a protocol error. `notifications/initialized`
    /// arrives on every startup, so getting this wrong breaks every session.
    #[test]
    fn a_notification_gets_no_reply() {
        assert!(line(json!({"jsonrpc":"2.0","method":"notifications/initialized"})).is_none());
        assert!(line(json!({"jsonrpc":"2.0","method":"notifications/cancelled"})).is_none());
    }

    /// The three methods that are injection surfaces if answered, plus the
    /// two capabilities mesimon does not declare.
    #[test]
    fn the_dangerous_methods_are_refused() {
        for m in [
            "skills/list",
            "server/discover",
            "resources/list",
            "prompts/list",
            "sampling/createMessage",
            "resources/templates/list",
        ] {
            let r = line(json!({"jsonrpc":"2.0","id":9,"method":m})).unwrap();
            assert_eq!(r["error"]["code"], mcp::METHOD_NOT_FOUND, "{m} must be refused");
        }
    }

    #[test]
    fn ping_is_answered() {
        let r = line(json!({"jsonrpc":"2.0","id":3,"method":"ping"})).unwrap();
        assert_eq!(r["result"], json!({}));
    }

    /// An unknown tool is a tool result, not a transport error: the model
    /// sometimes holds a `tool_reference` for a tool that no longer exists,
    /// and "no such tool" is an answer it can act on.
    #[test]
    fn an_unknown_tool_is_a_tool_error() {
        let r = line(json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
                            "params":{"name":"get_session","arguments":{}}}))
        .unwrap();
        assert!(r.get("error").is_none());
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("unknown tool"));
    }

    /// A dead daemon must read as "the board is closed", not as a crash.
    #[test]
    fn an_unreachable_daemon_is_a_legible_tool_error() {
        let r = line(json!({"jsonrpc":"2.0","id":5,"method":"tools/call",
                            "params":{"name":"get_ticket","arguments":{}}}))
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("not reachable"));
    }

    #[test]
    fn a_daemon_refusal_reaches_the_model_as_a_readable_result() {
        let v = render(Response::Err { message: "no such column: NOPE".into() });
        assert_eq!(v["isError"], true);
        assert_eq!(v["content"][0]["text"], "no such column: NOPE");
    }

    #[test]
    fn a_move_result_names_where_it_landed() {
        let v = render(Response::AgentMoved {
            column: "REVIEW".into(),
            board_version: 7,
            replayed: true,
            seen: None,
        });
        assert_eq!(v["isError"], false);
        let body: Value = serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["column"], "REVIEW");
        assert_eq!(body["replayed"], true);
        assert!(body.get("seen").is_none(), "an own-ticket move carries no stamp");
        // A keyed move (T-411) hands the fresh stamp back.
        let v = render(Response::AgentMoved {
            column: "TODO".into(),
            board_version: 8,
            replayed: false,
            seen: Some("ab12".into()),
        });
        let body: Value = serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["seen"], "ab12");
    }

    /// A start parked behind a worktree cut is accepted, and its receipt
    /// must not read as a refusal (T-466): the crown told a person to press
    /// T-600: the ask's receipt names the road the words took, and reads
    /// it from the flag when an older daemon sent none.
    #[test]
    fn an_ask_receipt_names_its_road() {
        let body = |held_for_person, road| {
            let v = render(Response::AgentAsked {
                key: "T-7".into(),
                replaced: false,
                seen: None,
                held_for_person,
                held_because: None,
                road,
            });
            assert_eq!(v["isError"], false);
            serde_json::from_str::<Value>(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        assert_eq!(body(false, Some(AskRoad::SentNow))["road"], "sent_now");
        assert_eq!(body(false, Some(AskRoad::SentImmediately))["road"], "sent_immediately");
        assert_eq!(body(false, Some(AskRoad::Queued))["road"], "queued");
        assert_eq!(body(true, Some(AskRoad::HeldForPerson))["road"], "held_for_person");
        assert_eq!(body(true, None)["road"], "held_for_person");
        assert_eq!(body(false, None)["road"], "queued");
        assert_eq!(body(false, None)["wakes"], mcp::CROWN_WAKES);
    }

    /// Shift+Enter on a start that landed thirty seconds later.
    #[test]
    fn a_start_receipt_says_started_or_waiting_never_false() {
        let body = |session_started, woken| {
            let v = render(Response::AgentStarted {
                key: "T-7".into(),
                session_started,
                budget_left: 0,
                tier: "deep".into(),
                woken,
                workspace: "worktree".into(),
            });
            assert_eq!(v["isError"], false);
            serde_json::from_str::<Value>(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let now = body(true, false);
        assert_eq!(now["status"], "started");
        assert_eq!(now["key"], "T-7");
        assert_eq!(now["budget_left"], 0);
        // T-584: the receipt names the tier the agent launched on, and the
        // paragraph says how the crown picks one.
        assert_eq!(now["tier"], "deep");
        assert!(mcp::CROWN_WAKES.contains(mcp::CROWN_TIERS));
        // T-537: the receipt says the board wakes the crown, so it polls nothing.
        assert_eq!(now["wakes"], mcp::CROWN_WAKES);
        assert!(mcp::CROWN_WAKES.contains("Nothing needs polling"));
        let parked = body(false, false);
        assert_eq!(parked["status"], "waiting_for_worktree");
        assert!(parked.get("session_started").is_none(), "no bool to read as a refusal");
        // T-583: the crown's parked agent woken, and where every start runs.
        assert_eq!(body(true, true)["status"], "woken");
        assert_eq!(body(false, true)["status"], "waiting_for_worktree", "a rebuild first");
        assert_eq!(now["workspace"], "worktree");
    }

    /// The crown's answer (T-569): the outcome the hooks saw, the answer as
    /// delivered, the reason only when there is one, and the one clause
    /// naming which questions stay a person's.
    #[test]
    fn an_answer_receipt_says_what_the_hooks_saw_and_whose_questions_are_whose() {
        let body = |outcome: &str, reason: Option<&str>| {
            let v = render(Response::AgentAnswered {
                key: "T-5".into(),
                outcome: outcome.into(),
                reason: reason.map(str::to_string),
                answer: "Okta".into(),
                seen: Some("ab12".into()),
            });
            assert_eq!(v["isError"], false);
            serde_json::from_str::<Value>(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let answered = body("answered", None);
        assert_eq!(answered["outcome"], "answered");
        assert_eq!(answered["answer"], "Okta");
        assert_eq!(answered["seen"], "ab12");
        assert!(answered.get("reason").is_none());
        assert_eq!(answered["persons_questions"], mcp::PERSONS_QUESTIONS);
        let unknown = body("unknown", Some("label_not_found"));
        assert_eq!(unknown["reason"], "label_not_found");
    }

    /// The crown's plan accept (T-582): the outcome the hooks saw, the
    /// reason only when there is one, and the clause naming which plans
    /// stay a person's.
    #[test]
    fn a_plan_receipt_says_what_the_hooks_saw_and_whose_plans_are_whose() {
        let body = |outcome: &str, reason: Option<&str>| {
            let v = render(Response::AgentPlanAccepted {
                key: "T-5".into(),
                outcome: outcome.into(),
                reason: reason.map(str::to_string),
                seen: Some("ab12".into()),
            });
            assert_eq!(v["isError"], false);
            serde_json::from_str::<Value>(v["content"][0]["text"].as_str().unwrap()).unwrap()
        };
        let accepted = body("accepted", None);
        assert_eq!(
            (accepted["key"].as_str(), accepted["outcome"].as_str()),
            (Some("T-5"), Some("accepted"))
        );
        assert_eq!(accepted["seen"], "ab12");
        assert!(accepted.get("reason").is_none());
        assert_eq!(accepted["persons_plans"], mcp::PERSONS_PLANS);
        let unknown = body("unknown", Some("a_person_answered"));
        assert_eq!(unknown["reason"], "a_person_answered");
    }

    #[test]
    fn garbage_on_stdin_does_not_produce_a_reply() {
        assert!(handle_line("not json", &PathBuf::from("/x"), uuid::Uuid::nil(), AgentTools::Full)
            .is_none());
    }
}
