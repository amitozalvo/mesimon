//! A column's `claude_mode` (T-117) is the `--permission-mode` a claude on a
//! ticket there is spawned with: `inherit` is the user's own
//! `permissions.defaultMode`, exactly the pass-through every spawn made
//! before; `plan`/`auto`/`manual` are the column's word; a sleep and wake
//! re-applies whatever the column says NOW; and nothing ever carries
//! `bypassPermissions`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use mesimon_core::board::{ClaudeMode, SessionKind, SessionState};
use mesimon_core::command::{Command, Response};

mod common;
use common::*;

#[test]
fn a_columns_claude_mode_rides_the_spawn_and_the_wake() {
    const STUB: &str = "#!/bin/sh\nwhile IFS= read -r line; do :; done\n";
    let Some(h) = Harness::boot_with_env(
        "clmode",
        Some(STUB),
        &[("MESIMON_PANE_QUIET_MS", "600000"), ("MESIMON_SLEEP_MIN_AGE_MS", "0")],
    ) else {
        return;
    };
    // The user's own default, where the daemon reads it (the harness points
    // MESIMON_CLAUDE_HOME at a scratch dir).
    let home = h.dir.join("claude-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("settings.json"), r#"{"permissions":{"defaultMode":"acceptEdits"}}"#)
        .unwrap();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("clmode");

    // The snapshot says what inherit resolves to.
    match c.request(Command::Snapshot) {
        Response::Board { claude_default_mode, .. } => {
            assert_eq!(claude_default_mode.as_deref(), Some("acceptEdits"));
        }
        other => panic!("{other:?}"),
    }

    let create = |c: &mut TestClient, title: &str| match c.request(Command::CreateTicket {
        column: "TODO".into(),
        title: title.into(),
        workspace: None,
    }) {
        Response::Created { id, .. } => id,
        other => panic!("create: {other:?}"),
    };
    let spawn = |c: &mut TestClient, ticket| match c.request(Command::SpawnSession {
        ticket,
        kind: SessionKind::Claude,
        submit_prompt: false,
        plan: false,
    }) {
        Response::Spawned { id, .. } => id,
        other => panic!("spawn: {other:?}"),
    };
    let mode_of = |c: &mut TestClient, sid| -> Option<String> {
        let rec = c.board().sessions.into_iter().find(|s| s.id == sid).unwrap();
        let at = rec.argv.iter().position(|a| a == "--permission-mode")?;
        rec.argv.get(at + 1).cloned()
    };
    let set_mode_on = |c: &mut TestClient, column: &str, mode: ClaudeMode| {
        let mut s = c.board().column(column).unwrap().settings.clone();
        s.claude_mode = mode;
        assert!(matches!(
            c.request(Command::SetColumnSettings { name: column.into(), settings: s }),
            Response::Ok
        ));
    };
    let set_mode = |c: &mut TestClient, mode: ClaudeMode| set_mode_on(c, "TODO", mode);

    // ---- inherit: the user's default, as before ----------------------------
    let t1 = create(&mut c, "inherits");
    let s1 = spawn(&mut c, t1);
    assert_eq!(mode_of(&mut c, s1).as_deref(), Some("acceptEdits"));

    // ---- the column's word ---------------------------------------------------
    set_mode(&mut c, ClaudeMode::Plan);
    let t2 = create(&mut c, "plans");
    let s2 = spawn(&mut c, t2);
    assert_eq!(mode_of(&mut c, s2).as_deref(), Some("plan"));
    // The live one keeps what it was born with.
    assert_eq!(mode_of(&mut c, s1).as_deref(), Some("acceptEdits"));

    // ---- a wake re-applies the column as it stands then ----------------------
    hook_send(&hook_sock, &s1.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &s1.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(s1, "idle", |s| matches!(s, SessionState::Idle { .. }));
    assert!(matches!(c.request(Command::SleepSession { id: s1 }), Response::Ok));
    c.await_state(s1, "sleeping", |s| *s == SessionState::Sleeping);
    // The prompt and the stop hooks are edges automove acts on (TODO's
    // `on_working`, IN PROGRESS's `on_done`), so the ticket may well have
    // left TODO by now — and the wake reads the column the ticket is in
    // THEN, which is the point: set the mode on that one.
    let column_now = c.board().ticket(t1).unwrap().column.clone();
    set_mode_on(&mut c, &column_now, ClaudeMode::Auto);
    match c.request(Command::WakeSession { id: s1 }) {
        Response::Spawned { id, .. } => assert_eq!(id, s1),
        other => panic!("wake: {other:?}"),
    }
    let rec = c.board().sessions.into_iter().find(|s| s.id == s1).unwrap();
    assert_eq!(
        mode_of(&mut c, s1).as_deref(),
        Some("auto"),
        "the wake picked the column up: {:?}",
        rec.argv
    );
    assert_eq!(
        rec.argv.iter().filter(|a| *a == "--permission-mode").count(),
        1,
        "one pair, replaced, never stacked: {:?}",
        rec.argv
    );

    // ---- back to inherit, and manual is `manual` -----------------------------
    set_mode(&mut c, ClaudeMode::Inherit);
    let t3 = create(&mut c, "inherits again");
    let s3 = spawn(&mut c, t3);
    assert_eq!(mode_of(&mut c, s3).as_deref(), Some("acceptEdits"));
    set_mode(&mut c, ClaudeMode::Manual);
    let t4 = create(&mut c, "manual");
    let s4 = spawn(&mut c, t4);
    assert_eq!(mode_of(&mut c, s4).as_deref(), Some("manual"));

    for rec in c.board().sessions {
        assert!(!rec.argv.iter().any(|a| a == "bypassPermissions"), "{:?}", rec.argv);
    }
    let _ = Duration::from_secs(0);
}
