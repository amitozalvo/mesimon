//! The board accepts Claude Code's folder trust dialog (T-844), end to end.
//!
//! The dialog comes before `SessionStart`, so no hook can say it is there:
//! a stub paints it as Claude Code 2.1.302 does (the cursor on `No, exit`,
//! `Yes, I trust this folder` under it), the daemon reads it off the pane
//! and the card reads TRUST with the dialog's words. The launch's brief
//! waits on it instead of going unsent. The board's `w`
//! (`Command::AcceptTrust`, the walk a phone's trust card asks for) presses
//! the dialog's own keys, Down and then Enter, the screen read before each;
//! the stub then paints its composer and the brief goes in by itself.
//!
//! The paste road's mechanics, so the daemon runs on the hook set under
//! both passes (`HOOKS`), as `spawn_ready_e2e` does.
//!
//! A Codex seat's dialog takes the same `w` (T-871): the synthetic runtime
//! (`fake_codex_runtime.py`) paints Codex 0.163's `Trust this folder?`, the
//! cursor on `1. Trust and continue`, and Enter on it paints the composer.

// Integration-test crate: `allow-unwrap-in-tests` only reaches items marked
// #[test], not the helpers beside them, so the D26 exemption is stated here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use std::time::Duration;

use mesimon_core::board::{AgentProvider, Reason, SessionKind, SessionRecord, SessionState};
use mesimon_core::command::{Command, Response};

const HOOKS: (&str, &str) = ("MESIMON_CLAUDE_ROAD", "hooks");

const BRIEF: &str = "## Brief\n\nmesimon-trust-11 the description waits for the dialog\n";

/// Claude Code 2.1.302's folder trust dialog, as a pane shows it, then the
/// keys it takes: Down and Up move the cursor between the two rows and
/// repaint; Enter on `Yes, I trust this folder` clears the screen, asks for
/// bracketed paste and paints the composer, as Claude Code does once it
/// starts; from there every byte read is kept in `got.bin`. Every key read
/// before that is appended to `keys.txt`, and `trusted` marks the accept.
const STUB: &str = r##"#!/usr/bin/env python3
import os, sys, tty
d = os.path.dirname(os.path.abspath(sys.argv[0]))
fd = sys.stdin.fileno()
tty.setcbreak(fd)
sel = "no"
def paint():
    rows = [
        "\x1b[2J\x1b[H Accessing workspace: " + os.getcwd(), "",
        " Quick safety check: Is this a project you created or one you trust? (Like",
        " your own code, a well-known open source project, or work from your team).",
        " If not, take a moment to review what's in this folder first.", "",
        " Claude Code'll be able to read, edit, and execute files here.", "",
    ]
    rows += [" ❯ No, exit", "   Yes, I trust this folder"] if sel == "no" else ["   No, exit", " ❯ Yes, I trust this folder"]
    rows += ["", " Enter to confirm · Esc to cancel"]
    sys.stdout.write("\r\n".join(rows))
    sys.stdout.flush()
paint()
log = open(os.path.join(d, "keys.txt"), "a")
buf = b""
while True:
    b = os.read(fd, 1)
    if not b:
        sys.exit(0)
    buf += b
    if buf in (b"\x1b", b"\x1b["):
        continue
    key = {b"\x1b[B": "Down", b"\x1b[A": "Up", b"\r": "Enter", b"\n": "Enter"}.get(buf, repr(buf))
    buf = b""
    log.write(key + "\n")
    log.flush()
    if key == "Down":
        sel = "yes"
        paint()
    elif key == "Up":
        sel = "no"
        paint()
    elif key == "Enter" and sel == "yes":
        break
sys.stdout.write("\x1b[?2004h\x1b[2J\x1b[H\x1b[999;1H\x1b[3A────────────────────\r\n❯ \r\n────────────────────\r\n  ? for shortcuts\x1b[H")
sys.stdout.flush()
open(os.path.join(d, "trusted"), "w").close()
got = open(os.path.join(d, "got.bin"), "ab")
while True:
    b = os.read(fd, 4096)
    if not b:
        break
    got.write(b)
    got.flush()
"##;

fn keys(h: &Harness) -> String {
    std::fs::read_to_string(h.dir.join("keys.txt")).unwrap_or_default()
}

fn got(h: &Harness) -> String {
    String::from_utf8_lossy(&std::fs::read(h.dir.join("got.bin")).unwrap_or_default()).into_owned()
}

fn record(c: &mut TestClient, sid: uuid::Uuid) -> SessionRecord {
    c.board().sessions.into_iter().find(|s| s.id == sid).expect("the record")
}

#[test]
fn the_board_trusts_the_workspace_with_the_dialog_s_own_keys_and_the_brief_follows() {
    // The composer wait is cut to 3 s: without the hold, the brief would be
    // unsent (`prompt_submit_not_ready`) well inside the wait below.
    let Some(h) = Harness::boot_bare(
        "trustok",
        Some(STUB),
        &[
            HOOKS,
            ("MESIMON_COMPOSER_WAIT_MS", "3000"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_SLEEP_MIN_AGE_MS", "0"),
        ],
    ) else {
        return;
    };
    let hooks = h.paths.hook_sock();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    let mut c = h.client("trustok");
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "trust me".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        c.request(Command::WriteNote { ticket, note: None, text: BRIEF.into(), rev: None }),
        Response::NoteWritten { .. }
    ));
    // Nothing to accept yet, in words.
    err_containing(c.request(Command::AcceptTrust { ticket }), "no live agent");
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let session = sid.to_string();

    // (1) The dialog reads TRUST off the screen, with its words, before any
    // hook has fired.
    c.await_state(sid, "the trust dialog", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Trust }
    });
    assert_eq!(
        record(&mut c, sid).detail.as_deref(),
        Some("Claude Code asks to trust this folder")
    );

    // (2) The brief waits on the dialog: past the composer wait, nothing
    // went unsent, nothing was abandoned, and no key went by itself.
    std::thread::sleep(Duration::from_secs(5));
    let rec = record(&mut c, sid);
    assert_eq!(rec.state, SessionState::RequiresAction { reason: Reason::Trust });
    assert!(rec.unsent.is_none(), "the brief went unsent: {:?}", rec.unsent);
    assert!(rec.pending_submit, "the launch's words are still owed");
    let abandoned = ["prompt_submit_abandoned", "prompt_submit_not_ready"];
    assert!(!abandoned.iter().any(|w| feed().contains(w)), "{}", feed());
    assert_eq!(keys(&h), "", "a key went into the dialog with nobody asking");
    assert_eq!(got(&h), "", "words went in past the dialog");

    // (3) The board's `w`: the dialog's own keys, Down to the Yes row and
    // Enter on it, the screen read before each; a second press while the
    // first walks is refused.
    assert!(matches!(c.request(Command::AcceptTrust { ticket }), Response::Ok));
    err_containing(c.request(Command::AcceptTrust { ticket }), "already trusting");
    wait_until(Duration::from_secs(10), "the dialog's Yes", || h.dir.join("trusted").exists());
    assert_eq!(keys(&h), "Down\nEnter\n");

    // (4) Claude Code starts: the composer paints, `SessionStart` fires, the
    // brief goes in as any launch's does, and the feed says the accept took.
    hook_send(&hooks, &session, "SessionStart", r#"{"source":"startup"}"#);
    c.await_state(sid, "the agent started", |s| {
        matches!(s, SessionState::Idle { .. } | SessionState::Running)
    });
    wait_until(Duration::from_secs(15), "the brief pasted", || {
        got(&h).contains("mesimon-trust-11")
    });
    wait_until(Duration::from_secs(10), "the accept's feed line", || {
        feed().contains(r#""cmd":"trust_accept""#) && feed().contains(r#""outcome":"answered""#)
    });
    let rec = record(&mut c, sid);
    assert!(rec.unsent.is_none(), "{:?}", rec.unsent);
    assert_eq!(keys(&h), "Down\nEnter\n", "no key went after the accept");
    // And nothing to accept any more.
    err_containing(c.request(Command::AcceptTrust { ticket }), "no trust dialog");
}

#[test]
fn the_board_trusts_a_codex_seat_s_folder_and_the_prompt_follows() {
    const FAKE_RUNTIME: &str = include_str!("common/fake_codex_runtime.py");
    let Some(h) = Harness::boot_bare("trustcodex", None, &[]) else { return };
    init_repo(&h.repo, "a.txt", "hello\n");
    // The runtime lives beside the board, named to the daemon on a restart
    // (its env is read at spawn), and opens on the trust dialog.
    let runtime = h.dir.join("fake-codex-runtime.py");
    std::fs::write(&runtime, FAKE_RUNTIME).unwrap();
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .unwrap();
    std::fs::write(
        h.dir.join("initial-screen.json"),
        serde_json::json!({
            "screen": "  Trust this folder? Codex can read, edit, and run files here, subject to\r\n  \
                       your permission settings.\r\n› 1. Trust and continue\r\n  2. Quit",
            "cursor": null
        })
        .to_string(),
    )
    .unwrap();
    h.set_env("MESIMON_CODEX_RUNTIME_BIN", &runtime);
    h.set_env("MESIMON_CODEX_BIN", h.dir.join("must-not-launch-real-codex"));
    h.set_env("MESIMON_CODEX_HOME", h.dir.join("synthetic-codex-home"));
    h.set_env("MESIMON_PANE_QUIET_MS", "600000");
    h.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
    h.restart();
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let feed = || std::fs::read_to_string(&feed_path).unwrap_or_default();
    let mut c = h.client("trustcodex");
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Codex }),
        Response::Ok
    ));
    let ticket = match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: "mesimon-trust-871 start after the folder".into(),
        workspace: None,
        tier: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("{other:?}"),
    };
    let sid = match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Codex,
        submit_prompt: true,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn failed: {other:?}"),
    };
    let input = h.dir.join(format!("{sid}.input"));
    let typed = || String::from_utf8_lossy(&std::fs::read(&input).unwrap_or_default()).into_owned();

    // (1) The dialog reads TRUST off the screen, the prompt held behind it.
    c.await_state(sid, "the trust dialog", |s| {
        *s == SessionState::RequiresAction { reason: Reason::Trust }
    });
    let rec = record(&mut c, sid);
    assert_eq!(rec.detail.as_deref(), Some("Codex asks to trust this folder"));
    assert!(rec.pending_submit, "the launch's words are still owed");
    assert_eq!(typed(), "", "a key went into the dialog with nobody asking");

    // (2) The board's `w`: the cursor already on the Yes row, Enter alone.
    assert!(matches!(c.request(Command::AcceptTrust { ticket }), Response::Ok));
    wait_until(Duration::from_secs(10), "the dialog's Yes", || {
        h.dir.join(format!("{sid}.trusted")).exists()
    });

    // (3) Codex starts at its composer: the projection settles on the accept
    // and the prompt goes in once, as any launch's does.
    c.await_state(sid, "the prompt after the trust", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(10), "the accept's feed line", || {
        feed().contains(r#""cmd":"trust_accept""#) && feed().contains(r#""outcome":"answered""#)
    });
    let rec = record(&mut c, sid);
    assert!(rec.unsent.is_none() && !rec.pending_submit, "{:?}", rec.unsent);
    assert_ne!(rec.detail.as_deref(), Some(mesimon_daemon::trust_dialog::NOT_ACCEPTED));
    let typed = typed();
    let before = typed.split("\u{1b}[200~").next().unwrap_or_default();
    assert!(matches!(before, "\r" | "\n"), "one Enter before the prompt: {typed:?}");
    assert_eq!(typed.matches("mesimon-trust-871").count(), 1, "{typed:?}");
    assert_eq!(std::fs::read_to_string(h.dir.join(format!("{sid}.submits"))).unwrap(), "1");
    err_containing(c.request(Command::AcceptTrust { ticket }), "no trust dialog");
}
