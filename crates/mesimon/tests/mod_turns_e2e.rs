//! The turn roads on the mod (T-575, T-576): a prompt for a Claude session
//! whose mod is up goes down its bridge as a `submit`, never typed into the
//! pane, and a question's answer as an `answer`, never keyed into a dialog.
//!
//! Every daemon here launches Claude on the mod road (`MESIMON_CLAUDE_ROAD=mod`)
//! under both passes, so the stand-in engine (`fake_claude_mod.py`) runs
//! beside each stub: it takes a `submit` into its own pane the way the engine
//! hands a prompt to the model, records every frame it read in
//! `mod-<session>.ndjson`, and relays the mod's reports through the real hook
//! binary. Files next to it stand for other mods: `mod-silent` (one that
//! never comes up), `mod-speaks` (an older one) and `mod-drop-submit` (one
//! whose engine drops every submit).

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord, SessionState};
use mesimon_core::command::{AgentTicketView, Command, Response};
use mesimon_core::road::Road;
use mesimon_core::Principal;
use serde_json::json;

const MOD: (&str, &str) = ("MESIMON_CLAUDE_ROAD", "mod");

/// A line reader that paints no composer: on the hook set's road a launch
/// into it would wait out the composer and fail (T-570). `stty -icanon`,
/// because the brief is longer than a canonical tty keeps of a line.
const READER: &str = "#!/bin/sh\nstty -icanon 2>/dev/null\nwhile IFS= read -r line; do \
                      printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// A 10 KB brief, as the rig's acceptance asks: paragraphs, a list, quotes
/// and a code fence, ending on a marker.
fn brief() -> String {
    let mut text =
        String::from("## The brief\n\nmesimon-mod-575 the whole description, verbatim.\n");
    let mut n = 0;
    while text.len() < 10_000 {
        n += 1;
        text.push_str(&format!(
            "\n- item {n}: \"quoted\" words, a `backtick`, an apostrophe's turn, and $HOME as text\n"
        ));
    }
    text.push_str("\n```sh\necho 'a fence'\n```\n\nmesimon-mod-576 the last line");
    text
}

fn feed(h: &Harness) -> String {
    std::fs::read_to_string(h.paths.state_dir.join("activity.jsonl")).unwrap_or_default()
}

/// A feed line holding every needle: the feed is flushed on the tick.
fn wait_feed(h: &Harness, needles: &[&str]) {
    let what = format!("a feed line with {needles:?}");
    wait_until(Duration::from_secs(5), &what, || {
        feed(h).lines().any(|l| needles.iter().all(|n| l.contains(n)))
    });
}

fn got(h: &Harness) -> String {
    std::fs::read_to_string(h.dir.join("got.txt")).unwrap_or_default()
}

/// Every frame the session's stand-in mod read, of `kind`.
fn frames(h: &Harness, sid: uuid::Uuid, kind: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(h.dir.join(format!("mod-{sid}.ndjson")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|f| f["kind"] == kind)
        .collect()
}

fn record(c: &mut TestClient, sid: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == sid).expect("the record")
}

fn ticket_with_brief(c: &mut TestClient, title: &str, brief: &str) -> ulid::Ulid {
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: brief.into() }),
        Response::NoteWritten { .. }
    ));
    ticket
}

fn spawn(c: &mut TestClient, ticket: ulid::Ulid, submit_prompt: bool) -> uuid::Uuid {
    match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    }
}

fn session_start(hook_sock: &Path, sid: uuid::Uuid) {
    hook_send_with(
        hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-mod.jsonl","cwd":"/tmp"}"#,
    );
}

fn prompt(c: &mut TestClient, ticket: ulid::Ulid, text: &str, resend: bool) -> Response {
    c.request(Command::PromptSession {
        ticket,
        text: text.into(),
        queued: false,
        accept_plan: false,
        plan: false,
        tier: None,
        resend,
    })
}

/// The bridge's first poll: the stand-in engine is up beside the stub.
fn wait_bridge(h: &Harness, sid: uuid::Uuid) {
    let pid = h.dir.join(format!("mod-bridge-{sid}.pid"));
    wait_until(Duration::from_secs(10), "the mod's bridge", || pid.exists());
    std::thread::sleep(Duration::from_millis(300));
}

#[test]
fn a_mod_launch_takes_its_whole_prompt_with_no_composer_and_a_live_prompt_likewise() {
    let Some(h) = Harness::boot_bare("modlaunch", Some(READER), &[MOD]) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("modlaunch");
    let brief = brief();
    let ticket = ticket_with_brief(&mut c, "mod me: do the brief", &brief);
    let sid = spawn(&mut c, ticket, true);
    let rec = record(&mut c, sid);
    assert_eq!(rec.road, Road::Mod);
    assert!(rec.pending_submit, "the prompt is owed");
    wait_bridge(&h, sid);

    // Nothing reaches a pane being born, on this road too: the words wait
    // for the pane's SessionStart as well as the mod.
    assert!(got(&h).is_empty(), "nothing before SessionStart: {:?}", got(&h));
    assert!(frames(&h, sid, "submit").is_empty());
    let t0 = Instant::now();
    session_start(&hook_sock, sid);
    wait_until(Duration::from_secs(5), "the brief at the stub", || {
        got(&h).contains("mesimon-mod-576 the last line")
    });
    let took = t0.elapsed();
    // One submit, the title over the description, every byte: no title was
    // typed and no paste was made, so it is the person's words whole.
    let submits = frames(&h, sid, "submit");
    assert_eq!(submits.len(), 1, "{submits:?}");
    assert_eq!(submits[0]["text"], format!("mod me: do the brief\n\n{brief}"));
    // No composer was read: this stub paints none, and the hook set's road
    // would have waited it out (30 s) and failed.
    assert!(took < Duration::from_secs(4), "no composer wait: {took:?}");
    wait_feed(&h, &["\"cmd\":\"prompt_by_mod\""]);
    assert!(!feed(&h).contains("prompt_submit_not_ready"));
    let rec = record(&mut c, sid);
    assert!(rec.ticket_read, "the brief went in with the first prompt");
    assert!(rec.pending_submit, "owed until UserPromptSubmit");

    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the ack", || !record(&mut c, sid).pending_submit);
    wait_feed(&h, &["\"cmd\":\"prompt_submitted\""]);

    // A prompt into the live session goes down the mod too, never pasted.
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(
        prompt(&mut c, ticket, "mesimon-probe-575 one more turn", false),
        Response::Ok
    ));
    wait_until(Duration::from_secs(5), "the live prompt at the stub", || {
        got(&h).contains("mesimon-probe-575 one more turn")
    });
    let submits = frames(&h, sid, "submit");
    assert_eq!(submits.len(), 2);
    assert_eq!(submits[1]["text"], "mesimon-probe-575 one more turn");
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the second ack", || !record(&mut c, sid).pending_submit);
    let _ = c.request(Command::KillSession { id: sid });
}

#[test]
fn a_mod_that_never_comes_up_leaves_the_words_to_the_paste_road() {
    let env = [MOD, ("MESIMON_MOD_BRIDGE_WAIT_MS", "1500")];
    let Some(h) = Harness::boot_with_env("modsilent", Some(READER), &env) else { return };
    std::fs::write(h.dir.join("mod-silent"), "").unwrap();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("modsilent");
    let ticket = ticket_with_brief(&mut c, "silent me", "## Brief\n\nmesimon-silent-1 by paste");
    let sid = spawn(&mut c, ticket, true);
    assert_eq!(record(&mut c, sid).road, Road::Mod);
    std::thread::sleep(Duration::from_millis(700));
    session_start(&hook_sock, sid);
    // The bridge wait runs out, and the words go the way an older Claude Code
    // takes them: into the composer, the title first, since none was typed.
    wait_until(Duration::from_secs(8), "the paste road's delivery", || {
        got(&h).contains("mesimon-silent-1 by paste")
    });
    let lines: Vec<String> = got(&h).lines().map(str::to_string).collect();
    let title = lines.iter().position(|l| l.trim() == "silent me").expect("the title line");
    let body = lines.iter().position(|l| l.contains("mesimon-silent-1")).expect("the brief");
    assert!(title < body, "{lines:?}");
    wait_feed(&h, &["\"cmd\":\"prompt_by_paste\""]);
    assert!(frames(&h, sid, "submit").is_empty());
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the ack", || !record(&mut c, sid).pending_submit);

    // A live prompt with no mod to take it is pasted, as before.
    hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(prompt(&mut c, ticket, "mesimon-silent-2 pasted", false), Response::Ok));
    wait_until(Duration::from_secs(5), "the pasted prompt", || {
        got(&h).contains("mesimon-silent-2 pasted")
    });
    let _ = c.request(Command::KillSession { id: sid });
}

#[test]
fn an_older_mod_that_speaks_no_submit_is_pasted_to() {
    let Some(h) = Harness::boot_with_env("modold", Some(READER), &[MOD]) else { return };
    std::fs::write(h.dir.join("mod-speaks"), "ping").unwrap();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("modold");
    let ticket = ticket_with_brief(&mut c, "old mod", "## Brief\n\nmesimon-old-1 by paste");
    let sid = spawn(&mut c, ticket, true);
    wait_bridge(&h, sid);
    session_start(&hook_sock, sid);
    wait_until(Duration::from_secs(8), "the paste road's delivery", || {
        got(&h).contains("mesimon-old-1 by paste")
    });
    assert!(frames(&h, sid, "submit").is_empty(), "a mod that cannot read a submit gets none");
    assert!(!feed(&h).contains("\"cmd\":\"prompt_by_mod\""));
    // Its ping still crosses the road.
    match c.request(Command::ModPing { session: sid }) {
        Response::ModPonged { .. } => {}
        other => panic!("ping: {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: sid });
}

#[test]
fn a_submit_the_mod_refuses_is_kept_unsent_and_its_resend_goes_down_the_mod() {
    let Some(h) = Harness::boot_bare("moddrop", Some(READER), &[MOD]) else { return };
    std::fs::write(h.dir.join("mod-drop-submit"), "").unwrap();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("moddrop");
    let ticket = ticket_with_brief(&mut c, "drop me", "## Brief\n\nmesimon-drop-1 resent whole");
    let sid = spawn(&mut c, ticket, true);
    wait_bridge(&h, sid);
    session_start(&hook_sock, sid);
    wait_until(Duration::from_secs(5), "the brief marked unsent", || {
        record(&mut c, sid).unsent.is_some()
    });
    let rec = record(&mut c, sid);
    assert!(rec.unsent.as_ref().unwrap().brief, "the brief, for the card's `brief not sent`");
    assert!(!rec.pending_submit);
    wait_feed(&h, &["\"cmd\":\"prompt_submit_refused\"", "\"outcome\":\"dropped\""]);
    assert!(got(&h).is_empty(), "nothing reached the stub");

    // The person's resend goes down the mod again, title and brief, whole.
    std::fs::remove_file(h.dir.join("mod-drop-submit")).unwrap();
    assert!(matches!(prompt(&mut c, ticket, "", true), Response::Ok));
    wait_until(Duration::from_secs(5), "the resent brief", || {
        got(&h).contains("mesimon-drop-1 resent whole")
    });
    let submits = frames(&h, sid, "submit");
    assert_eq!(submits.len(), 2);
    assert_eq!(submits[1]["text"], "drop me\n\n## Brief\n\nmesimon-drop-1 resent whole");
    wait_feed(&h, &["\"cmd\":\"prompt_resent\""]);
    hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    wait_until(Duration::from_secs(3), "the ack clears it", || {
        record(&mut c, sid).unsent.is_none()
    });
    let _ = c.request(Command::KillSession { id: sid });
}

fn read(c: &mut TestClient, session: uuid::Uuid, key: &str) -> AgentTicketView {
    match c.send(Principal::Agent { session }, Command::AgentReadTicket { key: key.into() }) {
        Response::AgentTicket { ticket } => ticket,
        other => panic!("read {key}: {other:?}"),
    }
}

/// The crown answers its worker's questions down the worker's mod (T-576):
/// one frame, by the labels, no key into the pane and no screen read; the
/// mod's report is the receipt and moves the card, since no `PostToolUse`
/// fires for a dialog the mod closed. A person who answers in the pane first
/// wins; a refusal the mod saw is the dialog's end.
#[test]
fn the_crown_answers_down_the_worker_s_mod_and_the_person_first_wins() {
    let env = [MOD, ("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")];
    let Some(h) = Harness::boot_with_env("modanswer", Some(READER), &env) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("modanswer");
    let make = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let a = make(&mut c, "coordinate");
    let w = make(&mut c, "mesimon-probe-576 worker");
    let kw = c.board().ticket(w).unwrap().short_key.clone();
    let sa = spawn(&mut c, a, false);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(Duration::from_millis(500));
    let v = read(&mut c, sa, &kw);
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentStartTicket {
                key: kw.clone(),
                seen: v.seen,
                plan: false,
                tier: None,
                workspace: Some("shared_checkout".into()),
            },
        ),
        Response::AgentStarted { .. }
    ));
    let ws = c.board().live_agent(w).expect("W holds a seat").id;
    wait_bridge(&h, ws);
    hook_send(&hook_sock, &ws.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    c.await_state(ws, "running", |s| *s == SessionState::Running);
    let asking = |s: &SessionState| {
        *s == SessionState::RequiresAction { reason: mesimon_core::board::Reason::Question }
    };
    let question = |request: &str| {
        json!({
            "tool_name": "AskUserQuestion",
            "tool_use_id": request,
            "tool_input": { "questions": [{
                "question": "Which colour?", "header": "Colour", "multiSelect": false,
                "options": [{ "label": "red", "description": "" },
                            { "label": "blue", "description": "" }]
            }]}
        })
        .to_string()
    };
    let answer = |c: &mut TestClient, request: &str| {
        let v = read(c, sa, &kw);
        let cmd = Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: v.seen,
            request: request.into(),
            index: Some(1),
            text: None,
            answers: None,
        };
        let sock = h.paths.orch_sock();
        std::thread::spawn(move || {
            TestClient::connect(&sock).send(Principal::Agent { session: sa }, cmd)
        })
    };
    let pane_keys =
        |c: &mut TestClient| match c.request(Command::PaneTail { session: ws, lines: 5 }) {
            Response::PaneTail { lines, .. } => lines.join("\n"),
            _ => String::new(),
        };

    // ---- the crown's answer, down the mod: the receipt is the mod's report --
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_m1"));
    c.await_state(ws, "asking", asking);
    let screen = pane_keys(&mut c);
    let call = answer(&mut c, "toolu_m1");
    match call.join().unwrap() {
        Response::AgentAnswered { outcome, answer, reason, .. } => {
            assert_eq!((outcome.as_str(), answer.as_str(), reason), ("answered", "blue", None));
        }
        other => panic!("answer_agent: {other:?}"),
    }
    let sent = frames(&h, ws, "answer");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["tool_use_id"], "toolu_m1");
    assert_eq!(sent[0]["answers"], json!({ "Which colour?": "blue" }));
    assert_eq!(pane_keys(&mut c), screen, "no key reached the pane");
    // No PostToolUse was sent: the mod's report moved the card.
    c.await_state(ws, "running on the answer", |s| *s == SessionState::Running);
    assert!(read(&mut c, sa, &kw).needs_you.is_none());

    // ---- the person answers in the pane first: theirs wins -----------------
    std::fs::write(h.dir.join(format!("mod-person-answered-{ws}")), "").unwrap();
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_m2"));
    c.await_state(ws, "asking again", asking);
    let call = answer(&mut c, "toolu_m2");
    wait_until(Duration::from_secs(5), "the answer at W's mod", || {
        frames(&h, ws, "answer").len() == 2
    });
    // The native dialog took the person's answer: its own PostToolUse.
    hook_send(&hook_sock, &ws.to_string(), "PostToolUse", &question("toolu_m2"));
    match call.join().unwrap() {
        Response::AgentAnswered { outcome, reason, .. } => {
            assert_eq!(outcome, "unknown");
            assert_eq!(reason.as_deref(), Some("a_person_answered"));
        }
        other => panic!("answer_agent: {other:?}"),
    }
    std::fs::remove_file(h.dir.join(format!("mod-person-answered-{ws}"))).unwrap();

    // ---- a refusal the mod saw ends the dialog: nothing left to answer -----
    hook_send(&hook_sock, &ws.to_string(), "PreToolUse", &question("toolu_m3"));
    c.await_state(ws, "asking a third time", asking);
    assert_eq!(
        read(&mut c, sa, &kw).needs_you.and_then(|n| n.request).as_deref(),
        Some("toolu_m3")
    );
    hook_send_road(
        &hook_sock,
        &ws.to_string(),
        "ModAnswer",
        Some("declined"),
        None,
        r#"{"hook_event_name":"PostToolUse","tool_name":"AskUserQuestion","tool_use_id":"toolu_m3"}"#,
        Some("mod"),
    );
    wait_until(Duration::from_secs(5), "the dismissal", || {
        read(&mut c, sa, &kw).needs_you.and_then(|n| n.request).is_none()
    });
    let v = read(&mut c, sa, &kw);
    let refused = c.send(
        Principal::Agent { session: sa },
        Command::AgentAnswerTicket {
            key: kw.clone(),
            seen: v.seen,
            request: "toolu_m3".into(),
            index: Some(0),
            text: None,
            answers: None,
        },
    );
    assert!(matches!(refused, Response::Err { .. }), "{refused:?}");
    assert_eq!(frames(&h, ws, "answer").len(), 2, "nothing more went down");
}
