//! The crown's train hold (T-919): `hold_ticket` takes one ticket out of
//! the armed merge train for review, as a person's `t` does, and hands it
//! back. Held, the train neither merges the branch nor asks its agent to
//! rebase, the worker's delivery wakes the crown instead of waiting for a
//! merge, and the crown's words still reach the worker; returned, the
//! train takes the branch by its own gates. The hold is the crown's alone
//! (`Action::HoldFromTrain`), a person's `t` stays the person's, a return
//! waits on an autonomous crown, every write is judged against the ticket
//! as read, and the crown's hold outlives a daemon restart.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use mesimon_core::board::{CrownMode, SessionState, WorkspaceStrategy};
use mesimon_core::command::{AgentTicketView, Command, CrownTouch, Deliver, Response};
use mesimon_core::Principal;
use serde_json::json;

mod common;
use common::*;

fn key_of(c: &mut TestClient, id: ulid::Ulid) -> String {
    c.board().ticket(id).unwrap().short_key.clone()
}

fn read(c: &mut TestClient, session: uuid::Uuid, key: &str) -> Result<AgentTicketView, String> {
    match c.send(
        Principal::Agent { session },
        Command::AgentReadTicket { key: key.into(), full: false, plan: false },
    ) {
        Response::AgentTicket { ticket } => Ok(ticket),
        Response::Err { message } => Err(message),
        other => panic!("read {key}: {other:?}"),
    }
}

fn touches(c: &mut TestClient) -> Vec<CrownTouch> {
    match c.request(Command::Snapshot) {
        Response::Board { crown_touches, .. } => crown_touches,
        other => panic!("snapshot: {other:?}"),
    }
}

/// A claude stub that writes every line it is given to `got.txt` beside
/// it, so a test reads what reached the pane: the crown's wakes, a
/// worker's asks, the train's rebase ask.
const RECORDING_STUB: &str =
    "#!/bin/sh\nstty -icanon 2>/dev/null\ncat | while IFS= read -r line; do \
                              printf '%s\\n' \"$line\" >> \"$(dirname \"$0\")/got.txt\"; done\n";

/// One commit in `repo` touching `file`.
fn commit(repo: &std::path::Path, file: &str) {
    std::fs::write(repo.join(file), format!("{file}\n")).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-qm", file]);
}

/// What `hold_ticket` answered: `(held, by, changed, detail)`, or the
/// refusal's words.
type Held = Result<(bool, String, bool, String), String>;

fn hold(
    c: &mut TestClient,
    session: uuid::Uuid,
    key: &str,
    on: bool,
    seen: Option<String>,
) -> Held {
    match c.send(
        Principal::Agent { session },
        Command::AgentHoldTicket { key: key.into(), hold: on, seen },
    ) {
        Response::AgentHeld { held, by, changed, detail, seen, .. } => {
            assert!(seen.is_some(), "a receipt carries the fresh stamp");
            Ok((held, by, changed, detail))
        }
        Response::Err { message } => Err(message),
        other => panic!("hold_ticket {key}: {other:?}"),
    }
}

/// `hold` with a fresh read's stamp.
fn hold_now(c: &mut TestClient, session: uuid::Uuid, key: &str, on: bool) -> Held {
    let seen = read(c, session, key).unwrap().seen;
    hold(c, session, key, on, seen)
}

fn flag(c: &mut TestClient, id: ulid::Ulid) -> bool {
    c.board().ticket(id).unwrap().manual_merge
}

/// The review the person asked for, on a train left armed: the crown holds
/// a worker's ticket before it delivers; the delivery wakes the crown with
/// the merge its own; the train neither merges the branch nor asks the
/// rebase a moved base calls for; the crown's correction reaches the
/// worker while it is held; and the return hands the branch to the train,
/// which merges it by its own gates. A merged branch is then refused a
/// hold: no merge is taken back.
#[test]
fn the_crown_holds_a_ticket_out_of_the_train_for_review() {
    // The flags (and the train) every second rather than every ten.
    let Some(h) = Harness::boot_with_env(
        "crown_hold",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_hold");
    let got = h.dir.join("got.txt");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let feed_count = |needle: &str| -> usize {
        std::fs::read_to_string(&feed_path).unwrap_or_default().matches(needle).count()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-919 held for review");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 1. held before the worker starts: on the flag, on every read ----
    assert_eq!(read(&mut c, sa, &kw).unwrap().held, None);
    let (held, by, changed, detail) = hold_now(&mut c, sa, &kw, true).unwrap();
    assert!(held && changed, "{detail}");
    assert_eq!(by, "crown");
    assert!(detail.contains("out of the merge train"), "{detail}");
    assert!(flag(&mut c, w), "the person's own flag carries the hold");
    assert_eq!(read(&mut c, sa, &kw).unwrap().held.as_deref(), Some("crown"));
    assert!(touches(&mut c).iter().any(|t| t.ticket == w && t.action == "held"));
    // The feed is flushed on the daemon's clock.
    wait_until(std::time::Duration::from_secs(5), "the hold in the feed", || {
        feed_count("\"hold_ticket\"") == 1
    });
    // The same state asked again changes nothing and is no error.
    let (held, by, changed, _) = hold_now(&mut c, sa, &kw, true).unwrap();
    assert!(held && !changed);
    assert_eq!(by, "crown");

    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let path = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    let branch = read(&mut c, sa, &kw).unwrap().branch.expect("a branch");
    std::thread::sleep(std::time::Duration::from_millis(500));
    // This connection arms the train and keeps it armed.
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let worker = format!("{kw} \"mesimon-probe-919 held for review\"");
    let merged = || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };

    // ---- 2. delivered while held: the crown hears it now, nothing merges --
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the delivery's wake", || {
        lines_with(&worker).iter().any(|l| l.contains(" delivered ("))
    });
    let delivered = lines_with(&worker).pop().unwrap();
    assert!(delivered.contains("merge: crown"), "the merge waits on the crown: {delivered}");
    let view = read(&mut c, sa, &kw).unwrap();
    assert_eq!(view.held.as_deref(), Some("crown"));
    let merge = view.merge.expect("a branch has a merger");
    assert_eq!(merge.by, "crown");
    assert!(merge.why.contains("held out of the train by the crown"), "{}", merge.why);
    // Several of the train's passes go by.
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert!(!merged(), "the train merged a held branch");
    assert_eq!(feed_count("merge_train_merged"), 0);
    assert_eq!(
        feed_count("\"crown_wake_deferred\""),
        0,
        "not held for a train that will not take it"
    );
    assert_eq!(feed_count("\"hold_ticket\""), 1, "the second hold changed nothing");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 3. the base moves: no rebase ask; the crown's correction lands ---
    commit(&h.repo, "base1.txt");
    wait_until(std::time::Duration::from_secs(10), "the flags to read it behind", || {
        read(&mut c, sa, &kw).unwrap().merge_state.as_deref() == Some("needs_rebase")
    });
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert!(lines_with("Rebase your current branch").is_empty(), "the train asked a held rebase");
    assert_eq!(feed_count("merge_train_rebase_asked"), 0);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentAskTicket {
            key: kw.clone(),
            text: "mesimon-probe-919 rebase and fix the review note".into(),
            seen: v.seen,
            plan: false,
            deliver: Deliver::Idle,
            expect_secs: None,
        },
    ) {
        Response::AgentAsked { .. } => {}
        other => panic!("ask_agent: {other:?}"),
    }
    // Autonomous sends it on its own; a person's send is the same road.
    let _ = c.request(Command::SendQueuedAsk { ticket: w });
    wait_until(std::time::Duration::from_secs(10), "the correction to reach the worker", || {
        lines_with("mesimon-probe-919 rebase and fix").len() == 1
    });
    start(&mut c, ws);
    git(&path, &["rebase", "-q", "main"]);
    commit(&path, "two.txt");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the corrected turn's wake", || {
        lines_with(&worker).len() >= 2
    });
    wait_until(std::time::Duration::from_secs(10), "the flags to read it ahead", || {
        read(&mut c, sa, &kw).unwrap().merge_state.as_deref() == Some("ahead")
    });
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert!(!merged(), "the train merged a held branch after its correction");
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 4. approved: returned, and the train takes it by its own gates ---
    let (held, by, changed, detail) = hold_now(&mut c, sa, &kw, false).unwrap();
    assert!(!held && changed, "{detail}");
    assert!(by.is_empty());
    assert!(detail.contains("nothing merges it now"), "{detail}");
    assert!(!flag(&mut c, w));
    assert_eq!(read(&mut c, sa, &kw).unwrap().held, None);
    assert!(touches(&mut c).iter().any(|t| t.ticket == w && t.action == "returned"));
    wait_until(std::time::Duration::from_secs(5), "the return in the feed", || {
        feed_count("\"return_ticket\"") == 1
    });
    wait_until(std::time::Duration::from_secs(15), "the train to merge it", merged);
    wait_until(std::time::Duration::from_secs(5), "the train's feed line", || {
        feed_count("merge_train_merged") == 1
    });
    wait_until(std::time::Duration::from_secs(10), "the merge's wake", || {
        lines_with(&worker).iter().any(|l| l.contains(" merged ("))
    });

    // ---- 5. merged: a hold takes back no merge ----------------------------
    wait_until(std::time::Duration::from_secs(10), "the flags to read it merged", || {
        read(&mut c, sa, &kw).unwrap().merge_state.as_deref() == Some("merged")
    });
    let refused = hold_now(&mut c, sa, &kw, true).unwrap_err();
    assert!(refused.contains("already merged"), "{refused}");
    assert!(!flag(&mut c, w));
}

/// Who may hold, and whose hold it is. An uncrowned session reads how the
/// crown is granted and moves nothing; the crown's own ticket and a
/// shared-checkout ticket are refused; a person's `t` is the person's to
/// return, and reading crowns nothing — a person's `t` after the crown read
/// the ticket makes the crown's write stale. A person's `t` on a crown's
/// hold ends it or makes it theirs. A supervised crown holds and does not
/// return. The crown's hold comes back the crown's after a restart, and a
/// crown that leaves leaves its holds standing as the person's.
#[test]
fn the_hold_is_the_crowns_and_a_persons_t_stays_the_persons() {
    let Some(h) = Harness::boot_with_env(
        "crown_hold_who",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let mut c = h.client("crown_hold_who");
    let feed_path = h.paths.state_dir.join("activity.jsonl");
    let feed_count = |needle: &str| -> usize {
        std::fs::read_to_string(&feed_path).unwrap_or_default().matches(needle).count()
    };
    let worktree = |c: &mut TestClient, title: &str| {
        let id = c.create_ticket(title);
        assert!(matches!(
            c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
            Response::Ok
        ));
        id
    };

    let a = c.create_ticket("coordinate");
    let ka = key_of(&mut c, a);
    let p = worktree(&mut c, "a person's hold");
    let kp = key_of(&mut c, p);
    let q = worktree(&mut c, "the crown's hold");
    let kq = key_of(&mut c, q);
    let s = c.create_ticket("in the checkout");
    assert!(matches!(
        c.request(Command::SetWorkspace {
            id: s,
            workspace: Some(WorkspaceStrategy::SharedCheckout)
        }),
        Response::Ok
    ));
    let ks = key_of(&mut c, s);
    let sa = c.spawn_claude(a);
    // A worker in the checkout: a claude on a worktree ticket waits for its cut.
    let sp = c.spawn_claude(s);
    std::thread::sleep(std::time::Duration::from_millis(500));

    // ---- 1. no crown yet: an agent reaching for another ticket is refused -
    let seen = read(&mut c, sp, &ks).unwrap().seen;
    let refused = hold(&mut c, sp, &kq, true, seen).unwrap_err();
    assert!(refused.contains("crown"), "{refused}");
    assert!(!flag(&mut c, q), "a refused caller moved nothing");
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    // A worker under the crown is refused the same way.
    let refused = hold(&mut c, sp, &kq, true, None).unwrap_err();
    assert!(refused.contains("crown"), "{refused}");
    assert!(!flag(&mut c, q));
    // Its own ticket is no road to a hold either: the key must be another's.
    let refused = hold_now(&mut c, sp, &ks, true).unwrap_err();
    assert!(refused.contains("own ticket"), "{refused}");
    assert!(!flag(&mut c, s));

    // ---- 2. the crown: not its own, not a checkout, not without a stamp --
    let refused = hold_now(&mut c, sa, &ka, true).unwrap_err();
    assert!(refused.contains("own ticket"), "{refused}");
    let refused = hold_now(&mut c, sa, &ks, true).unwrap_err();
    assert!(refused.contains("shared checkout"), "{refused}");
    assert!(!flag(&mut c, s));
    let refused = hold(&mut c, sa, &kq, true, None).unwrap_err();
    assert!(refused.contains("seen is required"), "{refused}");
    assert!(!flag(&mut c, q));

    // ---- 3. a person's `t` after the crown's read makes the write stale --
    let stale = read(&mut c, sa, &kp).unwrap().seen;
    assert!(matches!(c.request(Command::SetManualMerge { id: p, on: true }), Response::Ok));
    let refused = hold(&mut c, sa, &kp, false, stale).unwrap_err();
    assert!(refused.contains("changed since it was read"), "{refused}");
    assert!(flag(&mut c, p), "a stale return moved nothing");
    // Read fresh: the person's hold, which the crown does not return.
    assert_eq!(read(&mut c, sa, &kp).unwrap().held.as_deref(), Some("person"));
    let refused = hold_now(&mut c, sa, &kp, false).unwrap_err();
    assert!(refused.contains("by a person"), "{refused}");
    assert!(flag(&mut c, p));
    // Holding it again is the state it is in, and it stays the person's.
    let (held, by, changed, _) = hold_now(&mut c, sa, &kp, true).unwrap();
    assert!(held && !changed);
    assert_eq!(by, "person");
    assert_eq!(read(&mut c, sa, &kp).unwrap().held.as_deref(), Some("person"));
    assert_eq!(feed_count("\"hold_ticket\""), 0);

    // ---- 4. a person's `t` on the crown's hold ends it or makes it theirs -
    let (held, by, changed, _) = hold_now(&mut c, sa, &kq, true).unwrap();
    assert!(held && changed);
    assert_eq!(by, "crown");
    assert!(matches!(c.request(Command::SetManualMerge { id: q, on: false }), Response::Ok));
    assert_eq!(read(&mut c, sa, &kq).unwrap().held, None);
    let (_, by, changed, _) = hold_now(&mut c, sa, &kq, true).unwrap();
    assert!(changed);
    assert_eq!(by, "crown");
    assert!(matches!(c.request(Command::SetManualMerge { id: q, on: true }), Response::Ok));
    assert_eq!(read(&mut c, sa, &kq).unwrap().held.as_deref(), Some("person"));
    let refused = hold_now(&mut c, sa, &kq, false).unwrap_err();
    assert!(refused.contains("by a person"), "{refused}");
    assert!(matches!(c.request(Command::SetManualMerge { id: q, on: false }), Response::Ok));

    // ---- 5. supervised: the crown holds, a person returns ----------------
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Supervised }),
        Response::Ok
    ));
    let (held, by, changed, _) = hold_now(&mut c, sa, &kq, true).unwrap();
    assert!(held && changed);
    assert_eq!(by, "crown");
    let refused = hold_now(&mut c, sa, &kq, false).unwrap_err();
    assert!(refused.contains("Crown mode is supervised"), "{refused}");
    assert!(flag(&mut c, q), "a supervised crown's return moved nothing");
    assert!(matches!(
        c.request(Command::SetCrownMode { mode: CrownMode::Autonomous }),
        Response::Ok
    ));
    // Returning what is not held is the state it is in.
    let (_, _, _, _) = hold_now(&mut c, sa, &kq, false).unwrap();
    let (held, _, changed, detail) = hold_now(&mut c, sa, &kq, false).unwrap();
    assert!(!held && !changed, "{detail}");
    assert!(detail.contains("not held"), "{detail}");

    // ---- 6. the crown's hold outlives a restart, still the crown's -------
    let (held, by, changed, _) = hold_now(&mut c, sa, &kq, true).unwrap();
    assert!(held && changed);
    assert_eq!(by, "crown");
    drop(c);
    h.restart();
    let mut c = h.client("crown_hold_who_2");
    assert!(flag(&mut c, q), "the flag is on the ticket");
    assert!(flag(&mut c, p), "and the person's too");
    wait_until(std::time::Duration::from_secs(10), "the crown's seat back", || {
        read(&mut c, sa, &kq).is_ok()
    });
    assert_eq!(read(&mut c, sa, &kq).unwrap().held.as_deref(), Some("crown"));
    assert_eq!(read(&mut c, sa, &kp).unwrap().held.as_deref(), Some("person"));
    let (held, _, changed, _) = hold_now(&mut c, sa, &kq, false).unwrap();
    assert!(!held && changed, "the crown returns its own hold after a restart");
    assert!(!flag(&mut c, q));

    // ---- 7. through the shim: the word is required, the receipt reads ----
    let mut shim = Shim::start(&h.paths.orch_sock(), sa);
    shim.rpc("initialize", json!({"protocolVersion": "2025-11-25"}));
    shim.notify("notifications/initialized");
    let seen = read(&mut c, sa, &kq).unwrap().seen.unwrap();
    for bad in
        [json!({ "key": kq, "seen": seen }), json!({ "key": kq, "seen": seen, "action": "toggle" })]
    {
        let r = shim.call("hold_ticket", bad);
        assert_eq!(r["isError"], true, "action is a required word: {r}");
    }
    assert!(!flag(&mut c, q));
    let body = shim.call_ok("hold_ticket", json!({ "key": kq, "action": "hold", "seen": seen }));
    assert_eq!(body["outcome"], "held", "{body}");
    assert_eq!(body["held"], "crown", "{body}");
    assert_eq!(body["changed"], true, "{body}");
    assert!(body["seen"].is_string(), "{body}");
    let view = shim.call_ok("get_ticket", json!({ "key": kq }));
    assert_eq!(view["held"], "crown", "{view}");
    let body =
        shim.call_ok("hold_ticket", json!({ "key": kq, "action": "return", "seen": view["seen"] }));
    assert_eq!(body["outcome"], "returned", "{body}");
    assert!(body.get("held").is_none(), "{body}");
    let view = shim.call_ok("get_ticket", json!({ "key": kq }));
    assert!(view.get("held").is_none(), "absent while the train may take it: {view}");
    drop(shim);

    // ---- 8. the crown leaves: its holds stand, as the person's -----------
    let (_, by, changed, _) = hold_now(&mut c, sa, &kq, true).unwrap();
    assert!(changed);
    assert_eq!(by, "crown");
    assert!(matches!(c.request(Command::Uncrown), Response::Ok));
    assert!(flag(&mut c, q), "the hold stands");
    wait_until(std::time::Duration::from_secs(5), "the kept hold in the feed", || {
        feed_count("\"hold_kept\"") == 1
    });
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    assert_eq!(read(&mut c, sa, &kq).unwrap().held.as_deref(), Some("person"));
    let refused = hold_now(&mut c, sa, &kq, false).unwrap_err();
    assert!(refused.contains("by a person"), "{refused}");
}

/// The in-flight boundary: the train asked the worker to rebase, and the
/// crown holds the ticket inside that turn. The words already reached the
/// agent and are not taken back — the receipt says so — so the turn runs
/// and the branch comes out ahead; the train merges nothing after it while
/// the hold stands, and takes it once returned.
#[test]
fn a_hold_inside_the_trains_rebase_turn_lets_the_turn_run_and_merges_nothing() {
    let Some(h) = Harness::boot_with_env(
        "crown_hold_rebase",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_hold_rebase");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-919 held mid-rebase");
    let kw = key_of(&mut c, w);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: w, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let path = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the parked start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    let branch = read(&mut c, sa, &kw).unwrap().branch.expect("a branch");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(matches!(
        c.request(Command::SetAutomation { merge_train: true, merge_notice: false }),
        Response::Ok
    ));
    let merged = || {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&h.repo)
            .args(["merge-base", "--is-ancestor", &branch, "main"])
            .status()
            .is_ok_and(|s| s.success())
    };

    // The base moves first, so the delivery is behind it and the train asks.
    commit(&h.repo, "base1.txt");
    commit(&path, "one.txt");
    start(&mut c, ws);
    stop(&mut c, ws);
    let ask = format!("Rebase your current branch {branch} onto main");
    wait_until(std::time::Duration::from_secs(15), "the train's rebase ask", || {
        !lines_with(&ask).is_empty()
    });

    // The worker takes the ask; the crown holds the ticket inside that turn.
    start(&mut c, ws);
    let (held, by, changed, detail) = hold_now(&mut c, sa, &kw, true).unwrap();
    assert!(held && changed);
    assert_eq!(by, "crown");
    assert!(detail.contains("is not taken back"), "the receipt names the boundary: {detail}");
    git(&path, &["rebase", "-q", "main"]);
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(10), "the flags to read it ahead", || {
        read(&mut c, sa, &kw).unwrap().merge_state.as_deref() == Some("ahead")
    });
    std::thread::sleep(std::time::Duration::from_millis(4000));
    assert!(!merged(), "the train merged after the rebase turn of a held ticket");
    assert_eq!(lines_with(&ask).len(), 1, "one ask, never repeated while held");

    let (held, _, changed, _) = hold_now(&mut c, sa, &kw, false).unwrap();
    assert!(!held && changed);
    wait_until(std::time::Duration::from_secs(15), "the train to merge it once returned", merged);
}

/// T-927: the person moved the crown from one ticket to another, and the
/// new crown heard nothing of the workers the old one had started, nor of
/// the tickets it held for review, though each hold's receipt said their
/// delivery wakes it. What a crown started is the seat's: the crown worn
/// now hears it, and the worker reads that it works under a crown. A hold
/// is heard whoever started the agent, a person's own included, whose
/// agent the handover never makes the crown's. A restart between the
/// handover and the deliveries drops neither.
#[test]
fn a_new_crown_hears_the_old_crown_s_workers_and_the_tickets_it_holds() {
    let Some(h) = Harness::boot_with_env(
        "crown_handover",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_handover");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };
    let woke_by = |c: &mut TestClient, crown: ulid::Ulid, worker: ulid::Ulid| {
        touches(c).iter().any(|t| t.ticket == crown && t.action == "woke" && t.from == Some(worker))
    };

    let a = c.create_ticket("the first crown");
    let b = c.create_ticket("the second crown");
    let w = c.create_ticket("mesimon-probe-927 started by the first crown");
    let p = c.create_ticket("mesimon-probe-927 a person's own");
    let (kw, kp) = (key_of(&mut c, w), key_of(&mut c, p));
    for id in [w, p] {
        assert!(matches!(
            c.request(Command::SetWorkspace { id, workspace: Some(WorkspaceStrategy::Worktree) }),
            Response::Ok
        ));
    }
    let sa = c.spawn_claude(a);
    let sb = c.spawn_claude(b);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 1. the first crown starts a worker; a person starts their own ---
    let v = read(&mut c, sa, &kw).unwrap();
    match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: kw.clone(),
            seen: v.seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { .. } => {}
        other => panic!("start_agent: {other:?}"),
    }
    let w_path = std::path::PathBuf::from(wait_attached(&mut c, w).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the crown's start to land", || {
        c.board().live_agent(w).is_some()
    });
    let ws = c.board().live_agent(w).unwrap().id;
    // A person's own start, parked while its worktree is cut.
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: p,
            kind: mesimon_core::board::SessionKind::Claude,
            submit_prompt: false,
            plan: false,
        }),
        Response::Provisioning | Response::Spawned { .. }
    ));
    let p_path = std::path::PathBuf::from(wait_attached(&mut c, p).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the person's start to land", || {
        c.board().live_agent(p).is_some()
    });
    let sp = c.board().live_agent(p).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The worker is mid-turn when the crown moves.
    start(&mut c, ws);

    // ---- 2. the person moves the crown; the new one holds the person's ---
    assert!(matches!(c.request(Command::CrownTicket { id: b }), Response::Ok));
    start(&mut c, sb);
    stop(&mut c, sb);
    match c.send(Principal::Agent { session: ws }, Command::AgentGetTicket { full: false }) {
        Response::AgentTicket { ticket } => assert_eq!(
            ticket.under_crown.as_deref(),
            Some(mesimon_core::mcp::WORKER_UNDER_CROWN),
            "the worker works under the crown worn now"
        ),
        other => panic!("the worker's get_ticket: {other:?}"),
    }
    match c.send(Principal::Agent { session: sp }, Command::AgentGetTicket { full: false }) {
        Response::AgentTicket { ticket } => {
            assert!(ticket.under_crown.is_none(), "a person's agent is no crown's")
        }
        other => panic!("the person's agent's get_ticket: {other:?}"),
    }
    let (held, by, _, detail) = hold_now(&mut c, sb, &kp, true).unwrap();
    assert!(held && by == "crown", "{detail}");
    assert!(detail.contains("its delivery wakes this session"), "{detail}");
    drop(c);

    // ---- 3. a restart in between --------------------------------------------
    h.restart();
    let mut c = h.client("crown_handover_2");
    assert_eq!(read(&mut c, sb, &kp).unwrap().held.as_deref(), Some("crown"));
    assert!(
        c.board().sessions.iter().all(|s| s.ticket != p || s.started_by.is_none()),
        "the handover made no person's agent the crown's"
    );

    // ---- 4. the old crown's worker delivers: the new crown hears it --------
    let worker_w = format!("{kw} \"mesimon-probe-927 started by the first crown\"");
    commit(&w_path, "w.txt");
    stop(&mut c, ws);
    wait_until(std::time::Duration::from_secs(15), "the old crown's worker's wake", || {
        lines_with(&worker_w).iter().any(|l| l.contains(" delivered ("))
    });
    assert!(woke_by(&mut c, b, w), "the crown worn now was woken");
    start(&mut c, sb);
    stop(&mut c, sb);

    // ---- 5. the held person's agent delivers: the crown hears it too -------
    let worker_p = format!("{kp} \"mesimon-probe-927 a person's own\"");
    commit(&p_path, "p.txt");
    start(&mut c, sp);
    stop(&mut c, sp);
    wait_until(std::time::Duration::from_secs(15), "the held ticket's wake", || {
        lines_with(&worker_p).iter().any(|l| l.contains(" delivered ("))
    });
    assert!(woke_by(&mut c, b, p), "the hold's receipt kept its word");
    // The crown's own ticket and the first crown's never wake for either.
    assert!(!woke_by(&mut c, a, w) && !woke_by(&mut c, a, p));
}

/// T-936, T-920's specimen: a merge the board reads while the crown does
/// not hear the worker still moves what the crown last heard of it. The
/// crown holds a person's ticket and hears its delivery; the person's `t`
/// makes the hold theirs, and they merge it in a terminal, which the crown
/// does not hear. Watched again and looked at again after a restart, the
/// merge is no news: the next turn's line is `finished its turn`, never
/// `merged`.
#[test]
fn a_merge_the_crown_did_not_hear_is_not_news_later() {
    let Some(h) = Harness::boot_with_env(
        "crown_unheard_merge",
        Some(RECORDING_STUB),
        &[
            ("MESIMON_NO_TAG_SEED", "1"),
            ("MESIMON_PANE_QUIET_MS", "600000"),
            ("MESIMON_WT_REFRESH_TICKS", "4"),
        ],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_unheard_merge");
    let got = h.dir.join("got.txt");
    let lines_with = |needle: &str| -> Vec<String> {
        std::fs::read_to_string(&got)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(needle))
            .map(str::to_string)
            .collect()
    };
    let start = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
        c.await_state(sid, "running", |s| *s == SessionState::Running);
    };
    let stop = |c: &mut TestClient, sid: uuid::Uuid| {
        hook_send(&hook_sock, &sid.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
        c.await_state(sid, "idle", |s| matches!(s, SessionState::Idle { .. }));
    };

    let a = c.create_ticket("coordinate");
    let p = c.create_ticket("mesimon-probe-936 a person's own");
    let kp = key_of(&mut c, p);
    assert!(matches!(
        c.request(Command::SetWorkspace { id: p, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(&mut c, sa);
    stop(&mut c, sa);
    assert!(matches!(
        c.request(Command::SpawnSession {
            ticket: p,
            kind: mesimon_core::board::SessionKind::Claude,
            submit_prompt: false,
            plan: false,
        }),
        Response::Provisioning | Response::Spawned { .. }
    ));
    let path = std::path::PathBuf::from(wait_attached(&mut c, p).path.expect("a path"));
    wait_until(std::time::Duration::from_secs(15), "the person's start to land", || {
        c.board().live_agent(p).is_some()
    });
    let sp = c.board().live_agent(p).unwrap().id;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let worker = format!("{kp} \"mesimon-probe-936 a person's own\" ");

    // ---- 1. held by the crown, its delivery heard ---------------------------
    let (held, by, _, _) = hold_now(&mut c, sa, &kp, true).unwrap();
    assert!(held && by == "crown");
    commit(&path, "p.txt");
    wait_until(std::time::Duration::from_secs(15), "the flags to read it ahead", || {
        read(&mut c, sa, &kp).unwrap().merge_state.as_deref() == Some("ahead")
    });
    start(&mut c, sp);
    stop(&mut c, sp);
    wait_until(std::time::Duration::from_secs(15), "the held delivery's wake", || {
        lines_with(&format!("{worker}delivered")).len() == 1
    });
    start(&mut c, sa);
    stop(&mut c, sa);

    // ---- 2. the person's `t` takes the hold; they merge it by hand ---------
    assert!(matches!(c.request(Command::SetManualMerge { id: p, on: true }), Response::Ok));
    assert_eq!(read(&mut c, sa, &kp).unwrap().held.as_deref(), Some("person"));
    let branch = read(&mut c, sa, &kp).unwrap().branch.expect("a branch");
    git(&h.repo, &["merge", "--ff-only", "-q", &branch]);
    wait_until(std::time::Duration::from_secs(15), "the flags to read it merged", || {
        read(&mut c, sa, &kp).unwrap().merge_state.as_deref() == Some("merged")
    });
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(lines_with(&format!("{worker}merged")).is_empty(), "not the crown's to hear");

    // ---- 3. watched again, a restart, a turn: nothing merged is news -------
    let v = read(&mut c, sa, &kp).unwrap();
    assert!(matches!(
        c.send(
            Principal::Agent { session: sa },
            Command::AgentWatchTicket { key: kp.clone(), unwatch: false, seen: v.seen },
        ),
        Response::AgentTicketUpdated { .. }
    ));
    drop(c);
    h.restart();
    let mut c = h.client("crown_unheard_merge_2");
    start(&mut c, sa);
    stop(&mut c, sa);
    start(&mut c, sp);
    stop(&mut c, sp);
    wait_until(std::time::Duration::from_secs(15), "the watched turn's wake", || {
        lines_with(&worker).len() >= 2
    });
    let all = lines_with(&worker);
    assert!(all[1].starts_with(&format!("{worker}finished its turn")), "{all:?}");
    assert!(!all.iter().any(|l| l.starts_with(&format!("{worker}merged"))), "{all:?}");
}

/// T-936: the crown holds each ticket it starts, in either order, without a
/// second read. `start_agent`'s receipt carries the target's fresh stamp, as
/// `hold_ticket`'s does, so a hold right after a start takes the start's
/// stamp, and a start right after a hold takes the hold's. A stamp moves as
/// the agent's state does: the worktree's init script holds the launch here
/// so the hold lands before it.
#[test]
fn a_hold_and_a_start_take_each_other_s_stamp() {
    let Some(h) = Harness::boot_with_env(
        "crown_start_hold",
        Some(RECORDING_STUB),
        &[("MESIMON_NO_TAG_SEED", "1"), ("MESIMON_PANE_QUIET_MS", "600000")],
    ) else {
        return;
    };
    init_repo(&h.repo, "a.txt", "hello\n");
    std::fs::write(h.repo.join(mesimon_core::workspace::INIT_SCRIPT), "#!/bin/sh\nsleep 3\n")
        .unwrap();
    let hook_sock = h.paths.hook_sock();
    let mut c = h.client("crown_start_hold");
    let a = c.create_ticket("coordinate");
    let w = c.create_ticket("mesimon-probe-936 started, then held");
    let v = c.create_ticket("mesimon-probe-936 held, then started");
    let (kw, kv) = (key_of(&mut c, w), key_of(&mut c, v));
    // Filed with its workspace, as the crown files.
    assert!(matches!(
        c.request(Command::SetWorkspace { id: v, workspace: Some(WorkspaceStrategy::Worktree) }),
        Response::Ok
    ));
    let sa = c.spawn_claude(a);
    assert!(matches!(c.request(Command::CrownTicket { id: a }), Response::Ok));
    std::thread::sleep(std::time::Duration::from_millis(500));
    hook_send(&hook_sock, &sa.to_string(), "UserPromptSubmit", r#"{"prompt":"go"}"#);
    hook_send(&hook_sock, &sa.to_string(), "Stop", r#"{"stop_hook_active":false}"#);
    c.await_state(sa, "idle", |s| matches!(s, SessionState::Idle { .. }));
    let start = |c: &mut TestClient, key: &str, seen: Option<String>| match c.send(
        Principal::Agent { session: sa },
        Command::AgentStartTicket {
            key: key.into(),
            seen,
            plan: false,
            tier: None,
            workspace: Some("worktree".into()),
            expect_secs: None,
        },
    ) {
        Response::AgentStarted { seen, .. } => seen,
        other => panic!("start_agent {key}: {other:?}"),
    };

    // Started, then held with the start's stamp.
    let seen = read(&mut c, sa, &kw).unwrap().seen;
    let seen = start(&mut c, &kw, seen);
    assert!(seen.is_some(), "the receipt carries the stamp");
    let (held, by, changed, _) = hold(&mut c, sa, &kw, true, seen).unwrap();
    assert!(held && changed && by == "crown");

    // Held, then started with the hold's stamp.
    let seen = read(&mut c, sa, &kv).unwrap().seen;
    let seen = match c.send(
        Principal::Agent { session: sa },
        Command::AgentHoldTicket { key: kv.clone(), hold: true, seen },
    ) {
        Response::AgentHeld { held: true, seen, .. } => seen,
        other => panic!("hold_ticket {kv}: {other:?}"),
    };
    assert!(start(&mut c, &kv, seen).is_some());
    assert!(flag(&mut c, v), "the hold stands through the start");
}
