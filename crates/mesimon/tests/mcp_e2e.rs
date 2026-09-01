//! T-84: the board tools a mesimon-spawned session gets, the tier that keeps
//! everything else out of reach, the guards that stop the agent and `automove`
//! fighting over a card, and the write gate.
//!
//! Real tmux, an in-process daemon, the real built binary driven as Claude
//! Code drives it: `mesimon mcp` as a stdio subprocess speaking JSON-RPC.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command as Proc, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::{Board, SessionKind};
use mesimon_core::command::{Command, Envelope, Response};
use mesimon_core::Principal;
use serde_json::{json, Value};

// ------------------------------------------------------------ the wire

struct TestClient {
    write: UnixStream,
    read: BufReader<UnixStream>,
}

impl TestClient {
    fn connect(sock: &std::path::Path) -> Self {
        let stream = UnixStream::connect(sock).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let read = BufReader::new(stream.try_clone().unwrap());
        Self { write: stream, read }
    }

    fn send(&mut self, principal: Principal, command: Command) -> Response {
        let env = Envelope { principal, command };
        writeln!(self.write, "{}", serde_json::to_string(&env).unwrap()).unwrap();
        loop {
            let mut buf = String::new();
            self.read.read_line(&mut buf).expect("read");
            if let Ok(resp) = serde_json::from_str::<Response>(&buf) {
                return resp;
            }
        }
    }

    fn request(&mut self, command: Command) -> Response {
        self.send(Principal::Local, command)
    }
}

fn board_of(resp: Response) -> Board {
    match resp {
        Response::Board { board, .. } => board,
        other => panic!("expected board, got {other:?}"),
    }
}

fn notices_of(resp: &Response) -> Vec<String> {
    match resp {
        Response::Board { notices, .. } => notices.iter().map(|n| n.kind.clone()).collect(),
        other => panic!("expected board, got {other:?}"),
    }
}

fn hook_send(sock: &std::path::Path, session: &str, event: &str, body: &str) {
    let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["hook", "--sock"])
        .arg(sock)
        .args(["--session", session, "--event", event])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hook");
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

// ------------------------------------------- the shim, driven as Claude does

/// The real `mesimon mcp` process, spoken to over stdin/stdout exactly the way
/// Claude Code speaks to a stdio MCP server.
struct Shim {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Shim {
    fn start(sock: &std::path::Path, session: uuid::Uuid) -> Self {
        let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
            .args(["mcp", "--sock"])
            .arg(sock)
            .args(["--session", &session.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mcp shim");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out, next_id: 1 }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).expect("shim reply");
        let v: Value = serde_json::from_str(&line).expect("shim reply is json");
        assert_eq!(v["id"], json!(id), "reply id must match the request");
        v
    }

    fn notify(&mut self, method: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc":"2.0","method":method})).unwrap();
        stdin.flush().unwrap();
    }

    /// A tool call's parsed result body, asserting it was not an error.
    fn call_ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        let result = &r["result"];
        assert_eq!(result["isError"], false, "{name} failed: {result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    /// A tool call that is refused. Returns the message the model sees.
    fn call_err(&mut self, name: &str, args: Value) -> String {
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(r["result"]["isError"], true, "{name} unexpectedly succeeded: {r}");
        r["result"]["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn call_with_meta(&mut self, name: &str, args: Value, tool_use_id: &str) -> Value {
        let r = self.rpc(
            "tools/call",
            json!({"name": name, "arguments": args,
                   "_meta": {"claudecode/toolUseId": tool_use_id}}),
        );
        assert_eq!(r["result"]["isError"], false, "{name} failed: {r}");
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}

impl Drop for Shim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_column(c: &mut TestClient, ticket: ulid::Ulid, want: &str, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let board = board_of(c.request(Command::Snapshot));
        if board.ticket(ticket).unwrap().column == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: never reached {want}; column={} sessions={:?}",
            board.ticket(ticket).unwrap().column,
            board.sessions.iter().map(|s| (&s.state, &s.confidence)).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `automove` is edge-triggered on session-state transitions, so a test that
/// wants it to fire has to actually move the session — sending a second
/// `UserPromptSubmit` to an already-`Running` session produces no edge and no
/// move, and an assertion resting on that would pass for the wrong reason.
fn wait_for_idle(c: &mut TestClient, sid: uuid::Uuid, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let board = board_of(c.request(Command::Snapshot));
        let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
        if matches!(rec.state, mesimon_core::board::SessionState::Idle { .. }) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: session never went idle ({:?})", rec.state);
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// End the turn and start another — the only way to produce a fresh `Running`
/// edge, and therefore the only way to make `automove` consider a move.
fn turn(c: &mut TestClient, hook_sock: &std::path::Path, sid: uuid::Uuid, what: &str) {
    hook_send(hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    wait_for_idle(c, sid, what);
    hook_send(hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
}

// ------------------------------------------------------------------ the test

#[test]
fn agent_board_tools_tier_and_collisions() {
    if !common::require_tmux() {
        return;
    }
    let dir = std::path::PathBuf::from(format!("/tmp/msmn-e2e-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = mesimon_daemon::Paths::for_repo(&repo).unwrap();
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let tmux_sock = paths.tmux_sock();
    let state_dir = paths.state_dir.clone();
    let rt_dir = paths.rt_dir.clone();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    std::env::set_var("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    std::env::set_var("MESIMON_CLAUDE_BIN", &stub);
    // The no-undo window, widened past this test's wall clock so the guard is
    // certainly armed at every step. Its expiry is a `movegate` unit test —
    // sleeping out a real window here would only buy flakiness.
    std::env::set_var("MESIMON_PINGPONG_MS", "600000");

    let daemon_repo = repo.clone();
    let daemon = std::thread::spawn(move || {
        let _ = mesimon_daemon::run_foreground(&daemon_repo);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);

    let _ = c.request(Command::CreateTicket { column: "TODO".into(), title: "the work".into() });
    let _ = c.request(Command::CreateTicket { column: "REVIEW".into(), title: "decoy".into() });
    let board = board_of(c.request(Command::Snapshot));
    let ticket = board.tickets.iter().find(|t| t.title == "the work").unwrap().id;
    let key = board.ticket(ticket).unwrap().short_key.clone();

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    // ---- the config travels on argv and is installed nowhere -------------
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
    let i = rec.argv.iter().position(|a| a == "--mcp-config").expect("--mcp-config");
    let blob: Value = serde_json::from_str(&rec.argv[i + 1]).unwrap();
    assert_eq!(blob["mcpServers"]["mesimon"]["type"], "stdio");
    // The three files mesimon must never have written, and the one it must not
    // have created in the repo. This is the whole of "only for sessions
    // mesimon created": there is nowhere else for the config to have come from.
    for forbidden in
        [repo.join(".mcp.json"), repo.join(".claude/settings.local.json"), dir.join(".claude.json")]
    {
        assert!(!forbidden.exists(), "mesimon wrote {forbidden:?}");
    }

    // ---- the session knows which ticket it is on, from the shell ---------
    // MESIMON_TICKET now reaches a shared-checkout session too; before T-84 it
    // was worktree-only, which is the board default's blind spot.
    let env_out = Proc::new("tmux")
        .args(["-S", &tmux_sock.display().to_string(), "show-environment", "-t", &rec.sid16()])
        .output()
        .expect("tmux show-environment");
    let env_out = String::from_utf8_lossy(&env_out.stdout);
    assert!(
        env_out.contains(&format!("MESIMON_TICKET={key}")),
        "shared-checkout session must carry MESIMON_TICKET: {env_out}"
    );

    // ---- … and from the model, through the tools ------------------------
    let mut shim = Shim::start(&sock, sid);
    let init = shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    assert_eq!(init["result"]["serverInfo"]["name"], "mesimon");
    assert!(init["result"].get("instructions").is_none(), "instructions is the injection surface");
    assert_eq!(init["result"]["capabilities"], json!({"tools": {}}));
    shim.notify("notifications/initialized");

    let tools = shim.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["get_ticket", "list_board", "move_ticket"]);

    let t = shim.call_ok("get_ticket", json!({}));
    assert_eq!(t["key"], key.as_str());
    assert_eq!(t["title"], "the work");
    assert_eq!(t["workspace"], "shared_checkout");
    let allowed: Vec<&str> =
        t["allowed_columns"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(!allowed.contains(&t["column"].as_str().unwrap()), "current column is not a move");
    assert!(allowed.contains(&"REVIEW"));

    // ---- no tool reads a session, at any tier ---------------------------
    let listed = shim.call_ok("list_board", json!({}));
    let raw = serde_json::to_string(&listed).unwrap();
    for leak in ["session", "argv", "transcript", "cwd", "pid", "claude_session_id"] {
        assert!(!raw.contains(leak), "list_board leaked {leak:?}: {raw}");
    }
    assert!(raw.contains("decoy"), "the board is genuinely visible");

    // ---- the never-tier, on the wire ------------------------------------
    // Not "there is no tool for it" — the daemon refuses the command even when
    // it is handed one directly, which is what makes the tool list a summary
    // of the policy rather than the policy itself.
    let agent = Principal::Agent { session: sid };
    for forbidden in [
        Command::SpawnSession { ticket, kind: SessionKind::Bash, submit_prompt: false },
        Command::KillSession { id: sid },
        Command::DeleteTicket { id: ticket, discard_worktree: true },
        Command::ArchiveTicket { id: ticket },
        Command::RenameTicket { id: ticket, title: "hijacked".into() },
        Command::MoveTicket { id: ticket, column: "DONE".into(), before: None },
        Command::Snapshot,
        Command::Shutdown,
    ] {
        let label = format!("{forbidden:?}");
        match c.send(agent.clone(), forbidden) {
            Response::Err { message } => {
                assert!(message.contains("agent"), "{label}: unhelpful refusal {message:?}")
            }
            other => panic!("{label} must be refused for an agent, got {other:?}"),
        }
    }
    // A local client cannot borrow the agent path either.
    match c.request(Command::AgentGetTicket) {
        Response::Err { .. } => {}
        other => panic!("agent commands need an agent principal, got {other:?}"),
    }

    // ---- move_ticket, and the idempotency replay -------------------------
    let moved = shim.call_with_meta("move_ticket", json!({"to_column": "REVIEW"}), "toolu_01");
    assert_eq!(moved["column"], "REVIEW");
    assert_eq!(moved["replayed"], false);
    assert_eq!(board_of(c.request(Command::Snapshot)).ticket(ticket).unwrap().column, "REVIEW");

    // The retry after a dropped connection: the same tool-use id must replay
    // the first answer, never move the card a second time.
    let _ = c.request(Command::MoveTicket { id: ticket, column: "TODO".into(), before: None });
    let again = shim.call_with_meta("move_ticket", json!({"to_column": "REVIEW"}), "toolu_01");
    assert_eq!(again["replayed"], true, "a repeated tool-use id must replay");
    assert_eq!(
        board_of(c.request(Command::Snapshot)).ticket(ticket).unwrap().column,
        "TODO",
        "a replayed call must not move the card again"
    );

    // ---- refusals the model can act on ----------------------------------
    let msg = shim.call_err("move_ticket", json!({"to_column": "NO SUCH COLUMN"}));
    assert!(msg.contains("no such column"), "{msg}");

    // ---- the collision: an agent move vs automove ------------------------
    //
    // Session transitions are scarce here on purpose: the attention machine
    // has its own flap guard (>4 committed changes in 20 s pins it at Low
    // confidence, and `automove` refuses to move on Low). So this drives the
    // minimum number of turns and gets both directions of the collision out
    // of them.
    //
    // First, the positive control. The card is in TODO because a human put it
    // there; a `Running` edge drags a TODO ticket to IN PROGRESS, and that is
    // not an undo of REVIEW → TODO, so it must go through. A guard that
    // refused this would be worse than no guard: the board would just stop
    // working.
    hook_send(&hook_sock, &sid.to_string(), "SessionStart", r#"{"source":"startup"}"#);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"session_id":"x"}"#);
    wait_for_column(&mut c, ticket, "IN PROGRESS", "running after a human park");

    // That move was made by `automove`, so the agent may not now undo it —
    // and the refusal has to name what it is protecting, because an agent
    // told only "no" will simply try again.
    let msg = shim.call_err("move_ticket", json!({"to_column": "TODO"}));
    assert!(msg.contains("undo"), "the refusal must name what it protects: {msg}");

    // A different destination is not an undo. The agent says it is done.
    let moved = shim.call_ok("move_ticket", json!({"to_column": "REVIEW"}));
    assert_eq!(moved["column"], "REVIEW");

    // Now the other direction: work resumes, and `automove` would drag a
    // REVIEW ticket back to IN PROGRESS on the `Running` edge — the exact
    // reverse of the move the agent just made. That is the flap.
    turn(&mut c, &hook_sock, sid, "agent said review");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        board_of(c.request(Command::Snapshot)).ticket(ticket).unwrap().column,
        "REVIEW",
        "automove must not undo the move the agent just made"
    );

    // ---- the flap fuse ---------------------------------------------------
    // Six automatic moves of one ticket inside the window suspend automation
    // for it. Alternating agent moves are not caught by the no-undo rule —
    // that only fires against a DIFFERENT principal — so the fuse is what
    // stops a card oscillating, and it is the protection that will still hold
    // for whatever automation M5 adds.
    for i in 0..8 {
        let dest = if i % 2 == 0 { "REVIEW" } else { "IN PROGRESS" };
        let _ =
            shim.rpc("tools/call", json!({"name":"move_ticket","arguments":{"to_column":dest}}));
    }
    assert!(
        notices_of(&c.request(Command::Snapshot)).contains(&"automation_suspended".to_string()),
        "a blown fuse must be visible on the board, not silent"
    );
    let msg = shim.call_err("move_ticket", json!({"to_column": "DONE"}));
    assert!(msg.contains("suspended"), "{msg}");
    // A move by hand clears it, and the notice goes away with it.
    let _ = c.request(Command::MoveTicket { id: ticket, column: "TODO".into(), before: None });
    assert!(
        !notices_of(&c.request(Command::Snapshot)).contains(&"automation_suspended".to_string()),
        "moving by hand must clear the suspension"
    );
    // Not the column it just came from — that would be an undo, which is a
    // different rule and would pass for the wrong reason.
    let moved = shim.call_ok("move_ticket", json!({"to_column": "IN PROGRESS"}));
    assert_eq!(moved["column"], "IN PROGRESS", "the fuse is cleared, not permanent");

    // ---- the write gate --------------------------------------------------
    let board_file = repo.join(".mesimon/board/tickets").join(&key).join("ticket.toml");
    assert!(board_file.is_file(), "the ticket file exists to be protected");
    let denied =
        gate_verdict(&hook_sock, sid, &repo, &state_dir, &board_file.display().to_string());
    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecision"], "deny",
        "a structured write into .mesimon/ must be refused: {denied}"
    );
    let reason = denied["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("mesimon"), "the denial must say who refused: {reason}");
    // The state dir too.
    let denied = gate_verdict(
        &hook_sock,
        sid,
        &repo,
        &state_dir,
        &state_dir.join("sessions.json").display().to_string(),
    );
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    // …and ordinary source is silent. Empty stdout, never `{"decision":"ask"}`:
    // `ask` collapses to a deny in headless, which would refuse every edit.
    let out = gate_raw(
        &hook_sock,
        sid,
        &repo,
        &state_dir,
        &repo.join("src/main.rs").display().to_string(),
    );
    assert!(out.is_empty(), "the gate must have no opinion on ordinary writes, got {out:?}");

    // ---- teardown --------------------------------------------------------
    drop(shim);
    let _ = c.request(Command::Shutdown);
    let _ = daemon.join();
    let _ =
        Proc::new("tmux").args(["-S", &tmux_sock.display().to_string(), "kill-server"]).output();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&state_dir);
    let _ = std::fs::remove_dir_all(&rt_dir);
}

/// Run the real `mesimon gate` over a PreToolUse payload and return its stdout.
fn gate_raw(
    hook_sock: &std::path::Path,
    session: uuid::Uuid,
    repo: &std::path::Path,
    state_dir: &std::path::Path,
    file_path: &str,
) -> String {
    let mut child = Proc::new(env!("CARGO_BIN_EXE_mesimon"))
        .args(["gate", "--session", &session.to_string(), "--sock"])
        .arg(hook_sock)
        .arg("--deny-board")
        .arg(repo.join(".mesimon"))
        .arg("--deny-state")
        .arg(state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gate");
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Edit",
        "cwd": repo.display().to_string(),
        "tool_input": {"file_path": file_path, "old_string": "a", "new_string": "b"},
    });
    child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().expect("gate output");
    assert!(out.status.success(), "the gate always exits 0");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn gate_verdict(
    hook_sock: &std::path::Path,
    session: uuid::Uuid,
    repo: &std::path::Path,
    state_dir: &std::path::Path,
    file_path: &str,
) -> Value {
    let out = gate_raw(hook_sock, session, repo, state_dir, file_path);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("gate stdout is not json ({e}): {out:?}"))
}
