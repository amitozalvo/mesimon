//! The daemon core: one writer thread owns board state (D22); client threads
//! forward typed envelopes to it and write back responses. Every mutation calls
//! `authorize()` (D32c) even though v0.1 always allows.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use mesimon_core::attention::{self, Change, Machine};
use mesimon_core::board::{
    Board, Confidence, ExitReason, SessionKind, SessionRecord, SessionState, Ticket, UnknownReason,
};
use mesimon_core::command::{Command, Envelope, Event, GraceItem, Response, PROTOCOL_VERSION};
use mesimon_core::reconcile::{reconcile, state_for};
use mesimon_core::{authorize, fracindex, Action, Decision, Principal, Resource};
use mesimon_backend_tmux::TmuxBackend;

use crate::ingest::{self, HookFrame};
use crate::paths::Paths;
use crate::store;

const GRACE_SECS: u64 = 9;
const GATE_SESSION: &str = "msmn-gate";
/// The deadline wheel (11 §11.7.4 settle timers need finer than 1 s).
const TICK_MS: u64 = 250;
/// Every this-many ticks, check the private tmux server wholesale — pane-died
/// cannot fire for a dead server, so this guard is load-bearing.
const SERVER_GUARD_TICKS: u64 = 60;

struct GraceEntry {
    ticket: Ticket,
    sessions: Vec<SessionRecord>,
    expires: Instant,
}

enum Msg {
    Request(Envelope, Sender<Response>, Arc<Mutex<UnixStream>>),
    Hook(HookFrame),
    Tick,
}

pub struct Daemon {
    paths: Paths,
    board: Board,
    backend: TmuxBackend,
    grace: HashMap<ulid::Ulid, GraceEntry>,
    subscribers: Vec<Arc<Mutex<UnixStream>>>,
    focus: Option<uuid::Uuid>,
    shutting_down: bool,
    /// One attention machine per session, keyed by session UUID.
    machines: HashMap<uuid::Uuid, Machine>,
    /// Startup-modal probe progress per Spawning Claude session:
    /// 1 = the +10s probe ran, 2 = the +30s probe ran (11 §11.5.3 approx).
    probe_stage: HashMap<uuid::Uuid, u8>,
    ticks: u64,
}

pub fn run(paths: Paths) -> Result<()> {
    paths.ensure_dirs()?;

    // Singleton (02 §4): flock on the lock file; loser exits quietly.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock_file())?;
    let rc = unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Ok(()); // another daemon owns this repo
    }
    std::fs::write(paths.lock_file(), format!("{}\n", std::process::id()))?;

    let sock_path = paths.orch_sock();
    let _ = std::fs::remove_file(&sock_path); // stale — we hold the lock
    let listener = UnixListener::bind(&sock_path).context("bind orch.sock")?;

    let backend = TmuxBackend::new(paths.tmux_sock(), &paths.state_dir)?;
    let mut board = store::load(&paths)?;

    // Reconcile persisted records against the live private server (D24).
    let snap = backend.snapshot().unwrap_or_default();
    let rec = reconcile(&board.sessions, &snap);
    for (id, link) in &rec.links {
        if let Some(r) = board.sessions.iter_mut().find(|s| s.id == *id) {
            r.state = state_for(link, &r.state);
        }
    }
    store::save_sessions(&paths, &board)?;

    let (tx, rx) = channel::<Msg>();

    // The deadline wheel: grace expiry, settle timers, the server guard.
    let tick_tx = tx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        if tick_tx.send(Msg::Tick).is_err() {
            break;
        }
    });

    // Accept loop: one reader thread per client.
    let accept_tx = tx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = accept_tx.clone();
            std::thread::spawn(move || client_loop(stream, tx));
        }
    });

    // Hook ingest: one-shot SOCK_STREAM frames from `mesimon hook`. The 0600
    // socket is the authentication (11 §11.2.2 — no token in any agent env).
    let hook_path = paths.hook_sock();
    let _ = std::fs::remove_file(&hook_path);
    let hook_listener = UnixListener::bind(&hook_path).context("bind hook.sock")?;
    std::fs::set_permissions(
        &hook_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;
    let hook_tx = tx.clone();
    std::thread::spawn(move || {
        for stream in hook_listener.incoming().flatten() {
            let tx = hook_tx.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_millis(750)));
                let mut buf = Vec::new();
                use std::io::Read;
                if stream.read_to_end(&mut buf).is_ok() {
                    if let Some(frame) = ingest::parse_frame(&buf) {
                        let _ = tx.send(Msg::Hook(frame));
                    }
                }
            });
        }
    });

    let now = now_ms();
    let machines = board
        .sessions
        .iter()
        .map(|s| (s.id, Machine::new(s.state.clone(), now)))
        .collect();

    let mut d = Daemon {
        paths,
        board,
        backend,
        grace: HashMap::new(),
        subscribers: Vec::new(),
        focus: None,
        shutting_down: false,
        machines,
        probe_stage: HashMap::new(),
        ticks: 0,
    };

    for msg in rx {
        match msg {
            Msg::Tick => d.on_tick(),
            Msg::Hook(frame) => d.on_hook(frame),
            Msg::Request(env, reply, stream) => {
                let resp = d.handle(env, &stream);
                let shutdown = matches!(resp, Response::Ok) && d.shutting_down;
                let _ = reply.send(resp);
                if shutdown {
                    break;
                }
            }
        }
    }
    let _ = std::fs::remove_file(d.paths.orch_sock());
    let _ = std::fs::remove_file(d.paths.hook_sock());
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn client_loop(stream: UnixStream, tx: Sender<Msg>) {
    let writer = Arc::new(Mutex::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    }));
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Envelope>(&line) {
            Ok(env) => {
                let (rtx, rrx) = channel();
                if tx.send(Msg::Request(env, rtx, writer.clone())).is_err() {
                    break;
                }
                rrx.recv().unwrap_or(Response::Err { message: "daemon gone".into() })
            }
            Err(e) => Response::Err { message: format!("bad envelope: {e}") },
        };
        let mut w = match writer.lock() {
            Ok(w) => w,
            Err(_) => break,
        };
        let Ok(json) = serde_json::to_string(&resp) else { break };
        if writeln!(w, "{json}").is_err() {
            break;
        }
    }
}

impl Daemon {
    fn handle(&mut self, env: Envelope, stream: &Arc<Mutex<UnixStream>>) -> Response {
        // D32c invariant 2: the chokepoint is on every path, even though v0.1 allows.
        let action = match &env.command {
            Command::Hello { .. } | Command::Snapshot | Command::Subscribe | Command::GateStatus => Action::Read,
            _ => Action::Mutate,
        };
        if let Decision::Deny { reason } = authorize(&env.principal, &action, &Resource::Board) {
            return Response::Err { message: format!("denied: {reason}") };
        }

        match env.command {
            Command::Hello { version, .. } => {
                if version != PROTOCOL_VERSION {
                    return Response::Err {
                        message: format!("protocol {version} unsupported; daemon speaks {PROTOCOL_VERSION}"),
                    };
                }
                Response::Hello { version: PROTOCOL_VERSION, daemon_pid: std::process::id() }
            }
            Command::Snapshot => {
                self.refresh_states();
                self.snapshot()
            }
            Command::Subscribe => {
                self.subscribers.push(stream.clone());
                Response::Ok
            }
            Command::CreateTicket { column, title } => self.create_ticket(column, title),
            Command::RenameTicket { id, title } => self
                .with_ticket(id, |t| t.title = title)
                .unwrap_or(Response::Err { message: "no such ticket".into() }),
            Command::DeleteTicket { id } => self.delete_ticket(id),
            Command::RestoreTicket { id } => self.restore_ticket(id),
            Command::MoveTicket { id, column, before } => self.move_ticket(id, column, before),
            Command::SpawnSession { ticket, kind } => self.spawn_session(ticket, kind),
            Command::KillSession { id } => self.kill_session(id),
            Command::FocusStart { session } => self.focus_start(session),
            Command::FocusEnd { session } => {
                if self.focus == Some(session) {
                    self.focus = None;
                }
                Response::Ok
            }
            Command::GateStatus => self.gate_status(),
            Command::GatePassed => {
                let _ = std::fs::write(self.paths.gate_file(), "1");
                let _ = self.backend.kill_session(GATE_SESSION);
                Response::Ok
            }
            Command::Shutdown => {
                self.shutting_down = true;
                Response::Ok
            }
        }
    }

    /// One wheel tick (250 ms): grace expiry at the old 1 s cadence, settle
    /// timers, the wholesale-server guard.
    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks % 4 == 0 {
            self.expire_grace();
        }
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> = self
            .machines
            .iter_mut()
            .filter_map(|(id, m)| m.tick(now).map(|c| (*id, c)))
            .collect();
        let mut changed = false;
        for (id, change) in fired {
            changed |= self.apply_change(id, &change, None);
        }
        if self.ticks % 4 == 0 {
            changed |= self.probe_spawning();
        }
        if self.ticks % SERVER_GUARD_TICKS == 0 {
            changed |= self.guard_server();
        }
        if changed {
            let _ = store::save_sessions(&self.paths, &self.board);
            self.broadcast();
        }
    }

    /// 11 §11.5.3, approximated without byte streams: a Spawning Claude pane
    /// that painted output but never set Claude's OSC-0 title and never sent
    /// `SessionStart` is a startup modal (trust dialog); a pane with no output
    /// at all by +30s is `unknown`, never `failed`. Two one-shot tmux forks
    /// per Claude spawn, only while Spawning.
    fn probe_spawning(&mut self) -> bool {
        let now = now_ms();
        let due: Vec<(uuid::Uuid, String, u8)> = self
            .board
            .sessions
            .iter()
            .filter(|r| r.kind == SessionKind::Claude && r.state == SessionState::Spawning)
            .filter_map(|r| {
                let age = now.saturating_sub(r.state_changed_at.unwrap_or(now));
                let stage = self.probe_stage.get(&r.id).copied().unwrap_or(0);
                if age >= 30_000 && stage < 2 {
                    Some((r.id, r.sid16(), 2))
                } else if age >= 10_000 && stage < 1 {
                    Some((r.id, r.sid16(), 1))
                } else {
                    None
                }
            })
            .collect();
        let mut changed = false;
        for (id, sid, stage) in due {
            self.probe_stage.insert(id, stage);
            let bytes = self
                .backend
                .capture_tail(&sid, 3)
                .map(|lines| !lines.is_empty())
                .unwrap_or(false);
            let osc0 = self
                .backend
                .pane_title(&sid)
                .map(|t| t.contains("Claude") || t.contains('✳'))
                .unwrap_or(false);
            // At +10s only the modal case fires; the no-bytes verdict waits
            // for +30s (a slow spawn is not yet a missing one).
            if stage == 1 && !(bytes && !osc0) {
                continue;
            }
            let sig = mesimon_core::attention::Signal::SpawnProbe { bytes, osc0 };
            if let Some(m) = self.machines.get_mut(&id) {
                if let Some(change) = m.apply(&sig, now) {
                    changed |= self.apply_change(id, &change, None);
                }
            }
        }
        changed
    }

    /// pane-died cannot fire for a dead tmux server: if the private server is
    /// gone wholesale, every live card demotes to "unavailable" (D5 — never
    /// render anything from a dead supervisor as blocked).
    fn guard_server(&mut self) -> bool {
        let any_live = self.board.sessions.iter().any(|s| s.state.is_live());
        if !any_live || self.backend.server_alive() {
            return false;
        }
        let now = now_ms();
        let mut changed = false;
        for rec in &mut self.board.sessions {
            if rec.state.is_live() {
                rec.state = SessionState::Unknown { reason: UnknownReason::SupervisorDead };
                rec.confidence = Confidence::Stale;
                rec.waiting_since = None;
                rec.state_changed_at = Some(now);
                self.machines.insert(rec.id, Machine::new(rec.state.clone(), now));
                changed = true;
            }
        }
        changed
    }

    /// One hook frame off the ingest socket (Claude hook or tmux pane-died).
    fn on_hook(&mut self, frame: HookFrame) {
        let Some(id) = self.resolve_session(&frame.session) else { return };
        // D32c invariant 2: the agent-originated path passes the chokepoint too.
        if let Decision::Deny { .. } = authorize(
            &Principal::Agent { session: id },
            &Action::Mutate,
            &Resource::Session { id },
        ) {
            return;
        }
        let now = now_ms();
        let mut dirty = false;
        if let Some(t) = ingest::transcript_of(&frame) {
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                if rec.transcript_path.as_deref() != Some(t.as_str()) {
                    rec.transcript_path = Some(t);
                    dirty = true;
                }
            }
        }
        if let Some(sig) = ingest::signal_of(&frame) {
            let machine = self
                .machines
                .entry(id)
                .or_insert_with(|| Machine::new(SessionState::unknown(), now));
            if let Some(change) = machine.apply(&sig, now) {
                dirty |= self.apply_change(id, &change, ingest::detail_of(&frame));
            }
        }
        if dirty {
            let _ = store::save_sessions(&self.paths, &self.board);
            self.broadcast();
        }
    }

    /// Hooks send the session UUID; the tmux pane-died hook sends the sid16.
    fn resolve_session(&self, key: &str) -> Option<uuid::Uuid> {
        if let Ok(id) = key.parse::<uuid::Uuid>() {
            return self.board.sessions.iter().find(|s| s.id == id).map(|s| s.id);
        }
        self.board.sessions.iter().find(|s| s.sid16() == key).map(|s| s.id)
    }

    /// Fold one debounced machine transition into the session record.
    /// Returns whether anything visible changed (caller persists/broadcasts).
    fn apply_change(&mut self, id: uuid::Uuid, change: &Change, detail: Option<String>) -> bool {
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        let now = now_ms();
        rec.state = change.to.clone();
        rec.confidence = change.confidence;
        rec.state_changed_at = Some(now);
        rec.waiting_since =
            if attention::is_attention(&change.to) { Some(now) } else { None };
        match &change.to {
            SessionState::RequiresAction { .. }
            | SessionState::Failed { .. }
            | SessionState::Throttled => {
                if detail.is_some() {
                    rec.detail = detail;
                }
            }
            _ => rec.detail = None,
        }
        true
    }

    /// M1 liveness: re-reconcile against the live server on every snapshot so
    /// a session that died shows as exited without waiting for a daemon
    /// restart. (M2 replaces the pull with the pane-died hook push.)
    fn refresh_states(&mut self) {
        let Ok(snap) = self.backend.snapshot() else { return };
        let rec = reconcile(&self.board.sessions, &snap);
        let mut changed = false;
        for (id, link) in &rec.links {
            if let Some(r) = self.board.sessions.iter_mut().find(|s| s.id == *id) {
                let next = state_for(link, &r.state);
                if next != r.state {
                    r.state = next.clone();
                    self.machines.insert(*id, Machine::new(next, now_ms()));
                    changed = true;
                }
            }
        }
        if changed {
            let _ = store::save_sessions(&self.paths, &self.board);
        }
    }

    fn snapshot(&self) -> Response {
        let grace = self
            .grace
            .iter()
            .map(|(id, g)| GraceItem {
                id: *id,
                short_key: g.ticket.short_key.clone(),
                title: g.ticket.title.clone(),
                expires_in_secs: g.expires.saturating_duration_since(Instant::now()).as_secs(),
                live_sessions: g.sessions.len(),
            })
            .collect();
        Response::Board { board: self.board.clone(), grace }
    }

    fn broadcast(&mut self) {
        let line = match serde_json::to_string(&Event::BoardChanged) {
            Ok(l) => l,
            Err(_) => return,
        };
        self.subscribers.retain(|s| {
            let Ok(mut w) = s.lock() else { return false };
            writeln!(w, "{line}").is_ok()
        });
    }

    fn persist_and_notify(&mut self) {
        let _ = store::save_columns(&self.paths, &self.board);
        let _ = store::save_sessions(&self.paths, &self.board);
        self.broadcast();
    }

    fn with_ticket(&mut self, id: ulid::Ulid, f: impl FnOnce(&mut Ticket)) -> Option<Response> {
        let t = self.board.ticket_mut(id)?;
        f(t);
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        self.broadcast();
        Some(Response::Ok)
    }

    fn create_ticket(&mut self, column: String, title: String) -> Response {
        if !self.board.columns.iter().any(|c| c.name == column) {
            return Response::Err { message: format!("no such column: {column}") };
        }
        self.board.next_key += 1;
        let last = self
            .board
            .column_tickets(&column)
            .last()
            .map(|t| t.order.clone())
            .unwrap_or_default();
        let t = Ticket {
            id: ulid::Ulid::new(),
            short_key: format!("T-{}", self.board.next_key),
            title,
            column,
            order: fracindex::between(&last, ""),
            created_at: now_iso(),
        };
        let _ = store::save_ticket(&self.paths, &t);
        self.board.tickets.push(t);
        self.persist_and_notify();
        Response::Ok
    }

    fn delete_ticket(&mut self, id: ulid::Ulid) -> Response {
        let Some(pos) = self.board.tickets.iter().position(|t| t.id == id) else {
            return Response::Err { message: "no such ticket".into() };
        };
        let ticket = self.board.tickets.remove(pos);
        // Sessions detach and keep running through the grace band (D21).
        let sessions: Vec<SessionRecord> =
            self.board.sessions.iter().filter(|s| s.ticket == id).cloned().collect();
        self.board.sessions.retain(|s| s.ticket != id);
        let _ = store::delete_ticket_dir(&self.paths, &ticket.short_key);
        self.grace.insert(
            id,
            GraceEntry { ticket, sessions, expires: Instant::now() + Duration::from_secs(GRACE_SECS) },
        );
        self.persist_and_notify();
        Response::Ok
    }

    fn restore_ticket(&mut self, id: ulid::Ulid) -> Response {
        let Some(g) = self.grace.remove(&id) else {
            return Response::Err { message: "grace window expired".into() };
        };
        let _ = store::save_ticket(&self.paths, &g.ticket);
        self.board.tickets.push(g.ticket);
        self.board.sessions.extend(g.sessions);
        self.persist_and_notify();
        Response::Ok
    }

    fn expire_grace(&mut self) {
        let now = Instant::now();
        let expired: Vec<ulid::Ulid> =
            self.grace.iter().filter(|(_, g)| g.expires <= now).map(|(id, _)| *id).collect();
        if expired.is_empty() {
            return;
        }
        for id in expired {
            if let Some(g) = self.grace.remove(&id) {
                for s in &g.sessions {
                    // SIGTERM the group, then remove the pane (docs/19 §1 kill ladder;
                    // the M2 refinement adds the grace-then-kill-pane delay).
                    let _ = self.backend.signal_session(&s.sid16());
                    let _ = self.backend.kill_session(&s.sid16());
                }
            }
        }
        self.broadcast();
    }

    fn move_ticket(&mut self, id: ulid::Ulid, column: String, before: Option<ulid::Ulid>) -> Response {
        if !self.board.columns.iter().any(|c| c.name == column) {
            return Response::Err { message: format!("no such column: {column}") };
        }
        let order = {
            let siblings = self.board.column_tickets(&column);
            let siblings: Vec<&Ticket> = siblings.into_iter().filter(|t| t.id != id).collect();
            match before {
                Some(b) => {
                    let idx = siblings.iter().position(|t| t.id == b);
                    match idx {
                        Some(i) => {
                            let hi = siblings[i].order.clone();
                            let lo = if i == 0 { String::new() } else { siblings[i - 1].order.clone() };
                            fracindex::between(&lo, &hi)
                        }
                        None => fracindex::between(
                            &siblings.last().map(|t| t.order.clone()).unwrap_or_default(),
                            "",
                        ),
                    }
                }
                None => fracindex::between(
                    &siblings.last().map(|t| t.order.clone()).unwrap_or_default(),
                    "",
                ),
            }
        };
        match self.with_ticket(id, |t| {
            t.column = column;
            t.order = order;
        }) {
            Some(r) => r,
            None => Response::Err { message: "no such ticket".into() },
        }
    }

    fn spawn_session(&mut self, ticket: ulid::Ulid, kind: SessionKind) -> Response {
        if self.board.ticket(ticket).is_none() {
            return Response::Err { message: "no such ticket".into() };
        }
        let id = uuid::Uuid::new_v4();
        let argv: Vec<String> = match kind {
            SessionKind::Claude => {
                // Per-session observer hooks via --settings (11 §11.2.1).
                // Never --bare / --safe-mode — both silently clear them (S-D).
                let hook_bin = std::env::var("MESIMON_HOOK_BIN")
                    .map(std::path::PathBuf::from)
                    .or_else(|_| std::env::current_exe())
                    .unwrap_or_else(|_| std::path::PathBuf::from("mesimon"));
                let claude =
                    std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
                match crate::hook_settings::write_settings(&self.paths, id, &hook_bin) {
                    Ok(settings) => vec![
                        claude,
                        "--settings".into(),
                        settings.display().to_string(),
                        "--session-id".into(),
                        id.to_string(),
                    ],
                    Err(e) => {
                        return Response::Err { message: format!("hook settings: {e}") };
                    }
                }
            }
            SessionKind::Bash => vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())],
        };
        let cwd = self.paths.repo_root.clone();
        // Claude enters Spawning; the SessionStart hook flips it to Running.
        // Bash has no hook surface — a live pane is all "running" means (D15).
        let state = match kind {
            SessionKind::Claude => SessionState::Spawning,
            SessionKind::Bash => SessionState::Running,
        };
        let mut rec =
            SessionRecord::new(id, kind, ticket, argv.clone(), cwd.display().to_string(), state);
        rec.state_changed_at = Some(now_ms());
        if let Err(e) = self.backend.spawn(&rec.sid16(), &cwd, &argv, &[]) {
            return Response::Err { message: format!("spawn failed: {e}") };
        }
        self.machines.insert(id, Machine::new(rec.state.clone(), now_ms()));
        self.board.sessions.push(rec);
        self.persist_and_notify();
        Response::Spawned { id }
    }

    fn kill_session(&mut self, id: uuid::Uuid) -> Response {
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        let sid = rec.sid16();
        let _ = self.backend.signal_session(&sid);
        let _ = self.backend.kill_session(&sid);
        rec.state = SessionState::Exited { reason: ExitReason::Killed };
        rec.waiting_since = None;
        rec.detail = None;
        let (id, state) = (rec.id, rec.state.clone());
        self.machines.insert(id, Machine::new(state, now_ms()));
        self.persist_and_notify();
        Response::Ok
    }

    fn focus_start(&mut self, session: uuid::Uuid) -> Response {
        if let Some(holder) = self.focus {
            if holder != session {
                return Response::Err { message: "another session is focused".into() };
            }
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "no such session".into() };
        };
        self.focus = Some(session);
        Response::Attach { argv: self.backend.attach_argv(&rec.sid16()) }
    }

    fn gate_status(&mut self) -> Response {
        if self.paths.gate_file().is_file() {
            return Response::Gate { passed: true, attach_argv: None };
        }
        // Create (idempotently) the gate session the first-run ceremony attaches to (D20).
        let msg = "mesimon first-run check:\\n\\n  This is a live session view.\\n  Press Ctrl+] to return to the board.\\n";
        let argv = vec![
            "sh".into(),
            "-c".into(),
            format!("printf '{msg}'; while true; do sleep 3600; done"),
        ];
        let alive = self
            .backend
            .snapshot()
            .map(|s| s.iter().any(|p| p.session_name == GATE_SESSION && !p.pane_dead))
            .unwrap_or(false);
        if !alive {
            let _ = self.backend.kill_session(GATE_SESSION);
            if let Err(e) = self.backend.spawn(GATE_SESSION, &self.paths.repo_root, &argv, &[]) {
                return Response::Err { message: format!("gate spawn failed: {e}") };
            }
        }
        Response::Gate { passed: false, attach_argv: Some(self.backend.attach_argv(GATE_SESSION)) }
    }
}

fn now_iso() -> String {
    // Seconds precision is enough for created_at; avoid a chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}
