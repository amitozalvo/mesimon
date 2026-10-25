//! A launch's words go only into a painted composer (T-570).
//!
//! `SessionStart` fires during Claude Code's startup, before it puts the tty
//! in raw mode or asks for bracketed paste. A brief pasted on that edge went
//! in as plain bytes: echoed by the cooked tty, cut at its 1 KiB line, and
//! typed into the box where no Enter submitted it (T-566, 2026-10-02). Each
//! stub here stands in for one shape of that startup, and every byte it reads
//! is kept in `got.bin`.
//!
//! tmux brackets a paste only for an application that asked
//! (`\e[?2004h`), and each stub asks exactly when it paints its composer —
//! so a brief inside `\e[200~ … \e[201~` went in after the composer, and one
//! outside them beat it.
//!
//! These are the paste road's own mechanics, so every daemon here runs on
//! the hook set (`HOOKS`) under both passes: on the mod road a Claude
//! launch's words go down its mod as a `submit` and no composer is read
//! (T-575), which `mod_turns_e2e` holds.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::path::Path;
use std::time::{Duration, Instant};

use mesimon_core::board::{SessionKind, SessionRecord, SessionState, Unsent};
use mesimon_core::command::{Command, Response};
use mesimon_core::Principal;

const BRIEF: &str = "## Brief\n\nmesimon-ready-11 the description waits for the composer\n\n\
                     - and arrives whole, mesimon-ready-12";

/// The road these tests are about, whichever pass runs them.
const HOOKS: (&str, &str) = ("MESIMON_CLAUDE_ROAD", "hooks");

const OPEN: &str = "\x1b[200~";
const CLOSE: &str = "\x1b[201~";

/// Cooked and echoing for three seconds — the window `SessionStart` lands
/// in — and then what Claude Code does, in its order: raw input, bracketed
/// paste, the composer.
const SLOW: &str = "#!/bin/sh
d=\"$(dirname \"$0\")\"
sleep 3
stty -icanon -echo 2>/dev/null
printf '\\033[?2004h\\033[999;1H\\033[3A────────────────────\\n❯ \\n────────────────────\\n  ? for shortcuts'
: > \"$d/painted\"
exec cat >> \"$d/got.bin\"
";

/// Reads, and never paints a composer.
const NEVER: &str = "#!/bin/sh
stty -icanon -echo 2>/dev/null
exec cat >> \"$(dirname \"$0\")/got.bin\"
";

/// A composer that holds what a failed start left in it — the T-566 pane,
/// `❯ crown). The crown guessed…` — with the cursor after the text, and a
/// Ctrl+C that arrives as a byte (`-isig`) rather than a signal. It never
/// repaints, so the box reads as holding text for the life of the pane.
const STRAY: &str = "#!/bin/sh
stty -icanon -echo -isig 2>/dev/null
printf '\\033[?2004h\\033[999;1H\\033[3A────────────────────\\n❯ crown). The crown guessed\\n────────────────────\\n  ? for shortcuts\\033[2A\\033[28G'
exec cat >> \"$(dirname \"$0\")/got.bin\"
";

/// An empty composer that takes the first paste and then reads as holding
/// text for the life of the pane (the T-566 box, after the paste): no
/// repaint and no ack, so the Enters after the paste run out. (`<&0`: a
/// non-interactive shell's background job reads `/dev/null` otherwise.)
const LATE_STRAY: &str = "#!/bin/sh
d=\"$(dirname \"$0\")\"
stty -icanon -echo -isig 2>/dev/null
printf '\\033[?2004h\\033[999;1H\\033[3A────────────────────\\n❯ \\n────────────────────\\n  ? for shortcuts'
cat <&0 >> \"$d/got.bin\" &
until grep -q '200~' \"$d/got.bin\" 2>/dev/null; do sleep 0.05; done
printf '\\033[999;1H\\033[3A────────────────────\\n❯ crown). The crown guessed\\n────────────────────\\n  ? for shortcuts\\033[2A\\033[28G'
wait
";

/// A pane that paints nothing until cued (T-987 round 2), so launch words
/// wait for its box across a restart: `title` beside it paints the typed
/// title `recover me`, `draft` the title with a person's text after it,
/// `empty` an empty box. Every byte read goes to `got.bin`.
const TITLED: &str = "#!/bin/sh
d=\"$(dirname \"$0\")\"
stty -icanon -echo -isig 2>/dev/null
box() {
  printf '\\033[?2004h\\033[999;1H\\033[3A────────────────────\\n\\033[2K❯ %s\\n────────────────────\\n  ? for shortcuts\\033[2A\\033[%sG' \"$1\" \"$2\"
}
cat <&0 >> \"$d/got.bin\" &
while :; do
  [ -e \"$d/title\" ] && rm \"$d/title\" && box 'recover me' 13
  [ -e \"$d/draft\" ] && rm \"$d/draft\" && box 'recover me and a draft' 25
  [ -e \"$d/empty\" ] && rm \"$d/empty\" && box '' 3
  sleep 0.05
done
";

fn got(h: &Harness) -> String {
    String::from_utf8_lossy(&std::fs::read(h.dir.join("got.bin")).unwrap_or_default()).into_owned()
}

/// The bracketed pastes in what the stub read, in order, with tmux's CR for
/// a newline read back as the newline the paste carried.
fn pastes(text: &str) -> Vec<String> {
    text.split(OPEN)
        .skip(1)
        .filter_map(|rest| rest.split_once(CLOSE).map(|(body, _)| body.replace('\r', "\n")))
        .collect()
}

fn record(c: &mut TestClient, sid: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == sid).expect("the record")
}

/// A ticket with a description, and its claude started the composed way.
fn composed(c: &mut TestClient, h: &Harness, title: &str) -> (ulid::Ulid, uuid::Uuid) {
    let _ = c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
        tier: None,
    });
    let ticket = c.board().tickets.into_iter().find(|t| t.title == title).expect("ticket").id;
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: BRIEF.into(), rev: None }),
        Response::NoteWritten { .. }
    ));
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let tmux_sock = h.paths.tmux_sock();
    wait_until(Duration::from_secs(15), "the stub's pane", || {
        tmux(&tmux_sock)
            .args(["list-panes", "-a", "-F", "#{pane_pid}"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    });
    (ticket, sid)
}

fn session_start(hook_sock: &Path, sid: uuid::Uuid) {
    hook_send_with(
        hook_sock,
        &sid.to_string(),
        "SessionStart",
        Some("startup"),
        r#"{"session_id":"x","transcript_path":"/tmp/t-ready.jsonl","cwd":"/tmp"}"#,
    );
}

/// The edge lands while the stub is still cooked: nothing is typed then,
/// the brief goes in whole once the composer paints, inside the brackets
/// only a composer that asked for them gets, and the Enters follow it until
/// the ack.
#[test]
fn the_brief_waits_for_the_composer_and_arrives_whole() {
    let Some(h) = Harness::boot_bare("ready", Some(SLOW), &[HOOKS]) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("ready");
    let (ticket, sid) = composed(&mut c, &h, "ready me");
    session_start(&hook_sock, sid);

    // A cadence past the edge is when the brief used to go. Nothing may.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!h.dir.join("painted").exists(), "the stub is still starting");
    assert!(!got(&h).contains("mesimon-ready"), "nothing is read before raw mode");

    let deadline = Instant::now() + Duration::from_secs(15);
    let text = loop {
        let text = got(&h);
        if text.contains(CLOSE) && text.rsplit(CLOSE).next().is_some_and(|t| t.contains('\n')) {
            break text;
        }
        assert!(Instant::now() < deadline, "the brief never arrived after the composer: {text:?}");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(pastes(&text), vec![format!("\n\n{BRIEF}")], "one paste, whole: {text:?}");
    assert_eq!(text.matches("mesimon-ready-12").count(), 1, "nothing typed outside it: {text:?}");
    assert!(record(&mut c, sid).ticket_read, "the brief went in");

    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack", || !record(&mut c, sid).pending_submit);
    // The feed reaches disk on the next tick.
    wait_until(Duration::from_secs(3), "the ack's feed line", || {
        feed_count(&h, "prompt_submitted", ticket) == 1
    });
    assert_eq!(feed_count(&h, "prompt_submit_not_ready", ticket), 0);
    assert!(record(&mut c, sid).unsent.is_none());
    let _ = c.request(Command::KillSession { id: sid });
}

/// A pane that never paints a composer is a failed start, said out loud:
/// the feed line, the record's unsent mark the card and the `!N` chip read,
/// and `unsent` where the crown would otherwise read `idle`.
#[test]
fn a_composer_that_never_paints_is_a_failed_start() {
    let env = [("MESIMON_COMPOSER_WAIT_MS", "2000"), HOOKS];
    let Some(h) = Harness::boot_bare("notready", Some(NEVER), &env) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("notready");
    let (ticket, sid) = composed(&mut c, &h, "never me");
    session_start(&hook_sock, sid);

    wait_until(Duration::from_secs(10), "prompt_submit_not_ready", || {
        feed_count(&h, "prompt_submit_not_ready", ticket) == 1
    });
    let rec = record(&mut c, sid);
    assert_eq!(rec.unsent, Some(Unsent { text: String::new(), brief: true }));
    assert!(!rec.pending_submit, "nothing is owed any more");
    assert!(!rec.ticket_read, "the brief never went in");
    assert_eq!(
        c.board().needs_you_count(mesimon_core::clock::now_ms()),
        1,
        "the card and the chip say so"
    );
    assert!(!got(&h).contains("mesimon-ready"), "nothing was pasted: {:?}", got(&h));
    match c.send(Principal::Agent { session: sid }, Command::AgentGetTicket) {
        Response::AgentTicket { ticket: view } => {
            assert_eq!(view.state.map(|s| s.state).as_deref(), Some("unsent"));
        }
        other => panic!("get_ticket failed: {other:?}"),
    }
    let _ = c.request(Command::KillSession { id: sid });
}

/// The T-566 pane from the start: a box holding text other than the typed
/// title is no composer for the brief (T-987: the reader cannot tell it from
/// a person's draft, and words pasted under it would join it), so the brief
/// waits out the composer's wait with no key, and the seat keeps what it
/// never took. Its Shift+Enter (`resend`) clears the box with exactly one
/// Ctrl+C — a second exits Claude, so a box that still reads as holding is
/// pasted into as it is — and pastes the title and the brief whole; the
/// agent's ack clears the mark.
#[test]
fn a_box_holding_other_text_holds_the_brief_until_the_wait_runs_out() {
    let env = [("MESIMON_COMPOSER_WAIT_MS", "3000"), HOOKS];
    let Some(h) = Harness::boot_bare("resend", Some(STRAY), &env) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("resend");
    let (ticket, sid) = composed(&mut c, &h, "resend me");
    std::thread::sleep(Duration::from_millis(500));
    session_start(&hook_sock, sid);

    wait_until(Duration::from_secs(15), "prompt_submit_not_ready", || {
        feed_count(&h, "prompt_submit_not_ready", ticket) == 1
    });
    assert_eq!(feed_count(&h, "composer_held", ticket), 1, "the hold is said once");
    assert_eq!(record(&mut c, sid).unsent, Some(Unsent { text: String::new(), brief: true }));
    assert_eq!(c.board().needs_you_count(mesimon_core::clock::now_ms()), 1);
    assert!(!got(&h).contains('\x03'), "the wait clears nothing");
    assert!(pastes(&got(&h)).is_empty(), "nothing went under the text: {:?}", got(&h));

    let resend = |c: &mut TestClient| {
        c.request(Command::PromptSession {
            ticket,
            text: String::new(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: true,
            uploads: Vec::new(),
        })
    };
    assert!(matches!(resend(&mut c), Response::Ok));
    assert!(record(&mut c, sid).pending_submit, "the Enter is owed again");
    err_containing(resend(&mut c), "already waiting");

    wait_until(Duration::from_secs(10), "the resend's paste", || pastes(&got(&h)).len() == 1);
    let text = got(&h);
    assert_eq!(text.matches('\x03').count(), 1, "exactly one Ctrl+C: {text:?}");
    let ctrl_c = text.find('\x03').unwrap();
    let first = text.find(OPEN).unwrap();
    assert!(ctrl_c < first, "the box is cleared before the paste: {text:?}");
    assert_eq!(pastes(&text)[0], format!("resend me\n\n{BRIEF}"), "title and brief: {text:?}");
    assert!(record(&mut c, sid).unsent.is_some(), "the mark stays until the agent takes the words");

    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack clears the mark", || {
        let rec = record(&mut c, sid);
        rec.unsent.is_none() && !rec.pending_submit
    });
    assert_eq!(c.board().needs_you_count(mesimon_core::clock::now_ms()), 0);
    wait_until(Duration::from_secs(3), "the resend's feed line", || {
        feed_count(&h, "prompt_resent", ticket) == 1
    });
    err_containing(resend(&mut c), "nothing unsent");
    let _ = c.request(Command::KillSession { id: sid });
}

/// The T-566 pane after the paste (T-570): the brief went into an empty
/// composer, no ack came, and the Enters run out on a box that now holds
/// text, so the seat keeps what it never took. Its Shift+Enter (`resend`)
/// clears the box with exactly one Ctrl+C and pastes the title and the brief
/// whole into what is left; the agent's ack clears the mark.
#[test]
fn a_give_up_is_marked_and_the_resend_clears_the_box_once() {
    let Some(h) = Harness::boot_bare("resend-late", Some(LATE_STRAY), &[HOOKS]) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("resend-late");
    let (ticket, sid) = composed(&mut c, &h, "resend me");
    std::thread::sleep(Duration::from_millis(500));
    session_start(&hook_sock, sid);

    wait_until(Duration::from_secs(15), "prompt_submit_gave_up", || {
        feed_count(&h, "prompt_submit_gave_up", ticket) == 1
    });
    assert_eq!(record(&mut c, sid).unsent, Some(Unsent { text: String::new(), brief: true }));
    assert_eq!(c.board().needs_you_count(mesimon_core::clock::now_ms()), 1);
    assert!(!got(&h).contains('\x03'), "the first paste clears nothing");
    assert_eq!(pastes(&got(&h)).len(), 1, "the first paste went in: {:?}", got(&h));
    assert_eq!(feed_count(&h, "composer_held", ticket), 0, "an empty box held nothing");

    let resend = |c: &mut TestClient| {
        c.request(Command::PromptSession {
            ticket,
            text: String::new(),
            queued: false,
            immediately: false,
            accept_plan: false,
            plan: false,
            tier: None,
            resend: true,
            uploads: Vec::new(),
        })
    };
    assert!(matches!(resend(&mut c), Response::Ok));
    wait_until(Duration::from_secs(10), "the resend's paste", || pastes(&got(&h)).len() == 2);
    let text = got(&h);
    assert_eq!(text.matches('\x03').count(), 1, "exactly one Ctrl+C: {text:?}");
    let ctrl_c = text.find('\x03').unwrap();
    let second = text.match_indices(OPEN).nth(1).map(|(at, _)| at).unwrap();
    assert!(ctrl_c < second, "the box is cleared before the paste: {text:?}");
    assert_eq!(pastes(&text)[1], format!("resend me\n\n{BRIEF}"), "title and brief: {text:?}");
    hook_send_with(&hook_sock, &sid.to_string(), "UserPromptSubmit", None, r#"{}"#);
    wait_until(Duration::from_secs(3), "the ack clears the mark", || {
        let rec = record(&mut c, sid);
        rec.unsent.is_none() && !rec.pending_submit
    });
    let _ = c.request(Command::KillSession { id: sid });
}

/// A brief held for text in its box when the daemon restarts (T-987 round
/// 1): the restart's resend (T-722) is a recovery, not the person's resend,
/// so its one Ctrl+C clears only the title the start typed. The box's other
/// text holds the brief again, and the wait leaves it unsent: no Ctrl+C and
/// no paste ever reach the pane.
#[test]
fn a_restart_s_resend_never_clears_the_text_that_held_its_brief() {
    let env = [("MESIMON_COMPOSER_WAIT_MS", "6000"), HOOKS];
    let Some(h) = Harness::boot_bare("resend-restart", Some(STRAY), &env) else { return };
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("resend-restart");
    let (ticket, sid) = composed(&mut c, &h, "restart me");
    std::thread::sleep(Duration::from_millis(500));
    let transcript = h.dir.join(format!("{sid}.jsonl"));
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), &start);
    wait_until(Duration::from_secs(5), "the brief held", || {
        feed_count(&h, "composer_held", ticket) == 1
    });
    drop(c);
    h.restart();
    let mut c = h.client("resend-restart-2");
    wait_until(Duration::from_secs(10), "the restart's resend", || {
        feed_count(&h, "prompt_resent", ticket) == 1
    });
    wait_until(Duration::from_secs(20), "the resend's wait to run out", || {
        feed_count(&h, "prompt_submit_not_ready", ticket) == 1
    });
    assert_eq!(feed_count(&h, "composer_held", ticket), 2, "held again after the restart");
    assert_eq!(record(&mut c, sid).unsent, Some(Unsent { text: String::new(), brief: true }));
    let text = got(&h);
    assert!(!text.contains('\x03'), "no Ctrl+C into the box: {text:?}");
    assert!(pastes(&text).is_empty(), "no paste under its text: {text:?}");
    let _ = c.request(Command::KillSession { id: sid });
}

/// T-987 round 2: a recovery's one Ctrl+C is decided at its own edge, by a
/// fresh read of the box beside the clients. The brief waits for a box that
/// has not painted when the daemon restarts; the restart's resend (T-722)
/// then samples the box holding the typed title, and at a barrier between
/// that sample and the clear the box changes: to the title with a person's
/// text after it (no Ctrl+C, no paste, the wait leaves the brief unsent),
/// or to an empty box (no Ctrl+C, the brief pasted).
fn recovery_at_the_key(name: &str, cue: &str) -> Option<(Harness, ulid::Ulid, uuid::Uuid)> {
    let dir = std::env::temp_dir().join(format!("msmn-t987-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let barrier = dir.join("barrier");
    let reached = dir.join("barrier.reached");
    let env = [
        ("MESIMON_COMPOSER_WAIT_MS", "4000"),
        ("MESIMON_TEST_LAUNCH_BARRIER", barrier.to_str().unwrap()),
        HOOKS,
    ];
    let h = Harness::boot_bare(name, Some(TITLED), &env)?;
    let hook_sock = h.paths.hook_sock();
    let tmux_sock = h.paths.tmux_sock();
    let mut c = h.client(name);
    let (ticket, sid) = composed(&mut c, &h, "recover me");
    let sid16: String = sid.simple().to_string().chars().take(16).collect();
    let pane = || -> String {
        tmux(&tmux_sock)
            .args(["capture-pane", "-p", "-t", &sid16])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let transcript = h.dir.join(format!("{sid}.jsonl"));
    let start = format!(
        r#"{{"session_id":"x","transcript_path":"{}","cwd":"/tmp"}}"#,
        transcript.display()
    );
    hook_send_with(&hook_sock, &sid.to_string(), "SessionStart", Some("startup"), &start);
    c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(record(&mut c, sid).pending_submit, "the brief waits for a box");
    drop(c);
    h.restart_after(|| std::fs::write(&barrier, "").unwrap());
    let c = h.client(&format!("{name}-2"));
    wait_until(Duration::from_secs(10), "the restart's resend", || {
        feed_count(&h, "prompt_resent", ticket) == 1
    });
    std::fs::write(h.dir.join("title"), "").unwrap();
    wait_until(Duration::from_secs(15), "the clear's edge", || reached.exists());
    assert!(pane().contains("❯ recover me"), "the sample read the title: {}", pane());
    std::fs::write(h.dir.join(cue), "").unwrap();
    wait_until(Duration::from_secs(5), "the cue painted", || !h.dir.join(cue).exists());
    std::thread::sleep(Duration::from_millis(200));
    std::fs::remove_file(&barrier).unwrap();
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
    Some((h, ticket, sid))
}

#[test]
fn a_recovery_never_clears_text_after_the_title() {
    let Some((h, ticket, sid)) = recovery_at_the_key("recover-draft", "draft") else { return };
    let mut c = h.client("recover-draft-3");
    wait_until(Duration::from_secs(15), "the wait to run out", || {
        feed_count(&h, "prompt_submit_not_ready", ticket) == 1
    });
    let text = got(&h);
    assert!(!text.contains('\x03'), "no Ctrl+C into the draft: {text:?}");
    assert!(pastes(&text).is_empty(), "no paste under it: {text:?}");
    assert_eq!(record(&mut c, sid).unsent, Some(Unsent { text: String::new(), brief: true }));
    let _ = c.request(Command::KillSession { id: sid });
}

#[test]
fn a_recovery_pastes_into_an_empty_box_with_no_clear() {
    let Some((h, _, sid)) = recovery_at_the_key("recover-empty", "empty") else { return };
    let mut c = h.client("recover-empty-3");
    wait_until(Duration::from_secs(15), "the brief's paste", || pastes(&got(&h)).len() == 1);
    let text = got(&h);
    assert!(!text.contains('\x03'), "no Ctrl+C into an empty box: {text:?}");
    assert_eq!(pastes(&text)[0], format!("recover me\n\n{BRIEF}"), "title and brief: {text:?}");
    let _ = c.request(Command::KillSession { id: sid });
}
