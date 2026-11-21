//! T-1225, T-1232, T-1233: an agy session through the real daemon, with a
//! stand-in for agy that paints its box and its trust dialog, takes its
//! keys, runs the hooks and the title command the launch registered, and
//! dials the board's server as agy does. What agy itself does was measured
//! on 1.5.8 and 1.5.9 (STALE-MAP, T-1225, T-1232, T-1233).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::*;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Proc, Stdio};
use std::time::Duration;

use mesimon_core::board::{
    AgentProvider, AgyMode, CrownMode, Reason, SessionKind, SessionRecord, SessionState,
    StopReason, WorkspaceStrategy,
};
use mesimon_core::command::{Command, Response};
use mesimon_core::mesophon::{
    self as api, Answer, DialogAnswer, LocalAction, PermissionDecision, QuestionAnswer, Reply,
};
use mesimon_core::Principal;
use mesimon_team::{
    control::{Auth, Channel, Registration, Wire},
    crypto::{DeviceKeys, ObjectId},
    invite::InviteCode,
    relay::RelayEndpoint,
    wire::Credential,
};

const FAKE_AGY: &str = include_str!("../common/fake_agy.py");
const FAKE_RELAY: &str = include_str!("../common/fake_relay.py");
/// The id the stand-in mints at its first prompt.
const MINTED: &str = "11111111-2222-4333-8444-555555555555";

struct Bed {
    h: Harness,
    relay: Option<Child>,
}

impl Drop for Bed {
    fn drop(&mut self) {
        if let Some(mut relay) = self.relay.take() {
            let _ = relay.kill();
            let _ = relay.wait();
        }
    }
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
        Some(Self { h, relay: None })
    }

    /// A daemon wired to `fake_relay.py` on loopback so a paired `TestPhone`
    /// speaks the sealed control channel end-to-end.
    fn boot_with_relay(name: &str) -> Option<(Self, u16)> {
        let env = [
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_AGY", "1"),
        ];
        let mut relay_child: Option<Child> = None;
        let mut http_port: u16 = 0;
        let h = Harness::boot_staged(name, None, &env, |owner, repo| {
            init_repo(repo, "README.md", "agy fixture.\n");
            let fake = owner.dir.join("fake_agy.py");
            std::fs::write(&fake, FAKE_AGY).unwrap();
            std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .unwrap();
            let home = owner.dir.join("agy-home");
            std::fs::create_dir_all(&home).unwrap();
            let list = serde_json::json!({ "trustedWorkspaces": [repo.display().to_string()] });
            std::fs::write(home.join("settings.json"), list.to_string()).unwrap();
            owner.set_env("MESIMON_AGY_BIN", &fake);
            owner.set_env("MESIMON_AGY_HOME", &home);

            let relay_py = owner.dir.join("fake_relay.py");
            std::fs::write(&relay_py, FAKE_RELAY).unwrap();
            let child = Proc::new("python3")
                .args(["-B", relay_py.to_str().unwrap(), owner.dir.to_str().unwrap()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let info_path = owner.dir.join("relay-info.json");
            wait_until(Duration::from_secs(10), "fake relay listening", || info_path.exists());
            let info: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&info_path).unwrap()).unwrap();
            let tls_port = info["tls_port"].as_u64().unwrap();
            http_port = info["http_port"].as_u64().unwrap() as u16;
            let pin = info["pin"].as_str().unwrap();
            let endpoint = RelayEndpoint::parse(&format!("127.0.0.1:{tls_port} {pin}")).unwrap();
            let mut dev = mesimon_daemon::team::device::DeviceFile::fresh("Host".into(), endpoint);
            dev.credential = Some(Credential::generate());
            let dev_path = owner.dir.join("home/.local/state/mesimon/team/device.toml");
            dev.save(&dev_path).unwrap();
            relay_child = Some(child);
        })?;
        Some((Self { h, relay: relay_child }, http_port))
    }

    fn cue(&self, word: &str) {
        self.cue_with(word, "");
    }

    fn cue_with(&self, word: &str, body: &str) {
        let dir = self.h.dir.join("agy-cue");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(word), body).unwrap();
    }

    fn cue_dialog(&self, spec: serde_json::Value) {
        let dir = self.h.dir.join("agy-cue");
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = self.h.dir.join("dialog.json.tmp");
        std::fs::write(&tmp, spec.to_string()).unwrap();
        std::fs::rename(tmp, dir.join("dialog.json")).unwrap();
    }

    fn decisions(&self) -> Vec<serde_json::Value> {
        self.jsonl("agy-decisions.jsonl")
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
    wait_until(Duration::from_secs(10), "the words queued", || {
        bed.jsonl::<String>("agy-queued.jsonl") == ["then the docs"]
    });
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

    // T-1327: after a sign-in change while `agy-models.tsv` is still fresh,
    // `RefreshAgyModels` re-reads `agy models` on a worker without waiting out
    // `MODELS_TTL_MS`, is refused to an agent, and keeps the last good file if
    // a read lists no model.
    assert!(
        !mesimon_daemon::agents::agy::models_due(
            &models,
            mesimon_core::clock::now_ms(),
        ),
        "the file is well inside its 6 h TTL"
    );
    let denied = c.send(mesimon_core::Principal::Agent { session: id }, Command::RefreshAgyModels);
    assert!(matches!(denied, Response::Err { .. }), "local only: {denied:?}");
    let signed_in = "\
gemini-4-neon-high\tGemini 4 Neon (High)\n\
gemini-4-neon-low\tGemini 4 Neon (Low)\n\
gemini-4-carbon-high\tGemini 4 Carbon (High)\n\
claude-opus-5-5-high\tClaude Opus 5.5 (High)\n";
    std::fs::write(bed.h.dir.join("agy-models.txt"), signed_in).unwrap();
    assert!(matches!(c.request(Command::RefreshAgyModels), Response::Ok));
    wait_until(Duration::from_secs(10), "the refreshed models file", || {
        matches!(
            c.request(Command::Snapshot),
            Response::Board { agy_models, .. } if !agy_models.reading && agy_models.error.is_none()
        ) && std::fs::read_to_string(&models).ok().as_deref() == Some(signed_in)
    });
    let calls = std::fs::read_to_string(bed.h.dir.join("agy-models.log")).unwrap();
    assert_eq!(calls.lines().count(), 2, "the refresh ran one worker read");
    std::fs::write(bed.h.dir.join("agy-models.txt"), "").unwrap();
    assert!(matches!(c.request(Command::RefreshAgyModels), Response::Ok));
    wait_until(Duration::from_secs(10), "the empty read's error", || {
        matches!(
            c.request(Command::Snapshot),
            Response::Board { agy_models, .. }
                if !agy_models.reading
                    && agy_models.error.as_deref() == Some("agy models listed no model")
        )
    });
    assert_eq!(
        std::fs::read_to_string(&models).unwrap(),
        signed_in,
        "an empty read leaves the last good file in place"
    );

    // T-1238: the launch's `PreToolUse` write gate passes ordinary writes with
    // zero bytes on stdout and denies `.mesimon/` and the session's own folder
    // (`--add-dir` puts it in agy's workspace) with `{"decision":"deny",…}`,
    // reporting `GateDenied` to the daemon's activity feed.
    bed.cue_with(
        "gate-1-ok",
        &serde_json::json!({"name": "write_file", "args": {"Path": "src/ok.rs"}}).to_string(),
    );
    bed.cue_with(
        "gate-2-board",
        &serde_json::json!({
            "name": "write_to_file",
            "args": {"TargetFile": ".mesimon/board/columns.toml"}
        })
        .to_string(),
    );
    bed.cue_with(
        "gate-3-session",
        &serde_json::json!({
            "name": "edit_file",
            "args": {"Path": folder.join(".agents/hooks.json").display().to_string()}
        })
        .to_string(),
    );
    wait_until(Duration::from_secs(10), "the three PreToolUse gate runs", || {
        bed.jsonl::<serde_json::Value>("agy-gate.jsonl").len() == 3
    });
    let gates = bed.jsonl::<serde_json::Value>("agy-gate.jsonl");
    assert_eq!(gates[0]["code"], 0);
    assert_eq!(gates[0]["stdout"], "", "an ordinary write must emit zero bytes");
    for denied in [&gates[1], &gates[2]] {
        assert_eq!(denied["code"], 0);
        let out: serde_json::Value =
            serde_json::from_str(denied["stdout"].as_str().unwrap()).unwrap();
        assert_eq!(out["decision"], "deny", "{out}");
        assert!(out["reason"].as_str().unwrap().contains("mesimon"), "{out}");
    }
    assert_eq!(
        bed.printed().len(),
        3,
        "only SessionStart brief and the two denied PreToolUse calls printed: {:?}",
        bed.printed()
    );
    wait_until(Duration::from_secs(5), "GateDenied in activity feed", || {
        let feed =
            std::fs::read_to_string(bed.h.paths.state_dir.join("activity.jsonl")).unwrap_or_default();
        feed.contains("board_dir") && feed.contains("state_dir")
    });
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
fn agy_sign_in_waits_for_the_person_then_delivers_the_launch() {
    let Some((bed, port)) = Bed::boot_with_relay("agy-sign-in") else { return };
    bed.h.set_env("MESIMON_COMPOSER_WAIT_MS", "3000");
    bed.h.restart();
    let mut c = bed.h.client("agy-sign-in");
    agy_board(&mut c);
    let mut phone = TestPhone::pair(&mut c, port);

    for (submit, title) in [(false, "A title to edit"), (true, "A title to submit")] {
        bed.cue("signed-out");
        let t = ticket(&mut c, title);
        let Response::Spawned { id, .. } = start(&mut c, t, submit) else { panic!("spawn agy") };
        c.await_state(id, "agy at sign-in", |s| {
            *s == (SessionState::RequiresAction { reason: Reason::Auth })
        });
        assert!(c.record(id).on_agy_sign_in());
        let Response::AgentTicket { ticket: view } =
            c.send(Principal::Agent { session: id }, Command::AgentGetTicket { full: false })
        else {
            panic!("get_ticket")
        };
        let stop = view.needs_you.unwrap();
        assert!(stop.reason.contains("sign in") && stop.reason.contains("only the person"));
        assert!(stop.request.is_none());
        let agent = phone.agent(t);
        assert_eq!(agent.state, "needs attention");
        assert_eq!(agent.startup_notice.as_deref(), Some("agy asks you to sign in ∙ open its pane"));
        assert!(agent.dialog.is_none() && agent.trust.is_none() && agent.permission.is_none());
        assert!(!matches!(c.request(Command::AcceptTrust { ticket: t }), Response::Ok));
        err_containing(c.prompt(t, "words from the board"), "sign in");
        let doctor = bed.h.command(&["doctor", "agents"])
            .env("MESIMON_CLAUDE_BIN", "/bin/echo")
            .env("MESIMON_CODEX_BIN", "/bin/echo")
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&doctor.stdout).contains("agy is waiting for sign-in"));

        // Both delivery clocks would have expired without the sign-in hold.
        std::thread::sleep(Duration::from_millis(3500));
        assert!(bed.prompts().is_empty());
        assert!(!bed.h.dir.join("agy-sign-in-keys.jsonl").exists());
        bed.cue("signed-in");
        if submit {
            bed.await_prompt(0, "turn", title);
            c.await_state(id, "submitted title is working", |s| *s == SessionState::Running);
            assert_eq!(bed.prompts().len(), 1);
        } else {
            wait_until(Duration::from_secs(12), "draft title after sign-in", || {
                bed.box_text() == format!("{title} ")
            });
            assert!(bed.prompts().is_empty());
        }
        assert!(phone.agent(t).startup_notice.is_none());
        bed.cue("quit");
        c.await_state(id, "stand-in exited", |s| !s.has_pane());
    }
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

/// The checkout fact comes from the board's managed binding and reaches the
/// same get_ticket view the MCP shim serializes.
#[test]
fn an_agy_worktree_trust_stop_names_its_checkout_on_the_board_and_in_get_ticket() {
    let Some(bed) = Bed::boot("agy-tree-trust", true, false) else { return };
    let mut c = bed.h.client("agy-tree-trust");
    agy_board(&mut c);
    let Response::Created { id: ticket, .. } = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "Inspect the worktree".into(),
        workspace: Some(WorkspaceStrategy::Worktree),
        tier: None,
    }) else {
        panic!("create worktree ticket")
    };
    assert!(matches!(start(&mut c, ticket, false), Response::Provisioning));
    wait_until(Duration::from_secs(20), "worktree trust dialog", || {
        c.board().live_agent(ticket).is_some_and(SessionRecord::on_trust_dialog)
    });
    let rec = c.board().live_agent(ticket).unwrap().clone();
    let checkout = bed.h.paths.repo_root.display();
    let hint = format!("Trusting {checkout} once in agy covers this board's worktrees");
    assert_eq!(rec.detail, Some(format!("agy asks to trust this folder ∙ {hint}")));
    let Response::AgentTicket { ticket: view } = c.send(
        mesimon_core::Principal::Agent { session: rec.id },
        Command::AgentGetTicket { full: false },
    ) else {
        panic!("get_ticket")
    };
    assert_eq!(view.needs_you.unwrap().reason, format!("trust ∙ {hint}"));
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

struct TestPhone {
    sock: TcpStream,
    keys: DeviceKeys,
    channel: Channel,
    incarnation: String,
    next: u64,
}

impl TestPhone {
    fn pair(c: &mut TestClient, http_port: u16) -> Self {
        let enabled = c.request(Command::Mesophon { action: LocalAction::Enable });
        assert!(matches!(enabled, Response::Mesophon { .. }), "enable: {enabled:?}");
        wait_until_says(
            Duration::from_secs(10),
            "host connected to relay",
            || c.request(Command::Mesophon { action: LocalAction::Status }),
            |r| matches!(r, Response::Mesophon { info } if info.connected),
        );
        let Response::Mesophon { info } =
            c.request(Command::Mesophon { action: LocalAction::Pair })
        else {
            panic!("pair")
        };
        let code = info.code.expect("invite code");

        let mut sock = TcpStream::connect(("127.0.0.1", http_port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        sock.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(
            sock,
            "GET /control HTTP/1.1\r\n\
             Host: 127.0.0.1:{http_port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        sock.flush().unwrap();
        let mut hdr = Vec::new();
        let mut byte = [0u8; 1];
        while !hdr.ends_with(b"\r\n\r\n") {
            sock.read_exact(&mut byte).unwrap();
            hdr.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&hdr).starts_with("HTTP/1.1 101"), "{hdr:?}");

        let keys = DeviceKeys::generate();
        let auth = Auth {
            credential: None,
            register: Some(Registration { name: "Phone".into(), public: keys.public() }),
        };
        Self::ws_write(&mut sock, &serde_json::to_string(&auth).unwrap());
        let first: Wire = serde_json::from_str(&Self::ws_read(&mut sock)).unwrap();
        assert!(matches!(first, Wire::Authenticated { .. }));

        let invite = InviteCode::parse(&code).unwrap();
        let challenge = ObjectId::random();
        let pair_frame = Wire::Pair {
            challenge,
            hash: mesimon_team::hex::encode(&invite.secret_hash()),
            public: keys.public(),
            proof: mesimon_team::hex::encode(&invite.proof(&keys.public())),
        };
        Self::ws_write(&mut sock, &serde_json::to_string(&pair_frame).unwrap());
        let Wire::Welcome { welcome, .. } = serde_json::from_str(&Self::ws_read(&mut sock)).unwrap()
        else {
            panic!("expected welcome")
        };
        assert_eq!(welcome.connection, challenge);
        assert!(invite.names_owner(&welcome.host));
        let (channel, ready) = Channel::client(&welcome, &keys).unwrap();
        let Answer { reply: Reply::Ready { incarnation, next, features }, .. } =
            serde_json::from_value(ready).unwrap()
        else {
            panic!("expected ready")
        };
        assert!(features.iter().any(|f| f == "dialog_multi"), "{features:?}");
        Self { sock, keys, channel, incarnation, next }
    }

    fn ws_write(sock: &mut TcpStream, text: &str) {
        let payload = text.as_bytes();
        let n = payload.len();
        let mut frame = Vec::with_capacity(14 + n);
        frame.push(0x81);
        if n < 126 {
            frame.push(0x80 | n as u8);
        } else if n < 65536 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(n as u64).to_be_bytes());
        }
        frame.extend_from_slice(&[0, 0, 0, 0]);
        frame.extend_from_slice(payload);
        sock.write_all(&frame).unwrap();
        sock.flush().unwrap();
    }

    fn ws_read(sock: &mut TcpStream) -> String {
        loop {
            let mut h = [0u8; 2];
            sock.read_exact(&mut h).unwrap();
            let opcode = h[0] & 0x0f;
            let masked = (h[1] & 0x80) != 0;
            let mut len = (h[1] & 0x7f) as usize;
            if len == 126 {
                let mut b = [0u8; 2];
                sock.read_exact(&mut b).unwrap();
                len = u16::from_be_bytes(b) as usize;
            } else if len == 127 {
                let mut b = [0u8; 8];
                sock.read_exact(&mut b).unwrap();
                len = u64::from_be_bytes(b) as usize;
            }
            let mut mask = [0u8; 4];
            if masked {
                sock.read_exact(&mut mask).unwrap();
            }
            let mut data = vec![0u8; len];
            sock.read_exact(&mut data).unwrap();
            if masked {
                for (i, b) in data.iter_mut().enumerate() {
                    *b ^= mask[i & 3];
                }
            }
            match opcode {
                0x1 => return String::from_utf8(data).unwrap(),
                0x9 => {
                    let mut pong = vec![0x8A, 0x80 | (data.len() as u8), 0, 0, 0, 0];
                    pong.extend_from_slice(&data);
                    sock.write_all(&pong).unwrap();
                    sock.flush().unwrap();
                }
                0xA => {}
                other => panic!("unexpected ws opcode {other}"),
            }
        }
    }

    fn call(&mut self, request: api::Request) -> (u64, Reply) {
        let id = self.next;
        self.next += 1;
        let cmd = api::Command { incarnation: self.incarnation.clone(), id, request };
        let record = self.channel.seal(&self.keys, serde_json::to_value(&cmd).unwrap()).unwrap();
        let pkt = Wire::Packet { peer: String::new(), record };
        Self::ws_write(&mut self.sock, &serde_json::to_string(&pkt).unwrap());
        loop {
            let Wire::Packet { record, .. } =
                serde_json::from_str(&Self::ws_read(&mut self.sock)).unwrap()
            else {
                continue;
            };
            let ans: Answer = serde_json::from_value(self.channel.open(&record).unwrap()).unwrap();
            if ans.id == id {
                return (id, ans.reply);
            }
        }
    }

    fn agent(&mut self, ticket: ulid::Ulid) -> api::Agent {
        let (_, Reply::Board { tickets, .. }) = self.call(api::Request::Snapshot) else {
            panic!("snapshot")
        };
        let want = ticket.to_string();
        tickets.into_iter().find(|t| t.id == want).and_then(|t| t.agent).expect("ticket agent")
    }

    fn await_permission(&mut self, ticket: ulid::Ulid, label: &str) -> api::Permission {
        let mut found = None;
        wait_until(Duration::from_secs(10), label, || {
            found = self.agent(ticket).permission;
            found.is_some()
        });
        found.unwrap()
    }

    fn await_dialog(&mut self, ticket: ulid::Ulid, label: &str) -> api::Dialog {
        let mut found = None;
        wait_until(Duration::from_secs(10), label, || {
            let a = self.agent(ticket);
            if a.dialog.is_some() {
                assert_eq!(a.card_answers, Some(true), "agy dialogs are card-answerable");
            }
            found = a.dialog;
            found.is_some()
        });
        found.unwrap()
    }

    fn settle(&mut self, request: api::Request) -> (String, Option<String>) {
        let (cmd_id, first) = self.call(request);
        match first {
            Reply::Delivery { status, reason, .. } if status != "awaiting_delivery" => {
                return (status, reason);
            }
            Reply::Delivery { .. } => {}
            other => panic!("expected delivery receipt, got {other:?}"),
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            std::thread::sleep(Duration::from_millis(40));
            let (_, reply) = self.call(api::Request::Status { command: cmd_id });
            let Reply::Delivery { status, reason, .. } = reply else {
                panic!("expected status delivery, got {reply:?}");
            };
            if status != "awaiting_delivery" {
                return (status, reason);
            }
            assert!(std::time::Instant::now() < deadline, "delivery {cmd_id} never settled");
        }
    }
}

#[test]
fn board_and_phone_review_the_artifact_file_and_send_changes_after_rejection() {
    let Some((bed, port)) = Bed::boot_with_relay("agy-plan") else { return };
    let mut c = bed.h.client("agy-plan");
    agy_board(&mut c);
    let mut phone = TestPhone::pair(&mut c, port);
    let t = ticket(&mut c, "Plan the greeting");
    let Response::Spawned { id, .. } = c.request(Command::SpawnSession {
        ticket: t,
        kind: SessionKind::Agy,
        submit_prompt: true,
        plan: true,
    }) else {
        panic!("spawn")
    };
    bed.await_prompt(0, "turn", "Plan the greeting");
    let mut old = String::new();
    for (index, feedback) in
        ["Use the short greeting", "Keep the existing title", ""].iter().enumerate()
    {
        let body = format!("# Greeting plan {index}\n\nPreserve the introduction.");
        bed.cue_with("plan", &body);
        c.await_state(id, "file awaiting review", |s| {
            *s == SessionState::RequiresAction { reason: Reason::Plan }
        });
        let mut dialog = phone.await_dialog(t, "artifact on the phone");
        if index == 2 {
            // Same filename on screen and the previous card still projected:
            // the file changed before either the write hook or the next title.
            let file = bed.h.dir.join("agy-home/brain").join(MINTED).join("implementation_plan.md");
            std::fs::write(&file, "A replacement plan").unwrap();
            let (_, stale) = phone.call(api::Request::Dialog {
                ticket: t.to_string(),
                session: id.to_string(),
                request: dialog.request.clone(),
                response: DialogAnswer::Accept,
            });
            assert!(matches!(stale, Reply::Rejected {message} if message.contains("changed")));
            std::fs::write(&file, &body).unwrap();
            let before = dialog.request.clone();
            hook_send(&bed.h.paths.hook_sock(), &id.to_string(), "AgyTitle",
                &serde_json::json!({"conversation_id":MINTED,"cycle_mode":"plan","agent_state":"reviewing","artifact_count":1}).to_string());
            wait_until(Duration::from_secs(5), "fresh card after rewrite", || {
                if let Some(current) = phone.agent(t).dialog {
                    dialog = current;
                }
                dialog.request != before
            });
        }
        assert_eq!(dialog.content, api::DialogContent::Plan { markdown: body.clone() });
        let note = c.record(id).plan_note.unwrap();
        assert!(
            matches!(c.request(Command::ReadNote {ticket:t, note}), Response::Note {text,..} if text == body)
        );
        if index == 0 {
            let receipt = c.request(Command::ReviewPlan {
                ticket: t,
                session: id,
                request: dialog.request.clone(),
                feedback: feedback.to_string(),
            });
            assert!(matches!(receipt, Response::Ok), "{receipt:?}");
        } else {
            assert_ne!(old, dialog.request);
            let (_, stale) = phone.call(api::Request::Dialog {
                ticket: t.to_string(),
                session: id.to_string(),
                request: old.clone(),
                response: DialogAnswer::Accept,
            });
            assert!(matches!(stale, Reply::Rejected {message} if message.contains("changed")));
            let response = if feedback.is_empty() {
                DialogAnswer::Accept
            } else {
                DialogAnswer::Text { text: feedback.to_string() }
            };
            let (status, reason) = phone.settle(api::Request::Dialog {
                ticket: t.to_string(),
                session: id.to_string(),
                request: dialog.request.clone(),
                response,
            });
            assert_eq!(status, "answered", "{reason:?}");
        }
        let decision = if feedback.is_empty() { "Approved" } else { "Rejected" };
        bed.await_prompt(1 + index * 2, "turn", &format!("[{decision}] implementation_plan.md"));
        if !feedback.is_empty() {
            bed.await_prompt(2 + index * 2, "steer", feedback);
        }
        c.await_state(id, "continued turn", |s| *s == SessionState::Running);
        old = dialog.request;
    }
    bed.cue("stop");
    c.await_state(id, "idle before wake", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    assert!(!matches!(
        c.request(Command::PromptSession {
            ticket: t,
            text: "Plan the next change".into(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: true,
            tier: None,
            resend: false,
            uploads: vec![],
        }),
        Response::Err { .. }
    ));
    bed.await_prompt(6, "turn", "Plan the next change");
    let rec = c.record(id);
    assert!(rec.launched_in_plan());
    assert_eq!(pair(&rec.argv, "--conversation"), [MINTED]);
    bed.cue_with("plan-pair", "Review these in the pane");
    wait_until(Duration::from_secs(5), "phone's pane-only plan card", || {
        let agent = phone.agent(t);
        agent.pane_review && agent.dialog.is_none() && agent.card_answers.is_none()
    });
    let (_, stale) = phone.call(api::Request::Dialog {
        ticket: t.to_string(),
        session: id.to_string(),
        request: old,
        response: DialogAnswer::Accept,
    });
    assert!(matches!(stale, Reply::Rejected { .. }));
}

const PLAN_FILE: &str = "implementation_plan.md";

struct ReviewFixture {
    bed: Bed,
    client: TestClient,
    ticket: ulid::Ulid,
    session: uuid::Uuid,
    dir: PathBuf,
}

impl ReviewFixture {
    fn start(case: &str) -> Option<Self> {
        let bed = Bed::boot(case, true, true)?;
        let mut client = bed.h.client(case);
        agy_board(&mut client);
        let ticket = ticket(&mut client, "Review artifact");
        let Response::Spawned { id: session, .. } = client.request(Command::SpawnSession {
            ticket,
            kind: SessionKind::Agy,
            submit_prompt: true,
            plan: true,
        }) else {
            panic!("spawn")
        };
        client.await_state(session, "opening prompt observed", |s| *s == SessionState::Running);
        let dir = bed.h.dir.join("agy-home/brain").join(MINTED);
        std::fs::create_dir_all(&dir).unwrap();
        Some(Self { bed, client, ticket, session, dir })
    }

    fn hook(&self, event: &str, payload: serde_json::Value) {
        hook_send(
            &self.bed.h.paths.hook_sock(),
            &self.session.to_string(),
            event,
            &payload.to_string(),
        );
    }

    fn written(&self, name: &str) {
        self.hook("AgyPostToolUse", serde_json::json!({
            "conversationId": MINTED,
            "artifactDirectoryPath": self.dir,
            "toolCall": {"id":"same-call","name":"write_file","args":{
                "Path":self.dir.join(name), "ArtifactMetadata":{"UserFacing":true,"RequestFeedback":true}
            }}
        }));
    }

    fn reviewing(&self) {
        self.hook("AgyTitle", serde_json::json!({
            "conversation_id":MINTED,"cycle_mode":"plan","agent_state":"reviewing","artifact_count":1
        }));
    }

    fn note(&mut self) -> Option<String> {
        let note = self.client.record(self.session).plan_note?;
        match self.client.request(Command::ReadNote { ticket: self.ticket, note }) {
            Response::Note { text, .. } => Some(text),
            _ => None,
        }
    }

    fn await_note(&mut self, text: &str) -> String {
        wait_until(Duration::from_secs(5), "artifact note", || {
            self.note().as_deref() == Some(text)
        });
        self.client.record(self.session).agy_plan_request.unwrap()
    }

    fn decide(&mut self, request: String) -> Response {
        self.client.request(Command::ReviewPlan {
            ticket: self.ticket,
            session: self.session,
            request,
            feedback: "Keep these words".into(),
        })
    }

    fn open_panel_request(&mut self) -> SessionRecord {
        self.bed.cue_with("plan", "Visible plan");
        self.await_note("Visible plan");
        self.client.record(self.session)
    }
}

fn assert_pane_only(f: &mut ReviewFixture) {
    wait_until(Duration::from_secs(5), "review in agy's pane", || {
        let s = f.client.record(f.session);
        s.on_plan_dialog()
            && s.agy_plan_request.is_none()
            && s.detail.as_deref().is_some_and(|d| d.contains("agy's pane"))
    });
}

#[test]
fn narrow_review_note_write_must_succeed() {
    let Some(mut f) = ReviewFixture::start("blocked-note") else { return };
    let rec = f.open_panel_request();
    let id = rec.plan_note.unwrap();
    let board = f.client.board();
    let key = &board.ticket(f.ticket).unwrap().short_key;
    let notes = f.bed.h.paths.board_dir.join("board/tickets").join(key).join("notes");
    let blocked = notes.join(format!("{id}.tmp"));
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(f.dir.join(PLAN_FILE), "Replacement").unwrap();
    f.written(PLAN_FILE);
    f.reviewing();
    assert_pane_only(&mut f);
    assert_eq!(f.note().as_deref(), Some("Visible plan"));
    assert!(matches!(f.decide(rec.agy_plan_request.unwrap()), Response::Err { .. }));
    std::fs::remove_dir(blocked).unwrap();
    f.reviewing();
    f.await_note("Replacement");
}

#[test]
fn changing_the_saved_plan_note_withdraws_the_decision() {
    let Some(mut f) = ReviewFixture::start("edited-note") else { return };
    let rec = f.open_panel_request();
    let response = f.client.request(Command::WriteNote {
        ticket: f.ticket,
        note: rec.plan_note,
        text: "My annotation".into(),
        rev: None,
    });
    assert!(matches!(response, Response::NoteWritten { .. }));
    assert_pane_only(&mut f);
    f.reviewing();
    assert!(matches!(f.decide(rec.agy_plan_request.unwrap()), Response::Err { .. }));
    assert_eq!(f.note().as_deref(), Some("My annotation"));
}

#[test]
fn narrow_review_ordinary_ask_is_refused() {
    let Some(mut f) = ReviewFixture::start("ordinary-ask") else { return };
    let cmd = Command::PromptSession {
        ticket: f.ticket,
        text: "Draft begun before review".into(),
        queued: false,
        immediately: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend: false,
        uploads: vec![],
    };
    f.open_panel_request();
    assert!(matches!(f.client.request(cmd), Response::Err {message} if message.contains("review")));
    assert_eq!(f.bed.prompts().len(), 1);
    assert!(f.client.record(f.session).on_plan_dialog());
}

#[test]
fn narrow_review_copy_mode_requires_agy_confirmation() {
    let Some(mut f) = ReviewFixture::start("copy-mode") else { return };
    let rec = f.open_panel_request();
    f.bed.keys(&rec, &["C-r"]);
    let sock = f.bed.h.paths.tmux_sock();
    let sid = rec.sid16();
    wait_until(Duration::from_secs(4), "selected row", || {
        let out = tmux(&sock).args(["capture-pane", "-p", "-t", &sid]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).contains("Action required (1 left)")
    });
    assert!(tmux(&sock).args(["copy-mode", "-t", &sid]).status().unwrap().success());
    let reply = f.client.request(Command::ReviewPlan {
        ticket: f.ticket,
        session: f.session,
        request: rec.agy_plan_request.unwrap(),
        feedback: String::new(),
    });
    assert!(matches!(reply, Response::Err { .. }), "{reply:?}");
    assert!(f.client.record(f.session).on_plan_dialog());
    assert_eq!(f.bed.prompts().len(), 1);
}

#[test]
fn narrow_review_two_artifacts_stay_in_the_pane() {
    let Some(mut f) = ReviewFixture::start("two-artifacts") else { return };
    f.bed.cue_with("plan-pair", "Implementation steps");
    assert_pane_only(&mut f);
    assert_eq!(f.bed.prompts().len(), 1);
}

#[test]
fn narrow_review_restart_cannot_name_remaining_artifact() {
    let Some(mut f) = ReviewFixture::start("review-restart") else { return };
    let stale = f.open_panel_request().agy_plan_request.unwrap();
    let rec = f.client.record(f.session);
    f.bed.h.restart_after(|| {
        f.bed.cue_with("plan-pair", "New review while offline");
        let capture = || {
            let output = tmux(&f.bed.h.paths.tmux_sock())
                .args(["capture-pane", "-p", "-t", &rec.sid16()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        wait_until(Duration::from_secs(6), "two artifacts in offline pane", || {
            capture().contains("2 artifacts")
        });
        f.bed.keys(&rec, &["C-r", "Down", "y"]);
        f.bed.await_prompt(1, "turn", "[Approved] implementation_plan.md");
        wait_until(Duration::from_secs(5), "one artifact remains", || {
            capture().contains("1 artifact")
        });
    });
    f.client = f.bed.h.client("review-restart");
    f.reviewing();
    assert_pane_only(&mut f);
    assert!(matches!(f.decide(stale), Response::Err { .. }));
}

#[test]
fn narrow_review_column_without_named_plans_counts_no_acceptance() {
    let Some(mut f) = ReviewFixture::start("column-review") else { return };
    f.open_panel_request();
    let column = f.client.board().ticket(f.ticket).unwrap().column.clone();
    let reply = f.client.request(Command::PromptColumn {
        requests: Default::default(),
        column,
        text: String::new(),
        queued: true,
        accept_plan: true,
    });
    assert!(
        matches!(reply, Response::Err { .. } | Response::Asked { accepts: 0, .. }),
        "{reply:?}"
    );
    assert_eq!(f.bed.prompts().len(), 1);
}

#[test]
fn column_counts_the_named_plan_when_agy_confirms_it() {
    let Some(mut f) = ReviewFixture::start("confirmed-column") else { return };
    let req = f.open_panel_request().agy_plan_request.unwrap();
    let column = f.client.board().ticket(f.ticket).unwrap().column.clone();
    let ask = |request| Command::PromptColumn {
        column: column.clone(),
        text: String::new(),
        queued: true,
        accept_plan: true,
        requests: [(f.session, request)].into(),
    };
    assert!(matches!(
        f.client.request(ask("stale".into())),
        Response::Asked { accepts: 0, failed: 1, .. }
    ));
    let response = f.client.request(ask(req));
    assert!(matches!(response, Response::Asked { accepts: 1, failed: 0, .. }), "{response:?}");
    f.bed.await_prompt(1, "turn", "[Approved] implementation_plan.md");
}

#[test]
fn review_refuses_an_attached_person() {
    let Some(mut f) = ReviewFixture::start("attached") else { return };
    let rec = f.open_panel_request();
    let sock = f.bed.h.paths.tmux_sock();
    let sid = rec.sid16();
    let mut client = tmux(&sock)
        .args(["-C", "attach", "-t", &sid])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until(Duration::from_secs(5), "client attached", || {
        !tmux(&sock).args(["list-clients", "-t", &sid]).output().unwrap().stdout.is_empty()
    });
    let receipt = f.decide(rec.agy_plan_request.unwrap());
    drop(client.stdin.take());
    client.wait().unwrap();
    assert!(matches!(receipt, Response::Err {message} if message.contains("pane open")));
    assert_eq!(f.bed.prompts().len(), 1);
}

#[test]
fn review_refuses_a_composer_draft() {
    let Some(mut f) = ReviewFixture::start("draft") else { return };
    let rec = f.open_panel_request();
    f.bed.keys(&rec, &["draft"]);
    wait_until(Duration::from_secs(5), "draft in the box", || f.bed.box_text() == "draft");
    assert!(
        matches!(f.decide(rec.agy_plan_request.unwrap()), Response::Err {message} if message.contains("draft"))
    );
    assert_eq!(f.bed.box_text(), "draft");
    assert_eq!(f.bed.prompts().len(), 1);
}

#[test]
fn review_regression_rewrite() {
    let Some(mut f) = ReviewFixture::start("rewrite") else { return };
    let file = f.dir.join(PLAN_FILE);
    std::fs::write(&file, "First plan").unwrap();
    f.written(PLAN_FILE);
    f.reviewing();
    let old = f.await_note("First plan");
    std::fs::write(&file, "Second plan").unwrap();
    f.written(PLAN_FILE);
    f.reviewing();
    let new = f.await_note("Second plan");
    assert_ne!(old, new);
    assert!(matches!(f.decide(old), Response::Err {message} if message.contains("changed")));
}

#[test]
fn review_regression_missed_write() {
    let Some(mut f) = ReviewFixture::start("missed-write") else { return };
    std::fs::write(f.dir.join(PLAN_FILE), "Unwitnessed").unwrap();
    f.reviewing();
    assert_pane_only(&mut f);
}

#[test]
fn review_regression_failed_read() {
    let Some(mut f) = ReviewFixture::start("retry-read") else { return };
    f.written(PLAN_FILE);
    f.reviewing();
    wait_until(Duration::from_secs(5), "first title processed", || {
        f.bed.h.feed().contains("AgyPostToolUse")
            && f.bed.h.feed().rsplit("AgyPostToolUse").next().unwrap().contains("AgyTitle")
    });
    assert!(f.note().is_none());
    std::fs::write(f.dir.join(PLAN_FILE), "Now readable").unwrap();
    f.reviewing();
    f.await_note("Now readable");
}

#[test]
fn review_regression_truncation() {
    let Some(mut f) = ReviewFixture::start("truncated") else { return };
    std::fs::write(f.dir.join(PLAN_FILE), "x".repeat(40000)).unwrap();
    f.written(PLAN_FILE);
    f.reviewing();
    wait_until(Duration::from_secs(5), "bounded note", || f.note().is_some());
    let note = f.note().unwrap();
    assert!(note.contains("40000 bytes") && note.contains("truncated"));
    assert!(note.len() <= mesimon_core::board::NOTE_MAX_BYTES);
}

/// T-1235: a paired phone walks every captured agy confirmation and question
/// dialog shape over the sealed control channel, binds each card to its
/// `PreToolUse` call, confirms by the title feed's `working` edge, never
/// walks Option 3 (`Persist to settings.json`) or `always` / `Auto mode`,
/// and `mesimon hook` prints zero bytes on `PreToolUse`.
#[test]
fn an_agy_phone_approves_denies_and_answers_every_captured_dialog_shape() {
    let Some((bed, http_port)) = Bed::boot_with_relay("agy-phone") else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    agy_board(&mut c);
    let mut phone = TestPhone::pair(&mut c, http_port);
    let t = ticket(&mut c, "Drive agy from the phone");
    let Response::Spawned { id, .. } = start(&mut c, t, true) else { panic!("spawn agy") };
    bed.await_prompt(0, "turn", "Drive agy from the phone");
    c.await_state(id, "agy working", |s| *s == SessionState::Running);

    let decide = |phone: &mut TestPhone, req: &str, decision: PermissionDecision| {
        phone.settle(api::Request::Permission {
            ticket: t.to_string(),
            session: id.to_string(),
            request: req.into(),
            decision,
        })
    };
    let answer_q = |phone: &mut TestPhone, req: &str, response: DialogAnswer| {
        phone.settle(api::Request::Dialog {
            ticket: t.to_string(),
            session: id.to_string(),
            request: req.into(),
            response,
        })
    };

    // 1. File creation (`write_to_file`).
    bed.cue_dialog(serde_json::json!({
        "id": "call-create-1",
        "name": "write_to_file",
        "args": { "TargetFile": "/tmp/work/created.txt", "CodeContent": "hello" },
    }));
    let p = phone.await_permission(t, "write_to_file card");
    assert_eq!((p.request.as_str(), p.tool.as_str()), ("call-create-1", "write_to_file"));
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));
    c.await_state(id, "running after create", |s| *s == SessionState::Running);

    // 2. File edits (`replace_file_content`, `multi_replace_file_content`, `sed_file`),
    // including `Auto mode` standing between Yes and No.
    bed.cue_dialog(serde_json::json!({
        "id": "call-edit-1",
        "name": "replace_file_content",
        "auto_mode_choice": true,
        "args": { "TargetFile": "/tmp/work/lib.rs" },
    }));
    let p = phone.await_permission(t, "replace_file_content card");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));

    // Fire the next confirmation immediately inside the 1500 ms settle window:
    // `tool_confirmation_pending` cancels the settle and offers the new card.
    bed.cue_dialog(serde_json::json!({
        "id": "call-edit-2",
        "name": "multi_replace_file_content",
        "auto_mode_choice": true,
        "initial_cursor": 1,
        "args": { "TargetFile": "/tmp/work/mod.rs" },
    }));
    let p = phone.await_permission(t, "multi_replace_file_content card");
    assert_eq!(p.request, "call-edit-2");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Deny), ("answered".into(), None));
    c.await_state(id, "running after multi_replace deny", |s| *s == SessionState::Running);

    bed.cue_dialog(serde_json::json!({
        "id": "call-sed-1",
        "name": "sed_file",
        "args": { "File": "/tmp/work/sed.txt" },
    }));
    let p = phone.await_permission(t, "sed_file card");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));
    c.await_state(id, "running after sed", |s| *s == SessionState::Running);

    // 3. Command (`run_command`, `args.CommandLine`): Allow on row 1, and Deny
    // walking past Option 2 and Option 3 (`Persist to settings.json`) to row 4.
    bed.cue_dialog(serde_json::json!({
        "id": "call-cmd-allow",
        "name": "run_command",
        "args": { "CommandLine": "cargo test -p mesimon-core", "WaitMsBeforeAsync": 5000 },
    }));
    let p = phone.await_permission(t, "run_command allow card");
    assert_eq!(p.tool, "run_command");
    assert_eq!(p.input["CommandLine"], "cargo test -p mesimon-core");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));
    c.await_state(id, "running after cmd allow", |s| *s == SessionState::Running);

    bed.cue_dialog(serde_json::json!({
        "id": "call-cmd-deny",
        "name": "run_command",
        "initial_cursor": 2,
        "args": { "CommandLine": "printf denied", "BypassSandbox": false },
    }));
    let p = phone.await_permission(t, "run_command deny card");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Deny), ("answered".into(), None));
    c.await_state(id, "running after cmd deny", |s| *s == SessionState::Running);

    // 4. File access (`view_file`, cursor starting on `always allow`).
    bed.cue_dialog(serde_json::json!({
        "id": "call-view-1",
        "name": "view_file",
        "initial_cursor": 1,
        "args": { "AbsolutePath": "/tmp/outside/notes.md" },
    }));
    let p = phone.await_permission(t, "view_file card");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));
    c.await_state(id, "running after view_file", |s| *s == SessionState::Running);

    // 5. MCP tool (`call_mcp_tool`, cursor starting on `Persist to settings.json`).
    bed.cue_dialog(serde_json::json!({
        "id": "call-mcp-1",
        "name": "call_mcp_tool",
        "initial_cursor": 2,
        "args": { "ServerName": "vercel", "ToolName": "deploy_preview", "Arguments": {} },
    }));
    let p = phone.await_permission(t, "call_mcp_tool card");
    assert_eq!(decide(&mut phone, &p.request, PermissionDecision::Allow), ("answered".into(), None));
    c.await_state(id, "running after call_mcp_tool", |s| *s == SessionState::Running);

    // 6. Questions (`ask_question`): single choice, write-in, multi-select batch, and decline.
    bed.cue_dialog(serde_json::json!({
        "id": "call-q-choice",
        "name": "ask_question",
        "args": {
            "questions": [{
                "question": "Which color?",
                "options": ["Blue", "Green"],
                "is_multi_select": false,
            }]
        },
    }));
    let d = phone.await_dialog(t, "single question card");
    assert_eq!(d.request, "call-q-choice");
    assert_eq!(
        answer_q(&mut phone, &d.request, DialogAnswer::Choice { index: 1 }),
        ("answered".into(), None)
    );
    c.await_state(id, "running after single choice", |s| *s == SessionState::Running);

    bed.cue_dialog(serde_json::json!({
        "id": "call-q-text",
        "name": "ask_question",
        "args": {
            "questions": [{
                "question": "Which color?",
                "options": ["Blue", "Green"],
                "is_multi_select": false,
            }]
        },
    }));
    let d = phone.await_dialog(t, "write-in question card");
    assert_eq!(
        answer_q(&mut phone, &d.request, DialogAnswer::Text { text: "Violet".into() }),
        ("answered".into(), None)
    );
    c.await_state(id, "running after write-in", |s| *s == SessionState::Running);

    bed.cue_dialog(serde_json::json!({
        "id": "call-q-batch",
        "name": "ask_question",
        "args": {
            "questions": [
                { "question": "Pick toppings", "options": ["Cheese", "Olives"], "is_multi_select": true },
                { "question": "Crust?", "options": ["Thin", "Thick"], "is_multi_select": false },
            ]
        },
    }));
    let d = phone.await_dialog(t, "batch question card");
    assert_eq!(
        answer_q(
            &mut phone,
            &d.request,
            DialogAnswer::Answers {
                answers: vec![
                    QuestionAnswer::Choices { indices: vec![1] },
                    QuestionAnswer::Choice { index: 1 },
                ],
            },
        ),
        ("answered".into(), None)
    );
    c.await_state(id, "running after batch", |s| *s == SessionState::Running);

    bed.cue_dialog(serde_json::json!({
        "id": "call-q-skip",
        "name": "ask_question",
        "args": {
            "questions": [
                { "question": "First?", "options": ["A", "B"], "is_multi_select": false },
                { "question": "Second?", "options": ["C", "D"], "is_multi_select": false },
            ]
        },
    }));
    let d = phone.await_dialog(t, "decline batch card");
    assert_eq!(answer_q(&mut phone, &d.request, DialogAnswer::Reject), ("answered".into(), None));
    c.await_state(id, "running after skip", |s| *s == SessionState::Running);

    // Verify every decision landed on the exact intended row, Option 3 was
    // never touched, and `mesimon hook` printed zero bytes on `PreToolUse`.
    let decisions: Vec<(String, serde_json::Value)> = bed
        .decisions()
        .into_iter()
        .map(|v| {
            let key = v["id"].as_str().unwrap().to_string();
            let val = if v.get("choice").is_some() { v["choice"].clone() } else { v["answers"].clone() };
            (key, val)
        })
        .collect();
    assert_eq!(
        decisions,
        vec![
            ("call-create-1".into(), serde_json::json!("Yes, allow creation")),
            ("call-edit-1".into(), serde_json::json!("Yes, accept this change")),
            ("call-edit-2".into(), serde_json::json!("No, reject this change")),
            ("call-sed-1".into(), serde_json::json!("Yes, accept this change")),
            ("call-cmd-allow".into(), serde_json::json!("Yes, run command")),
            ("call-cmd-deny".into(), serde_json::json!("No, cancel")),
            ("call-view-1".into(), serde_json::json!("Yes, allow access")),
            ("call-mcp-1".into(), serde_json::json!("Yes, allow tool call")),
            ("call-q-choice".into(), serde_json::json!(["Green"])),
            ("call-q-text".into(), serde_json::json!(["Violet"])),
            ("call-q-batch".into(), serde_json::json!(["Olives", "Thick"])),
            ("call-q-skip".into(), serde_json::json!(["User Skipped", "User Skipped"])),
        ]
    );
    let agy_settings =
        std::fs::read_to_string(bed.h.dir.join("agy-home/settings.json")).unwrap();
    assert!(!agy_settings.contains("poisoned_by_option_3"), "{agy_settings}");
    assert!(
        bed.printed().iter().all(|(ev, _)| ev != "PreToolUse"),
        "PreToolUse must print nothing: {:?}",
        bed.printed()
    );
}

/// Replay agy 1.5.14: PreToolUse, working, working, confirmation pending.
/// A retry has its own call id even when the command is byte-for-byte identical.
#[test]
fn intervening_working_titles_preserve_the_command_and_refused_calls_never_type() {
    let Some((bed, http_port)) = Bed::boot_with_relay("agy-command-card") else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    agy_board(&mut c);
    let mut phone = TestPhone::pair(&mut c, http_port);
    let t = ticket(&mut c, "Review command retries");
    let Response::Spawned { id, .. } = start(&mut c, t, true) else { panic!("spawn agy") };
    bed.await_prompt(0, "turn", "Review command retries");

    let cue = |request: &str, command: &str, bypass: bool| bed.cue_dialog(serde_json::json!({
        "id": request, "name": "run_command",
        "args": {"CommandLine": command, "BypassSandbox": bypass}
    }));
    let answer = |request: &str, decision| api::Request::Permission {
        ticket: t.to_string(), session: id.to_string(), request: request.into(), decision
    };
    cue("ordinary", "printf hello", false);
    let card = phone.await_permission(t, "ordinary command after two working titles");
    assert_eq!(card.request, "ordinary");
    assert_eq!(phone.settle(answer("ordinary", PermissionDecision::Allow)), ("answered".into(), None));

    for (request, command, bypass, reason) in [
        ("retry", "printf hello", true, "outside agy's sandbox"),
        ("multiline", "printf 'one\ntwo\nthree\nfour\nfive\nsix'", false, "verify the whole command"),
    ] {
        cue(request, command, bypass);
        wait_until(Duration::from_secs(10), reason, || {
            let agent = phone.agent(t);
            agent.permission.is_none() && agent.permission_refusal.as_deref().is_some_and(|s| s.contains(reason))
        });
        for stale_or_current in ["ordinary", request] {
            for decision in [PermissionDecision::Allow, PermissionDecision::Deny] {
                assert!(matches!(phone.call(answer(stale_or_current, decision)).1, Reply::Rejected { .. }));
            }
        }
        let keys: Vec<serde_json::Value> = bed.jsonl("agy-dialog-keys.jsonl");
        assert!(keys.iter().all(|key| key["id"] == "ordinary"), "{keys:?}");
        assert_eq!(bed.decisions().len(), 1);
    }
}

/// T-1235: an autonomous crown answers an agy worker's `ask_question` stops
/// (single and multi-select/write-in batch) via `get_ticket` + `answer_agent`,
/// and is refused on a permission stop, which stays the person's alone.
#[test]
fn an_autonomous_crown_answers_agy_questions_and_never_approvals() {
    let Some(bed) = Bed::boot("agy-crown", true, true) else { return };
    let mut c = TestClient::connect(&bed.h.paths.orch_sock());
    assert!(matches!(c.request(Command::SetMcpTools { on: true }), Response::Ok));
    assert!(matches!(c.request(Command::SetCrownBudget { budget: 2 }), Response::Ok));
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));

    let crown_t = ticket(&mut c, "Coordinate the work");
    assert!(matches!(c.request(Command::CrownTicket { id: crown_t }), Response::Ok));
    // Spawn the sleep stub (`AgentProvider::Claude`) on the crown ticket first,
    // then switch the board's provider to `Agy` so only the worker runs `fake_agy.py`.
    let Response::Spawned { id: crown_session, .. } = c.request(Command::SpawnSession {
        ticket: crown_t,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) else {
        panic!("spawn crown")
    };
    agy_board(&mut c);
    let crown = Principal::Agent { session: crown_session };

    let worker_t = ticket(&mut c, "Build the parser");
    let worker_key = c.board().ticket(worker_t).unwrap().short_key.clone();
    let Response::AgentTicket { ticket: initial } = c.send(
        crown.clone(),
        Command::AgentReadTicket { key: worker_key.clone(), full: false, plan: false },
    ) else {
        panic!("read worker")
    };
    let Response::AgentStarted { session_started: true, .. } = c.send(
        crown.clone(),
        Command::AgentStartTicket {
            key: worker_key.clone(),
            seen: initial.seen,
            plan: false,
            tier: None,
            workspace: Some("shared_checkout".into()),
            expect_secs: None,
        },
    ) else {
        panic!("crown start worker")
    };
    let worker_id = c.board().live_agent(worker_t).unwrap().id;
    bed.await_prompt(0, "turn", "Build the parser");
    c.await_state(worker_id, "worker running", |s| *s == SessionState::Running);

    // 1. Single `ask_question` answered by the crown.
    bed.cue_dialog(serde_json::json!({
        "id": "crown-q-1",
        "name": "ask_question",
        "args": {
            "questions": [{
                "question": "Target edition?",
                "options": ["2021", "2024"],
                "is_multi_select": false,
            }]
        },
    }));
    c.await_state(worker_id, "worker asking crown", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Question }
    });
    let Response::AgentTicket { ticket: view } = c.send(
        crown.clone(),
        Command::AgentReadTicket { key: worker_key.clone(), full: false, plan: false },
    ) else {
        panic!("crown read question")
    };
    let needs = view.needs_you.expect("needs_you on question");
    assert_eq!(needs.reason, "question");
    assert_eq!(needs.answerable, Some(true));
    assert_eq!(needs.request.as_deref(), Some("crown-q-1"));
    assert_eq!(needs.questions.len(), 1);
    let Response::AgentAnswered { outcome, answer, .. } = c.send(
        crown.clone(),
        Command::AgentAnswerTicket {
            key: worker_key.clone(),
            seen: view.seen,
            request: "crown-q-1".into(),
            index: Some(1),
            text: None,
            answers: None,
        },
    ) else {
        panic!("crown answer single")
    };
    assert_eq!((outcome.as_str(), answer.as_str()), ("answered", "2024"));
    c.await_state(worker_id, "worker running after crown answer", |s| *s == SessionState::Running);

    // 2. Multi-select + write-in batch answered by the crown.
    bed.cue_dialog(serde_json::json!({
        "id": "crown-q-2",
        "name": "ask_question",
        "args": {
            "questions": [
                { "question": "Targets?", "options": ["macOS", "Linux"], "is_multi_select": true },
                { "question": "Flag?", "options": ["--fast", "--strict"], "is_multi_select": false },
            ]
        },
    }));
    c.await_state(worker_id, "worker asking batch", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Question }
    });
    let Response::AgentTicket { ticket: view } = c.send(
        crown.clone(),
        Command::AgentReadTicket { key: worker_key.clone(), full: false, plan: false },
    ) else {
        panic!("crown read batch")
    };
    let Response::AgentAnswered { outcome, .. } = c.send(
        crown.clone(),
        Command::AgentAnswerTicket {
            key: worker_key.clone(),
            seen: view.seen,
            request: "crown-q-2".into(),
            index: None,
            text: None,
            answers: Some(vec![
                QuestionAnswer::Choices { indices: vec![0, 1] },
                QuestionAnswer::Text { text: "--custom".into() },
            ]),
        },
    ) else {
        panic!("crown answer batch")
    };
    assert_eq!(outcome, "answered");
    c.await_state(worker_id, "worker running after batch", |s| *s == SessionState::Running);

    // A plan decision from the crown uses agy's receipt as well.
    bed.cue_with("plan", "Keep the parser public");
    c.await_state(worker_id, "worker plan", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Plan }
    });
    let view = c.agent_read(crown_session, &worker_key).unwrap();
    let request = view.needs_you.unwrap().request.unwrap();
    let reply = c.send(
        crown.clone(),
        Command::AgentAcceptPlan { key: worker_key.clone(), seen: view.seen, request },
    );
    assert!(
        matches!(reply, Response::AgentPlanAccepted { ref outcome, .. } if outcome == "accepted"),
        "{reply:?}"
    );
    assert!(bed.prompts().iter().any(|(_, p)| p == "[Approved] implementation_plan.md"));
    c.await_state(worker_id, "running after confirmed crown decision", |s| {
        *s == SessionState::Running
    });

    // 3. Permission stop (`run_command`) is never the crown's to answer.
    bed.cue_dialog(serde_json::json!({
        "id": "crown-perm-1",
        "name": "run_command",
        "args": { "CommandLine": "cargo publish" },
    }));
    c.await_state(worker_id, "worker asking permission", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Permission }
    });
    let Response::AgentTicket { ticket: view } = c.send(
        crown.clone(),
        Command::AgentReadTicket { key: worker_key.clone(), full: false, plan: false },
    ) else {
        panic!("crown read permission")
    };
    let needs = view.needs_you.expect("needs_you on permission");
    assert_eq!(needs.reason, "permission");
    assert!(needs.request.is_none(), "no crown request id on a permission stop");
    let refused = c.send(
        crown,
        Command::AgentAnswerTicket {
            key: worker_key,
            seen: view.seen,
            request: "crown-perm-1".into(),
            index: Some(0),
            text: None,
            answers: None,
        },
    );
    err_containing(refused, "permission");

    let decisions: Vec<(String, serde_json::Value)> = bed
        .decisions()
        .into_iter()
        .map(|v| (v["id"].as_str().unwrap().to_string(), v["answers"].clone()))
        .collect();
    assert_eq!(
        decisions,
        vec![
            ("crown-q-1".into(), serde_json::json!(["2024"])),
            ("crown-q-2".into(), serde_json::json!(["macOS, Linux", "--custom"])),
        ]
    );
    assert!(bed.printed().iter().all(|(ev, _)| ev != "PreToolUse"), "{:?}", bed.printed());
}
