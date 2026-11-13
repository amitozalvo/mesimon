//! T-1225: an agy session through the real daemon, with a stand-in for agy
//! that runs the hooks and the title command the launch registered. What
//! agy itself does with them was measured on 1.5.8 (STALE-MAP, T-1225).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use mesimon_core::board::{
    AgentProvider, Reason, SessionKind, SessionRecord, SessionState, StopReason, WorkspaceStrategy,
};
use mesimon_core::command::{Command, Response};

const FAKE_AGY: &str = include_str!("../common/fake_agy.py");
/// The id the stand-in mints at its first prompt.
const MINTED: &str = "11111111-2222-4333-8444-555555555555";

struct Bed {
    owner: TestFixture,
    repo: PathBuf,
    paths: mesimon_daemon::Paths,
}

impl Bed {
    /// A daemon whose `agy` is the stand-in. `on` is the switch.
    fn boot(name: &str, on: bool) -> Option<Self> {
        if !require_tmux() {
            return None;
        }
        let owner = TestFixture::new(name);
        let repo = owner.dir.join("repo");
        init_repo(&repo, "README.md", "agy fixture.\n");
        let fake = owner.dir.join("fake_agy.py");
        std::fs::write(&fake, FAKE_AGY).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        owner.set_env("MESIMON_AGY_BIN", &fake);
        owner.set_env("MESIMON_AGY_HOME", owner.dir.join("agy-home"));
        owner.set_env("MESIMON_SLEEP_MIN_AGE_MS", "0");
        owner.set_env("MESIMON_PANE_QUIET_MS", "600000");
        if on {
            owner.set_env("MESIMON_AGY", "1");
        }
        let paths = owner.paths(&repo);
        let _daemon = owner.daemon(&repo);
        wait_until(Duration::from_secs(10), "the daemon socket", || paths.orch_sock().exists());
        Some(Self { owner, repo, paths })
    }

    fn cue(&self, word: &str) {
        let dir = self.owner.dir.join("agy-cue");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(word), "").unwrap();
    }

    /// Every argv the stand-in was started with, in order.
    fn launches(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.owner.dir.join("agy-argv.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
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

fn session(c: &mut TestClient, id: uuid::Uuid) -> SessionRecord {
    c.board().sessions.iter().find(|s| s.id == id).cloned().expect("session")
}

fn pair<'a>(argv: &'a [String], flag: &str) -> Vec<&'a str> {
    argv.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
}

/// A start, a turn with a confirmation in it, a park and a wake: the seat's
/// state follows agy's own reports, the conversation it minted is the one
/// the wake names, and nothing is written into the checkout.
#[test]
fn an_agy_seat_follows_its_hooks_and_wakes_into_its_conversation() {
    let Some(bed) = Bed::boot("agy-turn", true) else { return };
    let mut c = TestClient::connect(&bed.paths.orch_sock());
    // The board's provider is agy's built-in tier.
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Agy }),
        Response::Ok
    ));
    assert_eq!(c.board().agent_provider, AgentProvider::Agy);
    let t = ticket(&mut c, "Teach the parser a new word");

    // A start that carries words is refused before any pane: the board has
    // no road into agy's prompt box yet.
    err_containing(start(&mut c, t, true), "takes no words");
    assert!(bed.launches().is_empty(), "nothing launched for the refused start");

    let id = match start(&mut c, t, false) {
        Response::Spawned { id, .. } => id,
        response => panic!("start: {response:?}"),
    };
    // The prompt box agy's title feed reports is the seat coming up.
    c.await_state(id, "agy at its prompt", |s| matches!(s, SessionState::Idle { .. }));
    let rec = session(&mut c, id);
    assert_eq!(rec.kind, SessionKind::Agy);
    assert_eq!(rec.agy_conversation_id, None, "agy mints its conversation at the first prompt");
    assert_eq!(rec.typed_title, None, "no title is typed into an agy pane");
    let launched = bed.launches();
    assert_eq!(launched.len(), 1, "{launched:?}");
    let folder = PathBuf::from(pair(&launched[0], "--add-dir")[0]);
    let settings = PathBuf::from(pair(&launched[0], "--settings")[0]);
    assert!(folder.starts_with(&bed.paths.state_dir), "{folder:?}");
    assert!(settings.starts_with(&bed.paths.state_dir), "{settings:?}");
    assert!(pair(&launched[0], "--conversation").is_empty(), "{launched:?}");
    assert!(!launched[0].iter().any(|a| a.contains("dangerously")), "{launched:?}");

    // The first model call is the prompt taken, and names the conversation.
    bed.cue("prompt");
    c.await_state(id, "agy working", |s| *s == SessionState::Running);
    wait_until(Duration::from_secs(5), "the conversation id", || {
        session(&mut c, id).agy_conversation_id.as_deref() == Some(MINTED)
    });
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

    // Words for the live pane are refused, not typed.
    err_containing(c.prompt(t, "and now the tests"), "takes no words");

    // Park and wake: the same folder and file, and agy's own conversation.
    assert!(matches!(c.request(Command::SleepSession { id }), Response::Ok));
    c.await_state(id, "parked", |s| matches!(s, SessionState::Sleeping));
    err_containing(c.prompt(t, "wake up and go"), "takes no words");
    assert!(matches!(c.request(Command::WakeSession { id }), Response::Spawned { .. }));
    wait_until(Duration::from_secs(10), "the wake's launch", || bed.launches().len() == 2);
    let woke = &bed.launches()[1];
    assert_eq!(pair(woke, "--conversation"), [MINTED]);
    assert_eq!(pair(woke, "--add-dir"), [folder.to_str().unwrap()]);
    c.await_state(id, "agy back at its prompt", |s| matches!(s, SessionState::Idle { .. }));

    // No hook and no title report printed a byte: an empty object on a tool
    // hook is agy's refusal of the call.
    assert!(!bed.owner.dir.join("agy-stdout.log").exists(), "a reporter wrote to stdout");
    // Nothing of mesimon's is in the checkout or in agy's own folder.
    assert!(!Path::new(&bed.repo).join(".agents").exists());
    assert!(!bed.owner.dir.join("agy-home").exists());
}

/// Without its switch nothing of agy's starts, whatever names it.
#[test]
fn agy_starts_nowhere_without_its_switch() {
    let Some(bed) = Bed::boot("agy-off", false) else { return };
    let mut c = TestClient::connect(&bed.paths.orch_sock());
    // A board may name agy all the same: a start's kind follows its tier.
    assert!(matches!(
        c.request(Command::SetAgentProvider { provider: AgentProvider::Agy }),
        Response::Ok
    ));
    let t = ticket(&mut c, "Quiet");
    err_containing(start(&mut c, t, false), "switched off");
    assert!(c.board().sessions.is_empty());
    assert!(bed.launches().is_empty());
}
