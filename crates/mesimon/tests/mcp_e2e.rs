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
use common::*;

use std::io::Write;
use std::process::{Command as Proc, Stdio};
use std::time::{Duration, Instant};

use mesimon_core::board::SessionKind;
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;
use serde_json::{json, Value};

// ------------------------------------------------------------ the wire

fn notices_of(resp: &Response) -> Vec<String> {
    match resp {
        Response::Board { notices, .. } => notices.iter().map(|n| n.kind.clone()).collect(),
        other => panic!("expected board, got {other:?}"),
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
    let fixture = common::TestFixture::new("mcp");
    let dir = fixture.dir.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    let paths = fixture.paths(&repo);
    let sock = paths.orch_sock();
    let hook_sock = paths.hook_sock();
    let tmux_sock = paths.tmux_sock();
    let state_dir = paths.state_dir.clone();

    let stub = dir.join("claude-stub.sh");
    std::fs::write(&stub, "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n").unwrap();
    std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    fixture.set_env("MESIMON_HOOK_BIN", env!("CARGO_BIN_EXE_mesimon"));
    fixture.set_env("MESIMON_CLAUDE_BIN", &stub);
    // The no-undo window, widened past this test's wall clock so the guard is
    // certainly armed at every step. Its expiry is a `movegate` unit test —
    // sleeping out a real window here would only buy flakiness.
    fixture.set_env("MESIMON_PINGPONG_MS", "600000");
    // The registry is built by hand below, from nothing: decline the starter
    // tags a fresh board is otherwise offered.
    fixture.set_env("MESIMON_NO_TAG_SEED", "1");

    let daemon_repo = repo.clone();
    let daemon = fixture.daemon(&daemon_repo);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon socket never appeared");
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut c = TestClient::connect(&sock);

    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "the work".into(),
        workspace: None,
        tier: None,
    });
    let _ = c.request(Command::CreateTicket {
        column: "REVIEW".into(),
        title: "decoy".into(),
        workspace: None,
        tier: None,
    });
    let board = board_of(c.request(Command::Snapshot));
    let ticket = board.tickets.iter().find(|t| t.title == "the work").unwrap().id;
    let key = board.ticket(ticket).unwrap().short_key.clone();

    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };

    // ---- the config travels on argv and is installed nowhere -------------
    let board = board_of(c.request(Command::Snapshot));
    let rec = board.sessions.iter().find(|s| s.id == sid).unwrap();
    // On the mod road (T-577) the mod registers the tools: no blob at all,
    // the tier on the pane (`launch_tools`).
    if test_road() == "hooks" {
        let i = rec.argv.iter().position(|a| a == "--mcp-config").expect("--mcp-config");
        let blob: Value = serde_json::from_str(&rec.argv[i + 1]).unwrap();
        assert_eq!(blob["mcpServers"]["mesimon"]["type"], "stdio");
    }
    assert_eq!(launch_tools(&tmux_sock, rec).as_deref(), Some("full"));
    // The three files mesimon must never have written, and the one it must not
    // have created in the repo. This is the whole of "only for sessions
    // mesimon created": there is nowhere else for the config to have come from.
    for forbidden in
        [repo.join(".mcp.json"), repo.join(".claude/settings.local.json"), dir.join(".claude.json")]
    {
        assert!(!forbidden.exists(), "mesimon wrote {forbidden:?}");
    }

    // ---- and the whole surface can be switched off (T-217) ---------------
    // Off means the flag is ABSENT, not an empty config: a session that was
    // never told about mesimon cannot be told about it later, and that is the
    // whole of what the switch promises. A second ticket, because a live
    // pane's argv was fixed at exec and nothing can revise it.
    assert!(matches!(c.request(Command::SetMcpTools { on: false }), Response::Ok));
    let quiet = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "no tools".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create failed: {other:?}"),
    };
    let quiet_sid = match c.request(Command::SpawnSession {
        ticket: quiet,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let board = board_of(c.request(Command::Snapshot));
    let quiet_rec = board.sessions.iter().find(|s| s.id == quiet_sid).unwrap();
    assert!(
        !quiet_rec.argv.iter().any(|a| a == "--mcp-config"),
        "the switch is off, so the flag is not there at all: {:?}",
        quiet_rec.argv
    );
    assert_eq!(launch_tools(&tmux_sock, quiet_rec), None, "and no tier on the pane");
    // The hooks are untouched — the two flags are different promises, and
    // turning the tools off must not also blind the board to attention: the
    // hook set, or on the mod road the mod, which carries the frames.
    let observed = if test_road() == "hooks" { "--settings" } else { "--plugin-dir" };
    assert!(quiet_rec.argv.iter().any(|a| a == observed), "{:?}", quiet_rec.argv);
    assert!(matches!(c.request(Command::SetMcpTools { on: true }), Response::Ok));

    // ---- the session knows which ticket it is on, from the shell ---------
    // MESIMON_TICKET now reaches a shared-checkout session too; before T-84 it
    // was worktree-only, which is the board default's blind spot.
    // It rides the launcher's `--set`, on the pane's command line on purpose:
    // a ticket key is not a secret, and `ps` naming a pane's ticket is useful.
    // (The user's captured environment is NOT there — shell_env_e2e holds that.)
    let start = tmux(&tmux_sock)
        .args(["list-panes", "-t", &rec.sid16(), "-F", "#{pane_start_command}"])
        .output()
        .expect("tmux list-panes");
    let start = String::from_utf8_lossy(&start.stdout);
    assert!(
        start.contains(&format!("MESIMON_TICKET={key}")),
        "shared-checkout session must carry MESIMON_TICKET: {start}"
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
            // The crown's three (T-411), its start (T-412), its sleep
            // (T-539), its ask (T-413), its answer (T-569), its plan
            // accept (T-582) and its merge (T-613): listed on every
            // full-tier session, refused by the daemon on every ticket but
            // the crowned one.
            "rename_ticket",
            "set_workspace",
            "archive_ticket",
            "start_agent",
            "sleep_agent",
            "ask_agent",
            "answer_agent",
            "accept_plan",
            "merge_ticket"
        ]
    );

    let t = shim.call_ok("get_ticket", json!({}));
    assert_eq!(t["key"], key.as_str());
    assert_eq!(t["title"], "the work");
    assert_eq!(t["workspace"], "shared_checkout");
    let allowed: Vec<&str> =
        t["allowed_columns"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(!allowed.contains(&t["column"].as_str().unwrap()), "current column is not a move");
    assert!(allowed.contains(&"REVIEW"));
    // T-376: the current column's own rules ride along, so the agent can see
    // what the board will do by itself and leave that move to the board.
    assert_eq!(t["column"], "TODO");
    assert_eq!(t["automove"], json!({"on_working": "IN PROGRESS", "on_done": null}));
    assert_eq!(t["tags"], json!([]), "nothing worn yet");
    assert_eq!(t["allowed_tags"], json!([]), "nothing in the registry yet");
    // Before anyone has made a tag there is nothing to wear, and the refusal
    // says where tags come from rather than minting one.
    let msg = shim.call_err("tag_ticket", json!({"name": "bug"}));
    assert!(msg.contains("no tags yet"), "{msg}");

    // ---- tags: worn ones on get_ticket, the registry as allowed_tags -----
    // A person tags the caller's ticket and registers two more names; the
    // agent sees what it wears and every word the board knows, group and all.
    assert!(matches!(
        c.request(Command::SetTag { id: ticket, group: 1, name: Some("BUG".into()) }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::RegisterTag { group: 1, name: "FEAT".into() }),
        Response::Ok
    ));
    assert!(matches!(
        c.request(Command::RegisterTag { group: 2, name: "P1".into() }),
        Response::Ok
    ));
    let t = shim.call_ok("get_ticket", json!({}));
    assert_eq!(t["tags"], json!([{"name": "BUG", "group": 1}]));
    assert_eq!(
        t["allowed_tags"],
        json!([{"name": "BUG", "group": 1}, {"name": "FEAT", "group": 1}, {"name": "P1", "group": 2}])
    );

    // ---- no tool reads a session, at any tier ---------------------------
    let listed = shim.call_ok("list_board", json!({}));
    let raw = serde_json::to_string(&listed).unwrap();
    for leak in ["session", "argv", "transcript", "cwd", "pid", "claude_session_id"] {
        assert!(!raw.contains(leak), "list_board leaked {leak:?}: {raw}");
    }
    assert!(raw.contains("decoy"), "the board is genuinely visible");

    // ---- create_ticket: a new card, no session, the agent as its author ---
    let made = shim.call_with_meta(
        "create_ticket",
        json!({"title": "  found: flaky test  ", "description": "# Seen\n\nwhile on the work"}),
        "toolu_create_1",
    );
    assert_eq!(made["column"], "TODO", "no column named means the first column");
    assert_eq!(made["replayed"], false);
    let new_key = made["key"].as_str().unwrap().to_string();
    assert!(new_key.starts_with("T-"), "a key, not an id: {new_key}");
    let board = board_of(c.request(Command::Snapshot));
    let new = board.tickets.iter().find(|t| t.short_key == new_key).expect("the ticket exists");
    assert_eq!(new.title, "found: flaky test", "trimmed, as typed");
    assert_eq!(new.column, "TODO");
    assert_eq!(new.notes.len(), 1, "the description is the first note");
    assert_eq!(new.notes[0].created_by, format!("agent:{sid}"));
    assert!(board.sessions.iter().all(|s| s.ticket != new.id), "a created ticket has no session");
    // The caller's own binding did not move.
    assert_eq!(shim.call_ok("get_ticket", json!({}))["key"], key.as_str());
    // A retry under the same tool-use id is the first receipt, not a second card.
    let again = shim.call_with_meta(
        "create_ticket",
        json!({"title": "  found: flaky test  "}),
        "toolu_create_1",
    );
    assert_eq!(again["key"], new_key.as_str());
    assert_eq!(again["replayed"], true);
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.tickets.iter().filter(|t| t.title == "found: flaky test").count(), 1);
    // A named column is honoured; a column that is not there is refused, and
    // the refusal is an answer the model can read rather than a transport error.
    let in_review = shim.call_ok("create_ticket", json!({"title": "second", "column": "REVIEW"}));
    assert_eq!(in_review["column"], "REVIEW");
    let refused = shim.call_err("create_ticket", json!({"title": "third", "column": "NOPE"}));
    assert!(refused.contains("no such column"), "{refused}");
    assert!(shim.call_err("create_ticket", json!({})).contains("title"));

    // ---- the default column (T-279): where an unplaced card lands -------
    // A person chooses it in Settings; an agent may not (it would be choosing
    // what the user sees first). Once chosen, an omitted column means that
    // one; a named column is still honoured; a rename carries it; deleting
    // the column puts the first column back. On a column of its own, so the
    // board the rest of this test walks keeps its shape.
    assert!(matches!(
        c.request(Command::AddColumn { name: "INBOX".into(), after: None }),
        Response::Ok
    ));
    assert!(matches!(
        c.send(
            Principal::Agent { session: sid },
            Command::SetDefaultColumn { column: Some("INBOX".into()) }
        ),
        Response::Err { .. }
    ));
    assert!(matches!(
        c.request(Command::SetDefaultColumn { column: Some("NOPE".into()) }),
        Response::Err { .. }
    ));
    assert!(matches!(
        c.request(Command::SetDefaultColumn { column: Some("INBOX".into()) }),
        Response::Ok
    ));
    let landed = shim.call_ok("create_ticket", json!({"title": "unplaced"}));
    assert_eq!(landed["column"], "INBOX", "no column named means the chosen default");
    let placed = shim.call_ok("create_ticket", json!({"title": "placed", "column": "TODO"}));
    assert_eq!(placed["column"], "TODO", "a named column still wins");
    assert!(matches!(
        c.request(Command::RenameColumn { name: "INBOX".into(), to: "LATER".into() }),
        Response::Ok
    ));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.default_column.as_deref(), Some("LATER"), "a rename carries the default");
    assert_eq!(shim.call_ok("create_ticket", json!({"title": "after rename"}))["column"], "LATER");
    let board = board_of(c.request(Command::Snapshot));
    for t in board.tickets.iter().filter(|t| t.column == "LATER") {
        assert!(matches!(
            c.request(Command::MoveTicket { id: t.id, column: "TODO".into(), before: None }),
            Response::Ok
        ));
    }
    assert!(matches!(c.request(Command::DeleteColumn { name: "LATER".into() }), Response::Ok));
    let board = board_of(c.request(Command::Snapshot));
    assert_eq!(board.default_column, None, "the default goes with its column");
    assert_eq!(
        shim.call_ok("create_ticket", json!({"title": "after delete"}))["column"],
        "TODO",
        "back to the first column"
    );
    // `None` is the first column by choice, and the board's word is the same.
    assert!(matches!(c.request(Command::SetDefaultColumn { column: None }), Response::Ok));
    assert_eq!(board_of(c.request(Command::Snapshot)).landing_column().as_deref(), Some("TODO"));

    // ---- create_ticket wears tags, by name, from the registry -----------
    // Names resolve case-insensitively when that is unambiguous, land on
    // their registry group, and a repeat is one tag, not a refusal.
    let tagged =
        shim.call_ok("create_ticket", json!({"title": "tagged", "tags": ["bug", "P1", "BUG"]}));
    let board = board_of(c.request(Command::Snapshot));
    let made = board.tickets.iter().find(|t| t.short_key == tagged["key"]).expect("minted");
    let worn: Vec<(u8, &str)> = made.tags.iter().map(|r| (r.group, r.name.as_str())).collect();
    assert_eq!(worn, [(1, "BUG"), (2, "P1")]);
    assert_eq!(board.tags.len(), 3, "the registry is the human's; nothing was added to it");
    // A name the board does not know is refused, not minted, and the refusal
    // says where the real names are. Two names on one group are refused too.
    // Either way no card is left behind.
    let before = board.tickets.len();
    let unknown = shim.call_err("create_ticket", json!({"title": "x", "tags": ["NOPE"]}));
    assert!(unknown.contains("no such tag") && unknown.contains("allowed_tags"), "{unknown}");
    let clash = shim.call_err("create_ticket", json!({"title": "x", "tags": ["BUG", "FEAT"]}));
    assert!(clash.contains("one tag per group"), "{clash}");
    let shaped = shim.call_err("create_ticket", json!({"title": "x", "tags": "BUG"}));
    assert!(shaped.contains("array"), "{shaped}");
    assert_eq!(board_of(c.request(Command::Snapshot)).tickets.len(), before, "nothing minted");

    // ---- tag_ticket: the user's vocabulary, worn but never written -------
    // Two more names, the same word on two axes, so the ambiguity has a case.
    for (group, name) in [(3u8, "frontend"), (4u8, "frontend")] {
        assert!(matches!(
            c.request(Command::RegisterTag { group, name: name.into() }),
            Response::Ok
        ));
    }
    let registry_before = board_of(c.request(Command::Snapshot)).tags.clone();
    assert_eq!(registry_before.len(), 5);
    let t = shim.call_ok("get_ticket", json!({}));
    assert_eq!(t["tags"], json!([{"name": "BUG", "group": 1}]), "what the person put on");
    assert_eq!(t["allowed_tags"].as_array().unwrap().len(), 5);
    // One per axis: the groupmate comes off, and the receipt says so. The
    // agent's spelling is not what is stored — the registry's is.
    let worn = shim.call_ok("tag_ticket", json!({"name": "feat"}));
    assert_eq!(worn["tags"], json!([{"name": "FEAT", "group": 1}]));
    assert_eq!(worn["replaced"], "BUG");
    assert!(board_of(c.request(Command::Snapshot)).ticket(ticket).unwrap().wears(1, "FEAT"));
    // Wearing what is worn: no change, no error, no groupmate named.
    let worn = shim.call_ok("tag_ticket", json!({"name": "FEAT"}));
    assert_eq!(worn["tags"], json!([{"name": "FEAT", "group": 1}]));
    assert!(worn["replaced"].is_null());
    // A name on two axes needs the axis said; said, it lands.
    let msg = shim.call_err("tag_ticket", json!({"name": "frontend"}));
    assert!(msg.contains("(3, 4)") && msg.contains("group"), "{msg}");
    let worn = shim.call_ok("tag_ticket", json!({"name": "frontend", "group": 3}));
    assert_eq!(
        worn["tags"],
        json!([{"name": "FEAT", "group": 1}, {"name": "frontend", "group": 3}])
    );
    // A word the user never chose is refused, not registered; so is a real
    // word on the wrong axis.
    let msg = shim.call_err("tag_ticket", json!({"name": "invented-by-agent"}));
    assert!(msg.contains("no such tag") && msg.contains("allowed_tags"), "{msg}");
    let msg = shim.call_err("tag_ticket", json!({"name": "bug", "group": 2}));
    assert!(msg.contains("in group 2"), "{msg}");
    // Taking one off, and taking it off again: idempotent, no error.
    let worn = shim.call_ok("tag_ticket", json!({"name": "FEAT", "remove": true}));
    assert_eq!(worn["tags"], json!([{"name": "frontend", "group": 3}]));
    let worn = shim.call_ok("tag_ticket", json!({"name": "FEAT", "remove": true}));
    assert_eq!(worn["tags"], json!([{"name": "frontend", "group": 3}]));
    // Through all of it the registry did not move: same five, same order.
    assert_eq!(board_of(c.request(Command::Snapshot)).tags, registry_before);

    // ---- the never-tier, on the wire ------------------------------------
    // Not "there is no tool for it" — the daemon refuses the command even when
    // it is handed one directly, which is what makes the tool list a summary
    // of the policy rather than the policy itself.
    let agent = Principal::Agent { session: sid };
    for forbidden in [
        Command::SpawnSession {
            ticket,
            kind: SessionKind::Bash,
            submit_prompt: false,
            plan: false,
        },
        Command::KillSession { id: sid },
        Command::DeleteTicket { id: ticket, discard_worktree: true },
        Command::ArchiveTicket { id: ticket },
        Command::RenameTicket { id: ticket, title: "hijacked".into() },
        Command::MoveTicket { id: ticket, column: "DONE".into(), before: None },
        // The human's tag commands: this one registers on the fly, and the
        // registry is the user's.
        Command::SetTag { id: ticket, group: 1, name: Some("hijacked".into()) },
        Command::RegisterTag { group: 4, name: "hijacked".into() },
        // Where its own cards land by default (T-279): the user's to choose.
        Command::SetDefaultColumn { column: Some("DONE".into()) },
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
    // The rules follow the column, not the ticket: in REVIEW it is the
    // turn's start that moves it, and the end of a turn does nothing.
    assert_eq!(
        shim.call_ok("get_ticket", json!({}))["automove"],
        json!({"on_working": "IN PROGRESS", "on_done": null})
    );

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
