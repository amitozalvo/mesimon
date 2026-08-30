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
use mesimon_core::adopt::{classify_tail_record, TailEvent, TailTool};
use mesimon_core::attention::{self, Change, Machine, Signal, TailHint};
use mesimon_core::board::{
    Board, Confidence, ExitReason, Provenance, SessionKind, SessionRecord, SessionState, Ticket,
    UnknownReason, WorkspaceStrategy,
};
use mesimon_core::command::{
    Command, Envelope, Event, ExternalItem, GraceItem, MergeOutcome, Resources, Response,
    WorktreeItem, PROTOCOL_VERSION,
};
use mesimon_core::reconcile::{reconcile, state_for};
use mesimon_core::{authorize, fracindex, Action, Decision, Principal, Resource};
use mesimon_backend_tmux::TmuxBackend;

use crate::feed::FeedWriter;
use crate::ingest::{self, HookFrame};
use crate::paths::Paths;
use crate::store;
use crate::tail::TailCursor;
use crate::worktree::{self, Binding, BindingStatus};

const GRACE_SECS: u64 = 9;
const GATE_SESSION: &str = "msmn-gate";
/// The deadline wheel (11 §11.7.4 settle timers need finer than 1 s).
const TICK_MS: u64 = 250;
/// Every this-many ticks, check the private tmux server wholesale — pane-died
/// cannot fire for a dead server, so this guard is load-bearing.
const SERVER_GUARD_TICKS: u64 = 60;
/// Observe-tier transcript polling cadence (2 s) — stat-then-read, adopted
/// hook-less sessions only.
const TAIL_POLL_TICKS: u64 = 8;
/// Transcript quiet past this while "running" (Tier-0) demotes to idle.
const TAIL_QUIET_MS: u64 = 45_000;
/// SIGTERM-to-kill-pane grace (docs/19 §1 kill ladder — never SIGKILL).
const REAP_GRACE: Duration = Duration::from_secs(5);
/// D23 floor: a session younger than this in its current state never sleeps.
const SLEEP_MIN_AGE_MS: u64 = 60_000;
/// RSS aggregate refresh (10 s) — one `ps` fork, only while panes exist.
const RSS_TICKS: u64 = 40;
/// Pane silent past this while `Running` means the turn is no longer in
/// flight — the Esc-interrupt catch (spike S-E: an interrupt fires no hook
/// and may write nothing to the transcript; the pane byte stream is the only
/// evidence left). A turn in flight repaints sub-second (spinner), so 8 s is
/// ~8x the largest gap measured while working; idle statusline bursts only
/// delay the verdict, never defeat it.
const PANE_QUIET_MS: u64 = 8_000;
/// Tickets in this column are sleep-safe: their sessions feed the header's
/// sleep suggestion. Interim hardcode — becomes a per-column sleep policy
/// (`never|offer|auto`) with M5's column policies.
const SLEEP_SAFE_COLUMN: &str = "DONE";

struct GraceEntry {
    ticket: Ticket,
    sessions: Vec<SessionRecord>,
    expires: Instant,
    /// The delete-gate's red "remove": the user confirmed losing unmerged
    /// work, so teardown may `branch -D` (M4).
    discard_worktree: bool,
}

enum Msg {
    Request(Envelope, Sender<Response>, Arc<Mutex<UnixStream>>),
    Hook(HookFrame),
    Tick,
    /// A provisioning thread finished (M4): the binding, or the failing stage.
    Provisioned(ulid::Ulid, std::result::Result<Binding, (String, String)>),
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
    feed: FeedWriter,
    /// Discovered foreign sessions (19 §4 tier 1). Never persisted; refreshed
    /// only on `RescanExternal` (the drawer opening).
    external: Vec<ExternalItem>,
    /// Observe-tier transcript cursors for adopted hook-less sessions.
    tails: HashMap<uuid::Uuid, TailCursor>,
    /// Panes SIGTERM'd and awaiting their grace-then-kill-pane (by sid16).
    reaping: HashMap<String, Instant>,
    /// (bytes, sessions seen) — `ps` aggregate, refreshed on the 10 s bucket.
    rss_cache: (u64, usize),
    /// Per-session slice of that aggregate, same bucket — feeds the sleep
    /// suggestion's "free ~X" figure.
    rss_by: HashMap<uuid::Uuid, u64>,
    /// (bytes, sessions) currently sleepable on sleep-safe tickets — the
    /// header suggestion, recomputed on the RSS bucket.
    reclaim_cache: (u64, usize),
    /// Recounted at startup and immediately before each spawn (14 §5.1).
    pty_cache: crate::resources::PtyFigures,
    /// Last breadcrumb pushed into the tmux status line — dedupes the
    /// set-option so board churn doesn't spam the server.
    last_status_left: Option<String>,
    /// The focused session's breadcrumb leaf ("claude"/"bash", or the pane
    /// title when the agent named itself). Cached at FocusStart — broadcast
    /// refreshes must not query tmux per board change.
    focus_label: String,
    /// tmux reports the hostname as `#{pane_title}` when the app never set
    /// one — cached once so title filtering doesn't fork per tick.
    hostname: String,
    /// Per-ticket worktree bindings (M4), persisted as worktrees.json.
    worktrees: worktree::Bindings,
    /// Spawn requests parked behind provisioning: replayed on Provisioned(Ok).
    pending_spawns: Vec<(ulid::Ulid, SessionKind)>,
    /// merged/ahead/conflict flags, refreshed on the 10 s bucket while
    /// bindings exist.
    wt_merged: HashMap<ulid::Ulid, bool>,
    wt_ahead: HashMap<ulid::Ulid, u32>,
    wt_needs_rebase: HashMap<ulid::Ulid, bool>,
    wt_conflicts: Vec<String>,
    /// Cached default-branch name (origin/HEAD → main/master/trunk → HEAD).
    base_branch: Option<String>,
    /// Tickets whose grace expired while their panes were still reaping —
    /// worktree teardown waits for the reaper (never remove a live cwd).
    pending_teardown: Vec<(ulid::Ulid, bool, Vec<String>)>,
    /// Writer-thread sender, cloned into provisioning threads.
    tx: Sender<Msg>,
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

    // The pane-died notify reuses the hook binary and frame (spike T-7: the
    // hook is the ONLY timely death signal). Conf covers fresh servers; the
    // live-server install below covers one that outlived a daemon restart.
    let hook_bin = std::env::var("MESIMON_HOOK_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|_| std::env::current_exe())
        .unwrap_or_else(|_| std::path::PathBuf::from("mesimon"));
    let pane_died = mesimon_backend_tmux::conf::pane_died_cmd(
        &hook_bin.display().to_string(),
        &paths.hook_sock().display().to_string(),
    );
    let backend = TmuxBackend::new(paths.tmux_sock(), &paths.state_dir, Some(&pane_died))?;
    if backend.server_alive() {
        let _ = backend.install_pane_died_hook(&pane_died);
        let _ = backend.install_copy_bindings();
    }
    let mut board = store::load(&paths)?;

    // Reconcile persisted records against the live private server (D24).
    let snap = backend.snapshot().unwrap_or_default();
    let rec = reconcile(&board.sessions, &snap);
    for (id, link) in &rec.links {
        if let Some(r) = board.sessions.iter_mut().find(|s| s.id == *id) {
            // Observe-only records (imported, never spawned) have no pane by
            // design — Missing is their normal condition, not a crash.
            let observe_only =
                r.provenance == Provenance::Adopted && r.argv.is_empty();
            if observe_only && matches!(link, mesimon_core::reconcile::Link::Missing) {
                continue;
            }
            r.state = state_for(link, &r.state, r.kind == SessionKind::Claude);
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
    let feed = FeedWriter::open(&paths.activity_log())?;

    // M4: load worktree bindings; reconcile (a missing dir is Evicted, not an
    // error — diffs still render from the object store); sweep our stale locks.
    let mut worktrees = worktree::load_bindings(&paths).unwrap_or_default();
    let mut wt_changed = worktree::reconcile_interrupted(&paths.repo_root, &mut worktrees);
    for b in worktrees.values_mut() {
        if b.status == BindingStatus::Attached && !b.path.is_dir() {
            b.status = BindingStatus::Evicted;
            wt_changed = true;
        }
    }
    if wt_changed {
        let _ = worktree::save_bindings(&paths, &worktrees);
    }
    if !worktrees.is_empty() {
        let _ = worktree::sweep_stale_locks(&paths.repo_root);
    }

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
        feed,
        external: Vec::new(),
        tails: HashMap::new(),
        reaping: HashMap::new(),
        rss_cache: (0, 0),
        rss_by: HashMap::new(),
        reclaim_cache: (0, 0),
        pty_cache: crate::resources::pty_figures(),
        last_status_left: None,
        focus_label: String::new(),
        hostname: std::process::Command::new("hostname")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default(),
        worktrees,
        pending_spawns: Vec::new(),
        wt_merged: HashMap::new(),
        wt_ahead: HashMap::new(),
        wt_needs_rebase: HashMap::new(),
        wt_conflicts: Vec::new(),
        base_branch: None,
        pending_teardown: Vec::new(),
        tx: tx.clone(),
    };
    d.refresh_worktree_flags();

    for msg in rx {
        match msg {
            Msg::Tick => d.on_tick(),
            Msg::Hook(frame) => d.on_hook(frame),
            Msg::Provisioned(ticket, result) => d.on_provisioned(ticket, result),
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
    let _ = d.feed.flush();
    let _ = std::fs::remove_file(d.paths.orch_sock());
    let _ = std::fs::remove_file(d.paths.hook_sock());
    Ok(())
}

impl Daemon {
    /// Per-session env for a worktree-bound ticket (M4 layer 1: silent,
    /// zero-token indication that the session lives in a ticket worktree —
    /// hooks and scripts key off it; the agent sees it when it looks).
    fn worktree_env(&self, ticket: ulid::Ulid, cwd: &std::path::Path) -> Vec<(String, String)> {
        let Some(b) = self.worktrees.get(&ticket) else { return Vec::new() };
        if b.status != BindingStatus::Attached || b.path != cwd {
            return Vec::new();
        }
        let key =
            self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        vec![
            ("MESIMON_TICKET".into(), key),
            ("MESIMON_WORKTREE_BRANCH".into(), b.branch.clone()),
        ]
    }
}

/// The user's own configured permission default mode, read from the same
/// config-home ladder the census uses (MESIMON_CLAUDE_HOME → CLAUDE_CONFIG_DIR
/// → ~/.claude). `permissions.defaultMode` first, top-level `defaultMode` as
/// the legacy spelling. Read-only — mesimon never writes config (doctor rule).
fn user_default_mode() -> Option<String> {
    let home = std::env::var("MESIMON_CLAUDE_HOME")
        .or_else(|_| std::env::var("CLAUDE_CONFIG_DIR"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
        });
    let text = std::fs::read_to_string(home.join("settings.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("permissions")
        .and_then(|p| p.get("defaultMode"))
        .or_else(|| v.get("defaultMode"))
        .and_then(|m| m.as_str())
        .map(str::to_string)
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
            Command::Hello { .. }
            | Command::Snapshot
            | Command::Subscribe
            | Command::GateStatus
            // Mutates only the daemon's discovery cache, never board state.
            | Command::RescanExternal => Action::Read,
            _ => Action::Mutate,
        };
        if let Decision::Deny { reason } = authorize(&env.principal, &action, &Resource::Board) {
            return Response::Err { message: format!("denied: {reason}") };
        }

        let feed_cmd: Option<(&'static str, Option<ulid::Ulid>)> = match &env.command {
            Command::CreateTicket { .. } => Some(("create_ticket", None)),
            Command::RenameTicket { id, .. } => Some(("rename_ticket", Some(*id))),
            Command::DeleteTicket { id, .. } => Some(("delete_ticket", Some(*id))),
            Command::SetWorkspace { id, .. } => Some(("set_workspace", Some(*id))),
            Command::MergeTicket { id } => Some(("merge_ticket", Some(*id))),
            Command::MergeToAgent { id, .. } => Some(("merge_to_agent", Some(*id))),
            Command::RestoreTicket { id } => Some(("restore_ticket", Some(*id))),
            Command::MoveTicket { id, .. } => Some(("move_ticket", Some(*id))),
            Command::SpawnSession { ticket, .. } => Some(("spawn_session", Some(*ticket))),
            Command::KillSession { .. } => Some(("kill_session", None)),
            Command::AttachExternal { ticket, .. } => Some(("attach_external", *ticket)),
            Command::ResumeExternal { ticket, .. } => Some(("resume_external", *ticket)),
            Command::ResumeSession { .. } => Some(("resume_session", None)),
            Command::SleepSession { .. } => Some(("sleep_session", None)),
            Command::WakeSession { .. } => Some(("wake_session", None)),
            Command::ReclaimAll => Some(("reclaim_all", None)),
            Command::PinAwake { .. } => Some(("pin_awake", None)),
            _ => None,
        };

        let resp = match env.command {
            Command::Hello { version, .. } => {
                if version != PROTOCOL_VERSION {
                    return Response::Err {
                        message: format!("protocol {version} unsupported; daemon speaks {PROTOCOL_VERSION}"),
                    };
                }
                Response::Hello { version: PROTOCOL_VERSION, daemon_pid: std::process::id() }
            }
            Command::Snapshot => self.snapshot(),
            Command::RescanExternal => {
                self.rescan_external();
                // Reply with the fresh board so the drawer opens in one round trip.
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
            Command::DeleteTicket { id, discard_worktree } => {
                self.delete_ticket(id, discard_worktree)
            }
            Command::SetWorkspace { id, workspace } => self.set_workspace(id, workspace),
            Command::MergeTicket { id } => self.merge_ticket(id),
            Command::MergeToAgent { id, request } => self.merge_to_agent(id, request),
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
            Command::AttachExternal { claude_session_id, ticket } => {
                match self.attach_external(claude_session_id, ticket) {
                    Ok(id) => {
                        self.persist_and_notify();
                        Response::Spawned { id }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            Command::ResumeExternal { claude_session_id, ticket, confirm } => {
                match self.attach_external(claude_session_id, ticket) {
                    Ok(id) => {
                        // Attach stands even if the resume below is refused —
                        // the session is on the board as observe-only either way.
                        let resp = self.resume_session(id, confirm);
                        self.persist_and_notify();
                        resp
                    }
                    Err(message) => Response::Err { message },
                }
            }
            Command::ResumeSession { id, confirm } => {
                let resp = self.resume_session(id, confirm);
                self.persist_and_notify();
                resp
            }
            Command::SleepSession { id } => match self.sleep_one(id, false) {
                Ok(()) => {
                    self.persist_and_notify();
                    Response::Ok
                }
                Err(message) => Response::Err { message },
            },
            Command::WakeSession { id } => {
                let resp = self.wake_session(id);
                self.persist_and_notify();
                resp
            }
            Command::ReclaimAll => {
                let (slept, skipped) = self.reclaim_all();
                if slept > 0 {
                    // Re-price the offer now — a taken suggestion must not
                    // linger in the header until the next 10 s RSS bucket.
                    self.reclaim_cache = self.reclaim_figures();
                    self.persist_and_notify();
                }
                Response::Reclaimed { slept, skipped }
            }
            Command::PinAwake { id, pinned } => {
                match self.board.sessions.iter_mut().find(|s| s.id == id) {
                    Some(rec) => {
                        rec.pinned_awake = pinned;
                        self.persist_and_notify();
                        Response::Ok
                    }
                    None => Response::Err { message: "no such session".into() },
                }
            }
        };
        if let Some((cmd, ticket)) = feed_cmd {
            match &resp {
                Response::Ok | Response::Spawned { .. } | Response::Provisioning => {
                    self.feed.board("local", cmd, ticket)
                }
                Response::Created { id } => self.feed.board("local", cmd, ticket.or(Some(*id))),
                Response::Merge {
                    outcome: MergeOutcome::Merged | MergeOutcome::AlreadyMerged, ..
                } => self.feed.board("local", cmd, ticket),
                _ => {}
            }
        }
        resp
    }

    /// One wheel tick (250 ms): grace expiry at the old 1 s cadence, settle
    /// timers, the wholesale-server guard.
    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks % 4 == 0 {
            self.expire_grace();
            self.sweep_reaping();
            self.process_teardowns();
        }
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> = self
            .machines
            .iter_mut()
            .filter_map(|(id, m)| m.tick(now).map(|c| (*id, c)))
            .collect();
        let mut changed = false;
        for (id, change) in fired {
            changed |= self.apply_change(id, &change, None, None);
        }
        if self.ticks % 4 == 0 {
            changed |= self.probe_spawning();
            changed |= self.probe_activity();
        }
        if self.ticks % TAIL_POLL_TICKS == 0 {
            changed |= self.poll_tails();
            changed |= self.refresh_titles();
        }
        if self.ticks % RSS_TICKS == 0 {
            changed |= self.refresh_rss();
            if !self.worktrees.is_empty() {
                self.refresh_worktree_flags();
            }
        }
        if self.ticks % server_guard_ticks() == 0 {
            changed |= self.guard_server();
        }
        if changed {
            let _ = store::save_sessions(&self.paths, &self.board);
            self.broadcast();
        }
        // ≤1 write() per wheel bucket, no fsync (14 §1.7).
        let _ = self.feed.flush();
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
            let resume = self
                .board
                .sessions
                .iter()
                .find(|r| r.id == id)
                .is_some_and(|r| r.argv.iter().any(|a| a == "--resume"));
            let sig = mesimon_core::attention::Signal::SpawnProbe { bytes, osc0, resume };
            if let Some(m) = self.machines.get_mut(&id) {
                if let Some(change) = m.apply(&sig, now) {
                    changed |= self.apply_change(id, &change, None, Some("probe"));
                }
            }
        }
        changed
    }

    /// The Esc-interrupt catch (11 §11.7.3's interrupt row, spike S-E): a
    /// user interrupt fires no hook, so a `Running` pane of ours that has
    /// stopped painting past the quiet threshold is a turn that ended. One
    /// `list-panes` fork, and only while something is actually Running.
    /// Demotion-only — promotion stays hooks-only, so a wrong verdict costs a
    /// cosmetic "idle" that the next real event corrects.
    fn probe_activity(&mut self) -> bool {
        let cands: Vec<(uuid::Uuid, String)> = self
            .board
            .sessions
            .iter()
            .filter(|r| {
                r.kind == SessionKind::Claude
                    && r.state == SessionState::Running
                    // The observe tier has no pane of ours; its quiet detector
                    // is the transcript's (poll_tails).
                    && !(r.provenance == Provenance::Adopted && r.argv.is_empty())
            })
            .map(|r| (r.id, r.sid16()))
            .collect();
        if cands.is_empty() {
            return false;
        }
        let Ok(activity) = self.backend.activity() else { return false };
        let now = now_ms();
        let quiet_ms = pane_quiet_ms();
        let mut changed = false;
        for (id, sid) in cands {
            // A pane missing from the listing is pane-died territory, not ours.
            let Some((_, at)) = activity.iter().find(|(name, _)| *name == sid) else { continue };
            if now.saturating_sub(at * 1000) < quiet_ms {
                continue;
            }
            if let Some(change) =
                self.machines.get_mut(&id).and_then(|m| m.apply(&Signal::PaneQuiet, now))
            {
                changed |= self.apply_change(id, &change, None, Some("activity"));
            }
        }
        changed
    }

    /// Session names for the board: latch each live pane's OSC-0 title onto
    /// its record (one batched tmux fork on the tail-poll bucket) so the TUI
    /// shows what the agent calls itself — the same source the focused tmux
    /// status line's breadcrumb leaf uses. The hostname means never-set (see
    /// `pane_title`) and a missing pane keeps the last name: latch, never
    /// clear, so sleeping/parked sessions stay recognizable.
    fn refresh_titles(&mut self) -> bool {
        if !self.board.sessions.iter().any(|r| r.state.has_pane()) {
            return false;
        }
        let Ok(titles) = self.backend.titles() else { return false };
        let mut changed = false;
        for rec in self.board.sessions.iter_mut().filter(|r| r.state.has_pane()) {
            let sid = rec.sid16();
            let Some((_, t)) = titles.iter().find(|(name, _)| *name == sid) else { continue };
            if t.is_empty() || *t == self.hostname {
                continue;
            }
            // Bound what rides the wire and the store; control chars out.
            // Claude Code prefixes its own spinner glyph inside the title
            // ("✳ fix the parser") — strip leading marks so the TUI's kind
            // mark isn't doubled (dogfood 2026-08-30: "✻ ✳ name" rows).
            let clean: String = t.chars().filter(|c| !c.is_control()).take(80).collect();
            let clean = clean
                .trim_start_matches(|c: char| {
                    matches!(c, '✳' | '✻' | '✽' | '✶' | '✢' | '*' | '·') || c.is_whitespace()
                })
                .to_string();
            if clean.is_empty() {
                continue;
            }
            if rec.title.as_deref() != Some(clean.as_str()) {
                rec.title = Some(clean);
                changed = true;
            }
        }
        changed
    }

    /// Observe tier (19 §4 tier 2): adopted sessions with no process of ours
    /// get their state from the transcript tail, at `Confidence::Low` only.
    /// Our own Claude sessions borrow the same tier while `Unknown` — a
    /// daemon restart mid-turn strands them there with no hook due until the
    /// next turn boundary (dogfood 2026-08-30: "?" while Claude visibly
    /// streams). Leaving `Unknown` ends the candidacy: hooks own again and
    /// the cursor is dropped.
    ///
    /// Third class, abort-only: our own `Running` sessions. An Esc interrupt
    /// fires no hook, and the pane-quiet probe is defeated by Claude Code's
    /// post-turn painting (dogfood 2026-08-30: an idle pane kept
    /// `window_activity` fresh for 60–80 s, so the interrupted card read
    /// "working" until the user killed it) — but the transcript records
    /// "[Request interrupted by user]" at the keypress. Only the Aborted
    /// hint is forwarded for this class: everything else stays hooks-owned,
    /// and a silent transcript during a long tool run must never demote.
    fn poll_tails(&mut self) -> bool {
        let now = now_ms();
        let cands: Vec<(uuid::Uuid, String, bool)> = self
            .board
            .sessions
            .iter()
            .filter_map(|r| {
                let observe_only =
                    r.provenance == Provenance::Adopted && r.argv.is_empty() && r.state.is_live();
                let ours = r.kind == SessionKind::Claude
                    && !(r.provenance == Provenance::Adopted && r.argv.is_empty());
                let ours_lost = ours && matches!(r.state, SessionState::Unknown { .. });
                let abort_only = ours && r.state == SessionState::Running;
                if !(observe_only || ours_lost || abort_only) {
                    return None;
                }
                r.transcript_path.clone().map(|t| (r.id, t, abort_only))
            })
            .collect();
        self.tails.retain(|id, _| cands.iter().any(|(cid, _, _)| cid == id));

        let mut changed = false;
        for (id, tpath, abort_only) in cands {
            let path = std::path::PathBuf::from(&tpath);
            // Mint-time backfill for Unknown sessions: a transcript that
            // never grows again (turn ended before the restart) would leave
            // the card at "?" until the next prompt. One bounded read of how
            // the transcript RESTED seeds a state — the only look at history
            // the "history is not activity" rule permits, because Low
            // confidence can seed a state but never announce anything.
            let fresh = self.tails.get(&id).is_none_or(|c| c.path != path);
            let backfill = if fresh
                && self
                    .board
                    .sessions
                    .iter()
                    .any(|r| r.id == id && matches!(r.state, SessionState::Unknown { .. }))
            {
                resting_hint(&path, now)
            } else {
                None
            };
            let cursor = self
                .tails
                .entry(id)
                .or_insert_with(|| TailCursor::at_end(path.clone(), now));
            if cursor.path != path {
                *cursor = TailCursor::at_end(path.clone(), now);
            }
            let lines = cursor.poll(now);
            let quiet = now.saturating_sub(cursor.grew_at);

            let mut hints: Vec<(TailHint, Option<String>)> = Vec::new();
            hints.extend(backfill.map(|h| (h, None)));
            for line in &lines {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
                match classify_tail_record(&v) {
                    TailEvent::Aborted => hints.push((TailHint::AbortedMidStream, None)),
                    _ if abort_only => {}
                    TailEvent::AssistantText { text } => hints
                        .push((TailHint::AssistantText, Some(crate::census::sanitize(&text)))),
                    TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion } => {
                        hints.push((TailHint::AskUserQuestion, None))
                    }
                    TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode } => {
                        hints.push((TailHint::ExitPlanMode, None))
                    }
                    TailEvent::TurnComplete => hints.push((TailHint::TurnComplete, None)),
                    TailEvent::Latch | TailEvent::Other => {}
                }
            }
            if !abort_only
                && hints.is_empty()
                && quiet >= TAIL_QUIET_MS
                && self
                    .board
                    .sessions
                    .iter()
                    .any(|r| r.id == id && r.state == SessionState::Running)
            {
                hints.push((TailHint::StaleQuiet, None));
            }

            for (hint, preview) in hints {
                let sig = Signal::TranscriptHint { kind: hint };
                if let Some(change) = self.machines.get_mut(&id).and_then(|m| m.apply(&sig, now))
                {
                    changed |= self.apply_change(id, &change, None, Some("tail"));
                }
                // The preview outlives the state word (apply_change wipes
                // detail outside attention states) — set it after.
                if let Some(p) = preview {
                    if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                        if rec.detail.as_deref() != Some(p.as_str()) {
                            rec.detail = Some(p);
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }

    /// pane-died cannot fire for a dead tmux server: if the private server is
    /// gone wholesale, every card whose pane WE hold demotes to "unavailable"
    /// (D5 — never render anything from a dead supervisor as blocked). Scope:
    /// records that actually have a pane on our server. Not `Sleeping` (no
    /// pane, no process — and batch sleep legitimately empties the private
    /// server, since tmux exits when its last session ends, which must not
    /// break the Sleeping latch), and not observe-only adopted records (their
    /// process lives in the user's own terminal, not our tmux).
    fn guard_server(&mut self) -> bool {
        fn ours_paned(r: &SessionRecord) -> bool {
            r.state.has_pane() && !(r.provenance == Provenance::Adopted && r.argv.is_empty())
        }
        if !self.board.sessions.iter().any(ours_paned) || self.backend.server_alive() {
            return false;
        }
        let now = now_ms();
        let mut changed = false;
        for rec in &mut self.board.sessions {
            if ours_paned(rec) {
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
        // Every received frame is feed-logged by NAME only — never its
        // payload (D11: prompt text is read, never stored).
        self.feed.hook_event(&frame.session, &frame.event, frame.reason.as_deref());
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
                    rec.transcript_path = Some(t.clone());
                    dirty = true;
                }
                // The transcript filename IS the conversation id — after an
                // in-app /resume the pane hosts a conversation that is not
                // the record's minted uuid, and this stem is the only place
                // the handoff surfaces (dogfood 2026-08-30: without it,
                // resume targeted the record's own id — a conversation that
                // never existed). Resume targets claude_session_id, so
                // relearn it here.
                let stem = std::path::Path::new(&t)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<uuid::Uuid>().ok());
                if let Some(cid) = stem {
                    // Own-id conversations keep the field empty (the rec.id
                    // fallback covers them); a later handoff back also
                    // clears a previously learned foreign id.
                    let learned = (cid != rec.id).then_some(cid);
                    if rec.claude_session_id != learned {
                        rec.claude_session_id = learned;
                        dirty = true;
                    }
                }
            }
        }
        if let Some(sig) = ingest::signal_of(&frame) {
            let machine = self
                .machines
                .entry(id)
                .or_insert_with(|| Machine::new(SessionState::unknown(), now));
            if let Some(change) = machine.apply(&sig, now) {
                dirty |= self.apply_change(id, &change, ingest::detail_of(&frame), Some(&frame.event));
            }
            // Harvest done (status came in the frame) — remove the dead pane
            // remain-on-exit was holding (docs/19 §1 lifecycle).
            if matches!(sig, Signal::PaneDied { .. }) {
                if let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) {
                    let _ = self.backend.kill_session(&rec.sid16());
                }
            }
        }
        if dirty {
            let _ = store::save_sessions(&self.paths, &self.board);
            self.broadcast();
        }
    }

    /// Hooks send the session UUID; the tmux pane-died hook sends the sid16.
    /// An adopted session may also surface under its claude-side id.
    fn resolve_session(&self, key: &str) -> Option<uuid::Uuid> {
        if let Ok(id) = key.parse::<uuid::Uuid>() {
            return self
                .board
                .sessions
                .iter()
                .find(|s| s.id == id || s.claude_session_id == Some(id))
                .map(|s| s.id);
        }
        self.board.sessions.iter().find(|s| s.sid16() == key).map(|s| s.id)
    }

    /// Fold one debounced machine transition into the session record.
    /// Returns whether anything visible changed (caller persists/broadcasts).
    fn apply_change(
        &mut self,
        id: uuid::Uuid,
        change: &Change,
        detail: Option<String>,
        hook: Option<&str>,
    ) -> bool {
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
        let snapshot = rec.clone();
        self.feed.session_state(&snapshot, &change.from, hook);
        self.auto_move(id, snapshot.ticket, &change.to, change.confidence);
        true
    }

    /// Automove (rules in `core::automove`): a session transition drags its
    /// ticket along the default template. The ticket file is saved here; the
    /// caller already persists/broadcasts for the state change itself.
    fn auto_move(
        &mut self,
        session: uuid::Uuid,
        ticket: ulid::Ulid,
        to: &SessionState,
        confidence: Confidence,
    ) {
        let Some(t) = self.board.ticket(ticket) else { return };
        let Some(dest) = mesimon_core::automove::automove(&t.column, to, confidence) else {
            return;
        };
        if !self.board.columns.iter().any(|c| c.name == dest) {
            return;
        }
        if let Decision::Deny { .. } = authorize(
            &Principal::Agent { session },
            &Action::Mutate,
            &Resource::Ticket { id: ticket },
        ) {
            return;
        }
        let order = fracindex::between(
            &self.board.column_tickets(dest).last().map(|t| t.order.clone()).unwrap_or_default(),
            "",
        );
        let Some(t) = self.board.ticket_mut(ticket) else { return };
        t.column = dest.to_string();
        t.order = order;
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        self.feed.board("agent", "automove", Some(ticket));
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
        let worktrees = self
            .worktrees
            .iter()
            .map(|(tid, b)| WorktreeItem {
                ticket: *tid,
                branch: b.branch.clone(),
                status: match &b.status {
                    BindingStatus::Queued => "queued",
                    BindingStatus::Provisioning => "provisioning",
                    BindingStatus::Attached => "attached",
                    BindingStatus::Evicted => "evicted",
                    BindingStatus::Error { .. } => "error",
                }
                .into(),
                merged: self.wt_merged.get(tid).copied().unwrap_or(false),
                conflict: !b.branch.is_empty() && self.wt_conflicts.contains(&b.branch),
                ahead: self.wt_ahead.get(tid).copied().unwrap_or(0),
                needs_rebase: self.wt_needs_rebase.get(tid).copied().unwrap_or(false),
                detail: match &b.status {
                    BindingStatus::Error { stage, message } => {
                        Some(format!("{stage}: {message}"))
                    }
                    _ => None,
                },
            })
            .collect();
        Response::Board {
            board: self.board.clone(),
            grace,
            external: self.external.clone(),
            resources: self.resources(),
            worktrees,
        }
    }

    /// Header figures (D33e) — real measurements only.
    fn resources(&self) -> Resources {
        Resources {
            live: self.board.sessions.iter().filter(|s| s.state.has_pane()).count(),
            asleep: self
                .board
                .sessions
                .iter()
                .filter(|s| matches!(s.state, SessionState::Sleeping))
                .count(),
            rss_bytes: self.rss_cache.0,
            rss_measured: self.rss_cache.1,
            pty_used: self.pty_cache.used,
            pty_total: self.pty_cache.total,
            pty_budget: self.pty_cache.budget,
            reclaim_bytes: self.reclaim_cache.0,
            reclaim_sessions: self.reclaim_cache.1,
        }
    }

    /// One `ps` fork on the 10 s bucket, over the pane process groups we own.
    /// The same bucket recomputes the sleep suggestion (nothing here forks
    /// more than the `ps` and the one snapshot).
    fn refresh_rss(&mut self) -> bool {
        if !self.board.sessions.iter().any(|s| s.state.has_pane()) {
            let had = self.rss_cache != (0, 0) || self.reclaim_cache != (0, 0);
            self.rss_cache = (0, 0);
            self.rss_by.clear();
            self.reclaim_cache = (0, 0);
            return had;
        }
        let Ok(snap) = self.backend.snapshot() else { return false };
        let ours: std::collections::HashSet<String> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.state.has_pane())
            .map(|s| s.sid16())
            .collect();
        let pgids: std::collections::HashSet<i32> = snap
            .iter()
            .filter(|p| !p.pane_dead && ours.contains(&p.session_name))
            .map(|p| p.pane_pid)
            .collect();
        let by_pgid = crate::resources::measure_rss_by(&pgids);
        self.rss_by = snap
            .iter()
            .filter(|p| !p.pane_dead)
            .filter_map(|p| {
                let bytes = by_pgid.get(&p.pane_pid)?;
                let rec = self.board.sessions.iter().find(|s| s.sid16() == p.session_name)?;
                Some((rec.id, *bytes))
            })
            .collect();
        let fresh = (by_pgid.values().sum::<u64>(), by_pgid.len());
        let reclaim = self.reclaim_figures();
        // Repaint-worthy only when a figure moves visibly (>1 MiB or count).
        let moved = fresh.1 != self.rss_cache.1
            || fresh.0.abs_diff(self.rss_cache.0) > 1024 * 1024
            || reclaim.1 != self.reclaim_cache.1
            || reclaim.0.abs_diff(self.reclaim_cache.0) > 1024 * 1024;
        self.rss_cache = fresh;
        self.reclaim_cache = reclaim;
        moved
    }

    /// The header's sleep suggestion: sessions on sleep-safe tickets that
    /// pass the D23 floors RIGHT NOW (same predicate the sleep keys use — the
    /// suggestion never offers what a keystroke would refuse), plus the RSS
    /// they hold. Column gating is hardcoded until per-column sleep policy
    /// lands with M5's column policies.
    fn reclaim_figures(&self) -> (u64, usize) {
        let safe: std::collections::HashSet<ulid::Ulid> = self
            .board
            .tickets
            .iter()
            .filter(|t| t.column == SLEEP_SAFE_COLUMN)
            .map(|t| t.id)
            .collect();
        let now = now_ms();
        let mut bytes = 0u64;
        let mut n = 0usize;
        for rec in self.board.sessions.iter().filter(|r| safe.contains(&r.ticket)) {
            if self.sleep_eligible(rec, now, true).is_ok() {
                bytes += self.rss_by.get(&rec.id).copied().unwrap_or(0);
                n += 1;
            }
        }
        (bytes, n)
    }

    /// The D33e spawn gate: refuse only at the OS boundary, naming the reason.
    /// The PTY recount happens here — "while a spawn is queued, never
    /// otherwise" (14 §5.1). A failed measurement never blocks a spawn.
    fn spawn_gate(&mut self) -> Option<String> {
        self.pty_cache = crate::resources::pty_figures();
        if self.pty_cache.total > 0 && self.pty_cache.budget == 0 {
            return Some(format!(
                "PTY budget exhausted ({} of {} allocated) — sleep sessions (Z)",
                self.pty_cache.used, self.pty_cache.total
            ));
        }
        if let Some(free) = crate::resources::free_ram_bytes() {
            if free < crate::resources::SPAWN_PEAK_BYTES {
                return Some(format!(
                    "low memory ({} MiB free < 200 MiB spawn peak) — sleep sessions (Z)",
                    free / (1024 * 1024)
                ));
            }
        }
        None
    }

    /// 19 §4 tier 1: transcript census, filtered to this repo (and worktrees),
    /// minus sessions already on the board (ours live in the same tree).
    fn rescan_external(&mut self) {
        let home = crate::census::claude_home();
        let roots = crate::census::repo_roots(&self.paths.repo_root);
        // Only working-set records hide a drawer row — an Exited import must
        // be re-importable, not shadow-banned by its own corpse.
        let known: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.state.is_live())
            .flat_map(|s| [Some(s.id), s.claude_session_id].into_iter().flatten())
            .collect();
        self.external = crate::census::scan(&home, &roots, &|id| known.contains(&id));
    }

    fn broadcast(&mut self) {
        // Keep the focused status line's `!N` live while a session holds focus
        // (attention transitions land here via persist_and_notify).
        self.refresh_status_line();
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
        let id = self.mint_ticket(column, title);
        self.persist_and_notify();
        Response::Created { id }
    }

    /// Append a new ticket to `column` (caller validated the column).
    fn mint_ticket(&mut self, column: String, title: String) -> ulid::Ulid {
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
            workspace: None,
        };
        let id = t.id;
        let _ = store::save_ticket(&self.paths, &t);
        self.board.tickets.push(t);
        id
    }

    fn delete_ticket(&mut self, id: ulid::Ulid, discard_worktree: bool) -> Response {
        let Some(pos) = self.board.tickets.iter().position(|t| t.id == id) else {
            return Response::Err { message: "no such ticket".into() };
        };
        // M4 delete gate (defense in depth — the TUI prompts first): an
        // unmerged worktree must be merged or explicitly discarded.
        if !discard_worktree {
            if let Some(b) = self.worktrees.get(&id) {
                if !b.branch.is_empty() && !self.ticket_merged(id, &b.branch) {
                    return Response::Err {
                        message: "worktree unmerged — merge it first, or delete with discard".into(),
                    };
                }
            }
        }
        let ticket = self.board.tickets.remove(pos);
        // Sessions detach and keep running through the grace band (D21).
        let sessions: Vec<SessionRecord> =
            self.board.sessions.iter().filter(|s| s.ticket == id).cloned().collect();
        self.board.sessions.retain(|s| s.ticket != id);
        let _ = store::delete_ticket_dir(&self.paths, &ticket.short_key);
        self.grace.insert(
            id,
            GraceEntry {
                ticket,
                sessions,
                expires: Instant::now() + Duration::from_secs(GRACE_SECS),
                discard_worktree,
            },
        );
        self.persist_and_notify();
        Response::Ok
    }

    fn ticket_merged(&self, _id: ulid::Ulid, branch: &str) -> bool {
        // Fresh check on gate paths (the 10 s cache may lag a just-made merge).
        let base = self
            .base_branch
            .clone()
            .or_else(|| worktree::default_branch(&self.paths.repo_root).ok());
        base.map(|b| worktree::is_merged(&self.paths.repo_root, branch, &b)).unwrap_or(false)
    }

    fn set_workspace(
        &mut self,
        id: ulid::Ulid,
        workspace: Option<WorkspaceStrategy>,
    ) -> Response {
        if self.board.ticket(id).is_none() {
            return Response::Err { message: "no such ticket".into() };
        }
        // Locked once anything exists that the choice would relocate.
        if self.board.sessions.iter().any(|s| s.ticket == id) {
            return Response::Err { message: "workspace locked — ticket has sessions".into() };
        }
        if self.worktrees.contains_key(&id) {
            return Response::Err { message: "workspace locked — worktree exists".into() };
        }
        match self.with_ticket(id, |t| t.workspace = workspace) {
            Some(r) => r,
            None => Response::Err { message: "no such ticket".into() },
        }
    }

    /// The merge key (M4): preflight in memory; merge only when clean; never
    /// resolve conflicts — that is MergeToAgent's job.
    fn merge_ticket(&mut self, id: ulid::Ulid) -> Response {
        let Some(b) = self.worktrees.get(&id) else {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no worktree on this ticket".into(),
            };
        };
        if b.branch.is_empty() {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "worktree has no branch yet".into(),
            };
        }
        let branch = b.branch.clone();
        // Quiet-tickets rule (author): never merge under a working agent.
        let busy = self.board.sessions.iter().any(|s| {
            s.ticket == id
                && matches!(
                    s.state,
                    SessionState::Spawning
                        | SessionState::Running
                        | SessionState::RequiresAction { .. }
                )
        });
        if busy {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "sessions still working — wait for them to finish".into(),
            };
        }
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        let Some(base) = self.base_branch.clone() else {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no default branch found".into(),
            };
        };
        // A branch whose tip never left the creation base is trivially an
        // ancestor of main — merge_check would call it "already merged".
        // Truth: there is nothing to merge yet (dogfood 2026-08-30).
        let tip = worktree::branch_tip(&self.paths.repo_root, &branch);
        let base_oid = self.worktrees.get(&id).map(|b| b.base_oid.clone()).unwrap_or_default();
        if tip.is_empty() || tip == base_oid {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no commits on the branch yet — nothing to merge".into(),
            };
        }
        if worktree::is_merged(&self.paths.repo_root, &branch, &base) {
            return Response::Merge {
                outcome: MergeOutcome::AlreadyMerged,
                detail: format!("{branch} is already in {base}"),
            };
        }
        // ff-only policy: base moved past the branch → the agent rebases +
        // tests in its worktree first. Mesimon never mints merge commits.
        if !worktree::ff_possible(&self.paths.repo_root, &branch, &base) {
            return Response::Merge {
                outcome: MergeOutcome::NeedsRebase,
                detail: format!("{base} moved — rebase first"),
            };
        }
        match worktree::ff_merge(&self.paths.repo_root, &branch, &base) {
            Ok(()) => {
                self.refresh_worktree_flags();
                self.persist_and_notify();
                Response::Merge {
                    outcome: MergeOutcome::Merged,
                    detail: format!("{branch} merged into {base}"),
                }
            }
            Err(e) => Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: worktree::merge_refusal_detail(&e.to_string(), &base),
            },
        }
    }

    /// The m flow's inject stages: paste a rebase request or the merged
    /// notice into the ticket's live claude session and submit it (T-5
    /// delivery). Explicit user gesture — the user pressed through the
    /// staged prompt.
    fn merge_to_agent(
        &mut self,
        id: ulid::Ulid,
        request: mesimon_core::command::MergeRequest,
    ) -> Response {
        let Some(b) = self.worktrees.get(&id) else {
            return Response::Err { message: "no worktree on this ticket".into() };
        };
        let branch = b.branch.clone();
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        let base = self.base_branch.clone().unwrap_or_else(|| "main".into());
        let Some(rec) = self
            .board
            .sessions
            .iter()
            .find(|s| s.ticket == id && s.kind == SessionKind::Claude && s.state.has_pane())
        else {
            return Response::Err {
                message: "no live claude session on this ticket — open one first".into(),
            };
        };
        let text = match request {
            mesimon_core::command::MergeRequest::Rebase => format!(
                "Rebase your current branch {branch} onto {base}, resolve any conflicts, \
                 then run the tests and fix any failures before we merge."
            ),
            mesimon_core::command::MergeRequest::MergedNotice => format!(
                "Your branch {branch} has been merged into {base}. The main checkout now \
                 contains this work."
            ),
        };
        match self.backend.paste_text(&rec.sid16(), &text) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Err { message: format!("could not deliver: {e}") },
        }
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
                let mut sids = Vec::new();
                for s in &g.sessions {
                    // SIGTERM the group now; the reaper's grace-then-kill-pane
                    // finishes the ladder (docs/19 §1 — never SIGKILL).
                    if s.state.has_pane() {
                        let _ = self.backend.signal_session(&s.sid16());
                        self.reaping.insert(s.sid16(), Instant::now() + REAP_GRACE);
                        sids.push(s.sid16());
                    }
                }
                // M4: worktree teardown waits for the reaper — never remove a
                // directory a live process still has as cwd (12 §12.6.5).
                if self.worktrees.contains_key(&id) {
                    self.pending_teardown.push((id, g.discard_worktree, sids));
                }
            }
        }
        self.broadcast();
    }

    /// Teardown transaction (12 §12.6.1), once every pane of the ticket left
    /// the reaper: unlock → remove --force (single force; NEVER -f -f) →
    /// branch -d if merged, -D only under the user's discard confirmation.
    fn process_teardowns(&mut self) {
        if self.pending_teardown.is_empty() {
            return;
        }
        let ready: Vec<usize> = self
            .pending_teardown
            .iter()
            .enumerate()
            .filter(|(_, (_, _, sids))| !sids.iter().any(|s| self.reaping.contains_key(s)))
            .map(|(i, _)| i)
            .collect();
        if ready.is_empty() {
            return;
        }
        for i in ready.into_iter().rev() {
            let (ticket, discard, _) = self.pending_teardown.remove(i);
            let Some(b) = self.worktrees.remove(&ticket) else { continue };
            let merged = !b.branch.is_empty() && self.ticket_merged(ticket, &b.branch);
            if b.path.is_dir() {
                let _ = worktree::remove(&self.paths.repo_root, &b.path);
            }
            if !b.branch.is_empty() {
                if merged {
                    let _ = worktree::delete_branch(&self.paths.repo_root, &b.branch, false);
                } else if discard {
                    let _ = worktree::delete_branch(&self.paths.repo_root, &b.branch, true);
                }
                // Unmerged without discard: keep the branch (commits survive).
            }
            self.wt_merged.remove(&ticket);
            self.wt_ahead.remove(&ticket);
            self.wt_needs_rebase.remove(&ticket);
        }
        let _ = worktree::save_bindings(&self.paths, &self.worktrees);
        self.refresh_worktree_flags();
        self.broadcast();
    }

    fn move_ticket(&mut self, id: ulid::Ulid, column: String, before: Option<ulid::Ulid>) -> Response {
        if !self.board.columns.iter().any(|c| c.name == column) {
            return Response::Err { message: format!("no such column: {column}") };
        }
        // M4 DONE gate (author rule 3): DONE means the work landed — an
        // unmerged worktree blocks the move. (Configurable later; magic now.)
        if column == "DONE" {
            if let Some(b) = self.worktrees.get(&id) {
                if !b.branch.is_empty() && !self.ticket_merged(id, &b.branch) {
                    return Response::Err {
                        message: "worktree unmerged — merge before DONE".into(),
                    };
                }
            }
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
        if let Some(message) = self.spawn_gate() {
            return Response::Err { message };
        }
        // M4: resolve the ticket's workspace to a cwd BEFORE any side effects.
        // A worktree ticket that is not provisioned yet queues provisioning and
        // parks this spawn; on_provisioned replays it.
        let cwd = match self.resolve_spawn_cwd(ticket) {
            Ok(Some(p)) => p,
            Ok(None) => {
                if !self.pending_spawns.iter().any(|(t, k)| *t == ticket && *k == kind) {
                    self.pending_spawns.push((ticket, kind));
                }
                self.persist_and_notify();
                return Response::Provisioning;
            }
            Err(message) => return Response::Err { message },
        };
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
                    Ok(settings) => {
                        let mut argv = vec![
                            claude,
                            "--settings".into(),
                            settings.display().to_string(),
                            "--session-id".into(),
                            id.to_string(),
                        ];
                        // Replicate the user's own configured permission mode
                        // as an explicit flag (dogfood 2026-08-30: a session in
                        // a fresh worktree lost the global defaultMode; the
                        // flag is the only mode source Claude Code checks
                        // deterministically, and --settings merge semantics
                        // are a documented gap). Pass-through only — mesimon
                        // never picks a mode the user didn't configure.
                        if let Some(mode) = user_default_mode() {
                            argv.push("--permission-mode".into());
                            argv.push(mode);
                        }
                        argv
                    }
                    Err(e) => {
                        return Response::Err { message: format!("hook settings: {e}") };
                    }
                }
            }
            SessionKind::Bash => vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())],
        };
        // Claude enters Spawning; the SessionStart hook flips it to Running.
        // Bash has no hook surface — a live pane is all "running" means (D15).
        let state = match kind {
            SessionKind::Claude => SessionState::Spawning,
            SessionKind::Bash => SessionState::Running,
        };
        let mut rec =
            SessionRecord::new(id, kind, ticket, argv.clone(), cwd.display().to_string(), state);
        rec.state_changed_at = Some(now_ms());
        let env = self.worktree_env(ticket, &cwd);
        if let Err(e) = self.backend.spawn(&rec.sid16(), &cwd, &argv, &env) {
            return Response::Err { message: format!("spawn failed: {e}") };
        }
        // Prefill the ticket title into the agent's input box — typed, never
        // submitted; the user edits and presses Enter (zero token injection).
        // Fresh Claude spawns only: resume/wake replay argv elsewhere and must
        // not retype into a restored conversation, and a Bash pane would put
        // the title on a shell command line.
        if kind == SessionKind::Claude {
            if let Some(title) =
                self.board.ticket(ticket).map(|t| t.title.trim()).filter(|t| !t.is_empty())
            {
                let _ = self.backend.send_text(&rec.sid16(), &format!("{title} "));
            }
        }
        self.machines.insert(id, Machine::new(rec.state.clone(), now_ms()));
        self.board.sessions.push(rec);
        self.lock_worktree(ticket, id);
        self.persist_and_notify();
        Response::Spawned { id }
    }

    /// M4 workspace resolution. `Ok(None)` = provisioning queued/in flight.
    fn resolve_spawn_cwd(
        &mut self,
        ticket: ulid::Ulid,
    ) -> std::result::Result<Option<std::path::PathBuf>, String> {
        let strategy = self
            .board
            .ticket(ticket)
            .map(|t| t.workspace_strategy())
            .unwrap_or(mesimon_core::board::DEFAULT_WORKSPACE);
        match strategy {
            WorkspaceStrategy::SharedCheckout => Ok(Some(self.paths.repo_root.clone())),
            WorkspaceStrategy::AdoptExisting => match self.worktrees.get(&ticket) {
                Some(b) if b.status == BindingStatus::Attached => Ok(Some(b.path.clone())),
                _ => Err("no worktree bound to this ticket — adopt one first".into()),
            },
            WorkspaceStrategy::Worktree => {
                match self.worktrees.get(&ticket).map(|b| b.status.clone()) {
                    Some(BindingStatus::Attached) => {
                        Ok(Some(self.worktrees[&ticket].path.clone()))
                    }
                    Some(BindingStatus::Queued) | Some(BindingStatus::Provisioning) => Ok(None),
                    Some(BindingStatus::Evicted) | Some(BindingStatus::Error { .. }) | None => {
                        self.queue_provision(ticket);
                        Ok(None)
                    }
                }
            }
        }
    }

    /// Mark Queued and start the off-thread provision if a slot is free
    /// (concurrency 2 — `worktree add` is ~1.8 s of filesystem work and must
    /// never run on the writer thread).
    fn queue_provision(&mut self, ticket: ulid::Ulid) {
        let prior = self.worktrees.get(&ticket).cloned();
        let entry = self.worktrees.entry(ticket).or_insert_with(|| Binding {
            path: std::path::PathBuf::new(),
            branch: String::new(),
            base_oid: String::new(),
            branch_oid: String::new(),
            status: BindingStatus::Queued,
            locked: false,
        });
        entry.status = BindingStatus::Queued;
        let in_flight = self
            .worktrees
            .values()
            .filter(|b| b.status == BindingStatus::Provisioning)
            .count();
        if in_flight >= 2 {
            let _ = worktree::save_bindings(&self.paths, &self.worktrees);
            return;
        }
        let Some(t) = self.board.ticket(ticket) else { return };
        let (key, title) = (t.short_key.clone(), t.title.clone());
        if let Some(b) = self.worktrees.get_mut(&ticket) {
            b.status = BindingStatus::Provisioning;
        }
        let _ = worktree::save_bindings(&self.paths, &self.worktrees);
        let repo = self.paths.repo_root.clone();
        let root = match worktree::ensure_root(&self.paths) {
            Ok(r) => r,
            Err(e) => {
                if let Some(b) = self.worktrees.get_mut(&ticket) {
                    b.status =
                        BindingStatus::Error { stage: "root".into(), message: e.to_string() };
                }
                return;
            }
        };
        let tx = self.tx.clone();
        let evicted = prior.filter(|b| b.status == BindingStatus::Evicted && !b.branch.is_empty());
        std::thread::spawn(move || {
            let result = match evicted {
                Some(prior) => worktree::provision_existing(&repo, ticket, &prior),
                None => worktree::provision(&repo, &root, ticket, &key, &title),
            };
            let _ = tx.send(Msg::Provisioned(ticket, result));
        });
    }

    fn on_provisioned(
        &mut self,
        ticket: ulid::Ulid,
        result: std::result::Result<Binding, (String, String)>,
    ) {
        match result {
            Ok(b) => {
                self.worktrees.insert(ticket, b);
                let pending: Vec<(ulid::Ulid, SessionKind)> = self
                    .pending_spawns
                    .iter()
                    .filter(|(t, _)| *t == ticket)
                    .cloned()
                    .collect();
                self.pending_spawns.retain(|(t, _)| *t != ticket);
                for (t, kind) in pending {
                    // A failed replay has no client waiting on it — leave a
                    // feed trace (the TUI's parked focus intent surfaces the
                    // "attached but no session" outcome to the user).
                    if let Response::Err { message } = self.spawn_session(t, kind) {
                        eprintln!("mesimon: parked spawn replay failed ({kind:?}): {message}");
                        self.feed.board("daemon", "spawn_replay_failed", Some(t));
                    }
                }
            }
            Err((stage, message)) => {
                self.pending_spawns.retain(|(t, _)| *t != ticket);
                if let Some(b) = self.worktrees.get_mut(&ticket) {
                    b.status = BindingStatus::Error { stage, message };
                }
            }
        }
        // A slot opened — start the next queued provision, if any.
        if let Some(next) = self
            .worktrees
            .iter()
            .find(|(_, b)| b.status == BindingStatus::Queued)
            .map(|(t, _)| *t)
        {
            self.queue_provision(next);
        }
        let _ = worktree::save_bindings(&self.paths, &self.worktrees);
        self.refresh_worktree_flags();
        self.persist_and_notify();
    }

    /// Take the worktree lock when a session starts in it (12 §12.3.2).
    fn lock_worktree(&mut self, ticket: ulid::Ulid, session: uuid::Uuid) {
        let Some(b) = self.worktrees.get_mut(&ticket) else { return };
        if b.status != BindingStatus::Attached || b.locked {
            return;
        }
        let key = self
            .board
            .ticket(ticket)
            .map(|t| t.short_key.clone())
            .unwrap_or_default();
        if worktree::lock(&self.paths.repo_root, &b.path, &key, session, std::process::id())
            .is_ok()
        {
            b.locked = true;
            let _ = worktree::save_bindings(&self.paths, &self.worktrees);
        }
    }

    /// merged/conflict flags + lazy unlock, on the 10 s bucket while bindings
    /// exist. One `worktree list` + one `merge-base` per binding — read-only,
    /// `--no-optional-locks`.
    fn refresh_worktree_flags(&mut self) {
        if self.worktrees.is_empty() {
            self.wt_merged.clear();
            self.wt_ahead.clear();
            self.wt_needs_rebase.clear();
            self.wt_conflicts.clear();
            return;
        }
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        let Some(base) = self.base_branch.clone() else { return };
        let tickets: Vec<ulid::Ulid> = self.worktrees.keys().copied().collect();
        for tid in tickets {
            let (branch, locked, attached) = {
                let b = &self.worktrees[&tid];
                (b.branch.clone(), b.locked, b.status == BindingStatus::Attached)
            };
            if branch.is_empty() {
                continue;
            }
            // "merged" means WORK landed: everything on the branch is in base
            // AND the tip moved past the creation base. A fresh branch is
            // trivially an ancestor of base — that is "no work yet", never
            // "merged" (dogfood 2026-08-30).
            let tip = worktree::branch_tip(&self.paths.repo_root, &branch);
            let base_oid = self.worktrees[&tid].base_oid.clone();
            let merged = !tip.is_empty()
                && tip != base_oid
                && worktree::is_merged(&self.paths.repo_root, &branch, &base);
            self.wt_merged.insert(tid, merged);
            self.wt_ahead
                .insert(tid, worktree::ahead_count(&self.paths.repo_root, &branch, &base));
            self.wt_needs_rebase.insert(
                tid,
                !merged && !worktree::ff_possible(&self.paths.repo_root, &branch, &base),
            );
            // Release the lock once the last session on the ticket is gone.
            if locked && attached {
                let live = self
                    .board
                    .sessions
                    .iter()
                    .any(|s| s.ticket == tid && s.state.is_live());
                if !live {
                    if let Some(b) = self.worktrees.get_mut(&tid) {
                        if worktree::unlock(&self.paths.repo_root, &b.path).is_ok() {
                            b.locked = false;
                        }
                    }
                }
            }
        }
        self.wt_conflicts = worktree::list_worktrees(&self.paths.repo_root)
            .map(|rows| worktree::branch_conflicts(&rows))
            .unwrap_or_default();
    }

    fn kill_session(&mut self, id: uuid::Uuid) -> Response {
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        let reap = rec.state.has_pane().then(|| rec.sid16());
        // Kill on a live session ends the process; the conversation survives
        // and its corpse stays on the ticket rail. Kill on an already-dead
        // record is the rail's dismissal gesture — the one exit the rail hides.
        let reason =
            if rec.state.is_live() { ExitReason::Killed } else { ExitReason::Dismissed };
        rec.state = SessionState::Exited { reason };
        rec.waiting_since = None;
        rec.detail = None;
        let (id, state) = (rec.id, rec.state.clone());
        self.machines.insert(id, Machine::new(state, now_ms()));
        if let Some(sid) = reap {
            let _ = self.backend.signal_session(&sid);
            self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        }
        self.persist_and_notify();
        Response::Ok
    }

    /// 19 §4 tier 2: mint an observe-only record for a discovered foreign
    /// session — no process, no tmux, no hooks. Tier-0 state comes from the
    /// tail poller; the census preview seeds the card detail.
    fn attach_external(
        &mut self,
        claude_session_id: uuid::Uuid,
        ticket: Option<ulid::Ulid>,
    ) -> std::result::Result<uuid::Uuid, String> {
        if let Some(t) = ticket {
            if self.board.ticket(t).is_none() {
                return Err("no such ticket".into());
            }
        }
        if self.board.sessions.iter().any(|s| {
            s.state.is_live()
                && (s.id == claude_session_id || s.claude_session_id == Some(claude_session_id))
        }) {
            return Err("session already on the board".into());
        }
        // A dead record for this conversation may already sit on the board —
        // reuse it (and its ticket) instead of minting a duplicate. This is
        // the other half of rescan_external's "re-importable, not shadow-
        // banned" rule: the drawer's R lands the session back where it lived.
        if let Some(id) = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                !s.state.is_live()
                    && (s.id == claude_session_id
                        || s.claude_session_id == Some(claude_session_id))
            })
            .max_by_key(|s| s.state_changed_at.unwrap_or(0))
            .map(|s| s.id)
        {
            if let Some(t) = ticket {
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.ticket = t;
                }
            }
            self.external.retain(|e| e.claude_session_id != claude_session_id);
            return Ok(id);
        }
        let Some(pos) =
            self.external.iter().position(|e| e.claude_session_id == claude_session_id)
        else {
            return Err("unknown external session — reopen the drawer to rescan".into());
        };
        let item = self.external.remove(pos);
        // Import gesture: no target ticket means mint one, named after the
        // session (title latch → preview → id), in the first column.
        let ticket = match ticket {
            Some(t) => t,
            None => {
                let Some(column) = self.board.sorted_columns().first().map(|c| c.name.clone())
                else {
                    return Err("board has no columns".into());
                };
                let title = item
                    .name
                    .clone()
                    .or_else(|| item.preview.clone())
                    .unwrap_or_else(|| item.claude_session_id.to_string()[..8].to_string());
                let title: String = title.chars().take(48).collect();
                self.mint_ticket(column, title)
            }
        };
        let id = uuid::Uuid::new_v4();
        let mut rec = SessionRecord::new(
            id,
            SessionKind::Claude,
            ticket,
            vec![], // no process of ours — the discriminator the tail poller keys on
            item.cwd.clone(),
            SessionState::unknown(),
        );
        rec.provenance = Provenance::Adopted;
        rec.claude_session_id = Some(claude_session_id);
        rec.transcript_path = Some(item.transcript_path.clone());
        rec.confidence = Confidence::Low;
        rec.state_changed_at = Some(now_ms());
        rec.detail = item.preview.clone();
        self.machines.insert(id, Machine::new(rec.state.clone(), now_ms()));
        self.board.sessions.push(rec);
        Ok(id)
    }

    /// The one resume builder (wake and takeover share it). D24: the argv
    /// array is the mechanism — resume restores neither `--settings` nor
    /// `--mcp-config` (09 §9), so we replay ours, swapping the identity flag.
    fn resume_argv(&self, rec: &SessionRecord) -> std::result::Result<Vec<String>, String> {
        let target = rec.claude_session_id.unwrap_or(rec.id);
        if !rec.argv.is_empty() {
            // Swap the identity flag to `--resume <target>`. An existing
            // `--resume` operand is rewritten too, never replayed verbatim:
            // an in-app /resume may have moved the pane onto a different
            // conversation since the argv was persisted (dogfood
            // 2026-08-30: a verbatim replay of a stale target crash-looped
            // "No conversation found" forever).
            let mut argv = Vec::with_capacity(rec.argv.len() + 1);
            let mut it = rec.argv.iter();
            while let Some(a) = it.next() {
                if a == "--session-id" || a == "--resume" {
                    let _ = it.next();
                    argv.push("--resume".into());
                    argv.push(target.to_string());
                } else {
                    argv.push(a.clone());
                }
            }
            return Ok(argv);
        }
        // Adopted with no argv of ours: build the full spawn argv fresh —
        // hooks via --settings keyed on OUR record uuid (never --bare, S-D).
        let hook_bin = std::env::var("MESIMON_HOOK_BIN")
            .map(std::path::PathBuf::from)
            .or_else(|_| std::env::current_exe())
            .unwrap_or_else(|_| std::path::PathBuf::from("mesimon"));
        let claude = std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
        let settings = crate::hook_settings::write_settings(&self.paths, rec.id, &hook_bin)
            .map_err(|e| format!("hook settings: {e}"))?;
        let mut argv = vec![
            claude,
            "--settings".into(),
            settings.display().to_string(),
            "--resume".into(),
            target.to_string(),
        ];
        if let Some(mode) = user_default_mode() {
            argv.push("--permission-mode".into());
            argv.push(mode);
        }
        Ok(argv)
    }

    /// Double-resume guard (09 §9: two resumes interleave one transcript).
    fn resume_guard(
        &self,
        rec_id: uuid::Uuid,
        claude_id: uuid::Uuid,
        confirm: bool,
    ) -> Option<String> {
        // Our own board: a second live record for the same claude session is
        // always a refusal — mesimon would be interleaving with itself.
        if self.board.sessions.iter().any(|s| {
            s.id != rec_id
                && s.state.has_pane()
                && !matches!(s.state, SessionState::Unknown { .. })
                && (s.id == claude_id || s.claude_session_id == Some(claude_id))
        }) {
            return Some("already running under mesimon".into());
        }
        if !confirm {
            let home = crate::census::claude_home();
            if let Some(pid) = crate::census::running_pid_for(&home, claude_id) {
                return Some(format!(
                    "running elsewhere (pid {pid}) — resuming would interleave transcripts; resume again to override"
                ));
            }
        }
        None
    }

    /// `claude --resume <id>` reads the transcript from Claude's own
    /// projects dir; a session that ended before its first prompt never
    /// wrote one, and claude exits 1 ("No conversation found") inside a
    /// second — which the pane-died path then records as a crash. Refuse
    /// up front with the honest reason instead.
    fn resume_transcript_missing(&self, rec: &SessionRecord, claude_id: uuid::Uuid) -> bool {
        let name = format!("{claude_id}.jsonl");
        // The record's path only vouches for the TARGET conversation when its
        // filename matches — after an in-app /resume it names a different
        // conversation, and trusting it waved a nonexistent target through
        // (dogfood 2026-08-30).
        if let Some(t) = &rec.transcript_path {
            let p = std::path::Path::new(t);
            if p.file_name().and_then(|f| f.to_str()) == Some(name.as_str()) && p.is_file() {
                return false;
            }
        }
        // transcript_path stale or never learned — the slug dir tracks cwd,
        // so scan every project dir for the session's file before refusing.
        let projects = crate::census::claude_home().join("projects");
        if let Ok(dirs) = std::fs::read_dir(&projects) {
            for d in dirs.flatten() {
                if d.path().join(&name).is_file() {
                    return false;
                }
            }
        }
        true
    }

    /// Takeover / wake: spawn `claude --resume` under this record's sid16.
    fn resume_session(&mut self, id: uuid::Uuid, confirm: bool) -> Response {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        if rec.kind != SessionKind::Claude {
            return Response::Err { message: "only claude sessions resume".into() };
        }
        if rec.state.has_pane() && !matches!(rec.state, SessionState::Unknown { .. }) {
            // Live states keep their pane; resuming over it would double-run.
            if !rec.argv.is_empty() {
                return Response::Err { message: "session is live — focus it instead".into() };
            }
        }
        let claude_id = rec.claude_session_id.unwrap_or(rec.id);
        if let Some(message) = self.resume_guard(id, claude_id, confirm) {
            return Response::Err { message };
        }
        if self.resume_transcript_missing(rec, claude_id) {
            return Response::Err {
                message: "no transcript to resume — the session ended before its first prompt"
                    .into(),
            };
        }
        let argv = match self.resume_argv(rec) {
            Ok(a) => a,
            Err(message) => return Response::Err { message },
        };
        let (sid, cwd, ticket) =
            (rec.sid16(), std::path::PathBuf::from(rec.cwd.clone()), rec.ticket);
        // M4: never silently relocate an agent — a removed worktree/cwd is an
        // explicit refusal, not a fallback into the main checkout.
        if !cwd.is_dir() {
            return Response::Err {
                message: format!("session's directory is gone ({}) — cannot resume", cwd.display()),
            };
        }
        if let Some(message) = self.spawn_gate() {
            return Response::Err { message };
        }
        self.reaping.remove(&sid); // a fresh pane must not meet a stale reap
        let _ = self.backend.kill_session(&sid); // clear any dead remain-on-exit pane
        let env = self.worktree_env(ticket, &cwd);
        if let Err(e) = self.backend.spawn(&sid, &cwd, &argv, &env) {
            return Response::Err { message: format!("resume spawn failed: {e}") };
        }
        let now = now_ms();
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.argv = argv;
            rec.state = SessionState::Spawning;
            rec.state_changed_at = Some(now);
            rec.waiting_since = None;
            rec.confidence = Confidence::High;
        }
        self.tails.remove(&id); // hooks own the state from here
        self.probe_stage.remove(&id);
        self.machines.insert(id, Machine::new(SessionState::Spawning, now));
        Response::Spawned { id }
    }

    /// D23 floors, tmux-recast. `Err` carries the user-facing refusal.
    /// The 60 s age floor guards only the bulk reclaim sweep — a deliberate
    /// keypress on one session is explicit intent and skips it.
    fn sleep_eligible(
        &self,
        rec: &SessionRecord,
        now: u64,
        enforce_floor: bool,
    ) -> std::result::Result<(), String> {
        if rec.pinned_awake {
            return Err("pinned awake".into());
        }
        match (rec.kind, &rec.state) {
            (SessionKind::Claude, SessionState::Idle { .. }) => {}
            (SessionKind::Claude, _) => return Err("only idle sessions sleep".into()),
            // Bash has no hook surface: Running IS its only live state, so the
            // manual path accepts it — guarded by the live-children check.
            (SessionKind::Bash, SessionState::Running) => {}
            (SessionKind::Bash, _) => return Err("no live shell to sleep".into()),
        }
        let age = now.saturating_sub(rec.state_changed_at.unwrap_or(now));
        if enforce_floor && age < sleep_min_age_ms() {
            return Err("too young — never sleep within 60s".into());
        }
        if rec.kind == SessionKind::Bash {
            // TIOCGPGRP recast: we hold no PTY master under tmux, so the
            // guard is the pane process's live children (D23 hard floor).
            let pane_pid = self
                .backend
                .snapshot()
                .ok()
                .and_then(|s| s.into_iter().find(|p| p.session_name == rec.sid16()))
                .map(|p| p.pane_pid);
            if let Some(pid) = pane_pid {
                let kids = live_children(pid);
                if !kids.is_empty() {
                    return Err(format!("bash has live children ({})", kids.join(", ")));
                }
            }
        }
        Ok(())
    }

    /// D23/14 §6.1, tmux-recast: copy transcript, park the record FIRST (the
    /// machine's Sleeping latch swallows the kill's own SessionEnd/pane-died),
    /// SIGTERM the group, kill-pane after grace. Never SIGKILL.
    fn sleep_one(&mut self, id: uuid::Uuid, enforce_floor: bool) -> std::result::Result<(), String> {
        let now = now_ms();
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return Err("no such session".into());
        };
        self.sleep_eligible(rec, now, enforce_floor)?;
        let (sid, kind, transcript) = (rec.sid16(), rec.kind, rec.transcript_path.clone());

        // B-A22's cheap half: a copy with no user+assistant pair means resume
        // would come back amnesiac — sleep anyway, but say so on the card.
        let mut warn = None;
        if kind == SessionKind::Claude {
            let copied = transcript.as_ref().and_then(|t| {
                let dir = self.paths.transcripts_dir();
                std::fs::create_dir_all(&dir).ok()?;
                let dst = dir.join(format!("{id}.jsonl"));
                std::fs::copy(t, &dst).ok()?;
                Some(dst)
            });
            match copied {
                Some(dst) if transcript_has_conversation(&dst) => {}
                _ => warn = Some("resume may lose context".to_string()),
            }
        }

        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.state = SessionState::Sleeping;
            rec.confidence = Confidence::High;
            rec.waiting_since = None;
            rec.state_changed_at = Some(now);
            rec.detail = warn;
        }
        self.machines.insert(id, Machine::new(SessionState::Sleeping, now));
        self.tails.remove(&id);
        self.probe_stage.remove(&id);
        let _ = self.backend.signal_session(&sid);
        self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        Ok(())
    }

    fn wake_session(&mut self, id: uuid::Uuid) -> Response {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        if !matches!(rec.state, SessionState::Sleeping) {
            return Response::Err { message: "not asleep".into() };
        }
        match rec.kind {
            SessionKind::Claude => self.resume_session(id, false),
            SessionKind::Bash => {
                let (sid, argv, cwd, ticket) = (
                    rec.sid16(),
                    rec.argv.clone(),
                    std::path::PathBuf::from(rec.cwd.clone()),
                    rec.ticket,
                );
                if !cwd.is_dir() {
                    return Response::Err {
                        message: format!(
                            "session's directory is gone ({}) — cannot wake",
                            cwd.display()
                        ),
                    };
                }
                if let Some(message) = self.spawn_gate() {
                    return Response::Err { message };
                }
                self.reaping.remove(&sid);
                let _ = self.backend.kill_session(&sid);
                let env = self.worktree_env(ticket, &cwd);
                if let Err(e) = self.backend.spawn(&sid, &cwd, &argv, &env) {
                    return Response::Err { message: format!("wake spawn failed: {e}") };
                }
                let now = now_ms();
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.state = SessionState::Running;
                    rec.state_changed_at = Some(now);
                }
                self.machines.insert(id, Machine::new(SessionState::Running, now));
                Response::Spawned { id }
            }
        }
    }

    /// 04's reclaim: sleep everything eligible, report the honest split.
    fn reclaim_all(&mut self) -> (usize, usize) {
        // The header offer's action: sleep-safe tickets only. Z must sleep
        // exactly the set the suggestion prices, never sessions on tickets
        // still in play (2026-08-30 rescope; per-column policy lands in M5).
        let safe: std::collections::HashSet<ulid::Ulid> = self
            .board
            .tickets
            .iter()
            .filter(|t| t.column == SLEEP_SAFE_COLUMN)
            .map(|t| t.id)
            .collect();
        let candidates: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|r| safe.contains(&r.ticket))
            .filter(|r| {
                matches!(
                    (r.kind, &r.state),
                    (SessionKind::Claude, SessionState::Idle { .. })
                        | (SessionKind::Bash, SessionState::Running)
                )
            })
            .map(|r| r.id)
            .collect();
        let mut slept = 0;
        let mut skipped = 0;
        for id in candidates {
            match self.sleep_one(id, true) {
                Ok(()) => slept += 1,
                Err(_) => skipped += 1,
            }
        }
        (slept, skipped)
    }

    /// The grace half of the kill ladder: SIGTERM already went out; once the
    /// deadline passes, remove the pane (remain-on-exit keeps it visible
    /// meanwhile, and pane-died usually beats us here).
    fn sweep_reaping(&mut self) {
        let now = Instant::now();
        let due: Vec<String> = self
            .reaping
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(s, _)| s.clone())
            .collect();
        for sid in due {
            self.reaping.remove(&sid);
            let _ = self.backend.kill_session(&sid);
        }
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
        if matches!(rec.state, SessionState::Sleeping) {
            return Response::Err { message: "asleep — wake it first".into() };
        }
        if rec.provenance == Provenance::Adopted && rec.argv.is_empty() {
            // Observe-only: no pane, no hooks, no input (19 §4 tier 2).
            return Response::Err { message: "external session — resume it to take over".into() };
        }
        self.focus = Some(session);
        let sid16 = rec.sid16();
        let kind = rec.kind;
        let argv = self.backend.attach_argv(&sid16);
        // Breadcrumb leaf: the session's own name when the agent set one
        // (OSC-0 pane title; tmux reports the hostname when it never did),
        // else the kind word.
        let kind_word = match kind {
            SessionKind::Claude => "claude",
            SessionKind::Bash => "bash",
        };
        self.focus_label = match self.backend.pane_title(&sid16) {
            Ok(t) => {
                let t = t.trim();
                if t.is_empty() || t == self.hostname {
                    kind_word.to_string()
                } else {
                    tmux_text(t, 24)
                }
            }
            Err(_) => kind_word.to_string(),
        };
        self.refresh_status_line();
        Response::Attach { argv }
    }

    /// The focused status line renders the breadcrumb — same component as the
    /// TUI header: ` mesimon > project !N > ticket title `. The needs-you
    /// count uses terminal yellow (the 16-colour attn of 06 §2.7, both
    /// flavors) popped out of the reversed bar; tmux chrome is backend-owned
    /// display, not the wire — the daemon still never styles a wire string.
    fn refresh_status_line(&mut self) {
        let Some(focused) = self.focus else { return };
        let repo = tmux_text(
            &self
                .paths
                .repo_root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            32,
        );
        let title = self
            .board
            .sessions
            .iter()
            .find(|s| s.id == focused)
            .and_then(|s| self.board.ticket(s.ticket))
            .map(|t| tmux_text(&t.title, 48))
            .unwrap_or_default();
        let queue = mesimon_core::attention::attention_queue(&self.board);
        let needs_you = queue.len();
        // The user is looking at this pane: a `!1` that means "the session
        // you're inside" is noise, so the chip only shows when somewhere
        // ELSE needs them too.
        let only_self = needs_you == 1 && queue[0].id == focused;
        let attn = if needs_you > 0 && !only_self {
            // Painted chip, not bare fg: `noreverse` alone drops the segment
            // to the terminal's default background (illegible on light
            // terminals). Graphite's attn pair (06 §2.2) — legibility is
            // internal to the chip, so it needs no flavor detection here;
            // tmux maps the hex down to 256/16 colours itself.
            format!("#[noreverse]#[fg=#131417,bg=#F0A93A,bold] !{needs_you} #[default]")
        } else {
            String::new()
        };
        let line = format!(
            " mesimon > #[bold]{repo}#[nobold]{attn} > {title} > #[bold]{}#[nobold] ",
            self.focus_label
        );
        if self.last_status_left.as_deref() != Some(&line)
            && self.backend.set_status_left(&line).is_ok()
        {
            self.last_status_left = Some(line);
        }
    }

    fn gate_status(&mut self) -> Response {
        if self.paths.gate_file().is_file() {
            return Response::Gate { passed: true, attach_argv: None };
        }
        // Create (idempotently) the gate session the first-run ceremony attaches to (D20).
        let msg = "mesimon first-run check:\\n\\n  This is a live session view.\\n  Press Ctrl+] (or Ctrl+5, on any layout) to return to the board.\\n";
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

/// Text destined for the tmux status line: `#` doubled (tmux format escape),
/// quotes and control characters stripped, hard char cap.
fn tmux_text(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in s.chars().take(max_chars) {
        match ch {
            '#' => out.push_str("##"),
            '"' | '\'' | ';' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Test seam only — e2e cannot wait out the real 15 s server guard.
fn server_guard_ticks() -> u64 {
    std::env::var("MESIMON_SERVER_GUARD_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&t| t > 0)
        .unwrap_or(SERVER_GUARD_TICKS)
}

/// Test seam only — e2e cannot wait out the real 60 s floor.
fn sleep_min_age_ms() -> u64 {
    std::env::var("MESIMON_SLEEP_MIN_AGE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SLEEP_MIN_AGE_MS)
}

/// Test seam only — e2e cannot spend 8 real seconds per quiet verdict.
fn pane_quiet_ms() -> u64 {
    std::env::var("MESIMON_PANE_QUIET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(PANE_QUIET_MS)
}

/// How an `Unknown` session's transcript rested → the hint that seeds its
/// recovered state (Low confidence). Quiet gating uses the file mtime: a
/// trailing assistant record on a long-quiet file is a turn that died, not
/// one in flight.
fn resting_hint(path: &std::path::Path, now: u64) -> Option<TailHint> {
    let quiet = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| now.saturating_sub(d.as_millis() as u64))
        .unwrap_or(u64::MAX);
    match crate::tail::last_event(path)? {
        TailEvent::TurnComplete => Some(TailHint::TurnComplete),
        TailEvent::Aborted => Some(TailHint::AbortedMidStream),
        TailEvent::NeedsHuman { tool: TailTool::AskUserQuestion } => {
            Some(TailHint::AskUserQuestion)
        }
        TailEvent::NeedsHuman { tool: TailTool::ExitPlanMode } => Some(TailHint::ExitPlanMode),
        TailEvent::AssistantText { .. } => {
            if quiet < TAIL_QUIET_MS {
                Some(TailHint::AssistantText)
            } else {
                Some(TailHint::StaleQuiet)
            }
        }
        // A trailing user/attachment record: the turn may be in flight — say
        // nothing while the file is fresh, idle once it has clearly died.
        TailEvent::Other => {
            if quiet >= TAIL_QUIET_MS {
                Some(TailHint::StaleQuiet)
            } else {
                None
            }
        }
        TailEvent::Latch => None,
    }
}

/// Direct live children of a pid, by name — the tmux-recast bash-sleep guard.
fn live_children(pid: i32) -> Vec<String> {
    let Ok(out) = std::process::Command::new("pgrep").args(["-lP", &pid.to_string()]).output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_string))
        .collect()
}

/// B-A22's cheap assertion: the copied transcript holds a real conversation.
fn transcript_has_conversation(path: &std::path::Path) -> bool {
    let Ok(f) = std::fs::File::open(path) else { return false };
    let (mut user, mut assistant) = (false, false);
    for line in BufReader::new(f).lines().map_while(|l| l.ok()) {
        user |= line.contains("\"type\":\"user\"");
        assistant |= line.contains("\"type\":\"assistant\"");
        if user && assistant {
            return true;
        }
    }
    false
}

fn now_iso() -> String {
    // Seconds precision is enough for created_at; avoid a chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}
