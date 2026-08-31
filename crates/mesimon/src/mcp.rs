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
//! One connection per call, deliberately. Tool calls happen at human scale, and
//! a fresh connect removes every stale-socket path on a daemon that gets
//! restarted many times a day during its own development.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::mcp::{self, ToolCall};
use mesimon_core::Principal;
use serde_json::{json, Value};

/// Long enough that a writer thread busy with a provisioning burst still
/// answers, short enough that a wedged daemon does not hold the agent's turn.
const READ_TIMEOUT_SECS: u64 = 20;

pub fn run(args: &[String]) -> ! {
    let Some(sock) = val(args, "--sock").map(PathBuf::from) else {
        eprintln!("mesimon mcp: --sock is required");
        std::process::exit(2);
    };
    let Some(session) = val(args, "--session").and_then(|s| s.parse::<uuid::Uuid>().ok()) else {
        eprintln!("mesimon mcp: --session must be a uuid");
        std::process::exit(2);
    };

    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = handle_line(&line, &sock, session) else { continue };
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
fn handle_line(line: &str, sock: &PathBuf, session: uuid::Uuid) -> Option<Value> {
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
        "tools/list" => Some(ok(id, json!({ "tools": mcp::tools() }))),
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
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let call = match mcp::parse_tool_call(name, &args) {
        Ok(c) => c,
        Err(message) => return ok(id, tool_error(&message)),
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
        ToolCall::GetTicket => Command::AgentGetTicket,
        ToolCall::ListBoard => Command::AgentListBoard,
        ToolCall::MoveTicket { to_column, idempotency_key } => {
            Command::AgentMoveTicket { to_column, idempotency_key: idempotency_key.or(tool_use_id) }
        }
    };
    let env = Envelope { principal: Principal::Agent { session }, command };
    match ask(sock, &env) {
        Ok(resp) => ok(id, render(resp)),
        Err(message) => ok(id, tool_error(&message)),
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
        Response::AgentMoved { column, board_version, replayed } => {
            text(&json!({ "column": column, "board_version": board_version, "replayed": replayed }))
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
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(READ_TIMEOUT_SECS)))
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
        handle_line(&v.to_string(), &PathBuf::from("/nonexistent.sock"), uuid::Uuid::nil())
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
    fn tools_list_returns_the_three() {
        let r = line(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 3);
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
        });
        assert_eq!(v["isError"], false);
        let body: Value = serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["column"], "REVIEW");
        assert_eq!(body["replayed"], true);
    }

    #[test]
    fn garbage_on_stdin_does_not_produce_a_reply() {
        assert!(handle_line("not json", &PathBuf::from("/x"), uuid::Uuid::nil()).is_none());
    }
}
