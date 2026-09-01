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
use mesimon_backend_tmux::TmuxBackend;
use mesimon_core::adopt::{classify_tail_record, TailEvent, TailTool};
use mesimon_core::attention::{self, Change, Machine, Signal, StartSource, TailHint};
use mesimon_core::board::{
    sanitize_tag, Archived, Board, Confidence, ExitReason, Provenance, SessionKind, SessionRecord,
    SessionState, Ticket, UnknownReason, WorkspaceStrategy,
};
use mesimon_core::command::{
    AgentBoardView, AgentTicketRow, AgentTicketView, Command, Envelope, Event, ExternalItem,
    GraceItem, MergeOutcome, Notice, Resources, Response, WorktreeItem, PROTOCOL_VERSION,
};
use mesimon_core::mcp;
use mesimon_core::reconcile::{reconcile, state_for};
use mesimon_core::{authorize, fracindex, Action, Decision, Principal, Resource};

use crate::feed::FeedWriter;
use crate::hook_settings::mesimon_bin;
use crate::ingest::{self, HookFrame};
use crate::movegate::{MoveGate, Position};
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
/// Gap between presses of an owed, unacknowledged Enter, and how many presses
/// to spend before giving up. T-5 measured the `UserPromptSubmit` ack at
/// ~94 ms, so 500 ms is a wide margin, and 10 attempts covers ~5 s of Claude
/// startup — well past the ~1 s at which a fresh pane starts reading.
const SUBMIT_RETRY_MS: u64 = 500;
const SUBMIT_ATTEMPTS: u8 = 10;
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
/// A sleep-safe ticket whose sessions have all been asleep this long feeds
/// the header's archive suggestion (same offer-not-action shape as sleep).
const ARCHIVE_SUGGEST_MS: u64 = 3_600_000;

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
    /// Standing advisories about persisted state, rebuilt at startup and
    /// carried on every snapshot. Not transient: each one describes a
    /// condition still true on disk.
    notices: Vec<mesimon_core::command::Notice>,
    /// (mtime_ms, len) of our own executable, captured at startup so it
    /// describes the binary actually running — not whatever landed at that
    /// path since. A newer client compares it to decide we are stale.
    exe_stamp: Option<mesimon_core::command::ExeStamp>,
    /// Set only when `spawn_detached` started us. A human's foreground
    /// `mesimon daemon --repo` is never restarted under them.
    detached: bool,
    /// A state file we could not read (or that a newer mesimon wrote) is
    /// still on disk. Writing over it would destroy the only copy, so these
    /// bar the corresponding save. Enforced at the `persist_*` chokepoints.
    columns_barred: bool,
    sessions_barred: bool,
    worktrees_barred: bool,
    /// One attention machine per session, keyed by session UUID.
    machines: HashMap<uuid::Uuid, Machine>,
    /// Startup-modal probe progress per Spawning Claude session:
    /// 1 = the +10s probe ran, 2 = the +30s probe ran (11 §11.5.3 approx).
    probe_stage: HashMap<uuid::Uuid, u8>,
    /// Sessions whose owed Enter has been pressed but not yet acknowledged:
    /// `(next attempt epoch-ms, attempts left)`. Transient, never persisted —
    /// a daemon restart abandons the offer rather than typing into a pane it
    /// no longer understands.
    submit_retry: HashMap<uuid::Uuid, (u64, u8)>,
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
    /// Tickets currently archive-suggestable — recomputed on the 1 s bucket
    /// (NOT the RSS bucket: refresh_rss early-returns when no pane exists,
    /// which is exactly the all-asleep scenario archive looks for).
    archive_cache: usize,
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
    /// The bool is the request's `submit_prompt` — a parked Shift+Enter must
    /// still submit its prompt when the worktree finally lands.
    pending_spawns: Vec<(ulid::Ulid, SessionKind, bool)>,
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
    /// What restrains every mover that is not a person (T-84). See
    /// `crate::movegate` for why authority alone cannot do this job.
    moves: MoveGate,
    /// Bumped on every broadcast. An opaque "something changed" token handed
    /// to agents so a future `if_version` has something to compare, and so a
    /// tool result can be told apart from a stale one.
    board_version: u64,
    /// `(session, idempotency_key) -> resulting column`, for replaying a
    /// mutating tool call the agent believes failed. A mid-call transport drop
    /// hands the model the literal string `Connection closed` AFTER the move
    /// has been persisted; without this the retry moves the card twice.
    agent_replay: HashMap<(uuid::Uuid, String), String>,
}

pub fn run(paths: Paths) -> Result<()> {
    // First statement: the stamp must describe the binary that is executing,
    // not one that replaced it at the same path while we were starting.
    let exe_stamp = crate::exe_stamp();
    paths.ensure_dirs()?;

    // Singleton (02 §4): flock on the lock file; loser exits quietly.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.lock_file())?;
    let rc = unsafe {
        libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB)
    };
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
    // A malformed state file is a NOTICE, not a startup failure: this runs
    // after orch.sock is already bound, so a hard fail here left the client
    // staring at a 5 s blank terminal with the real cause in daemon.log.
    let store::Loaded { mut board, mut notices, columns_write_barred, sessions_write_barred } =
        store::load(&paths)?;

    // Reconcile persisted records against the live private server (D24).
    let snap = backend.snapshot().unwrap_or_default();
    let rec = reconcile(&board.sessions, &snap);
    for (id, link) in &rec.links {
        if let Some(r) = board.sessions.iter_mut().find(|s| s.id == *id) {
            // Observe-only records (imported, never spawned) have no pane by
            // design — Missing is their normal condition, not a crash.
            let observe_only = r.provenance == Provenance::Adopted && r.argv.is_empty();
            if observe_only && matches!(link, mesimon_core::reconcile::Link::Missing) {
                continue;
            }
            r.state = state_for(link, &r.state, r.kind == SessionKind::Claude);
        }
    }
    if !sessions_write_barred {
        store::save_sessions(&paths, &board)?;
    }

    let (tx, rx) = channel::<Msg>();

    // The deadline wheel: grace expiry, settle timers, the server guard.
    let tick_tx = tx.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        if tick_tx.send(Msg::Tick).is_err() {
            break;
        }
    });

    // M4: load worktree bindings; reconcile (a missing dir is Evicted, not an
    // error — diffs still render from the object store); sweep our stale locks.
    // NOT unwrap_or_default(): a parse failure used to yield an empty map,
    // and the next save_bindings wrote it back — orphaning every real
    // worktree and msmn/* branch with nothing to reconstruct from. Recover
    // from git + our ownership markers instead, and bar writes until the
    // recovery verifies.
    let (mut worktrees, wt_notices, worktrees_barred) = worktree::load_or_recover(&paths);
    notices.extend(wt_notices);
    let mut wt_changed = worktree::reconcile_interrupted(&paths.repo_root, &mut worktrees);
    for b in worktrees.values_mut() {
        if b.status == BindingStatus::Attached && !b.path.is_dir() {
            b.status = BindingStatus::Evicted;
            wt_changed = true;
        }
    }
    if wt_changed && !worktrees_barred {
        let _ = worktree::save_bindings(&paths, &worktrees);
    }
    if !worktrees.is_empty() {
        let _ = worktree::sweep_stale_locks(&paths.repo_root);
    }

    // Accept loop: one reader thread per client. Diff commands (M4b) are
    // served right there — read-only, off the writer thread, bounded by the
    // permit pool in DiffCtx.
    let accept_tx = tx.clone();
    let diff_ctx = Arc::new(DiffCtx {
        paths: paths.clone(),
        permits: Arc::new((Mutex::new(DIFF_PERMITS), std::sync::Condvar::new())),
        worktrees_barred: Arc::new(std::sync::atomic::AtomicBool::new(worktrees_barred)),
    });
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = accept_tx.clone();
            let ctx = diff_ctx.clone();
            std::thread::spawn(move || client_loop(stream, tx, ctx));
        }
    });

    // Hook ingest: one-shot SOCK_STREAM frames from `mesimon hook`. The 0600
    // socket is the authentication (11 §11.2.2 — no token in any agent env).
    let hook_path = paths.hook_sock();
    let _ = std::fs::remove_file(&hook_path);
    let hook_listener = UnixListener::bind(&hook_path).context("bind hook.sock")?;
    std::fs::set_permissions(&hook_path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
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
        .map(|s| (s.id, Machine::restore(s.state.clone(), s.confidence, now)))
        .collect();
    let feed = FeedWriter::open(&paths.activity_log())?;

    let mut d = Daemon {
        paths,
        board,
        backend,
        grace: HashMap::new(),
        subscribers: Vec::new(),
        focus: None,
        shutting_down: false,
        notices,
        exe_stamp,
        detached: std::env::var_os("MESIMON_DETACHED").is_some(),
        columns_barred: columns_write_barred,
        sessions_barred: sessions_write_barred,
        worktrees_barred,
        machines,
        probe_stage: HashMap::new(),
        submit_retry: HashMap::new(),
        ticks: 0,
        feed,
        external: Vec::new(),
        tails: HashMap::new(),
        reaping: HashMap::new(),
        rss_cache: (0, 0),
        rss_by: HashMap::new(),
        reclaim_cache: (0, 0),
        archive_cache: 0,
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
        moves: MoveGate::new(),
        board_version: 0,
        agent_replay: HashMap::new(),
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
    /// Per-session env (layer 1 of "the session knows where it is": silent,
    /// zero-token, keyed off by hooks and shell scripts; the agent sees it
    /// when it looks).
    ///
    /// `MESIMON_TICKET` goes to EVERY session mesimon spawns. It used to be
    /// worktree-only, which meant a shared-checkout session — the board
    /// default — had no way to name its own ticket from the shell at all
    /// (T-84). `MESIMON_WORKTREE_BRANCH` stays conditional, because a session
    /// in the main checkout genuinely has no branch of its own.
    ///
    /// Layer 2 is the MCP tool surface: this tells the *shell* which ticket it
    /// is on, `get_ticket` tells the *model*.
    fn session_env(&self, ticket: ulid::Ulid, cwd: &std::path::Path) -> Vec<(String, String)> {
        let Some(key) = self.board.ticket(ticket).map(|t| t.short_key.clone()) else {
            return Vec::new();
        };
        let mut env = vec![("MESIMON_TICKET".to_string(), key)];
        if let Some(b) = self.worktrees.get(&ticket) {
            if b.status == BindingStatus::Attached && b.path == cwd {
                env.push(("MESIMON_WORKTREE_BRANCH".into(), b.branch.clone()));
            }
        }
        env
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

/// How many git-backed diff requests run at once, across all connections.
const DIFF_PERMITS: usize = 2;

/// Ceiling on a `PaneTail` answer. A pane is at most its own height, so this
/// is a bound on a malformed request, not a display choice — the client asks
/// for what its zone can hold.
const MAX_PANE_TAIL_LINES: u16 = 200;
/// And on how much of each line rides the wire: a pane can hold a single line
/// thousands of columns wide (`capture-pane -J` joins wrapped ones).
const MAX_PANE_TAIL_COLS: usize = 1000;

/// Everything the off-writer diff service needs, shared across connections.
struct DiffCtx {
    paths: Paths,
    permits: Arc<(Mutex<usize>, std::sync::Condvar)>,
    /// Set when startup could not read `worktrees.json`. Without it this
    /// thread reads an empty map and tells the user a real worktree ticket
    /// has no worktree — a lie. It never quarantines: only the writer thread
    /// renames, so the two can never race.
    worktrees_barred: Arc<std::sync::atomic::AtomicBool>,
}

/// RAII permit from the bounded diff pool.
struct PermitGuard<'a>(&'a (Mutex<usize>, std::sync::Condvar));

impl<'a> PermitGuard<'a> {
    fn acquire(pool: &'a (Mutex<usize>, std::sync::Condvar)) -> Self {
        let (lock, cv) = pool;
        let mut n = lock.lock().unwrap_or_else(|p| p.into_inner());
        while *n == 0 {
            n = cv.wait(n).unwrap_or_else(|p| p.into_inner());
        }
        *n -= 1;
        PermitGuard(pool)
    }
}

impl Drop for PermitGuard<'_> {
    fn drop(&mut self) {
        let (lock, cv) = self.0;
        if let Ok(mut n) = lock.lock() {
            *n += 1;
        } else {
            return;
        }
        cv.notify_one();
    }
}

/// DiffList/DiffFile, served on the connection thread (M4b): read-only, no
/// board access, no BoardChanged — the writer thread never sees them.
fn serve_diff(ctx: &DiffCtx, env: &Envelope) -> Response {
    let ticket = match &env.command {
        Command::DiffList { ticket } | Command::DiffFile { ticket, .. } => *ticket,
        _ => return Response::Err { message: "not a diff command".into() },
    };
    // D32c invariant 2 holds on this path too — the short-circuit must not
    // bypass the chokepoint.
    if let Decision::Deny { reason } =
        authorize(&env.principal, &Action::Read, &Resource::Ticket { id: ticket })
    {
        return Response::Err { message: format!("denied: {reason}") };
    }
    let _permit = PermitGuard::acquire(&ctx.permits);
    let bindings = match worktree::load_bindings(&ctx.paths) {
        Ok(b) => b,
        Err(e) => return Response::Err { message: format!("read bindings: {e}") },
    };
    let Some(binding) = bindings.get(&ticket) else {
        // Distinguish "this ticket has none" from "we cannot read the file":
        // after a quarantine the file is gone and load_bindings answers with
        // an empty map, which would otherwise read as the former.
        if ctx.worktrees_barred.load(std::sync::atomic::Ordering::Relaxed) {
            return Response::Err {
                message: "worktree bindings are unavailable — see the board notice".into(),
            };
        }
        return Response::Err {
            message: "no worktree on this ticket — review is per-branch".into(),
        };
    };
    match &binding.status {
        BindingStatus::Queued | BindingStatus::Provisioning => {
            return Response::Err {
                message: "worktree is still provisioning — try again in a moment".into(),
            };
        }
        BindingStatus::Error { stage, .. } if binding.branch.is_empty() => {
            return Response::Err {
                message: format!("worktree failed at {stage} — no branch to diff"),
            };
        }
        _ => {}
    }
    let repo = &ctx.paths.repo_root;
    match &env.command {
        Command::DiffList { .. } => crate::diff::diff_list(repo, binding)
            .unwrap_or_else(|e| Response::Err { message: e.to_string() }),
        Command::DiffFile { path, context, .. } => {
            match crate::diff::diff_file(repo, binding, path, *context) {
                Ok(file) => Response::DiffFile { file },
                Err(e) => Response::Err { message: e.to_string() },
            }
        }
        _ => unreachable!(),
    }
}

fn client_loop(stream: UnixStream, tx: Sender<Msg>, diff_ctx: Arc<DiffCtx>) {
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
            Ok(env) => match env.command {
                // Read-only diff service: answered here, never forwarded —
                // a slow git must not stall the single writer (M4b).
                Command::DiffList { .. } | Command::DiffFile { .. } => serve_diff(&diff_ctx, &env),
                _ => {
                    let (rtx, rrx) = channel();
                    if tx.send(Msg::Request(env, rtx, writer.clone())).is_err() {
                        break;
                    }
                    rrx.recv().unwrap_or(Response::Err { message: "daemon gone".into() })
                }
            },
            Err(e) => Response::Err { message: format!("bad envelope: {e}") },
        };
        // Serialize BEFORE taking the writer lock: this same Arc sits in
        // `Daemon::subscribers`, and the writer thread's broadcast() blocks
        // on it — a large response must hold it only for the write itself.
        let Ok(json) = serde_json::to_string(&resp) else { break };
        let mut w = match writer.lock() {
            Ok(w) => w,
            Err(_) => break,
        };
        if writeln!(w, "{json}").is_err() {
            break;
        }
    }
}

impl Daemon {
    fn handle(&mut self, env: Envelope, stream: &Arc<Mutex<UnixStream>>) -> Response {
        // The agent tier is a separate path, deliberately (T-84). Sharing the
        // local dispatch would mean every arm below carries an implicit "and
        // is this an agent?" that somebody eventually forgets. Here the answer
        // is settled once, before any board state is touched.
        if let Principal::Agent { session } = env.principal {
            return self.handle_agent(session, env.command);
        }
        // `Automation` is the daemon's own rules acting on their own; it is
        // constructed internally and never arrives over a socket. A client
        // claiming to be one is confused, and answering it would hand a
        // caller the full local command set under a name that reads as
        // "mesimon did this" in the activity feed.
        if let Principal::Automation { .. } = env.principal {
            return Response::Err {
                message: "automation is not a principal a client may claim".into(),
            };
        }
        // D32c invariant 2: the chokepoint is on every path, even though v0.1 allows.
        let action = match &env.command {
            Command::Hello { .. }
            | Command::Snapshot
            | Command::Subscribe
            | Command::GateStatus
            // Mutates only the daemon's discovery cache, never board state.
            | Command::RescanExternal
            | Command::DiffList { .. }
            | Command::DiffFile { .. }
            | Command::PaneTail { .. } => Action::Read,
            _ => Action::Mutate,
        };
        // Reading a pane IS reading the session, and the chokepoint should
        // say so: `authorize` denies an agent `Resource::Session` outright,
        // and naming the resource here is what makes that rule reachable
        // rather than merely true.
        let resource = match &env.command {
            Command::PaneTail { session, .. } => Resource::Session { id: *session },
            _ => Resource::Board,
        };
        if let Decision::Deny { reason } = authorize(&env.principal, &action, &resource) {
            return Response::Err { message: format!("denied: {reason}") };
        }

        let feed_cmd: Option<(&'static str, Option<ulid::Ulid>)> = match &env.command {
            Command::CreateTicket { .. } => Some(("create_ticket", None)),
            Command::RenameTicket { id, .. } => Some(("rename_ticket", Some(*id))),
            Command::DeleteTicket { id, .. } => Some(("delete_ticket", Some(*id))),
            Command::SetWorkspace { id, .. } => Some(("set_workspace", Some(*id))),
            Command::SetTag { id, .. } => Some(("set_tag", Some(*id))),
            Command::ForgetTag { .. } => Some(("forget_tag", None)),
            Command::RegisterTag { .. } => Some(("register_tag", None)),
            Command::RenameTag { .. } => Some(("rename_tag", None)),
            Command::SetTagColor { .. } => Some(("set_tag_color", None)),
            Command::MergeTicket { id } => Some(("merge_ticket", Some(*id))),
            Command::MergeToAgent { id, .. } => Some(("merge_to_agent", Some(*id))),
            Command::RestoreTicket { id } => Some(("restore_ticket", Some(*id))),
            Command::ArchiveTicket { id } => Some(("archive_ticket", Some(*id))),
            Command::UnarchiveTicket { id } => Some(("unarchive_ticket", Some(*id))),
            Command::ArchiveAll => Some(("archive_all", None)),
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
                        message: format!(
                            "protocol {version} unsupported; daemon speaks {PROTOCOL_VERSION}"
                        ),
                    };
                }
                Response::Hello {
                    version: PROTOCOL_VERSION,
                    daemon_pid: std::process::id(),
                    // MESIMON_FAKE_BUILD lets a stale daemon be manufactured
                    // without shipping two binaries (test seam).
                    build: std::env::var("MESIMON_FAKE_BUILD")
                        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string()),
                    exe_stamp: self.exe_stamp,
                    detached: self.detached,
                }
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
            // A barred columns.toml means next_key cannot be persisted, so a
            // new ticket's short_key would regress on the next start and
            // save_ticket would write over an existing ticket directory.
            Command::CreateTicket { .. } if self.columns_barred => {
                Response::Err { message: self.barred_message("columns") }
            }
            Command::CreateTicket { column, title } => self.create_ticket(column, title),
            Command::RenameTicket { id, title } => self
                .with_ticket(id, |t| t.title = title)
                .unwrap_or(Response::Err { message: "no such ticket".into() }),
            Command::DeleteTicket { id, discard_worktree } => {
                self.delete_ticket(id, discard_worktree)
            }
            // Worktree work is refused wholesale while the bindings file is
            // barred: acting would either strand a new worktree we cannot
            // record, or tear down a real one on a guess (D26, fail closed).
            Command::SetWorkspace { .. }
            | Command::MergeTicket { .. }
            | Command::MergeToAgent { .. }
                if self.worktrees_barred =>
            {
                Response::Err { message: self.barred_message("worktrees") }
            }
            Command::SetWorkspace { id, workspace } => self.set_workspace(id, workspace),
            Command::SetTag { id, group, name } => self.set_tag(id, group, name),
            Command::ForgetTag { group, name } => self.forget_tag(group, name),
            Command::RegisterTag { group, name } => self.register_tag(group, name),
            Command::RenameTag { group, from, to } => self.rename_tag(group, from, to),
            Command::SetTagColor { group, name, color } => self.set_tag_color(group, name, color),
            Command::MergeTicket { id } => self.merge_ticket(id),
            Command::MergeToAgent { id, request } => self.merge_to_agent(id, request),
            Command::RestoreTicket { id } => self.restore_ticket(id),
            Command::ArchiveTicket { id } => self.archive_ticket(id),
            Command::UnarchiveTicket { id } => self.unarchive_ticket(id),
            Command::ArchiveAll => {
                let (archived, skipped) = self.archive_all();
                if archived > 0 {
                    self.persist_and_notify();
                }
                Response::Archived { archived, skipped }
            }
            Command::MoveTicket { id, column, before } => self.move_ticket(id, column, before),
            Command::SpawnSession { ticket, .. }
                if self.worktrees_barred && self.ticket_wants_worktree(ticket) =>
            {
                Response::Err { message: self.barred_message("worktrees") }
            }
            Command::SpawnSession { ticket, kind, submit_prompt } => {
                self.spawn_session(ticket, kind, submit_prompt)
            }
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
            // Never reaches the writer — client_loop short-circuits these to
            // serve_diff on the connection thread (M4b). Defensive arm only.
            Command::DiffList { .. } | Command::DiffFile { .. } => Response::Err {
                message: "diff commands are served on the connection thread".into(),
            },
            Command::PaneTail { session, lines } => self.pane_tail(session, lines),
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
            // The agent tier, reached only via `handle_agent`. A local client
            // sending one of these is either confused or probing; either way
            // the answer is no, not "acts as the agent whose id you guessed".
            Command::AgentGetTicket | Command::AgentListBoard | Command::AgentMoveTicket { .. } => {
                Response::Err { message: "agent commands require an agent principal".into() }
            }
        };
        if let Some((cmd, ticket)) = feed_cmd {
            match &resp {
                Response::Ok | Response::Spawned { .. } | Response::Provisioning => {
                    self.feed.board("local", cmd, ticket)
                }
                Response::Created { id } => self.feed.board("local", cmd, ticket.or(Some(*id))),
                Response::Merge {
                    outcome: MergeOutcome::Merged | MergeOutcome::AlreadyMerged,
                    ..
                } => self.feed.board("local", cmd, ticket),
                _ => {}
            }
        }
        resp
    }

    /// One wheel tick (250 ms): grace expiry at the old 1 s cadence, settle
    /// timers, the wholesale-server guard.
    /// How long to wait for the `UserPromptSubmit` ack before pressing Enter
    /// again, and how many presses to spend before giving up and leaving the
    /// title typed. T-5 measured the ack at ~94 ms, so 500 ms is a wide
    /// margin; 10 attempts covers ~5 s of Claude startup.
    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks % 4 == 0 {
            self.expire_grace();
            self.sweep_reaping();
            self.process_teardowns();
        }
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> =
            self.machines.iter_mut().filter_map(|(id, m)| m.tick(now).map(|c| (*id, c))).collect();
        let mut changed = false;
        for (id, change) in fired {
            changed |= self.apply_change(id, &change, None, None);
        }
        if self.ticks % 4 == 0 {
            changed |= self.probe_spawning();
            changed |= self.probe_activity();
            let a = self.archive_figures();
            if a != self.archive_cache {
                self.archive_cache = a;
                changed = true;
            }
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
        changed |= self.retry_pending_submits(now);
        if changed {
            self.persist_sessions();
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
            let bytes =
                self.backend.capture_tail(&sid, 3).map(|lines| !lines.is_empty()).unwrap_or(false);
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

    /// The ticket page's preview zone: what a shell pane has on screen,
    /// oldest line first. Read-only — no state moves, nothing is broadcast,
    /// and the record is only consulted for the pane's name.
    ///
    /// This rides the writer thread, unlike the diff service. That exception
    /// exists for git, which can spend seconds in a packfile; `capture-pane`
    /// is one small fork the tick already makes twice a second, and paying a
    /// second `DiffCtx` to move it off would buy nothing.
    fn pane_tail(&self, session: uuid::Uuid, lines: u16) -> Response {
        let Some(rec) = self.board.sessions.iter().find(|r| r.id == session) else {
            return Response::Err { message: "no such session".into() };
        };
        if !rec.state.has_pane() {
            return Response::Err { message: "session has no pane".into() };
        }
        let n = lines.clamp(1, MAX_PANE_TAIL_LINES) as usize;
        match self.backend.capture_tail(&rec.sid16(), n) {
            // A pane holds whatever a command decided to print, so bound what
            // rides the wire here; what is *drawable* stays the client's own
            // question, the same way transcript text is.
            Ok(v) => Response::PaneTail {
                lines: v
                    .into_iter()
                    .map(|l| l.chars().take(MAX_PANE_TAIL_COLS).collect())
                    .collect(),
            },
            Err(e) => Response::Err { message: e.to_string() },
        }
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
            let cursor =
                self.tails.entry(id).or_insert_with(|| TailCursor::at_end(path.clone(), now));
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
                    TailEvent::AssistantText { text } => {
                        hints.push((TailHint::AssistantText, Some(crate::census::sanitize(&text))))
                    }
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
                if let Some(change) = self.machines.get_mut(&id).and_then(|m| m.apply(&sig, now)) {
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
                self.machines
                    .insert(rec.id, Machine::restore(rec.state.clone(), Confidence::Stale, now));
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
        // D32c invariant 2: the hook-driven path passes the chokepoint too.
        //
        // The principal is `Automation`, not `Agent`. A hook frame is the
        // daemon observing a session, not an agent asking for anything — and
        // since T-84 that difference is load-bearing: `Agent` may never read
        // or change a session at any tier, so ingesting under it would refuse
        // every frame mesimon exists to receive.
        let ingest_by = Principal::Automation { rule: "hook".into() };
        if let Decision::Deny { .. } =
            authorize(&ingest_by, &Action::Mutate, &Resource::Session { id })
        {
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
                dirty |=
                    self.apply_change(id, &change, ingest::detail_of(&frame), Some(&frame.event));
            }
            // Harvest done (status came in the frame) — remove the dead pane
            // remain-on-exit was holding (docs/19 §1 lifecycle).
            if matches!(sig, Signal::PaneDied { .. }) {
                if let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) {
                    let _ = self.backend.kill_session(&rec.sid16());
                }
            }
            // The composer's Shift+Enter, second half: the pane is provably
            // alive and reading, so press the Enter its prefilled title has
            // been waiting for. `Startup` only — a Resume/Clear/Compact
            // SessionStart lands in a conversation that already has the
            // prompt, and an Enter there would submit an empty turn.
            if matches!(sig, Signal::SessionStart { source: StartSource::Startup }) {
                dirty |= self.deliver_pending_submit(id);
            }
            // ...and its ack. Any prompt reaching Claude closes the offer,
            // including one the user typed themselves — either way there is
            // nothing left to press Enter for.
            if matches!(sig, Signal::UserPromptSubmit) {
                dirty |= self.ack_pending_submit(id);
            }
        }
        if dirty {
            self.persist_sessions();
            self.broadcast();
        }
    }

    /// Press Enter on a session whose prefilled title is still sitting
    /// unsubmitted, and keep pressing until Claude says it took.
    ///
    /// `SessionStart` is the earliest moment the pane MIGHT be reading
    /// keystrokes, but it is not proof that it is: Claude fires that hook
    /// during startup, so the frame can reach the daemon milliseconds before
    /// Claude's input loop exists (dogfood 2026-08-31 — the press landed 5 ms
    /// after the frame and was lost; the same sequence with a 750 ms gap
    /// submits). One press on that edge is therefore a race, and this is the
    /// side that must not lose it.
    ///
    /// So the edge only STARTS the delivery. `UserPromptSubmit` is the ack —
    /// T-5 named it the "prompt accepted" signal and measured it at ~94 ms —
    /// and until it arrives `retry_pending_submits` presses again. An Enter
    /// into an already-submitted (hence empty) box is a no-op, so a redundant
    /// press costs nothing; a lost one costs the whole feature.
    fn deliver_pending_submit(&mut self, id: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        if !rec.pending_submit || self.submit_retry.contains_key(&id) {
            return false;
        }
        let sid16 = rec.sid16();
        let _ = self.backend.send_enter(&sid16);
        self.submit_retry.insert(id, (now_ms() + SUBMIT_RETRY_MS, SUBMIT_ATTEMPTS));
        false
    }

    /// The unacknowledged half of the above: press Enter again, on cadence,
    /// until `UserPromptSubmit` clears the flag or the attempts run out.
    /// Giving up leaves the title typed in the box — which is exactly what an
    /// ordinary spawn leaves behind, so the worst case is the old behaviour.
    fn retry_pending_submits(&mut self, now: u64) -> bool {
        if self.submit_retry.is_empty() {
            return false;
        }
        let due: Vec<uuid::Uuid> =
            self.submit_retry.iter().filter(|(_, (at, _))| *at <= now).map(|(id, _)| *id).collect();
        let mut changed = false;
        for id in due {
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
                self.submit_retry.remove(&id);
                continue;
            };
            // A pane that died, or one showing a startup modal, is not a pane
            // to keep pressing Enter into — the modal's Enter is an ANSWER,
            // and mesimon does not answer dialogs on the user's behalf.
            let pressable = rec.pending_submit
                && rec.state.has_pane()
                && matches!(
                    rec.state,
                    SessionState::Spawning | SessionState::Idle { .. } | SessionState::Running
                );
            if !pressable {
                self.submit_retry.remove(&id);
                if rec.pending_submit {
                    let ticket = rec.ticket;
                    self.clear_pending_submit(id);
                    self.feed.board("daemon", "prompt_submit_abandoned", Some(ticket));
                    changed = true;
                }
                continue;
            }
            let sid16 = rec.sid16();
            let ticket = rec.ticket;
            let (_, left) = self.submit_retry[&id];
            let _ = self.backend.send_enter(&sid16);
            if left <= 1 {
                self.submit_retry.remove(&id);
                self.clear_pending_submit(id);
                self.feed.board("daemon", "prompt_submit_gave_up", Some(ticket));
                changed = true;
            } else {
                self.submit_retry.insert(id, (now + SUBMIT_RETRY_MS, left - 1));
            }
        }
        changed
    }

    /// Claude acknowledged a prompt: the owed Enter is paid (by us or by the
    /// user typing their own), so stop pressing.
    fn ack_pending_submit(&mut self, id: uuid::Uuid) -> bool {
        self.submit_retry.remove(&id);
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        if !rec.pending_submit {
            return false;
        }
        let ticket = rec.ticket;
        self.clear_pending_submit(id);
        self.feed.board("user", "prompt_submitted", Some(ticket));
        true
    }

    fn clear_pending_submit(&mut self, id: uuid::Uuid) {
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = false;
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
        rec.waiting_since = if attention::is_attention(&change.to) { Some(now) } else { None };
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
        self.auto_move(snapshot.ticket, &change.to, change.confidence);
        true
    }

    /// Automove (rules in `core::automove`): a session transition drags its
    /// ticket along the default template.
    ///
    /// The principal is `Automation`, NOT `Agent` — that distinction is the
    /// whole of T-84's collision design. Before MCP existed, `Agent` was the
    /// only principal that meant "not the human", so automove borrowed it.
    /// Now that an agent can ask for a move itself, the board has to be able
    /// to tell the two apart: to restrict one without breaking the other, to
    /// say in the feed who moved a card, and to refuse a move that would undo
    /// one the other just made.
    fn auto_move(&mut self, ticket: ulid::Ulid, to: &SessionState, confidence: Confidence) {
        let Some(t) = self.board.ticket(ticket) else { return };
        let Some(dest) = mesimon_core::automove::automove(&t.column, to, confidence) else {
            return;
        };
        let by = Principal::Automation { rule: "automove".into() };
        // Every refusal path (archived, missing column, ping-pong, fuse) lives
        // in place_ticket, and a refused automove is silent by design: the
        // board simply does not move, and the feed carries the reason.
        let _ = self.place_ticket(ticket, dest, Position::Top, &by, "automove");
    }

    /// Everything an agent session may ask the daemon for (T-84).
    ///
    /// The layers run in this order, and the order matters: the command
    /// allowlist is checked before the session is even looked up, so probing
    /// for a valid session id tells a caller nothing it did not already know.
    ///
    /// * **L2 — the command allowlist.** `mcp::agent_allows` is an exhaustive
    ///   match with no wildcard arm, so a command added to the wire protocol
    ///   will not compile until someone decides whether an agent may send it.
    /// * **L1 — the binding.** The ticket comes from the session record, never
    ///   from the request. No agent command carries a ticket id, so there is
    ///   no ownership check to get wrong and no id to guess.
    /// * **L3 — the tier.** `authorize()`, with the precise resource.
    ///
    /// The shim that speaks MCP runs inside the agent's own process tree and
    /// is therefore untrusted. It holds none of this.
    fn handle_agent(&mut self, session: uuid::Uuid, cmd: Command) -> Response {
        if !mcp::agent_allows(&cmd) {
            return Response::Err { message: "not available to an agent session".into() };
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "unknown session".into() };
        };
        // Observe-only adopted records have no argv of ours and were never
        // launched with the tool config; a request claiming to be one is not
        // something mesimon started.
        if rec.provenance != Provenance::Spawned {
            return Response::Err { message: "not a session mesimon spawned".into() };
        }
        if !rec.state.is_live() {
            return Response::Err { message: "session has exited".into() };
        }
        let ticket = rec.ticket;

        match cmd {
            Command::AgentGetTicket => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Read, &Resource::Ticket { id: ticket })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                match self.agent_ticket_view(ticket) {
                    Some(view) => Response::AgentTicket { ticket: view },
                    None => Response::Err { message: "no such ticket".into() },
                }
            }
            Command::AgentListBoard => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } = authorize(&by, &Action::Read, &Resource::Board) {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                Response::AgentBoard { board: self.agent_board_view() }
            }
            Command::AgentMoveTicket { to_column, idempotency_key } => {
                // Replay before acting. A mid-call transport drop hands the
                // model the literal string `Connection closed` AFTER the move
                // has been persisted, so the honest answer to a repeat is the
                // first answer — not a second move.
                if let Some(key) = &idempotency_key {
                    if let Some(column) = self.agent_replay.get(&(session, key.clone())) {
                        return Response::AgentMoved {
                            column: column.clone(),
                            board_version: self.board_version,
                            replayed: true,
                        };
                    }
                }
                let by = Principal::Agent { session };
                match self.place_ticket(ticket, &to_column, Position::Top, &by, "agent_move") {
                    Ok(column) => {
                        if let Some(key) = idempotency_key {
                            self.remember_agent_result(session, key, &column);
                        }
                        // `place_ticket` already saved the ticket file and
                        // broadcast. A move touches no session and no column,
                        // so there is nothing else to persist.
                        Response::AgentMoved {
                            column,
                            board_version: self.board_version,
                            replayed: false,
                        }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            // Unreachable: `agent_allows` above admits exactly three commands.
            _ => Response::Err { message: "not available to an agent session".into() },
        }
    }

    /// Remember a mutating tool call's result so a retry replays it.
    ///
    /// Bounded rather than pruned per session: the map is a safety net for a
    /// dropped connection, not a log, and an unbounded one on a daemon that
    /// runs for weeks is a slow leak nobody would ever look for.
    fn remember_agent_result(&mut self, session: uuid::Uuid, key: String, column: &str) {
        const MAX_REPLAY_ENTRIES: usize = 512;
        if self.agent_replay.len() >= MAX_REPLAY_ENTRIES {
            self.agent_replay.clear();
        }
        self.agent_replay.insert((session, key), column.to_string());
    }

    /// Where `move_ticket` would actually accept a move to, right now.
    ///
    /// This is why `to_column` needs no schema `enum`: the valid set travels
    /// as transient result data instead of becoming permanent model context.
    /// It excludes the current column (a move to where it already is is not a
    /// move) and any column whose gate would refuse — so a ticket with an
    /// unmerged worktree does not advertise DONE and then refuse it.
    fn agent_allowed_columns(&self, id: ulid::Ulid) -> Vec<String> {
        let Some(t) = self.board.ticket(id) else { return Vec::new() };
        let unmerged = self
            .worktrees
            .get(&id)
            .is_some_and(|b| !b.branch.is_empty() && !self.ticket_merged(id, &b.branch));
        self.board
            .sorted_columns()
            .into_iter()
            .map(|c| c.name.clone())
            .filter(|name| name != &t.column)
            .filter(|name| !(unmerged && name == "DONE"))
            .collect()
    }

    /// The ticket's merge state as a word — the same four the `m` flow derives
    /// from git, and a word rather than an enum for the same reason
    /// `WorktreeItem.status` is one.
    fn merge_state_word(&self, id: ulid::Ulid) -> Option<&'static str> {
        let b = self.worktrees.get(&id)?;
        if b.branch.is_empty() {
            return None;
        }
        if self.wt_merged.get(&id).copied().unwrap_or(false) {
            return Some("merged");
        }
        if self.wt_needs_rebase.get(&id).copied().unwrap_or(false) {
            return Some("needs_rebase");
        }
        if self.wt_ahead.get(&id).copied().unwrap_or(0) > 0 {
            return Some("ahead");
        }
        Some("clean")
    }

    fn agent_ticket_view(&self, id: ulid::Ulid) -> Option<AgentTicketView> {
        let t = self.board.ticket(id)?;
        let workspace = match t.workspace_strategy() {
            WorkspaceStrategy::Worktree => "worktree",
            WorkspaceStrategy::SharedCheckout => "shared_checkout",
            WorkspaceStrategy::AdoptExisting => "adopt_existing",
        };
        Some(AgentTicketView {
            key: t.short_key.clone(),
            title: t.title.clone(),
            column: t.column.clone(),
            workspace: workspace.to_string(),
            branch: self.worktrees.get(&id).map(|b| b.branch.clone()).filter(|b| !b.is_empty()),
            merge_state: self.merge_state_word(id).map(str::to_string),
            allowed_columns: self.agent_allowed_columns(id),
            board_version: self.board_version,
        })
    }

    /// The board as an agent sees it: columns, and tickets' key/title/column.
    ///
    /// A hand-written projection rather than `Snapshot` with fields removed —
    /// the difference is that this one cannot grow a session field by accident
    /// when the board model does. Archived tickets are excluded because
    /// `column_tickets` is the archived-exclusion chokepoint and an archived
    /// ticket is off the board.
    fn agent_board_view(&self) -> AgentBoardView {
        let columns: Vec<String> =
            self.board.sorted_columns().into_iter().map(|c| c.name.clone()).collect();
        let tickets = columns
            .iter()
            .flat_map(|name| {
                self.board.column_tickets(name).into_iter().map(|t| AgentTicketRow {
                    key: t.short_key.clone(),
                    title: t.title.clone(),
                    column: t.column.clone(),
                })
            })
            .collect();
        AgentBoardView { columns, tickets, board_version: self.board_version }
    }

    /// The one function that moves a ticket between columns.
    ///
    /// Three callers today — the human's `MoveTicket`, `automove`, and the
    /// agent's `move_ticket` — and a fourth when M5 grows column on-enter
    /// actions. They were three separate implementations before T-84, which
    /// meant the DONE gate bound only one of them and nothing could see that
    /// two movers were undoing each other. Everything a move must obey now
    /// lives here, so a new mover obeys it by construction rather than by
    /// somebody remembering.
    fn place_ticket(
        &mut self,
        id: ulid::Ulid,
        dest: &str,
        pos: Position,
        by: &Principal,
        rule: &str,
    ) -> std::result::Result<String, String> {
        let Some(t) = self.board.ticket(id) else { return Err("no such ticket".into()) };
        if t.is_archived() {
            return Err("ticket archived — restore it first".into());
        }
        let from = t.column.clone();
        if !self.board.columns.iter().any(|c| c.name == dest) {
            return Err(format!("no such column: {dest}"));
        }
        // A move to where it already is is a no-op, not an event: it must not
        // reach the feed, the ping-pong guard or the flap fuse.
        if from == dest {
            return Ok(from);
        }
        // M4 DONE gate (author rule 3): DONE means the work landed — an
        // unmerged worktree blocks the move. It lived inside the human's
        // move path until T-84, which would have let an automation route
        // around the one rule that keeps the board from claiming something
        // shipped when git says it did not.
        if dest == "DONE" {
            if let Some(b) = self.worktrees.get(&id) {
                if !b.branch.is_empty() && !self.ticket_merged(id, &b.branch) {
                    return Err("worktree unmerged — merge before DONE".into());
                }
            }
        }
        if let Decision::Deny { reason } = authorize(by, &Action::Mutate, &Resource::Ticket { id })
        {
            return Err(format!("denied: {reason}"));
        }
        if let Decision::Deny { reason } =
            authorize(by, &Action::Mutate, &Resource::Column { name: dest.to_string() })
        {
            return Err(format!("denied: {reason}"));
        }
        if let Err(refusal) = self.moves.check(id, &from, dest, by, Instant::now()) {
            self.feed.board(by.actor(), &format!("move_refused:{}", refusal.tag()), Some(id));
            return Err(refusal.message());
        }

        let order = self.order_within(dest, id, &pos);
        let automatic = !by.is_human();
        // Bracket the mutation so an automation firing from inside it (M5's
        // on-enter actions) sees depth > 0 and is refused. Nothing recurses
        // today; that is exactly why this is cheap to put in now.
        if automatic {
            self.moves.enter();
        }
        if let Some(t) = self.board.ticket_mut(id) {
            t.column = dest.to_string();
            t.order = order;
            let t = t.clone();
            let _ = store::save_ticket(&self.paths, &t);
        }
        self.moves.record(id, &from, dest, by, Instant::now());
        self.feed.board(by.actor(), rule, Some(id));
        self.broadcast();
        if automatic {
            self.moves.leave();
        }
        Ok(dest.to_string())
    }

    /// The fractional index a ticket takes in its destination column.
    fn order_within(&self, dest: &str, id: ulid::Ulid, pos: &Position) -> String {
        let siblings = self.board.column_tickets(dest);
        let siblings: Vec<&Ticket> = siblings.into_iter().filter(|t| t.id != id).collect();
        let append = || {
            fracindex::between(&siblings.last().map(|t| t.order.clone()).unwrap_or_default(), "")
        };
        match pos {
            // Automatic moves land at the TOP: the move is fresh news (just
            // started, just finished), so it outranks what was already there.
            Position::Top => fracindex::between(
                "",
                &siblings.first().map(|t| t.order.clone()).unwrap_or_default(),
            ),
            Position::Before(None) => append(),
            Position::Before(Some(b)) => match siblings.iter().position(|t| t.id == *b) {
                Some(i) => {
                    let hi = siblings[i].order.clone();
                    let lo = if i == 0 { String::new() } else { siblings[i - 1].order.clone() };
                    fracindex::between(&lo, &hi)
                }
                None => append(),
            },
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
                    BindingStatus::Error { stage, message } => Some(format!("{stage}: {message}")),
                    _ => None,
                },
                path: (b.status == BindingStatus::Attached).then(|| b.path.display().to_string()),
            })
            .collect();
        // Standing notices, plus any ticket whose flap fuse is currently
        // blown. The fuse is a real change in how the board behaves — cards
        // stop moving themselves — so it is said out loud rather than left
        // for the user to notice as an absence.
        let mut notices = self.notices.clone();
        let mut fused: Vec<String> = self
            .moves
            .fused_tickets()
            .filter_map(|id| self.board.ticket(*id))
            .map(|t| t.short_key.clone())
            .collect();
        if !fused.is_empty() {
            fused.sort();
            notices.push(Notice::new(
                "automation_suspended",
                format!(
                    "automatic moves suspended for {} — moved too often, too fast. \
                     Moving one by hand clears it.",
                    fused.join(", ")
                ),
            ));
        }
        Response::Board {
            board: self.board.clone(),
            grace,
            external: self.external.clone(),
            resources: self.resources(),
            worktrees,
            notices,
        }
    }

    /// The one sentence every barred refusal says. Names the file that needs
    /// a human and the command that explains it — never a raw serde error.
    fn barred_message(&self, which: &str) -> String {
        let path = match which {
            "columns" => self.paths.board_dir.join("board/columns.toml"),
            _ => worktree::bindings_file(&self.paths),
        };
        format!(
            "{} could not be read and is being preserved — run `mesimon doctor` \
             for the fix; nothing was changed",
            path.display()
        )
    }

    /// Does this ticket resolve to a worktree workspace? A shared-checkout
    /// spawn touches no bindings and stays allowed while worktrees are barred.
    fn ticket_wants_worktree(&self, id: ulid::Ulid) -> bool {
        self.board
            .ticket(id)
            .map(|t| t.workspace_strategy() == mesimon_core::board::WorkspaceStrategy::Worktree)
            .unwrap_or(false)
    }

    /// The single write path for `columns.toml`. Barred means a file we
    /// could not read — or one a newer mesimon wrote — is still sitting
    /// there, and writing would destroy the only copy.
    fn persist_columns(&self) {
        if self.columns_barred {
            return;
        }
        let _ = store::save_columns(&self.paths, &self.board);
    }

    fn persist_sessions(&self) {
        if self.sessions_barred {
            return;
        }
        let _ = store::save_sessions(&self.paths, &self.board);
    }

    /// The single write path for `worktrees.json`. This one guards real work:
    /// overwriting an unreadable bindings file orphans live git worktrees and
    /// `msmn/*` branches, and nothing can reconstruct them afterwards.
    fn persist_worktrees(&self) {
        if self.worktrees_barred {
            return;
        }
        let _ = worktree::save_bindings(&self.paths, &self.worktrees);
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
            archive_tickets: self.archive_cache,
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
        let ours: std::collections::HashSet<String> =
            self.board.sessions.iter().filter(|s| s.state.has_pane()).map(|s| s.sid16()).collect();
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

    /// The header's archive suggestion: sleep-safe tickets that hold no pane
    /// (the exact predicate the A key gates on — the suggestion never offers
    /// what the keystroke would refuse) and are untouched past the hour:
    /// sleeping sessions all asleep that long, or — with no live sessions at
    /// all (none, or exited corpses only) — the newest of created_at and any
    /// corpse's last change that old. Pure board scan, no forks — cheap
    /// enough for the 1 s bucket, which it must use: the RSS bucket's
    /// no-pane early-return fires precisely when archive candidates exist.
    fn archive_figures(&self) -> usize {
        self.archive_candidates().len()
    }

    /// The offer's exact candidate set — ArchiveAll takes THIS, nothing
    /// broader (the Z/ReclaimAll rule).
    fn archive_candidates(&self) -> Vec<ulid::Ulid> {
        let now = now_ms();
        let threshold = archive_suggest_ms();
        self.board
            .tickets
            .iter()
            .filter(|t| !t.is_archived() && t.column == SLEEP_SAFE_COLUMN)
            .filter(|t| {
                if self.board.ticket_awake_sessions(t.id) > 0 {
                    return false;
                }
                let sessions: Vec<_> =
                    self.board.sessions.iter().filter(|s| s.ticket == t.id).collect();
                let sleeping: Vec<_> =
                    sessions.iter().filter(|s| matches!(s.state, SessionState::Sleeping)).collect();
                if sleeping.is_empty() {
                    sessions
                        .iter()
                        .filter_map(|s| s.state_changed_at)
                        .chain(created_at_ms(&t.created_at))
                        .max()
                        .is_some_and(|at| now.saturating_sub(at) >= threshold)
                } else {
                    sleeping.iter().all(|s| {
                        s.state_changed_at.is_some_and(|at| now.saturating_sub(at) >= threshold)
                    })
                }
            })
            .map(|t| t.id)
            .collect()
    }

    /// X: archive every ticket the offer prices. Per-ticket gate re-checked
    /// (a session can wake between pricing and the keypress); one broadcast.
    fn archive_all(&mut self) -> (usize, usize) {
        let ids = self.archive_candidates();
        let at = now_iso();
        let mut archived = 0;
        let mut skipped = 0;
        for id in ids {
            if self.board.ticket_awake_sessions(id) > 0 {
                skipped += 1;
                continue;
            }
            if let Some(t) = self.board.ticket_mut(id) {
                t.archived = Some(Archived { at: at.clone(), by: "local".into() });
                let t = t.clone();
                let _ = store::save_ticket(&self.paths, &t);
                archived += 1;
            }
        }
        self.archive_cache = self.archive_figures();
        (archived, skipped)
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
        self.board_version = self.board_version.wrapping_add(1);
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
        self.persist_columns();
        self.persist_sessions();
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
        let last =
            self.board.column_tickets(&column).last().map(|t| t.order.clone()).unwrap_or_default();
        let t = Ticket {
            id: ulid::Ulid::new(),
            short_key: format!("T-{}", self.board.next_key),
            title,
            column,
            order: fracindex::between(&last, ""),
            created_at: now_iso(),
            workspace: None,
            tags: Vec::new(),
            archived: None,
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
                        message: "worktree unmerged — merge it first, or delete with discard"
                            .into(),
                    };
                }
            }
        }
        let ticket = self.board.tickets.remove(pos);
        self.moves.forget(id);
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

    fn set_workspace(&mut self, id: ulid::Ulid, workspace: Option<WorkspaceStrategy>) -> Response {
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

    /// Set or clear the ticket's tag on one axis. Sanitization happens HERE,
    /// at the boundary, not in the TUI: a tag name is user text that lands on
    /// a card row, and a width hazard there strands cells the diff never
    /// repaints. Not gated on any write bar — `save_ticket` is the deliberate
    /// exception (a ticket file we could not read is absent from
    /// `board.tickets` and so self-bars).
    fn set_tag(&mut self, id: ulid::Ulid, group: u8, name: Option<String>) -> Response {
        if self.board.ticket(id).is_none() {
            return Response::Err { message: "no such ticket".into() };
        }
        if !(1..=10).contains(&group) {
            return Response::Err { message: "tag group must be 1-10".into() };
        }
        let clean = match name {
            None => None,
            Some(raw) => match sanitize_tag(&raw) {
                Some(c) => Some(c),
                None => return Response::Err { message: "empty tag name".into() },
            },
        };
        // Using a name is what puts it in the vocabulary — that is the whole
        // of "create on the fly". The registry is board-level and persisted,
        // so it outlives the tickets: clearing this ticket's tag below never
        // retires the name, and the cycle keeps its shape.
        if let Some(name) = clean.as_deref() {
            if self.board.register_tag(group, name).is_ok() {
                self.persist_columns();
            }
        }
        match self.with_ticket(id, |t| t.set_tag(group, clean)) {
            Some(r) => r,
            None => Response::Err { message: "no such ticket".into() },
        }
    }

    /// Put a name in the registry. Creating and wearing are separate gestures
    /// in the picker, so this touches no ticket.
    fn register_tag(&mut self, group: u8, name: String) -> Response {
        if !(1..=10).contains(&group) {
            return Response::Err { message: "tag group must be 1-10".into() };
        }
        let Some(clean) = sanitize_tag(&name) else {
            return Response::Err { message: "empty tag name".into() };
        };
        match self.board.register_tag(group, &clean) {
            Ok(()) => {
                self.persist_columns();
                self.broadcast();
                Response::Ok
            }
            Err(message) => Response::Err { message },
        }
    }

    fn rename_tag(&mut self, group: u8, from: String, to: String) -> Response {
        let Some(clean) = sanitize_tag(&to) else {
            return Response::Err { message: "empty tag name".into() };
        };
        match self.board.rename_tag(group, &from, &clean) {
            Ok(touched) => {
                let files: Vec<Ticket> =
                    touched.iter().filter_map(|id| self.board.ticket(*id).cloned()).collect();
                for t in &files {
                    let _ = store::save_ticket(&self.paths, t);
                }
                self.persist_columns();
                self.broadcast();
                Response::Ok
            }
            Err(message) => Response::Err { message },
        }
    }

    fn set_tag_color(&mut self, group: u8, name: String, color: u8) -> Response {
        match self.board.set_tag_color(group, &name, color) {
            Ok(()) => {
                self.persist_columns();
                self.broadcast();
                Response::Ok
            }
            Err(message) => Response::Err { message },
        }
    }

    /// Retire a name from the registry and strip it from every ticket wearing
    /// it. The two must move together: a ticket left wearing a retired tag
    /// shows a pip the cycle can neither reach nor clear.
    fn forget_tag(&mut self, group: u8, name: String) -> Response {
        if !(1..=10).contains(&group) {
            return Response::Err { message: "tag group must be 1-10".into() };
        }
        // Note who wears it BEFORE the removal: only those files changed, and
        // rewriting every ticket on the board to retire one tag would be a
        // write amplification the board does not need.
        let wearers: Vec<ulid::Ulid> = self
            .board
            .tickets
            .iter()
            .filter(|t| t.tag_in(group).is_some_and(|tag| tag.name == name))
            .map(|t| t.id)
            .collect();
        if !self.board.forget_tag(group, &name) {
            return Response::Err { message: format!("no tag {name:?} in group {group}") };
        }
        let touched: Vec<Ticket> =
            wearers.iter().filter_map(|id| self.board.ticket(*id).cloned()).collect();
        for t in &touched {
            let _ = store::save_ticket(&self.paths, t);
        }
        self.persist_columns();
        self.broadcast();
        Response::Ok
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

    /// Archive: off the board, everything kept (ticket file, sleeping
    /// sessions, worktree binding + branch). Gated on the ticket holding no
    /// pane — archive means everything is already asleep.
    fn archive_ticket(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            None => return Response::Err { message: "no such ticket".into() },
            Some(t) if t.is_archived() => {
                return Response::Err { message: "already archived".into() }
            }
            Some(_) => {}
        }
        if self.board.ticket_awake_sessions(id) > 0 {
            return Response::Err { message: "sessions still awake — sleep them first".into() };
        }
        let at = now_iso();
        let resp = self
            .with_ticket(id, |t| t.archived = Some(Archived { at, by: "local".into() }))
            .unwrap_or(Response::Err { message: "no such ticket".into() });
        // Re-price now — a taken offer must not linger until the next bucket.
        self.archive_cache = self.archive_figures();
        resp
    }

    /// Restore lands in the column the ticket was archived from — `column`
    /// and `order` survived archival untouched.
    fn unarchive_ticket(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            None => return Response::Err { message: "no such ticket".into() },
            Some(t) if !t.is_archived() => return Response::Err { message: "not archived".into() },
            Some(_) => {}
        }
        // Guard the landing column (fixed template today; policies in M5).
        let fallback = {
            let t = self.board.ticket(id).expect("checked above");
            if self.board.columns.iter().any(|c| c.name == t.column) {
                None
            } else {
                self.board.sorted_columns().first().map(|c| c.name.clone()).map(|col| {
                    let tail = self
                        .board
                        .column_tickets(&col)
                        .last()
                        .map(|t| t.order.clone())
                        .unwrap_or_default();
                    (col, fracindex::between(&tail, ""))
                })
            }
        };
        // A ticket coming back from the archive starts clean: whatever the
        // move gate remembered about it — a reversal to refuse, a blown fuse —
        // describes a board state from before it left, and applying it to the
        // ticket's first move back would be a refusal nobody could explain.
        self.moves.forget(id);
        self.with_ticket(id, |t| {
            t.archived = None;
            if let Some((col, order)) = fallback {
                t.column = col;
                t.order = order;
            }
        })
        .unwrap_or(Response::Err { message: "no such ticket".into() })
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
        if self.worktrees_barred {
            // Leave the tree and the branch standing. Removing either while
            // the bindings file is unreadable would destroy real work on a
            // guess — the one irreversible move here (D26).
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
        self.persist_worktrees();
        self.refresh_worktree_flags();
        self.broadcast();
    }

    fn move_ticket(
        &mut self,
        id: ulid::Ulid,
        column: String,
        before: Option<ulid::Ulid>,
    ) -> Response {
        match self.place_ticket(
            id,
            &column,
            Position::Before(before),
            &Principal::Local,
            "move_ticket",
        ) {
            Ok(_) => Response::Ok,
            Err(message) => Response::Err { message },
        }
    }

    fn spawn_session(
        &mut self,
        ticket: ulid::Ulid,
        kind: SessionKind,
        submit_prompt: bool,
    ) -> Response {
        if self.board.ticket(ticket).is_none() {
            return Response::Err { message: "no such ticket".into() };
        }
        // An archived ticket must not grow a live pane no board surface shows.
        if self.board.ticket(ticket).is_some_and(|t| t.is_archived()) {
            return Response::Err { message: "ticket archived — restore it first".into() };
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
                if !self.pending_spawns.iter().any(|(t, k, _)| *t == ticket && *k == kind) {
                    self.pending_spawns.push((ticket, kind, submit_prompt));
                }
                self.persist_and_notify();
                return Response::Provisioning;
            }
            Err(message) => return Response::Err { message },
        };
        let id = uuid::Uuid::new_v4();
        let argv: Vec<String> = match kind {
            SessionKind::Claude => match self.claude_argv(id, "--session-id", &id.to_string()) {
                Ok(argv) => argv,
                Err(message) => return Response::Err { message },
            },
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
        let env = self.session_env(ticket, &cwd);
        if let Err(e) = self.backend.spawn(&rec.sid16(), &cwd, &argv, &env) {
            return Response::Err { message: format!("spawn failed: {e}") };
        }
        // Prefill the ticket title into the agent's input box — typed, never
        // submitted; the user edits and presses Enter (zero token injection).
        // Fresh Claude spawns only: resume/wake replay argv elsewhere and must
        // not retype into a restored conversation, and a Bash pane would put
        // the title on a shell command line.
        //
        // `submit_prompt` (the composer's Shift+Enter) does not change WHAT is
        // delivered — the same typed title — only whether mesimon also presses
        // Enter on the user's behalf. That press cannot happen here: T-5 arm C
        // (2026-08-31) showed Claude's paste detection eats a CR that arrives
        // with the text. It is parked on the record and delivered on the
        // `SessionStart` frame instead (`deliver_pending_submit`).
        //
        // The title rides argv nowhere: `claude <title>` would dispatch a
        // title that happens to name a subcommand ("doctor", "update") to that
        // subcommand instead, silently, and `--` does not shield it (measured
        // 2026-08-31). Keystrokes have no such vocabulary.
        if kind == SessionKind::Claude {
            if let Some(title) =
                self.board.ticket(ticket).map(|t| t.title.trim()).filter(|t| !t.is_empty())
            {
                if self.backend.send_text(&rec.sid16(), &format!("{title} ")).is_ok() {
                    rec.pending_submit = submit_prompt;
                }
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
                    Some(BindingStatus::Attached) => Ok(Some(self.worktrees[&ticket].path.clone())),
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
        let in_flight =
            self.worktrees.values().filter(|b| b.status == BindingStatus::Provisioning).count();
        if in_flight >= 2 {
            self.persist_worktrees();
            return;
        }
        let Some(t) = self.board.ticket(ticket) else { return };
        let (key, title) = (t.short_key.clone(), t.title.clone());
        if let Some(b) = self.worktrees.get_mut(&ticket) {
            b.status = BindingStatus::Provisioning;
        }
        self.persist_worktrees();
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
                let pending: Vec<(ulid::Ulid, SessionKind, bool)> =
                    self.pending_spawns.iter().filter(|(t, _, _)| *t == ticket).cloned().collect();
                self.pending_spawns.retain(|(t, _, _)| *t != ticket);
                for (t, kind, submit) in pending {
                    // A failed replay has no client waiting on it — leave a
                    // feed trace (the TUI's parked focus intent surfaces the
                    // "attached but no session" outcome to the user).
                    if let Response::Err { message } = self.spawn_session(t, kind, submit) {
                        eprintln!("mesimon: parked spawn replay failed ({kind:?}): {message}");
                        self.feed.board("daemon", "spawn_replay_failed", Some(t));
                    }
                }
            }
            Err((stage, message)) => {
                self.pending_spawns.retain(|(t, _, _)| *t != ticket);
                if let Some(b) = self.worktrees.get_mut(&ticket) {
                    b.status = BindingStatus::Error { stage, message };
                }
            }
        }
        // A slot opened — start the next queued provision, if any.
        if let Some(next) =
            self.worktrees.iter().find(|(_, b)| b.status == BindingStatus::Queued).map(|(t, _)| *t)
        {
            self.queue_provision(next);
        }
        self.persist_worktrees();
        self.refresh_worktree_flags();
        self.persist_and_notify();
    }

    /// Take the worktree lock when a session starts in it (12 §12.3.2).
    fn lock_worktree(&mut self, ticket: ulid::Ulid, session: uuid::Uuid) {
        let Some(b) = self.worktrees.get_mut(&ticket) else { return };
        if b.status != BindingStatus::Attached || b.locked {
            return;
        }
        let key = self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        if worktree::lock(&self.paths.repo_root, &b.path, &key, session, std::process::id()).is_ok()
        {
            b.locked = true;
            self.persist_worktrees();
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
            self.wt_ahead.insert(tid, worktree::ahead_count(&self.paths.repo_root, &branch, &base));
            self.wt_needs_rebase.insert(
                tid,
                !merged && !worktree::ff_possible(&self.paths.repo_root, &branch, &base),
            );
            // Release the lock once the last session on the ticket is gone.
            if locked && attached {
                let live = self.board.sessions.iter().any(|s| s.ticket == tid && s.state.is_live());
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
        let reason = if rec.state.is_live() { ExitReason::Killed } else { ExitReason::Dismissed };
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
                    && (s.id == claude_session_id || s.claude_session_id == Some(claude_session_id))
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
        let Some(pos) = self.external.iter().position(|e| e.claude_session_id == claude_session_id)
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
        self.machines.insert(id, Machine::restore(rec.state.clone(), Confidence::Low, now_ms()));
        self.board.sessions.push(rec);
        Ok(id)
    }

    /// The MCP config this session is launched with. See
    /// `hook_settings::mcp_config_json` for why it looks the way it does.
    fn mcp_config_json(&self, session: uuid::Uuid) -> String {
        crate::hook_settings::mcp_config_json(&self.paths, &mesimon_bin(), session)
    }

    /// The one argv builder for a Claude session. Fresh spawns pass
    /// `--session-id`; the adopted-resume fallback passes `--resume`.
    ///
    /// This existed twice before T-84 — once in `spawn_session`, once in
    /// `resume_argv` — which meant a flag added to one was silently missing
    /// for adopted and taken-over sessions.
    fn claude_argv(
        &self,
        id: uuid::Uuid,
        identity_flag: &str,
        identity_value: &str,
    ) -> std::result::Result<Vec<String>, String> {
        // Per-session observer hooks via --settings (11 §11.2.1).
        // Never --bare / --safe-mode — both silently clear them (S-D).
        let settings = crate::hook_settings::write_settings(&self.paths, id, &mesimon_bin())
            .map_err(|e| format!("hook settings: {e}"))?;
        let claude = std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into());
        let mut argv = vec![
            claude,
            "--settings".into(),
            settings.display().to_string(),
            "--mcp-config".into(),
            self.mcp_config_json(id),
            identity_flag.to_string(),
            identity_value.to_string(),
        ];
        // Replicate the user's own configured permission mode as an explicit
        // flag (dogfood 2026-08-30: a session in a fresh worktree lost the
        // global defaultMode; the flag is the only mode source Claude Code
        // checks deterministically, and --settings merge semantics are a
        // documented gap). Pass-through only — mesimon never picks a mode the
        // user didn't configure.
        if let Some(mode) = user_default_mode() {
            argv.push("--permission-mode".into());
            argv.push(mode);
        }
        Ok(argv)
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
                } else if a == "--mcp-config" {
                    // Regenerate rather than replay. The blob names the
                    // mesimon binary by absolute path, and a persisted argv
                    // outlives an install — `U` reloads onto a new binary and
                    // a replayed blob would point the shim at the old one.
                    let _ = it.next();
                    argv.push("--mcp-config".into());
                    argv.push(self.mcp_config_json(rec.id));
                } else {
                    argv.push(a.clone());
                }
            }
            return Ok(argv);
        }
        // Adopted with no argv of ours: build the full spawn argv fresh —
        // hooks via --settings keyed on OUR record uuid (never --bare, S-D).
        self.claude_argv(rec.id, "--resume", &target.to_string())
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
        if self.board.ticket(rec.ticket).is_some_and(|t| t.is_archived()) {
            return Response::Err { message: "ticket archived — restore it first".into() };
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
        let env = self.session_env(ticket, &cwd);
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
            // The new pane carries no prefill (resume restores the
            // conversation, and the prompt is already in it), so an Enter
            // owed by the old one is stale — never carry it across.
            rec.pending_submit = false;
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
    fn sleep_one(
        &mut self,
        id: uuid::Uuid,
        enforce_floor: bool,
    ) -> std::result::Result<(), String> {
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
        if self.board.ticket(rec.ticket).is_some_and(|t| t.is_archived()) {
            return Response::Err { message: "ticket archived — restore it first".into() };
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
                let env = self.session_env(ticket, &cwd);
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
        let due: Vec<String> =
            self.reaping.iter().filter(|(_, t)| **t <= now).map(|(s, _)| s.clone()).collect();
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

/// Test seam only — e2e cannot wait out the real hour.
fn archive_suggest_ms() -> u64 {
    std::env::var("MESIMON_ARCHIVE_SUGGEST_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ARCHIVE_SUGGEST_MS)
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

/// `created_at`'s `@<unix secs>` stamp as epoch ms; None for anything else
/// (an unparsable stamp never feeds the archive suggestion).
fn created_at_ms(created_at: &str) -> Option<u64> {
    created_at.strip_prefix('@').and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000)
}

fn now_iso() -> String {
    // Seconds precision is enough for created_at; avoid a chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}
