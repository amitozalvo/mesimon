//! T-1225, T-1232, T-1233: an agy session through the real daemon, with a
//! stand-in for agy that paints its box and its trust dialog, takes its
//! keys, runs the hooks and the title command the launch registered, and
//! dials the board's server as agy does. What agy itself does was measured
//! on 1.5.8 and 1.5.9 (STALE-MAP, T-1225, T-1232, T-1233).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use mesimon_core::board::{
    AgentProvider, AgyMode, Reason, SessionKind, SessionRecord, SessionState, StopReason,
    WorkspaceStrategy,
};
use mesimon_core::command::{Command, Response};

const FAKE_AGY: &str = include_str!("../common/fake_agy.py");
/// The id the stand-in mints at its first prompt.
const MINTED: &str = "11111111-2222-4333-8444-555555555555";

struct Bed {
    h: Harness,
}

impl Bed {
    /// A daemon whose `agy` is the stand-in. `on` is the switch; `trusted`
    /// writes the person's own agy settings naming the checkout, so no
    /// dialog stands at a start there.
    fn boot(name: &str, on: bool, trusted: bool) -> Option<Self> {
        let mut env = vec![("MESIMON_SLEEP_MIN_AGE_MS", "0"), ("MESIMON_PANE_QUIET_MS", "600000")];
        if on {
            env.push(("MESIMON_AGY", "1"));
        }
        let h = Harness::boot_staged(name, None, &env, |owner, repo| {
            init_repo(repo, "README.md", "agy fixture.\n");
            let fake = owner.dir.join("fake_agy.py");
            std::fs::write(&fake, FAKE_AGY).unwrap();
            std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .unwrap();
            let home = owner.dir.join("agy-home");
            if trusted {
                std::fs::create_dir_all(&home).unwrap();
                let list = serde_json::json!({ "trustedWorkspaces": [repo.display().to_string()] });
                std::fs::write(home.join("settings.json"), list.to_string()).unwrap();
            }
            owner.set_env("MESIMON_AGY_BIN", &fake);
            owner.set_env("MESIMON_AGY_HOME", &home);
        })?;
        Some(Self { h })
    }

    fn cue(&self, word: &str) {
        let dir = self.h.dir.join("agy-cue");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(word), "").unwrap();
    }

    /// What the hooks printed, one `(event, stdout)` per handler that did.
    fn printed(&self) -> Vec<(String, String)> {
        self.jsonl::<serde_json::Value>("agy-stdout.jsonl")
            .into_iter()
            .map(|v| (v["event"].as_str().unwrap().into(), v["stdout"].as_str().unwrap().into()))
            .collect()
    }

    /// Every argv the stand-in was started with, in order.
    fn launches(&self) -> Vec<Vec<String>> {
        self.jsonl("agy-argv.jsonl")
    }

    /// Every prompt the stand-in ran, as `(how, text)`: a `turn` its box
    /// submitted, a `queued` one run after a turn's `Stop`, a `steer` its
    /// alt+m handed to a running turn.
    fn prompts(&self) -> Vec<(String, String)> {
        self.jsonl::<serde_json::Value>("agy-prompts.jsonl")
            .into_iter()
            .map(|v| (v["how"].as_str().unwrap().into(), v["text"].as_str().unwrap().into()))
            .collect()
    }

    fn jsonl<T: serde::de::DeserializeOwned>(&self, name: &str) -> Vec<T> {
        std::fs::read_to_string(self.h.dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn await_prompt(&self, n: usize, how: &str, text: &str) {
        wait_until(Duration::from_secs(15), &format!("prompt {n}: {how} {text:?}"), || {
            self.prompts().get(n).is_some_and(|(h, t)| h == how && t == text)
        });
    }

    /// What the stand-in's box holds.
    fn box_text(&self) -> String {
        std::fs::read_to_string(self.h.dir.join("agy-box.txt")).unwrap_or_default()
    }

    fn keys(&self, rec: &SessionRecord, keys: &[&str]) {
        tmux(&self.h.paths.tmux_sock())
            .args(["send-keys", "-t", &rec.sid16()])
            .args(keys)
            .output()
            .unwrap();
    }
}

fn ticket(c: &mut TestClient, title: &str) -> ulid::Ulid {
    match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: Some(WorkspaceStrategy::SharedCheckout),
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        response => panic!("create ticket: {response:?}"),
    }
}

fn start(c: &mut TestClient, ticket: ulid::Ulid, submit_prompt: bool) -> Response {
    c.request(Command::SpawnSession { ticket, kind: SessionKind::Agy, submit_prompt, plan: false })
}

fn pair<'a>(argv: &'a [String], flag: &str) -> Vec<&'a str> {
    argv.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
}

fn agy_board(c: &mut TestClient) {
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Agy }),
        Response::Ok
    ));
    assert_eq!(c.board().agent_provider, AgentProvider::Agy);
}

/// A child's Stop reaches the parent's hook socket. The parent can also
/// finish its own invocation while the title still lists a running child.
#[test]
fn agy_keeps_the_ticket_in_progress_until_the_whole_seat_finishes() {
    let Some(bed) = Bed::boot("agy-subagents", true, true) else { return };
    let mut c = bed.h.client("agy-subagents");
    agy_board(&mut c);
    let t = ticket(&mut c, "Inspect the project");
    let Response::Spawned { id, .. } = start(&mut c, t, true) else { panic!("spawn agy") };
    c.await_state(id, "lead working", |s| *s == SessionState::Running);
    let report = |event, payload: serde_json::Value| {
        hook_send(&bed.h.paths.hook_sock(), &id.to_string(), event, &payload.to_string());
    };
    let title = |status| {
        serde_json::json!({
            "conversation_id": MINTED, "agent_state": "idle", "subagents": [{"status": status}]
        })
    };
    report("AgyTitle", title("running"));
    report("AgyStop", serde_json::json!({"conversationId":"abc123", "fullyIdle": true}));
    report("AgyStop", serde_json::json!({"conversationId": MINTED, "fullyIdle": true}));
    // Give an erroneous leave its full settle and a daemon tick to move the card.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(c.record(id).state, SessionState::Running);
    assert_eq!(c.board().ticket(t).unwrap().column, "IN PROGRESS");
    report("AgyTitle", title("completed"));
    wait_until(Duration::from_secs(10), "the finished ticket reaches REVIEW", || {
        c.board().ticket(t).unwrap().column == "REVIEW"
    });
    assert_eq!(c.record(id).state, SessionState::Idle { stop_reason: StopReason::EndTurn });
}

/// A start that carries words, a turn with a confirmation in it, words into
/// the live box at rest and mid-turn (queued, and handed over by alt+m), an
/// Esc, a park and a wake with words: every prompt goes in by the box, the
/// seat's state follows agy's own reports, and the wake names the
/// conversation agy minted. Nothing is written into the checkout.
#[test]
fn an_agy_seat_takes_words_by_its_box_and_wakes_into_its_conversation() {
    let Some(bed) = Bed::boot("agy-turn", true, true) else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    agy_board(&mut c);
    // The board's tools and the brief, both on (T-1232).
    assert!(matches!(c.request(Command::SetMcpTools { on: true }), Response::Ok));
    assert!(matches!(c.request(Command::SetSystemPrompt { on: true }), Response::Ok));
    let title = "Teach the parser a new word";
    let t = ticket(&mut c, title);

    // The start's words are the title, pasted into the box once it draws.
    let id = match start(&mut c, t, true) {
        Response::Spawned { id, .. } => id,
        response => panic!("start: {response:?}"),
    };
    bed.await_prompt(0, "turn", title);
    // No column says a mode: the person's own agy default, no flag (T-1234).
    assert!(pair(&bed.launches()[0], "--mode").is_empty(), "{:?}", bed.launches());
    c.await_state(id, "agy working", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(5), "the conversation id", || {
        c.record(id).agy_conversation_id.as_deref() == Some(MINTED)
    });
    let launched = bed.launches();
    assert_eq!(launched.len(), 1, "{launched:?}");
    assert!(!launched[0].iter().any(|a| a.contains(title)), "no words on argv: {launched:?}");
    assert!(!launched[0].iter().any(|a| a.starts_with("--prompt")), "{launched:?}");
    let folder = PathBuf::from(pair(&launched[0], "--add-dir")[0]);
    assert!(folder.starts_with(&bed.h.paths.state_dir), "{folder:?}");
    assert!(!launched[0].iter().any(|a| a.contains("dangerously")), "{launched:?}");
    let settings = PathBuf::from(pair(&launched[0], "--settings")[0]);

    // The board's server beside the hooks, on this board's socket and this
    // session, and one allow rule for its tools (T-1232).
    let read = |path: &Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };
    let config = read(&folder.join(".agents/mcp_config.json"));
    let args: Vec<String> = config["mcpServers"]["mesimon"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    assert_eq!(pair(&args, "--sock"), [bed.h.paths.orch_sock().to_str().unwrap()]);
    assert_eq!(pair(&args, "--session"), [id.to_string().as_str()]);
    assert_eq!(read(&settings)["permissions"]["allow"], serde_json::json!(["mcp(mesimon/*)"]));
    // The shim answered as agy dials it: `server/discover` refused at once,
    // then the older handshake and the board's tools.
    wait_until(Duration::from_secs(10), "the stand-in's server probe", || {
        bed.h.dir.join("agy-mcp.json").exists()
    });
    let mcp = read(&bed.h.dir.join("agy-mcp.json"));
    assert_eq!(mcp["server/discover"]["error"]["code"], -32601, "{mcp}");
    assert_eq!(mcp["initialize"]["result"]["protocolVersion"], "2025-11-25", "{mcp}");
    let tools = mcp["tools/list"]["result"]["tools"].as_array().expect("tools/list");
    assert!(tools.iter().any(|t| t["name"] == "get_ticket"), "{mcp}");

    // A pending confirmation asks the person; its answer is work again.
    bed.cue("ask");
    c.await_state(id, "agy asking", |s| {
        *s == (SessionState::RequiresAction { reason: Reason::Permission })
    });
    bed.cue("answer");
    c.await_state(id, "agy working again", |s| *s == SessionState::Running);
    bed.cue("stop");
    c.await_state(id, "agy's turn ended", |s| {
        *s == (SessionState::Idle { stop_reason: StopReason::EndTurn })
    });

    // Words for the live pane at rest: a turn of their own.
    assert!(matches!(c.prompt(t, "and now the tests"), Response::Ok));
    bed.await_prompt(1, "turn", "and now the tests");
    c.await_state(id, "agy on the asked turn", |s| *s == SessionState::Running);

    // Mid-turn, an Enter queues them under the turn: agy runs them after
    // its `Stop`, and the box reading empty was their receipt.
    assert!(matches!(c.prompt(t, "then the docs"), Response::Ok));
    wait_until(Duration::from_secs(10), "the words queued", || bed.box_text().is_empty());
    assert_eq!(bed.prompts().len(), 2, "nothing ran yet: {:?}", bed.prompts());
    bed.cue("stop");
    bed.await_prompt(2, "queued", "then the docs");
    c.await_state(id, "agy on the queued turn", |s| *s == SessionState::Running);

    // `immediately` is agy's alt+m: the running turn takes the words.
    let now = Command::PromptSession {
        ticket: t,
        text: "look at the tests first".into(),
        queued: false,
        immediately: true,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: Vec::new(),
    };
    assert!(matches!(c.request(now), Response::Ok));
    bed.await_prompt(3, "steer", "look at the tests first");

    // An Esc fires no `Stop`: the feed's idle under a running turn ends it.
    let rec = c.record(id);
    bed.keys(&rec, &["Escape"]);
    c.await_state(id, "agy interrupted", |s| {
        *s == (SessionState::Idle { stop_reason: StopReason::Interrupted })
    });

    // The column's agy mode (T-1234) reaches the wake's argv; a column
    // Claude mode is not agy's. The ticket's column as it stands now:
    // automove may have moved it.
    let board = c.board();
    let column = board.ticket(t).unwrap().column.clone();
    let mut settings = board.column(&column).unwrap().settings.clone();
    settings.agy_mode = AgyMode::AcceptEdits;
    settings.claude_mode = mesimon_core::board::ClaudeMode::Plan;
    assert!(matches!(
        c.request(Command::SetColumnSettings { name: column, settings }),
        Response::Ok
    ));

    // Park, and wake with words: agy's own conversation, the words by the box.
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    c.await_state(id, "parked", |s| matches!(s, SessionState::Sleeping));
    assert!(!matches!(c.prompt(t, "wake up and go"), Response::Err { .. }));
    wait_until(Duration::from_secs(10), "the wake's launch", || bed.launches().len() == 2);
    let woke = &bed.launches()[1];
    assert_eq!(pair(woke, "--conversation"), [MINTED]);
    assert_eq!(pair(woke, "--add-dir"), [folder.to_str().unwrap()]);
    assert_eq!(pair(woke, "--mode"), ["accept-edits"], "{woke:?}");
    bed.await_prompt(4, "turn", "wake up and go");
    c.await_state(id, "agy on the wake's turn", |s| *s == SessionState::Running);

    // One handler printed, once: the brief's, on the one `SessionStart` (the
    // first prompt's; a wake has none), as agy's system message. No reporter
    // printed a byte: an empty object on a tool hook is agy's refusal of the
    // call.
    let printed = bed.printed();
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert_eq!(printed[0].0, "SessionStart");
    let answer: serde_json::Value = serde_json::from_str(printed[0].1.trim()).unwrap();
    assert_eq!(
        answer["injectSteps"][0]["systemMessage"]["systemMessage"],
        mesimon_core::brief::TEXT,
        "{answer}"
    );
    // Nothing of mesimon's is in the checkout or in agy's own folder.
    assert!(!Path::new(&bed.h.repo).join(".agents").exists());
    let home: Vec<_> = std::fs::read_dir(bed.h.dir.join("agy-home"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(home, ["settings.json"], "agy's own folder holds the person's file alone");

    // `agy models` was read off the writer, once, into the machine's file
    // the tier page reads (T-1234), in agy's words.
    let models = bed.h.paths.state_dir.parent().unwrap().join("agy-models.tsv");
    wait_until(Duration::from_secs(10), "the models file", || models.exists());
    assert_eq!(
        std::fs::read_to_string(&models).unwrap(),
        "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n\
         gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n"
    );
    let calls = std::fs::read_to_string(bed.h.dir.join("agy-models.log")).unwrap();
    assert_eq!(calls.lines().count(), 1, "one read for a daemon whose file is fresh");
}

/// A folder the person never trusted gets agy's own dialog: the seat reads
/// TRUST, the board's accept walks it (Enter on its `> Yes` row), and a
/// plain start's title is typed into the box once it draws, never sent.
/// T-1237: an agy seat's transcript is the hooks' path, never the title
/// feed's; its card and the phone read its words off it, its ticket is
/// counted from its model steps with a context reading against the feed's
/// window, and the plan's quota is the machine's agy reading, with no
/// sign-in kept anywhere the board shows.
#[test]
fn an_agy_seat_s_transcript_cost_context_and_quota_reach_the_board() {
    let Some(bed) = Bed::boot("agy-words", true, true) else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    agy_board(&mut c);
    let title = "Count the parser's words";
    let t = ticket(&mut c, title);
    let id = match start(&mut c, t, true) {
        Response::Spawned { id, .. } => id,
        response => panic!("start: {response:?}"),
    };
    bed.await_prompt(0, "turn", title);
    c.await_state(id, "agy working", |s| *s == SessionState::Running);
    let want = bed
        .h
        .dir
        .join("agy-brain")
        .join(MINTED)
        .join(".system_generated/logs/transcript_full.jsonl");
    wait_until(Duration::from_secs(5), "the hooks' transcript path", || {
        c.record(id).transcript_path.as_deref() == want.to_str()
    });
    // Mid-turn the card reads the person's words and the agent thinking.
    let peek = |path: &Path| {
        mesimon_daemon::agents::read_preview(SessionKind::Agy, path).expect("a preview")
    };
    let mid = peek(&want);
    assert_eq!(mid.text, Some(format!("> {title}")));
    assert_eq!(mid.activity, Some(mesimon_daemon::agents::AgentActivity::Thinking));

    bed.cue("stop");
    c.await_state(id, "agy's turn ended", |s| matches!(s, SessionState::Idle { .. }));
    let path = c.record(id).transcript_path.expect("the path stands");
    assert_eq!(Path::new(&path), want, "the title feed's path never replaced it");
    let done = format!("done: {title}");
    assert_eq!(peek(&want).text.as_deref(), Some(done.as_str()));
    let page = mesimon_daemon::agents::read_transcript(
        SessionKind::Agy,
        &want,
        mesimon_daemon::agents::transcript::Ask::default(),
    )
    .expect("a page");
    let rows: Vec<&str> = page.rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(rows, [title, done.as_str()], "the phone's rows, no metadata");

    // The Stop asks the cost pass; the quota came with the title reports.
    let read = |c: &mut TestClient| match c.request(Command::Snapshot) {
        Response::Board { usage, costs, .. } => (usage, costs),
        other => panic!("snapshot: {other:?}"),
    };
    let mut seen = None;
    wait_until(Duration::from_secs(20), "the ticket's agy count", || {
        let (usage, costs) = read(&mut c);
        let cost = costs.into_iter().find(|x| x.ticket == t);
        let ready = cost.as_ref().is_some_and(|x| x.tokens > 0) && usage.agy.reading.is_some();
        seen = Some((usage, cost));
        ready
    });
    let (usage, cost) = seen.unwrap();
    let cost = cost.unwrap();
    assert_eq!(cost.tokens, 14_030, "one model step: 2000 in, 12000 cached, 30 out");
    assert_eq!(cost.unpriced, 0, "Gemini is priced");
    assert!(cost.usd > 0.0);
    assert_eq!(cost.context_pct, Some(1), "14,000 of the feed's 1,048,576");
    let reading = usage.agy.reading.expect("agy's quota");
    assert_eq!(reading.plan.as_deref(), Some("Google AI Pro"));
    let windows: Vec<(String, String)> =
        reading.windows.iter().map(|w| (w.label.clone(), w.percent_word())).collect();
    assert_eq!(windows, [("5h".into(), "25%".into()), ("week".into(), "10%".into())]);
    let shown = serde_json::to_string(&c.request(Command::Snapshot)).unwrap();
    assert!(!shown.contains("example.invalid"), "the sign-in is never kept");
    let _ = c.request(Command::SleepSession { id });
}

#[test]
fn an_agy_trust_dialog_is_accepted_and_the_title_waits_in_the_box() {
    let Some(bed) = Bed::boot("agy-trust", true, false) else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    agy_board(&mut c);
    let title = "Rename the flag";
    let t = ticket(&mut c, title);
    let id = match start(&mut c, t, false) {
        Response::Spawned { id, .. } => id,
        response => panic!("start: {response:?}"),
    };
    c.await_state(id, "agy at its trust dialog", |s| {
        *s == (SessionState::RequiresAction { reason: Reason::Trust })
    });
    assert_eq!(bed.box_text(), "", "nothing typed into the dialog");
    assert!(matches!(c.request(Command::AcceptTrust { ticket: t }), Response::Ok));
    c.await_state(id, "agy at its prompt", |s| matches!(s, SessionState::Idle { .. }));
    wait_until(Duration::from_secs(10), "the title in the box", || {
        bed.box_text() == format!("{title} ")
    });
    assert_eq!(c.record(id).typed_title.as_deref(), Some(title));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(bed.prompts().is_empty(), "the title is the person's to send: {:?}", bed.prompts());
    assert!(bed.printed().is_empty(), "{:?}", bed.printed());
}

/// Without its switch nothing of agy's starts, whatever names it.
#[test]
fn agy_starts_nowhere_without_its_switch() {
    let Some(bed) = Bed::boot("agy-off", false, false) else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    // A board may name agy all the same: a start's kind follows its tier.
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Agy }),
        Response::Ok
    ));
    let t = ticket(&mut c, "Quiet");
    err_containing(start(&mut c, t, false), "switched off");
    assert!(c.board().sessions.is_empty());
    assert!(bed.launches().is_empty());
    assert!(!bed.h.dir.join("agy-models.log").exists(), "no `agy models` either");
}
