//! Board-owned sleep inhibition, observed independently of terminal handovers.
//!
//! The observer never starts or restarts a daemon. A disconnected daemon
//! leaves the last activity level in place until a fresh snapshot arrives.
//! The holder's lock is never held across daemon I/O: disabling the preference
//! or dropping the board releases it immediately, even if a request is stuck.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mesimon_core::board::Board;
use mesimon_core::command::{Command, Pending, Response};

use crate::caffeine::{Caffeine, Hold};
use crate::client::{Client, Transport};

const BEAT: Duration = Duration::from_millis(100);
const DIAL_EVERY: Duration = Duration::from_secs(2);

fn mid_turn(board: &Board, pending: &[Pending]) -> bool {
    board.sessions.iter().any(mesimon_core::quiet::is_mid_turn)
        || pending.iter().any(|p| p.in_flight)
}

struct State {
    keeper: Caffeine,
    enabled: bool,
    mid_turn: bool,
    generation: u64,
}

impl State {
    fn drive(&mut self) {
        self.keeper.drive(self.enabled && self.mid_turn);
    }
}

/// The TUI's handle; the worker owns observation, while this handle owns the
/// permission to hold. Dropping it revokes that permission synchronously.
pub struct Monitor {
    shared: Arc<Mutex<State>>,
    wake: Sender<()>,
}

impl Monitor {
    pub fn start(
        repo: &Path,
        hold: Hold,
        enabled: bool,
        board: &Board,
        pending: &[Pending],
    ) -> Self {
        let (monitor, wake) = Self::channel(hold, enabled, mid_turn(board, pending));
        let mut worker = Worker::new(repo.to_path_buf(), monitor.shared.clone());
        if let Err(error) = std::thread::Builder::new()
            .name("mesimon-awake".into())
            .spawn(move || worker.run(&wake))
        {
            let mut state = monitor.state();
            state.enabled = false;
            state.drive();
            state.keeper = Caffeine::new(Hold::None);
            // Keep the failure visible through the same status channel as
            // holder failures; no unobserved assertion survives startup.
            state.keeper.report_trouble(format!("keep awake: observer did not start ∙ {error}"));
        }
        monitor
    }

    fn channel(hold: Hold, enabled: bool, mid_turn: bool) -> (Self, Receiver<()>) {
        let (wake, rx) = channel();
        let mut state = State { keeper: Caffeine::new(hold), enabled, mid_turn, generation: 0 };
        state.drive();
        (Self { shared: Arc::new(Mutex::new(state)), wake }, rx)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut state = self.state();
        if state.enabled == enabled {
            return;
        }
        state.enabled = enabled;
        state.generation += 1;
        // Re-enabling must fetch current activity, not resurrect a snapshot
        // from before the observer was disabled.
        state.mid_turn = false;
        state.drive();
        let _ = self.wake.send(());
    }

    pub fn possible(&self) -> bool {
        self.state().keeper.possible()
    }

    pub fn status(&self) -> (bool, Option<String>) {
        let mut state = self.state();
        (state.keeper.holding(), state.keeper.trouble())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.set_enabled(false);
        // Dropping the sender stops the worker. Do not join a worker that
        // could be waiting on a daemon response; it can no longer acquire.
    }
}

struct Worker {
    repo: PathBuf,
    shared: Arc<Mutex<State>>,
    client: Option<Box<dyn Transport + Send>>,
    last_dial: Option<Instant>,
    generation: Option<u64>,
    owes_look: bool,
}

impl Drop for Worker {
    fn drop(&mut self) {
        let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if state.enabled {
            // An unexpected observer exit must not leave an unobserved
            // assertion alive in the board process.
            state.enabled = false;
            state.mid_turn = false;
            state.keeper = Caffeine::new(Hold::None);
            state.keeper.report_trouble("keep awake: observer stopped".into());
        }
    }
}

impl Worker {
    fn new(repo: PathBuf, shared: Arc<Mutex<State>>) -> Self {
        Self { repo, shared, client: None, last_dial: None, generation: None, owes_look: true }
    }

    fn run(&mut self, wake: &Receiver<()>) {
        loop {
            self.pass();
            match wake.recv_timeout(BEAT) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn pass(&mut self) {
        let (enabled, generation) = {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            // Poll child liveness even when no new snapshot is available.
            state.drive();
            (state.enabled, state.generation)
        };
        if !enabled {
            self.client = None;
            self.last_dial = None;
            self.owes_look = true;
            return;
        }
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.owes_look = true;
        }
        if !self.client.as_mut().is_some_and(|c| c.healthy()) {
            if self.last_dial.is_some_and(|t| t.elapsed() < DIAL_EVERY) {
                return;
            }
            self.last_dial = Some(Instant::now());
            self.client = Client::connect_observer(&self.repo)
                .ok()
                .map(|c| Box::new(c) as Box<dyn Transport + Send>);
            self.owes_look = true;
        }
        let Some(client) = self.client.as_mut() else { return };
        while client.poll_event() {
            self.owes_look = true;
        }
        if !self.owes_look {
            return;
        }
        if let Ok(Response::Board { board, pending, .. }) = client.request(Command::Snapshot) {
            let mut state = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            // A response begun before a disable/re-enable cannot authorize
            // a new hold. The next pass will request a fresh snapshot.
            if state.enabled && state.generation == generation {
                state.mid_turn = mid_turn(&board, &pending);
                state.drive();
                self.owes_look = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{Reason, SessionKind, SessionRecord, SessionState, StopReason};

    struct FakeDaemon {
        snapshots: Receiver<Response>,
        current: Response,
    }

    impl Transport for FakeDaemon {
        fn request(&mut self, command: Command) -> anyhow::Result<Response> {
            assert!(matches!(command, Command::Snapshot), "observer only reads snapshots");
            Ok(self.current.clone())
        }

        fn poll_event(&mut self) -> bool {
            if let Ok(snapshot) = self.snapshots.try_recv() {
                self.current = snapshot;
                true
            } else {
                false
            }
        }
    }

    fn snapshot(kind: SessionKind, state: SessionState, in_flight: bool) -> Response {
        let mut board = Board::default();
        board.sessions.push(SessionRecord::new(
            uuid::Uuid::from_u128(1),
            kind,
            ulid::Ulid(1),
            vec![],
            "/repo".into(),
            state,
        ));
        Response::Board {
            board,
            crown_touches: Vec::new(),
            machine_tiers: Default::default(),
            pending: if in_flight {
                vec![Pending {
                    ticket: ulid::Ulid(1),
                    action: mesimon_core::command::PendingAction::Ask,
                    waits_on: vec![],
                    text: None,
                    in_flight: true,
                    by: None,
                    sends: false,
                    accept_plan: false,
                    held: None,
                    plan: false,
                }]
            } else {
                vec![]
            },
            grace: vec![],
            external: vec![],
            external_scanning: false,
            resources: Default::default(),
            worktrees: vec![],
            notices: vec![],
            shell_env: Default::default(),
            git: Default::default(),
            automation: Default::default(),
            claude_md: Default::default(),
            claude_default_mode: None,
            status_top: false,
            team: Default::default(),
            mesophon: Default::default(),
            terminals: Vec::new(),
        }
    }

    fn idle() -> SessionState {
        SessionState::Idle { stop_reason: StopReason::EndTurn }
    }

    fn daemon(worker: &mut Worker, initial: Response) -> Sender<Response> {
        let (tx, snapshots) = channel();
        worker.client = Some(Box::new(FakeDaemon { snapshots, current: initial }));
        worker.owes_look = true;
        tx
    }

    fn wait_for(monitor: &Monitor, held: bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while monitor.status().0 != held {
            assert!(Instant::now() < deadline, "observer did not reach holding={held}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn observes_work_permission_and_completion_without_any_app_ticks() {
        let (monitor, wake) = Monitor::channel(Hold::program("cat"), true, false);
        let mut worker = Worker::new("/repo".into(), monitor.shared.clone());
        let tx = daemon(&mut worker, snapshot(SessionKind::Claude, idle(), false));
        let thread = std::thread::spawn(move || worker.run(&wake));
        // The main thread does no App work: it can be blocked in a handover
        // for the entire sequence, with notifications disabled.
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            tx.send(snapshot(kind, SessionState::Running, false)).unwrap();
            wait_for(&monitor, true);
            tx.send(snapshot(
                kind,
                SessionState::RequiresAction { reason: Reason::Permission },
                false,
            ))
            .unwrap();
            wait_for(&monitor, false);
            tx.send(snapshot(kind, SessionState::Running, false)).unwrap();
            wait_for(&monitor, true);
            // A finished Claude record is provably quiet; unobserved Codex
            // intentionally holds, as quiet::is_mid_turn already tests.
            tx.send(snapshot(SessionKind::Claude, idle(), false)).unwrap();
            wait_for(&monitor, false);
        }
        tx.send(snapshot(SessionKind::Bash, SessionState::Running, true)).unwrap();
        wait_for(&monitor, true);
        tx.send(snapshot(SessionKind::Bash, SessionState::Running, false)).unwrap();
        wait_for(&monitor, false);
        // T-357: a Codex record whose runtime died before confirming cleanup
        // keeps `codex_stopping` (the checkout is still owned), but with no
        // pane there is nothing to keep the machine awake for.
        tx.send(snapshot(SessionKind::Codex, SessionState::Running, false)).unwrap();
        wait_for(&monitor, true);
        let mut dismissed = snapshot(
            SessionKind::Codex,
            SessionState::Exited { reason: mesimon_core::board::ExitReason::Dismissed },
            false,
        );
        if let Response::Board { board, .. } = &mut dismissed {
            board.sessions[0].codex_stopping = true;
            board.sessions[0].observation_hold = true;
            assert!(mesimon_core::quiet::is_working(&board.sessions[0]));
        }
        tx.send(dismissed).unwrap();
        wait_for(&monitor, false);
        drop(monitor);
        thread.join().unwrap();
    }

    #[test]
    fn disabling_releases_and_reenabling_requires_a_fresh_snapshot() {
        let (monitor, _wake) = Monitor::channel(Hold::program("cat"), true, true);
        let mut worker = Worker::new("/repo".into(), monitor.shared.clone());
        daemon(&mut worker, snapshot(SessionKind::Claude, SessionState::Running, false));
        worker.pass();
        assert!(monitor.status().0);
        monitor.set_enabled(false);
        assert!(!monitor.status().0, "release does not wait for the observer");
        worker.pass();
        assert!(worker.client.is_none(), "off drops the subscription");
        monitor.set_enabled(true);
        assert!(!monitor.status().0, "old activity cannot acquire a hold");
        daemon(&mut worker, snapshot(SessionKind::Claude, idle(), false));
        worker.pass();
        assert!(!monitor.status().0);
    }

    #[test]
    fn disconnect_holds_last_activity_and_reconnect_refreshes_it() {
        let (monitor, _wake) = Monitor::channel(Hold::program("cat"), true, true);
        let mut worker = Worker::new("/repo".into(), monitor.shared.clone());
        worker.last_dial = Some(Instant::now());
        worker.pass();
        assert!(monitor.status().0, "a reconnect blip must not interrupt a turn");
        daemon(&mut worker, snapshot(SessionKind::Claude, idle(), false));
        worker.pass();
        assert!(!monitor.status().0);
    }

    #[test]
    fn drop_releases_during_a_blocked_request_and_its_reply_cannot_reacquire() {
        struct Blocked {
            entered: Sender<()>,
            resume: Receiver<()>,
        }
        impl Transport for Blocked {
            fn request(&mut self, _: Command) -> anyhow::Result<Response> {
                self.entered.send(()).unwrap();
                self.resume.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(snapshot(SessionKind::Claude, SessionState::Running, false))
            }
            fn poll_event(&mut self) -> bool {
                false
            }
        }
        let (monitor, _wake) = Monitor::channel(Hold::program("cat"), true, true);
        let shared = monitor.shared.clone();
        let mut worker = Worker::new("/repo".into(), shared.clone());
        let (entered, waiting) = channel();
        let (resume, paused) = channel();
        worker.client = Some(Box::new(Blocked { entered, resume: paused }));
        let thread = std::thread::spawn(move || worker.pass());
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(monitor);
        assert!(!shared.lock().unwrap().keeper.holding());
        resume.send(()).unwrap();
        thread.join().unwrap();
        assert!(!shared.lock().unwrap().keeper.holding(), "late reply cannot revive the hold");
    }
}
