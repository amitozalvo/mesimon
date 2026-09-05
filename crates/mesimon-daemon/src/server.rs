//! The daemon core: one writer thread owns board state (D22); client threads
//! forward typed envelopes to it and write back responses. Every mutation calls
//! `authorize()` (D32c) even though v0.1 always allows.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use mesimon_backend_tmux::TmuxBackend;
use mesimon_core::adopt::{classify_tail_record, SessionsPidFile, TailEvent, TailTool};
use mesimon_core::attention::{self, Change, Machine, Signal, StartSource, TailHint};
use mesimon_core::board::{
    sanitize_tag, Archived, Board, Confidence, ExitReason, Provenance, SessionKind, SessionRecord,
    SessionState, Tag, TagRef, Ticket, UnknownReason, WorkspaceStrategy,
};
use mesimon_core::command::{
    AgentBoardView, AgentTagView, AgentTicketRow, AgentTicketView, Command, DiffTarget, Envelope,
    Event, ExternalItem, GraceItem, MergeOutcome, Notice, Resources, Response, WorktreeItem,
    PROTOCOL_VERSION,
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
/// flight — the recordless Esc-interrupt catch (spike S-E: an interrupt fires
/// no hook, and one landing before the first assistant output writes nothing
/// to the transcript either; the pane byte stream is the only evidence left).
///
/// This is the FALLBACK, and the number is sized for a fallback. The 8 s it
/// started at rested on "a turn in flight repaints sub-second (spinner)",
/// which was true when measured and is not now: on Claude Code 2.1.257 a
/// working pane holds `#{window_activity}` still for 6–10 s as a matter of
/// course and for up to ~50 s while the model streams a large tool input (an
/// `Edit`/`Write` payload — nothing paints until the call is whole). Under
/// 8 s the probe fired four times in ninety seconds on one ticket, and each
/// verdict blanked the card until the next `PostToolUse` put the spinner
/// back (dogfood 2026-09-02, T-71; over the whole activity log 19 of the
/// probe's 40 verdicts were followed by a `PostToolUse` or `Stop`, i.e. by
/// the turn it had just declared dead). The primary catch — the transcript's
/// `[Request interrupted by user]` record, `poll_tails`' abort-only class —
/// lands in ~2 s regardless, and post-interrupt painting already held the
/// pane "active" for 60–80 s live (STALE-MAP, T-50), so the recordless case
/// was never a fast one. Sixty seconds clears every working silence
/// measured and costs that rare case a minute it was mostly paying anyway.
const PANE_QUIET_MS: u64 = 60_000;
/// A `status: idle` in Claude's session file counts only when stamped this
/// far after the Running spell began: the previous turn's `idle` write and
/// this turn's `UserPromptSubmit` hook can land in either order (a prompt
/// typed ahead is submitted the instant the turn ends), and a stale idle read
/// as this turn's would blank a card that just started working. That hazard
/// is milliseconds wide; a person's Esc is not. It was 1 s for an hour and
/// blocked a live Esc 938 ms after Enter (dogfood 2026-09-04).
const STATUS_IDLE_MARGIN_MS: u64 = 250;
/// How long a Running session with no session file goes between looks for
/// one (an older Claude Code writes none; a scan is ~40 small reads).
const STATUS_FILE_RETRY_MS: u64 = 30_000;
/// Tickets in this column are sleep-safe: their sessions feed the header's
/// sleep suggestion. Interim hardcode — becomes a per-column sleep policy
/// (`never|offer|auto`) with M5's column policies.
const SLEEP_SAFE_COLUMN: &str = "DONE";
/// A sleep-safe ticket whose sessions have all been asleep this long feeds
/// the header's archive suggestion (same offer-not-action shape as sleep).
const ARCHIVE_SUGGEST_MS: u64 = 3_600_000;

/// `probe_status_files`' memory of one Running session's session file.
struct StatusProbe {
    path: Option<std::path::PathBuf>,
    /// Epoch ms of the last failed search; 0 means never looked.
    looked_at: u64,
}

struct GraceEntry {
    ticket: Ticket,
    sessions: Vec<SessionRecord>,
    expires: Instant,
    /// The delete-gate's red "remove": the user confirmed losing unmerged
    /// work, so teardown may `branch -D` (M4).
    discard_worktree: bool,
    /// The note bodies, read off the disk before the ticket directory went:
    /// `delete_ticket_dir` is eager (a crash inside the band must not
    /// resurrect a deleted ticket on the next load), so undo has nowhere
    /// else to get them back from. Bounded — `NOTE_MAX_BYTES` a note.
    notes: Vec<(ulid::Ulid, String)>,
}

/// Raised by the SIGTERM handler, honoured on the next wheel tick.
static TERM_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigterm(_: libc::c_int) {
    TERM_REQUESTED.store(true, Ordering::Relaxed);
    // One TERM asks; a second one kills. A daemon wedged past the flag must
    // still be killable the way it always was.
    // SAFETY: `signal` is async-signal-safe, and SIG_DFL is a valid disposition.
    unsafe { libc::signal(libc::SIGTERM, libc::SIG_DFL) };
}

/// Make SIGTERM a clean shutdown (`begin_shutdown` on the next tick) instead of
/// an instant death. Called by the `daemon` subcommand only — never by the
/// in-process daemons the e2e suite runs, whose process is the test runner's.
pub fn install_sigterm_handler() {
    // SAFETY: the handler touches one atomic and one async-signal-safe call.
    unsafe { libc::signal(libc::SIGTERM, on_sigterm as *const () as libc::sighandler_t) };
}

enum Msg {
    Request(Envelope, Sender<Response>, Arc<Mutex<UnixStream>>),
    Hook(HookFrame),
    Tick,
    /// A provisioning thread finished (M4): the binding, or the failing stage.
    Provisioned(ulid::Ulid, std::result::Result<Binding, (String, String)>),
    /// A shell-environment capture finished. Off-thread because it forks the
    /// user's login shell and runs their rc files (`crate::shellenv`).
    ShellEnvCaptured(std::result::Result<crate::shellenv::ShellEnv, String>),
    /// A git sample of the board's own checkout landed (T-124), with the
    /// verdict of the fetch that preceded it when one was asked for.
    GitSampled(mesimon_core::command::RepoGit, Option<std::result::Result<(), String>>),
    /// A client's reader thread returned: its connection is closed. The
    /// merge train it may have armed disarms with it (2026-09-04).
    ClientGone(Arc<Mutex<UnixStream>>),
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
    /// A prompt typed on the board at a SLEEPING claude, held until the wake
    /// it triggered has a pane that reads (the board's Shift+Enter on a
    /// parked agent, 2026-09-04). Delivered by `retry_pending_submits` on
    /// the first tick after the `SessionStart` edge, as a bracketed paste —
    /// never typed ahead into the pty, which is canonical-mode input capped
    /// at 1 KiB until Claude sets raw mode. In memory beside `submit_retry`
    /// for the same reason it is: a restart drops the words rather than
    /// pasting them into a pane it no longer understands.
    ///
    /// Since T-224 (2026-09-05) the composed spawn parks here too: the
    /// ticket's DESCRIPTION, to be pasted under the typed title before the
    /// owed Enter, so the agent's first prompt is the whole brief and not the
    /// title alone. `Parked::brief` says which of the two it is, because only
    /// the second stamps `SessionRecord::ticket_read`.
    pending_prompt: HashMap<uuid::Uuid, Parked>,
    /// Asks parked until the ticket's CHECKOUT is quiet (2026-09-04, after
    /// five claudes in one checkout committed at once): the board's
    /// Shift+Enter with the field's toggle at `queued`. FIFO per checkout,
    /// one entry per ticket, pasted by `drain_queue` when `checkout_holders`
    /// is empty. In memory for `pending_prompt`'s reason: a restart drops
    /// the words rather than pasting them into a pane it no longer
    /// understands, and the mark on the card goes with them.
    queued: Vec<QueuedAsk>,
    /// Tickets whose pane mesimon pasted into ON ITS OWN CLOCK — a queued
    /// ask, the train's rebase request or merged notice — whose
    /// `UserPromptSubmit` has not landed yet: `(expiry ms, feed word on the
    /// ack)`. The daemon cannot tell its own paste's ack from a keystroke of
    /// the user's, so the next prompt on the ticket closes it either way; a
    /// ticket here counts as WORKING (`quiet::working_tickets`), which is
    /// what keeps a second paste out of the same checkout in the same pass.
    inflight: HashMap<ulid::Ulid, (u64, &'static str)>,
    /// The merge train (2026-09-04): armed by a connection, what it asked,
    /// its fuse. See `crate::train`.
    train: crate::train::Train,
    /// The base branch's tip as of the last `refresh_worktree_flags` — a
    /// rebase ask is recorded against it, and repeated only once it moves.
    base_tip: String,
    ticks: u64,
    feed: FeedWriter,
    /// Discovered foreign sessions (19 §4 tier 1). Never persisted; refreshed
    /// only on `RescanExternal` (the drawer opening).
    external: Vec<ExternalItem>,
    /// Observe-tier transcript cursors for adopted hook-less sessions.
    tails: HashMap<uuid::Uuid, TailCursor>,
    /// Per Running Claude session: where its `~/.claude/sessions/<pid>.json`
    /// is, or when we last failed to find it (`probe_status_files`).
    status_files: HashMap<uuid::Uuid, StatusProbe>,
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
    agent_replay: HashMap<(uuid::Uuid, String), AgentReplay>,
    /// The user's own shell environment, as their login shell last reported
    /// it. Every spawn hands this to the pane, because a Claude pane is exec'd
    /// directly by tmux and so reads no rc file of its own.
    shell_env: crate::shellenv::ShellEnv,
    /// A capture is in flight. One at a time: it forks a shell, and a second
    /// one racing the first would only decide which stale answer wins.
    shell_env_capturing: bool,
    /// Why the last capture failed, if it did. The previous environment stays
    /// in force — a broken rc file must not empty a working pane env.
    shell_env_error: Option<String>,
    /// This binary, for the pane launcher (`mesimon exec`) and the hook.
    self_exe: std::path::PathBuf,
    /// Where the board's own checkout stands (T-124): the SAMPLED part only —
    /// the fetch bookkeeping beside it is stamped into the snapshot, so an
    /// armed fetch cannot make every cycle read as a change.
    git_cache: mesimon_core::command::RepoGit,
    /// Whether the repo's own `CLAUDE.md` tells a session to read its ticket
    /// (T-217), behind an mtime gate — see `claudemd::Sampler`.
    claude_md: crate::claudemd::Sampler,
    /// A sample (or fetch + sample) is running on a worker thread.
    git_inflight: bool,
    /// Something asked for a sample while one was in flight: run again when
    /// it lands rather than drop the ask (a merge just moved main, a press).
    git_wanted: bool,
    /// The next sample fetches first — the periodic cadence came due, or the
    /// menu row was pressed.
    git_fetch_wanted: bool,
    /// The in-flight sample is fetching. What the menu row shows meanwhile.
    git_fetching: bool,
    /// `MESIMON_GIT_FETCH`, read once. Zero = only the menu row fetches.
    git_fetch_every: Duration,
    /// When the last fetch was ATTEMPTED — a failing remote is retried on the
    /// cadence, never every sample.
    git_last_fetch: Option<Instant>,
    /// When the last fetch succeeded (unix ms), 0 = never.
    git_fetched_at_ms: u64,
    /// The last fetch's first stderr line; cleared by the next success.
    git_fetch_error: Option<String>,
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
    // Same-uid only, like hook.sock: the socket's mode is the whole auth.
    std::fs::set_permissions(&sock_path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;

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
    // The user may have left a session while the daemon was down (this repo
    // restarts one after every daemon-side rebuild). Reconcile still says
    // `Exited`; `park_on_exit` gets the same say it would have had live —
    // but only over records THIS reconcile just moved, never over corpses
    // that were already persisted as dead, which must stay dead.
    let mut just_exited = Vec::new();
    for (id, link) in &rec.links {
        if let Some(r) = board.sessions.iter_mut().find(|s| s.id == *id) {
            // Observe-only records (imported, never spawned) have no pane by
            // design — Missing is their normal condition, not a crash.
            let observe_only = r.provenance == Provenance::Adopted && r.argv.is_empty();
            if observe_only && matches!(link, mesimon_core::reconcile::Link::Missing) {
                continue;
            }
            let was_live = r.state.is_live();
            r.state = state_for(link, &r.state, r.kind == SessionKind::Claude);
            if was_live && matches!(r.state, SessionState::Exited { reason: ExitReason::UserQuit })
            {
                just_exited.push(*id);
            }
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
                if (&mut stream).take(ingest::HOOK_FRAME_MAX_BYTES).read_to_end(&mut buf).is_ok() {
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
        .map(|s| {
            let m = Machine::restore_with_teammates(
                s.state.clone(),
                s.confidence,
                s.idle_teammates.iter().cloned(),
                now,
            );
            (s.id, m)
        })
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
        pending_prompt: HashMap::new(),
        queued: Vec::new(),
        inflight: HashMap::new(),
        train: Default::default(),
        base_tip: String::new(),
        ticks: 0,
        feed,
        external: Vec::new(),
        tails: HashMap::new(),
        status_files: HashMap::new(),
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
        shell_env: crate::shellenv::ShellEnv::default(),
        shell_env_capturing: false,
        shell_env_error: None,
        self_exe: hook_bin.clone(),
        git_cache: mesimon_core::command::RepoGit::default(),
        claude_md: crate::claudemd::Sampler::default(),
        git_inflight: false,
        git_wanted: false,
        git_fetch_wanted: false,
        git_fetching: false,
        git_fetch_every: crate::gitstatus::fetch_every_from_env(),
        git_last_fetch: None,
        git_fetched_at_ms: 0,
        git_fetch_error: None,
    };
    let mut parked = false;
    for id in just_exited {
        parked |= d.park_on_exit(id);
    }
    if parked {
        // The reconcile above already saved the corpse; write the park over
        // it, so a daemon that dies in its first second does not lose it.
        d.persist_sessions();
    }
    d.refresh_worktree_flags();
    // Whether the repo already tells its sessions to read their ticket. One
    // read at startup, then only when a `stat` says the file moved.
    d.claude_md.refresh(&d.paths.repo_root);
    // The checkout's own state, off-thread; the header is blank until it lands.
    d.queue_git_sample();
    // Ask the user's shell what the environment is, immediately. Until the
    // answer lands, spawns fall back to the daemon's own inherited env — which
    // is what every spawn used before this existed, so the window is a
    // regression to the old behaviour rather than to no behaviour at all.
    d.queue_shell_env_capture();

    for msg in rx {
        match msg {
            // SIGTERM (`pkill -f "mesimon daemon"` after a rebuild) takes the
            // same road as `Shutdown`: the handler only raises a flag, and the
            // wheel — ≤250 ms away — is where it is honoured, on the writer
            // thread, with the machines in hand.
            Msg::Tick if TERM_REQUESTED.load(Ordering::Relaxed) => {
                d.begin_shutdown();
                break;
            }
            Msg::Tick => d.on_tick(),
            Msg::Hook(frame) => d.on_hook(frame),
            Msg::Provisioned(ticket, result) => d.on_provisioned(ticket, result),
            Msg::ShellEnvCaptured(result) => d.on_shell_env(result),
            Msg::GitSampled(sample, fetched) => d.on_git_sampled(sample, fetched),
            Msg::ClientGone(stream) => d.on_client_gone(&stream),
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
    ///
    /// Only mesimon's own variables: the user's captured environment reaches
    /// the pane through the launcher's file (`launch`), never through here.
    fn session_vars(&self, ticket: ulid::Ulid, cwd: &std::path::Path) -> Vec<(String, String)> {
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

    /// A pane's real command line: `mesimon exec --env <file> --set K=V -- argv`.
    ///
    /// The captured environment is read INSIDE the pane from a 0600 file and
    /// applied by `exec`, so it is never spelled on a tmux command line where
    /// every user on the machine can read it. `--set` carries mesimon's own
    /// per-session variables, applied last so nothing captured can shadow
    /// them; a ticket key on the command line is a feature, not a leak. The
    /// record keeps the RAW argv (`SessionRecord::argv`) and is wrapped here
    /// at every spawn, so a binary that moved wraps with its new path.
    fn launch(&self, argv: &[String], vars: &[(String, String)]) -> Vec<String> {
        let mut out = vec![
            self.self_exe.display().to_string(),
            "exec".into(),
            "--env".into(),
            self.paths.shell_env_file().display().to_string(),
        ];
        for (k, v) in vars {
            out.push("--set".into());
            out.push(format!("{k}={v}"));
        }
        out.push("--".into());
        out.extend(argv.iter().cloned());
        out
    }

    /// What the launcher reads: the admissible variables plus `PATH`, `K=V\0`
    /// entries, written whole-or-not (temp + rename) at 0600. `PATH` is here
    /// like everything else — the launcher sets it in the pane — AND on the
    /// tmux client (`set_path`), so tmux's own lookups and the pane agree.
    fn write_shell_env_file(&self) -> Result<()> {
        let mut content = String::new();
        let path = self.shell_env.path.iter().map(|p| ("PATH".to_string(), p.clone()));
        for (k, v) in self.shell_env.vars.iter().cloned().chain(path) {
            content.push_str(&k);
            content.push('=');
            content.push_str(&v);
            content.push('\0');
        }
        store::write_atomic(&self.paths.shell_env_file(), &content, store::PRIVATE)
    }

    /// Ask the user's login shell for its environment, off the writer thread.
    ///
    /// One at a time (`shell_env_capturing`): the capture forks a shell and
    /// runs the user's rc files, and two racing captures would differ only in
    /// which stale answer happened to land second.
    fn queue_shell_env_capture(&mut self) {
        if self.shell_env_capturing {
            return;
        }
        self.shell_env_capturing = true;
        let dump = self.paths.shell_env_dump();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::ShellEnvCaptured(crate::shellenv::capture(&dump)));
        });
    }

    /// A capture landed. `PATH` goes to the backend (it rides the tmux CLIENT
    /// environment — `TmuxBackend::set_path` says why), the rest is held for
    /// the next spawn's `-e`.
    ///
    /// Nothing restarts and nothing is retro-fitted: a live pane keeps the
    /// environment it was born with, because changing a running process's
    /// environment is not a thing anyone can do. Sleep/wake is how an existing
    /// session picks the new one up, and the ticket page says so.
    fn on_shell_env(&mut self, result: std::result::Result<crate::shellenv::ShellEnv, String>) {
        self.shell_env_capturing = false;
        match result {
            Ok(env) => {
                self.shell_env_error = None;
                self.backend.set_path(env.path.clone());
                // Best-effort, and only cosmetic for panes: it keeps
                // `show-environment -g` from telling the next person debugging
                // this a two-day-old story.
                let _ = self.backend.publish_path();
                self.shell_env = env;
                // The previous file stands if this fails, and the failure is
                // said out loud: a pane silently getting last week's exports
                // is the bug this whole mechanism exists to end.
                if let Err(e) = self.write_shell_env_file() {
                    self.shell_env_error = Some(format!("writing the environment file: {e}"));
                }
            }
            // The previous environment stands. A broken rc file must not be
            // able to empty the environment every future pane gets.
            Err(e) => self.shell_env_error = Some(e),
        }
        self.notices.retain(|n| n.kind != SHELL_ENV_NOTICE);
        if let Some(e) = &self.shell_env_error {
            self.notices.push(
                mesimon_core::command::Notice::new(
                    SHELL_ENV_NOTICE,
                    "could not read your shell environment — sessions keep the last one",
                )
                .with_detail(e.clone()),
            );
        }
        self.persist_and_notify();
    }

    /// Has an rc file moved since the capture the panes are being given?
    ///
    /// False while a capture is in flight, so taking the offer makes the
    /// suggestion go away immediately rather than after the shell returns.
    fn shell_env_stale(&self) -> bool {
        !self.shell_env_capturing
            && self.shell_env.rc_stamp > 0
            && crate::shellenv::rc_stamp() > self.shell_env.rc_stamp
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

/// How long a paste of mesimon's own counts as a turn before its
/// `UserPromptSubmit` is given up on (a modal in the pane, a box that is
/// not reading).
const INFLIGHT_MS: u64 = 10_000;

/// How long a pane must have been silent before the train pastes into it:
/// a person typing there makes it active, and their half-sentence must not
/// get mesimon's appended and submitted.
const TRAIN_PANE_QUIET_MS: u64 = 5_000;

/// Words waiting for a pane that reads (`Daemon::pending_prompt`). `brief`
/// marks the composed spawn's paste of the ticket description (T-224): it is
/// what stamps `ticket_read` on the record when it lands, where the board's
/// ask at a sleeping claude — the user's own words — stamps nothing.
struct Parked {
    text: String,
    brief: bool,
}

/// An ask parked until its checkout is quiet (see `Daemon::queued`).
struct QueuedAsk {
    ticket: ulid::Ulid,
    /// The pane it was queued at (`Board::pane_target` then); a different
    /// session in the seat at delivery time drops it.
    session: uuid::Uuid,
    /// The checkout key: `SessionRecord.cwd`, compared as a string.
    cwd: String,
    text: String,
    #[allow(dead_code)]
    queued_at: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The notice kind a failed shell-env capture stands under. One kind, replaced
/// rather than appended, so a shell that fails on every reload leaves one row.
const SHELL_ENV_NOTICE: &str = "shell_env";

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
    let target = match &env.command {
        Command::DiffList { target } | Command::DiffFile { target, .. } => *target,
        _ => return Response::Err { message: "not a diff command".into() },
    };
    // The short-circuit runs before `handle_agent`, so `mcp::agent_allows` —
    // which denies both diff commands — is never reached on the path they
    // actually take. Say it here, where it is true: a diff is a read of the
    // user's whole working tree, and D10's never-tier means it.
    if matches!(env.principal, Principal::Agent { .. }) && !mcp::agent_allows(&env.command) {
        return Response::Err { message: "denied: not in the agent tier".into() };
    }
    // D32c invariant 2 holds on this path too — the short-circuit must not
    // bypass the chokepoint. The checkout is the board's own, so it is a read
    // of the board rather than of any one ticket.
    let resource = match target {
        DiffTarget::Ticket { id } => Resource::Ticket { id },
        DiffTarget::Checkout => Resource::Board,
    };
    if let Decision::Deny { reason } = authorize(&env.principal, &Action::Read, &resource) {
        return Response::Err { message: format!("denied: {reason}") };
    }
    let _permit = PermitGuard::acquire(&ctx.permits);
    let repo = &ctx.paths.repo_root;
    let DiffTarget::Ticket { id: ticket } = target else {
        return match &env.command {
            Command::DiffList { .. } => crate::diff::checkout_diff_list(repo)
                .unwrap_or_else(|e| Response::Err { message: e.to_string() }),
            Command::DiffFile { path, context, .. } => {
                match crate::diff::checkout_diff_file(repo, path, *context) {
                    Ok(file) => Response::DiffFile { file },
                    Err(e) => Response::Err { message: e.to_string() },
                }
            }
            _ => unreachable!(),
        };
    };
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

/// The longest request line `orch.sock` will buffer. Commands are small
/// (a prompt is capped at 4 KiB before it is even sent); this is headroom.
const ORCH_LINE_MAX_BYTES: u64 = 1 << 20;

fn client_loop(stream: UnixStream, tx: Sender<Msg>, diff_ctx: Arc<DiffCtx>) {
    let writer = Arc::new(Mutex::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    }));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        // Bounded: the MCP shim in an agent's process tree is a client too,
        // and an unterminated line must not grow the writer's heap. A line
        // that hits the cap (or EOF) without its newline ends the connection.
        match std::io::Read::take(&mut reader, ORCH_LINE_MAX_BYTES).read_line(&mut line) {
            Ok(n) if n == 0 || !line.ends_with('\n') => break,
            Ok(_) => {}
            Err(_) => break,
        }
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
    // Same `tx` as the requests above, so it orders after them: whatever
    // this connection armed is disarmed once its last word is in.
    let _ = tx.send(Msg::ClientGone(writer));
}

/// What a mutating agent tool call left behind, kept under its idempotency
/// key so a retry gets the first receipt. Keyed by tool as well as by key:
/// a `move_ticket` retry must never be answered with a `create_ticket`
/// receipt that happened to share a client-minted id.
#[derive(Debug, Clone)]
enum AgentReplay {
    Moved { column: String },
    Created { key: String, column: String },
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
        // D32c invariant 2: the chokepoint is on every path, even though v0.1
        // allows. What a command IS — read or mutate, logged or not, about
        // which ticket — is `Command::meta`, one exhaustive table in core.
        let meta = env.command.meta();
        // Reading a pane IS reading the session, and the chokepoint should
        // say so: `authorize` denies an agent `Resource::Session` outright,
        // and naming the resource here is what makes that rule reachable
        // rather than merely true.
        let resource = match &env.command {
            Command::PaneTail { session, .. } => Resource::Session { id: *session },
            // Typing into a pane is changing the session, and the chokepoint
            // should say which one. `Board` when there is no target: the
            // command is about to refuse anyway, and inventing a session id
            // to authorize against would be the wrong kind of tidy.
            Command::PromptSession { ticket, .. } => match self.prompt_target(*ticket) {
                Some(id) => Resource::Session { id },
                None => Resource::Board,
            },
            // A note is the ticket's: the first local commands to name the
            // precise resource, which is what `authorize` was built to hear.
            Command::ReadNote { ticket, .. }
            | Command::WriteNote { ticket, .. }
            | Command::NoteToAgent { ticket, .. } => Resource::Ticket { id: *ticket },
            _ => Resource::Board,
        };
        if let Decision::Deny { reason } = authorize(&env.principal, &meta.action, &resource) {
            return Response::Err { message: format!("denied: {reason}") };
        }
        let feed_cmd = meta.logged.then(|| (env.command.wire_name(), meta.subject));

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
                .with_ticket(id, |t| t.title = mesimon_core::board::sanitize_title(&title))
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
            Command::MoveTag { group, name, to_group, to_index } => {
                self.move_tag(group, name, to_group, to_index)
            }
            Command::MergeTicket { id } => self.merge_ticket(id, &Principal::Local),
            Command::MergeToAgent { id, request } => {
                self.merge_to_agent(id, request, &Principal::Local)
            }
            Command::PromptSession { ticket, text, queued } => {
                self.prompt_session(ticket, text, queued)
            }
            Command::DropQueuedAsk { ticket } => self.drop_queued_ask(ticket),
            Command::SetAutomation { merge_train, merge_notice } => {
                self.set_automation(merge_train, merge_notice, stream)
            }
            Command::ReadNote { ticket, note } => self.read_note(ticket, note),
            Command::WriteNote { ticket, note, text } => {
                self.write_note(ticket, note, text, &Principal::Local)
            }
            Command::NoteToAgent { ticket, note } => self.note_to_agent(ticket, note),
            Command::RestoreTicket { id } => self.restore_ticket(id),
            Command::ArchiveTicket { id } => self.archive_ticket(id),
            Command::UnarchiveTicket { id } => self.unarchive_ticket(id),
            Command::SnoozeTicket { id, until, needs_you } => {
                self.snooze_ticket(id, until, needs_you)
            }
            Command::SeenTicket { id } => self.seen_ticket(id),
            Command::SetManualMerge { id, on } => self.set_manual_merge(id, on),
            Command::SetMcpTools { on } => self.set_mcp_tools(on),
            Command::SetSystemPrompt { on } => self.set_system_prompt(on),
            Command::IgnoreBriefOffer => self.ignore_brief_offer(),
            Command::ArchiveAll => {
                let (archived, skipped) = self.archive_all();
                if archived > 0 {
                    self.persist_and_notify();
                }
                Response::Archived { archived, skipped }
            }
            Command::ReloadShellEnv => {
                self.queue_shell_env_capture();
                // Broadcast now, not on the capture's return: `reloading`
                // becoming true is what takes the offer off the header, and a
                // slow rc file must not leave the chip standing for 15 s as
                // though the press had missed.
                self.persist_and_notify();
                Response::Ok
            }
            Command::GitFetch => {
                if self.git_cache.upstream.is_none() {
                    return Response::Err { message: "no upstream to fetch".into() };
                }
                self.git_fetch_wanted = true;
                self.queue_git_sample();
                // `fetching` becoming true is what the menu row shows for the
                // press; a slow remote must not leave the row looking missed.
                self.broadcast();
                Response::Ok
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
                self.begin_shutdown();
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
                        Response::Spawned { id, fresh: false }
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
            Command::AgentGetTicket
            | Command::AgentListBoard
            | Command::AgentMoveTicket { .. }
            | Command::AgentReadNote { .. }
            | Command::AgentWriteNote { .. }
            | Command::AgentCreateTicket { .. }
            | Command::AgentTagTicket { .. } => {
                Response::Err { message: "agent commands require an agent principal".into() }
            }
        };
        if let Some((cmd, ticket)) = feed_cmd {
            let cmd = cmd.as_str();
            match &resp {
                Response::Ok
                | Response::Spawned { .. }
                | Response::Provisioning
                | Response::NoteWritten { .. } => self.feed.board("local", cmd, ticket),
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
            changed |= self.wake_snoozed(now / 1000);
            // The queued asks' safety net: the edge above is the road, this
            // is the clock (a paste that never got its ack, a target that
            // went without a state change of its own).
            changed |= self.sweep_queue();
            changed |= self.expire_inflight(now);
            changed |= self.drain_queue(now);
            let a = self.archive_figures();
            if a != self.archive_cache {
                self.archive_cache = a;
                changed = true;
            }
        }
        if self.ticks % TAIL_POLL_TICKS == 0 {
            changed |= self.poll_tails();
            changed |= self.probe_status_files();
            changed |= self.refresh_titles();
        }
        if self.ticks % RSS_TICKS == 0 {
            changed |= self.refresh_rss();
        }
        // The worktree flags on their own cadence (a seam for the train's
        // e2e), and the train's pass right behind them, on fresh flags.
        if self.ticks % wt_refresh_ticks() == 0 && !self.worktrees.is_empty() {
            self.refresh_worktree_flags();
            changed |= self.train_pass();
        }
        // The CLAUDE.md sample, on the same slow bucket but off the worktree
        // guard: a board with no worktrees still has a CLAUDE.md. Two `stat`s
        // unless something moved, so it costs the same as asking whether to
        // ask.
        if self.ticks % wt_refresh_ticks() == 0 {
            changed |= self.claude_md.refresh(&self.paths.repo_root);
        }
        // One tick off the writer's own burst above: the sample is a fork on
        // a worker, but its spawn should not stack on the worktree flags.
        if self.ticks % RSS_TICKS == 1 {
            if !self.git_fetch_every.is_zero()
                && self.git_last_fetch.is_none_or(|t| t.elapsed() >= self.git_fetch_every)
            {
                self.git_fetch_wanted = true;
            }
            self.queue_git_sample();
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

    /// The one shutdown road, for `Command::Shutdown` and SIGTERM alike.
    ///
    /// A pending settle is a frame the session already sent — a `Stop` one
    /// second before a `U` reload — and it dies with the process unless it is
    /// committed here: the restart re-derives it from the transcript at Low
    /// confidence, where automove refuses to move, so the ticket sat in IN
    /// PROGRESS with its turn over (dogfood 2026-09-01, T-140). Committing
    /// runs the ordinary `apply_change` path, so the automove fires and the
    /// records are persisted before the socket goes.
    fn begin_shutdown(&mut self) {
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> =
            self.machines.iter_mut().filter_map(|(id, m)| m.flush(now).map(|c| (*id, c))).collect();
        let mut changed = false;
        for (id, change) in fired {
            changed |= self.apply_change(id, &change, None, Some("shutdown"));
        }
        if changed {
            self.persist_sessions();
            self.broadcast();
        }
        self.shutting_down = true;
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

    /// The recordless Esc. Pressed before the first assistant output, Claude
    /// Code hands the prompt back to the box and writes NOTHING to the
    /// transcript (spike S-E's case, live 2026-09-04: "the conv returned to
    /// the last message before I prompted") — no hook, no record, and the
    /// pane-quiet probe a minute later was the whole catch. But Claude Code
    /// keeps `~/.claude/sessions/<pid>.json` for its own peers (`notify_idle`
    /// among its `peerFeatures`), and its `status` flips `busy` → `idle` at
    /// the keypress with `statusUpdatedAt` beside it. Measured over the 40
    /// live files on this machine 2026-09-04: every `busy` was a Running
    /// record of ours, every `idle` an Idle one, and the interrupted
    /// session's stamp was the Esc's own second. Doc 11 §11.3 barred the file
    /// from setting state as "best-effort enrichment"; the measurement says
    /// the status is written at every edge, so it gets PaneQuiet's row sixty
    /// seconds earlier — demotion-only, Medium, `Running` only. The stamp
    /// must post-date this Running spell by `STATUS_IDLE_MARGIN_MS`. The
    /// file is found once per spell by sessionId + live pid
    /// (`census::status_file_for`), then read in place.
    fn probe_status_files(&mut self) -> bool {
        let now = now_ms();
        let cands: Vec<(uuid::Uuid, uuid::Uuid, u64)> = self
            .board
            .sessions
            .iter()
            .filter(|r| {
                r.kind == SessionKind::Claude
                    && r.state == SessionState::Running
                    && !(r.provenance == Provenance::Adopted && r.argv.is_empty())
            })
            .map(|r| (r.id, r.claude_session_id.unwrap_or(r.id), r.state_changed_at.unwrap_or(now)))
            .collect();
        self.status_files.retain(|id, _| cands.iter().any(|(c, _, _)| c == id));
        if cands.is_empty() {
            return false;
        }
        let home = crate::census::claude_home();
        let mut changed = false;
        for (id, claude_id, since) in cands {
            let probe =
                self.status_files.entry(id).or_insert(StatusProbe { path: None, looked_at: 0 });
            if probe.path.is_none() && now.saturating_sub(probe.looked_at) >= STATUS_FILE_RETRY_MS {
                probe.path = crate::census::status_file_for(&home, claude_id);
                probe.looked_at = now;
            }
            let Some(path) = probe.path.clone() else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else {
                probe.path = None;
                continue;
            };
            let Ok(pf) = serde_json::from_str::<SessionsPidFile>(&text) else { continue };
            if pf.session_id != Some(claude_id) {
                probe.path = None;
                continue;
            }
            let idle = pf.status.as_deref() == Some("idle")
                && pf.status_updated_at.is_some_and(|at| at >= since + STATUS_IDLE_MARGIN_MS);
            if !idle {
                continue;
            }
            if let Some(change) =
                self.machines.get_mut(&id).and_then(|m| m.apply(&Signal::StatusFileIdle, now))
            {
                changed |= self.apply_change(id, &change, None, Some("status"));
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
            let clean: String =
                mesimon_core::text::scrub_cells(t, false).chars().take(80).collect();
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
    /// "[Request interrupted by user]" at the keypress — known by the words,
    /// since the flag beside them is optional (`adopt::is_interrupt`). Only the Aborted
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
                self.machines.insert(
                    rec.id,
                    Machine::restore_with_teammates(
                        rec.state.clone(),
                        Confidence::Stale,
                        rec.idle_teammates.iter().cloned(),
                        now,
                    ),
                );
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
            // A death that names a pane still ALIVE is the previous tenant's.
            // The pane-died notify carries only the session name, and a
            // wake re-uses the record's sid16 for its new pane; sleep
            // SIGTERMs and returns, so an ask or a `c` a moment later spawns
            // into the name while the old pane's death is still on its way
            // up the hook socket — and that frame, landing on `Spawning`,
            // was read as the NEW pane crashing (prompt_e2e, 2026-09-04,
            // the ask at a sleeping claude). Only the window where a pane
            // was just born can be ambiguous, so only there is tmux asked;
            // "listed and not dead" is the one answer that refutes a death.
            if matches!(sig, Signal::PaneDied { .. }) && self.pane_reborn(id) {
                return;
            }
            // A prompt reached the agent — every road ends here: the board's
            // Shift+Enter field, the composer's submit, a line typed in the
            // pane. The person's own last move on the ticket stops being one
            // the no-undo rule protects, BEFORE the `Running` edge below asks
            // automove to reverse it (T-186). An agent's move keeps its guard.
            if matches!(sig, Signal::UserPromptSubmit) {
                if let Some(t) = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket) {
                    self.moves.asked_by_hand(t);
                    // Our own paste's ack, or the user talking to the agent
                    // while an ask waited — which drops it (2026-09-04).
                    dirty |= self.ack_owed(t);
                }
            }
            let machine = self
                .machines
                .entry(id)
                .or_insert_with(|| Machine::new(SessionState::unknown(), now));
            let change = machine.apply(&sig, now);
            // The machine's idle-teammate set is what decides whether the
            // NEXT Stop parks or ends the turn (T-135); it survives a daemon
            // restart only on the record.
            if matches!(sig, Signal::TeammateIdle { .. } | Signal::TeammateMessaged { .. }) {
                let idle: Vec<String> = machine.idle_teammates().map(str::to_string).collect();
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    if rec.idle_teammates != idle {
                        rec.idle_teammates = idle;
                        dirty = true;
                    }
                }
            }
            if let Some(change) = change {
                dirty |=
                    self.apply_change(id, &change, ingest::detail_of(&frame), Some(&frame.event));
                // ...unless the user simply left. A clean exit is a park.
                dirty |= self.park_on_exit(id);
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
            // been waiting for. `Startup` and `Resume` — the two edges a
            // spawn or a wake lands on; a Clear/Compact SessionStart is a
            // conversation ending inside a living pane, and nothing is owed
            // there. `Resume` joined `Startup` on 2026-09-04 for the board's
            // ask at a sleeping claude: the wake is a `--resume`, and the
            // prompt it carries waits on this very edge. The flag is what
            // gates it — an in-app `/resume` in a pane that owes nothing is
            // a no-op here.
            if matches!(
                sig,
                Signal::SessionStart { source: StartSource::Startup | StartSource::Resume }
            ) {
                dirty |= self.deliver_pending_submit(id);
            }
            // ...and its ack. Any prompt reaching Claude closes the offer,
            // including one the user typed themselves — either way there is
            // nothing left to press Enter for.
            if matches!(sig, Signal::UserPromptSubmit) {
                dirty |= self.ack_pending_submit(id);
            }
        }
        // An approved plan is the agent's note on the ticket.
        if let Some(plan) = ingest::plan_of(&frame) {
            dirty |= self.record_plan(id, plan);
        }
        if dirty {
            self.persist_sessions();
            self.broadcast();
        }
    }

    /// Write the plan an approved `ExitPlanMode` carried as a note on the
    /// session's ticket, authored by the agent (2026-09-03). The plan file
    /// Claude keeps under `~/.claude/plans/` is off the board; this is the
    /// same document on it, and the note is fetched exactly the way the
    /// agent's own `write_note` would be — sanitized, capped and stamped by
    /// `write_note`, authorized as the agent on the ticket, named in the feed
    /// and never quoted there. ONE note per session, replaced on every
    /// approval the way the plan file is, so a re-plan is a revision and not
    /// a second note; a note the user deleted since is not resurrected, the
    /// next approval mints a fresh one. On a ticket with no description it
    /// becomes `notes[0]`, which is what any first note does. Returns
    /// whether the record changed.
    fn record_plan(&mut self, session: uuid::Uuid, plan: String) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return false;
        };
        let ticket = rec.ticket;
        let existing = rec
            .plan_note
            .filter(|n| self.board.ticket(ticket).is_some_and(|t| t.note(*n).is_some()));
        let by = Principal::Agent { session };
        if let Decision::Deny { .. } =
            authorize(&by, &Action::Mutate, &Resource::Ticket { id: ticket })
        {
            return false;
        }
        let Response::NoteWritten { note: Some(id) } = self.write_note(ticket, existing, plan, &by)
        else {
            return false;
        };
        self.feed.board(by.actor(), "plan_note", Some(ticket));
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == session) else {
            return false;
        };
        if rec.plan_note == Some(id) {
            return false;
        }
        rec.plan_note = Some(id);
        true
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
        // A prompt still parked has not been typed yet, so there is nothing
        // to press Enter on: the edge only starts the clock, and the first
        // tick pastes. T-5's correction is the reason for the gap — input on
        // this edge is a race Claude's startup can lose, and a lost Enter is
        // re-pressed for free where a lost paste is the user's words gone.
        if !self.pending_prompt.contains_key(&id) {
            let _ = self.backend.send_enter(&sid16);
        }
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
            if !rec.pressable() {
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
            // The board's ask at a sleeping claude, delivered: the pane has
            // been up a cadence past its `SessionStart`, so the parked words
            // go in the way a live pane takes them — bracketed paste, then a
            // separate Enter (`paste_text`, the T-5 shape). The presses that
            // follow are the ordinary retries; a paste is made once.
            match self.pending_prompt.remove(&id) {
                Some(Parked { text, brief }) => {
                    let _ = self.backend.paste_text(&sid16, &text);
                    // The brief went in with the first prompt: this session
                    // has read its ticket, whatever it does with the tools.
                    if brief {
                        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                            rec.ticket_read = true;
                            changed = true;
                        }
                    }
                }
                None => {
                    let _ = self.backend.send_enter(&sid16);
                }
            }
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
        self.pending_prompt.remove(&id);
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = false;
        }
    }

    /// A `Spawning` record whose pane tmux lists as alive: a pane-died frame
    /// for it belongs to the pane that held the name before (see the caller).
    fn pane_reborn(&self, id: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        if !matches!(rec.state, SessionState::Spawning) {
            return false;
        }
        let sid16 = rec.sid16();
        self.backend
            .snapshot()
            .ok()
            .and_then(|panes| panes.into_iter().find(|p| p.session_name == sid16))
            .is_some_and(|p| !p.pane_dead)
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
        // A turn ended, or a target died: the queued asks look again. The
        // settle that lands `Idle{EndTurn}` comes through here from the
        // tick, and so does the shutdown flush — the words go out on the
        // way down rather than being lost with the restart.
        self.sweep_queue();
        self.drain_queue(now_ms());
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
                    Some(view) => {
                        // The agent read its ticket: the page stops saying
                        // it has not (T-224). A persisted fact, and a real
                        // delta for the page to draw, so it is broadcast.
                        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == session)
                        {
                            if !rec.ticket_read {
                                rec.ticket_read = true;
                                self.persist_sessions();
                                self.broadcast();
                            }
                        }
                        Response::AgentTicket { ticket: view }
                    }
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
                    if let Some(AgentReplay::Moved { column }) =
                        self.agent_replay.get(&(session, key.clone()))
                    {
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
                            self.remember_agent_result(
                                session,
                                key,
                                AgentReplay::Moved { column: column.clone() },
                            );
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
            // The note tools: the ticket is the binding's, and a note id off
            // it reads as "no such note" inside the handlers.
            Command::AgentReadNote { note } => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Read, &Resource::Ticket { id: ticket })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                self.read_note(ticket, note)
            }
            Command::AgentWriteNote { note, text } => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: ticket })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let resp = self.write_note(ticket, note, text, &by);
                if matches!(resp, Response::NoteWritten { .. }) {
                    self.feed.board(by.actor(), "write_note", Some(ticket));
                }
                resp
            }
            Command::AgentCreateTicket { title, column, description, tags, idempotency_key } => {
                // Replay first, for the same reason as a move: a retry after
                // `Connection closed` must not file the same work twice.
                if let Some(key) = &idempotency_key {
                    if let Some(AgentReplay::Created { key: short_key, column }) =
                        self.agent_replay.get(&(session, key.clone()))
                    {
                        return Response::AgentCreated {
                            key: short_key.clone(),
                            column: column.clone(),
                            board_version: self.board_version,
                            replayed: true,
                        };
                    }
                }
                let by = Principal::Agent { session };
                let resp = self.agent_create_ticket(&by, title, column, description, tags);
                if let (Some(key), Response::AgentCreated { key: short_key, column, .. }) =
                    (idempotency_key, &resp)
                {
                    self.remember_agent_result(
                        session,
                        key,
                        AgentReplay::Created { key: short_key.clone(), column: column.clone() },
                    );
                }
                resp
            }
            Command::AgentTagTicket { name, group, remove } => {
                let by = Principal::Agent { session };
                self.agent_tag_ticket(&by, ticket, &name, group, remove)
            }
            // Unreachable: `agent_allows` above admits exactly seven commands.
            _ => Response::Err { message: "not available to an agent session".into() },
        }
    }

    /// `create_ticket`, for an agent: the same mint a human's composer gets
    /// (`mint_ticket`, `sanitize_title`), authorized as a MUTATE on the
    /// destination COLUMN — one card appended, the board itself untouched —
    /// and refused under the same columns bar the local arm honours, since a
    /// `next_key` that cannot be persisted would regress into an existing
    /// ticket's directory on the next start. The description, when there is
    /// one, is written as the first note by `write_note`, so it carries the
    /// agent as its author the way any note an agent writes does. Tags are
    /// resolved BEFORE the mint (`resolve_agent_tags`): a bad name refuses
    /// the whole call and leaves no half-filed card behind.
    fn agent_create_ticket(
        &mut self,
        by: &Principal,
        title: String,
        column: Option<String>,
        description: Option<String>,
        tags: Vec<String>,
    ) -> Response {
        let Some(column) = column.or_else(|| self.board.columns.first().map(|c| c.name.clone()))
        else {
            return Response::Err { message: "the board has no columns".into() };
        };
        if !self.board.columns.iter().any(|c| c.name == column) {
            return Response::Err { message: format!("no such column: {column}") };
        }
        if let Decision::Deny { reason } =
            authorize(by, &Action::Mutate, &Resource::Column { name: column.clone() })
        {
            return Response::Err { message: format!("denied: {reason}") };
        }
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let title = mesimon_core::board::sanitize_title(&title);
        if title.trim().is_empty() {
            return Response::Err { message: "title is empty".into() };
        }
        let tags = match self.resolve_agent_tags(&tags) {
            Ok(refs) => refs,
            Err(message) => return Response::Err { message },
        };
        let id = self.mint_ticket(column.clone(), title);
        if !tags.is_empty() {
            if let Some(t) = self.board.ticket_mut(id) {
                for r in tags {
                    t.set_tag(r.group, Some(r.name));
                }
                let t = t.clone();
                let _ = store::save_ticket(&self.paths, &t);
            }
        }
        self.persist_and_notify();
        self.feed.board(by.actor(), "create_ticket", Some(id));
        let key = self.board.ticket(id).map(|t| t.short_key.clone()).unwrap_or_default();
        if let Some(text) = description {
            if let Response::Err { message } = self.write_note(id, None, text, by) {
                // The ticket exists either way; the honest receipt says both.
                return Response::Err {
                    message: format!(
                        "ticket {key} created, but its description was not: {message}"
                    ),
                };
            }
        }
        Response::AgentCreated { key, column, board_version: self.board_version, replayed: false }
    }

    /// Tag NAMES from an agent, as registry references — or the reason one
    /// of them is not. The registry is the human's vocabulary and an agent
    /// may not add to it (`RegisterTag` is never-tier), so an unknown name
    /// is refused rather than minted; a name that lives on more than one
    /// axis is refused rather than guessed; and two names on one axis are
    /// refused because a ticket wears one tag per group and picking the
    /// survivor would be inventing the agent's intent.
    fn resolve_agent_tags(&self, names: &[String]) -> Result<Vec<TagRef>, String> {
        let mut refs: Vec<TagRef> = Vec::new();
        for raw in names {
            let Some(name) = sanitize_tag(raw) else {
                return Err("empty tag name".into());
            };
            let def = match self.lookup_agent_tag(&name, None) {
                Ok(def) => def,
                Err(groups) if groups.is_empty() => {
                    return Err(format!(
                        "no such tag: {name} (get_ticket lists the board's tags as allowed_tags)"
                    ))
                }
                Err(groups) => {
                    let groups: Vec<String> = groups.iter().map(u8::to_string).collect();
                    return Err(format!(
                        "tag {name} is on more than one group ({}); spell it as the board does",
                        groups.join(", ")
                    ));
                }
            };
            if refs.iter().any(|r| r.group == def.group && r.name == def.name) {
                continue;
            }
            if let Some(other) = refs.iter().find(|r| r.group == def.group) {
                return Err(format!(
                    "one tag per group: {} and {} are both on group {}",
                    other.name, def.name, def.group
                ));
            }
            refs.push(def);
        }
        Ok(refs)
    }

    /// One agent-spelled name looked up in the registry, the lookup both
    /// `create_ticket` and `tag_ticket` share: exact spelling first, then a
    /// unique case-insensitive match (models read `BUG` off `allowed_tags`
    /// and send back `bug`); `group` narrows the search to one axis. `Err`
    /// carries the axes the name was found on — none means the board does not
    /// know the word, two or more means it cannot be told which. What comes
    /// back is the registry's OWN spelling, never the agent's string, so the
    /// registry is never written on either road.
    fn lookup_agent_tag(&self, name: &str, group: Option<u8>) -> Result<TagRef, Vec<u8>> {
        let on_axis = |t: &&Tag| group.is_none_or(|g| g == t.group);
        let exact: Vec<&Tag> =
            self.board.tags.iter().filter(on_axis).filter(|t| t.name == name).collect();
        let found = if exact.is_empty() {
            self.board
                .tags
                .iter()
                .filter(on_axis)
                .filter(|t| t.name.eq_ignore_ascii_case(name))
                .collect()
        } else {
            exact
        };
        match found.as_slice() {
            [one] => Ok(TagRef { name: one.name.clone(), group: one.group }),
            many => Err(many.iter().map(|t| t.group).collect()),
        }
    }

    /// `tag_ticket`, for an agent: wear (or take off) a tag the board ALREADY
    /// has, on the caller's own ticket.
    ///
    /// The one deliberate difference from the human's `set_tag` is that the
    /// registry is read and never written. There, using a name is what puts
    /// it in the vocabulary — no setup step for a person. Here a name the
    /// registry does not hold is refused, because the registry is the user's
    /// language: which words exist, what each axis means, what colour each
    /// wears. An agent choosing from it helps; an agent adding to it fills an
    /// axis (ten is the cap) with words nobody at the keyboard chose, on an
    /// axis it cannot know the meaning of, and every one of them stays in the
    /// picker forever. So `columns.toml` is never touched on this path and no
    /// write bar applies.
    ///
    /// Authorized as `Mutate` on the ticket, like a note: one card's own
    /// metadata, the board itself untouched. Idempotent — wearing what is worn
    /// and removing what is absent both succeed and change nothing — so a
    /// retry after a dropped connection needs no replay map. The receipt is
    /// the ticket's whole tag list, so the model sees what its call did,
    /// including the groupmate that came off to make room.
    fn agent_tag_ticket(
        &mut self,
        by: &Principal,
        ticket: ulid::Ulid,
        name: &str,
        group: Option<u8>,
        remove: bool,
    ) -> Response {
        if let Decision::Deny { reason } =
            authorize(by, &Action::Mutate, &Resource::Ticket { id: ticket })
        {
            return Response::Err { message: format!("denied: {reason}") };
        }
        let Some(t) = self.board.ticket(ticket) else {
            return Response::Err { message: "no such ticket".into() };
        };
        let def = match self.lookup_agent_tag(name, group) {
            Ok(def) => def,
            Err(groups) if groups.is_empty() => {
                let message = if self.board.tags.is_empty() {
                    "the board has no tags yet; tags are created in the board's tag picker"
                        .to_string()
                } else if let Some(g) = group {
                    format!(
                        "no such tag: {name} in group {g} (get_ticket lists the board's tags as allowed_tags)"
                    )
                } else {
                    format!(
                        "no such tag: {name} (get_ticket lists the board's tags as allowed_tags)"
                    )
                };
                return Response::Err { message };
            }
            Err(groups) => {
                let groups: Vec<String> = groups.iter().map(u8::to_string).collect();
                return Response::Err {
                    message: format!(
                        "tag {name} is on more than one group ({}); pass group to say which",
                        groups.join(", ")
                    ),
                };
            }
        };
        let before = t.tag_in(def.group).map(|r| r.name.clone());
        let wearing = before.as_deref() == Some(def.name.as_str());
        let changed = if remove { wearing } else { !wearing };
        if changed {
            let next = if remove { None } else { Some(def.name.clone()) };
            if self.with_ticket(ticket, |t| t.set_tag(def.group, next)).is_none() {
                return Response::Err { message: "no such ticket".into() };
            }
            self.feed.board(by.actor(), "tag_ticket", Some(ticket));
        }
        let tags = self
            .board
            .ticket(ticket)
            .map(|t| {
                t.tags
                    .iter()
                    .map(|r| AgentTagView { name: r.name.clone(), group: r.group })
                    .collect()
            })
            .unwrap_or_default();
        // The groupmate that came off to make room: only on a put, and only
        // when there was a different one there.
        let replaced = if remove || wearing { None } else { before };
        Response::AgentTagged { tags, replaced, board_version: self.board_version }
    }

    /// Remember a mutating tool call's result so a retry replays it.
    ///
    /// Bounded rather than pruned per session: the map is a safety net for a
    /// dropped connection, not a log, and an unbounded one on a daemon that
    /// runs for weeks is a slow leak nobody would ever look for.
    fn remember_agent_result(&mut self, session: uuid::Uuid, key: String, result: AgentReplay) {
        const MAX_REPLAY_ENTRIES: usize = 512;
        if self.agent_replay.len() >= MAX_REPLAY_ENTRIES {
            self.agent_replay.clear();
        }
        self.agent_replay.insert((session, key), result);
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
            tags: t
                .tags
                .iter()
                .map(|r| AgentTagView { name: r.name.clone(), group: r.group })
                .collect(),
            allowed_tags: self
                .board
                .tags
                .iter()
                .map(|d| AgentTagView { name: d.name.clone(), group: d.group })
                .collect(),
            board_version: self.board_version,
            description: t.description().and_then(|n| {
                store::read_note(&self.paths, &t.short_key, n.id).ok().map(|text| {
                    let cut = mesimon_core::text::cap_bytes(&text, AGENT_DESCRIPTION_MAX_BYTES);
                    if cut.len() < text.len() {
                        format!("{cut}…")
                    } else {
                        text
                    }
                })
            }),
            notes: t
                .notes
                .iter()
                .map(|n| mesimon_core::command::AgentNoteView {
                    id: n.id,
                    name: n.name.clone(),
                    by: n.edited_by.clone(),
                    edited_at: n.edited_at.clone(),
                })
                .collect(),
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
        // A move to the column it is already in is not a column move: it must
        // not reach the feed, the ping-pong guard or the flap fuse. But the
        // MOVE ghost's drop arrives on this same command, and dropped in its
        // own column it means "same column, new slot" — the board's only
        // in-column reorder. `Position::Before` is what carries a slot (every
        // automatic mover says `Top`), so that case is answered as an order
        // change and nothing else. Returning `Ok` for it without moving
        // anything is what made the drop look like it worked and land nowhere.
        if from == dest {
            if let Position::Before(_) = pos {
                self.reorder_within(id, dest, &pos, by)?;
            }
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
            // The card's age is time in column, so a column change is the
            // one thing that restarts it (a reorder returned above).
            t.entered_at = Some(now_iso());
            let t = t.clone();
            let _ = store::save_ticket(&self.paths, &t);
        }
        self.moves.record(id, &from, dest, by, Instant::now());
        if by.is_human() {
            self.train.hand_touched(id);
        }
        self.feed.board(by.actor(), rule, Some(id));
        self.broadcast();
        if automatic {
            self.moves.leave();
        }
        Ok(dest.to_string())
    }

    /// The in-column reorder: `order`, and nothing else.
    ///
    /// No feed line, no ping-pong record, no fuse tick — a card sliding within
    /// its column changes no state any automation watches, and counting it as
    /// a move would let a few drags by hand trip a fuse built for automations.
    /// The column and archived checks are the caller's; this only authorizes
    /// the write.
    fn reorder_within(
        &mut self,
        id: ulid::Ulid,
        column: &str,
        pos: &Position,
        by: &Principal,
    ) -> std::result::Result<(), String> {
        if let Decision::Deny { reason } = authorize(by, &Action::Mutate, &Resource::Ticket { id })
        {
            return Err(format!("denied: {reason}"));
        }
        let siblings: Vec<ulid::Ulid> =
            self.board.column_tickets(column).iter().map(|t| t.id).collect();
        let at = siblings.iter().position(|t| *t == id);
        let others: Vec<ulid::Ulid> = siblings.into_iter().filter(|t| *t != id).collect();
        // Removing the card and reinserting it at `want` in what is left puts
        // it back at index `want` — so `want == at` is the ghost dropped where
        // it was picked up. Minting a fresh index for that would only lengthen
        // the fractional key and broadcast a board that did not change.
        let want = match pos {
            Position::Before(Some(b)) => others.iter().position(|t| t == b).unwrap_or(others.len()),
            _ => others.len(),
        };
        if at == Some(want) {
            return Ok(());
        }
        let order = self.order_within(column, id, pos);
        if let Some(t) = self.board.ticket_mut(id) {
            t.order = order;
            let t = t.clone();
            let _ = store::save_ticket(&self.paths, &t);
        }
        self.broadcast();
        Ok(())
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
        let mut train_fused: Vec<String> = self
            .train
            .fused_tickets()
            .filter_map(|id| self.board.ticket(*id))
            .map(|t| t.short_key.clone())
            .collect();
        if !train_fused.is_empty() {
            train_fused.sort();
            notices.push(Notice::new(
                "merge_train_suspended",
                format!(
                    "merge train suspended for {} — asked to rebase too often. \
                     m on it, or a move by hand, clears it.",
                    train_fused.join(", ")
                ),
            ));
        }
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
            shell_env: mesimon_core::command::ShellEnvStatus {
                stale: self.shell_env_stale(),
                reloading: self.shell_env_capturing,
                failed: self.shell_env_error.is_some(),
                vars: self.shell_env.vars.len(),
            },
            git: mesimon_core::command::RepoGit {
                fetching: self.git_fetching,
                fetch_every_secs: self.git_fetch_every.as_secs(),
                fetched_at_ms: self.git_fetched_at_ms,
                fetch_error: self.git_fetch_error.clone(),
                ..self.git_cache.clone()
            },
            pending: self.pending_items(),
            automation: self.automation_status(),
            claude_md: self.claude_md.status(),
        }
    }

    /// What mesimon owes each ticket (see `Pending`). Empty until the queued
    /// ask and the merge train land; kept in one place so the snapshot road
    /// forks no git.
    fn pending_items(&self) -> Vec<mesimon_core::command::Pending> {
        use mesimon_core::command::Pending;
        let mut out: Vec<Pending> = self
            .queued
            .iter()
            .map(|q| mesimon_core::command::Pending {
                ticket: q.ticket,
                action: "ask".into(),
                waits_on: self.keys_of(&self.checkout_holders(&q.cwd)),
                text: Some(q.text.clone()),
                in_flight: false,
            })
            .collect();
        for (t, (_, word)) in &self.inflight {
            if *word == "queued_ask_delivered" {
                out.push(mesimon_core::command::Pending {
                    ticket: *t,
                    action: "ask".into(),
                    waits_on: Vec::new(),
                    text: None,
                    in_flight: true,
                });
            }
        }
        // What the train will do once the board is quiet — said before it
        // happens, so the card can be watched rather than discovered.
        if self.train.is_armed() && !self.worktrees_barred {
            let plan = self.train_plan();
            let waits_on = self.keys_of(&self.board_busy());
            for t in plan.merge {
                let tip = self
                    .worktrees
                    .get(&t)
                    .map(|b| worktree::branch_tip(&self.paths.repo_root, &b.branch))
                    .unwrap_or_default();
                out.push(Pending {
                    ticket: t,
                    action: "merge".into(),
                    waits_on: waits_on.clone(),
                    text: self.train.refusal(t, &tip, &self.base_tip).map(String::from),
                    in_flight: false,
                });
            }
            for t in plan.rebase {
                out.push(Pending {
                    ticket: t,
                    action: "rebase".into(),
                    waits_on: waits_on.clone(),
                    text: None,
                    in_flight: false,
                });
            }
        }
        out
    }

    fn automation_status(&self) -> mesimon_core::command::AutomationStatus {
        let mut train_asked: Vec<mesimon_core::command::TrainAsk> = self
            .train
            .asked()
            .iter()
            .map(|(t, r)| mesimon_core::command::TrainAsk {
                ticket: *t,
                current: r.base_oid == self.base_tip,
                at_ms: r.at_ms,
                by: if r.by_hand { "local" } else { "train" }.into(),
            })
            .collect();
        train_asked.sort_by_key(|a| a.ticket);
        let mut train_suspended: Vec<ulid::Ulid> = self.train.fused_tickets().copied().collect();
        train_suspended.sort();
        mesimon_core::command::AutomationStatus {
            merge_train: self.train.is_armed(),
            merge_notice: self.train.notice(),
            train_asked,
            train_suspended,
        }
    }

    /// Sample the checkout's git state on a worker thread — fetching first
    /// when a fetch is wanted. One at a time: a second ask while one is in
    /// flight is remembered and run when it lands, never dropped.
    fn queue_git_sample(&mut self) {
        if self.git_inflight {
            self.git_wanted = true;
            return;
        }
        self.git_inflight = true;
        self.git_wanted = false;
        let fetch = std::mem::take(&mut self.git_fetch_wanted) && self.git_cache.upstream.is_some();
        self.git_fetching = fetch;
        let repo = self.paths.repo_root.clone();
        let branch = self.git_cache.branch.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let verdict = if fetch {
                crate::gitstatus::remote_of(&repo, &branch)
                    .map(|remote| crate::gitstatus::fetch(&repo, &remote))
            } else {
                None
            };
            let _ = tx.send(Msg::GitSampled(crate::gitstatus::sample(&repo), verdict));
        });
    }

    /// A sample landed. Broadcast only on a visible change: an idle checkout
    /// sampled every 10 s must not repaint every board.
    fn on_git_sampled(
        &mut self,
        sample: mesimon_core::command::RepoGit,
        fetched: Option<std::result::Result<(), String>>,
    ) {
        self.git_inflight = false;
        let mut changed = std::mem::replace(&mut self.git_fetching, false);
        if let Some(verdict) = fetched {
            self.git_last_fetch = Some(Instant::now());
            match verdict {
                Ok(()) => {
                    self.git_fetched_at_ms = now_ms();
                    self.git_fetch_error = None;
                    // A fetch can mint `refs/remotes/origin/HEAD` (git ≥ 2.48
                    // follows the remote's HEAD), which is the first rung of
                    // `default_branch`'s ladder: let it be asked again.
                    self.base_branch = None;
                }
                Err(e) => self.git_fetch_error = Some(e),
            }
            changed = true;
        }
        if sample != self.git_cache {
            self.git_cache = sample;
            changed = true;
        }
        if self.git_wanted {
            self.queue_git_sample();
        }
        if changed {
            self.broadcast();
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
                t.archived = Some(Archived {
                    at: at.clone(),
                    by: "local".into(),
                    until: None,
                    needs_you: false,
                });
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
        // A title is user text on a card row; scrubbed and bounded here, at
        // the boundary — the composer's own cap is a courtesy a client can lift.
        let title = mesimon_core::board::sanitize_title(&title);
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
            entered_at: Some(now_iso()),
            woke_at: None,
            manual_merge: false,
            workspace: None,
            tags: Vec::new(),
            notes: Vec::new(),
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
        self.train.forget(id);
        self.forget_queued(id, "queued_ask_dropped", "local");
        self.inflight.remove(&id);
        // Sessions detach and keep running through the grace band (D21).
        let sessions: Vec<SessionRecord> =
            self.board.sessions.iter().filter(|s| s.ticket == id).cloned().collect();
        self.board.sessions.retain(|s| s.ticket != id);
        // Bodies first, then the directory: what undo will need is in memory
        // before the only copy is removed (dogfood 2026-09-03, T-71: a note
        // written, the ticket deleted and restored, and the editor could only
        // say "note file missing" from then on).
        let notes = ticket
            .notes
            .iter()
            .filter_map(|n| {
                store::read_note(&self.paths, &ticket.short_key, n.id).ok().map(|text| (n.id, text))
            })
            .collect();
        let _ = store::delete_ticket_dir(&self.paths, &ticket.short_key);
        self.grace.insert(
            id,
            GraceEntry {
                ticket,
                sessions,
                expires: Instant::now() + Duration::from_secs(GRACE_SECS),
                discard_worktree,
                notes,
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

    /// Reposition a tag in the registry. The wearers move with it, so this
    /// writes their files the way `rename_tag` does — a ticket left holding
    /// the old axis would wear a pip its row can no longer reach.
    fn move_tag(&mut self, group: u8, name: String, to_group: u8, to_index: usize) -> Response {
        match self.board.move_tag(group, &name, to_group, to_index) {
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
    fn merge_ticket(&mut self, id: ulid::Ulid, by: &Principal) -> Response {
        if let Decision::Deny { .. } = authorize(by, &Action::Mutate, &Resource::Ticket { id }) {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "not allowed".into(),
            };
        }
        // A person's merge clears the train's memory of this ticket the way a
        // hand move clears the movegate's fuse.
        if by.is_human() {
            self.train.hand_touched(id);
        }
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
        // A CLAUDE — a shell is pinned `Running` for the life of its pane
        // (D15), and an ff-merge never touches the worktree it sits in
        // (2026-09-04; it refused every `m` under a `!` shell before).
        let busy = self.board.sessions.iter().any(|s| {
            s.ticket == id
                && s.kind == SessionKind::Claude
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
                // The merge just moved the checkout's branch: the header's
                // `↑` should say so before the next 10 s bucket.
                self.queue_git_sample();
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
        by: &Principal,
    ) -> Response {
        if let Decision::Deny { .. } = authorize(by, &Action::Mutate, &Resource::Ticket { id }) {
            return Response::Err { message: "not allowed".into() };
        }
        if by.is_human() {
            self.train.hand_touched(id);
        }
        let Some(b) = self.worktrees.get(&id) else {
            return Response::Err { message: "no worktree on this ticket".into() };
        };
        let branch = b.branch.clone();
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        let base = self.base_branch.clone().unwrap_or_else(|| "main".into());
        let Some(rec) = self.board.pane_target(id) else {
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
        let sid = rec.sid16();
        match self.backend.paste_text(&sid, &text) {
            Ok(()) => {
                // A delivered rebase ask is remembered against the base tip,
                // by hand or by train: the train does not ask again until
                // the base moves (2026-09-04).
                if matches!(request, mesimon_core::command::MergeRequest::Rebase) {
                    self.train.record_ask(
                        id,
                        self.base_tip.clone(),
                        now_ms(),
                        by.is_human(),
                        Instant::now(),
                    );
                }
                Response::Ok
            }
            Err(e) => Response::Err { message: format!("could not deliver: {e}") },
        }
    }

    // ------------------------------------------------------------ merge train

    /// `Command::SetAutomation`: arm the train on THIS connection (the
    /// latest arming client owns it), or disarm it from any. Accepted under
    /// the worktrees bar — the pass no-ops there and the bar's own notice
    /// says why — so the Settings row never fights it.
    fn set_automation(
        &mut self,
        merge_train: bool,
        merge_notice: bool,
        stream: &Arc<Mutex<UnixStream>>,
    ) -> Response {
        if merge_train {
            self.train.arm(stream, merge_notice);
        } else if self.train.is_armed() {
            self.train.disarm();
            self.feed.board("local", "merge_train_disarmed", None);
        }
        self.broadcast();
        Response::Ok
    }

    /// A client's reader thread returned. Its subscription goes (the lazy
    /// prune in `broadcast` would catch it on the next failed write), and
    /// the train it armed stops: nobody is watching the board it drives.
    fn on_client_gone(&mut self, stream: &Arc<Mutex<UnixStream>>) {
        self.subscribers.retain(|s| !Arc::ptr_eq(s, stream));
        if self.train.owned_by(stream) {
            self.train.disarm();
            self.feed.board("automation", "merge_train_disarmed", None);
            self.broadcast();
        }
    }

    fn train_flags(&self) -> HashMap<ulid::Ulid, mesimon_core::train::WtFlags> {
        self.worktrees
            .iter()
            .map(|(t, b)| {
                (
                    *t,
                    mesimon_core::train::WtFlags {
                        attached: b.status == BindingStatus::Attached,
                        ahead: self.wt_ahead.get(t).copied().unwrap_or(0),
                        merged: self.wt_merged.get(t).copied().unwrap_or(false),
                        needs_rebase: self.wt_needs_rebase.get(t).copied().unwrap_or(false),
                        conflict: self.wt_conflicts.contains(&b.branch),
                    },
                )
            })
            .collect()
    }

    /// What the train would do, over the cached flags — no git on this road.
    fn train_plan(&self) -> mesimon_core::train::Plan {
        let flags = self.train_flags();
        let asked = self.train.asked_tips();
        let fused: std::collections::HashSet<ulid::Ulid> =
            self.train.fused_tickets().copied().collect();
        mesimon_core::train::plan(&mesimon_core::train::Input {
            board: &self.board,
            flags: &flags,
            base_tip: &self.base_tip,
            asked: &asked,
            fused: &fused,
        })
    }

    /// One pass of the train (2026-09-04), after `refresh_worktree_flags`
    /// on its bucket: ONE action, only while the whole board is quiet. A
    /// merge first — the first REVIEW candidate in board order, through the
    /// same road a hand `m` takes under `Principal::Automation`, then the
    /// merged notice into its agent if that is on (a turn starts; the next
    /// pass waits for it). Else ONE rebase ask, into a pane that has been
    /// quiet for `TRAIN_PANE_QUIET_MS` — a user mid-sentence in the pane
    /// must never get mesimon's appended to theirs. A refused merge is
    /// remembered per tip pair so a dirty main is not retried every bucket.
    fn train_pass(&mut self) -> bool {
        if !self.train.is_armed()
            || self.worktrees_barred
            || self.worktrees.is_empty()
            || self.base_branch.is_none()
        {
            return false;
        }
        if !self.board_busy().is_empty() {
            return false;
        }
        let plan = self.train_plan();
        let by = Principal::Automation { rule: mesimon_core::train::RULE.into() };
        let now = now_ms();
        for t in plan.merge {
            let tip = self
                .worktrees
                .get(&t)
                .map(|b| worktree::branch_tip(&self.paths.repo_root, &b.branch))
                .unwrap_or_default();
            if self.train.refused(t, &tip, &self.base_tip) {
                continue;
            }
            match self.merge_ticket(t, &by) {
                Response::Merge { outcome: MergeOutcome::Merged, .. } => {
                    self.feed.board("automation", "merge_train_merged", Some(t));
                    if self.train.notice() {
                        if self.board.pane_target(t).is_some() {
                            let req = mesimon_core::command::MergeRequest::MergedNotice;
                            match self.merge_to_agent(t, req, &by) {
                                Response::Ok => {
                                    self.feed.board("automation", "merge_train_notified", Some(t));
                                    self.inflight.insert(
                                        t,
                                        (now + INFLIGHT_MS, "merge_train_notice_landed"),
                                    );
                                }
                                _ => self.feed.board(
                                    "automation",
                                    "merge_train_refused:deliver_failed",
                                    Some(t),
                                ),
                            }
                        } else {
                            self.feed.board("automation", "merge_train_refused:no_pane", Some(t));
                        }
                    }
                    return true;
                }
                Response::Merge { outcome: MergeOutcome::Refused, detail } => {
                    self.train.refuse(t, tip, self.base_tip.clone(), detail);
                    self.feed.board("automation", "merge_train_refused:merge", Some(t));
                }
                // NeedsRebase / AlreadyMerged: the cached flags lagged git;
                // the refresh that ran before this pass will not next time.
                _ => return true,
            }
        }
        let Some(t) = plan.rebase.first().copied() else { return false };
        let Some(sid) = self.board.pane_target(t).map(|s| s.sid16()) else { return false };
        let Ok(activity) = self.backend.activity() else { return false };
        let quiet = activity
            .iter()
            .find(|(name, _)| *name == sid)
            .is_some_and(|(_, at)| now.saturating_sub(at * 1000) >= TRAIN_PANE_QUIET_MS);
        if !quiet {
            return false;
        }
        let was_fused = self.train.is_fused(t);
        match self.merge_to_agent(t, mesimon_core::command::MergeRequest::Rebase, &by) {
            Response::Ok => {
                self.feed.board("automation", "merge_train_rebase_asked", Some(t));
                self.inflight.insert(t, (now + INFLIGHT_MS, "merge_train_rebase_landed"));
                if !was_fused && self.train.is_fused(t) {
                    self.feed.board("automation", "merge_train_suspended", Some(t));
                }
                true
            }
            _ => {
                self.feed.board("automation", "merge_train_refused:deliver_failed", Some(t));
                false
            }
        }
    }

    // ------------------------------------------------------------ notes

    /// The ticket's description — `notes[0]`'s body — when it has one with
    /// words in it. Read on the writer thread like `read_note`; an unreadable
    /// file is "no description", never a refused spawn.
    fn description_body(&self, ticket: ulid::Ulid) -> Option<String> {
        let t = self.board.ticket(ticket)?;
        let meta = t.description()?;
        let body = store::read_note(&self.paths, &t.short_key, meta.id).ok()?;
        (!body.trim().is_empty()).then(|| body.trim_end().to_string())
    }

    /// One note's body, whole, for the ticket page or an agent. The file is
    /// read on the writer thread: it is bounded at `NOTE_MAX_BYTES` and the
    /// diff service's off-thread road exists for git, not for one small read.
    fn read_note(&self, ticket: ulid::Ulid, note: ulid::Ulid) -> Response {
        let Some(t) = self.board.ticket(ticket) else {
            return Response::Err { message: "no such ticket".into() };
        };
        let Some(meta) = t.note(note) else {
            return Response::Err { message: "no such note".into() };
        };
        match store::read_note(&self.paths, &t.short_key, note) {
            Ok(text) => Response::Note { text, meta: meta.clone() },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Response::Err { message: "note file missing".into() }
            }
            Err(e) => Response::Err { message: format!("could not read the note: {e}") },
        }
    }

    /// Create, replace or (blank text on an existing note) delete a note.
    /// The body is written BEFORE the metadata so a crash between the two
    /// leaves an orphan file, never a listed note with no file. `by` is who
    /// is asking — the stamp on the meta, never trusted from the wire.
    fn write_note(
        &mut self,
        ticket: ulid::Ulid,
        note: Option<ulid::Ulid>,
        text: String,
        by: &Principal,
    ) -> Response {
        use mesimon_core::board::{note_name, sanitize_note, NoteMeta};
        let text = sanitize_note(&text);
        let blank = text.trim().is_empty();
        let Some(t) = self.board.ticket(ticket) else {
            return Response::Err { message: "no such ticket".into() };
        };
        if let Some(id) = note {
            if t.note(id).is_none() {
                return Response::Err { message: "no such note".into() };
            }
        }
        let key = t.short_key.clone();
        let now = now_iso();
        let author = by.note_author();
        match (note, blank) {
            (None, true) => Response::Err { message: "nothing to save".into() },
            (Some(id), true) => {
                if let Err(e) = store::delete_note(&self.paths, &key, id) {
                    return Response::Err { message: format!("could not delete the note: {e}") };
                }
                self.with_ticket(ticket, |t| t.notes.retain(|n| n.id != id));
                Response::NoteWritten { note: None }
            }
            (existing, false) => {
                let id = existing.unwrap_or_else(ulid::Ulid::new);
                if let Err(e) = store::save_note(&self.paths, &key, id, &text) {
                    return Response::Err { message: format!("could not write the note: {e}") };
                }
                let name = note_name(&text);
                self.with_ticket(ticket, |t| match t.notes.iter_mut().find(|n| n.id == id) {
                    Some(n) => {
                        n.name = name;
                        n.rev += 1;
                        n.edited_at = now.clone();
                        n.edited_by = author.clone();
                    }
                    None => t.notes.push(NoteMeta {
                        id,
                        name,
                        rev: 1,
                        created_at: now.clone(),
                        created_by: author.clone(),
                        edited_at: now.clone(),
                        edited_by: author.clone(),
                    }),
                });
                Response::NoteWritten { note: Some(id) }
            }
        }
    }

    /// Tell the ticket's live claude a note changed — `merge_to_agent`'s
    /// twin: mesimon's own sentence, pasted and submitted, on a human's
    /// gesture only. The note's name is user text headed for another
    /// process, so it crosses `scrub_text`.
    fn note_to_agent(&mut self, ticket: ulid::Ulid, note: ulid::Ulid) -> Response {
        let Some(t) = self.board.ticket(ticket) else {
            return Response::Err { message: "no such ticket".into() };
        };
        let Some(meta) = t.note(note) else {
            return Response::Err { message: "no such note".into() };
        };
        let name = mesimon_core::text::scrub_text(&meta.name);
        let Some(rec) = self.board.pane_target(ticket) else {
            return Response::Err {
                message: "no live claude session on this ticket — start or wake one first".into(),
            };
        };
        let text = format!(
            "Note \"{name}\" on this ticket was just updated; read_note with id {note} \
             returns the new text."
        );
        match self.backend.paste_text(&rec.sid16(), &text) {
            Ok(()) => Response::Ok,
            Err(e) => Response::Err { message: format!("could not deliver: {e}") },
        }
    }

    /// The one live claude a prompt from the board would reach: first in
    /// spawn order, which is the same session `board_enter` focuses, so the
    /// key that asks and the key that goes there cannot land on different
    /// panes. `has_pane` and not `is_live` — a parked session is live and has
    /// no process to type at.
    fn prompt_target(&self, ticket: ulid::Ulid) -> Option<uuid::Uuid> {
        self.board.pane_target(ticket).map(|s| s.id)
    }

    /// The board's Shift+Enter: deliver a prompt the user typed and submit
    /// it, leaving them on the board. Same delivery as `merge_to_agent` —
    /// bracketed paste, then a SEPARATE Enter, because a CR in the same byte
    /// burst is absorbed as pasted content (T-5) — and same explicit-gesture
    /// footing: a key was pressed and a sentence was typed by a person.
    ///
    /// What differs is whose words travel. The merge flow pastes mesimon's;
    /// this pastes only the user's, sanitized by subtraction alone
    /// (`sanitize_prompt`), so the README's zero-prompt-injection promise
    /// holds for it in the strongest form the promise has: mesimon does not
    /// add a token, and here it does not author one either.
    fn prompt_session(&mut self, ticket: ulid::Ulid, text: String, queued: bool) -> Response {
        // Blank in, nothing out: an empty paste would press Enter on a turn
        // the user never wrote.
        let Some(text) = mesimon_core::command::sanitize_prompt(&text) else {
            return Response::Err { message: "nothing to send".into() };
        };
        if queued {
            return self.enqueue_ask(ticket, text);
        }
        // Sending now while an ask waits is the user talking to the agent
        // ahead of it: the waiting words are theirs to drop, and they just
        // did (the TUI's status says so).
        self.forget_queued(ticket, "queued_ask_dropped", "local");
        let Some(id) = self.prompt_target(ticket) else {
            return self.prompt_sleeping(ticket, text);
        };
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        match self.backend.paste_text(&rec.sid16(), &text) {
            // The board's own picture of the session is now a turn behind:
            // the record still says `Idle` until the agent's `UserPromptSubmit`
            // hook lands, and that is the hook's to say, not ours. What we
            // broadcast is the feed entry above — the card catches up when
            // the agent does, the same way it does for a prompt typed in the
            // pane.
            Ok(()) => Response::Ok,
            Err(e) => Response::Err { message: format!("could not deliver: {e}") },
        }
    }

    // ------------------------------------------------------------ queued asks

    /// Tickets holding a CHECKOUT: a working claude with that cwd
    /// (`quiet::working_tickets` — a shell never counts), a paste of ours
    /// still owed its ack, and the sessions of a deleted ticket riding out
    /// the grace band there, judged by their frozen state (conservative:
    /// they keep running for 30 s in the same tree).
    fn checkout_holders(&self, cwd: &str) -> Vec<ulid::Ulid> {
        let inflight: std::collections::HashSet<ulid::Ulid> =
            self.inflight.keys().copied().collect();
        let mut out = mesimon_core::quiet::working_tickets(&self.board, &inflight, Some(cwd));
        for g in self.grace.values() {
            if g.sessions.iter().any(|s| s.cwd == cwd && mesimon_core::quiet::is_working(s)) {
                out.push(g.ticket.id);
            }
        }
        out
    }

    /// Every working ticket on the board — the merge train's gate.
    fn board_busy(&self) -> Vec<ulid::Ulid> {
        let inflight: std::collections::HashSet<ulid::Ulid> =
            self.inflight.keys().copied().collect();
        let mut out = mesimon_core::quiet::working_tickets(&self.board, &inflight, None);
        for g in self.grace.values() {
            if g.sessions.iter().any(mesimon_core::quiet::is_working) {
                out.push(g.ticket.id);
            }
        }
        out
    }

    fn keys_of(&self, ids: &[ulid::Ulid]) -> Vec<String> {
        ids.iter().filter_map(|t| self.board.ticket(*t)).map(|t| t.short_key.clone()).collect()
    }

    /// The board's Shift+Enter with the field at `queued`: park the words
    /// until no claude sharing this ticket's checkout is mid-turn. A pane is
    /// required — a Sleeping claude would have to be woken at delivery time
    /// into the very checkout just judged quiet, with `resume_session`'s own
    /// refusals arriving seconds later — and so is the shared checkout: a
    /// worktree's checkout is its own, and there the toggle is not offered.
    /// One entry per ticket (a second replaces the words in place, keeping
    /// the turn), then a drain: a checkout already quiet sends at once.
    fn enqueue_ask(&mut self, ticket: ulid::Ulid, text: String) -> Response {
        let Some(rec) = self.board.pane_target(ticket) else {
            return Response::Err {
                message: "a queued ask needs an awake claude — wake it first".into(),
            };
        };
        let (session, cwd) = (rec.id, rec.cwd.clone());
        let shared = self
            .board
            .ticket(ticket)
            .is_some_and(|t| t.workspace_strategy() == WorkspaceStrategy::SharedCheckout)
            && !self.worktrees.contains_key(&ticket);
        if !shared {
            return Response::Err {
                message: "a worktree ticket's checkout is its own — send it now".into(),
            };
        }
        let now = now_ms();
        if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == ticket) {
            q.text = text;
            q.session = session;
            self.feed.board("local", "queued_ask_replaced", Some(ticket));
        } else {
            self.queued.push(QueuedAsk { ticket, session, cwd: cwd.clone(), text, queued_at: now });
            self.feed.board("local", "queued_ask", Some(ticket));
        }
        self.drain_queue(now);
        self.broadcast();
        if self.queued.iter().any(|q| q.ticket == ticket) {
            let behind = self.keys_of(&self.checkout_holders(&cwd));
            Response::Queued { behind }
        } else if self.inflight.contains_key(&ticket) {
            Response::Ok
        } else {
            Response::Err { message: "could not deliver".into() }
        }
    }

    /// Paste the first waiting ask of every QUIET checkout — one per
    /// checkout per pass, since the paste itself makes it busy again (the
    /// in-flight marker, until the agent's `UserPromptSubmit` says so). The
    /// target must still be the pane the ask was queued at: a fresh spawn in
    /// the seat is a different conversation.
    fn drain_queue(&mut self, now: u64) -> bool {
        let mut changed = false;
        let mut seen: Vec<String> = Vec::new();
        let mut i = 0;
        while i < self.queued.len() {
            let cwd = self.queued[i].cwd.clone();
            if seen.contains(&cwd) || !self.checkout_holders(&cwd).is_empty() {
                seen.push(cwd);
                i += 1;
                continue;
            }
            seen.push(cwd);
            let q = self.queued.remove(i);
            changed = true;
            match self.board.pane_target(q.ticket) {
                Some(rec) if rec.id == q.session => {
                    let sid = rec.sid16();
                    match self.backend.paste_text(&sid, &q.text) {
                        Ok(()) => {
                            self.inflight
                                .insert(q.ticket, (now + INFLIGHT_MS, "queued_ask_delivered"));
                            self.feed.board("automation", "queued_ask_sent", Some(q.ticket));
                        }
                        Err(_) => {
                            self.feed.board("automation", "queued_ask_failed", Some(q.ticket))
                        }
                    }
                }
                _ => {
                    self.feed.board("automation", "queued_ask_dropped_target_gone", Some(q.ticket))
                }
            }
        }
        changed
    }

    /// Drop the asks whose ticket or target is gone: the ticket deleted or
    /// archived, the pane dead or parked, or a different session in the
    /// seat. The explicit cancels (`kill_session`, `sleep_one`,
    /// `delete_ticket`, a send-now) name their reason; this is the net.
    fn sweep_queue(&mut self) -> bool {
        let stale: Vec<ulid::Ulid> = self
            .queued
            .iter()
            .filter(|q| {
                let ticket_gone = self.board.ticket(q.ticket).is_none_or(|t| t.is_archived());
                let target_gone =
                    self.board.pane_target(q.ticket).is_none_or(|s| s.id != q.session);
                ticket_gone || target_gone
            })
            .map(|q| q.ticket)
            .collect();
        for t in &stale {
            self.forget_queued(*t, "queued_ask_dropped_target_gone", "automation");
        }
        !stale.is_empty()
    }

    /// A paste that never got its `UserPromptSubmit` within `INFLIGHT_MS`:
    /// the words sit in the box, as an ordinary spawn leaves them, and the
    /// checkout stops counting as busy for them.
    fn expire_inflight(&mut self, now: u64) -> bool {
        let dead: Vec<ulid::Ulid> =
            self.inflight.iter().filter(|(_, (until, _))| *until <= now).map(|(t, _)| *t).collect();
        for t in &dead {
            self.inflight.remove(t);
            self.feed.board("automation", "paste_unacked", Some(*t));
        }
        !dead.is_empty()
    }

    /// A prompt reached the ticket's agent. Ours in flight — the ack; or the
    /// user's own while an ask waited — which drops the ask: they talked to
    /// the agent ahead of it, and the parked words may now be moot.
    fn ack_owed(&mut self, ticket: ulid::Ulid) -> bool {
        let mut changed = false;
        if let Some((_, word)) = self.inflight.remove(&ticket) {
            self.feed.board("automation", word, Some(ticket));
            changed = true;
        }
        changed | self.forget_queued(ticket, "queued_ask_dropped_by_hand", "local")
    }

    fn forget_queued(&mut self, ticket: ulid::Ulid, why: &str, actor: &str) -> bool {
        let before = self.queued.len();
        self.queued.retain(|q| q.ticket != ticket);
        if self.queued.len() != before {
            self.feed.board(actor, why, Some(ticket));
            return true;
        }
        false
    }

    /// A blank Enter in the reopened field: the person dropped the ask.
    fn drop_queued_ask(&mut self, ticket: ulid::Ulid) -> Response {
        if self.forget_queued(ticket, "queued_ask_dropped", "local") {
            self.broadcast();
            Response::Ok
        } else {
            Response::Err { message: "nothing queued on this ticket".into() }
        }
    }

    /// The same key at a SLEEPING claude wakes it and asks (2026-09-04, user:
    /// "ask claude on sleeping agent auto wakes it for the user"). Before this
    /// the key was inert there — a parked agent has no box to type into — and
    /// the user pressed `c`, waited for the pane, came back and asked; three
    /// gestures for one sentence. The wake is `resume_session`'s, untouched
    /// (the double-resume guard, the fresh conversation where the transcript
    /// is gone, the cwd refusal), so `Response::Spawned` travels back with
    /// `fresh` and the board can say which it was. The words are PARKED, not
    /// typed: the pane does not exist yet, and what reaches it is decided on
    /// the `SessionStart` edge the way the composer's Enter is
    /// (`deliver_pending_submit`). `pending_submit` is set for the same
    /// reason the composer sets it — the launching arc on the card is how the
    /// user watches the ask land — and the seat is still ONE claude: a wake
    /// re-enters the record, it never mints a second.
    fn prompt_sleeping(&mut self, ticket: ulid::Ulid, text: String) -> Response {
        let Some(id) = self
            .board
            .live_claude(ticket)
            .filter(|s| matches!(s.state, SessionState::Sleeping))
            .map(|s| s.id)
        else {
            return Response::Err {
                message: "no live claude session on this ticket — start or wake one first".into(),
            };
        };
        let resp = self.resume_session(id, false);
        if let Response::Spawned { .. } = &resp {
            self.pending_prompt.insert(id, Parked { text, brief: false });
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.pending_submit = true;
            }
            self.persist_and_notify();
        }
        resp
    }

    fn restore_ticket(&mut self, id: ulid::Ulid) -> Response {
        let Some(mut g) = self.grace.remove(&id) else {
            return Response::Err { message: "grace window expired".into() };
        };
        // Bodies back before the metadata that lists them, the same order
        // `write_note` keeps: an orphan file is harmless, a listed note with
        // no file is the editor's dead end. One whose body could not be read
        // at delete time is dropped from the list rather than restored as
        // exactly that.
        for (nid, text) in &g.notes {
            let _ = store::save_note(&self.paths, &g.ticket.short_key, *nid, text);
        }
        let carried: Vec<ulid::Ulid> = g.notes.iter().map(|(nid, _)| *nid).collect();
        g.ticket.notes.retain(|n| carried.contains(&n.id));
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
            .with_ticket(id, |t| {
                t.archived =
                    Some(Archived { at, by: "local".into(), until: None, needs_you: false })
            })
            .unwrap_or(Response::Err { message: "no such ticket".into() });
        // Re-price now — a taken offer must not linger until the next bucket.
        self.archive_cache = self.archive_figures();
        resp
    }

    /// Snooze: an archive with a deadline (T-74). `archive_ticket`'s gates
    /// plus a deadline that has not already passed, since a snooze that
    /// wakes on the next tick is a refusal nobody could see — but where the
    /// archive REFUSES over an awake session, a snooze puts it to sleep
    /// first (2026-09-04, user request): a snooze says "not now", and an
    /// agent that has stopped is exactly what `x` would have parked before
    /// the `z`. Only what `x` would accept goes — `sleep_eligible` without
    /// the bulk sweep's age floor, since a `z` is as deliberate as an `x` —
    /// and it is all-or-nothing: every pane on the ticket is judged BEFORE
    /// any is signalled, so a claude still working (or waiting on the user,
    /// or pinned awake, or a shell with a live child) holds the ticket on
    /// the board with nothing on it touched, in words that name it.
    /// `wake_snoozed` on the tick wheel is the other half; the sessions
    /// stay asleep when the ticket returns, and `c` wakes them.
    fn snooze_ticket(&mut self, id: ulid::Ulid, until: u64, needs_you: bool) -> Response {
        match self.board.ticket(id) {
            None => return Response::Err { message: "no such ticket".into() },
            Some(t) if t.is_archived() => {
                return Response::Err { message: "already archived".into() }
            }
            Some(_) => {}
        }
        if until <= now_secs() {
            return Response::Err { message: "snooze deadline is already past".into() };
        }
        let now = now_ms();
        let awake: Vec<(uuid::Uuid, SessionKind)> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.ticket == id && s.state.has_pane())
            .map(|s| (s.id, s.kind))
            .collect();
        for (sid, kind) in &awake {
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == *sid) else { continue };
            if let Err(why) = self.sleep_eligible(rec, now, false) {
                let who = match kind {
                    SessionKind::Claude => "claude",
                    SessionKind::Bash => "shell",
                };
                return Response::Err { message: format!("{who} still awake — {why}") };
            }
        }
        for (sid, _) in &awake {
            // Judged eligible a moment ago on this same thread; a refusal
            // here would be a record that vanished between the two loops.
            let _ = self.sleep_one(*sid, false);
        }
        if !awake.is_empty() {
            self.persist_sessions();
        }
        let at = now_iso();
        let resp = self
            .with_ticket(id, |t| {
                t.woke_at = None;
                t.archived = Some(Archived {
                    at,
                    by: "local".into(),
                    until: Some(format!("@{until}")),
                    needs_you,
                });
            })
            .unwrap_or(Response::Err { message: "no such ticket".into() });
        self.archive_cache = self.archive_figures();
        resp
    }

    /// The cursor rested on a ticket a snooze woke lit: the mark comes off.
    /// A no-op — no write, no broadcast — on a ticket that wears none, so
    /// the TUI can send it whenever it likes.
    fn seen_ticket(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            None => Response::Err { message: "no such ticket".into() },
            Some(t) if !t.is_woke() => Response::Ok,
            Some(_) => self
                .with_ticket(id, |t| t.woke_at = None)
                .unwrap_or(Response::Err { message: "no such ticket".into() }),
        }
    }

    /// Take a ticket off the merge train, or put it back (T-227). The flag
    /// is the ticket's and the planner reads it, so the next `pending_items`
    /// already lists nothing for it; a person's gesture on the ticket also
    /// clears the train's memory of it, the way a hand `m` does. A no-op
    /// when nothing changes — no write, no broadcast.
    fn set_manual_merge(&mut self, id: ulid::Ulid, on: bool) -> Response {
        match self.board.ticket(id) {
            None => return Response::Err { message: "no such ticket".into() },
            Some(t) if t.manual_merge == on => return Response::Ok,
            Some(_) => {}
        }
        self.train.hand_touched(id);
        self.with_ticket(id, |t| t.manual_merge = on)
            .unwrap_or(Response::Err { message: "no such ticket".into() })
    }

    /// Turn the agent tool surface on or off for this board (T-217).
    ///
    /// Only ever the whole board, only ever a person: `mcp::agent_allows`
    /// denies the command, so nothing an agent says can reach here. What
    /// changes is what the NEXT spawn or wake is built with — a running pane's
    /// argv was fixed at exec and nothing can revise it, which is the sentence
    /// the Settings row spends its detail on.
    fn set_mcp_tools(&mut self, on: bool) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.mcp_tools == on {
            return Response::Ok;
        }
        self.board.mcp_tools = on;
        self.persist_and_notify();
        Response::Ok
    }

    /// The agent brief's switch (T-224): `brief::TEXT` on the argv of every
    /// claude this board starts from now on, through `brief::FLAG`. Same
    /// shape and same reach as `set_mcp_tools` — the NEXT spawn or wake is
    /// what changes, a running pane's argv was fixed at exec — and it is
    /// honoured only while the tools are on (`claude_argv`), because the
    /// sentence names `get_ticket`.
    fn set_system_prompt(&mut self, on: bool) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.system_prompt == on {
            return Response::Ok;
        }
        self.board.system_prompt = on;
        self.persist_and_notify();
        Response::Ok
    }

    /// "Never ask again" on the agent-brief offer: a stamp on the board, so
    /// the chip and its menu row stop. Needs `columns.toml` writable, or the
    /// offer would be back on the next restart and look like it had failed.
    /// Declining for now and copying never arrive here; they write nothing.
    fn ignore_brief_offer(&mut self) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        self.board.claude_md_ignored = true;
        self.persist_and_notify();
        Response::Ok
    }

    /// The tick wheel's half of a snooze: every ticket whose deadline has
    /// passed comes back — at the TOP of its column, the way every automatic
    /// move lands (the return is fresh news), with its age restarted, and lit
    /// if the snooze asked for it. Restore-by-hand's rules travel with it:
    /// the move gate forgets the ticket, and a column that vanished while it
    /// slept falls back to the first. One write per ticket and NO broadcast
    /// here — `on_tick` fires one for everything the bucket changed. Returns
    /// whether anything woke.
    fn wake_snoozed(&mut self, now: u64) -> bool {
        let due: Vec<ulid::Ulid> = self
            .board
            .tickets
            .iter()
            .filter(|t| t.snooze_until_secs().is_some_and(|until| until <= now))
            .map(|t| t.id)
            .collect();
        if due.is_empty() {
            return false;
        }
        for id in due {
            let Some(t) = self.board.ticket(id) else { continue };
            let needs_you = t.archived.as_ref().is_some_and(|a| a.needs_you);
            let col = if self.board.columns.iter().any(|c| c.name == t.column) {
                t.column.clone()
            } else {
                match self.board.sorted_columns().first() {
                    Some(c) => c.name.clone(),
                    None => continue,
                }
            };
            // `column_tickets` never lists an archived ticket, so the order is
            // computed against the board it is about to rejoin.
            let order = self.order_within(&col, id, &Position::Top);
            self.moves.forget(id);
            self.train.forget(id);
            let stamp = now_iso();
            if let Some(t) = self.board.ticket_mut(id) {
                t.archived = None;
                t.column = col;
                t.order = order;
                t.entered_at = Some(stamp.clone());
                t.woke_at = needs_you.then_some(stamp);
                let t = t.clone();
                let _ = store::save_ticket(&self.paths, &t);
            }
            self.feed.board("automation", "snooze_woke", Some(id));
        }
        self.archive_cache = self.archive_figures();
        true
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
        self.train.forget(id);
        let now = now_iso();
        self.with_ticket(id, |t| {
            t.archived = None;
            if let Some((col, order)) = fallback {
                if t.column != col {
                    t.entered_at = Some(now);
                }
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
        // One claude per ticket (2026-09-02). Everything that has to pick
        // "the" agent of a ticket — the board's prompt, the merge notice,
        // Enter's focus, automove, the card glyph — assumed one, and with two
        // they picked the first in spawn order while the second did the
        // work, automove ping-ponged the column between their turns, and both
        // edited one worktree with no coordination. Parallelism inside a
        // ticket is the agent's own subagents; a second seat is a shell.
        // Gated on `is_live`, so a parked claude also holds the seat — `c`
        // wakes it rather than starting a rival beside it. Only NEW records
        // are refused: a record that already exists resumes as it did, so a
        // board that predates this keeps every session it has.
        if kind == SessionKind::Claude {
            if let Some(held) = self.board.live_claude(ticket) {
                let verb =
                    if matches!(held.state, SessionState::Sleeping) { "wake" } else { "focus" };
                return Response::Err {
                    message: format!("ticket already has a claude session — {verb} it instead"),
                };
            }
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
        let launch = self.launch(&argv, &self.session_vars(ticket, &cwd));
        if let Err(e) = self.backend.spawn(&rec.sid16(), &cwd, &launch) {
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
                    // And when mesimon is the one pressing Enter, the whole
                    // brief goes with it (T-224, 2026-09-05): the ticket's
                    // description — the user's own words, written for this
                    // ticket — is parked to be PASTED under the title on the
                    // first tick after `SessionStart`, the way a wake-and-ask
                    // delivers its words (never typed ahead: canonical-mode
                    // input keeps 1 KiB). Agents skipped `get_ticket` however
                    // CLAUDE.md asked; a prompt cannot be skipped. The plain
                    // Enter road stays title-only — the user is about to edit
                    // the box, and a 32 KiB description is not editable there.
                    if submit_prompt {
                        if let Some(body) = self.description_body(ticket) {
                            self.pending_prompt
                                .insert(id, Parked { text: format!("\n\n{body}"), brief: true });
                        }
                    }
                }
            }
        }
        self.machines.insert(id, Machine::new(rec.state.clone(), now_ms()));
        self.board.sessions.push(rec);
        self.lock_worktree(ticket, id);
        self.persist_and_notify();
        Response::Spawned { id, fresh: false }
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
        self.base_tip = worktree::branch_tip(&self.paths.repo_root, &base);
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
        let (id, state, ticket) = (rec.id, rec.state.clone(), rec.ticket);
        self.machines.insert(id, Machine::new(state, now_ms()));
        if let Some(sid) = reap {
            let _ = self.backend.signal_session(&sid);
            self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        }
        // The user ended the session an ask was waiting for.
        self.forget_queued(ticket, "queued_ask_dropped", "local");
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
        let mut argv = vec![claude, "--settings".into(), settings.display().to_string()];
        // The board's own switch (T-217). Off means the flag is not there at
        // all — not an empty config, not a server with no tools: a session
        // that was never told about mesimon cannot be told about it later,
        // and that is the whole of what "off" is worth promising.
        if self.board.mcp_tools {
            argv.push("--mcp-config".into());
            argv.push(self.mcp_config_json(id));
        }
        // The agent brief (T-224): opt-in, and only beside the tools it
        // names — a system prompt telling the model to call a tool it does
        // not have would be the lie the switch exists to avoid.
        if self.brief_on() {
            argv.push(mesimon_core::brief::FLAG.into());
            argv.push(mesimon_core::brief::TEXT.into());
        }
        argv.push(identity_flag.to_string());
        argv.push(identity_value.to_string());
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

    /// The brief rides the argv only while BOTH switches are on: the sentence
    /// names `get_ticket`, and a session with no tools has nothing to call.
    fn brief_on(&self) -> bool {
        self.board.mcp_tools && self.board.system_prompt
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
                    //
                    // And DROP it where the board's switch is off (T-217): a
                    // wake is the road a session takes to pick the switch up,
                    // since a live pane's argv was fixed at exec.
                    let _ = it.next();
                    if self.board.mcp_tools {
                        argv.push("--mcp-config".into());
                        argv.push(self.mcp_config_json(rec.id));
                    }
                } else if a == mesimon_core::brief::FLAG {
                    // The brief, on the same two rules (T-224): regenerated
                    // rather than replayed, so a binary whose TEXT moved is
                    // what a wake says; and dropped where the switch is off.
                    let _ = it.next();
                    if self.brief_on() {
                        argv.push(mesimon_core::brief::FLAG.into());
                        argv.push(mesimon_core::brief::TEXT.into());
                    }
                } else {
                    argv.push(a.clone());
                }
            }
            // The switch travels both ways. A record born while the tools
            // were off carries no flag to rewrite, so turning them back on
            // would reach only sessions spawned afterwards — and "wake it to
            // pick the setting up" would be true in one direction and a lie
            // in the other. Inserted after `--settings <path>` so the argv
            // reads the way `claude_argv` builds it.
            if self.board.mcp_tools && !argv.iter().any(|a| a == "--mcp-config") {
                let at = argv
                    .iter()
                    .position(|a| a == "--settings")
                    .map(|i| (i + 2).min(argv.len()))
                    .unwrap_or(argv.len().min(1));
                argv.splice(at..at, ["--mcp-config".to_string(), self.mcp_config_json(rec.id)]);
            }
            // And the brief both ways too, after the tools it names.
            if self.brief_on() && !argv.iter().any(|a| a == mesimon_core::brief::FLAG) {
                let at = argv
                    .iter()
                    .position(|a| a == "--mcp-config")
                    .map(|i| (i + 2).min(argv.len()))
                    .unwrap_or(argv.len().min(1));
                argv.splice(
                    at..at,
                    [mesimon_core::brief::FLAG.to_string(), mesimon_core::brief::TEXT.to_string()],
                );
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
        // Nothing to come back to? Then "resume" and "start fresh" have the
        // SAME outcome — no conversation is lost either way — and refusing was
        // pure friction: the record could never be entered again, and the row
        // went on offering `enter resume` forever. So start a fresh
        // conversation in the same record instead, and say so.
        //
        // The conversation gets a NEWLY MINTED id rather than reusing the
        // record's own. Claude has already been handed `rec.id` once, and
        // whether it will accept that id a second time is not something this
        // code knows — a fresh uuid cannot collide by construction. Nothing
        // downstream cares: mesimon's identity is `rec.id` and travels in the
        // `--settings` and `--mcp-config` blobs (D24 — identity is never
        // discovered), so hook routing and the MCP principal are untouched.
        // `claude_session_id` is the field that already exists for exactly
        // this — "the conversation this record hosts is not its own uuid" —
        // and the in-app `/resume` relearn writes it the same way.
        let fresh = self.resume_transcript_missing(rec, claude_id).then(uuid::Uuid::new_v4);
        let argv = match fresh {
            Some(new_id) => match self.claude_argv(rec.id, "--session-id", &new_id.to_string()) {
                Ok(a) => a,
                Err(message) => return Response::Err { message },
            },
            None => match self.resume_argv(rec) {
                Ok(a) => a,
                Err(message) => return Response::Err { message },
            },
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
        let launch = self.launch(&argv, &self.session_vars(ticket, &cwd));
        if let Err(e) = self.backend.spawn(&sid, &cwd, &launch) {
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
            self.pending_prompt.remove(&id);
            if let Some(new_id) = fresh {
                // Point the record at the conversation it is actually hosting
                // now. The stale path would otherwise send the NEXT resume
                // back to the id that had no transcript — the dead end, one
                // wake later.
                rec.claude_session_id = Some(new_id);
                rec.transcript_path = None;
            }
        }
        self.tails.remove(&id); // hooks own the state from here
        self.probe_stage.remove(&id);
        self.machines.insert(id, Machine::new(SessionState::Spawning, now));
        Response::Spawned { id, fresh: fresh.is_some() }
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
        // The user parked the claude an ask was waiting for: a queued ask
        // needs an awake pane, and waking it later against their gesture is
        // not what they asked for.
        if let Some(t) = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket) {
            self.forget_queued(t, "queued_ask_dropped", "local");
        }
        Ok(())
    }

    /// A Claude session the user left on purpose is PARKED, not buried.
    ///
    /// Ctrl+C-out, `/exit` and Ctrl+D end the process; they do not end the
    /// conversation. `claude --resume` brings it back by exactly the road
    /// `wake_session` already drives, so recording it as a corpse asked the
    /// user to learn a second gesture for a state that is `Sleeping` in
    /// everything but name — no process, no pane, one key from running. It
    /// also cost them the things `is_live()` gates: the ticket's worktree
    /// lock was released under a session that was coming back.
    ///
    /// The scope is deliberately narrow — only what `resume_session` will
    /// actually accept afterwards:
    ///
    /// * `UserQuit` only, which is where BOTH clean-exit roads land
    ///   (`SessionEnd{prompt_input_exit}` and `pane-died` status 0), so
    ///   whichever wins the race parks. `Crashed` keeps its error mark (a
    ///   nonzero exit is worth seeing and is still resumable with Enter),
    ///   `LoggedOut` would wake into an auth wall, and `Cleared`/`Resumed`/
    ///   `Killed`/`Dismissed` are not deaths of this shape.
    /// * argv non-empty: an observe-only adopted record has no pane of ours
    ///   and nothing to replay.
    /// * the SAME transcript predicate `resume_session` refuses on. A session
    ///   Ctrl+C-ed before its first prompt wrote no conversation, and parking
    ///   it would mint a sleeper that can never wake. That one really did
    ///   just end.
    ///
    /// Re-minting the machine as `Sleeping` is what makes the aftermath
    /// harmless: the latch swallows the pane-died that follows the hook (or
    /// the SessionEnd that follows pane-died), so the park cannot be flipped
    /// back into a corpse by its own echo.
    fn park_on_exit(&mut self, id: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        if rec.kind != SessionKind::Claude
            || rec.argv.is_empty()
            || !matches!(rec.state, SessionState::Exited { reason: ExitReason::UserQuit })
        {
            return false;
        }
        let claude_id = rec.claude_session_id.unwrap_or(rec.id);
        if self.resume_transcript_missing(rec, claude_id) {
            return false;
        }
        let (sid, transcript) = (rec.sid16(), rec.transcript_path.clone());

        // Sleep's B-A22 copy, same reason and same place: the conversation
        // belongs to Claude's own store, and this is our snapshot of it.
        if let Some(t) = &transcript {
            let dir = self.paths.transcripts_dir();
            if std::fs::create_dir_all(&dir).is_ok() {
                let _ = std::fs::copy(t, dir.join(format!("{id}.jsonl")));
            }
        }

        let now = now_ms();
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.state = SessionState::Sleeping;
            rec.confidence = Confidence::High;
            rec.waiting_since = None;
            rec.state_changed_at = Some(now);
            rec.detail = None;
        }
        self.machines.insert(id, Machine::new(SessionState::Sleeping, now));
        self.tails.remove(&id);
        self.probe_stage.remove(&id);
        // No SIGTERM: the process left on its own. The pane is only still
        // standing because remain-on-exit is holding the corpse, so hand it
        // to the same reaper sleep uses rather than killing it inline.
        self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        let snapshot = self.board.sessions.iter().find(|s| s.id == id).cloned();
        if let Some(s) = snapshot {
            self.feed.session_state(
                &s,
                &SessionState::Exited { reason: ExitReason::UserQuit },
                None,
            );
        }
        true
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
                let launch = self.launch(&argv, &self.session_vars(ticket, &cwd));
                if let Err(e) = self.backend.spawn(&sid, &cwd, &launch) {
                    return Response::Err { message: format!("wake spawn failed: {e}") };
                }
                let now = now_ms();
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.state = SessionState::Running;
                    rec.state_changed_at = Some(now);
                }
                self.machines.insert(id, Machine::new(SessionState::Running, now));
                Response::Spawned { id, fresh: false }
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
        // Sessions in the attention set plus tickets a snooze woke lit —
        // the header chip's number, so the two never disagree.
        let needs_you = self.board.needs_you_count();
        // The user is looking at this pane: a `!1` that means "the session
        // you're inside" is noise, so the chip only shows when somewhere
        // ELSE needs them too.
        let only_self = needs_you == 1 && queue.first().is_some_and(|s| s.id == focused);
        let attn = if needs_you > 0 && !only_self {
            // Painted chip, not bare fg: `noreverse` alone drops the segment
            // to the terminal's default background (illegible on light
            // terminals). Graphite's attn pair (06 §2.2) — legibility is
            // internal to the chip, so it needs no flavor detection here;
            // tmux maps the hex down to 256/16 colours itself. The daemon
            // is theme-blind (06 §2.9) — five themes exist in the TUI and
            // this chip wears graphite's pair on all of them.
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
            if let Err(e) = self.backend.spawn(GATE_SESSION, &self.paths.repo_root, &argv) {
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
/// The worktree flags' (and the train's) cadence, in ticks: `RSS_TICKS`
/// unless `MESIMON_WT_REFRESH_TICKS` says otherwise — a test seam, since an
/// e2e cannot wait 10 s a step.
fn wt_refresh_ticks() -> u64 {
    std::env::var("MESIMON_WT_REFRESH_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&t| t > 0)
        .unwrap_or(RSS_TICKS)
}

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

/// Test seam only — e2e cannot spend a real minute per quiet verdict.
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

/// How much of the description rides `get_ticket`. The whole of it is one
/// `read_note` away; this keeps a routine call from carrying 32 KiB.
const AGENT_DESCRIPTION_MAX_BYTES: usize = 4096;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_iso() -> String {
    // Seconds precision is enough for created_at; avoid a chrono dependency.
    format!("@{}", now_secs())
}
