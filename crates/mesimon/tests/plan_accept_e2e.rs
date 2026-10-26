//! The board accepts an agent's plan (T-420), end to end: the stub paints
//! Claude Code's plan dialog at its default row, the hook lands the `≡`,
//! and a `PromptSession` with `accept_plan` — blank, or with words — makes
//! the daemon press ONE Enter into the pane. `read` returns on a newline,
//! so a `got:` line (or `got:planned `, the spawn's title prefill submitted
//! by the first Enter) proves the Enter and nothing else went. The words go
//! in the moment the harness confirms the press; a pane at `Plan` with no dialog
//! the daemon recognises gives the flag up after its tries and says so in
//! the feed, and the words keep waiting as an unflagged ask does.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{Reason, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

/// Claude Code's dialog as the pane shows it, then every line read echoed
/// as `got:<line>` — an Enter alone is `got:`. A line reading `clear`
/// wipes the screen, so the same pane can then show NO dialog.
const STUB: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
printf ' Would you like to proceed?\\n\\n ❯ 1. Yes, and use auto mode\\n   2. Yes, manually approve edits\\n   3. No, keep planning\\n   4. Tell Claude what to change\\n'\n\
cat | while IFS= read -r line; do \
  case \"$line\" in clear) printf '\\033[2J\\033[H';; esac; \
  printf 'got:%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

#[test]
fn the_board_accepts_a_plan_with_one_enter_and_the_words_ride_the_approval() {
    let Some(h) = Harness::boot_with_env(
        "planok",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let mut c = h.client("planok");
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    // An Enter of the daemon's submits whatever sits in the box: the title
    // the spawn typed as a prefill (T-224) the first time, nothing after.
    // A paste brings its own Enter and its own words, so those lines are
    // not presses.
    let presses =
        || text().lines().filter(|l| matches!(*l, "got:" | "got:planned " | "got:planned")).count();

    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "planned".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();
    let plan = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##;
    let start = |c: &mut TestClient| {
        hook_send(&hooks, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient| {
        hook_send(&hooks, &session, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let plan_ready = |c: &mut TestClient| {
        hook_send(&hooks, &session, "PreToolUse", plan);
        c.await_state(sid, "plan ready", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Plan }
        });
    };
    let approved = |c: &mut TestClient| {
        hook_send(&hooks, &session, "PostToolUse", plan);
        c.await_state(sid, "running on the plan", |s| *s == SessionState::Running);
    };
    // Let the stub paint before anything reads the screen.
    std::thread::sleep(Duration::from_millis(800));
    start(&mut c);
    stop(&mut c);

    // (1) The `≡` is up: a blank accept is one Enter, and the entry leaves
    // the queue with it — nothing was asked. The harness confirms it
    // through its own hook, and the feed names the press.
    plan_ready(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: String::new(),
            queued: false,
            immediately: false,
            accept_plan: true,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    wait_until(Duration::from_secs(10), "the accept's Enter", || presses() == 1);
    wait_until(Duration::from_secs(5), "the queue to empty", || {
        pending_of(&mut c, Some(ticket)).is_empty()
    });
    assert_eq!(text(), "got:planned \n", "one Enter and nothing else went: {:?}", text());
    approved(&mut c);
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"plan_accepted""#)
    });
    stop(&mut c);

    // (2) Queued while the agent still works, with the flag: nothing goes
    // until the plan is ready; then the Enter, and the words the moment
    // the harness confirms it — at the head of the approved turn, not on
    // the idle after.
    start(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-420 then do this".into(),
            queued: true,
            immediately: false,
            accept_plan: true,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| p.accept_plan && !p.in_flight), "{p:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(), 1, "no plan yet, no Enter");
    plan_ready(&mut c);
    wait_until(Duration::from_secs(10), "the second accept's Enter", || presses() == 2);
    assert!(!text().contains("mesimon-probe-420"), "the words wait for the approval: {:?}", text());
    assert!(
        pending_of(&mut c, Some(ticket)).iter().any(|p| p.accept_plan),
        "the row says accepting until the harness confirms"
    );
    approved(&mut c);
    wait_until(Duration::from_secs(10), "the words at the head of the approved turn", || {
        text().contains("got:mesimon-probe-420 then do this")
    });
    assert_eq!(presses(), 2, "the paste's own Enter rides the paste: {:?}", text());
    wait_until(Duration::from_secs(5), "the feed line", || {
        feed().contains(r#""cmd":"queued_ask_sent_after_plan""#)
    });
    // The paste is acked by the next prompt hook, then the turn ends.
    start(&mut c);
    wait_until(Duration::from_secs(5), "the ack", || pending_of(&mut c, Some(ticket)).is_empty());
    stop(&mut c);

    // (3) A pane at `Plan` showing no dialog the daemon recognises: the
    // flag is given up after its tries, the feed says so, and the words
    // stay queued the way an unflagged ask does — behind the `≡`.
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "clear".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Ok
    ));
    wait_until(Duration::from_secs(5), "the screen to clear", || text().contains("got:clear"));
    start(&mut c);
    stop(&mut c);
    plan_ready(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-421 unrecognised".into(),
            queued: true,
            immediately: false,
            accept_plan: true,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    wait_until(Duration::from_secs(25), "the flag to be given up", || {
        feed().contains(r#""cmd":"plan_accept_unrecognised""#)
    });
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| !p.accept_plan && !p.in_flight), "{p:?}");
    assert_eq!(presses(), 2, "no blind Enter: {:?}", text());
    assert!(!text().contains("mesimon-probe-421"));

    let _ = c.request(Command::Shutdown);
}

/// The dialog again, each line echoed with the pane's ticket key in front
/// (`MESIMON_TICKET`, which every spawn gets), so two panes writing one
/// file can be told apart.
const STUB_KEYED: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\n\
printf ' Would you like to proceed?\\n\\n ❯ 1. Yes, and use auto mode\\n   2. Yes, manually approve edits\\n   3. No, keep planning\\n   4. Tell Claude what to change\\n'\n\
cat | while IFS= read -r line; do \
  printf 'got:%s:%s\\n' \"$MESIMON_TICKET\" \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// T-429: accepts obey the checkout. Two shared-checkout tickets, both on
/// their plan dialogs, both accepted from the board: ONE Enter lands — the
/// first in board order — and the second's row names the first
/// (`accepts plan ∙ after T-1`). The second press goes only when the first
/// agent's approved turn has ended, never while it implements. Then the
/// column's own accept: `PromptColumn` with the flag parks both, and the
/// presses go the same way, the first now and the other as the checkout
/// goes quiet. Worktree tickets are unaffected — their checkout is their
/// own — and that is `checkout_holders`' cwd clause, not tested again here.
#[test]
fn accepts_go_one_per_quiet_checkout_and_a_column_accepts_every_plan() {
    let Some(h) = Harness::boot_with_env(
        "planq",
        Some(STUB_KEYED),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let mut c = h.client("planq");
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();

    let mut make = |title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let (ta, tb) = (make("planA"), make("planB"));
    let board = c.board();
    let key = |t: ulid::Ulid| board.ticket(t).unwrap().short_key.clone();
    let (ka, kb) = (key(ta), key(tb));
    // An Enter of the daemon's submits whatever the box holds: the title
    // prefill the first time, nothing after. Counted per pane.
    let presses = |k: &str| {
        let own = format!("got:{k}:");
        text()
            .lines()
            .filter(|l| {
                l.strip_prefix(own.as_str()).is_some_and(|rest| {
                    matches!(rest, "" | "planA" | "planA " | "planB" | "planB ")
                })
            })
            .count()
    };
    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let (sa, sb) = (spawn(&mut c, ta), spawn(&mut c, tb));
    let plan = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##;
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let plan_ready = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "PreToolUse", plan);
        c.await_state(sid, "plan ready", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Plan }
        });
    };
    let approved = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hooks, &sid.to_string(), "PostToolUse", plan);
        c.await_state(sid, "running on the plan", |s| *s == SessionState::Running);
    };
    let accept = |c: &mut TestClient, ticket| {
        assert!(matches!(
            c.request(Command::PromptSession {
                ticket,
                text: String::new(),
                queued: false,
                immediately: false,
                accept_plan: true,
                plan: false,
                tier: None,
                resend: false,
                uploads: Vec::new(),
            }),
            Response::Queued { .. }
        ));
    };
    // Let both stubs paint, then walk each through a turn so the checkout
    // starts quiet (a stub emits no `SessionStart`, so a fresh record sits
    // at `Spawning`, which is WORKING).
    std::thread::sleep(Duration::from_millis(800));
    for sid in [sa, sb] {
        start(&mut c, sid);
        stop(&mut c, sid);
    }
    // One column, A above B: the presses go in board order.
    let column = c.board().ticket(ta).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: tb, column: column.clone(), before: None }),
        Response::Ok
    ));

    // (1) Both dialogs up, both accepted from the board. A's Enter goes;
    // B's waits on A and its row says so.
    plan_ready(&mut c, sa);
    plan_ready(&mut c, sb);
    accept(&mut c, ta);
    accept(&mut c, tb);
    wait_until(Duration::from_secs(10), "A's Enter", || presses(&ka) == 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 0, "B waits on A's press in flight: {:?}", text());
    let pb = pending_of(&mut c, Some(tb));
    assert!(pb.iter().any(|p| p.accept_plan && p.waits_on == vec![ka.clone()]), "{pb:?}");
    // A's approval is confirmed and A implements: still B's checkout is
    // held, by a working agent now.
    approved(&mut c, sa);
    wait_until(Duration::from_secs(5), "A's entry to leave the queue", || {
        pending_of(&mut c, Some(ta)).is_empty()
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 0, "B waits while A implements: {:?}", text());
    let pb = pending_of(&mut c, Some(tb));
    assert!(pb.iter().any(|p| p.accept_plan && p.waits_on == vec![ka.clone()]), "{pb:?}");
    // A's turn ends: B's press goes.
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "B's Enter after A's turn", || presses(&kb) == 1);
    assert_eq!(presses(&ka), 1, "A was pressed once: {:?}", text());
    approved(&mut c, sb);
    wait_until(Duration::from_secs(5), "B's entry to leave the queue", || {
        pending_of(&mut c, Some(tb)).is_empty()
    });
    stop(&mut c, sb);

    // (2) The column accepts every plan (T-429): both on their dialogs
    // again, one `PromptColumn` with the flag and no words parks both —
    // the receipt counts them — and the presses go as before: A's now, B's
    // when A's approved turn has ended.
    plan_ready(&mut c, sa);
    plan_ready(&mut c, sb);
    // Automove carried both cards on as their turns ended; put B under A
    // again so the order is the test's, not the automove's.
    let column = c.board().ticket(ta).unwrap().column.clone();
    assert!(matches!(
        c.request(Command::MoveTicket { id: tb, column: column.clone(), before: None }),
        Response::Ok
    ));
    match c.request(Command::PromptColumn {
        column: column.clone(),
        text: String::new(),
        queued: true,
        accept_plan: true,
    }) {
        Response::Asked { accepts, sent, woke, started, skipped, failed, .. } => {
            assert_eq!((accepts, sent, woke, started, skipped, failed), (2, 0, 0, 0, 0, 0));
        }
        other => panic!("{other:?}"),
    }
    wait_until(Duration::from_secs(10), "A's second Enter", || presses(&ka) == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(presses(&kb), 1, "B waits on A again: {:?}", text());
    approved(&mut c, sa);
    stop(&mut c, sa);
    wait_until(Duration::from_secs(10), "B's second Enter", || presses(&kb) == 2);
    approved(&mut c, sb);
    wait_until(Duration::from_secs(5), "the queue to empty", || {
        pending_of(&mut c, None).is_empty()
    });
    // No Enter went anywhere it was not owed.
    assert_eq!((presses(&ka), presses(&kb)), (2, 2), "{:?}", text());

    let _ = c.request(Command::Shutdown);
}

/// One Claude seat on a stub that only sleeps, with the transcript its
/// `SessionStart` names (T-957): the test writes what Claude Code writes,
/// and sends what its hooks send.
struct Bed {
    h: Harness,
    c: TestClient,
    ticket: ulid::Ulid,
    sid: uuid::Uuid,
    transcript: std::path::PathBuf,
}

const REFUSED: &str = "The user doesn't want to proceed with this tool use. The tool use was \
                       rejected (eg. if it was a file edit, the new_string was NOT written to the \
                       file). To tell you how to proceed, the user said: keep planning.";

impl Bed {
    fn boot(name: &str) -> Option<Self> {
        let h = Harness::boot_with_env(
            name,
            Some("#!/bin/sh\nexec sleep 120\n"),
            &[("MESIMON_PANE_QUIET_MS", "600000")],
        )?;
        let mut c = h.client(name);
        let ticket = match c.request(Command::CreateTicket {
            column: "TODO".into(),
            title: "refused".into(),
            workspace: None,
            tier: None,
        }) {
            Response::Created { id, .. } => id,
            other => panic!("{other:?}"),
        };
        let sid = match c.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Claude,
            submit_prompt: false,
            plan: false,
        }) {
            Response::Spawned { id, .. } => id,
            other => panic!("spawn failed: {other:?}"),
        };
        let transcript = h.dir.join(format!("{name}.jsonl"));
        let bed = Bed { h, c, ticket, sid, transcript };
        bed.append(serde_json::json!({"uuid": "p", "type": "user", "message": {"content": "go"}}));
        Some(bed)
    }

    fn hook(&self, event: &str, body: &str) {
        hook_send(&self.h.paths.hook_sock(), &self.sid.to_string(), event, body);
    }

    /// The transcript named, the seat at rest, then running.
    fn start(&mut self) {
        let start = serde_json::json!({"session_id": "x", "transcript_path": self.transcript,
            "cwd": "/tmp"});
        let sock = self.h.paths.hook_sock();
        hook_send_with(
            &sock,
            &self.sid.to_string(),
            "SessionStart",
            Some("startup"),
            &start.to_string(),
        );
        self.c.await_state(self.sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
        self.prompt();
    }

    fn prompt(&mut self) {
        self.hook("UserPromptSubmit", r#"{"prompt":"go"}"#);
        self.c.await_state(self.sid, "running", |s| *s == SessionState::Running);
    }

    fn append(&self, record: serde_json::Value) {
        self.append_all(&[record]);
    }

    /// `records` in one write, so no tail read falls between them.
    fn append_all(&self, records: &[serde_json::Value]) {
        use std::io::Write;
        let text: String = records.iter().map(|r| format!("{r}\n")).collect();
        let mut f =
            std::fs::OpenOptions::new().create(true).append(true).open(&self.transcript).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn input(tool: &str) -> serde_json::Value {
        match tool {
            "ExitPlanMode" => serde_json::json!({"plan": "# Plan"}),
            _ => serde_json::json!({"questions": [{"question": "Which?", "header": "Which",
                "options": [{"label": "A", "description": ""}, {"label": "B", "description": ""}],
                "multiSelect": false}]}),
        }
    }

    /// The call written, as the engine writes it before its hook.
    fn call(&self, tool: &str, id: &str) {
        self.append(serde_json::json!({"uuid": format!("c-{id}"), "type": "assistant",
            "message": {"content": [{"type": "tool_use", "id": id, "name": tool,
                "input": Self::input(tool)}]}}));
    }

    /// The dialog's `PreToolUse`, and the seat stopped on it.
    fn stop_on(&mut self, tool: &str, id: &str) {
        let reason = if tool == "ExitPlanMode" { Reason::Plan } else { Reason::Question };
        let frame = serde_json::json!({"tool_name": tool, "tool_use_id": id, "tool_input": Self::input(tool)});
        self.hook("PreToolUse", &frame.to_string());
        self.c
            .await_state(self.sid, "dialog up", |s| *s == SessionState::RequiresAction { reason });
    }

    /// A one-question `AskUserQuestion` input asking `q`.
    fn ask(q: &str) -> serde_json::Value {
        serde_json::json!({"questions": [{"question": q, "header": "Which",
            "options": [{"label": "A", "description": ""}, {"label": "B", "description": ""}],
            "multiSelect": false}]})
    }

    /// A hook body for `tool`'s call, naming `id` when given.
    fn frame(tool: &str, id: Option<&str>, input: &serde_json::Value) -> String {
        let mut f = serde_json::json!({"tool_name": tool, "tool_input": input});
        if let Some(id) = id {
            f["tool_use_id"] = serde_json::json!(id);
        }
        f.to_string()
    }

    /// A call's line as the engine writes it, under its message's `id`.
    fn line(message: &str, tool: &str, id: &str, input: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({"uuid": format!("c-{id}"), "type": "assistant", "message": {
            "id": message, "content": [{"type": "tool_use", "id": id, "name": tool,
                "input": input}]}})
    }

    /// The interrupt record an Esc writes.
    fn esc() -> serde_json::Value {
        serde_json::json!({"uuid": "esc", "type": "user", "timestamp": now_iso(),
            "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}})
    }

    /// The call the record is held on, as persisted.
    fn held_call(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.h.paths.sessions_file()).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        v["sessions"].as_array()?.iter().find(|s| s["id"] == self.sid.to_string())?["dialog_call"]
            .as_str()
            .map(String::from)
    }

    /// Two questions in one message (T-988), `toolu_a` "Which colour?" and
    /// `toolu_b` "Which size?" under `msg_1`, their lines written first.
    /// `reversed`: B's `PreToolUse` is heard first, so A is held. Returns
    /// once the seat is on the questions, the expected call is held and both
    /// requests are decided.
    fn two_questions(&mut self, reversed: bool) {
        let (qa, qb) = (Self::ask("Which colour?"), Self::ask("Which size?"));
        self.append_all(&[
            Self::line("msg_1", "AskUserQuestion", "toolu_a", &qa),
            Self::line("msg_1", "AskUserQuestion", "toolu_b", &qb),
        ]);
        self.two_questions_heard(reversed);
    }

    /// `two_questions`' hooks alone, the lines left to the caller.
    fn two_questions_heard(&mut self, reversed: bool) {
        let (qa, qb) = (Self::ask("Which colour?"), Self::ask("Which size?"));
        let (a, b) = (
            Self::frame("AskUserQuestion", Some("toolu_a"), &qa),
            Self::frame("AskUserQuestion", Some("toolu_b"), &qb),
        );
        let decided =
            self.decisions().iter().filter(|d| d["source"] == "PermissionRequest").count();
        let (first, second) = if reversed { (&b, &a) } else { (&a, &b) };
        self.hook("PreToolUse", first);
        self.hook("PreToolUse", second);
        self.hook("PermissionRequest", &Self::frame("AskUserQuestion", None, &qa));
        self.hook("PermissionRequest", &Self::frame("AskUserQuestion", None, &qb));
        let question = SessionState::RequiresAction { reason: Reason::Question };
        self.c.await_state(self.sid, "on the questions", |s| *s == question);
        let held = if reversed { "toolu_a" } else { "toolu_b" };
        wait_until(Duration::from_secs(10), "the held call named, both requests decided", || {
            self.held_call().as_deref() == Some(held)
                && self.decisions().iter().filter(|d| d["source"] == "PermissionRequest").count()
                    >= decided + 2
        });
    }

    fn result(&self, id: &str, error: bool, text: &str) {
        self.append(Self::result_of(id, error, text));
    }

    fn result_of(id: &str, error: bool, text: &str) -> serde_json::Value {
        serde_json::json!({"uuid": format!("r-{id}"), "type": "user",
            "timestamp": now_iso(), "message": {"content": [{"type": "tool_result",
                "tool_use_id": id, "is_error": error, "content": text}]}})
    }

    fn state(&mut self) -> Option<SessionState> {
        board_of(self.c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == self.sid)
            .map(|s| s.state.clone())
    }

    fn column(&mut self) -> String {
        board_of(self.c.request(Command::Snapshot)).ticket(self.ticket).unwrap().column.clone()
    }

    /// The machine's decisions for the seat, from the feed.
    fn decisions(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["kind"] == "state_decision" && v["session"] == self.sid.to_string())
            .collect()
    }

    fn departures(&self) -> Vec<serde_json::Value> {
        self.decisions().into_iter().filter(|v| v["source"] == "dialog_result").collect()
    }
}

/// Now, as Claude Code stamps a transcript record.
fn now_iso() -> String {
    iso_ago(0)
}

/// `ms` milliseconds before now, as Claude Code stamps a transcript record.
fn iso_ago(ago: u64) -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
        as u64
        - ago;
    let (secs, milli) = (ms / 1000, ms % 1000);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant's algorithm), UTC.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// T-447 (2026-09-23): a person's refusal of a plan or a question in the
/// pane ("No, keep planning", the words on `Tell Claude what to change`, an
/// Esc) fires no hook of its own — no `PostToolUse`, no `PostToolUseFailure`,
/// no `PermissionDenied`. The card wore "plan" through four tool completions.
/// T-447 cleared it on the agent's next `PreToolUse`, a frame neither the
/// hook set nor the classic mod sends for an ordinary tool (T-957); what
/// the engine writes is the dialog's own result in the transcript, at the
/// keypress (2.1.310), and that is the departure. A subagent's completion
/// and a sibling's result in the transcript are not the lead's answer and
/// leave the dialog held (T-595).
#[test]
fn a_refused_dialog_clears_on_its_own_result_in_the_transcript() {
    let Some(mut b) = Bed::boot("planno") else { return };
    b.start();
    for (tool, id, reason) in [
        ("ExitPlanMode", "toolu_plan", Reason::Plan),
        ("AskUserQuestion", "toolu_ask", Reason::Question),
    ] {
        // The call beside a sibling, as the engine writes them, then the
        // dialog's hook.
        let sibling = format!("{id}_sibling");
        b.call("Read", &sibling);
        b.call(tool, id);
        b.stop_on(tool, id);
        // A subagent's tool ending, and the sibling's result: not the answer.
        b.hook(
            "PostToolUse",
            r#"{"tool_name":"Read","tool_input":{"file_path":"/x"},"agent_id":"sub-1"}"#,
        );
        b.result(&sibling, false, "x");
        std::thread::sleep(Duration::from_millis(4000));
        assert_eq!(
            b.state(),
            Some(SessionState::RequiresAction { reason }),
            "a subagent's call or a sibling's result left the dialog"
        );
        // The refusal: no frame, the dialog's own result on the transcript.
        b.result(id, true, REFUSED);
        b.c.await_state(b.sid, "working after the refusal", |s| *s == SessionState::Running);
    }
}

/// T-957 as Claude Code sends it (T-1014): the rig's RF1 on 2.1.310, a
/// Sonnet worker in plan mode on the mod road whose plan the person refuses
/// with Esc in the pane, replayed frame for frame and line for line. No hook
/// fires for the Esc. Unlike the sequence written by hand above, the plan's
/// call is not on disk while its dialog is up: the engine writes that
/// message's lines (the plan file's `Write`, then the call) at the Esc, in
/// the same millisecond as the call's refused result ("… STOP what you are
/// doing …", `toolDenialKind: user-rejected`), the interrupt and the turn's
/// close. So the departure is read from an ended call (`closed_dialog`), and
/// the seat rests interrupted.
#[test]
fn a_plan_refused_with_esc_leaves_on_its_own_result_as_recorded() {
    let trace = trace::Trace::load("t957-plan-refused-esc");
    let Some(mut b) = Bed::boot("planno_rec") else { return };
    let plan = trace
        .frames()
        .into_iter()
        .find(|(e, _, p)| *e == "PreToolUse" && p["tool_name"] == "ExitPlanMode")
        .map(|(_, _, p)| p["tool_use_id"].as_str().unwrap().to_string())
        .expect("the plan's call");
    let sock = b.h.paths.hook_sock();
    let mut r = trace.replay(&sock, b.sid, &b.transcript);
    r.through(trace::frame("UserPromptSubmit", None));
    b.c.await_state(b.sid, "running", |s| *s == SessionState::Running);
    // The model's first seconds: a tail read while running mints the cursor.
    std::thread::sleep(Duration::from_millis(2500));
    let on_plan = SessionState::RequiresAction { reason: Reason::Plan };
    r.through(trace::frame("PermissionRequest", Some("ExitPlanMode")));
    b.c.await_state(b.sid, "on the plan", |s| *s == on_plan);
    assert_eq!(b.held_call().as_deref(), Some(plan.as_str()), "the plan's call is held");
    assert!(
        !std::fs::read_to_string(&b.transcript).unwrap().contains(&plan),
        "the call's line is not on disk while its dialog stands"
    );
    // A dialog standing is not left by the clock.
    std::thread::sleep(Duration::from_millis(3000));
    assert_eq!(b.state(), Some(on_plan.clone()), "the plan stands");

    // ---- the Esc: no frame, the message's lines and the refusal --------------
    r.rest();
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    b.c.await_state(b.sid, "interrupted off the plan", |s| *s == interrupted);
    let left = b
        .departures()
        .into_iter()
        .any(|d| d["decision"]["before"]["state"] == serde_json::to_value(&on_plan).unwrap());
    assert!(left, "the plan's own result decided it: {:?}", b.decisions());
}

/// T-447's own road, kept (T-957): the native road (T-657) relays every
/// `tool.call` as a `PreToolUse`, so there the agent's next call is the
/// refusal's first frame, and no transcript result is written here for
/// anything to read instead. A subagent's call (`agent_id`) is not the
/// lead's answer and leaves the dialog held; the lead's clears it.
#[test]
fn a_refused_dialog_clears_on_the_native_road_s_next_tool_call() {
    let Some(mut b) = Bed::boot("planno_native") else { return };
    b.prompt();
    for (tool, id, reason) in [
        ("ExitPlanMode", "toolu_plan", Reason::Plan),
        ("AskUserQuestion", "toolu_ask", Reason::Question),
    ] {
        b.stop_on(tool, id);
        let sub = r#"{"tool_name":"Read","tool_input":{"file_path":"/x"},"agent_id":"sub-1"}"#;
        b.hook("PreToolUse", sub);
        b.hook("PostToolUse", sub);
        std::thread::sleep(Duration::from_millis(2000));
        assert_eq!(b.state(), Some(SessionState::RequiresAction { reason }), "a subagent's call");
        b.hook("PreToolUse", r#"{"tool_name":"Bash","tool_input":{"command":"grep -n x"}}"#);
        b.c.await_state(b.sid, "working after the refusal", |s| *s == SessionState::Running);
    }
}

/// T-957 round 1: the dialog's result is read on the tail's next poll, up to
/// 2 s after it was written, and a fast agent's `Stop` can come first: the
/// projected dialog drops with it and the record settles to the turn's end.
/// The result read inside that settle must not turn it back to `Running`:
/// the stated end stands, and the ticket moves on it (REVIEW). For a
/// refusal and for an accept. The poll's phase is not the test's to set,
/// so a case repeats, each try staggered against the poll's 2 s, until the
/// feed shows the result read inside the `Stop`'s settle (three in four do).
#[test]
fn a_stop_heard_before_the_dialog_s_result_is_read_keeps_its_turn_end() {
    let Some(mut b) = Bed::boot("planstop") else { return };
    b.start();
    let stop = r#"{"stop_hook_active":false}"#;
    let mut n = 0;
    // A shared runner's tail polls land inside the settle in two tries of
    // eleven, not three in four (T-1019, macos-14): more tries there.
    let tries = if std::env::var_os("MESIMON_CI").is_some() { 24 } else { 8 };
    for (error, text) in [(true, REFUSED), (false, "User has approved your plan.")] {
        let mut met = false;
        for _ in 0..tries {
            n += 1;
            let id = format!("toolu_s{n}");
            b.call("ExitPlanMode", &id);
            b.stop_on("ExitPlanMode", &id);
            let seen = b.departures().len();
            // A try lasts about three tail polls, so each starts at nearly
            // the same phase of the poll: stagger them to sweep it.
            std::thread::sleep(Duration::from_millis((n * 700 % 2000) as u64));
            b.result(&id, error, text);
            b.hook("Stop", stop);
            b.c.await_state(b.sid, "the turn's end", |s| {
                *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
            });
            wait_until(Duration::from_secs(6), "the move on the turn's end", || {
                b.column() == "REVIEW"
            });
            // Past the settle and two polls: the end stood.
            std::thread::sleep(Duration::from_millis(4500));
            assert_eq!(
                b.state(),
                Some(SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn })
            );
            assert_eq!(b.column(), "REVIEW");
            let departures = b.departures();
            if let Some(d) = departures.get(seen) {
                if d["decision"]["before"]["pending"]["state"]["state"] == "idle" {
                    assert_eq!(
                        d["decision"]["after"]["pending"], d["decision"]["before"]["pending"],
                        "the Stop's leave kept: {d}"
                    );
                    met = true;
                }
            }
            b.prompt();
            if met {
                break;
            }
        }
        assert!(met, "no read of the result landed inside a Stop's settle in {tries} tries");
    }
}

/// T-957's replan guard: the old dialog's result never ends the new one.
/// (1) B's `PreToolUse` lands before A's result is read and before B's own
/// line reaches the file: the hook stream projects B, so A's end is
/// dropped; B's line and then B's result end B. (2) B's `PreToolUse` lands
/// inside the settle A's result started: the leave is cancelled and the
/// record stays on B.
#[test]
fn a_replan_s_hook_keeps_its_dialog_against_the_old_result() {
    let Some(mut b) = Bed::boot("planguard") else { return };
    b.start();
    let plan = SessionState::RequiresAction { reason: Reason::Plan };

    // ---- (1) B's hook first, A's result read after, B's line later --------
    b.call("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_b");
    b.result("toolu_a", true, REFUSED);
    std::thread::sleep(Duration::from_millis(4500));
    assert_eq!(b.state(), Some(plan.clone()), "A's end dropped while B stands");
    assert!(b.departures().is_empty(), "nothing given to the machine: {:?}", b.departures());
    b.call("ExitPlanMode", "toolu_b");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(b.state(), Some(plan.clone()));
    b.result("toolu_b", true, REFUSED);
    b.c.await_state(b.sid, "B's own end", |s| *s == SessionState::Running);

    // ---- (2) B's hook inside the settle A's result started ----------------
    b.call("ExitPlanMode", "toolu_c");
    b.stop_on("ExitPlanMode", "toolu_c");
    let seen = b.departures().len();
    b.result("toolu_c", true, REFUSED);
    wait_until(Duration::from_secs(6), "C's end read", || b.departures().len() > seen);
    let left = std::time::Instant::now();
    b.call("ExitPlanMode", "toolu_d");
    b.stop_on("ExitPlanMode", "toolu_d");
    assert!(left.elapsed() < Duration::from_millis(1500), "inside the settle");
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(b.state(), Some(plan), "the replan inside the settle kept the dialog");
}

/// T-957 round 1: a refusal written while no daemon read the tail. A record
/// on a dialog keeps it across a restart, and the new daemon's cursor
/// starts at the end of the file, past the call and its result; its first
/// look back for the held call finds it ended inside the spell, and the
/// record leaves: running after words, interrupted after an Esc.
#[test]
fn a_refusal_in_a_restart_s_window_is_read_when_the_cursor_starts() {
    let Some(mut b) = Bed::boot("planrestart") else { return };
    b.start();
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    for (n, (tool, id, reason, esc)) in [
        ("ExitPlanMode", "toolu_r1", Reason::Plan, false),
        ("AskUserQuestion", "toolu_r2", Reason::Question, true),
    ]
    .into_iter()
    .enumerate()
    {
        b.call(tool, id);
        b.stop_on(tool, id);
        b.h.restart_after(|| {
            b.result(id, true, REFUSED);
            if esc {
                b.append(serde_json::json!({"uuid": "abort", "type": "user", "message": {
                    "content": [{"type": "text",
                        "text": "[Request interrupted by user for tool use]"}]}}));
            }
        });
        b.c = b.h.client(&format!("planrestart_{n}"));
        let want = if esc { interrupted.clone() } else { SessionState::Running };
        b.c.await_state(b.sid, "off the dialog after the restart", |s| *s == want);
        let held = b.decisions().iter().any(|d| {
            d["source"] == "dialog_result"
                && d["decision"]["before"]["state"]
                    == serde_json::to_value(SessionState::RequiresAction { reason }).unwrap()
        });
        assert!(held, "the dialog's end, read off the tail: {:?}", b.departures());
    }
}

/// Words riding an approval wait while a person has the pane open (T-958,
/// round 1): the harness confirms the accept, the paste's guard refuses the
/// words before any key, and the entry goes back on the queue as an
/// ordinary ask, its one-shot spent. At the agent's idle the person is
/// still there and nothing goes; once they leave, the words go once. The
/// hook road alone: on the mod road the words are a `submit`, no key.
#[test]
fn words_riding_an_approval_wait_for_a_person_to_leave() {
    if test_road() == "mod" {
        return;
    }
    let Some(h) = Harness::boot_with_env(
        "planheld",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let mut c = h.client("planheld");
    let text = || std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default();
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    let presses =
        || text().lines().filter(|l| matches!(*l, "got:" | "got:planned " | "got:planned")).count();
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "planned".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();
    let pane = sid.simple().to_string()[..16].to_string();
    let tmux_sock = h.paths.tmux_sock();
    let plan = r##"{"tool_name":"ExitPlanMode","tool_input":{"plan":"# Plan"}}"##;
    let start = |c: &mut TestClient| {
        hook_send(&hooks, &session, "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient| {
        hook_send(&hooks, &session, "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    std::thread::sleep(Duration::from_millis(800));
    start(&mut c);
    stop(&mut c);

    start(&mut c);
    assert!(matches!(
        c.request(Command::PromptSession {
            ticket,
            text: "mesimon-probe-958 then this".into(),
            queued: true,
            immediately: false,
            accept_plan: true,
            plan: false,
            tier: None,
            resend: false,
            uploads: Vec::new(),
        }),
        Response::Queued { .. }
    ));
    hook_send(&hooks, &session, "PreToolUse", plan);
    c.await_state(sid, "plan ready", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Plan }
    });
    wait_until(Duration::from_secs(10), "the accept's Enter", || presses() == 1);
    // A person opens the pane, and the harness confirms the accept.
    let client = person_attaches(&tmux_sock, &pane);
    hook_send(&hooks, &session, "PostToolUse", plan);
    c.await_state(sid, "running on the plan", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(5), "the words kept", || {
        feed().lines().any(|l| {
            l.contains(r#""cmd":"queued_ask_kept""#)
                && l.contains(r#""outcome":"a_person_attached""#)
        })
    });
    assert!(!feed().contains(r#""cmd":"queued_ask_failed""#), "nothing failed");
    assert!(!text().contains("mesimon-probe-958"), "no key under their hands: {:?}", text());
    let p = pending_of(&mut c, Some(ticket));
    assert!(p.iter().any(|p| !p.accept_plan && !p.in_flight), "kept as an ordinary ask: {p:?}");
    // The agent's idle, the person still there: nothing goes.
    stop(&mut c);
    std::thread::sleep(Duration::from_millis(2000));
    assert!(!text().contains("mesimon-probe-958"), "still theirs: {:?}", text());
    // Gone: the words go once.
    person_leaves(&tmux_sock, &pane, client);
    wait_until(Duration::from_secs(10), "the kept words", || {
        text().contains("got:mesimon-probe-958 then this")
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(text().matches("mesimon-probe-958").count(), 1, "once: {:?}", text());
}

/// T-957 round 2: an Esc on a plan and, at once, the person's next prompt.
/// The interrupt the tail reads ended the turn before the prompt, so it must
/// not end the new one, at High (the dialog's end) or at Low (the hint):
/// with the prompt's line in the same read as the interrupt, and with the
/// prompt's hook beating its line to the file. Unpinned and with the flap
/// pin armed. An Esc with no prompt after it still rests interrupted.
#[test]
fn an_esc_then_a_prompt_at_once_leaves_the_new_turn_running() {
    const STOP: &str = "The user doesn't want to proceed with this tool use. The tool use was \
                        rejected (eg. if it was a file edit, the new_string was NOT written to \
                        the file). STOP what you are doing and wait for the user to tell you \
                        how to proceed.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let esc = |b: &Bed, id: &str| {
        b.result(id, true, STOP);
        b.append(serde_json::json!({"uuid": format!("abort-{id}"), "type": "user",
            "timestamp": now_iso(), "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}));
        b.append(serde_json::json!({"uuid": format!("close-{id}"), "type": "system",
            "subtype": "turn_duration", "durationMs": 1, "timestamp": now_iso()}));
    };
    let prompt_line = |b: &Bed| {
        b.append(serde_json::json!({"uuid": "next", "type": "user", "timestamp": now_iso(),
            "message": {"content": "Do it another way."}}));
    };
    // Whether the dialog's own decision (the last `PreToolUse`'s) left the
    // pin armed.
    let pin_of = |b: &Bed| {
        let dialogs = || -> Vec<serde_json::Value> {
            b.decisions().into_iter().filter(|d| d["source"] == "PreToolUse").collect()
        };
        wait_until(Duration::from_secs(5), "the dialog's decision", || !dialogs().is_empty());
        let d = dialogs().pop().unwrap();
        d["decision"]["after"]["pinned_until"]
            .as_u64()
            .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
    };
    for (pinned, same_read) in [(false, true), (false, false), (true, true), (true, false)] {
        let name = format!("planesc_{}_{}", u8::from(pinned), u8::from(same_read));
        let Some(mut b) = Bed::boot(&name) else { return };
        b.start();
        if pinned {
            // More than four committed changes in 20 s arm the pin.
            for _ in 0..3 {
                b.hook("Stop", r#"{"stop_hook_active":false}"#);
                b.c.await_state(b.sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
                b.prompt();
            }
        }
        b.call("ExitPlanMode", "toolu_e");
        b.stop_on("ExitPlanMode", "toolu_e");
        assert_eq!(pin_of(&b), pinned, "the pin as this case wants it");
        esc(&b, "toolu_e");
        if same_read {
            prompt_line(&b);
        }
        b.hook("UserPromptSubmit", r#"{"prompt":"Do it another way."}"#);
        if !same_read {
            std::thread::sleep(Duration::from_millis(2500));
            prompt_line(&b);
        }
        // Past the settle and two reads of the tail.
        std::thread::sleep(Duration::from_millis(4500));
        assert_eq!(
            b.state(),
            Some(SessionState::Running),
            "the new turn (pinned: {pinned}, same read: {same_read}): {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
        // The Esc alone: interrupted.
        b.call("ExitPlanMode", "toolu_alone");
        b.stop_on("ExitPlanMode", "toolu_alone");
        esc(&b, "toolu_alone");
        b.c.await_state(b.sid, "interrupted by the Esc alone", |s| *s == interrupted);
    }
}

/// T-957 round 3: an Esc and the next prompt before a fresh cursor's first
/// read. A seat that rested has no cursor; it stops on a plan, the person
/// Escs it and submits a new prompt at once, and the daemon hears the
/// prompt (a pending `Running`) while the file ends at the old interrupt,
/// the new prompt's line not landed yet. The first read seeds the abort
/// from history and looks the closed plan up: neither the High end nor the
/// Low hint may reach the machine, and the new turn stays `Running`. The
/// poll's phase is not the test's to set, so a case repeats, staggered,
/// until the feed shows the fence took the old interrupt.
#[test]
fn an_esc_and_a_prompt_before_a_fresh_cursor_s_first_read_leave_the_new_turn_running() {
    let fenced = |b: &Bed| {
        std::fs::read_to_string(b.h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .matches("\"cmd\":\"tail_interrupt_before_prompt\"")
            .count()
    };
    let mut met = false;
    // A seat of its own each try, so the flap pin is off at the first read,
    // as it is after a restart: the pin would drop the Low hint whatever
    // the fence did.
    for n in 0..8u64 {
        let Some(mut b) = Bed::boot(&format!("planfresh{n}")) else { return };
        b.start();
        b.hook("Stop", r#"{"stop_hook_active":false}"#);
        b.c.await_state(b.sid, "at rest", |s| matches!(s, SessionState::Idle { .. }));
        // A tail poll at rest forgets the cursor; then the phase stagger.
        std::thread::sleep(Duration::from_millis(2200 + n * 700 % 2000));
        b.call("ExitPlanMode", "toolu_f");
        b.stop_on("ExitPlanMode", "toolu_f");
        b.result(
            "toolu_f",
            true,
            "The user doesn't want to proceed with this tool use. STOP what you are doing.",
        );
        b.append(serde_json::json!({"uuid": "abort-f", "type": "user",
            "timestamp": now_iso(), "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}));
        b.hook("UserPromptSubmit", r#"{"prompt":"again"}"#);
        std::thread::sleep(Duration::from_millis(2500));
        b.append(serde_json::json!({"uuid": "next-f", "type": "user",
            "timestamp": now_iso(), "message": {"content": "again"}}));
        std::thread::sleep(Duration::from_millis(2500));
        let pinned = b.decisions().iter().any(|d| {
            d["source"] == "PreToolUse"
                && d["decision"]["after"]["pinned_until"]
                    .as_u64()
                    .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
        });
        assert!(!pinned, "the pin is off at the plan, try {n}");
        assert_eq!(
            b.state(),
            Some(SessionState::Running),
            "the new turn, try {n}: {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
        if fenced(&b) > 0 {
            met = true;
            break;
        }
    }
    assert!(met, "no first read met the heard prompt in eight tries");
}

/// T-957 rounds 3 and 4: a prompt heard late keeps its own turn's Esc. The
/// writer was busy: the prompt's and the plan's hooks are consumed in the
/// engine's order, and the lines the engine wrote meanwhile (the prompt,
/// the plan's call, its result and the Esc's interrupt) are read after. No
/// prompt comes after them, so the Esc is this turn's and the seat rests
/// interrupted. No clock and no words decide it: the case holds with the
/// prompt's line stamped seconds back, and with a hook whose words the
/// transcript line does not repeat (a slash command's markup).
#[test]
fn a_prompt_heard_late_keeps_its_own_turn_s_esc() {
    for (n, (heard, written)) in [
        ("Plan it.", "Plan it."),
        ("/plan it", "<command-name>/plan</command-name>\n<command-args>it</command-args>"),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(mut b) = Bed::boot(&format!("planlate{n}")) else { return };
        b.start();
        // A tail read while running mints the cursor these lines are read by.
        std::thread::sleep(Duration::from_millis(2500));
        b.hook("UserPromptSubmit", &serde_json::json!({ "prompt": heard }).to_string());
        b.stop_on("ExitPlanMode", "toolu_late");
        let line = |v: serde_json::Value, ago: u64| {
            let mut v = v;
            v["timestamp"] = serde_json::json!(iso_ago(ago));
            b.append(v);
        };
        line(
            serde_json::json!({"uuid": "late-p", "type": "user", "message": {"content": written}}),
            3000,
        );
        line(
            serde_json::json!({"uuid": "late-c", "type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "toolu_late", "name": "ExitPlanMode",
                "input": {"plan": "# Plan"}}]}}),
            2900,
        );
        line(
            serde_json::json!({"uuid": "late-r", "type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "toolu_late", "is_error": true,
                "content": "The user doesn't want to proceed with this tool use. STOP what you are doing."}]}}),
            2800,
        );
        line(
            serde_json::json!({"uuid": "late-a", "type": "user", "message": {"content": [
            {"type": "text", "text": "[Request interrupted by user for tool use]"}]}}),
            2790,
        );
        b.c.await_state(b.sid, "interrupted by its own turn's Esc", |s| {
            *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted }
        });
    }
}

/// T-957 round 4: repeated words. The person's prompt repeats an earlier
/// one's words, back to back or with another prompt between, and after an
/// Esc on a plan its hook is heard before its line lands. Words identify a
/// text, not its occurrence: the hearing's order alone says the prompt came
/// after the plan, so neither the plan's interrupted end nor the Low hint
/// beside it reaches the machine, and the new turn stays `Running`, unpinned
/// and pinned.
#[test]
fn a_repeated_prompt_heard_before_its_line_leaves_the_new_turn_running() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    for (pinned, between) in [(false, false), (false, true), (true, false), (true, true)] {
        let name = format!("planrepeat_{}_{}", u8::from(pinned), u8::from(between));
        let Some(mut b) = Bed::boot(&name) else { return };
        // The first "go": its line (the bed's first) and its hook.
        b.start();
        if between {
            b.append(serde_json::json!({"uuid": "other", "type": "user", "timestamp": now_iso(),
                "message": {"content": "something else"}}));
            b.hook("UserPromptSubmit", r#"{"prompt":"something else"}"#);
        }
        if pinned {
            // More than four committed changes in 20 s arm the pin.
            for _ in 0..3 {
                b.hook("Stop", r#"{"stop_hook_active":false}"#);
                b.c.await_state(b.sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
                b.prompt();
            }
        }
        // A tail read while running mints the cursor, so the Esc is read
        // live, and the fence meets its end.
        std::thread::sleep(Duration::from_millis(2500));
        b.call("ExitPlanMode", "toolu_r");
        b.stop_on("ExitPlanMode", "toolu_r");
        b.result("toolu_r", true, STOP);
        b.append(serde_json::json!({"uuid": "abort-r", "type": "user", "timestamp": now_iso(),
            "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}));
        // "go" again: its hook first, its line 2.5 s later.
        b.hook("UserPromptSubmit", r#"{"prompt":"go"}"#);
        std::thread::sleep(Duration::from_millis(2500));
        b.append(serde_json::json!({"uuid": "go-again", "type": "user", "timestamp": now_iso(),
            "message": {"content": "go"}}));
        std::thread::sleep(Duration::from_millis(2500));
        let armed = b.decisions().into_iter().rev().find(|d| d["source"] == "PreToolUse");
        let armed = armed.is_some_and(|d| {
            d["decision"]["after"]["pinned_until"]
                .as_u64()
                .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
        });
        assert_eq!(armed, pinned, "the pin as this case wants it");
        let fenced = std::fs::read_to_string(b.h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .contains("\"cmd\":\"tail_interrupt_before_prompt\"");
        assert!(fenced, "the fence took the old Esc");
        assert_eq!(
            b.state(),
            Some(SessionState::Running),
            "the new turn (pinned: {pinned}, a prompt between: {between}): {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
    }
}

/// T-957 round 5: an Esc on plan A whose seat is already on a newer dialog
/// B. The person Escs A and submits; the agent stops on B (a plan, or a
/// question) and its hook is heard before the tail reads A's result and
/// interrupt, B's line not written yet. Nothing of A's may reach the
/// machine: its High end is dropped (B is projected; or, for a question, the
/// held tool is no longer A's), and no Low hint is left beside it to settle
/// B. Past the settle and two reads, the seat is still on B, and its ticket
/// still needs the person with B's request. The pin is off. Round 7: the
/// same with A's interrupt record presented twice, no copy of it left.
#[test]
fn an_old_plan_s_esc_leaves_a_newer_dialog_standing() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    // Round 7: and with A's interrupt record presented twice (a replay).
    for (n, (tool, reason, word, replayed)) in [
        ("ExitPlanMode", Reason::Plan, "plan", false),
        ("AskUserQuestion", Reason::Question, "question", false),
        ("ExitPlanMode", Reason::Plan, "plan", true),
        ("AskUserQuestion", Reason::Question, "question", true),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(mut b) = Bed::boot(&format!("plannewer{n}")) else { return };
        b.start();
        // A tail read while running mints the cursor, so A's Esc is read live.
        std::thread::sleep(Duration::from_millis(2500));
        b.call("ExitPlanMode", "toolu_a");
        b.stop_on("ExitPlanMode", "toolu_a");
        b.result("toolu_a", true, STOP);
        let interrupt = serde_json::json!({"uuid": "abort-a", "type": "user",
            "timestamp": now_iso(), "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}});
        b.append(interrupt.clone());
        if replayed {
            b.append(interrupt);
        }
        // The new prompt and B, heard before the tail reads A's end.
        b.hook("UserPromptSubmit", r#"{"prompt":"Plan it again."}"#);
        b.stop_on(tool, "toolu_b");
        std::thread::sleep(Duration::from_millis(2500));
        b.append(serde_json::json!({"uuid": "next-b", "type": "user", "timestamp": now_iso(),
            "message": {"content": "Plan it again."}}));
        b.call(tool, "toolu_b");
        std::thread::sleep(Duration::from_millis(2500));
        let pinned = b.decisions().iter().any(|d| {
            d["decision"]["after"]["pinned_until"]
                .as_u64()
                .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
        });
        assert!(!pinned, "the pin is off ({word}, replayed: {replayed})");
        assert_eq!(
            b.state(),
            Some(SessionState::RequiresAction { reason }),
            "still on B ({word}, replayed: {replayed}): {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
        let needs = match b.c.send(
            mesimon_core::Principal::Agent { session: b.sid },
            Command::AgentGetTicket { full: false },
        ) {
            Response::AgentTicket { ticket } => ticket.needs_you,
            other => panic!("get_ticket: {other:?}"),
        };
        let needs = needs.expect("B still needs the person");
        assert_eq!(needs.reason, word);
        assert_eq!(needs.request.as_deref(), Some("toolu_b"), "B's request ({word})");
    }
}

/// T-957 rounds 9 and 10: an old interrupt presented again in a later read.
/// Plan A, the person Escs and submits, and B is heard (projected) before
/// the tail reads A's end. The reads then come apart, each part appended
/// only once the feed shows the read before it decided: `result A + X | X`,
/// and `result A | X | X`. A's end is dropped in its read
/// (`tail_end_of_another_dialog`, B is projected), and the X that a later
/// read presents again has no end to ride and no call: on a seat whose
/// dialog a hook named, it moves nothing (`tail_abort_beside_a_dialog`).
/// Past that decision's settle, the seat is still on B with B's request.
/// The pin is off.
#[test]
fn an_old_interrupt_replayed_in_a_later_read_leaves_a_newer_dialog_standing() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    for (n, split) in [false, true].into_iter().enumerate() {
        let Some(mut b) = Bed::boot(&format!("planreplay{n}")) else { return };
        b.start();
        // A tail read while running mints the cursor, so the reads are live.
        std::thread::sleep(Duration::from_millis(2500));
        b.call("ExitPlanMode", "toolu_a");
        b.stop_on("ExitPlanMode", "toolu_a");
        b.hook("UserPromptSubmit", r#"{"prompt":"Plan it again."}"#);
        b.stop_on("ExitPlanMode", "toolu_b");
        let lines = |b: &Bed, cmd: &str| {
            std::fs::read_to_string(b.h.paths.state_dir.join("activity.jsonl"))
                .unwrap_or_default()
                .matches(&format!("\"cmd\":\"{cmd}\""))
                .count()
        };
        let read_and = |b: &Bed, cmd: &str, count: usize| {
            wait_until(Duration::from_secs(10), &format!("the read deciding {cmd}"), || {
                lines(b, cmd) >= count
            });
        };
        let interrupt = serde_json::json!({"uuid": "abort-a", "type": "user",
            "timestamp": now_iso(), "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}});
        let result = Bed::result_of("toolu_a", true, STOP);
        if split {
            // `result A | X | X`: A's end, then its Esc, each dropped in its read.
            b.append(result);
            read_and(&b, "tail_end_of_another_dialog", 1);
            b.append(interrupt.clone());
            read_and(&b, "tail_end_of_another_dialog", 2);
        } else {
            // `result A + X | X`: A's interrupted end, dropped in one read.
            b.append_all(&[result, interrupt.clone()]);
            read_and(&b, "tail_end_of_another_dialog", 1);
        }
        b.append(interrupt);
        read_and(&b, "tail_abort_beside_a_dialog", 1);
        // Past the settle of anything that read could have started.
        std::thread::sleep(Duration::from_millis(2000));
        let pinned = b.decisions().iter().any(|d| {
            d["decision"]["after"]["pinned_until"]
                .as_u64()
                .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
        });
        assert!(!pinned, "the pin is off (split: {split})");
        assert_eq!(
            b.state(),
            Some(SessionState::RequiresAction { reason: Reason::Plan }),
            "still on B (split: {split}): {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
        let needs = match b.c.send(
            mesimon_core::Principal::Agent { session: b.sid },
            Command::AgentGetTicket { full: false },
        ) {
            Response::AgentTicket { ticket } => ticket.needs_you,
            other => panic!("get_ticket: {other:?}"),
        };
        let needs = needs.expect("B still needs the person");
        assert_eq!(needs.request.as_deref(), Some("toolu_b"), "B's request (split: {split})");
    }
}

/// T-957 round 9: a restart between a newer dialog's hook and its line. A
/// is refused with words, B's `PreToolUse` is heard while the record is
/// still on its plan, and the daemon restarts before B's line is written:
/// the projection is gone, but the record names B. The new daemon's first
/// read finds A's result newer than the spell, and that ends nothing: the
/// seat is still on B, needing the person. Once B's line lands, the board
/// rebuilds B's dialog from it, and `get_ticket` reads B's request again.
#[test]
fn a_restart_before_a_newer_dialog_s_line_keeps_it() {
    let Some(mut b) = Bed::boot("planrestartb") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    b.call("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_a");
    b.result("toolu_a", true, REFUSED);
    b.hook("UserPromptSubmit", r#"{"prompt":"Plan it again."}"#);
    b.stop_on("ExitPlanMode", "toolu_b");
    b.h.restart_after(|| {});
    b.c = b.h.client("planrestartb_2");
    let needs = |b: &mut Bed| match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you,
        other => panic!("get_ticket: {other:?}"),
    };
    // The first reads and the settle, B's line held back.
    std::thread::sleep(Duration::from_millis(4500));
    assert_eq!(
        b.state(),
        Some(SessionState::RequiresAction { reason: Reason::Plan }),
        "still on B after the restart: {:?}",
        b.decisions().iter().rev().take(4).collect::<Vec<_>>()
    );
    assert_eq!(needs(&mut b).map(|n| n.reason).as_deref(), Some("plan"));
    b.call("ExitPlanMode", "toolu_b");
    wait_until(Duration::from_secs(10), "B's dialog rebuilt from its line", || {
        needs(&mut b).and_then(|n| n.request).as_deref() == Some("toolu_b")
    });
    assert_eq!(b.state(), Some(SessionState::RequiresAction { reason: Reason::Plan }));
}

/// T-957 round 10: a dialog too big to draw still names its call. Plan A is
/// projected, then the agent asks a question B whose input passes the
/// projection's 16 KiB bound: nothing projects it, but B's hook names its
/// call on the record, so A's identity does not linger. B's own result and
/// Esc end B, interrupted, live and after a restart that came between B's
/// hook and its end.
#[test]
fn a_question_too_big_to_draw_still_ends_on_its_own_result() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    for (n, restart) in [false, true].into_iter().enumerate() {
        let Some(mut b) = Bed::boot(&format!("planbig{n}")) else { return };
        b.start();
        std::thread::sleep(Duration::from_millis(2500));
        b.call("ExitPlanMode", "toolu_a");
        b.stop_on("ExitPlanMode", "toolu_a");
        let big = serde_json::json!({"questions": [{"question": "Which?", "header": "Which",
            "options": [{"label": "A", "description": "x".repeat(17 * 1024)},
                {"label": "B", "description": ""}], "multiSelect": false}]});
        b.append(serde_json::json!({"uuid": "c-big", "type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "toolu_b", "name": "AskUserQuestion", "input": big}]}}));
        let frame = serde_json::json!({"tool_name": "AskUserQuestion", "tool_use_id": "toolu_b",
            "tool_input": big});
        b.hook("PreToolUse", &frame.to_string());
        b.c.await_state(b.sid, "on the question", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Question }
        });
        if restart {
            b.h.restart_after(|| {});
            b.c = b.h.client(&format!("planbig{n}_2"));
            std::thread::sleep(Duration::from_millis(2500));
        }
        b.result("toolu_b", true, STOP);
        b.append(serde_json::json!({"uuid": "esc-b", "type": "user", "timestamp": now_iso(),
            "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}));
        b.c.await_state(b.sid, "off the question its own Esc ended", |s| *s == interrupted);
        let needs = match b.c.send(
            mesimon_core::Principal::Agent { session: b.sid },
            Command::AgentGetTicket { full: false },
        ) {
            Response::AgentTicket { ticket } => ticket.needs_you,
            other => panic!("get_ticket: {other:?}"),
        };
        assert!(needs.is_none(), "no dialog stands (restart: {restart}): {needs:?}");
    }

    // ---- T-988: B too big to draw is A's sibling, its hook heard first ----
    // A's answer holds the seat on B, B's own fence restored with it though
    // B never projects: B's Esc ends it, not read as an earlier turn's.
    let Some(mut b) = Bed::boot("planbigsibling") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    let small = Bed::input("AskUserQuestion");
    let big = serde_json::json!({"questions": [{"question": "Which?", "header": "Which",
        "options": [{"label": "A", "description": "x".repeat(17 * 1024)},
            {"label": "B", "description": ""}], "multiSelect": false}]});
    b.append_all(&[("toolu_a", &small), ("toolu_b", &big)].map(|(id, input)| {
        serde_json::json!({"uuid": format!("c-{id}"), "type": "assistant", "message": {
            "id": "msg_1", "content": [{"type": "tool_use", "id": id,
                "name": "AskUserQuestion", "input": input}]}})
    }));
    let frame = |id: Option<&str>, input: &serde_json::Value| {
        let mut f = serde_json::json!({"tool_name": "AskUserQuestion", "tool_input": input});
        if let Some(id) = id {
            f["tool_use_id"] = serde_json::json!(id);
        }
        f.to_string()
    };
    b.hook("PreToolUse", &frame(Some("toolu_b"), &big));
    b.hook("PreToolUse", &frame(Some("toolu_a"), &small));
    b.hook("PermissionRequest", &frame(None, &small));
    b.hook("PermissionRequest", &frame(None, &big));
    b.c.await_state(b.sid, "on the question", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Question }
    });
    b.hook("PostToolUse", &frame(Some("toolu_a"), &small));
    b.result("toolu_a", false, "Your questions have been answered.");
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(
        b.state(),
        Some(SessionState::RequiresAction { reason: Reason::Question }),
        "B stands"
    );
    b.result("toolu_b", true, STOP);
    b.append(serde_json::json!({"uuid": "esc-b", "type": "user", "timestamp": now_iso(),
        "message": {"content": [{"type": "text",
            "text": "[Request interrupted by user for tool use]"}]}}));
    b.c.await_state(b.sid, "off the question B's own Esc ended", |s| *s == interrupted);
}

/// T-957 round 11: an id-less `PermissionRequest` names a call only as a
/// repeat. Plan A is named and projected; question B arrives as an id-less
/// `PermissionRequest` alone (its `PreToolUse` lost, a hook failing open),
/// its call on the transcript. It is no repeat of A, another tool, so A's
/// call is let go: after a restart B is rebuilt from its line, and B's own
/// result and Esc end it. Control: an id-less `PermissionRequest` repeating
/// A keeps A's call (after a restart A's refusal with words, which has no
/// interrupt to fall back on, ends A by that call alone, A's call line kept
/// off the transcript) and A's prompt-count fence (a new prompt, then A's
/// Esc, which ends nothing).
#[test]
fn an_id_less_permission_request_keeps_a_call_only_as_a_repeat() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let esc = |b: &Bed, id: &str| {
        b.append_all(&[
            Bed::result_of(id, true, STOP),
            serde_json::json!({"uuid": format!("esc-{id}"), "type": "user",
                "timestamp": now_iso(), "message": {"content": [{"type": "text",
                    "text": "[Request interrupted by user for tool use]"}]}}),
        ]);
    };
    let needs = |b: &mut Bed| match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you,
        other => panic!("get_ticket: {other:?}"),
    };
    let id_less = |b: &Bed, tool: &str| {
        let frame = serde_json::json!({"tool_name": tool, "tool_input": Bed::input(tool)});
        b.hook("PermissionRequest", &frame.to_string());
    };

    // ---- the replacement: B names no call, and A's is let go ---------------
    let Some(mut b) = Bed::boot("planidless") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    b.call("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_a");
    b.call("AskUserQuestion", "toolu_b");
    id_less(&b, "AskUserQuestion");
    b.c.await_state(b.sid, "on the question", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Question }
    });
    b.h.restart_after(|| {});
    b.c = b.h.client("planidless_2");
    wait_until(Duration::from_secs(10), "B rebuilt from its line", || {
        needs(&mut b).and_then(|n| n.request).as_deref() == Some("toolu_b")
    });
    esc(&b, "toolu_b");
    b.c.await_state(b.sid, "off the question its own Esc ended", |s| *s == interrupted);
    assert!(needs(&mut b).is_none(), "no dialog stands");

    // ---- the control, after a restart: a repeat keeps A's call -------------
    let Some(mut b) = Bed::boot("planrepeatcall") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    // A's call line stays off the transcript (as if past the window), so
    // only the call the record names can tie A's result to A.
    b.stop_on("ExitPlanMode", "toolu_a");
    id_less(&b, "ExitPlanMode");
    // The repeat decided before the restart: its decision in the feed.
    wait_until(Duration::from_secs(10), "the PermissionRequest decided", || {
        b.decisions().iter().any(|d| d["source"] == "PermissionRequest")
    });
    b.h.restart_after(|| {});
    b.c = b.h.client("planrepeatcall_2");
    std::thread::sleep(Duration::from_millis(2500));
    b.result("toolu_a", true, REFUSED);
    b.c.await_state(b.sid, "A's refusal ended A by its call", |s| *s == SessionState::Running);
    assert!(needs(&mut b).is_none(), "no dialog stands");

    // ---- the control, live: a repeat keeps A's prompt-count fence ----------
    let Some(mut b) = Bed::boot("planrepeatfence") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    b.call("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_a");
    id_less(&b, "ExitPlanMode");
    b.hook("UserPromptSubmit", r#"{"prompt":"Plan it again."}"#);
    esc(&b, "toolu_a");
    wait_until(Duration::from_secs(10), "A's Esc fenced as the turn before", || {
        std::fs::read_to_string(b.h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .contains("\"cmd\":\"tail_interrupt_before_prompt\"")
    });
    std::thread::sleep(Duration::from_millis(2000));
    assert_eq!(b.state(), Some(SessionState::Running), "the new turn runs");
}

/// T-957 round 12: a repeat is told by the interaction's print, not by its
/// drawing. A question past the projection's 16 KiB bound is never drawn,
/// so an identical id-less `PermissionRequest` after its named `PreToolUse`
/// had no projection to be compared with, and let the call go. Here the
/// writer is busy: the question's call, its refusal and its Esc are on disk
/// before a reader with no cursor consumes the named `PreToolUse` and the
/// identical id-less `PermissionRequest`. The print says repeat, the call is
/// kept, and the first read ends the question by it: `Interrupted`, no
/// `needs_you`. Control: an id-less `PermissionRequest` with another input,
/// both past the bound, is a replacement and names no call.
#[test]
fn a_question_too_big_to_draw_keeps_its_call_through_an_id_less_repeat() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let big = |n: &str| {
        serde_json::json!({"questions": [{"question": format!("Which {n}?"), "header": "Which",
            "options": [{"label": "A", "description": "x".repeat(17 * 1024)},
                {"label": "B", "description": ""}], "multiSelect": false}]})
    };
    let frame = |id: Option<&str>, input: &serde_json::Value| {
        let mut f = serde_json::json!({"tool_name": "AskUserQuestion", "tool_input": input});
        if let Some(id) = id {
            f["tool_use_id"] = serde_json::json!(id);
        }
        f.to_string()
    };
    let held_call = |b: &Bed| -> Option<String> {
        let text = std::fs::read_to_string(b.h.paths.sessions_file()).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        v["sessions"].as_array()?.iter().find(|s| s["id"] == b.sid.to_string())?["dialog_call"]
            .as_str()
            .map(String::from)
    };
    let decided = |b: &Bed, event: &str, n: usize| {
        wait_until(Duration::from_secs(10), &format!("{event} decided"), || {
            b.decisions().iter().filter(|d| d["source"] == event).count() >= n
        });
    };

    // ---- the repeat, read by a fresh reader after its end is on disk -------
    let Some(mut b) = Bed::boot("planbigrepeat") else { return };
    b.start();
    b.hook("Stop", r#"{"stop_hook_active":false}"#);
    b.c.await_state(b.sid, "at rest", |s| matches!(s, SessionState::Idle { .. }));
    // A tail read at rest forgets the cursor.
    std::thread::sleep(Duration::from_millis(2500));
    let input = big("one");
    b.append_all(&[
        serde_json::json!({"uuid": "c-b", "type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": "toolu_b", "name": "AskUserQuestion", "input": input}]}}),
        Bed::result_of("toolu_b", true, STOP),
        serde_json::json!({"uuid": "esc-b", "type": "user", "timestamp": now_iso(),
            "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}),
    ]);
    b.hook("PreToolUse", &frame(Some("toolu_b"), &input));
    b.hook("PermissionRequest", &frame(None, &input));
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    b.c.await_state(b.sid, "off the question by its kept call", |s| *s == interrupted);
    let needs = match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you,
        other => panic!("get_ticket: {other:?}"),
    };
    assert!(needs.is_none(), "no dialog stands: {needs:?}");

    // ---- the control: another input, past the bound too, replaces ----------
    let Some(mut b) = Bed::boot("planbigreplace") else { return };
    b.start();
    b.hook("PreToolUse", &frame(Some("toolu_c"), &big("one")));
    decided(&b, "PreToolUse", 1);
    assert_eq!(held_call(&b).as_deref(), Some("toolu_c"), "the named call is held");
    b.hook("PermissionRequest", &frame(None, &big("two")));
    decided(&b, "PermissionRequest", 1);
    assert_eq!(held_call(&b), None, "a replacement names no call");
    // An id-less repeat of what replaced it keeps none either.
    b.hook("PermissionRequest", &frame(None, &big("two")));
    decided(&b, "PermissionRequest", 2);
    assert_eq!(held_call(&b), None);
}

/// T-957 round 13: a record held on a dialog from before the print. Plan A
/// is named and projected, and the daemon restarts with the record as an
/// older build left it, no call and no print. T-933's rebuild brings A back
/// from its open call and seeds A's print with it, so A's identical id-less
/// `PermissionRequest` reads as the repeat it is: A keeps its own request,
/// and A's result and Esc end it, `Interrupted`, no `needs_you`. Control: an
/// id-less `PermissionRequest` with another input replaces A's request.
#[test]
fn a_dialog_rebuilt_for_an_older_record_keeps_its_request_through_a_repeat() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let needs = |b: &mut Bed| match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you,
        other => panic!("get_ticket: {other:?}"),
    };
    for (n, same) in [true, false].into_iter().enumerate() {
        let Some(mut b) = Bed::boot(&format!("planolder{n}")) else { return };
        b.start();
        b.call("ExitPlanMode", "toolu_a");
        b.stop_on("ExitPlanMode", "toolu_a");
        // The record as a build before the print and the call wrote it.
        let sessions = b.h.paths.sessions_file();
        b.h.restart_after(|| {
            let text = std::fs::read_to_string(&sessions).unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
            for s in v["sessions"].as_array_mut().unwrap() {
                let s = s.as_object_mut().unwrap();
                s.remove("dialog_call");
                s.remove("dialog_print");
            }
            std::fs::write(&sessions, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        });
        b.c = b.h.client(&format!("planolder{n}_2"));
        wait_until(Duration::from_secs(10), "A rebuilt from its open call", || {
            needs(&mut b).and_then(|n| n.request).as_deref() == Some("toolu_a")
        });
        let input = if same {
            Bed::input("ExitPlanMode")
        } else {
            serde_json::json!({"plan": "# Another plan"})
        };
        let frame = serde_json::json!({"tool_name": "ExitPlanMode", "tool_input": input});
        b.hook("PermissionRequest", &frame.to_string());
        wait_until(Duration::from_secs(10), "the PermissionRequest decided", || {
            b.decisions().iter().any(|d| d["source"] == "PermissionRequest")
        });
        let request = needs(&mut b).and_then(|n| n.request);
        if !same {
            assert_ne!(request.as_deref(), Some("toolu_a"), "another input replaces A");
            continue;
        }
        assert_eq!(request.as_deref(), Some("toolu_a"), "the repeat keeps A's request");
        b.append_all(&[
            Bed::result_of("toolu_a", true, STOP),
            serde_json::json!({"uuid": "esc-a", "type": "user", "timestamp": now_iso(),
                "message": {"content": [{"type": "text",
                    "text": "[Request interrupted by user for tool use]"}]}}),
        ]);
        b.c.await_state(b.sid, "off A by its own Esc", |s| *s == interrupted);
        assert!(needs(&mut b).is_none(), "no dialog stands");
    }
}

/// T-968, refuted: the reader's first ordering. A state-only rule (the
/// machine taking a dialog's end from the dialog alone, nothing settling)
/// cannot stand in for the prompt-order fence, because the prompt's pending
/// `Running` is cancellable. A is held, its call and print on the record;
/// the person Escs it and speaks; the newer turn's plan B, the same tool's
/// same input, arrives as an id-less `PermissionRequest` alone (its
/// `PreToolUse` lost) inside the prompt's settle. The print reads it as A's
/// repeat, so A's call is kept, and its `PreToolUse` signal re-affirms the
/// plan and cancels the pending leave. Then the tail reads A's result and
/// interrupt with the record on the plan and nothing settling. Only the
/// fence (`spoken_since`) knows a prompt came after A: B stands.
#[test]
fn a_same_print_repeat_inside_a_prompt_s_settle_keeps_the_newer_dialog() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let Some(mut b) = Bed::boot("planrepeatsettle") else { return };
    b.start();
    // A tail read while running mints the cursor, so A's Esc is read live.
    std::thread::sleep(Duration::from_millis(2500));
    b.call("ExitPlanMode", "toolu_a");
    b.stop_on("ExitPlanMode", "toolu_a");
    let asked = b.decisions().len();
    b.hook("UserPromptSubmit", r#"{"prompt":"Plan it again."}"#);
    let frame = serde_json::json!({"tool_name": "ExitPlanMode",
        "tool_input": Bed::input("ExitPlanMode")});
    b.hook("PermissionRequest", &frame.to_string());
    // Both hooks decided, the second inside the first's settle: the plan
    // re-affirmed, nothing pending.
    wait_until(Duration::from_secs(5), "the id-less repeat decided", || {
        b.decisions().iter().skip(asked).any(|d| d["source"] == "PermissionRequest")
    });
    let repeat = b.decisions().into_iter().skip(asked).find(|d| d["source"] == "PermissionRequest");
    let repeat = repeat.unwrap();
    assert!(repeat["decision"]["before"]["pending"]["state"]["state"] == "running", "{repeat}");
    assert!(repeat["decision"]["after"]["pending"].is_null(), "the settle cancelled: {repeat}");
    // A's result and Esc, in one write.
    b.append_all(&[
        Bed::result_of("toolu_a", true, STOP),
        serde_json::json!({"uuid": "esc-a", "type": "user", "timestamp": now_iso(),
            "message": {"content": [{"type": "text",
                "text": "[Request interrupted by user for tool use]"}]}}),
    ]);
    // Past the settle and two reads: B still needs the person, and the
    // fence took A's Esc.
    std::thread::sleep(Duration::from_millis(4500));
    assert_eq!(
        b.state(),
        Some(SessionState::RequiresAction { reason: Reason::Plan }),
        "B stands: {:?}",
        b.decisions().iter().rev().take(4).collect::<Vec<_>>()
    );
    assert!(
        std::fs::read_to_string(b.h.paths.state_dir.join("activity.jsonl"))
            .unwrap_or_default()
            .contains("\"cmd\":\"tail_interrupt_before_prompt\""),
        "the fence took A's Esc"
    );
}

/// T-968, refuted: the reader's second ordering. A rule that refuses a
/// dialog's end over any pending leave also refuses a genuine same-turn
/// Esc. The person accepts the plan (its `PostToolUse` schedules
/// `Running`) and Escs at once, before the model writes anything; the tail
/// reads the accepted call's result and the interrupt right after it inside
/// that settle. The end is the dialog's own, no prompt came between, and it
/// replaces the pending `Running`: the seat rests `Idle{Interrupted}` at
/// High. The poll's phase is not the test's to set, so a fresh seat each
/// try (the flap pin off) repeats, staggered, until a read lands inside the
/// settle; every try must rest interrupted at High.
#[test]
fn an_esc_inside_an_accept_s_settle_rests_interrupted() {
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let mut met = false;
    for n in 0..8u64 {
        let Some(mut b) = Bed::boot(&format!("planacceptesc{n}")) else { return };
        b.start();
        // A tail read while running mints the cursor; then the phase stagger.
        std::thread::sleep(Duration::from_millis(2500 + n * 700 % 2000));
        b.call("ExitPlanMode", "toolu_ok");
        b.stop_on("ExitPlanMode", "toolu_ok");
        let accept = serde_json::json!({"tool_name": "ExitPlanMode", "tool_use_id": "toolu_ok",
            "tool_input": Bed::input("ExitPlanMode")});
        b.hook("PostToolUse", &accept.to_string());
        b.append_all(&[
            Bed::result_of("toolu_ok", false, "User has approved your plan."),
            serde_json::json!({"uuid": "esc-ok", "type": "user", "timestamp": now_iso(),
                "message": {"content": [{"type": "text", "text": "[Request interrupted by user]"}]}}),
        ]);
        b.c.await_state(b.sid, "interrupted by the Esc after the accept", |s| *s == interrupted);
        let rec = board_of(b.c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == b.sid)
            .cloned()
            .unwrap();
        assert_eq!(
            rec.confidence,
            mesimon_core::board::Confidence::High,
            "the dialog's own end, try {n}"
        );
        // Met: the end read while the accept's `Running` was settling, and
        // it replaced that leave.
        met = b.departures().iter().any(|d| {
            d["decision"]["before"]["pending"]["state"]["state"] == "running"
                && d["decision"]["after"]["pending"]["state"]
                    == serde_json::to_value(&interrupted).unwrap()
        });
        if met {
            break;
        }
    }
    assert!(met, "no read of the Esc landed inside the accept's settle in eight tries");
}

/// T-988: two plans or two questions in one message, measured on Claude
/// Code 2.1.310 with a real Sonnet on both roads: the dialogs are drawn one
/// after the other, and A's answer moved the seat to `Running` while B stood
/// 6 to 8 s. Questions go `PreToolUse(A) PreToolUse(B) PermissionRequest(A)
/// PermissionRequest(B)` (distinct inputs, so A's id-less request is the
/// earlier call's, not a replacement); plans go `PreToolUse(A)
/// PermissionRequest(A) PreToolUse(B) PermissionRequest(B)` (one plan file,
/// so the same input for both), the two calls' lines under one message id.
/// Each hook is a process of its own, and live
/// B's `PreToolUse` once arrived before A's. Each `PostToolUse` names its
/// call. A's answer, with its result on the transcript, leaves the seat on
/// B's dialog past the settle, in either arrival order; B's answer leaves it.
#[test]
fn two_dialogs_in_one_message_hold_the_seat_until_the_second_is_answered() {
    let ask = |q: &str| {
        serde_json::json!({"questions": [{"question": q, "header": "Which",
            "options": [{"label": "A", "description": ""}, {"label": "B", "description": ""}],
            "multiSelect": false}]})
    };
    let held_call = |b: &Bed| -> Option<String> {
        let text = std::fs::read_to_string(b.h.paths.sessions_file()).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        v["sessions"].as_array()?.iter().find(|s| s["id"] == b.sid.to_string())?["dialog_call"]
            .as_str()
            .map(String::from)
    };
    let needs = |b: &mut Bed| match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you.and_then(|n| n.request),
        other => panic!("get_ticket: {other:?}"),
    };
    // Each case's hooks in the order the daemon hears them: (event, call).
    // Each hook is a process of its own: live, B's `PreToolUse` arrived 14 ms
    // before A's.
    let reordered =
        [("PreToolUse", 1), ("PreToolUse", 0), ("PermissionRequest", 0), ("PermissionRequest", 1)];
    let plans =
        [("PreToolUse", 0), ("PermissionRequest", 0), ("PreToolUse", 1), ("PermissionRequest", 1)];
    // Two questions in the order Claude Code sends them is the recording's
    // (`two_questions_in_one_message_hold_the_seat_as_recorded`, T-1014).
    let cases = [
        (
            "twoasksreordered",
            "AskUserQuestion",
            Reason::Question,
            [ask("Which colour?"), ask("Which size?")],
            reordered,
        ),
        (
            "twoplans",
            "ExitPlanMode",
            Reason::Plan,
            [Bed::input("ExitPlanMode"), Bed::input("ExitPlanMode")],
            plans,
        ),
    ];
    for (name, tool, reason, inputs, order) in cases {
        let Some(mut b) = Bed::boot(name) else { return };
        b.start();
        // A tail read while running mints the cursor, so results are read live.
        std::thread::sleep(Duration::from_millis(2500));
        let held = SessionState::RequiresAction { reason };
        let ids = ["toolu_a", "toolu_b"];
        let frame = |id: Option<&str>, input: &serde_json::Value| {
            let mut f = serde_json::json!({"tool_name": tool, "tool_input": input});
            if let Some(id) = id {
                f["tool_use_id"] = serde_json::json!(id);
            }
            f.to_string()
        };
        b.append_all(&[0, 1].map(|n| {
            serde_json::json!({"uuid": format!("c-{}", ids[n]), "type": "assistant",
                "message": {"id": "msg_1", "content": [{"type": "tool_use", "id": ids[n],
                    "name": tool, "input": inputs[n]}]}})
        }));
        for (event, n) in order {
            let named = (event == "PreToolUse").then_some(ids[n]);
            b.hook(event, &frame(named, &inputs[n]));
        }
        b.c.await_state(b.sid, "on the first dialog", |s| *s == held);
        wait_until(Duration::from_secs(10), "both requests decided", || {
            b.decisions().iter().filter(|d| d["source"] == "PermissionRequest").count() >= 2
        });
        let last_named = order.iter().rev().find(|(e, _)| *e == "PreToolUse").map(|(_, n)| ids[*n]);
        assert_eq!(held_call(&b).as_deref(), last_named, "{name}: the last named call is held");

        // ---- A answered: B is still to be answered --------------------------
        b.hook("PostToolUse", &frame(Some("toolu_a"), &inputs[0]));
        b.result("toolu_a", false, "User has answered.");
        // Past the settle and two tail reads.
        std::thread::sleep(Duration::from_millis(3500));
        assert_eq!(
            b.state(),
            Some(held.clone()),
            "{name}: B stands: {:?}",
            b.decisions().iter().rev().take(4).collect::<Vec<_>>()
        );
        assert_eq!(held_call(&b).as_deref(), Some("toolu_b"), "{name}: still held on B");
        assert_eq!(needs(&mut b).as_deref(), Some("toolu_b"), "{name}: get_ticket names B");

        // ---- B answered: the seat leaves ------------------------------------
        b.hook("PostToolUse", &frame(Some("toolu_b"), &inputs[1]));
        b.result("toolu_b", false, "User has answered.");
        b.c.await_state(b.sid, "running on B's answer", |s| *s == SessionState::Running);
        assert!(needs(&mut b).is_none(), "{name}: no dialog stands");
    }
}

/// T-988 as Claude Code sends it (T-1014): the rig's QQ1 on 2.1.310, a
/// Sonnet worker on the mod road asking two questions in one message, the
/// person answering each in the pane, replayed frame for frame and line for
/// line. Two things differ from the sequence measured by hand above: each
/// call's `PreToolUse` is followed by its own `PermissionRequest` (A, A's,
/// B, B's), and the calls' lines reach the transcript about 0.7 s after both
/// hooks, not before them. A's answer (its `PostToolUse`, then its result
/// 0.1 s later) leaves the seat on B's question; B's leaves it.
#[test]
fn two_questions_in_one_message_hold_the_seat_as_recorded() {
    let trace = trace::Trace::load("t988-two-questions");
    let Some(mut b) = Bed::boot("twoasks_rec") else { return };
    let calls: Vec<String> = trace
        .frames()
        .into_iter()
        .filter(|(e, _, p)| *e == "PreToolUse" && p["tool_name"] == "AskUserQuestion")
        .map(|(_, _, p)| p["tool_use_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(calls.len(), 2, "two calls: {:?}", trace.events());
    let (a, second) = (&calls[0], &calls[1]);
    let sock = b.h.paths.hook_sock();
    let mut r = trace.replay(&sock, b.sid, &b.transcript);
    r.through(trace::frame("UserPromptSubmit", None));
    b.c.await_state(b.sid, "running", |s| *s == SessionState::Running);
    // The model's first seconds: a tail read while running mints the cursor.
    std::thread::sleep(Duration::from_millis(2500));
    let question = SessionState::RequiresAction { reason: Reason::Question };
    let answered_a = r.until(trace::frame("PostToolUse", Some("AskUserQuestion"))).unwrap();
    assert_eq!(answered_a["payload"]["tool_use_id"], a.as_str(), "A is answered first");
    b.c.await_state(b.sid, "on the questions", |s| *s == question);
    wait_until(Duration::from_secs(10), "the last named call held", || {
        b.held_call().as_deref() == Some(second.as_str())
    });

    // ---- A answered: B is still to be answered ------------------------------
    r.through(trace::result_of(a));
    // Past the settle and two tail reads.
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(b.state(), Some(question.clone()), "B stands: {:?}", b.decisions());
    assert_eq!(b.held_call().as_deref(), Some(second.as_str()), "still held on B");
    let needs = match b
        .c
        .send(mesimon_core::Principal::Agent { session: b.sid }, Command::AgentGetTicket)
    {
        Response::AgentTicket { ticket } => ticket.needs_you.and_then(|n| n.request),
        other => panic!("get_ticket: {other:?}"),
    };
    assert_eq!(needs.as_deref(), Some(second.as_str()), "get_ticket names B");

    // ---- B answered: the seat leaves, and the turn ends ---------------------
    r.through(trace::result_of(second));
    b.c.await_state(b.sid, "running on B's answer", |s| *s == SessionState::Running);
    r.rest();
    b.c.await_state(b.sid, "the turn's end", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
    });
}

/// T-988 round 1 (T-996's cold read): a sibling is a call of the held
/// call's message with no result on the transcript, and only that. Live on
/// 2.1.310, an Esc on two questions or a "No" on two plans writes both
/// refusals in one millisecond, in call order, then the interrupt; "Chat
/// about this" on A writes A's refusal alone and B stays drawn, answerable.
/// - **Chat about A** (no hook): B answered leaves the seat, never held back
///   on the refused A.
/// - **Esc on both**, in either result order (the hook set wrote A's first,
///   the mod road B's first, 33 ms before the interrupt): the held call's
///   end and its interrupt take the seat out, the other refusal read as the
///   sibling's, never as a word that takes the interrupt from it.
/// - **A's result read by the tail before A's `PostToolUse`** (A held): the
///   answer re-points to B and re-affirms it, so the leave A's end set
///   settling does not commit.
/// - **A late answer of an earlier message** moves nothing.
#[test]
fn a_sibling_is_an_unanswered_call_of_the_held_call_s_message() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let question = SessionState::RequiresAction { reason: Reason::Question };
    let (qa, qb) = (Bed::ask("Which colour?"), Bed::ask("Which size?"));
    let (frame, line, esc) = (Bed::frame, Bed::line, Bed::esc);
    let two = |name: &str, reversed: bool| -> Option<Bed> {
        let mut b = Bed::boot(name)?;
        b.start();
        std::thread::sleep(Duration::from_millis(2500));
        b.two_questions(reversed);
        Some(b)
    };

    // ---- chat about A: A refused on the transcript, no hook; B answered ---
    let Some(mut b) = two("sibchat", false) else { return };
    b.result("toolu_a", true, "The user doesn't want to proceed with this tool use.");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(b.state(), Some(question.clone()), "B stands after A's refusal");
    b.hook("PostToolUse", &frame("AskUserQuestion", Some("toolu_b"), &qb));
    b.result("toolu_b", false, "Your questions have been answered.");
    b.c.await_state(b.sid, "running on B's answer, not held on A", |s| *s == SessionState::Running);

    // ---- Esc on both, A held: one write, A's end and its interrupt --------
    let Some(mut b) = two("sibesc", true) else { return };
    b.append_all(&[
        Bed::result_of("toolu_a", true, STOP),
        Bed::result_of("toolu_b", true, STOP),
        esc(),
    ]);
    b.c.await_state(b.sid, "interrupted by the held call's Esc", |s| *s == interrupted);

    // ---- Esc on both, B held, B's refusal written first (the mod road's
    // order, live): A's refusal between B's end and the interrupt is the
    // sibling's, and the interrupt is B's.
    let Some(mut b) = two("sibescrev", false) else { return };
    b.append_all(&[
        Bed::result_of("toolu_b", true, STOP),
        Bed::result_of("toolu_a", true, STOP),
        esc(),
    ]);
    b.c.await_state(b.sid, "interrupted by the held call's Esc", |s| *s == interrupted);

    // ---- A's result read before A's hook, A held: the seat stays on B -----
    let Some(mut b) = two("sibtailfirst", true) else { return };
    let before = b.departures().len();
    b.result("toolu_a", false, "Your questions have been answered.");
    wait_until(Duration::from_secs(10), "the tail read A's end", || b.departures().len() > before);
    b.hook("PostToolUse", &frame("AskUserQuestion", Some("toolu_a"), &qa));
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(
        b.state(),
        Some(question.clone()),
        "B stands: {:?}",
        b.decisions().iter().rev().take(4).collect::<Vec<_>>()
    );
    b.hook("PostToolUse", &frame("AskUserQuestion", Some("toolu_b"), &qb));
    b.result("toolu_b", false, "Your questions have been answered.");
    b.c.await_state(b.sid, "running on B's answer", |s| *s == SessionState::Running);

    // ---- a late answer of an earlier message moves nothing ----------------
    // Plans w1 and y1 in one message, both accepted on the transcript; the
    // next message's plan w2 is held when w1's `PostToolUse` arrives. The
    // seat is not moved to y1, w1's sibling, and w2's own answer leaves it.
    let Some(mut b) = Bed::boot("sibolder") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    let plan = Bed::input("ExitPlanMode");
    let planned = SessionState::RequiresAction { reason: Reason::Plan };
    b.append_all(&[
        line("msg_1", "ExitPlanMode", "toolu_w1", &plan),
        line("msg_1", "ExitPlanMode", "toolu_y1", &plan),
    ]);
    b.hook("PreToolUse", &frame("ExitPlanMode", Some("toolu_w1"), &plan));
    b.hook("PreToolUse", &frame("ExitPlanMode", Some("toolu_y1"), &plan));
    b.c.await_state(b.sid, "on the plans", |s| *s == planned);
    b.append_all(&[
        Bed::result_of("toolu_w1", false, "User has approved your plan."),
        Bed::result_of("toolu_y1", false, "User has approved your plan."),
        line("msg_2", "ExitPlanMode", "toolu_w2", &plan),
    ]);
    b.hook("PreToolUse", &frame("ExitPlanMode", Some("toolu_w2"), &plan));
    std::thread::sleep(Duration::from_millis(2500));
    b.hook("PostToolUse", &frame("ExitPlanMode", Some("toolu_w1"), &plan));
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(
        b.state(),
        Some(planned.clone()),
        "w2 stands: {:?}",
        b.decisions().iter().rev().take(4).collect::<Vec<_>>()
    );
    b.hook("PostToolUse", &frame("ExitPlanMode", Some("toolu_w2"), &plan));
    b.result("toolu_w2", false, "User has approved your plan.");
    b.c.await_state(b.sid, "running on w2's answer", |s| *s == SessionState::Running);
}

/// T-988 round 2 (T-996's second read). Two questions in one message, B held.
/// - **A `Stop` before A's late completion**, B still open on the transcript:
///   the stated end wins, `Idle{EndTurn}`, and B is neither held again nor
///   drawn.
/// - **A cursor minted after both calls' lines**, before their refusals (the
///   seat reached both dialogs before the first transcript poll): the
///   measured mod order, B's refusal, A's, the interrupt, ends interrupted
///   at High, A's refusal read as B's sibling's.
/// - **A split read across a committed `Running`**: the held call's refusal
///   in one read, its `Running` committed, then the sibling's refusal and
///   the interrupt in the next: interrupted at High, unpinned and under the
///   flap pin, with the hooks in order and reversed (round 3).
#[test]
fn a_stated_end_and_a_split_read_keep_the_held_call_s_end() {
    const STOP: &str =
        "The user doesn't want to proceed with this tool use. STOP what you are doing.";
    let interrupted =
        SessionState::Idle { stop_reason: mesimon_core::board::StopReason::Interrupted };
    let qa = Bed::ask("Which colour?");
    let confidence = |b: &mut Bed| {
        board_of(b.c.request(Command::Snapshot))
            .sessions
            .iter()
            .find(|s| s.id == b.sid)
            .map(|s| s.confidence)
    };
    let needs = |b: &mut Bed| match b.c.send(
        mesimon_core::Principal::Agent { session: b.sid },
        Command::AgentGetTicket { full: false },
    ) {
        Response::AgentTicket { ticket } => ticket.needs_you,
        other => panic!("get_ticket: {other:?}"),
    };

    // ---- a Stop, then A's late completion with B open ---------------------
    let Some(mut b) = Bed::boot("sibstop") else { return };
    b.start();
    std::thread::sleep(Duration::from_millis(2500));
    b.two_questions(false);
    // The tail's first look at the held dialogs (it re-reads an open call
    // once a minute), so only the completion meets the Stop's settle.
    std::thread::sleep(Duration::from_millis(2500));
    b.hook("Stop", r#"{"stop_hook_active":false}"#);
    b.hook("PostToolUse", &Bed::frame("AskUserQuestion", Some("toolu_a"), &qa));
    b.c.await_state(b.sid, "the Stop's end", |s| {
        *s == SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn }
    });
    std::thread::sleep(Duration::from_millis(2000));
    assert_eq!(
        b.state(),
        Some(SessionState::Idle { stop_reason: mesimon_core::board::StopReason::EndTurn })
    );
    assert!(needs(&mut b).is_none(), "B is not drawn again");
    assert_eq!(b.held_call(), None);

    // ---- a cursor minted after both calls' lines ---------------------------
    let Some(mut b) = Bed::boot("sibminted") else { return };
    b.start();
    b.hook("Stop", r#"{"stop_hook_active":false}"#);
    b.c.await_state(b.sid, "at rest", |s| matches!(s, SessionState::Idle { .. }));
    // At rest: a tail read forgets the cursor.
    std::thread::sleep(Duration::from_millis(2500));
    let qb = Bed::ask("Which size?");
    b.append_all(&[
        serde_json::json!({"uuid": "p2", "type": "user", "message": {"content": "ask me"}}),
        Bed::line("msg_1", "AskUserQuestion", "toolu_a", &qa),
        Bed::line("msg_1", "AskUserQuestion", "toolu_b", &qb),
    ]);
    b.prompt();
    b.two_questions_heard(false);
    // A poll mints the cursor at the end of the file, after both lines.
    std::thread::sleep(Duration::from_millis(2500));
    b.append_all(&[
        Bed::result_of("toolu_b", true, STOP),
        Bed::result_of("toolu_a", true, STOP),
        Bed::esc(),
    ]);
    b.c.await_state(b.sid, "interrupted by the held call's Esc", |s| *s == interrupted);
    assert_eq!(confidence(&mut b), Some(mesimon_core::board::Confidence::High));

    // ---- a split read across a committed Running, unpinned and pinned -----
    // In order (B held, B's refusal first), and reversed (A's `PreToolUse`
    // heard last, so A is held, and A's refusal first: B's, read after
    // A's `Running` committed, is the sibling's whichever branch reads it).
    for (pinned, reversed) in [(false, false), (true, false), (false, true), (true, true)] {
        let (held, sibling) =
            if reversed { ("toolu_a", "toolu_b") } else { ("toolu_b", "toolu_a") };
        let name = format!("sibsplit{}{}", u8::from(pinned), u8::from(reversed));
        let Some(mut b) = Bed::boot(&name) else { return };
        b.start();
        if pinned {
            // More than four committed changes in 20 s arm the pin.
            for _ in 0..3 {
                b.hook("Stop", r#"{"stop_hook_active":false}"#);
                b.c.await_state(b.sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
                b.prompt();
            }
        } else {
            std::thread::sleep(Duration::from_millis(2500));
        }
        b.two_questions(reversed);
        b.result(held, true, STOP);
        b.c.await_state(b.sid, "running on the held call's refusal", |s| {
            *s == SessionState::Running
        });
        let armed =
            b.decisions().iter().rev().find(|d| d["source"] == "dialog_result").is_some_and(|d| {
                d["decision"]["after"]["pinned_until"]
                    .as_u64()
                    .is_some_and(|until| until > d["at_ms"].as_u64().unwrap())
            });
        assert_eq!(armed, pinned, "the pin as this case wants it");
        b.append_all(&[Bed::result_of(sibling, true, STOP), Bed::esc()]);
        b.c.await_state(b.sid, "interrupted by the held call's Esc", |s| *s == interrupted);
        assert_eq!(
            confidence(&mut b),
            Some(mesimon_core::board::Confidence::High),
            "the held call's own end (pinned: {pinned}, reversed: {reversed})"
        );
    }
}
