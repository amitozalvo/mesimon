//! The daemon core: one writer thread owns board state (D22); client threads
//! forward typed envelopes to it and write back responses. Every mutation calls
//! `authorize()` (D32c) even though v0.1 always allows.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use mesimon_backend_tmux::TmuxBackend;
use mesimon_core::attention::{self, Change, EndKind, Machine, Signal, StartSource};
use mesimon_core::board::{
    agent_reason_word, agent_state_word, bash_mode_foreground, foreground_of, sanitize_tag,
    AgentProvider, AgentTools, Archived, Board, Confidence, CrownMode, ExitReason, PickedUp,
    Provenance, Reason, SessionKind, SessionRecord, SessionState, StopReason, Tag, TagRef, Ticket,
    UnknownReason, WorkspaceStrategy, PICKED_AT_DESK, PICKED_BY_AGENT,
};
use mesimon_core::command::{
    AgentAutomoveView, AgentBackgroundView, AgentBoardView, AgentMergeView, AgentNeedsYouView,
    AgentQuestionView, AgentRepoView, AgentStateView, AgentTagView, AgentTicketRow,
    AgentTicketView, AskRoad, Command, CrownTouch, Deliver, DiffTarget, Envelope, Event,
    ExternalItem, GraceItem, MergeOutcome, Notice, Pending, PendingAction, Resources, Response,
    TerminalItem, WorktreeItem, WorktreeRepoItem, PROTOCOL_VERSION,
};
use mesimon_core::crown;
use mesimon_core::mcp;
use mesimon_core::reconcile::{reconcile, state_for};
use mesimon_core::road::{self, ModCommand, Road};
use mesimon_core::usage::{Provider, Wants};
use mesimon_core::{authorize, fracindex, Action, Decision, Principal, Resource};

use crate::agents::claude::user_default_mode;
use crate::agents::{AgentRecovery, LaunchContext, LaunchSpec, RecoveryChannel, RecoverySample};
use crate::feed::FeedWriter;
use crate::ingest::{self, HookFrame};
use crate::movegate::{MoveGate, Position};
use crate::paths::Paths;
use crate::store;
use crate::worktree::{self, Binding, BindingStatus};
use crownwake::{CrownWake, ProbeWhy, TurnAsk, TurnProbe, WakeCause};

/// Pictures a note references, prepared and ready to write: their metadata
/// and their bytes.
type Images = Vec<(mesimon_core::attachment::Attachment, Vec<u8>)>;

/// What a ticket is minted with — the argument to `Daemon::mint_full`, the
/// one builder every minting road shares (T-243).
pub(crate) struct Mint {
    pub column: String,
    pub title: String,
    /// Absent means the column's own default.
    pub workspace: Option<WorkspaceStrategy>,
    /// The ticket an agent filed this one from; a person's mint has none.
    pub from: Option<ulid::Ulid>,
    /// Registry references, spelled by the caller; one per group.
    pub tags: Vec<TagRef>,
    /// The description and the pictures it references, already prepared.
    /// Blank text is no note.
    pub note: Option<(String, Images)>,
    /// The composer's tier pick (T-443), a tier id; `None` is the default.
    pub tier: Option<String>,
    /// The Mesophon envelope a paired browser filed this from (T-497).
    pub envelope: Option<String>,
}

/// The composer's draft as `CreateTicketWithNote` carries it: what
/// `create_ticket_with_note` turns into a `Mint`.
pub(crate) struct Draft {
    pub column: String,
    pub title: String,
    pub workspace: Option<WorkspaceStrategy>,
    pub text: String,
    pub uploads: Vec<ulid::Ulid>,
    pub tags: Vec<TagRef>,
    pub tier: Option<String>,
}

impl Mint {
    /// A title alone in a column: the thin `CreateTicket` and the adoption
    /// of an external session.
    fn bare(column: String, title: String, workspace: Option<WorkspaceStrategy>) -> Self {
        Mint {
            column,
            title,
            workspace,
            from: None,
            tags: Vec::new(),
            note: None,
            tier: None,
            envelope: None,
        }
    }
}

mod attachments;
mod bridge;
mod crownledger;
mod crownplan;
mod crownwake;
mod mesophon;
mod shelf;
mod teamglue;
mod tiers;
mod trainhold;

const GRACE_SECS: u64 = 9;
/// How long a crown touch (T-411) rides the snapshot: long enough for the
/// board to light the card and leave its residue, short enough that a burst
/// of edits never accumulates. The feed is the record.
const CROWN_TOUCH_MS: u64 = 10_000;
/// How long a ticket keeps the crown's last touch for a phone (T-623): a
/// phone is glanced at, not watched, so it says what the crown did for the
/// hour Now keeps a stopped agent (T-560). One per ticket, in memory.
const CROWN_TOUCH_KEPT_MS: u64 = 3_600_000;
const GATE_SESSION: &str = "msmn-gate";

/// Who holds the exclusive focus token (D22): a ticket's session, or the
/// project's terminal (T-273) — a named tmux session on the private server
/// that belongs to no ticket, like the gate's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Session(uuid::Uuid),
    Terminal { ticket: Option<ulid::Ulid> },
}

/// The token and the CONNECTION that took it. A board hands it back when its
/// handover returns (`FocusEnd`/`TerminalEnd`) — but a board KILLED while it
/// is inside the pane never reaches that line: the terminal window closed
/// (cmd+W), the process crashed, somebody `kill`ed it. The token would then
/// strand until the daemon restarted and refuse every later attach with
/// "another session is focused" — a board that cannot open any of its own
/// sessions. So the holder is a `Weak` on that client's writer, the merge
/// train's shape (`train.rs`): the connection dying IS the release.
struct FocusHold {
    what: Focus,
    by: Weak<Mutex<UnixStream>>,
}

/// The terminal's tmux session name: one per DIRECTORY, so the root's and each
/// worktree's persist independently and an attach lands back in the same
/// shell. Keyed by the ticket's ULID, not its key: teardown runs after the
/// ticket left the board and the binding carries no key.
fn terminal_name(ticket: Option<ulid::Ulid>) -> String {
    match ticket {
        None => "msmn-term".to_string(),
        Some(id) => format!("msmn-term-{id}"),
    }
}

/// `terminal_name` read back off a tmux session name: `Some(None)` for the
/// checkout's terminal, `Some(Some(ticket))` for a ticket's, `None` for any
/// other session on the server. What lets the poll bucket list the
/// terminals alive without the daemon remembering which it opened (T-366:
/// a restart forgets, the panes do not).
fn terminal_ticket_of(name: &str) -> Option<Option<ulid::Ulid>> {
    if name == "msmn-term" {
        return Some(None);
    }
    let rest = name.strip_prefix("msmn-term-")?;
    ulid::Ulid::from_string(rest).ok().map(Some)
}

/// The `!` terminals alive in a pane snapshot, foreground unknown — the
/// seed at startup, before the first poll has read a command.
fn terminals_in(
    snap: &[mesimon_core::reconcile::PaneSnapshot],
) -> std::collections::BTreeMap<Option<ulid::Ulid>, Option<String>> {
    snap.iter()
        .filter(|p| !p.pane_dead)
        .filter_map(|p| terminal_ticket_of(&p.session_name))
        .map(|t| (t, None))
        .collect()
}
/// The deadline wheel (11 §11.7.4 settle timers need finer than 1 s).
const TICK_MS: u64 = 250;
/// Every this-many ticks, check the private tmux server wholesale — pane-died
/// cannot fire for a dead server, so this guard is load-bearing.
const SERVER_GUARD_TICKS: u64 = 60;
/// A pane that dies this soon after its spawn, with a status, died at launch
/// (T-690: 40 ms, exit 1, on every agent of a board macOS had cut off) —
/// the one edge the folder-access probe is asked on.
const LAUNCH_DEATH_MS: u64 = 1_000;
/// Observe-tier transcript polling cadence (2 s) — stat-then-read, adopted
/// hook-less sessions only.
const TAIL_POLL_TICKS: u64 = 8;
/// Gap between presses of an owed, unacknowledged Enter, and how many presses
/// to spend before giving up. T-5 measured the `UserPromptSubmit` ack at
/// ~94 ms, so 500 ms is a wide margin, and 10 attempts covers ~5 s of Claude
/// startup — well past the ~1 s at which a fresh pane starts reading.
const SUBMIT_RETRY_MS: u64 = 500;
const SUBMIT_ATTEMPTS: u8 = 10;
/// How long words parked for a Claude pane wait for its composer to paint
/// (T-570), from the `SessionStart` edge or a resend. The presses start
/// counting only once the paste is in, so this is the startup's own budget:
/// a fresh Claude paints well inside a second, three of them beside a cargo
/// build took several, and a pane that has shown none by now is a failed
/// start the card says out loud.
const COMPOSER_WAIT_MS: u64 = 30_000;
/// How long words parked for a Claude pane on the mod road wait for its
/// mod's bridge to poll (T-575), from the `SessionStart` edge, before they
/// take the paste road instead. A mod's bridge polls a few milliseconds after
/// `session.start`; one that has not by now never loaded or is wedged (T-588
/// measured one launch whose mod relayed nothing for its whole life), and the
/// words are the person's: they go the way an older Claude Code takes them.
const MOD_BRIDGE_WAIT_MS: u64 = 10_000;
/// How long a plan-dialog Enter of ours (T-420) has to be confirmed by the
/// harness's own hooks before the feed calls it unconfirmed.
const PLAN_ACCEPT_CONFIRM_MS: u64 = 8_000;
/// Passes (one per second) a flagged ask keeps looking for a dialog it
/// recognises on a pane at `Plan` before it drops the flag (T-420).
const PLAN_ACCEPT_TRIES: u8 = 10;
/// SIGTERM-to-kill-pane grace (docs/19 §1 kill ladder — never SIGKILL).
const REAP_GRACE: Duration = Duration::from_secs(5);
/// How often an orphaned Codex cleanup record (T-357) is re-checked for
/// known owners after a check refused. Each check forks one `ps`.
const CODEX_ORPHAN_RETRY: Duration = Duration::from_secs(15);
/// T-405: how long an unconfirmed Codex cleanup may own a LIVE ticket's
/// checkout before the sweep goes looking for its runtime. A deleted ticket
/// needs no such wait — the deletion, past its undo window, is the person's
/// acknowledgement (T-357) — but a live one says nothing, so the latch gets a
/// clock instead. Well past the reaper's grace and any late `stopped` report,
/// and far short of the hours the flag used to stand for.
const CODEX_CLEANUP_STALE_MS: u64 = 60_000;
/// D23 floor: a session younger than this in its current state never sleeps.
const SLEEP_MIN_AGE_MS: u64 = 60_000;
/// RSS aggregate refresh (10 s) — one `ps` fork, only while panes exist.
const RSS_TICKS: u64 = 40;
/// How often the tickets' transcripts are read for their cost (T-327) when
/// no turn's end asked sooner: 30 s, so a long turn's figure still moves.
const COST_TICKS: u64 = 120;
/// How often the archive offer is priced again when nothing on the board
/// changed (T-713): 60 s. What it waits for is a ticket ageing past the
/// hour, which a minute's lateness does not change; a board change prices
/// it at the next 1 s bucket (`archive_due`).
const ARCHIVE_TICKS: u64 = 240;
/// A sleep-safe ticket whose sessions have all been asleep this long feeds
/// the header's archive suggestion (same offer-not-action shape as sleep).
const ARCHIVE_SUGGEST_MS: u64 = 3_600_000;

/// A worktree waiting to go (12 §12.6.1). `sids` are the panes the reaper
/// still holds — the teardown waits for every one of them, since a
/// directory a live process has as cwd is never removed.
struct Teardown {
    ticket: ulid::Ulid,
    why: TeardownWhy,
    sids: Vec<String>,
}

/// Why a worktree is going, which is what decides what stays behind.
enum TeardownWhy {
    /// The ticket left through the grace band: the binding goes with it,
    /// and `discard` is the user's explicit `-D` on an unmerged branch.
    Deleted { discard: bool },
    /// The ticket was archived with its work landed (T-278): the directory
    /// goes, `branch -d` is tried, and the binding stays `Evicted` exactly
    /// while the branch survives, so a restore replays it.
    Archived,
}

/// A wake parked behind a worktree being rebuilt (T-278): the record's cwd
/// went with an archived worktree, and `on_provisioned` replays the resume.
/// `prompt` is a Shift+Enter's words on a sleeping claude, parked with it.
struct PendingResume {
    ticket: ulid::Ulid,
    session: uuid::Uuid,
    confirm: bool,
    prompt: Option<String>,
    /// The ticket's brief leads `prompt` (T-603): the wake of a seat that
    /// never took a prompt, asked with a blank field.
    brief: bool,
    /// Wake in plan mode (T-434), carried across the rebuild.
    plan: bool,
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
    attachments: Vec<(mesimon_core::attachment::Attachment, Vec<u8>)>,
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
/// Whether a frame ends a session's open permission wait (T-395): the
/// dialog's tool ran or failed (the person answered it in the pane), or the
/// turn, the conversation or the pane moved on. A subagent's frame is about
/// another dialog. The wait's stream is closed unanswered, so `mesimon
/// approve` prints nothing and exits, on the hook set's road and the mod's
/// alike: on the mod road that ends the mod's hold in
/// `classic.PermissionRequest`, which a person's own answer does not abort
/// (T-573 row 4, T-581).
fn releases_permission(frame: &HookFrame) -> bool {
    matches!(
        frame.event.as_str(),
        "UserPromptSubmit"
            | "Stop"
            | "StopFailure"
            | "SessionEnd"
            | "SessionStart"
            | "PaneDied"
            | "PostToolUse"
            | "PostToolUseFailure"
    ) && frame.payload.get("agent_id").is_none()
}

pub fn install_sigterm_handler() {
    // SAFETY: the handler touches one atomic and one async-signal-safe call.
    unsafe { libc::signal(libc::SIGTERM, on_sigterm as *const () as libc::sighandler_t) };
}

struct ClientReply {
    response: Response,
    /// Shutdown waits for the connection writer to put the response on wire.
    delivered: Option<Sender<()>>,
}

enum Msg {
    Request(Envelope, Sender<ClientReply>, Arc<Mutex<UnixStream>>),
    Hook(HookFrame),
    /// A frame the mod relayed (`road: mod`, T-574): since T-577 the frames
    /// a mod launch reports by, ingested as the hook set's were
    /// (`Daemon::on_mod_hook`).
    ModHook(HookFrame),
    /// The `auto` road's probe of the Claude Code on PATH came back.
    RoadProbed(crate::modroad::Probe),
    RemotePermission(HookFrame, UnixStream),
    CodexSnapshots(Vec<(uuid::Uuid, Option<crate::agents::codex::Snapshot>)>),
    /// The known-owner checks for orphaned Codex cleanup records came back
    /// (T-357): per record, its generation and whether every known native
    /// owner is provably gone. Off-thread because it forks `ps`.
    CodexOrphansChecked(Vec<(uuid::Uuid, Option<u64>, std::result::Result<(), String>)>),
    /// The wheel.
    Tick,
    /// A provisioning thread finished (M4): the binding, or the failing
    /// stage, and how long it took — the number the journal keeps (T-368).
    Provisioned(
        ulid::Ulid,
        std::result::Result<Binding, (String, String)>,
        Duration,
        Vec<worktree::InitReport>,
    ),
    /// The init script started in a leg of a provisioning worktree (T-614):
    /// the ticket page reads `init script running` for as long as it does.
    ProvisionInit(ulid::Ulid),
    /// A workspace provision landed one more leg (T-368): `(done, total)`,
    /// the snapshot's `7/19` while the binding is `provisioning`.
    ProvisionProgress(ulid::Ulid, u32, u32),
    /// A shell-environment capture finished. Off-thread because it forks the
    /// user's login shell and runs their rc files (`crate::shellenv`).
    ShellEnvCaptured(std::result::Result<crate::shellenv::ShellEnv, String>),
    /// The external census landed (T-437): every foreign transcript under
    /// the board's roots, not yet filtered against the sessions the board
    /// learned while the walk ran.
    ExternalScanned(Vec<ExternalItem>),
    /// The archive offer's worktrees, measured off the writer thread
    /// (T-679): each tree's path and the bytes it holds.
    TreesSized(Vec<(std::path::PathBuf, u64)>),
    /// A git sample of the board's own checkout landed (T-124), with the
    /// verdict of the fetch that preceded it when one was asked for.
    GitSampled(mesimon_core::command::RepoGit, crate::gitstatus::Fetched),
    /// A client's reader thread returned: its connection is closed. The
    /// merge train it may have armed disarms with it (2026-09-04).
    ClientGone(Arc<Mutex<UnixStream>>),
    /// The worktree flags sampled on a worker (T-216) — one `for-each-ref`
    /// and one `rev-list` per binding, per repository the legs live in
    /// (T-368), off the writer thread. The `u64` is the `wt_gen` the sample
    /// started under: a synchronous refresh in the meantime (a merge, a
    /// teardown) makes it stale, and it is dropped.
    WorktreeFlags(u64, Vec<worktree::RepoSample>),
    /// One ticket's own flags sample (T-678): a turn's end on a branch the
    /// train may take, and the number it started under.
    TicketFlags(ulid::Ulid, u64, Vec<worktree::RepoSample>),
    /// A worktree's teardown finished on a worker (T-561): `worktree remove
    /// --force` walks and unlinks the whole tree, `target/` included, and
    /// an `archive_all` queued a dozen of them for one tick, so the board
    /// hung 5–13 s while the writer did the unlinking. `archived` is the
    /// road that asked (the binding stays `Evicted` when the branch
    /// survived `branch -d`), `took` the number the journal keeps.
    TornDown {
        ticket: ulid::Ulid,
        archived: bool,
        branch_kept: bool,
        took: Duration,
    },
    /// A look at a worker's work at its turn's end (T-469): what decides
    /// whether the crown hears of it. Off-thread because it forks git.
    TurnProbed(TurnProbe),
    /// The relay executor finished a job (T-215). What it means is decided
    /// here, on the writer, in `teamglue`.
    Team(crate::team::sync::Done),
    Control(u64, crate::team::control_io::Event, Sender<()>),
    /// A phone's transcript page was read off this thread (T-626): the
    /// peer that asked, its grant, the command and the answer.
    TranscriptRead(String, mesimon_team::crypto::BoardId, u64, mesimon_core::mesophon::Reply),
    /// The conversations a shelf build read off the writer thread (T-698).
    ShelfRead(Box<shelf::Build>, Vec<shelf::PageRead>),
    /// A quota probe came back (T-327): whose, and what it said.
    UsageRead(Provider, crate::usage::Outcome),
    /// A pass over the tickets' transcripts landed (T-327): what each read
    /// found, to fold into the cost ledger.
    CostScanned(Vec<crate::cost::Done>),
}

pub struct Daemon {
    paths: Paths,
    board: Board,
    backend: TmuxBackend,
    grace: HashMap<ulid::Ulid, GraceEntry>,
    uploads: crate::attachments::Uploads,
    subscribers: Vec<Arc<Mutex<UnixStream>>>,
    focus: Option<FocusHold>,
    shutting_down: bool,
    /// `daemon.log`: started / stopping / stopped / slow turn (`journal`).
    journal: crate::journal::Journal,
    /// Each connection's Hello `client` string, by the writer `Arc`'s address,
    /// so a `Shutdown` can be journalled with who asked. Pruned on ClientGone.
    clients: HashMap<usize, String>,
    /// When `begin_shutdown` ran, so `stopped` can say how long the flush took.
    stop_started: Option<Instant>,
    /// The slowest stage of the turn in hand, for the slow-turn line: a
    /// tick's probes, or the provisioned turn's replay and flags (T-430).
    tick_slowest: (&'static str, Duration),
    /// Standing advisories about persisted state, rebuilt at startup and
    /// carried on every snapshot. Not transient: each one describes a
    /// condition still true on disk.
    notices: Vec<mesimon_core::command::Notice>,
    /// A server restart a person asked for (T-690): the panes were parked,
    /// and the server is killed once they are reaped or at this deadline.
    server_restart: Option<Instant>,
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
    /// `queue.json` (T-418): the same bar, for the same reason.
    queue_barred: bool,
    /// `started.json` (T-441): the same bar. The set filters either way.
    started_barred: bool,
    /// `costs.json` (T-327): the same bar. The ledger counts either way.
    costs_barred: bool,
    /// One attention machine per session, keyed by session UUID.
    machines: HashMap<uuid::Uuid, Machine>,
    /// Words mesimon owes a pane an ack for, by session — the ONE ledger
    /// (T-244) every paste road of the daemon's lands in: the composed
    /// spawn's Enter, the board's ask at a sleeping claude, a queued ask,
    /// the train's notice and rebase request, the crown's wake, a note's
    /// nudge, a Codex pane's parked paste. Transient, never persisted: a
    /// restart abandons the offer rather than typing into a pane it no
    /// longer understands, and the words go with it — never typed ahead
    /// into the pty, which is canonical-mode input capped at 1 KiB until the
    /// agent sets raw mode. `UserPromptSubmit` (or Codex's new-turn edge)
    /// settles an entry (`ack_owed`); the tick presses and expires the rest
    /// (`settle_owed`). A ticket with an entry counts as WORKING
    /// (`quiet::working_tickets`), which is what keeps a second paste out of
    /// the same checkout in the same pass.
    owed: HashMap<uuid::Uuid, Owed>,
    /// Asks parked until the ticket's CHECKOUT is quiet (2026-09-04, after
    /// five claudes in one checkout committed at once): the board's
    /// Shift+Enter with the field's toggle at `queued`. One entry per
    /// ticket, in BOARD order (`queue_order`), delivered by `drain_queue`
    /// when `checkout_holders` is empty — pasted into a pane, or, since
    /// T-294, waking the ticket's parked claude or starting one. The
    /// `Pane` entries are in memory for `owed`'s reason: a restart
    /// drops the words rather than pasting them into a pane it no longer
    /// understands, and the mark on the card goes with them. The `Start`
    /// and `Wake` entries ride `queue.json` (`askqueue`, T-418) and come
    /// back after a restart: a column's worth of parked starts died with an
    /// agent's `pkill` of the daemon, and nothing on the board said so.
    queued: Vec<QueuedAsk>,
    /// Panes mesimon pressed Enter into on their PLAN DIALOG (T-420), by
    /// session, with the ms deadline by which the harness's own hooks must
    /// have moved the record off `Plan`. The press is a person's — the
    /// board's Shift+Enter, or a queued ask they flagged — and the harness
    /// confirms it: Claude's `PostToolUse ExitPlanMode`, Codex's next turn.
    /// Past the deadline the feed says `plan_accept_unconfirmed` and the
    /// card keeps its `≡`, because the card never claims an approval the
    /// hooks did not see. Memory-only: a restart forgets a press in flight
    /// and the next daemon reads the pane as it is.
    plan_accept: HashMap<uuid::Uuid, u64>,
    /// Ticks on which a flagged ask found its pane at `Plan` but not
    /// showing a dialog it recognised (T-420). The dialog paints after the
    /// hook, so the first few are expected; past `PLAN_ACCEPT_TRIES` the
    /// flag is dropped, the feed says `plan_accept_unrecognised`, and the
    /// ask waits behind the `≡` the way an unflagged one does.
    plan_accept_tries: HashMap<uuid::Uuid, u8>,
    /// The merge train (2026-09-04): armed by a connection, what it asked,
    /// its fuse. See `crate::train`. Its open asks are kept in `train.json`
    /// (T-635).
    train: crate::train::Train,
    /// `train.json` could not be read, or a newer build wrote it.
    train_barred: bool,
    /// `train.json` as last written, so an unchanged one is not written again.
    train_written: String,
    /// `columns.toml`'s and `sessions.json`'s text as last written; a save
    /// whose text is the same writes nothing.
    columns_written: String,
    sessions_written: String,
    ticks: u64,
    feed: FeedWriter,
    /// Discovered foreign sessions (19 §4 tier 1). Never persisted; refreshed
    /// only on `RescanExternal` (the drawer opening).
    external: Vec<ExternalItem>,
    /// A census is walking `~/.claude` on a worker (T-437): it reads the head
    /// and tail of every transcript under every project — 1.2 s over 2,000
    /// files on the author's machine — which held the writer, so every
    /// client and every hook frame, for the whole walk.
    external_scanning: bool,
    /// A rescan asked for while one was walking: the answer in flight was
    /// started against an older `known` set, so one more walk follows it.
    external_rescan_wanted: bool,
    /// Every conversation a session mesimon spawned has held (T-441,
    /// `crate::started`): the census leaves these out after the record that
    /// held one is gone or has moved on.
    started: std::collections::HashSet<String>,
    /// What each ticket's agents have spent (T-327, `crate::cost`): every
    /// transcript they held, how far it is read, tokens by hour and model.
    costs: crate::cost::Ledger,
    /// A pass over the transcripts is on a worker.
    cost_scanning: bool,
    /// A turn ended: read the transcripts on the next bucket, not the clock.
    cost_due: bool,
    /// Provider-owned passive observation cursors; never persisted.
    recovery: HashMap<uuid::Uuid, Box<dyn AgentRecovery>>,
    /// A person has seen the unknown-cleanup warning for this exact generation.
    /// Never persisted or inherited by queued/automatic resume operations.
    cleanup_resume_offers: HashMap<uuid::Uuid, u64>,
    /// Panes SIGTERM'd and awaiting their grace-then-kill-pane (by sid16).
    reaping: HashMap<String, Instant>,
    /// (bytes, sessions seen) — `ps` aggregate, refreshed on the 10 s bucket.
    rss_cache: (u64, usize),
    /// Per-session slice of that aggregate, same bucket — feeds the sleep
    /// suggestion's "free ~X" figure.
    rss_by: HashMap<uuid::Uuid, u64>,
    /// The command each live shell record's pane is running (T-366), from
    /// the same fork as the titles. In memory ONLY: it is written into the
    /// snapshot's records (`SessionRecord::foreground`) and never into
    /// `sessions.json` — a persisted foreground would outlive the command.
    foregrounds: HashMap<uuid::Uuid, String>,
    /// The `!` terminals alive on the private server, by directory (`None`
    /// the checkout's), with the command each is running. Seeded from the
    /// startup snapshot, kept by the poll bucket, entered by `open_terminal`
    /// so the ghost row is on the rail the moment the user is back.
    terminals: std::collections::BTreeMap<Option<ulid::Ulid>, Option<String>>,
    /// (bytes, sessions) currently sleepable on sleep-safe tickets — the
    /// header suggestion, recomputed on the RSS bucket.
    reclaim_cache: (u64, usize),
    /// Tickets currently archive-suggestable — recomputed on the 1 s bucket
    /// when `archive_due`, else every `ARCHIVE_TICKS` (NOT the RSS bucket:
    /// refresh_rss early-returns when no pane exists, which is exactly the
    /// all-asleep scenario archive looks for).
    archive_cache: usize,
    /// The board changed since the offer was last priced: set by every
    /// broadcast, so a move, a sleep, a ticked box or a column's offers
    /// reach the header within the second, not the minute.
    archive_due: bool,
    /// The disk the archive offer frees (T-679): the sum of `tree_sizes`
    /// over the trees it would tear down, on the same 1 s bucket.
    archive_bytes: u64,
    /// Each priced tree's measured size, kept while it stays priced: a
    /// tree on the offer has sat untouched for an hour, so one walk holds.
    tree_sizes: HashMap<std::path::PathBuf, u64>,
    /// A walk is out; the next one waits for it.
    trees_sizing: bool,
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
    /// A parked Shift+Enter must still submit its prompt — and carry the
    /// words it was given (T-294) — when the worktree finally lands.
    pending_spawns: Vec<PendingSpawn>,
    /// Wakes parked behind provisioning (T-278), replayed beside the spawns.
    pending_resumes: Vec<PendingResume>,
    /// Each ticket's flags folded over its legs, refreshed on the 10 s
    /// bucket while bindings exist: merged/ahead/rebase, the tip, where a
    /// squash or a rebase-merge landed the work (T-267), and the base tip a
    /// rebase ask is recorded against — a workspace ticket's is its legs'
    /// joined, so any leg's base moving is news.
    wt_agg: HashMap<ulid::Ulid, worktree::Aggregate>,
    /// Each binding's flags PER LEG (T-368), in leg order — the snapshot's
    /// per-repo rows, `ticket_merged`'s content memo (handed back to the
    /// next sample so the patch scan runs only when a tip moved), and what
    /// `wt_agg` is the `aggregate` of. A single-repo binding is one
    /// leg.
    wt_repos: HashMap<ulid::Ulid, Vec<worktree::RepoFlags>>,
    /// A workspace provision's `(done, total)` while it runs (T-368).
    wt_progress: HashMap<ulid::Ulid, (u32, u32)>,
    /// The tickets whose provisioning is in the init script (T-614).
    wt_init: std::collections::HashSet<ulid::Ulid>,
    /// Every branch checked out twice, across every repository the legs
    /// live in.
    wt_conflicts: Vec<String>,
    /// Bumped by every synchronous flag refresh; a worker's sample carries
    /// the value it started under and lands only if nothing bumped it since.
    wt_gen: u64,
    /// A worker is out sampling the flags: the tick asks for no second one.
    wt_inflight: bool,
    /// Every flag sample is numbered as it starts (T-678): `wt_seq` the
    /// last one started, `wt_inflight_seq` the board's sample out now, and
    /// `wt_fresh` the newest absorbed per ticket — so a sample older than
    /// one already taken for a ticket leaves that ticket alone, and a
    /// finished turn can wait for a look that began after it (`trainhold`).
    wt_seq: u64,
    wt_inflight_seq: u64,
    wt_fresh: HashMap<ulid::Ulid, u64>,
    /// A ticket's own look out now (T-678), and whether another is wanted
    /// when it lands: one in flight per ticket.
    ticket_looks: HashMap<ulid::Ulid, bool>,
    /// Finished turns on train tickets waiting for that look (T-678).
    awaiting_looks: HashMap<ulid::Ulid, trainhold::AwaitingLook>,
    /// Cached default-branch name (origin/HEAD → main/master/trunk → HEAD).
    base_branch: Option<String>,
    /// Cached remote-tracking ref per leg name (`""` the root): `origin/main`
    /// where git has one, the ref a merged PR lands on (T-267). Asked once
    /// per leg — the value is whether there is one — and forgotten with
    /// `base_branch`, since a fetch can mint either.
    upstreams: HashMap<String, Option<String>>,
    /// Worktrees on their way out — a deleted ticket's once its grace
    /// expired, an archived ticket's once its work landed (T-278) — each
    /// waiting for the reaper (never remove a live cwd).
    pending_teardown: Vec<Teardown>,
    /// Worktrees a worker is removing right now (T-561), from the moment
    /// `process_teardowns` hands one over until its `Msg::TornDown` lands.
    /// Memory only: a daemon that stops mid-flight finds the tree gone or
    /// standing at start, and reads the binding off the disk either way.
    tearing_down: std::collections::HashSet<ulid::Ulid>,
    /// Writer-thread sender, cloned into provisioning threads.
    tx: Sender<Msg>,
    /// Board sharing (T-215): identity, this board's sharing state, and the
    /// relay executor's handle.
    team: teamglue::TeamCtx,
    control: mesophon::Control,
    /// A Codex observation in flight: one ask to the observer per answer.
    codex_polling: bool,
    /// The Codex observer, spawned at the first live runtime (T-688).
    codex_observer: Option<Sender<Vec<(uuid::Uuid, std::path::PathBuf)>>>,
    codex_ready: std::collections::HashSet<uuid::Uuid>,
    /// Native startup UI is checked until its composer is seen once per
    /// generation. Idle sessions then need no repeated terminal subprocess.
    codex_native_ready: HashMap<uuid::Uuid, Option<u64>>,
    codex_input_due: HashMap<uuid::Uuid, u64>,
    /// Orphaned Codex cleanup records (T-357): when each is next due a
    /// known-owner check, and the reason the last check refused, so the
    /// journal says it once rather than every retry.
    codex_orphan_due: HashMap<uuid::Uuid, (Instant, Option<String>)>,
    /// One known-owner check in flight at a time.
    codex_orphan_checking: bool,
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
    /// The crown's latest edit of each ticket it touched (T-411), so the
    /// board can light the card an agent just changed: the snapshot carries
    /// those of the last `CROWN_TOUCH_MS`, a phone's board those of the last
    /// `CROWN_TOUCH_KEPT_MS` (T-623). In memory on purpose, like the move
    /// gate's: the feed is the record, this is what the next frame needs.
    crown_touches: HashMap<ulid::Ulid, CrownTouch>,
    /// The crown's asks dropped before they were sent (T-568), by the
    /// ticket they were for: what the crown's `get_ticket` on it reads as
    /// `asked`. In memory, like the asks themselves (T-413); the crown's
    /// next ask to the ticket clears it.
    crown_dropped: HashMap<ulid::Ulid, CrownDrop>,
    /// Wakes the crown is owed (T-414, T-469, T-527): one per worker that
    /// delivered, answered the crown's ask, raised its hand or was merged
    /// since the crown's last turn, rendered into ONE sentence when the crown itself
    /// is idle. Kept in `crown.json` (T-602), so a restart loses none.
    /// Uncrowning drops them.
    crown_wakes: Vec<CrownWake>,
    /// Each worker's work as its last judged turn left it and as the last
    /// wake about it described it (T-469): what the next turn's end is
    /// compared with — a second idle at the same tip delivers nothing —
    /// and where a wake's delta runs from. Kept in `crown.json` with the
    /// wakes (T-602). A new crown starts with it empty.
    crown_heard: HashMap<ulid::Ulid, crownwake::Heard>,
    /// Workers the restored ledger names, owed one look after the restart
    /// (T-602, `hear_restored`), with the turn the last daemon did not
    /// judge; let go at `crown_recheck_until`.
    crown_recheck: HashMap<ulid::Ulid, crownwake::Unjudged>,
    crown_recheck_until: u64,
    /// `crown.json` could not be read, or a newer build wrote it.
    crown_barred: bool,
    /// The ledger as last written, so an unchanged one is not written again.
    crown_written: String,
    /// Tickets whose branch the worktree flags just read `merged` after
    /// reading it unmerged, or on a first reading of a branch the crown was
    /// told of (T-527), heard by the crown on the next tick (`hear_merges`).
    /// Filled only while a crown is worn.
    crown_landed: Vec<ulid::Ulid>,
    /// What the turn now running on a ticket was asked for (T-469), set by
    /// the ack of words that carried a reason (`Owed::asked`) and taken by
    /// that turn's end: the crown's ask comes back as an answer, a
    /// merge-flow sentence as a merge step the crown is not woken for.
    turn_asks: HashMap<ulid::Ulid, TurnAsk>,
    /// The crown's words that went in mid-turn and whose ack outlived the
    /// in-flight window (T-600), by ticket: the mark the ack would have
    /// set, kept for the turn that takes them (`crownwake::LateAsk`).
    late_asks: HashMap<ulid::Ulid, crownwake::LateAsk>,
    /// Tickets whose agent has worked since its last end of turn (T-591),
    /// set on an edge into a working state and taken by the next `EndTurn`:
    /// that end is a finished turn, and an idle re-entered with no turn
    /// between — a stale demote, a `SessionStart` in a living pane — is not
    /// a second one. On the writer, so the probe that runs off it cannot
    /// race the next turn's start.
    turns_open: std::collections::HashSet<ulid::Ulid>,
    /// Records whose stretch idle with background tasks the crown has been
    /// told of (T-599, `hear_lingering`), forgotten on the record's next
    /// foreground turn. Kept in `crown.json` (T-602).
    lingered: std::collections::HashSet<uuid::Uuid>,
    /// Tickets the crown watches (T-712, `watch_ticket`): ones it did not
    /// start, whose delivery, finished turn, raised hand and merge wake it
    /// as a worker it started does (`crownwake::crown_hears`). A watch ends
    /// at the merge, with `unwatch`, when the ticket leaves the board, when
    /// the row is turned off, and with the crown. Kept in `crown.json`.
    crown_watched: std::collections::BTreeSet<ulid::Ulid>,
    /// The session whose dialog the `answer_agent` call in hand queued an
    /// answer for (T-569): the writer loop parks that call's reply with the
    /// delivery (`control_park_reply`), which answers it when it settles.
    answer_waits: Option<uuid::Uuid>,
    /// The mod road (T-574): the road decision's cache, the commands queued
    /// for each session's mod and the polls parked for them.
    modroad: bridge::ModRoad,
    /// What the request in hand asked the writer to park (a bridge's poll, a
    /// ping), like `answer_waits`.
    mod_park: Option<bridge::Park>,
    /// The crown's plan accepts on their way (T-582), by the worker's
    /// session: pressed on the board's own road when the checkout lets
    /// them, each holding its `accept_plan` call's reply until it settles.
    /// In memory, like `plan_accept`.
    crown_plans: HashMap<uuid::Uuid, crownplan::CrownPlan>,
    /// The worker session whose plan the `accept_plan` call in hand
    /// registered a press for (T-582): the writer loop parks that call's
    /// reply with it (`crown_plan_park_reply`).
    plan_waits: Option<uuid::Uuid>,
    /// The card's line after the crown answered a question (T-569),
    /// `answered by T-411: Okta`, by the session it rides in `detail`:
    /// kept through the question's own leave, which is the answer landing,
    /// and gone at the next state edge (`apply_change`). In memory: a
    /// restart's first edge would clear it anyway.
    crown_answer_lines: HashMap<uuid::Uuid, String>,
    /// The machine's agent tiers (T-443), `tiers.toml` as last read — the
    /// layer under `board.tiers`, re-read when another board changes it.
    machine_tiers: tiers::MachineTierCache,
    /// Subscription quota (T-327): the machine's one reading as this daemon
    /// holds it, each provider's schedule, and what each board asked for.
    usage: crate::usage::UsageState,
    /// When each ticket's tier was last picked, so a seat is relaunched on
    /// the pick a person stopped at rather than on every `^n` on the way
    /// (`tiers::TIER_SETTLE_MS`). In memory: a restart settles at once.
    tier_set_at: HashMap<ulid::Ulid, u64>,
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
    /// The next sample also fetches every nested repo with a remote — a
    /// person's press on the push / pull lists (T-455). The periodic fetch
    /// never sets it: it stays the board's own branch.
    git_fetch_nested_wanted: bool,
    /// The nested repos the in-flight pass is fetching, by census name.
    git_fetching_repos: std::collections::HashSet<String>,
    /// Each nested repo's fetches by mesimon, stamped into its `RepoSync`
    /// the way the root's are into `RepoGit`: this fetch writes no
    /// `FETCH_HEAD`, so the sample alone would never see it happen.
    git_nested_fetch: HashMap<String, NestedFetch>,
}

/// mesimon's own fetches of one nested repo (T-455).
#[derive(Debug, Default)]
struct NestedFetch {
    /// The last success (unix ms), 0 = none.
    ok_at_ms: u64,
    /// The last failure since then: when, and git's first stderr line.
    error: Option<(u64, String)>,
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
    let mut journal = crate::journal::Journal::open(&paths.daemon_log());
    journal.line(&format!(
        "started pid {} build {} exe {} repo {} {}",
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        exe_stamp.map_or_else(
            || "unknown".to_string(),
            |s| format!("mtime {} len {}", s.mtime_ms, s.len)
        ),
        paths.repo_root.display(),
        if std::env::var_os("MESIMON_DETACHED").is_some() { "detached" } else { "foreground" },
    ));

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
        .or_else(|_| mesimon_core::exe::current_exe())
        .unwrap_or_else(|_| std::path::PathBuf::from("mesimon"));
    let pane_died = mesimon_backend_tmux::conf::pane_died_cmd(
        &hook_bin.display().to_string(),
        &paths.hook_sock().display().to_string(),
    );
    let backend = TmuxBackend::new(paths.tmux_sock(), &paths.state_dir, Some(&pane_died))?;
    if backend.server_alive() {
        let _ = backend.install_pane_died_hook(&pane_died);
        let _ = backend.install_copy_bindings();
        let _ = backend.install_scroll_bindings();
    }
    // A malformed state file is a NOTICE, not a startup failure: this runs
    // after orch.sock is already bound, so a hard fail here left the client
    // staring at a 5 s blank terminal with the real cause in daemon.log.
    // Recover only previously authorized local imports, on this sole writer,
    // before any client can observe or edit their partially accepted tickets.
    let import_notices = store::imports::recover(&paths);
    let store::Loaded { mut board, mut notices, columns_write_barred, sessions_write_barred } =
        store::load(&paths)?;
    notices.extend(import_notices);

    // Reconcile persisted records against the live private server (D24).
    let snap = backend.snapshot().unwrap_or_default();
    let rec = reconcile(&board.sessions, &snap);
    // The user may have left a session while the daemon was down (this repo
    // restarts one after every daemon-side rebuild). Reconcile still says
    // `Exited`; `park_on_exit` gets the same say it would have had live —
    // but only over records THIS reconcile just moved, never over corpses
    // that were already persisted as dead, which must stay dead.
    let mut just_exited = Vec::new();
    let mut owed_abandoned = Vec::new();
    for (id, link) in &rec.links {
        if let Some(r) = board.sessions.iter_mut().find(|s| s.id == *id) {
            // Observe-only records (imported, never spawned) have no pane by
            // design — Missing is their normal condition, not a crash.
            let observe_only = r.observe_only();
            if observe_only && matches!(link, mesimon_core::reconcile::Link::Missing) {
                continue;
            }
            let was_live = r.state.is_live();
            let had_pane = r.state.has_pane();
            r.state = state_for(link, &r.state, r.kind.is_agent());
            if r.kind == SessionKind::Codex && (had_pane || r.state.has_pane()) {
                // Unsent words are intentionally in memory (the same policy
                // as Claude's owed ledger). A new daemon must not reconstruct
                // and submit a partial prompt after losing those words.
                if r.pending_submit && !r.codex_submit_sent {
                    r.pending_submit = false;
                }
                r.observation_hold = true;
                r.codex_stopping |= had_pane && !r.state.has_pane();
            } else if r.pending_submit {
                // A Claude record's owed Enter died with the ledger (`owed`
                // is memory, on purpose): nothing will press it and nothing
                // will ack it, so the flag would hold the checkout — and the
                // card's launching arc — until a kill or a wake (T-244). The
                // title sits in the box, as a plain spawn leaves it.
                r.pending_submit = false;
                owed_abandoned.push(r.ticket);
            }
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
        // A leg gone is the binding evicted (T-368): `provision_existing`
        // brings back exactly the legs that are missing.
        let whole = b.path.is_dir() && b.legs(&paths.repo_root, "").iter().all(|l| l.path.is_dir());
        if b.status == BindingStatus::Attached && !whole {
            b.status = BindingStatus::Evicted;
            wt_changed = true;
        }
    }
    if wt_changed && !worktrees_barred {
        let _ = worktree::save_bindings(&paths, &worktrees);
    }

    // T-418: the queue's starts and wakes come back. A `Wake` whose record
    // `sessions.json` no longer carries has no seat to stand on and is not
    // restored; everything else is re-checked by `sweep_queue` below, the
    // same predicate a live entry is swept by.
    let (queue_entries, queue_notices, queue_barred) = crate::askqueue::load_or_recover(&paths);
    notices.extend(queue_notices);
    let (started, started_notices, started_barred) =
        crate::started::load_or_recover(&paths, &board.sessions);
    notices.extend(started_notices);
    let (costs, cost_notices, costs_barred) = crate::cost::load_or_recover(&paths);
    notices.extend(cost_notices);
    let (crown_ledger, crown_notices, crown_barred) = crownledger::load_or_recover(&paths);
    notices.extend(crown_notices);
    let (train_file, train_notices, train_barred) = crate::train::load_or_recover(&paths);
    notices.extend(train_notices);
    let queued: Vec<QueuedAsk> = queue_entries
        .into_iter()
        .filter_map(|e| {
            let (seat, cwd) = match e.seat {
                crate::askqueue::PersistedSeat::Wake { session } => {
                    let rec = board.sessions.iter().find(|s| s.id == session)?;
                    (QueuedSeat::Wake(session), rec.cwd.clone())
                }
                crate::askqueue::PersistedSeat::Start { provider } => {
                    (QueuedSeat::Start(provider), paths.repo_root.display().to_string())
                }
            };
            Some(QueuedAsk {
                ticket: e.ticket,
                seat,
                cwd,
                text: e.text,
                queued_at: e.queued_at,
                by: None,
                sends: false,
                accept_plan: false,
                send_on_accept: false,
                held: None,
                plan: e.plan,
                deliver: Deliver::Idle,
            })
        })
        .collect();
    if !worktrees.is_empty() {
        let _ = worktree::sweep_stale_locks(&paths.repo_root);
        // The legs of a workspace binding hold their locks in the children.
        for name in crate::gitstatus::census(&paths.repo_root) {
            let _ = worktree::sweep_stale_locks(&paths.repo_root.join(name));
        }
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
    // Readers may finish out of order; enqueue frames in ACCEPT order. Otherwise
    // a tiny PreToolUse can overtake SessionStart and be erased by late startup.
    let (read_tx, read_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut next = 0u64;
        let mut completed = std::collections::BTreeMap::new();
        for (ordinal, frame) in read_rx {
            completed.insert(ordinal, frame);
            while let Some(frame) = completed.remove(&next) {
                if let Some(frame) = frame {
                    let (frame, reply): (HookFrame, UnixStream) = frame;
                    let message = if frame.event == "RemotePermission" {
                        Msg::RemotePermission(frame, reply)
                    } else if frame.road == mesimon_core::road::Road::Mod {
                        Msg::ModHook(frame)
                    } else {
                        Msg::Hook(frame)
                    };
                    let _ = hook_tx.send(message);
                }
                next += 1;
            }
        }
    });
    std::thread::spawn(move || {
        for (ordinal, stream) in hook_listener.incoming().flatten().enumerate() {
            let tx = read_tx.clone();
            std::thread::spawn(move || {
                // Every reader sends completion, including malformed/timed-out
                // frames, so a missing event cannot strand the ordered queue.
                let frame = stream.try_clone().ok().and_then(|reader| {
                    ingest::read_hook_frame(reader, Duration::from_millis(750))
                        .map(|frame| (frame, stream))
                });
                let _ = tx.send((ordinal as u64, frame));
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
    let modroad = bridge::ModRoad::start(&paths);

    let mut d = Daemon {
        paths,
        board,
        backend,
        grace: HashMap::new(),
        uploads: Default::default(),
        subscribers: Vec::new(),
        focus: None,
        shutting_down: false,
        journal,
        clients: HashMap::new(),
        stop_started: None,
        tick_slowest: ("", Duration::ZERO),
        notices,
        exe_stamp,
        detached: std::env::var_os("MESIMON_DETACHED").is_some(),
        columns_barred: columns_write_barred,
        sessions_barred: sessions_write_barred,
        worktrees_barred,
        queue_barred,
        started_barred,
        costs_barred,
        machines,
        owed: HashMap::new(),
        queued,
        plan_accept: HashMap::new(),
        plan_accept_tries: HashMap::new(),
        train: Default::default(),
        train_barred,
        train_written: String::new(),
        columns_written: String::new(),
        sessions_written: String::new(),
        ticks: 0,
        feed,
        external: Vec::new(),
        external_scanning: false,
        external_rescan_wanted: false,
        started,
        costs,
        cost_scanning: false,
        cost_due: true,
        recovery: HashMap::new(),
        cleanup_resume_offers: HashMap::new(),
        reaping: HashMap::new(),
        server_restart: None,
        rss_cache: (0, 0),
        rss_by: HashMap::new(),
        foregrounds: HashMap::new(),
        terminals: terminals_in(&snap),
        reclaim_cache: (0, 0),
        archive_cache: 0,
        archive_due: true,
        archive_bytes: 0,
        tree_sizes: HashMap::new(),
        trees_sizing: false,
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
        pending_resumes: Vec::new(),
        wt_agg: HashMap::new(),
        wt_repos: HashMap::new(),
        wt_progress: HashMap::new(),
        wt_init: std::collections::HashSet::new(),
        wt_conflicts: Vec::new(),
        wt_gen: 0,
        wt_inflight: false,
        wt_seq: 0,
        wt_inflight_seq: 0,
        wt_fresh: HashMap::new(),
        ticket_looks: HashMap::new(),
        awaiting_looks: HashMap::new(),
        base_branch: None,
        upstreams: HashMap::new(),
        pending_teardown: Vec::new(),
        tearing_down: Default::default(),
        tx: tx.clone(),
        team: teamglue::TeamCtx::new(tx.clone()),
        control: mesophon::Control::new(tx.clone()),
        codex_polling: false,
        codex_observer: None,
        codex_ready: std::collections::HashSet::new(),
        codex_native_ready: HashMap::new(),
        codex_input_due: HashMap::new(),
        codex_orphan_due: HashMap::new(),
        codex_orphan_checking: false,
        moves: MoveGate::new(),
        board_version: 0,
        agent_replay: HashMap::new(),
        crown_touches: HashMap::new(),
        crown_dropped: HashMap::new(),
        crown_wakes: Vec::new(),
        crown_heard: HashMap::new(),
        crown_recheck: HashMap::new(),
        crown_recheck_until: 0,
        crown_barred,
        crown_written: String::new(),
        crown_landed: Vec::new(),
        turn_asks: HashMap::new(),
        late_asks: HashMap::new(),
        turns_open: std::collections::HashSet::new(),
        lingered: std::collections::HashSet::new(),
        crown_watched: std::collections::BTreeSet::new(),
        answer_waits: None,
        modroad,
        mod_park: None,
        crown_plans: HashMap::new(),
        plan_waits: None,
        crown_answer_lines: HashMap::new(),
        machine_tiers: tiers::MachineTierCache::new(crate::paths::machine_tiers_file().ok()),
        usage: crate::usage::UsageState::new(crate::usage::shared_file()),
        tier_set_at: HashMap::new(),
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
        git_fetch_nested_wanted: false,
        git_fetching_repos: std::collections::HashSet::new(),
        git_nested_fetch: HashMap::new(),
    };
    // The crown's ledger (T-602): what it was told, and what it is owed.
    d.restore_crown(crown_ledger);
    // The train's open rebase asks (T-635): the hold through a restart.
    d.restore_train(train_file);
    // The restored entries (T-418), judged once as any entry is (a gone
    // ticket, a seat someone took) and written back so the file is the list
    // again; each survivor is announced, so the feed says where the marks
    // on the cards came from. The drain itself waits for the tick: the
    // checkouts are `Unknown` until the reconcile speaks
    // (`checkout_unresolved`).
    let restored: Vec<(ulid::Ulid, &'static str)> =
        d.queued.iter().map(|q| (q.ticket, q.seat.word())).collect();
    d.sweep_queue();
    for (ticket, word) in restored {
        if d.queued.iter().any(|q| q.ticket == ticket) {
            d.feed.board("automation", &format!("queued_{word}_restored"), Some(ticket));
        }
    }
    d.persist_queue();
    // Board sharing (T-215): identity, this board's sharing state, the
    // relay executor. Before any client can observe the board, so the first
    // snapshot already says whether it is shared.
    d.team_start();
    d.control_start();
    let mut parked = false;
    for id in just_exited {
        parked |= d.park_on_exit(id);
    }
    for ticket in owed_abandoned {
        d.feed.board("daemon", "prompt_submit_abandoned", Some(ticket));
        parked = true;
    }
    if parked {
        // The reconcile above already saved the corpse; write the park over
        // it, so a daemon that dies in its first second does not lose it.
        d.persist_sessions();
    }
    // Deleted tickets may have left an owned server pending cleanup when
    // the previous daemon stopped. Retain their hold until acknowledgement.
    let abandoned: Vec<_> = d
        .board
        .sessions
        .iter()
        .filter(|session| {
            session.kind == SessionKind::Codex
                && !session.argv.is_empty()
                && d.board.ticket(session.ticket).is_none()
        })
        .map(|session| session.id)
        .collect();
    for id in abandoned {
        let by = Principal::Automation { rule: "deleted_session_cleanup".into() };
        if !matches!(
            authorize(&by, &Action::Mutate, &Resource::Session { id }),
            Decision::Deny { .. }
        ) {
            d.kill_session(id);
        }
    }
    d.refresh_worktree_flags();
    // On fresh flags: the landed trees of archived tickets the last daemon
    // left standing (T-481). The first tick tears them down.
    d.reclaim_archived();
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
        // Every turn is timed and named BEFORE it runs (the message is
        // consumed by it): a turn past `journal::SLOW_TURN` earns a line
        // saying what it handled — the instrument a four-minute silence
        // taught us to want (2026-09-05).
        let started = Instant::now();
        d.control_prompt_edge(matches!(&msg, Msg::Hook(f) if f.event == "UserPromptSubmit"));
        let what: std::borrow::Cow<'static, str> = match &msg {
            Msg::Tick => "tick".into(),
            Msg::ModHook(f) => format!("mod frame {}", f.event).into(),
            Msg::RoadProbed(_) => "claude road probed".into(),
            Msg::CodexSnapshots(_) => "Codex observations".into(),
            Msg::CodexOrphansChecked(_) => "Codex orphans checked".into(),
            Msg::Hook(f) => format!("hook {}", f.event).into(),
            Msg::RemotePermission(..) => "remote permission".into(),
            Msg::Request(env, ..) => format!("request {}", env.command.wire_name()).into(),
            Msg::Provisioned(..) => "provisioned".into(),
            Msg::ProvisionInit(..) => "provision init".into(),
            Msg::ProvisionProgress(..) => "provision progress".into(),
            Msg::ShellEnvCaptured(_) => "shell env captured".into(),
            Msg::ExternalScanned(_) => "external scanned".into(),
            Msg::TreesSized(_) => "trees sized".into(),
            Msg::UsageRead(p, _) => format!("usage read {}", p.word()).into(),
            Msg::CostScanned(_) => "costs scanned".into(),
            Msg::GitSampled(..) => "git sampled".into(),
            Msg::ClientGone(_) => "client gone".into(),
            Msg::WorktreeFlags(..) => "worktree flags".into(),
            Msg::TicketFlags(..) => "ticket flags".into(),
            Msg::TornDown { .. } => "torn down".into(),
            Msg::TurnProbed(..) => "turn probed".into(),
            Msg::Team(_) => "team".into(),
            Msg::Control(..) => "mesophon".into(),
            Msg::TranscriptRead(..) => "transcript read".into(),
            Msg::ShelfRead(..) => "shelf read".into(),
        };
        d.tick_slowest = ("", Duration::ZERO);
        match msg {
            Msg::Control(generation, event, ack) => {
                d.on_control(generation, event);
                let _ = ack.send(());
            }
            // SIGTERM (`pkill -f "mesimon daemon"` after a rebuild) takes the
            // same road as `Shutdown`: the handler only raises a flag, and the
            // wheel — ≤250 ms away — is where it is honoured, on the writer
            // thread, with the machines in hand.
            Msg::Tick if TERM_REQUESTED.load(Ordering::Relaxed) => {
                d.begin_shutdown("SIGTERM");
                break;
            }
            Msg::Tick => d.on_tick(),
            Msg::Hook(frame) => d.on_hook(frame),
            Msg::ModHook(frame) => d.on_mod_hook(frame),
            Msg::RoadProbed(probe) => d.on_road_probed(probe),
            Msg::RemotePermission(frame, stream) => d.control_permission_wait(frame, stream),
            Msg::CodexSnapshots(snapshots) => d.on_codex_snapshots(snapshots),
            Msg::CodexOrphansChecked(results) => d.on_codex_orphans_checked(results),
            Msg::Provisioned(ticket, result, took, inits) => {
                d.on_provisioned(ticket, result, took, inits)
            }
            Msg::ProvisionInit(ticket) => d.on_provision_init(ticket),
            Msg::ProvisionProgress(ticket, done, total) => {
                d.on_provision_progress(ticket, done, total)
            }
            Msg::ShellEnvCaptured(result) => d.on_shell_env(result),
            Msg::ExternalScanned(items) => d.on_external_scanned(items),
            Msg::TreesSized(sized) => d.on_trees_sized(sized),
            Msg::UsageRead(p, outcome) => d.on_usage_read(p, outcome),
            Msg::CostScanned(done) => d.on_cost_scanned(done),
            Msg::GitSampled(sample, fetched) => d.on_git_sampled(sample, fetched),
            Msg::ClientGone(stream) => d.on_client_gone(&stream),
            Msg::WorktreeFlags(gen, flags) => d.on_worktree_flags(gen, flags),
            Msg::TicketFlags(ticket, seq, flags) => d.on_ticket_flags(ticket, seq, flags),
            Msg::TornDown { ticket, archived, branch_kept, took } => {
                d.on_torn_down(ticket, archived, branch_kept, took)
            }
            Msg::TurnProbed(probe) => d.on_turn_probed(probe),
            Msg::Team(done) => d.on_team(done),
            Msg::TranscriptRead(peer, grant, command, reply) => {
                d.control_transcript_read(&peer, grant, command, reply)
            }
            Msg::ShelfRead(build, pages) => d.shelf_read(*build, pages),
            Msg::Request(env, reply, stream) => {
                let resp = d.handle(env, &stream);
                // `answer_agent` (T-569) answers when its delivery settles:
                // the reply waits with it, and the shim's call with the reply.
                // `accept_plan` (T-582) the same way, when its press settles.
                let reply = match (d.answer_waits.take(), d.plan_waits.take()) {
                    (Some(id), _) => d.control_park_reply(id, reply),
                    (None, Some(id)) => d.crown_plan_park_reply(id, reply),
                    (None, None) => Some(reply),
                };
                // A bridge's poll and a ping wait with the mod road (T-574).
                let reply = match (reply, d.mod_park.take()) {
                    (Some(reply), Some(park)) => d.mod_park_reply(park, reply),
                    (reply, _) => reply,
                };
                if let Some(reply) = reply {
                    let shutdown = matches!(resp, Response::Ok) && d.shutting_down;
                    let (delivered, receipt) = channel();
                    let _ = reply.send(ClientReply {
                        response: resp,
                        delivered: shutdown.then_some(delivered),
                    });
                    if shutdown {
                        // Sending to the connection thread is not delivery:
                        // main may otherwise exit before that thread writes
                        // the final newline. A stalled/disconnected client
                        // cannot hold the daemon indefinitely.
                        let _ = receipt.recv_timeout(Duration::from_secs(2));
                        break;
                    }
                }
            }
        }
        let slowest = d.tick_slowest;
        d.journal.slow_turn(started, &what, Some((slowest.0, slowest.1)));
    }
    // The crown's ledger on the way down (T-602): whatever the shutdown's
    // own settles owed or held is there for the next daemon.
    d.persist_crown();
    let _ = d.feed.flush();
    let _ = std::fs::remove_file(d.paths.orch_sock());
    let _ = std::fs::remove_file(d.paths.hook_sock());
    let took = d.stop_started.map_or(0, |t| t.elapsed().as_millis());
    d.journal.line(&format!("stopped ∙ shutdown took {took} ms"));
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
        let mut env = vec![("MESIMON_TICKET".to_string(), key.clone())];
        if let Some(b) = self.worktrees.get(&ticket) {
            if b.status == BindingStatus::Attached && b.path == cwd {
                env.push(("MESIMON_WORKTREE_BRANCH".into(), b.branch.clone()));
            }
        }
        env
    }

    /// A launch's variables: the session's own, and for a Claude launch the
    /// mod's (T-574) — never a shell's or Codex's pane.
    fn launch_vars(
        &self,
        ticket: ulid::Ulid,
        cwd: &std::path::Path,
        session: uuid::Uuid,
        kind: SessionKind,
        pick: &bridge::Pick,
    ) -> Vec<(String, String)> {
        let road = pick.road;
        let mut env = self.session_vars(ticket, cwd);
        if road == mesimon_core::road::Road::Mod {
            env.extend(self.mod_vars(session));
            // The native relays (T-658): this Claude Code keeps the classic
            // hook events from the mod, so it reports from its own.
            if pick.native() {
                env.push(("MESIMON_MOD_NATIVE".into(), "1".into()));
            }
            // The tier the mod registers (T-577), as the shim's `--tools`:
            // what it LISTS; the daemon checks the tier at every call.
            let tools = self.agent_tools_for(ticket);
            if tools != mesimon_core::board::AgentTools::Off {
                env.push(("MESIMON_MOD_TOOLS".into(), tools.word().into()));
            }
        }
        // T-573's research seam: the spike mod observes into a log under the
        // state dir and must never hot-reload (the daemon writes that dir).
        if kind == SessionKind::Claude && mod_dir().is_some() {
            if let Some(key) =
                env.iter().find(|(k, _)| k == "MESIMON_TICKET").map(|(_, v)| v.clone())
            {
                let log = self.paths.state_dir.join("mod-log").join(&key);
                let _ = std::fs::create_dir_all(&log);
                if road != mesimon_core::road::Road::Mod {
                    env.push(("CLAUDE_CODE_PLUGIN_DIR_WATCH".into(), "0".into()));
                }
                env.push(("MESIMON_MOD_LOG".into(), log.display().to_string()));
                env.push((
                    "MESIMON_MOD_GATE_BOARD".into(),
                    self.paths.board_dir.display().to_string(),
                ));
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
        if rig_no_flags() {
            out.push("--set".into());
            out.push("DISABLE_GROWTHBOOK=1".into());
        }
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
                self.warm_road_probe();
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

/// How long a paste of mesimon's own counts as a turn before its
/// `UserPromptSubmit` is given up on (a modal in the pane, a box that is
/// not reading).
const INFLIGHT_MS: u64 = 10_000;

/// A spawn parked behind worktree provisioning, replayed by `on_provisioned`.
struct PendingSpawn {
    ticket: ulid::Ulid,
    kind: SessionKind,
    submit_prompt: bool,
    /// The words the request carried, if any (T-294): a queued start's ask,
    /// or a send-now one. `None` is the ordinary spawn, whose whole prompt is
    /// the title and the brief.
    prompt: Option<String>,
    /// The crown's ticket when the crown asked (T-412), carried across the
    /// provisioning wait so the replayed record still counts against the
    /// budget.
    started_by: Option<ulid::Ulid>,
    /// Start in plan mode (T-434), carried across the wait.
    plan: bool,
}

/// Words waiting for a pane that reads (`Owed::parked`). `brief` marks the
/// composed spawn's paste of the ticket description (T-224): it is what
/// stamps `ticket_read` on the record when it lands, where the board's ask
/// at a sleeping claude — the user's own words — stamps nothing. `title`
/// leads the paste with the ticket's title: a resend (T-570), whose Ctrl+C
/// cleared the one the spawn typed.
#[derive(Clone)]
struct Parked {
    text: String,
    brief: bool,
    title: bool,
}

/// The feed line an owed paste earns on its ack: who is credited, and the
/// word. A launch road's ack is the user's prompt landing; a road of the
/// daemon's own clock — the train, the queue, the crown — names itself.
#[derive(Clone, Copy)]
struct Ack {
    by: &'static str,
    word: &'static str,
}

impl Ack {
    /// The user's words reached the agent: the composed spawn, the ask at a
    /// sleeping claude, the `p` prompt, a note's nudge, a manual merge
    /// request. Every launch road takes this one whatever its caller says
    /// (`Daemon::park`), so a queued Start or Wake never reads as an
    /// in-flight ASK (`ask_in_flight`).
    const PROMPT: Ack = Ack { by: "user", word: "prompt_submitted" };
    /// A queued ask's paste landed — the one word the snapshot's
    /// `Pending::in_flight` row and the ask receipts read.
    const QUEUED: Ack = Ack { by: "automation", word: "queued_ask_delivered" };
}

/// One paste of mesimon's own, waiting for the pane's `UserPromptSubmit`
/// (or a Codex pane's new turn). One entry per session (`Daemon::owed`).
/// The two shapes are one struct because their difference is a COUNT, not a
/// kind (T-244): a LAUNCH entry parks its words for the first tick after
/// `SessionStart` and presses Enter `SUBMIT_ATTEMPTS` times on a cadence
/// until the ack; a PASTED entry went in with its own Enter and only waits,
/// `INFLIGHT_MS`, before the checkout stops counting it as busy.
struct Owed {
    ticket: ulid::Ulid,
    /// Not yet pasted: `Some` on the launch roads (the composed spawn, the
    /// wake-and-ask, a Codex pane's paste, which `drive_codex_inputs` makes
    /// on its own clock); `None` once pasted, and from birth for a Claude
    /// live-pane paste.
    parked: Option<Parked>,
    /// Enter presses left. `SUBMIT_ATTEMPTS` on the launch road; 0 when the
    /// paste carried its own Enter.
    presses: u8,
    /// The next press, epoch ms. `None` until the `SessionStart` edge arms
    /// it (`arm_owed`): the edge is a race Claude's startup can lose, and a
    /// lost Enter is re-pressed for free where a lost paste is the user's
    /// words gone.
    next_press: Option<u64>,
    /// The give-up clock, epoch ms: `INFLIGHT_MS` after a Claude live-pane
    /// paste. `None` on the launch roads (the attempts are their clock) and
    /// for Codex (the record's hold is: `pending_submit` keeps the checkout
    /// until a new turn is observed, and nothing here may expire under it).
    expires: Option<u64>,
    /// The composer wait (T-570), epoch ms: words still parked go into a
    /// Claude pane only once `composer::read` finds its composer, and one
    /// that has shown none by then is a failed start. Set with the first
    /// press; the presses count down only after the paste.
    ready_by: Option<u64>,
    /// What was pasted, kept until the ack so a give-up can say what never
    /// reached the agent (`SessionRecord.unsent`).
    sent: Option<Parked>,
    /// A resend (T-570): one Ctrl+C into a composer holding stray text
    /// before the paste, never a second — two exit Claude.
    clear_first: bool,
    /// Parked words wait for the session's mod (T-575): they go down its
    /// bridge as a `submit` when it first polls, and take the paste road
    /// only if it has not by `bridge_by`. `false` on the hooks road and once
    /// they fell back.
    mod_road: bool,
    /// The mod-road wait's end, epoch ms: set with the first press.
    bridge_by: Option<u64>,
    /// The `submit` frame these words went down in (T-575): the bridge's
    /// ack of it is the mod's receipt (`taken`), and a mod that reports it
    /// refused lands the words on `unsent`.
    frame: Option<String>,
    taken: bool,
    /// The frame is a `fill` (T-601): the mod puts the words in the empty
    /// composer, and its `filled` is the daemon's cue to press Claude Code's
    /// send-now (`road::SEND_NOW_KEYS`). A refused fill sends them as a
    /// `submit` instead (`on_mod_fill`).
    fill: bool,
    ack: Ack,
    /// Why these words were sent, when that matters to the crown (T-469):
    /// its ack marks the turn that took them (`Daemon::turn_asks`).
    asked: Option<TurnAsk>,
}

impl Owed {
    /// A launch entry: words parked for a pane on its way, an Enter owed.
    fn launch(ticket: ulid::Ulid, parked: Parked, ack: Ack) -> Self {
        Owed {
            ticket,
            parked: Some(parked),
            presses: SUBMIT_ATTEMPTS,
            next_press: None,
            expires: None,
            ready_by: None,
            sent: None,
            clear_first: false,
            mod_road: false,
            bridge_by: None,
            frame: None,
            taken: false,
            fill: false,
            ack,
            asked: None,
        }
    }

    /// A paste that already went in with its Enter: only the ack is owed.
    fn pasted(ticket: ulid::Ulid, ack: Ack, now: u64) -> Self {
        Owed {
            ticket,
            parked: None,
            presses: 0,
            next_press: None,
            expires: Some(now + INFLIGHT_MS),
            ready_by: None,
            sent: None,
            clear_first: false,
            mod_road: false,
            bridge_by: None,
            frame: None,
            taken: false,
            fill: false,
            ack,
            asked: None,
        }
    }

    /// Words that went down the session's mod as a `submit` (T-575): only
    /// the ack is owed, as for a paste that carried its Enter, and the words
    /// are kept until it comes.
    fn submitted(ticket: ulid::Ulid, words: Parked, frame: String, ack: Ack, now: u64) -> Self {
        Owed { sent: Some(words), frame: Some(frame), ..Owed::pasted(ticket, ack, now) }
    }
}

/// An ask parked until its checkout is quiet (see `Daemon::queued`). The
/// list holds the entries and the BOARD holds their order (`queue_order`):
/// a card moved up its column goes first, the way the merge train reads
/// the board (T-263).
struct QueuedAsk {
    ticket: ulid::Ulid,
    /// The seat it was queued at; a seat that changed at delivery time drops
    /// it rather than redirecting the words (`sweep_queue`).
    seat: QueuedSeat,
    /// The checkout key: `SessionRecord.cwd`, compared as a string.
    cwd: String,
    /// Empty only for a `Start`, where the prompt is the ticket's own title
    /// and brief — the composed spawn, waiting its turn.
    text: String,
    queued_at: u64,
    /// The crown ticket whose agent queued these words (T-413). `Some` is a
    /// HELD ask unless `sends`: `drain_queue` never takes it, only a
    /// person's send (`SendQueuedAsk`) delivers it and a take-back returns
    /// it — the road that keeps a person between one session and another's
    /// turn. Never persisted, sending or not: it dies with the daemon, and
    /// the crown may ask again.
    by: Option<ulid::Ulid>,
    /// The crown's ask goes by the queue like a person's (T-550): the
    /// board's crown mode is autonomous and the crown started this agent, so
    /// `drain_queue` delivers it once the agent is idle. False on every
    /// person's ask; set by the crown's handler after `park_ask`.
    sends: bool,
    /// Accept the agent's plan on the way (T-420): when the pane it was
    /// queued at reaches its plan dialog, `service_plan_accepts` presses the
    /// harness's default and clears this. Pane seats only — the flag is
    /// about a dialog, and only a pane shows one — so it never rides
    /// `queue.json`.
    accept_plan: bool,
    /// The press went in and the words go the moment the harness confirms
    /// it — the record leaving `Plan` (T-420, user 2026-09-23: "immediately
    /// after plan was approved send the words", so "main moved since the
    /// plan started" reaches the agent before it writes a line). Not on the
    /// idle after, not behind the checkout: the approved turn is this
    /// agent's own. A press never confirmed clears this, and the words wait
    /// as an ordinary ask does.
    send_on_accept: bool,
    /// `Some` is an ask the daemon HELD (T-420): the agent stopped on a
    /// question after these words were queued, and the answer may change
    /// what they should say. `drain_queue` skips it the way it skips a
    /// crown's ask; a person's `^y` sends it, and reopening the field on it
    /// re-queues it clean. The word is the card's (`agent asked`).
    held: Option<&'static str>,
    /// The delivery starts a plan-mode turn (T-434): a start or a wake
    /// launches with `--permission-mode plan`, and a pane is parked and
    /// woken with it once idle. Rides `queue.json` (schema 2) on the
    /// seats that ride it.
    plan: bool,
    /// The level the crown asked these words to go at (T-600, T-601) and
    /// the board held them for a person: the card's field reopens at it,
    /// and `^y` sends them at once, by Claude Code's send-now where the
    /// crown asked `immediately`. Set by the crown's handler after
    /// `park_ask`; never persisted, as no crown's ask is.
    deliver: Deliver,
}

/// A crown's ask that left the queue unsent (T-568): whose crown it was,
/// who dropped it (`person` or `board`, `AgentAskedView::by`) and when.
struct CrownDrop {
    crown: ulid::Ulid,
    by: &'static str,
    at_ms: u64,
}

impl QueuedAsk {
    /// Waits for a person's `^y`: the crown's ask the board does not let it
    /// send (T-413, T-550), or one the daemon held on a question (T-420).
    /// The queue neither delivers it nor lets it take its checkout's turn.
    fn held_for_person(&self) -> bool {
        self.held.is_some() || (self.by.is_some() && !self.sends)
    }
}

/// Where a prompt's claude is (`Daemon::seat_of`), and therefore how it is
/// delivered. A queued ask remembers the seat it was aimed at: the words go
/// to that claude or nowhere, never to whoever is sitting there later.
enum QueuedSeat {
    /// A pane to paste into — today's ask.
    Pane(uuid::Uuid),
    /// A parked claude: the delivery wakes it and parks the words for the
    /// first tick after `SessionStart` (T-294).
    Wake(uuid::Uuid),
    /// No claude at all: the delivery starts one on the ticket's title, with
    /// the words (if any) under the brief (T-294).
    Start(AgentProvider),
}

impl QueuedSeat {
    /// The feed's word for this seat (`queued_<word>_restored`).
    fn word(&self) -> &'static str {
        match self {
            QueuedSeat::Pane(_) => "ask",
            QueuedSeat::Wake(_) => "wake",
            QueuedSeat::Start(_) => "start",
        }
    }

    /// What the snapshot carries for this seat (`Pending::action`), which
    /// is what makes the card say `claude starts` rather than `queued`.
    fn action(&self) -> PendingAction {
        match self {
            QueuedSeat::Pane(_) => PendingAction::Ask,
            QueuedSeat::Wake(_) => PendingAction::Wake,
            QueuedSeat::Start(_) => PendingAction::Start,
        }
    }
}

use mesimon_core::clock::{now_ms, now_secs};

fn no_such_ticket() -> Response {
    Response::Err { message: "no such ticket".into() }
}

/// Why nothing may grow on an archived ticket until it is restored.
const TICKET_ARCHIVED: &str = "ticket archived — restore it first";

/// The crown's `archive_ticket` while the board's switch is off (T-590):
/// result data, so it may instruct, and it names the row a person turns and
/// the road the crown has instead.
fn crown_archive_off(key: &str, restore: bool) -> String {
    const ROW: &str = "Settings → Agents → Crown archives tickets is off";
    if restore {
        format!("{ROW}; a person restores {key}")
    } else {
        format!("{ROW}; move {key} to DONE instead, or a person archives")
    }
}

/// The crown's watch while the row is off (T-712): the row, and the road
/// that stays — a person's own word when the ticket is done.
fn crown_watch_off(key: &str) -> String {
    format!(
        "Settings → Agents → Crown watches tickets is off; a person turns it on, or says when \
         {key} is done"
    )
}

/// The crown's next step at a branch behind its base (T-613): the words
/// `merge_ticket`'s refusal ends on, the same whether the flags read it
/// before the merge or the merge itself answered `NeedsRebase`.
fn rebase_first() -> &'static str {
    "ask the agent to rebase first (ask_agent), and merge_ticket once its turn ends with \
     merge_state ahead"
}

/// One boolean key of a `prefs.json` (T-613): `None` where the file, the
/// key or the type is not there, so the next file is asked. The TUI owns
/// the file; this reads one flag of it at the moment the daemon decides.
fn pref_flag(path: &std::path::Path, key: &str) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?.get(key)?.as_bool()
}

/// The notice kind a failed shell-env capture stands under. One kind, replaced
/// rather than appended, so a shell that fails on every reload leaves one row.
const SHELL_ENV_NOTICE: &str = "shell_env";
/// The folder cut-off's notice kind (T-690), the wire's own word.
const SERVER_CUT_OFF: &str = mesimon_core::command::SERVER_CUT_OFF;

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
    if matches!(
        env.principal,
        Principal::Paired { .. } | Principal::Remote { .. } | Principal::Automation { .. }
    ) {
        return Response::Err { message: "principal cannot be claimed by a client".into() };
    }
    let target = match &env.command {
        Command::DiffList { target } | Command::DiffFile { target, .. } => target.clone(),
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
    // bypass the chokepoint. The checkout is the board's own, and so is its
    // history, so either is a read of the board rather than of any one ticket.
    let resource = match target {
        DiffTarget::Ticket { id } => Resource::Ticket { id },
        DiffTarget::Checkout | DiffTarget::Commit { .. } => Resource::Board,
    };
    if let Decision::Deny { reason } = authorize(&env.principal, &Action::Read, &resource) {
        return Response::Err { message: format!("denied: {reason}") };
    }
    let _permit = PermitGuard::acquire(&ctx.permits);
    let repo = &ctx.paths.repo_root;
    if let DiffTarget::Commit { oid, repo: nested } = &target {
        let repo = match crate::gitstatus::commit_dir(repo, nested.as_deref()) {
            Ok(dir) => dir,
            Err(message) => return Response::Err { message },
        };
        return match &env.command {
            Command::DiffList { .. } => crate::diff::commit_diff_list(&repo, oid)
                .unwrap_or_else(|e| Response::Err { message: e.to_string() }),
            Command::DiffFile { path, context, .. } => {
                match crate::diff::commit_diff_file(&repo, oid, path, *context) {
                    Ok(file) => Response::DiffFile { file },
                    Err(e) => Response::Err { message: e.to_string() },
                }
            }
            _ => unreachable!(),
        };
    }
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
    // A workspace binding (T-368) is diffed leg by leg, one list.
    if binding.is_workspace() {
        return match &env.command {
            Command::DiffList { .. } => crate::diff::workspace_diff_list(repo, binding)
                .unwrap_or_else(|e| Response::Err { message: e.to_string() }),
            Command::DiffFile { path, context, .. } => {
                match crate::diff::workspace_diff_file(repo, binding, path, *context) {
                    Ok(file) => Response::DiffFile { file },
                    Err(e) => Response::Err { message: e.to_string() },
                }
            }
            _ => unreachable!(),
        };
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

/// A connection's identity for the writer's own maps: the address of the
/// `Arc` the connection thread shares with it (what `subscribers` and the
/// train already compare by `Arc::ptr_eq`).
fn conn_key(stream: &Arc<Mutex<UnixStream>>) -> usize {
    Arc::as_ptr(stream) as usize
}

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
        let mut delivered = None;
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
                    match rrx.recv() {
                        Ok(reply) => {
                            delivered = reply.delivered;
                            reply.response
                        }
                        Err(_) => Response::Err { message: "daemon gone".into() },
                    }
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
        if writeln!(w, "{json}").and_then(|()| w.flush()).is_err() {
            break;
        }
        if let Some(delivered) = delivered {
            let _ = delivered.send(());
        }
    }
    // Same `tx` as the requests above, so it orders after them: whatever
    // this connection armed is disarmed once its last word is in.
    let _ = tx.send(Msg::ClientGone(writer));
}

/// What an agent's `create_ticket` asks to file, as the wire carried it.
struct AgentFiling {
    title: String,
    column: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
    /// A tier id or name (T-584), the crown's alone.
    tier: Option<String>,
    /// A word (T-583), required of the crown.
    workspace: Option<String>,
}

/// What a mutating agent tool call left behind, kept under its idempotency
/// key so a retry gets the first receipt. Keyed by tool as well as by key:
/// a `move_ticket` retry must never be answered with a `create_ticket`
/// receipt that happened to share a client-minted id.
#[derive(Debug, Clone)]
enum AgentReplay {
    Moved { column: String },
    Created { key: String, column: String, workspace: String },
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
        // `Remote` is minted by the daemon's own sync from a record whose
        // signature verified (T-215). Over the socket it is a same-uid
        // client dressing up as a teammate. Paired identities are minted only
        // by the authenticated Mesophon control connection.
        if matches!(env.principal, Principal::Remote { .. } | Principal::Paired { .. }) {
            return Response::Err {
                message: "remote identities cannot be claimed by a local client".into(),
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
            Command::ImportTicket { column, .. } | Command::CreateTicketWithNote { column, .. } => Resource::Column { name: column.clone() },
            Command::ReadAttachment { ticket, .. } | Command::SaveNoteWithAttachments { ticket, .. } => Resource::Ticket { id: *ticket },
            Command::PaneTail { session, .. } => Resource::Session { id: *session },
            // Same rule as the line above, for the pane the user is inside
            // (T-299). `Board` where nothing is focused: there is no session
            // to name and the command answers `None` anyway.
            Command::FocusQuiet => match self.focus_held() {
                Some(Focus::Session(id)) => Resource::Session { id },
                _ => Resource::Board,
            },
            // Typing into a pane is changing the session, and the chokepoint
            // should say which one. `Board` when there is no target: the
            // command is about to refuse anyway, and inventing a session id
            // to authorize against would be the wrong kind of tidy.
            Command::PromptSession { ticket, .. } => match self.prompt_target(*ticket) {
                Some(id) => Resource::Session { id },
                None => Resource::Board,
            },
            // A column's worth of panes (T-378): the column is the resource
            // the chokepoint hears, the way an import into one is.
            Command::PromptColumn { column, .. } => Resource::Column { name: column.clone() },
            // A note is the ticket's: the first local commands to name the
            // precise resource, which is what `authorize` was built to hear.
            Command::DuplicateTicket { id: ticket }
            | Command::ReadNote { ticket, .. }
            | Command::WriteNote { ticket, .. }
            | Command::NoteToAgent { ticket, .. }
            // Growing a ticket a shell is changing the ticket (T-366).
            | Command::AdoptTerminal { ticket } => Resource::Ticket { id: *ticket },
            _ => Resource::Board,
        };
        if let Decision::Deny { reason } = authorize(&env.principal, &meta.action, &resource) {
            return Response::Err { message: format!("denied: {reason}") };
        }
        // A viewer's copy of a team board (T-335): the same chokepoint, one
        // rule later, so the refusal names the board's owner.
        if let Some(message) = self.team_read_only(&env.command) {
            return Response::Err { message };
        }
        let feed_cmd = meta.logged.then(|| (env.command.wire_name(), meta.subject));
        // Another board may have changed the machine's tiers (T-443): one
        // `stat`, so what this command reads or launches is current.
        self.refresh_machine_tiers();

        let resp = match env.command {
            Command::Mesophon { action } => self.control_local(action),
            Command::Hello { version, client } => {
                self.clients.insert(conn_key(stream), client);
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
                // The board as it stands, `external_scanning` set: the drawer
                // opens on the last answer and the walk's `BoardChanged`
                // brings the new one (T-437).
                self.snapshot()
            }
            Command::Subscribe => {
                let was_headless = !self.git_has_reader();
                self.subscribers.push(stream.clone());
                // A headless daemon stopped sampling (T-251): the first board
                // back gets a fresh header now rather than at the next bucket.
                if was_headless {
                    self.queue_git_sample();
                }
                Response::Ok
            }
            Command::CreateTicket { column, title, workspace, tier } => {
                self.create_ticket(&env.principal, column, title, workspace, tier)
            }
            Command::ImportTicket { column, content, origin } => {
                self.import_ticket(&env.principal, column, content, origin)
            }
            Command::DuplicateTicket { id } => self.duplicate_ticket(&env.principal, id),
            Command::RenameTicket { id, title } => {
                self.with_ticket(id, |t| t.title = mesimon_core::board::sanitize_title(&title))
            }
            Command::DeleteTicket { id, discard_worktree } => {
                self.delete_ticket(id, discard_worktree)
            }
            // Worktree work is refused wholesale while the bindings file is
            // barred: acting would either strand a new worktree we cannot
            // record, or tear down a real one on a guess (D26, fail closed).
            // A spawn and a workspace choice refuse on their own roads
            // (`resolve_spawn_cwd`, `set_workspace`, T-683), so every caller
            // of either is covered without a guard of its own.
            Command::MergeTicket { .. } | Command::MergeToAgent { .. }
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
            Command::MergeTicket { id } => self.hand_merge(id),
            Command::MergeToAgent { id, request } => {
                self.merge_to_agent(id, request, &Principal::Local, Ack::PROMPT)
            }
            Command::PromptSession { ticket, resend: true, .. } => self.resend_unsent(ticket),
            Command::PromptSession {
                ticket,
                text,
                queued,
                immediately,
                accept_plan,
                plan,
                tier,
                resend: false,
            } => self.prompt_session(ticket, text, queued, immediately, accept_plan, plan, tier),
            Command::PromptColumn { column, text, queued, accept_plan } => {
                self.prompt_column(&column, text, queued, accept_plan)
            }
            Command::DropQueuedAsk { ticket } => self.drop_queued_ask(ticket),
            Command::SendQueuedAsk { ticket } => self.send_queued_ask(ticket),
            Command::TakeQueuedAsk { ticket } => self.take_queued_ask(ticket),
            Command::SetAutomation { merge_train, merge_notice } => {
                self.set_automation(merge_train, merge_notice, stream)
            }
            Command::DiscardAttachmentUploads { uploads } => {
                self.uploads.discard(&crate::attachments::Owner::stream(stream), &uploads);
                Response::Ok
            }
            Command::UploadAttachment { upload, offset, data, complete } => {
                let owner = crate::attachments::Owner::stream(stream);
                match self.uploads.chunk(&owner, upload, offset, &data, complete) {
                    Ok(upload) => Response::AttachmentUploaded { upload },
                    Err(e) => Response::Err { message: format!("could not upload picture: {e:#}") },
                }
            }
            Command::ReadAttachment { ticket, attachment } => {
                self.read_attachment(ticket, attachment)
            }
            Command::SaveNoteWithAttachments { ticket, note, text, uploads } => {
                let owner = crate::attachments::Owner::stream(stream);
                self.save_note_with_attachments(&owner, ticket, note, text, uploads, &Principal::Local)
            }
            Command::CreateTicketWithNote {
                column,
                title,
                workspace,
                text,
                uploads,
                tags,
                tier,
            } => self.create_ticket_with_note(
                stream,
                Draft { column, title, workspace, text, uploads, tags, tier },
            ),
            Command::ReadNote { ticket, note } => self.read_note(ticket, note),
            Command::WriteNote { ticket, note, text, rev } => {
                self.write_note(ticket, note, text, rev, &Principal::Local)
            }
            Command::NoteToAgent { ticket, note } => self.note_to_agent(ticket, note),
            Command::RestoreTicket { id } => self.restore_ticket(id),
            Command::ArchiveTicket { id } => self.archive_ticket(id, &Principal::Local),
            Command::UnarchiveTicket { id } => self.unarchive_ticket(id),
            Command::SnoozeTicket { id, until, needs_you } => {
                self.snooze_ticket(id, until, needs_you)
            }
            Command::SeenTicket { id } => self.seen_ticket(id),
            Command::LowerHand { id } => self.lower_hand(id),
            Command::OpenedTicket { id } => self.opened_ticket(id),
            Command::SetManualMerge { id, on } => self.set_manual_merge(id, on),
            Command::CrownTicket { id } => self.crown_ticket(id),
            Command::Uncrown => self.uncrown(),
            Command::SetMcpTools { on } => self.set_mcp_tools(on),
            Command::SetAgentProvider { provider } => self.set_default_tier(
                mesimon_core::tier::TierScope::Board,
                Some(mesimon_core::tier::builtin_id(provider).to_string()),
            ),
            Command::SetTicketTier { id, tier } => self.set_ticket_tier(id, tier),
            Command::SaveTier { scope, tier } => self.save_tier(scope, tier),
            Command::DeleteTier { scope, id } => self.delete_tier(scope, id),
            Command::MoveTier { scope, id, to_index } => self.move_tier(scope, id, to_index),
            Command::SetDefaultTier { scope, id } => self.set_default_tier(scope, id),
            Command::SetCrownBudget { budget } => self.set_crown_budget(budget),
            Command::SetCrownMode { mode } => self.set_crown_mode(mode),
            Command::SetCrownArchives { on } => self.set_crown_archives(on),
            Command::SetCrownWatches { on } => self.set_crown_watches(on),
            Command::SetStatusLine { top } => self.set_status_line(top),
            Command::SetUsageWants { claude, codex } => {
                if self.usage.set_wants(conn_key(stream), Wants { claude, codex }) {
                    self.broadcast();
                }
                Response::Ok
            }
            Command::RefreshUsage { claude, codex } => {
                self.usage.ask(Wants { claude, codex });
                if self.drive_usage(now_ms()) {
                    self.broadcast();
                }
                Response::Ok
            }
            Command::SetSystemPrompt { on } => self.set_system_prompt(on),
            Command::SetDefaultColumn { column } => self.set_default_column(column.as_deref()),
            Command::SetFollowUpMode { mode } => {
                if self.columns_barred {
                    Response::Err { message: self.barred_message("columns") }
                } else {
                    self.board.follow_up_mode = mode;
                    self.persist_and_notify();
                    Response::Ok
                }
            }
            Command::SetAgentPrompt { which, text } => self.set_agent_prompt(which, text),
            Command::IgnoreBriefOffer => self.ignore_brief_offer(),
            Command::TeamSignIn { relay, display_name, code } => {
                self.team_sign_in(relay, display_name, code)
            }
            Command::TeamSignOut => self.team_sign_out(),
            Command::RedeemCode { code } => self.team_redeem(code),
            Command::ShareBoard { notes } => self.team_share(notes),
            Command::UnshareBoard => self.team_unshare(),
            Command::MintInvite { role } => self.team_mint_invite(role),
            Command::RevokeMember { device } => self.team_revoke(device),
            Command::JoinBoard { code } => self.team_join(code),
            Command::LeaveBoard => self.team_leave(),
            Command::TeamRefresh => self.team_refresh(),
            Command::AddColumn { name, after } => self.add_column(name, after),
            Command::RenameColumn { name, to } => self.rename_column(&name, &to),
            Command::DeleteColumn { name } => self.delete_column(&name),
            Command::ReorderColumn { name, before } => self.reorder_column(&name, before),
            Command::SetColumnSettings { name, settings } => {
                self.set_column_settings(&name, settings)
            }
            Command::SortColumn { column, by } => self.sort_column(&column, by),
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
            Command::RestartServer => self.restart_server(),
            Command::GitFetch => {
                // The board's own upstream, or any workspace repo compared
                // with a remote (T-455): a press fetches all of them.
                let nested = self.git_cache.nested.iter().any(|s| s.upstream.is_some());
                if self.git_cache.upstream.is_none() && !nested {
                    return Response::Err { message: "no upstream to fetch".into() };
                }
                self.git_fetch_wanted = true;
                self.git_fetch_nested_wanted = true;
                self.queue_git_sample();
                // `fetching` becoming true is what the menu row shows for the
                // press; a slow remote must not leave the row looking missed.
                self.broadcast();
                Response::Ok
            }
            Command::MoveTicket { id, column, before } => self.move_ticket(id, column, before),
            Command::SpawnSession { ticket, kind, submit_prompt, plan } => self.spawn_session(
                ticket,
                if kind.is_agent() {
                    self.tier_book().start_provider(ticket).session_kind()
                } else {
                    kind
                },
                submit_prompt,
                None,
                None,
                plan,
            ),
            Command::KillSession { id } => self.kill_session(id),
            Command::FocusStart { session } => self.focus_start(session, stream),
            Command::FocusEnd { session } => {
                if self.focus_held() == Some(Focus::Session(session)) {
                    self.focus = None;
                }
                Response::Ok
            }
            Command::OpenTerminal { ticket } => self.open_terminal(ticket, stream),
            Command::TerminalEnd => {
                if matches!(self.focus_held(), Some(Focus::Terminal { .. })) {
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
                let who = self
                    .clients
                    .get(&conn_key(stream))
                    .cloned()
                    .unwrap_or_else(|| "a client that sent no hello".to_string());
                self.begin_shutdown(&format!("shutdown asked by {who}"));
                Response::Ok
            }
            // Never reaches the writer — client_loop short-circuits these to
            // serve_diff on the connection thread (M4b). Defensive arm only.
            Command::DiffList { .. } | Command::DiffFile { .. } => Response::Err {
                message: "diff commands are served on the connection thread".into(),
            },
            Command::PaneTail { session, lines } => self.pane_tail(session, lines),
            Command::TerminalTail { ticket, lines } => self.terminal_tail(ticket, lines),
            Command::AdoptTerminal { ticket } => self.adopt_terminal(ticket),
            Command::FocusQuiet => self.focus_quiet(),
            Command::AttachExternal { claude_session_id, ticket } => {
                match self.attach_external(&env.principal, claude_session_id, ticket) {
                    Ok(id) => {
                        self.persist_and_notify();
                        Response::Spawned { id, fresh: false }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            Command::ResumeExternal { claude_session_id, ticket, confirm } => {
                match self.attach_external(&env.principal, claude_session_id, ticket) {
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
                let resp = self.resume_session_with_cleanup_ack(
                    id,
                    confirm,
                    env.principal.is_human(),
                    false,
                );
                self.retype_title(&resp);
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
                self.retype_title(&resp);
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
            // The agent tier, reached only via `handle_agent`. A local client
            // sending one of these is either confused or probing; either way
            // the answer is no, not "acts as the agent whose id you guessed".
            Command::AgentGetTicket
            | Command::AgentReadTicket { .. }
            | Command::AgentListBoard
            | Command::AgentMoveTicket { .. }
            | Command::AgentReadAttachment { .. }
            | Command::AgentReadNote { .. }
            | Command::AgentWriteNote { .. }
            | Command::AgentCreateTicket { .. }
            | Command::AgentTagTicket { .. }
            | Command::AgentRenameTicket { .. }
            | Command::AgentSetWorkspace { .. }
            | Command::AgentArchiveTicket { .. }
            | Command::AgentStartTicket { .. }
            | Command::AgentSleepTicket { .. }
            | Command::AgentWatchTicket { .. }
            | Command::AgentMergeTicket { .. }
            | Command::AgentAskTicket { .. }
            | Command::AgentAnswerTicket { .. }
            | Command::AgentAcceptPlan { .. }
            | Command::AgentRaiseHand { .. }
            // The mod's bridge polls as its session, never as a person.
            | Command::ModNext { .. } => {
                Response::Err { message: "agent commands require an agent principal".into() }
            }
            Command::ModPing { session } => self.mod_ping(session),
        };
        if let Some((cmd, ticket)) = feed_cmd {
            let cmd = cmd.as_str();
            match &resp {
                Response::Ok
                | Response::Spawned { .. }
                | Response::Provisioning
                | Response::NoteWritten { .. } => self.feed.board("local", cmd, ticket),
                Response::Created { id, .. } => self.feed.board("local", cmd, ticket.or(Some(*id))),
                Response::Merge {
                    outcome: MergeOutcome::Merged | MergeOutcome::AlreadyMerged,
                    ..
                } => self.feed.board("local", cmd, ticket),
                _ => {}
            }
        }
        resp
    }

    /// A stage of the turn in hand took `took`: remembered when it is the
    /// slowest so far, so the slow-turn line can name it.
    fn note_stage(&mut self, name: &'static str, took: Duration) {
        if took > self.tick_slowest.1 {
            self.tick_slowest = (name, took);
        }
    }

    /// One wheel tick (250 ms): grace expiry at the old 1 s cadence, settle
    /// timers, the wholesale-server guard.
    /// How long to wait for the `UserPromptSubmit` ack before pressing Enter
    /// again, and how many presses to spend before giving up and leaving the
    /// title typed. T-5 measured the ack at ~94 ms, so 500 ms is a wide
    /// margin; 10 attempts covers ~5 s of Claude startup.
    fn on_tick(&mut self) {
        self.uploads.prune();
        self.tick_mod_road();
        self.ticks += 1;
        self.team_tick();
        self.control_tick();
        // Every stage is timed and the slowest remembered, so a slow tick's
        // journal line can name the probe that took the second.
        macro_rules! stage {
            ($name:literal, $e:expr) => {{
                let t = Instant::now();
                let r = $e;
                self.note_stage($name, t.elapsed());
                r
            }};
        }
        if self.ticks.is_multiple_of(4) {
            stage!("expire_grace", self.expire_grace());
            stage!("sweep_reaping", self.sweep_reaping());
            stage!("sweep_codex_orphans", self.sweep_codex_orphans());
            stage!("process_teardowns", self.process_teardowns());
        }
        self.poll_codex();
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> =
            self.machines.iter_mut().filter_map(|(id, m)| m.tick(now).map(|c| (*id, c))).collect();
        let mut changed = self.probe_codex_startup(now);
        changed |= self.drive_codex_inputs(now);
        for (id, change) in fired {
            changed |= stage!("apply_change", self.apply_change(id, &change, None, None));
        }
        // The crown's plan accepts (T-582) answer on the wheel, as the
        // answers' deliveries do: the receipt follows its hook edge closely.
        changed |= stage!("settle_crown_plans", self.settle_crown_plans());
        if self.ticks.is_multiple_of(4) {
            changed |= stage!("probe_spawning", self.probe_spawning());
            changed |= stage!("probe_activity", self.probe_activity());
            changed |= stage!("wake_snoozed", self.wake_snoozed(now / 1000));
            changed |= stage!("finish_server_restart", self.finish_server_restart());
            // A blown move fuse that went a window with nothing trying lapses
            // (T-468); its advisory goes with it.
            changed |= self.moves.expire(Instant::now());
            // The queued asks' safety net: the edge above is the road, this
            // is the clock (a paste that never got its ack, a target that
            // went without a state change of its own).
            changed |= stage!("sweep_queue", self.sweep_queue());
            changed |= stage!("relaunch_silent_mods", self.relaunch_silent_mods(now));
            changed |= stage!("rescue_silent_mods", self.rescue_silent_mods(now));
            changed |= stage!("settle_owed", self.settle_owed(now));
            changed |= stage!("settle_plan_accepts", self.settle_plan_accepts(now));
            changed |= stage!("service_plan_accepts", self.service_plan_accepts(now));
            changed |= stage!("service_crown_plans", self.service_crown_plans());
            self.refresh_machine_tiers();
            changed |= stage!("drain_tier_switches", self.drain_tier_switches());
            changed |= stage!("drain_queue", self.drain_queue());
            changed |= stage!("hear_lingering", self.hear_lingering());
            changed |= stage!("hear_restored", self.hear_restored());
            changed |= stage!("hear_merges", self.hear_merges());
            changed |= stage!("hear_deferred", self.hear_deferred());
            changed |= stage!("hear_stepped", self.hear_stepped());
            changed |= stage!("settle_looks", self.settle_looks());
            changed |= stage!("drain_crown_wakes", self.drain_crown_wakes());
            stage!("persist_crown", self.persist_crown());
            changed |= stage!("drive_usage", self.drive_usage(now));
            if !self.cost_scanning && (self.cost_due || self.ticks.is_multiple_of(COST_TICKS)) {
                stage!("queue_cost_scan", self.queue_cost_scan());
            }
            // One candidate scan serves the count and the disk figure.
            if self.archive_due || self.ticks.is_multiple_of(ARCHIVE_TICKS) {
                self.archive_due = false;
                let candidates = stage!("archive_figures", self.archive_candidates());
                if candidates.len() != self.archive_cache {
                    self.archive_cache = candidates.len();
                    changed = true;
                }
                changed |= stage!("price_archive", self.price_archive(&candidates));
            }
        }
        if self.ticks.is_multiple_of(TAIL_POLL_TICKS) {
            changed |= stage!("poll_tails", self.poll_tails());
            changed |= stage!("probe_status_files", self.probe_status_files());
            changed |= stage!("refresh_panes", self.refresh_panes());
        }
        if self.ticks.is_multiple_of(inactivity_park_ticks()) {
            changed |= stage!("park_inactive", self.park_inactive(now));
        }
        if self.ticks.is_multiple_of(RSS_TICKS) {
            changed |= stage!("refresh_rss", self.refresh_rss());
        }
        // The worktree flags on their own cadence (a seam for the train's
        // e2e), sampled on a worker; the train's pass runs when they land
        // (`on_worktree_flags`), on fresh flags.
        if self.ticks.is_multiple_of(wt_refresh_ticks()) && !self.worktrees.is_empty() {
            stage!("queue_worktree_flags", self.queue_worktree_flags());
        }
        // The CLAUDE.md sample, on the same slow bucket but off the worktree
        // guard: a board with no worktrees still has a CLAUDE.md. Two `stat`s
        // unless something moved, so it costs the same as asking whether to
        // ask.
        if self.ticks.is_multiple_of(wt_refresh_ticks()) {
            changed |= stage!("claude_md", self.claude_md.refresh(&self.paths.repo_root));
        }
        if self.ticks % wt_refresh_ticks() == 1 {
            if !self.git_fetch_every.is_zero()
                && self.git_last_fetch.is_none_or(|t| t.elapsed() >= self.git_fetch_every)
            {
                self.git_fetch_wanted = true;
            }
            // One tick off the writer's own burst above: the sample is a fork on
            // a worker, but its spawn should not stack on the worktree flags.
            // The same slow bucket as those flags — the sample is now what
            // lets the train retry a refused merge (T-289), so a test that
            // shortens one has to shorten both or watch the train stay stuck.
            // Only while something reads it (T-251): a headless daemon would
            // otherwise fork `git status` per nested repo every bucket, forever,
            // for a header nobody has open. `Subscribe` fires the catch-up.
            if self.git_has_reader() {
                stage!("queue_git_sample", self.queue_git_sample());
            }
        }
        if self.ticks.is_multiple_of(server_guard_ticks()) {
            changed |= stage!("guard_server", self.guard_server());
            changed |= stage!("recheck_server_access", self.recheck_server_access());
        }
        if changed {
            stage!("persist_sessions", self.persist_sessions());
            stage!("broadcast", self.broadcast());
        }
        // ≤1 write() per wheel bucket, no fsync (14 §1.7).
        let _ = stage!("feed_flush", self.feed.flush());
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
    fn begin_shutdown(&mut self, why: &str) {
        self.journal.line(&format!("stopping: {why}"));
        self.stop_started = Some(Instant::now());
        self.poll_codex();
        let now = now_ms();
        let fired: Vec<(uuid::Uuid, Change)> =
            self.machines.iter_mut().filter_map(|(id, m)| m.flush(now).map(|c| (*id, c))).collect();
        let mut changed = self.drive_codex_inputs(now);
        for (id, change) in fired {
            changed |= self.apply_change(id, &change, None, Some("shutdown"));
        }
        if changed {
            self.persist_sessions();
            self.broadcast();
        }
        self.shutting_down = true;
    }

    fn probe_spawning(&mut self) -> bool {
        self.poll_agent_recovery(RecoveryChannel::Startup)
    }

    fn probe_activity(&mut self) -> bool {
        self.poll_agent_recovery(RecoveryChannel::Activity)
    }

    fn probe_status_files(&mut self) -> bool {
        self.poll_agent_recovery(RecoveryChannel::Status)
    }

    /// Batch common pane sampling while providers own eligibility, native
    /// parsing and recovery heuristics. Only this authorized writer feeds
    /// normalized evidence into machines and updates board previews.
    fn poll_agent_recovery(&mut self, channel: RecoveryChannel) -> bool {
        let now = now_ms();
        self.recovery.retain(|id, _| self.board.sessions.iter().any(|r| r.id == *id));
        let mut candidates = Vec::new();
        for record in &self.board.sessions {
            let Some(adapter) = crate::agents::adapter(record.kind) else { continue };
            let recovery = self.recovery.entry(record.id).or_insert_with(|| adapter.recovery());
            if recovery.needs_poll(record, channel, now) {
                candidates.push((record.id, record.sid16()));
            }
        }
        if candidates.is_empty() {
            return false;
        }
        let activity = if channel == RecoveryChannel::Activity {
            let Ok(activity) = self.backend.activity() else { return false };
            activity
        } else {
            Vec::new()
        };
        let mut changed = false;
        for (id, sid) in candidates {
            let sample = match channel {
                RecoveryChannel::Startup => RecoverySample::Startup {
                    has_output: self
                        .backend
                        .capture_tail(&sid, 3)
                        .map(|lines| !lines.is_empty())
                        .unwrap_or(false),
                    title: self.backend.pane_title(&sid).ok(),
                },
                RecoveryChannel::Activity => {
                    // A missing pane belongs to pane-death reconciliation.
                    let Some((_, at)) = activity.iter().find(|(name, _)| *name == sid) else {
                        continue;
                    };
                    RecoverySample::Activity { last_output_ms: at.saturating_mul(1000) }
                }
                RecoveryChannel::Status => RecoverySample::Status,
                RecoveryChannel::Transcript => RecoverySample::Transcript,
            };
            let Some(record) = self.board.sessions.iter().find(|record| record.id == id) else {
                continue;
            };
            let Some(recovery) = self.recovery.get_mut(&id) else { continue };
            let observations = recovery.poll(record, sample, now);
            for observation in observations {
                let principal = Principal::Automation { rule: "agent_recovery".into() };
                if matches!(
                    authorize(&principal, &Action::Mutate, &Resource::Session { id }),
                    Decision::Deny { .. }
                ) {
                    continue;
                }
                if let Some(change) =
                    self.observe_signal(id, &observation.signal, now, observation.source)
                {
                    changed |= self.apply_change(id, &change, None, Some(observation.source));
                }
                // The preview outlives the state word: apply_change may clear
                // detail, so apply the authorized preview after the transition.
                if let Some(preview) = observation.preview {
                    if let Some(record) =
                        self.board.sessions.iter_mut().find(|record| record.id == id)
                    {
                        if record.detail.as_deref() != Some(preview.as_str()) {
                            record.detail = Some(preview);
                            changed = true;
                        }
                    }
                }
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
        self.tail_of(&rec.sid16(), lines)
    }

    /// `pane_tail` for the `!` terminal (T-366): the ticket page previews
    /// the terminal before it is adopted. Only a terminal the poll (or
    /// `open_terminal`) has listed — a name nobody opened is not captured.
    fn terminal_tail(&self, ticket: Option<ulid::Ulid>, lines: u16) -> Response {
        if !self.terminals.contains_key(&ticket) {
            return Response::Err { message: "no terminal".into() };
        }
        self.tail_of(&terminal_name(ticket), lines)
    }

    fn tail_of(&self, name: &str, lines: u16) -> Response {
        let n = lines.clamp(1, MAX_PANE_TAIL_LINES) as usize;
        match self.backend.capture_tail_sized(name, n) {
            // A pane holds whatever a command decided to print, so bound what
            // rides the wire here; what is *drawable* stays the client's own
            // question, the same way transcript text is.
            Ok((v, cols)) => Response::PaneTail {
                lines: v
                    .into_iter()
                    .map(|l| l.chars().take(MAX_PANE_TAIL_COLS).collect())
                    .collect(),
                cols,
            },
            Err(e) => Response::Err { message: e.to_string() },
        }
    }

    /// How long the person inside the focused pane has been quiet (T-299).
    ///
    /// One `list-clients` fork, and only on the notification thread's ask —
    /// which happens when it is holding a line ABOUT the watched ticket and
    /// is deciding whether to swallow it, so on a board nobody is attached
    /// to it never runs at all. Same writer-thread reasoning as `pane_tail`:
    /// one small tmux fork, where the off-thread treatment exists for git.
    ///
    /// Every way of not knowing is `None`, and the caller reads `None` as
    /// away. Only a SESSION attach can answer: the `!` terminal and the gate
    /// are somebody's own shell, and T-292's suppression was never theirs.
    fn focus_quiet(&self) -> Response {
        let Some(Focus::Session(id)) = self.focus_held() else {
            return Response::FocusQuiet { quiet_ms: None };
        };
        let Some(rec) = self.board.sessions.iter().find(|r| r.id == id) else {
            return Response::FocusQuiet { quiet_ms: None };
        };
        let secs = self.backend.client_quiet_secs(&rec.sid16()).ok().flatten();
        Response::FocusQuiet { quiet_ms: secs.map(|s| s.saturating_mul(1000)) }
    }

    /// Session names for the board: latch each live pane's OSC-0 title onto
    /// its record (one batched tmux fork on the tail-poll bucket) so the TUI
    /// shows what the agent calls itself — the same source the focused tmux
    /// status line's breadcrumb leaf uses. The hostname means never-set (see
    /// `pane_title`) and a missing pane keeps the last name: latch, never
    /// clear, so sleeping/parked sessions stay recognizable.
    fn refresh_panes(&mut self) -> bool {
        let paned = self.board.sessions.iter().any(|r| r.state.has_pane());
        // A quiet board — no pane of ours, no terminal we know of — forks
        // nothing. A terminal enters the map through `open_terminal` or the
        // startup snapshot, so one that exists is always polled.
        if !paned && self.terminals.is_empty() {
            return false;
        }
        let Ok(facts) = self.backend.pane_facts() else { return false };
        let mut changed = false;
        for rec in self.board.sessions.iter_mut().filter(|r| r.state.has_pane()) {
            if crate::agents::adapter(rec.kind).is_some_and(|adapter| {
                adapter.capabilities().observation == crate::agents::ObservationMode::Structured
            }) {
                continue;
            }
            let sid = rec.sid16();
            let Some(f) = facts.iter().find(|f| f.session_name == sid) else { continue };
            let t = &f.title;
            if t.is_empty() || *t == self.hostname {
                continue;
            }
            let clean = crate::agents::adapter(rec.kind)
                .map(|adapter| adapter.normalize_title(t))
                .unwrap_or_else(|| crate::agents::pane_title(t));
            if clean.is_empty() {
                continue;
            }
            if rec.title.as_deref() != Some(clean.as_str()) {
                rec.title = Some(clean);
                changed = true;
            }
        }
        // The foregrounds (T-366): a shell record's pane is running a
        // command when tmux names something other than the shell it was
        // born with; the terminals likewise, against the daemon's `$SHELL`,
        // which is what `open_terminal` launched. Both live in maps, never on
        // the record on disk — so a change here is broadcast and NOT
        // reported as `changed`, which would persist the sessions for a
        // fact that belongs to a live pane.
        let mut foregrounds = HashMap::new();
        for rec in
            self.board.sessions.iter().filter(|r| r.kind == SessionKind::Bash && r.state.has_pane())
        {
            let sid = rec.sid16();
            let Some(f) = facts.iter().find(|f| f.session_name == sid && !f.pane_dead) else {
                continue;
            };
            let shell = rec.argv.first().map(String::as_str).unwrap_or_default();
            if let Some(cmd) = foreground_of(&f.current_command, shell) {
                foregrounds.insert(rec.id, cmd);
            }
        }
        // A `!` command in an idle Claude's composer (T-707): bash mode fires
        // no hook, keeps `#{pane_current_command}` at `claude` (the shell is a
        // detached child) and writes the transcript only when the command
        // ends, so the pane's children are the one live fact. Sampled only
        // while an idle Claude pane exists, one `ps` for all of them, and
        // only a shell younger than the idle spell counts — the MCP servers
        // and a background task's shell are older (`bash_mode_foreground`).
        let now = now_ms();
        let idle_claudes: Vec<(uuid::Uuid, i32, u64)> = self
            .board
            .sessions
            .iter()
            .filter(|r| r.kind == SessionKind::Claude && r.state.has_pane())
            .filter(|r| {
                matches!(
                    r.state,
                    SessionState::Idle {
                        stop_reason: StopReason::EndTurn
                            | StopReason::Interrupted
                            | StopReason::Unknown
                            | StopReason::Monitoring
                    }
                )
            })
            .filter_map(|r| {
                let sid = r.sid16();
                let f = facts.iter().find(|f| f.session_name == sid && !f.pane_dead)?;
                let spell = now.saturating_sub(r.state_changed_at.unwrap_or(now)) / 1000;
                (f.pane_pid > 0 && spell > 0).then_some((r.id, f.pane_pid, spell))
            })
            .collect();
        if !idle_claudes.is_empty() {
            let rows = process_rows();
            for (id, pane_pid, spell) in idle_claudes {
                if let Some(cmd) = bash_mode_foreground(pane_pid, spell, &rows) {
                    foregrounds.insert(id, cmd);
                }
            }
        }
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let terminals: std::collections::BTreeMap<_, _> = facts
            .iter()
            .filter(|f| !f.pane_dead)
            .filter_map(|f| {
                let t = terminal_ticket_of(&f.session_name)?;
                Some((t, foreground_of(&f.current_command, &shell)))
            })
            .collect();
        if foregrounds != self.foregrounds || terminals != self.terminals {
            self.foregrounds = foregrounds;
            self.terminals = terminals;
            if !changed {
                self.broadcast();
            }
        }
        changed
    }

    fn poll_tails(&mut self) -> bool {
        self.poll_agent_recovery(RecoveryChannel::Transcript)
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
            r.state.has_pane() && !r.observe_only()
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

    /// Whether a process under the private server may read the checkout
    /// (T-690), said as a standing `server_cut_off` notice while it may not.
    ///
    /// macOS keys a process's access to Documents, Desktop and Downloads to
    /// its responsible process; a server forked on the first `new-session`
    /// inherited the terminal app that opened the board, and once that app
    /// quit every pane under it read `Operation not permitted` on the
    /// checkout — agents dead at launch, workers unable to commit, nothing
    /// in mesimon changed. The probe is `ls` through `run-shell`, which runs
    /// where a pane would. Asked when a spawn dies at launch and on the
    /// server guard's cadence while the notice stands; a server that is
    /// gone clears it, since the next spawn starts a fresh one (on macOS
    /// responsible for itself, `TmuxBackend::ensure_server`).
    fn probe_server_access(&mut self) -> bool {
        let cut = match self.backend.folder_access(&self.paths.repo_root) {
            mesimon_backend_tmux::Access::Denied(why) if mesimon_backend_tmux::is_cut_off(&why) => {
                Some(why)
            }
            // Another refusal — a shell that is not POSIX, a checkout that
            // moved — is not macOS's cut-off and raises no notice that says
            // it is; the journal keeps the words.
            mesimon_backend_tmux::Access::Denied(why) => {
                self.journal.line(&format!(
                    "server probe: ls under the private tmux server failed on {}: {why}",
                    self.paths.repo_root.display()
                ));
                None
            }
            mesimon_backend_tmux::Access::NoServer | mesimon_backend_tmux::Access::Readable => None,
        };
        let standing = self.notices.iter().any(|n| n.kind == SERVER_CUT_OFF);
        if cut.is_some() == standing {
            return false;
        }
        self.notices.retain(|n| n.kind != SERVER_CUT_OFF);
        match cut {
            Some(why) => {
                self.journal.line(&format!(
                    "server cut off: a process under the private tmux server cannot read {}: {why}",
                    self.paths.repo_root.display()
                ));
                self.notices.push(
                    Notice::new(
                        SERVER_CUT_OFF,
                        "macOS cut the private tmux server off from this folder ∙ the Esc menu restarts it",
                    )
                    .with_path(self.paths.repo_root.display())
                    .with_detail(why),
                );
            }
            None => self.journal.line(
                "server cut off: cleared, a process under the server reads the checkout again",
            ),
        }
        true
    }

    /// The probe again, only while its notice stands: a server restarted by
    /// any road (the menu, a `kill-server` by hand) takes the notice down.
    fn recheck_server_access(&mut self) -> bool {
        if !self.notices.iter().any(|n| n.kind == SERVER_CUT_OFF) {
            return false;
        }
        self.probe_server_access()
    }

    /// The Esc menu's `Restart the private tmux server` (T-690).
    ///
    /// Every paned session parks by the road `x` takes — the conversation
    /// snapshotted, the record `Sleeping`, SIGTERM then the reaper — a shell
    /// closes as `x` closes it, and the server itself goes in
    /// `finish_server_restart` once the panes are reaped, so no agent is
    /// SIGHUP'd mid-exit. Refused whole, before anything parks, when any
    /// session is not parkable: an agent mid-turn, a shell with live
    /// children. Automatic restart was not built: the measured safe moment
    /// is "no pane mid-turn", which is this gate, and the person is the one
    /// who knows whether a turn is about to start.
    fn restart_server(&mut self) -> Response {
        let now = now_ms();
        let paned: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|r| r.state.has_pane() && !r.observe_only())
            .map(|r| r.id)
            .collect();
        for id in &paned {
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == *id) else { continue };
            if let Err(why) = self.sleep_eligible(rec, now, false) {
                let key = self
                    .board
                    .ticket(rec.ticket)
                    .map_or_else(|| "a ticket".to_string(), |t| t.short_key.clone());
                let who = if rec.kind.is_agent() { "agent" } else { "shell" };
                return Response::Err {
                    message: format!("{key}'s {who} cannot park yet — {why}; nothing restarted"),
                };
            }
        }
        for id in &paned {
            if let Err(why) = self.sleep_one(*id, false) {
                self.journal.line(&format!("server restart: session {id} did not park: {why}"));
            }
        }
        self.server_restart = Some(Instant::now() + REAP_GRACE + Duration::from_secs(2));
        self.journal.line(&format!(
            "server restart asked: {} session(s) parked; the server goes once they are reaped",
            paned.len()
        ));
        self.persist_and_notify();
        Response::Ok
    }

    /// The second half of `restart_server`: once the reaper has taken every
    /// pane (or the deadline passes on one that would not go), kill the
    /// server. The `!` terminals go with it — a shell holds nothing to
    /// park. The cut-off notice comes down here; the next spawn or wake
    /// starts a fresh server.
    fn finish_server_restart(&mut self) -> bool {
        let Some(due) = self.server_restart else { return false };
        if !self.reaping.is_empty() && Instant::now() < due {
            return false;
        }
        self.server_restart = None;
        match self.backend.kill_server() {
            Ok(()) => self.journal.line(
                "server restart: the private tmux server was killed; the next spawn or wake starts a fresh one",
            ),
            Err(e) => self.journal.line(&format!("server restart: kill-server failed: {e}")),
        }
        self.foregrounds.clear();
        self.notices.retain(|n| n.kind != SERVER_CUT_OFF);
        true
    }

    fn poll_codex(&mut self) {
        if self.codex_polling {
            return;
        }
        let paths: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter(|s| s.owns_codex_runtime())
            .map(|s| (s.id, crate::agents::codex::snapshot_path(&self.paths, s.id)))
            .collect();
        if paths.is_empty() {
            return;
        }
        // One observer for the daemon's life (T-688), where a thread per
        // tick re-read every file whether or not it changed. An observer
        // that ended (its last report found no daemon) is replaced.
        let observer = self.codex_observer.get_or_insert_with(|| {
            let tx = self.tx.clone();
            crate::agents::codex::spawn_observer(move |snapshots| {
                tx.send(Msg::CodexSnapshots(snapshots)).is_ok()
            })
        });
        if observer.send(paths).is_ok() {
            self.codex_polling = true;
        } else {
            self.codex_observer = None;
        }
    }

    fn on_codex_snapshots(
        &mut self,
        snapshots: Vec<(uuid::Uuid, Option<crate::agents::codex::Snapshot>)>,
    ) {
        self.codex_polling = false;
        let now = now_ms();
        let mut dirty = false;
        let mut plans = Vec::new();
        let mut stopped = Vec::new();
        let mut quota = Vec::new();
        for (id, snapshot) in snapshots {
            let principal = Principal::Automation { rule: "codex_observer".into() };
            if matches!(
                authorize(&principal, &Action::Mutate, &Resource::Session { id }),
                Decision::Deny { .. }
            ) {
                continue;
            }
            let Some(rec) = self
                .board
                .sessions
                .iter_mut()
                .find(|s| s.id == id && s.kind == SessionKind::Codex && s.holds_process())
            else {
                continue;
            };
            let valid = snapshot.filter(|s| {
                s.session == id
                    && Some(s.generation) == rec.codex_generation
                    && (s.stopped
                        || (now.saturating_sub(s.heartbeat_ms) <= 5_000
                            && s.heartbeat_ms <= now + 1_000))
            });
            let Some(mut snapshot) = valid else {
                self.codex_ready.remove(&id);
                // A new runtime gets a bounded startup grace, but holds the
                // checkout throughout it. Lost evidence never means done.
                if rec.state == SessionState::Spawning
                    && now.saturating_sub(rec.state_changed_at.unwrap_or(now)) < 30_000
                {
                    continue;
                }
                dirty |= !rec.observation_hold;
                rec.observation_hold = true;
                rec.codex_pending_seq = None;
                if rec.codex_stopping {
                    continue;
                }
                if let Some(machine) = self.machines.get_mut(&id) {
                    if let Some(change) = machine.apply(&Signal::ObservationLost, now) {
                        dirty |=
                            self.apply_change(id, &change, None, Some("codex_observation_lost"));
                    }
                }
                continue;
            };
            if snapshot.sequence < rec.codex_observed_seq {
                continue;
            }
            // The session's own quota report (T-327), taken after the loop.
            if let Some(limits) = snapshot.rate_limits.take() {
                quota.push((snapshot.rate_limits_at_ms, limits));
            }
            if snapshot.thread_id.is_some() && snapshot.thread_id != rec.codex_thread_id {
                dirty |= rec.title.is_some();
                rec.title = None;
            }
            if let (Some(title), Some(adapter)) =
                (&snapshot.title, crate::agents::adapter(rec.kind))
            {
                let clean = adapter.normalize_title(title);
                if !clean.is_empty() && rec.title.as_ref() != Some(&clean) {
                    rec.title = Some(clean);
                    dirty = true;
                }
            }
            // Closing the native local plan dialog is not a successful turn.
            // Preserve the exact dismissed turn across daemon handover while
            // the runtime continues reporting its structured plan hold.
            if snapshot.turn_id.is_some()
                && snapshot.turn_id == rec.codex_plan_dismissed_turn
                && snapshot.state
                    == (SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan })
            {
                snapshot.state = SessionState::Idle { stop_reason: StopReason::Unknown };
                snapshot.observation_hold = false;
            }
            if let (Some(plan), Some(key)) = (&snapshot.plan, &snapshot.plan_key) {
                if rec.agent_plan_key.as_ref() != Some(key) {
                    plans.push((id, key.clone(), plan.clone()));
                }
            }
            if snapshot.stopped {
                stopped.push(id);
                self.codex_ready.remove(&id);
                dirty |= rec.codex_stopping || rec.observation_hold || rec.pending_submit;
                rec.codex_stopping = false;
                rec.observation_hold = false;
                rec.pending_submit = false;
                rec.pending_prefill = false;
                rec.codex_submit_sent = false;
                rec.codex_pending_seq = None;
                rec.codex_observed_seq = snapshot.sequence;
                continue;
            }
            if rec.codex_stopping {
                continue;
            }
            if !snapshot.observation_hold && matches!(snapshot.state, SessionState::Idle { .. }) {
                self.codex_ready.insert(id);
            } else {
                self.codex_ready.remove(&id);
            }
            let newer = snapshot.sequence > rec.codex_observed_seq;
            let new_turn = snapshot.turn_id.is_some() && snapshot.turn_id != rec.codex_turn_id;
            let ticket = rec.ticket;
            let held = snapshot.observation_hold || (rec.pending_submit && !new_turn);
            dirty |= rec.observation_hold != held;
            rec.observation_hold = held;
            if let Some(thread) = snapshot.thread_id {
                dirty |= rec.codex_thread_id.as_ref() != Some(&thread);
                rec.codex_thread_id = Some(thread);
            }
            if let Some(history) = snapshot.history_path {
                rec.transcript_path = Some(history);
            }
            if !newer {
                // Restore the current projection after handover without
                // replaying an already observed completion as a new automove.
                if matches!(rec.state, SessionState::Unknown { .. }) && !snapshot.observation_hold {
                    rec.state = snapshot.state.clone();
                    rec.confidence = Confidence::High;
                    self.machines
                        .insert(id, Machine::restore(snapshot.state, Confidence::High, now));
                    dirty = true;
                }
                continue;
            }
            rec.codex_pending_seq = Some(snapshot.sequence);
            rec.codex_turn_id = snapshot.turn_id;
            dirty = true;
            if new_turn {
                rec.codex_plan_dialog_seen = false;
                rec.codex_plan_dismissed_turn = None;
                rec.pending_prefill = false;
                rec.pending_submit = false;
                rec.codex_submit_sent = false;
                self.codex_input_due.remove(&id);
                self.moves.asked_by_hand(ticket);
                // The Codex counterpart of `UserPromptSubmit`: whatever was
                // owed on this pane, a turn began.
                self.ack_owed(id);
                self.lower_hand_on(ticket);
            }
            let signal = match snapshot.state {
                SessionState::Spawning => continue,
                SessionState::Running => Signal::TurnStarted,
                SessionState::Idle { stop_reason: StopReason::EndTurn } => {
                    Signal::TurnEnded { outcome: attention::TurnOutcome::Completed }
                }
                SessionState::Idle { stop_reason: StopReason::Interrupted } => {
                    Signal::TurnEnded { outcome: attention::TurnOutcome::Interrupted }
                }
                SessionState::Idle { .. } => Signal::Ready,
                SessionState::RequiresAction { reason } => Signal::Attention { reason },
                SessionState::Failed { reason } => {
                    Signal::TurnEnded { outcome: attention::TurnOutcome::Failed(reason) }
                }
                SessionState::Unknown { .. } => Signal::ObservationLost,
                // Runtime lifecycle cannot silently park or kill a board
                // record; the tmux supervisor lifecycle owns those transitions.
                _ => continue,
            };
            let machine = self
                .machines
                .entry(id)
                .or_insert_with(|| Machine::new(SessionState::unknown(), now));
            let (change, decision) = machine.apply_explained(&signal, now);
            if change.is_none() && machine.view().pending.is_none() {
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.codex_observed_seq = snapshot.sequence;
                    rec.codex_pending_seq = None;
                }
            }
            self.feed.state_decision(id, "codex", &decision);
            if let Some(change) = change {
                dirty |= self.apply_change(id, &change, None, Some("codex"));
            }
        }
        let orphans: Vec<_> = stopped
            .into_iter()
            .filter(|id| {
                self.board.sessions.iter().find(|session| session.id == *id).is_some_and(
                    |session| {
                        self.board.ticket(session.ticket).is_none()
                            && !self.grace.contains_key(&session.ticket)
                    },
                )
            })
            .collect();
        self.board.sessions.retain(|session| !orphans.contains(&session.id));
        dirty |= !orphans.is_empty();
        for (id, key, plan) in plans {
            self.record_plan(id, plan);
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.agent_plan_key = Some(key);
                dirty = true;
            }
        }
        let mut quota_moved = false;
        for (at, limits) in quota {
            if let Some(reading) = mesimon_core::usage::parse_codex(&limits, at) {
                quota_moved |= self.usage.passive(Provider::Codex, reading);
            }
        }
        if dirty {
            self.persist_and_notify();
        } else if quota_moved {
            self.broadcast();
        }
    }

    fn probe_codex_startup(&mut self, now: u64) -> bool {
        let candidates: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.kind == SessionKind::Codex
                    && s.state.has_pane()
                    && !s.argv.is_empty()
                    && (s.pending_prefill
                        || self.codex_native_ready.get(&s.id) != Some(&s.codex_generation)
                        || matches!(
                            s.state,
                            SessionState::RequiresAction {
                                reason: mesimon_core::board::Reason::Plan
                                    | mesimon_core::board::Reason::Trust
                                    | mesimon_core::board::Reason::Auth,
                            }
                        ))
            })
            .map(|s| (s.id, s.sid16(), s.state.clone(), s.codex_generation))
            .collect();
        let mut dirty = false;
        for (id, sid, state, generation) in candidates {
            let principal = Principal::Automation { rule: "codex_startup".into() };
            if matches!(
                authorize(&principal, &Action::Mutate, &Resource::Session { id }),
                Decision::Deny { .. }
            ) {
                continue;
            }
            let Ok(screen) = self.backend.capture_input_screen(&sid) else { continue };
            if crate::agents::codex::input_ready(&screen) {
                self.codex_native_ready.insert(id, generation);
            }
            let signal = if state
                == (SessionState::RequiresAction { reason: mesimon_core::board::Reason::Plan })
            {
                // An Enter of ours is in flight on this dialog (T-420): the
                // composer coming back is the accepted plan's turn starting,
                // not a dismissal, and the observation stream says which.
                if self.plan_accept.contains_key(&id) {
                    continue;
                }
                let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
                    continue;
                };
                if crate::agents::codex::plan_dialog(&screen) {
                    dirty |= !rec.codex_plan_dialog_seen;
                    rec.codex_plan_dialog_seen = true;
                    continue;
                }
                if !rec.codex_plan_dialog_seen || !crate::agents::codex::input_ready(&screen) {
                    continue;
                }
                rec.codex_plan_dismissed_turn = rec.codex_turn_id.clone();
                rec.codex_plan_dialog_seen = false;
                rec.observation_hold = rec.pending_submit;
                self.codex_ready.insert(id);
                dirty = true;
                Signal::Ready
            } else if let Some(reason) = crate::agents::codex::startup_attention(&screen) {
                Signal::Attention { reason }
            } else if self.codex_ready.contains(&id)
                && crate::agents::codex::input_ready(&screen)
                && matches!(
                    state,
                    SessionState::RequiresAction {
                        reason: mesimon_core::board::Reason::Trust
                            | mesimon_core::board::Reason::Auth
                    }
                )
            {
                Signal::Ready
            } else {
                continue;
            };
            if let Some(machine) = self.machines.get_mut(&id) {
                if let Some(change) = machine.apply(&signal, now) {
                    dirty |= self.apply_change(id, &change, None, Some("codex_startup"));
                }
            }
        }
        dirty
    }

    /// Paste only into an independently visible native input, then send one
    /// Enter after paste detection settles. The persisted sent latch survives
    /// daemon replacement; an unacknowledged submission holds the checkout.
    fn drive_codex_inputs(&mut self, now: u64) -> bool {
        let ids: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.kind == SessionKind::Codex
                    && s.state.has_pane()
                    && (s.pending_prefill || s.pending_submit || self.parked(s.id))
            })
            .map(|s| s.id)
            .collect();
        let mut dirty = false;
        for id in ids {
            if !self.control_delivery_allowed(id) {
                continue;
            }
            let paired = self.control_delivery_principal(id);
            let action = if paired.is_some() { Action::PromptExisting } else { Action::Mutate };
            let principal =
                paired.unwrap_or(Principal::Automation { rule: "agent_prompt_delivery".into() });
            if matches!(
                authorize(&principal, &action, &Resource::Session { id }),
                Decision::Deny { .. }
            ) {
                continue;
            }
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { continue };
            if rec.codex_submit_sent
                || !self.codex_ready.contains(&id)
                || !matches!(rec.state, SessionState::Idle { .. })
            {
                continue;
            }
            let sid = rec.sid16();
            let ticket = rec.ticket;
            let pending_prefill = rec.pending_prefill;
            let submit = rec.pending_submit;
            let Ok(screen) = self.backend.capture_input_screen(&sid) else { continue };
            if !crate::agents::codex::input_ready(&screen) {
                continue;
            }
            if let Some(due) = self.codex_input_due.get(&id).copied() {
                if now < due {
                    continue;
                }
                if submit && self.backend.send_enter(&sid).is_ok() {
                    self.control_submitted(id);
                    if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                        rec.codex_submit_sent = true;
                        rec.observation_hold = true;
                    }
                    dirty = true;
                }
                self.codex_input_due.remove(&id);
                continue;
            }
            let mut text = if pending_prefill {
                self.board.ticket(ticket).map(|t| t.title.clone()).unwrap_or_default()
            } else {
                String::new()
            };
            let parked = self.owed.get(&id).and_then(|o| o.parked.as_ref());
            let brief = parked.is_some_and(|p| p.brief);
            if brief {
                if let Some(body) = self.description_body(ticket) {
                    if !body.is_empty() {
                        text.push_str("\n\n");
                        text.push_str(&body);
                    }
                }
            }
            if let Some(parked) = parked {
                if !parked.text.is_empty() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(&parked.text);
                }
            }
            if !text.is_empty() && self.backend.paste_input(&sid, &text).is_err() {
                continue;
            }
            self.control_pasted(id);
            // The words are in; the entry stays for its ack (the new-turn
            // edge), so a queued ask's word still lands on the feed.
            if let Some(owed) = self.owed.get_mut(&id) {
                owed.parked = None;
            }
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.pending_prefill = false;
                rec.ticket_read |= brief;
            }
            if submit {
                self.codex_input_due.insert(id, now + 500);
            }
            dirty = true;
        }
        dirty
    }

    /// One hook frame off the ingest socket (Claude hook or tmux pane-died).
    fn on_hook(&mut self, frame: HookFrame) {
        // Every received frame is feed-logged by NAME only — never its
        // payload (D11: prompt text is read, never stored).
        // A hook-set frame for a session whose pane reports through the mod
        // (T-577) came from nothing mesimon launched there: the pane has no
        // hook set, or a native one's holds `PermissionRequest` alone
        // (T-658), the one event its mod does not relay. Tmux's `pane-died`
        // and the gate's and the approve's reports are not the hook set's
        // events and pass.
        if frame.road == Road::Hooks
            && mesimon_core::road::RELAYED_EVENTS.contains(&frame.event.as_str())
            && self
                .resolve_session(&frame.session)
                .and_then(|id| self.board.sessions.iter().find(|s| s.id == id))
                .is_some_and(|rec| {
                    rec.frames_by_mod() && !(rec.native && frame.event == "PermissionRequest")
                })
        {
            return;
        }
        self.feed.hook_event_by(&frame.session, &frame.event, frame.reason.as_deref(), frame.road);
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
        self.control_release_permission(id, &frame);
        self.control_observe_dialog(id, &frame);
        let now = now_ms();
        let mut observation = self
            .board
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .and_then(|record| {
                // Shell panes historically accept the hook protocol too:
                // an agent launched inside one can report attention and exit.
                // Keep that compatibility without giving shells agent-seat
                // capabilities or interpreting Claude hooks for Codex.
                let hook_kind = match record.kind {
                    SessionKind::Bash => SessionKind::Claude,
                    kind => kind,
                };
                crate::agents::adapter(hook_kind).map(|adapter| adapter.parse_hook(&frame, record))
            })
            .unwrap_or_default();
        // Pane death is transport lifecycle shared by shells and both agents.
        if frame.event == "PaneDied" {
            observation.signal = Some(Signal::PaneDied {
                status: frame.reason.as_deref().and_then(|s| s.parse().ok()),
            });
        }
        let mut dirty = observation.metadata_changed;
        let signal = observation.signal;
        if let Some(sig) = signal {
            // A death that names ANOTHER pane is the previous tenant's. A
            // wake re-uses the record's session name and its session uuid
            // for the new pane; sleep SIGTERMs and returns, so an ask, a `c`
            // or the very next `x` spawns into the name while the old
            // process is still going down — its `SessionEnd` hook and the
            // pane-died behind it are still on their way up the hook socket,
            // and read by name they land on the new pane's record as its
            // death (prompt_e2e, 2026-09-04; T-381). A pane key — server
            // pid and pane id, `conf::PANE_KEY` — is never reused, so the
            // frame carries it (`--pane` on the tmux hook, `TMUX` and
            // `TMUX_PANE` through a Claude hook) and the record keeps the
            // one it spawned: a mismatch is dropped, no tmux asked (T-245).
            // A frame or a record without one is trusted as before.
            if let Some((theirs, ours)) = self.straggler_death(id, &sig, frame.pane.as_deref()) {
                self.journal.line(&format!(
                    "straggler dropped: session {id} {} from pane {theirs}, record's pane is {ours}",
                    frame.event
                ));
                self.feed.hook_event(&frame.session, &frame.event, Some("straggler"));
                return;
            }
            if matches!(sig, Signal::PaneDied { .. }) {
                if let Some(record) = self
                    .board
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == id && s.kind == SessionKind::Codex)
                {
                    record.codex_stopping = true;
                    record.observation_hold = true;
                    dirty = true;
                }
            }
            // A prompt reached the agent — every road ends here: the board's
            // Shift+Enter field, the composer's submit, a line typed in the
            // pane. The person's own last move on the ticket stops being one
            // the no-undo rule protects, BEFORE the `Running` edge below asks
            // automove to reverse it (T-186). An agent's move keeps its guard.
            // The quota moves at a turn's end, and a rate-limit stop means a
            // window is full (T-327). A Claude hook is Claude's quota.
            match &sig {
                Signal::Stop { .. } => {
                    self.usage.turn_ended(Provider::Claude);
                    self.cost_due = true;
                }
                Signal::StopFailure { class: attention::StopFailureClass::RateLimit } => {
                    self.usage.limited(Provider::Claude)
                }
                _ => {}
            }
            if matches!(sig, Signal::UserPromptSubmit) {
                if let Some(t) = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket) {
                    self.moves.asked_by_hand(t);
                    // Our own paste's ack — the owed Enter paid, by us or by
                    // the user typing their own — or the user talking to the
                    // agent while an ask waited, which drops it (2026-09-04).
                    dirty |= self.ack_owed(id);
                    // A raised hand has been answered (T-107). Same road,
                    // same reasoning: whoever typed, the agent is no longer
                    // waiting on a person.
                    dirty |= self.lower_hand_on(t);
                }
            }
            // A pane that dies within a second of its spawn, with a status,
            // is the one symptom macOS's cut-off has from the board (T-690):
            // ask the server whether a process under it may read the
            // checkout, and say so. Judged before the machine moves the
            // record off `Spawning`.
            let died_at_launch = matches!(sig, Signal::PaneDied { status: Some(s) } if s != 0)
                && self.board.sessions.iter().any(|s| {
                    s.id == id
                        && matches!(s.state, SessionState::Spawning)
                        && now.saturating_sub(s.state_changed_at.unwrap_or(0)) <= LAUNCH_DEATH_MS
                });
            let machine = self
                .machines
                .entry(id)
                .or_insert_with(|| Machine::new(SessionState::unknown(), now));
            let (change, decision) = machine.apply_explained(&sig, now);
            self.feed.state_decision(id, &frame.event, &decision);
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
                dirty |= self.apply_change(id, &change, observation.detail, Some(&frame.event));
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
            if died_at_launch {
                dirty |= self.probe_server_access();
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
                dirty |= self.arm_owed(id, now);
            }
        }
        // An approved plan is the agent's note on the ticket.
        if let Some(plan) = observation.plan {
            dirty |= self.record_plan(id, plan);
        }
        if dirty {
            self.persist_sessions();
            self.broadcast();
        }
    }

    /// Publish the adapter's authoritative plan through normal agent-note
    /// authorization. Claude supplies an approved ExitPlanMode plan; Codex
    /// supplies a completed plan item, which may still await native approval.
    /// One note per session is revised on each new plan. A user-deleted note
    /// gets a fresh identity on the next plan. Content is sanitized and capped
    /// by write_note, and the feed records metadata only.
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
        // The plan is a capture from a hook frame, not a caller who can act
        // on a refusal: a plan past the note limit lands cut rather than not
        // at all (the whole plan is in the transcript). Callers are refused.
        let plan = mesimon_core::board::sanitize_note(&plan);
        let Response::NoteWritten { note: Some(id) } =
            self.write_note(ticket, existing, plan, None, &by)
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

    // ------------------------------------------------------------ owed pastes

    /// Park words on a record whose pane is on its way — the composed
    /// spawn's brief, the board's ask at a sleeping claude, a Codex pane's
    /// paste — and owe the Enter: the first tick after `SessionStart`
    /// pastes them (`settle_owed`). `pending_submit` goes on the record for
    /// the card's launching arc (`glyphs::is_launching`) and, for Codex, as
    /// the persisted checkout hold. A launch road — a wake, a start — acks
    /// as the user's prompt landing (`Ack::PROMPT`) whatever asked for it;
    /// only a Codex pane's paste carries its caller's word, as a Claude
    /// pane's does.
    fn park(&mut self, id: uuid::Uuid, ticket: ulid::Ulid, parked: Parked, ack: Ack) {
        let mut owed = Owed::launch(ticket, parked, ack);
        owed.mod_road = self.on_mod_road(id);
        self.owed.insert(id, owed);
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = true;
            rec.codex_submit_sent = false;
        }
        self.persist_and_notify();
    }

    /// Words parked on this session and not yet pasted.
    fn parked(&self, id: uuid::Uuid) -> bool {
        self.owed.get(&id).is_some_and(|o| o.parked.is_some())
    }

    /// Anything owed on any of the ticket's sessions.
    fn owed_on(&self, ticket: ulid::Ulid) -> bool {
        self.owed.values().any(|o| o.ticket == ticket)
    }

    /// A queued ask's paste on this ticket, waiting for its ack — the one
    /// entry the snapshot shows as `queued ∙ sending` and the ask receipts
    /// count as `sent`. A queued Start or Wake owes `Ack::PROMPT` instead
    /// and is read from its record.
    fn ask_in_flight(&self, ticket: ulid::Ulid) -> bool {
        self.owed.values().any(|o| o.ticket == ticket && o.ack.word == Ack::QUEUED.word)
    }

    /// Keep what a launch road could not deliver on its Claude seat (T-570),
    /// for the card, the crown and the seat's Shift+Enter (`resend_unsent`).
    /// A seat with no pane has nothing to resend into and says so in its
    /// own state.
    fn mark_unsent(&mut self, id: uuid::Uuid, words: Option<Parked>) -> bool {
        let Some(Parked { text, brief, .. }) = words else { return false };
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        if rec.kind != SessionKind::Claude || !rec.state.has_pane() {
            return false;
        }
        rec.unsent = Some(mesimon_core::board::Unsent { text, brief });
        true
    }

    /// The seat's Shift+Enter over unsent words (T-570): park them again on
    /// the owed road, armed now — the pane is long past `SessionStart` — with
    /// the composer wait and one Ctrl+C owed for a box holding what the
    /// failed start left. A brief is the ticket's title and description read
    /// anew, as the first paste read them (T-117). The mark stays until the
    /// ack, so a resend that fails again still says so.
    fn resend_unsent(&mut self, ticket: ulid::Ulid) -> Response {
        let Some(rec) = self.board.pane_target(ticket) else {
            return Response::Err {
                message: "no live agent session on this ticket — start or wake one first".into(),
            };
        };
        let Some(unsent) = rec.unsent_words().cloned() else {
            return Response::Err { message: "nothing unsent on this seat".into() };
        };
        // A dialog's Enter is an answer (`SessionRecord::pressable`): the
        // person answers it in the pane, then resends.
        if !matches!(
            rec.state,
            SessionState::Spawning | SessionState::Idle { .. } | SessionState::Running
        ) {
            return Response::Err {
                message: format!(
                    "{} is waiting on a dialog ∙ answer it in the pane, then resend",
                    mesimon_core::keymap::AGENT_WORD
                ),
            };
        }
        let id = rec.id;
        if self.owed.contains_key(&id) {
            return Response::Err {
                message: "a prompt is already waiting for this session".into(),
            };
        }
        let parked = Parked { text: unsent.text, brief: unsent.brief, title: unsent.brief };
        self.send_launch_words(id, ticket, parked, "prompt_resent")
    }

    /// Launch words for a seat whose pane is long past `SessionStart`: a
    /// resend (T-570), or the brief of a conversation that never took a
    /// prompt (T-603). Armed now, with the composer wait and one Ctrl+C owed
    /// for a box holding text — what a failed start left, or the title a
    /// plain start typed.
    fn send_launch_words(
        &mut self,
        id: uuid::Uuid,
        ticket: ulid::Ulid,
        parked: Parked,
        word: &'static str,
    ) -> Response {
        let now = now_ms();
        // A session whose mod is up takes the words by its `submit` (T-575):
        // nothing is typed, so there is no stray text to clear first, and
        // whatever the box holds stays the person's. One whose mod never
        // came up — the reason its words went unsent, most likely — takes
        // the paste road below.
        if self.mod_speaks(id, "submit") && !road::is_command(&self.launch_words(ticket, &parked)) {
            if !self.mod_submit(id, ticket, parked, Ack::PROMPT) {
                return Response::Err { message: "nothing to send".into() };
            }
            // The card's launching arc, as a resend on the paste road draws.
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.pending_submit = true;
            }
            self.feed.board("local", word, Some(ticket));
            self.persist_and_notify();
            return Response::Ok;
        }
        let mut owed = Owed::launch(ticket, parked, Ack::PROMPT);
        owed.next_press = Some(now);
        owed.ready_by = Some(now + composer_wait_ms());
        owed.clear_first = true;
        self.owed.insert(id, owed);
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = true;
        }
        self.feed.board("local", word, Some(ticket));
        self.persist_and_notify();
        Response::Ok
    }

    /// A Claude seat whose conversation never took a prompt (T-603), by the
    /// cheap reading the snapshot can afford on every build: idle at the
    /// composer `SessionStart` left (or parked), nothing owed or unsent,
    /// and no file at the transcript path `SessionStart` named — Claude
    /// Code writes none before the first prompt (measured on 2.1.288).
    fn unprompted_hint(&self, rec: &mesimon_core::board::SessionRecord) -> bool {
        rec.kind == SessionKind::Claude
            && matches!(
                rec.state,
                SessionState::Idle { stop_reason: StopReason::Unknown } | SessionState::Sleeping
            )
            && rec.unsent.is_none()
            && !self.owed.contains_key(&rec.id)
            && rec.transcript_path.as_deref().is_some_and(|p| !std::path::Path::new(p).is_file())
    }

    /// `unprompted_hint`, confirmed the way a wake judges a conversation
    /// gone (`history_missing`, which also looks where a conversation that
    /// moved with its cwd went): the reading a blank ask acts on.
    fn unprompted(&self, id: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return false };
        self.unprompted_hint(rec)
            && crate::agents::adapter(rec.kind).is_some_and(|a| a.history_missing(rec))
    }

    /// A blank ask at a seat that never took a prompt (T-603): the ticket's
    /// title and description, as a start sends them. A plain start (`c`)
    /// typed the title and left it for the person, and its wake came up on
    /// an empty composer; the field's blank Enter there did nothing at all.
    /// It goes now whatever the toggle says: there is no turn of this seat's
    /// to wait for, as a worktree's start never waits.
    fn send_brief(&mut self, ticket: ulid::Ulid, seat: QueuedSeat, plan: bool) -> Response {
        let brief = Parked { text: String::new(), brief: true, title: true };
        match seat {
            QueuedSeat::Pane(id) if plan || self.tier_owed(id) => {
                self.relaunch_with(ticket, id, Some(brief), plan, "local")
            }
            QueuedSeat::Pane(id) => {
                // The title a plain start typed would sit in the box under
                // the brief's own turn on the mod road, where nothing is
                // pasted over it: the paste road's one Ctrl+C, into a box
                // seen holding text, clears it there too.
                if self.mod_speaks(id, "submit") {
                    use crate::agents::claude::composer::{self, Composer};
                    if let Some(sid) =
                        self.board.sessions.iter().find(|s| s.id == id).map(|s| s.sid16())
                    {
                        let holding = self
                            .backend
                            .capture_input_screen(&sid)
                            .is_ok_and(|screen| composer::read(&screen) == Composer::Holding);
                        if holding {
                            let _ = self.backend.clear_input(&sid);
                        }
                    }
                }
                self.send_launch_words(id, ticket, brief, "prompt_brief")
            }
            QueuedSeat::Wake(_) => self.wake_with(ticket, brief, plan),
            QueuedSeat::Start(_) => Response::Err { message: "nothing to send".into() },
        }
    }

    /// The words a launch road delivers, as they stand NOW: a brief is the
    /// ticket's description read at delivery (T-117), so a ticket described
    /// after its spawn still gets it, and no description means the title
    /// alone. The words a Shift+Enter carried into an empty seat (T-294) ride
    /// UNDER the brief, in the order they were written: the ticket says what
    /// the work is, the user says what to do about it first. `title` leads
    /// with the ticket's title: a resend, whose Ctrl+C cleared the one the
    /// spawn typed, and a mod launch, which types none (T-575); the spawn's
    /// own paste goes under the one it typed.
    fn launch_words(&self, ticket: ulid::Ulid, parked: &Parked) -> String {
        let Parked { text, brief, title } = parked;
        let text = if *brief {
            let brief =
                self.description_body(ticket).map(|b| format!("\n\n{b}")).unwrap_or_default();
            match (brief.is_empty(), text.is_empty()) {
                (_, true) => brief,
                (true, false) => format!("\n\n{text}"),
                (false, false) => format!("{brief}\n\n{text}"),
            }
        } else {
            text.clone()
        };
        if *title {
            let lead = self.board.ticket(ticket).map_or("", |t| t.title.trim());
            format!("{lead}{text}").trim_start().to_string()
        } else {
            text
        }
    }

    /// Whether this session is a Claude launch on the mod road (T-575): its
    /// parked words wait for its mod's bridge rather than its composer.
    fn on_mod_road(&self, id: uuid::Uuid) -> bool {
        self.board
            .sessions
            .iter()
            .any(|s| s.id == id && s.kind == SessionKind::Claude && s.road == Road::Mod)
    }

    /// Send words down the session's mod as a `submit` (T-575):
    /// `$.prompt.submit({ text, asUser: true })` in the mod, a turn of its
    /// own, never typed into the pane, so no composer is read and no Enter is
    /// pressed. The ack is still `UserPromptSubmit`. `false` when the words
    /// come to nothing.
    fn mod_submit(&mut self, id: uuid::Uuid, ticket: ulid::Ulid, words: Parked, ack: Ack) -> bool {
        let text = self.launch_words(ticket, &words);
        if text.is_empty() {
            return false;
        }
        // The brief went in with the first prompt, as a paste's does; a
        // ticket with no description sent its title alone, and nothing was
        // read.
        let read = words.brief && self.description_body(ticket).is_some();
        let frame = self.mod_enqueue(id, ModCommand::Submit { text });
        self.owed.insert(id, Owed::submitted(ticket, words, frame, ack, now_ms()));
        if read {
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.ticket_read = true;
            }
        }
        self.feed.board("daemon", "prompt_by_mod", Some(ticket));
        true
    }

    /// Words for Claude Code's send-now down the session's mod (T-601): a
    /// `fill` puts them in its empty composer, and its `filled` is the cue
    /// for the keys (`on_mod_fill`). Owed as a `submit` is: the ack is the
    /// `UserPromptSubmit` the send fires. `false` when the words come to
    /// nothing.
    fn mod_fill(&mut self, id: uuid::Uuid, ticket: ulid::Ulid, words: Parked, ack: Ack) -> bool {
        let text = self.launch_words(ticket, &words);
        if text.is_empty() {
            return false;
        }
        let frame = self.mod_enqueue(id, ModCommand::Fill { text });
        let mut owed = Owed::submitted(ticket, words, frame, ack, now_ms());
        owed.fill = true;
        self.owed.insert(id, owed);
        self.feed.board("daemon", "prompt_by_mod_fill", Some(ticket));
        true
    }

    /// The mod's report on a `fill` (T-601), the frame's id as its reason.
    /// `filled`: the words stand in the composer, and the send-now goes in.
    /// Refused (the person's draft in the box, a dialog) or thrown: the words
    /// still go at once, by the plain `submit`, which the engine holds to the
    /// running turn's end, and the feed says why the level fell.
    pub(super) fn on_mod_fill(&mut self, id: uuid::Uuid, frame: &HookFrame) {
        let Some(fid) = frame.reason.as_deref() else { return };
        let outcome = frame.payload["outcome"].as_str().unwrap_or("");
        let ours = self.owed.get(&id).is_some_and(|o| o.fill && o.frame.as_deref() == Some(fid));
        if !ours {
            self.journal.line(&format!("mod fill {fid} for session {id}: {outcome}, nothing owed"));
            return;
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { return };
        let sid = rec.sid16();
        if outcome == "filled" {
            self.mod_taken(id, fid);
            let ticket = self.owed.get(&id).map(|o| o.ticket);
            match self.backend.send_now(&sid) {
                Ok(()) => self.feed.board("daemon", "prompt_sent_immediately", ticket),
                Err(e) => {
                    // The words stand in the box; the person sends them.
                    self.journal.line(&format!("send-now for session {id} failed: {e}"));
                    self.feed.board_outcome("daemon", "prompt_send_now_failed", ticket, "keys");
                }
            }
            self.persist_and_notify();
            return;
        }
        let said = frame.payload["reason"].as_str().or(frame.payload["error"].as_str());
        let why = said.map(|s| mesimon_core::text::cap_bytes(s, 120)).unwrap_or("refused");
        self.journal.line(&format!("mod fill {fid} for session {id}: {outcome}: {why}"));
        let Some(owed) = self.owed.remove(&id) else { return };
        let asked = owed.asked;
        let ticket = owed.ticket;
        let Some(words) = owed.sent else { return };
        self.feed.board_outcome("daemon", "prompt_send_now_refused", Some(ticket), why);
        if self.mod_submit(id, ticket, words, owed.ack) {
            if let Some(o) = self.owed.get_mut(&id) {
                o.asked = asked;
            }
        } else if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = false;
        }
        self.persist_and_notify();
    }

    /// A session's mod came up (its bridge's first poll from this pane):
    /// words a launch parked for it go down if the pane's `SessionStart` is
    /// in too (T-575).
    pub(super) fn mod_bridge_up(&mut self, id: uuid::Uuid) {
        if self.mod_deliver_parked(id) {
            self.persist_and_notify();
        }
    }

    /// Words a launch parked for a mod session go down its bridge once both
    /// edges are in: the pane's `SessionStart` (the entry is armed: nothing
    /// reaches a pane being born, on either road, and a `UserPromptSubmit`
    /// ahead of its `SessionStart` would be read as late startup) and the
    /// mod's first poll. Either may come first; whichever is second sends
    /// them. A mod that does not speak `submit` (a session still on an
    /// older one) leaves them to the paste road, on the composer's clock.
    /// `true` when the ledger changed.
    fn mod_deliver_parked(&mut self, id: uuid::Uuid) -> bool {
        let Some(owed) = self.owed.get(&id) else { return false };
        if !owed.mod_road || owed.parked.is_none() || owed.next_press.is_none() {
            return false;
        }
        if !self.mod_bridged(id) {
            return false;
        }
        let command = owed.parked.as_ref().is_some_and(|p| {
            let ticket = owed.ticket;
            road::is_command(&self.launch_words(ticket, p))
        });
        if command || !self.mod_speaks(id, "submit") {
            if let Some(owed) = self.owed.get_mut(&id) {
                owed.mod_road = false;
            }
            self.journal.line(&format!(
                "session {id}'s words are pasted: {}",
                if command { "a slash command" } else { "its mod does not speak submit" }
            ));
            return true;
        }
        let Some(owed) = self.owed.remove(&id) else { return false };
        let Some(words) = owed.parked else { return false };
        let asked = owed.asked;
        if !self.mod_submit(id, owed.ticket, words, owed.ack) {
            self.drop_owed(id);
            return true;
        }
        if let Some(o) = self.owed.get_mut(&id) {
            o.asked = asked;
        }
        true
    }

    /// The session's bridge printed `frame` (its next poll acked it): the
    /// mod has the words (T-575).
    pub(super) fn mod_taken(&mut self, id: uuid::Uuid, frame: &str) {
        if let Some(owed) = self.owed.get_mut(&id).filter(|o| o.frame.as_deref() == Some(frame)) {
            owed.taken = true;
        }
    }

    /// The mod's report on a `submit` (T-575), the frame's id as its reason.
    /// `entered` is the receipt again, and delivery (T-650): the prompt is
    /// the session's, its turn begun or queued behind the running one, so
    /// words the card called unsent are not, and its resend would send them
    /// twice. The ack stays `UserPromptSubmit`. A submit the engine refused
    /// (`dropped`: a hook beneath blocked it) or that threw (`rejected`)
    /// keeps the words unsent, the card's `brief not sent` and its resend,
    /// as a launch whose composer never painted does.
    pub(super) fn on_mod_submit(&mut self, id: uuid::Uuid, frame: &HookFrame) {
        let Some(fid) = frame.reason.as_deref() else { return };
        let outcome = frame.payload["outcome"].as_str().unwrap_or("");
        if outcome == "entered" {
            self.mod_taken(id, fid);
            let delivered = self
                .board
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .is_some_and(|rec| rec.unsent.take().is_some());
            if delivered {
                self.persist_and_notify();
            }
            return;
        }
        let ours = self.owed.get(&id).is_some_and(|o| o.frame.as_deref() == Some(fid));
        if !ours {
            self.journal
                .line(&format!("mod submit {fid} for session {id}: {outcome}, nothing owed"));
            return;
        }
        let said = frame.payload["error"].as_str().or(frame.payload["reason"].as_str());
        self.journal.line(&format!(
            "mod submit {fid} for session {id}: {outcome}: {}",
            said.map(|s| mesimon_core::text::cap_bytes(s, 300)).unwrap_or("no reason given")
        ));
        let Some(owed) = self.drop_owed(id) else { return };
        self.mark_unsent(id, owed.sent);
        self.feed.board_outcome("daemon", "prompt_submit_refused", Some(owed.ticket), outcome);
        self.persist_and_notify();
    }

    /// Take the entry back and drop the record's owed mark with it.
    fn drop_owed(&mut self, id: uuid::Uuid) -> Option<Owed> {
        let owed = self.owed.remove(&id);
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pending_submit = false;
        }
        owed
    }

    /// A prompt reached this session's agent: ours owed — the ack, with its
    /// feed word; or the user's own while an ask waited on the ticket — which
    /// drops the ask: they talked to the agent ahead of it, and the parked
    /// words may now be moot. The one settling site for every road (T-244).
    fn ack_owed(&mut self, id: uuid::Uuid) -> bool {
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        let ticket = rec.ticket;
        // Words a launch could not deliver (T-570) are moot once the agent
        // takes a prompt, whoever typed it.
        let mut changed = rec.unsent.take().is_some();
        if let Some(owed) = self.drop_owed(id) {
            self.feed.board(owed.ack.by, owed.ack.word, Some(ticket));
            // A phone's words that waited for this pane (a relaunch onto
            // its tier pick, T-643, or a composer still coming up) went in.
            self.control_submitted(id);
            self.late_asks.remove(&ticket);
            if let Some(ask) = owed.asked {
                self.mark_turn(ticket, ask);
            }
            changed = true;
        } else {
            self.late_ask_acked(ticket);
        }
        changed | self.forget_queued(ticket, "queued_ask_dropped_by_hand", "local", "person")
    }

    /// Start the delivery of an owed Enter: the pane is provably alive.
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
    /// and until it arrives `settle_owed` presses again. An Enter into an
    /// already-submitted (hence empty) box is a no-op, so a redundant press
    /// costs nothing; a lost one costs the whole feature. Words still parked
    /// have not been typed yet, so there is nothing to press Enter on: the
    /// edge only starts the clock, and the first tick pastes — a lost Enter
    /// is re-pressed for free where a lost paste is the user's words gone.
    /// And a Claude pane's first tick pastes only into a painted composer
    /// (T-570): the edge also starts the composer wait.
    fn arm_owed(&mut self, id: uuid::Uuid, now: u64) -> bool {
        let Some(owed) = self.owed.get_mut(&id) else { return false };
        if owed.presses == 0 || owed.next_press.is_some() {
            return false;
        }
        owed.next_press = Some(now + SUBMIT_RETRY_MS);
        owed.ready_by = Some(now + composer_wait_ms());
        if owed.parked.is_none() {
            if let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) {
                let _ = self.backend.send_enter(&rec.sid16());
            }
        }
        // A mod session whose bridge already polled takes its words now
        // (T-575); otherwise its first poll sends them.
        self.mod_deliver_parked(id)
    }

    /// A launch on the mod alone whose `SessionStart` never came (T-577).
    /// Its words wait for that edge, and on the mod road no hook set sends it:
    /// a mod that relays nothing (T-594's silent launch, which the rig's three
    /// starts at once still met) left the seat `spawning` with its brief
    /// parked for ever. Twice the bridge wait after the spawn its words are
    /// armed as if the edge had come, on the paste road: the composer read
    /// still gates the paste, the prompt it submits is an event the mod reads
    /// on a dispatch of its own (which brings its relays, its tools and its
    /// bridge up), and a mod that never wakes leaves them `unsent`, on the
    /// card. Journalled and fed (`mod_silent`).
    fn rescue_silent_mods(&mut self, now: u64) -> bool {
        let after = mod_bridge_wait_ms().saturating_mul(2);
        let silent: Vec<uuid::Uuid> = self
            .owed
            .iter()
            .filter(|(_, o)| o.next_press.is_none() && o.presses > 0 && o.parked.is_some())
            .filter_map(|(id, _)| self.board.sessions.iter().find(|s| s.id == *id))
            .filter(|r| {
                r.kind == SessionKind::Claude
                    && r.frames_by_mod()
                    && r.state == SessionState::Spawning
                    && now.saturating_sub(r.state_changed_at.unwrap_or(now)) >= after
            })
            .map(|r| r.id)
            .collect();
        let mut changed = false;
        for id in silent {
            let Some(owed) = self.owed.get_mut(&id) else { continue };
            owed.mod_road = false;
            let ticket = owed.ticket;
            self.journal.line(&format!(
                "session {id}: no SessionStart from its mod {} s after the spawn; its words take the paste road",
                after / 1000
            ));
            self.feed.board_outcome("daemon", "mod_silent", Some(ticket), "no SessionStart");
            self.arm_owed(id, now);
            changed = true;
        }
        changed
    }

    /// The tick's pass over the ledger: expire the pastes whose ack never
    /// came, and press again where an Enter is due — on cadence, until
    /// `UserPromptSubmit` settles the entry or the attempts run out. Giving
    /// up leaves the words in the box, and the checkout stops counting as
    /// busy for them either way — but never silently (T-570): a Claude seat
    /// keeps what it never took as `unsent`, which lights its card and
    /// which its Shift+Enter resends.
    ///
    /// Words still parked for a Claude pane wait for its composer
    /// (`composer::read`): a paste before Claude has asked for bracketed
    /// paste goes in as plain bytes, echoed and cut by the cooked tty
    /// (T-566). Until the composer paints, the cadence runs and nothing is
    /// typed or pressed; past `COMPOSER_WAIT_MS` the start has failed.
    fn settle_owed(&mut self, now: u64) -> bool {
        if self.owed.is_empty() {
            return false;
        }
        let mut changed = false;
        let expired: Vec<uuid::Uuid> = self
            .owed
            .iter()
            .filter(|(_, o)| o.expires.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(owed) = self.drop_owed(id) {
                // Taken, and unacked only because a turn is still running
                // (T-600): the crown's mark waits for the turn that takes
                // the words, not for this window. A fill's send-now (T-601)
                // went into the running turn, as a paste does.
                if let (Some(ask @ TurnAsk::Crown(_)), true) =
                    (owed.asked, owed.frame.is_none() || owed.taken)
                {
                    self.ask_unacked(owed.ticket, ask, owed.frame.is_some() && !owed.fill);
                }
                match &owed.frame {
                    // The session's mod never took the words (T-575): they
                    // are taken back and kept unsent, as a paste into a box
                    // that never painted is.
                    Some(frame) if !owed.taken => {
                        self.mod_unqueue(id, frame);
                        self.mark_unsent(id, owed.sent);
                        self.feed.board("daemon", "prompt_submit_unreceived", Some(owed.ticket));
                    }
                    // Taken and queued behind a turn that is still running:
                    // the checkout stops counting it, as a paste's does.
                    Some(_) => self.feed.board("automation", "submit_unacked", Some(owed.ticket)),
                    None => self.feed.board("automation", "paste_unacked", Some(owed.ticket)),
                }
                changed = true;
            }
        }
        let due: Vec<uuid::Uuid> = self
            .owed
            .iter()
            .filter(|(_, o)| o.next_press.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else {
                self.owed.remove(&id);
                continue;
            };
            // A pane that died, or one showing a startup modal, is not a pane
            // to keep pressing Enter into — the modal's Enter is an ANSWER,
            // and mesimon does not answer dialogs on the user's behalf.
            if !rec.pressable() {
                let ticket = rec.ticket;
                let claude = rec.kind == SessionKind::Claude;
                let words = self.drop_owed(id).and_then(|o| o.sent.or(o.parked));
                // A Claude seat stopped by a startup modal still has a pane
                // to resend into once the dialog is answered (T-570).
                if claude {
                    self.mark_unsent(id, words);
                }
                self.feed.board("daemon", "prompt_submit_abandoned", Some(ticket));
                changed = true;
                continue;
            }
            let sid16 = rec.sid16();
            let ticket = rec.ticket;
            let claude = rec.kind == SessionKind::Claude;
            let parked = self.owed.get(&id).is_some_and(|o| o.parked.is_some());
            // Words for a mod session wait for its bridge (T-575), which
            // takes them as a `submit` the moment it first polls
            // (`mod_bridge_up`); one that has not polled by `bridge_by` never
            // will, and the words take the paste road from here, title and
            // all, with the composer's own wait.
            if claude && parked && self.mod_deliver_parked(id) {
                changed = true;
                continue;
            }
            if claude && parked {
                let Some(owed) = self.owed.get_mut(&id) else { continue };
                if owed.mod_road {
                    let bridge_by = *owed.bridge_by.get_or_insert(now + mod_bridge_wait_ms());
                    if now < bridge_by {
                        owed.next_press = Some(now + SUBMIT_RETRY_MS);
                        continue;
                    }
                    owed.mod_road = false;
                    owed.ready_by = Some(now + composer_wait_ms());
                    self.journal.line(&format!(
                        "no mod bridge for session {id} in {} ms: its launch words take the paste \
                         road",
                        mod_bridge_wait_ms()
                    ));
                    self.feed.board("daemon", "prompt_by_paste", Some(ticket));
                }
            }
            if claude && parked {
                use crate::agents::claude::composer::{self, Composer};
                let composer = self
                    .backend
                    .capture_input_screen(&sid16)
                    .map_or(Composer::Absent, |screen| composer::read(&screen));
                let Some(owed) = self.owed.get_mut(&id) else { continue };
                let ready_by = *owed.ready_by.get_or_insert(now + composer_wait_ms());
                match composer {
                    Composer::Absent if ready_by <= now => {
                        let words = owed.parked.take();
                        self.drop_owed(id);
                        self.mark_unsent(id, words);
                        self.feed.board("daemon", "prompt_submit_not_ready", Some(ticket));
                        changed = true;
                        continue;
                    }
                    Composer::Absent => {
                        owed.next_press = Some(now + SUBMIT_RETRY_MS);
                        continue;
                    }
                    // The resend's one Ctrl+C (T-570): the box holds what
                    // the failed start left in it. The paste waits a cadence
                    // for the cleared box, and a box that still reads as
                    // holding then is pasted into as it is — never a second
                    // press, which exits Claude.
                    Composer::Holding if owed.clear_first => {
                        owed.clear_first = false;
                        let _ = self.backend.clear_input(&sid16);
                        owed.next_press = Some(now + SUBMIT_RETRY_MS);
                        continue;
                    }
                    Composer::Holding | Composer::Empty => {}
                }
            }
            let Some(owed) = self.owed.get_mut(&id) else { continue };
            let left = owed.presses;
            // The parked words, delivered: the pane has been up a cadence
            // past its `SessionStart` and, for Claude, shows its composer, so
            // they go in the way a live pane takes them — bracketed paste,
            // then a separate Enter (`paste_text`, the T-5 shape). The presses
            // that follow are the ordinary retries; a paste is made once.
            if let Some(parked) = &owed.parked {
                owed.sent = Some(parked.clone());
            }
            match owed.parked.take() {
                Some(parked) => {
                    let brief = parked.brief;
                    let text = self.launch_words(ticket, &parked);
                    if text.is_empty() {
                        let _ = self.backend.send_enter(&sid16);
                    } else {
                        let _ = self.backend.paste_text(&sid16, &text);
                        // The brief went in with the first prompt: this
                        // session has read its ticket, whatever it does with
                        // the tools.
                        if brief {
                            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                                rec.ticket_read = true;
                                changed = true;
                            }
                        }
                    }
                }
                None => {
                    let _ = self.backend.send_enter(&sid16);
                }
            }
            if left <= 1 {
                let words = self.drop_owed(id).and_then(|o| o.sent.or(o.parked));
                if claude {
                    self.mark_unsent(id, words);
                }
                self.feed.board("daemon", "prompt_submit_gave_up", Some(ticket));
                changed = true;
            } else if let Some(owed) = self.owed.get_mut(&id) {
                owed.presses = left - 1;
                owed.next_press = Some(now + SUBMIT_RETRY_MS);
            }
        }
        changed
    }

    /// A death frame — pane-died, or the `SessionEnd` a process runs on its
    /// way out — that names a pane other than the record's belongs to the
    /// pane that held the name before (see the caller): `Some((theirs,
    /// ours))`. `logout`, `clear` and `resume` are never a kill's echo and
    /// stay out of it, and so does a shell's `SessionEnd`: a shell record's
    /// hook frames are the agent-inside-a-shell compatibility road, and no
    /// shell wake kills a claude. A frame with no pane key (a hook binary
    /// older than T-245, a frame sent from outside a pane) or a record with
    /// none (an older `sessions.json`) is trusted: `None`.
    fn straggler_death(
        &self,
        id: uuid::Uuid,
        sig: &Signal,
        frame_pane: Option<&str>,
    ) -> Option<(String, String)> {
        let rec = self.board.sessions.iter().find(|s| s.id == id)?;
        let death = match sig {
            Signal::PaneDied { .. } => true,
            Signal::SessionEnd { kind } => {
                rec.kind.is_agent() && matches!(kind, EndKind::Other | EndKind::PromptInputExit)
            }
            _ => false,
        };
        if !death {
            return None;
        }
        let theirs = frame_pane?;
        let ours = rec.pane_key.as_deref()?;
        (theirs != ours).then(|| (theirs.to_string(), ours.to_string()))
    }

    /// The pid of the record's own pane while tmux lists it alive and not
    /// dead — `None` for a dead, remain-on-exit pane and for no pane at all.
    /// The pane's process IS the agent's (`mesimon exec` execs), so this is
    /// the pid a `~/.claude/sessions` file would name for it.
    fn own_pane_pid(&self, rec: &SessionRecord) -> Option<i32> {
        let sid16 = rec.sid16();
        self.backend
            .snapshot()
            .ok()?
            .into_iter()
            .find(|p| p.session_name == sid16)
            .filter(|p| !p.pane_dead)
            .map(|p| p.pane_pid)
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

    fn observe_signal(
        &mut self,
        id: uuid::Uuid,
        signal: &Signal,
        now: u64,
        source: &str,
    ) -> Option<Change> {
        let machine = self.machines.get_mut(&id)?;
        let (change, decision) = machine.apply_explained(signal, now);
        self.feed.state_decision(id, source, &decision);
        change
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
        if rec.kind == SessionKind::Codex {
            if let Some(sequence) = rec.codex_pending_seq.take() {
                rec.codex_observed_seq = sequence;
            }
        }
        rec.state = change.to.clone();
        rec.confidence = change.confidence;
        let stop_words = detail.is_some();
        if change.from != change.to {
            rec.state_changed_at = Some(now);
            rec.waiting_since = if attention::is_attention(&change.to) { Some(now) } else { None };
        }
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
        // The crown's answer reads on the card until the next state edge
        // (T-569): kept through the question's own leave — the answer
        // landing — and through a commit that moved nothing, and gone at
        // any other edge, or under a stop's own words. Its plan accept
        // (T-582) the same, through the plan's leave.
        if let Some(line) = self.crown_answer_lines.get(&id) {
            let keeps = if change.from == change.to {
                !stop_words
            } else {
                matches!(
                    change.from,
                    SessionState::RequiresAction { reason: Reason::Question | Reason::Plan }
                )
            };
            if keeps {
                rec.detail = Some(line.clone());
            } else {
                self.crown_answer_lines.remove(&id);
            }
        }
        let snapshot = rec.clone();
        self.feed.session_state(&snapshot, &change.from, hook);
        // A turn ending settles the ticket's rebase ask (T-435): the train's
        // hold for a ticket in its rebase turn lasts exactly that turn.
        if change.from != change.to && !mesimon_core::quiet::is_working(&snapshot) {
            self.train.settle(snapshot.ticket);
            self.persist_train();
        }
        // A raised hand is answered by the NEXT turn beginning (T-311), not
        // by the one that raised it ending — that half is T-107's whole
        // design and is untouched. `UserPromptSubmit` was the only turn-start
        // the daemon knew, and T-228 measured a second: a `!` bash command in
        // Claude Code puts its output into the conversation and the model
        // takes a turn on it with no prompt hook at all. That is exactly the
        // shape of an answer to a hand — "run `gcloud auth login`, then tell
        // me" — so answering the agent the way it asked left the `!` up on a
        // card that was visibly working again (dogfood 2026-09-07). The edge
        // is the promotion T-228 already built, and nothing else: only from
        // `EndTurn`, because `Background` is a park a teammate's report
        // resumes with no person involved and `Interrupted`/`Unknown` are
        // guesses, and only at High, because `SubagentStop` promotes an
        // inferred idle back to Running as a CORRECTION of a misread rather
        // than as a new turn. A hand may not come down on an inference.
        // And by the dialog it stood for going away (T-611): an agent that
        // raised its hand and then asked in its own pane went on working once
        // the person answered there, and no prompt came to take the mark
        // down. A dialog's leave into `Running` is the person's doing either
        // way — the accept's `PostToolUse` (the mod's `ModAnswer` is ingested
        // as one) or T-447's refusal road, the agent's next own call — and
        // only High, for the same reason as above. A shell's state is not the
        // agent's, and a quota or a startup modal is not a person's answer.
        let dialog_left = matches!(
            change.from,
            SessionState::RequiresAction {
                reason: Reason::Question | Reason::Plan | Reason::Permission | Reason::Elicitation
            }
        );
        let turn_began =
            matches!(change.from, SessionState::Idle { stop_reason: StopReason::EndTurn });
        if change.confidence == Confidence::High
            && (turn_began || (dialog_left && snapshot.kind.is_agent()))
            && matches!(change.to, SessionState::Running)
        {
            self.lower_hand_on(snapshot.ticket);
        }
        self.auto_move(snapshot.ticket, id, &change.to, change.confidence);
        // A turn ended (T-469): what it was asked for goes with it, and an
        // `EndTurn` — `Background` is a park and the rest are guesses — at
        // Medium or better, the confidence automove's `on_done` takes, sends
        // a look at what it left, which decides whether the crown hears of
        // it. After the automove, so the look reads the column the turn
        // left the card in. Not a move, so the move gate's depth rule holds
        // by construction: nothing here calls `place_ticket`. An agent's
        // turn only: a shell on the same ticket changing state must not take
        // its claude's mark.
        // An edge into a working state opens a turn (T-591), so the next
        // `EndTurn` is a finished one.
        // A foreground turn ends a stretch idle with background tasks
        // (T-599): the next one is new, and may wake the crown again.
        if change.to == SessionState::Running {
            self.lingered.remove(&id);
        }
        if snapshot.kind.is_agent() && change.from != change.to {
            // A turn that took the crown's ask and ended with background
            // tasks still running answered it (T-599): the crown asked
            // whether the wait is the work, and the words are the answer.
            let answered_with_tasks = change.to
                == (SessionState::Idle { stop_reason: StopReason::Background })
                && matches!(self.turn_asks.get(&snapshot.ticket), Some(TurnAsk::Crown(_)))
                && matches!(change.confidence, Confidence::High | Confidence::Medium);
            if answered_with_tasks {
                self.turn_ended(snapshot.ticket, true);
            } else if mesimon_core::quiet::is_working(&snapshot) {
                self.turns_open.insert(snapshot.ticket);
            } else {
                let end_turn =
                    matches!(change.to, SessionState::Idle { stop_reason: StopReason::EndTurn })
                        && matches!(change.confidence, Confidence::High | Confidence::Medium);
                self.turn_ended(snapshot.ticket, end_turn);
                // The person's banner waits for a look at what the turn
                // left, where the train may take it (T-678).
                if end_turn {
                    self.look_after_turn(snapshot.ticket);
                }
            }
        }
        // The agent stopped on a question (T-420): the answer may change
        // what a queued follow-up should say, so the words wait for a
        // person's `^y` rather than going after the turn. Which stops are
        // questions is `question_stop`'s, and `park_ask` reads the same
        // predicate for words queued after the stop (T-565).
        if change.to.question_stop() {
            self.hold_queued_on_question(id);
        }
        // A question or a plan from a claude the crown started wakes the
        // crown (T-569, T-582) where the board lets it answer; off, the
        // person is the one to wake, and the card's needs-you already does.
        // A secret, a form or a permission is never the crown's.
        if let Some(cause) = crownwake::asks_the_crown(self.board.crown_mode, snapshot.kind, change)
        {
            self.note_crown_wake(snapshot.ticket, cause, None, None);
        }
        // A turn ended, or a target died: the queued asks look again. The
        // settle that lands `Idle{EndTurn}` comes through here from the
        // tick, and so does the shutdown flush — the words go out on the
        // way down rather than being lost with the restart. The crown's
        // wake looks after the queue, so a person's ask takes the turn.
        self.sweep_queue();
        // A seat that owes a tier switch (T-443) takes its idle first: the
        // relaunch is what a person asked for when they picked the tier.
        self.drain_tier_switches();
        self.drain_queue();
        self.drain_crown_wakes();
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
    ///
    /// `session` is whose edge this is: the move gate does not count a
    /// session's card following that session's own turns toward its fuse
    /// (T-468).
    fn auto_move(
        &mut self,
        ticket: ulid::Ulid,
        session: uuid::Uuid,
        to: &SessionState,
        confidence: Confidence,
    ) {
        let Some(t) = self.board.ticket(ticket) else { return };
        // The rule is the ticket's COLUMN's (T-117): `on_working`/`on_done`,
        // never a column name compared here.
        let Some(col) = self.board.column(&t.column) else { return };
        let from = t.column.clone();
        let decision = mesimon_core::automove::explain(&col.settings, to, confidence);
        let dest = decision.destination.map(str::to_string);
        self.feed.movement_decision(ticket, &from, dest.as_deref(), decision.outcome);
        let Some(dest) = dest else {
            return;
        };
        let by = Principal::Automation { rule: "automove".into() };
        let outcome =
            match self.place_ticket(ticket, &dest, Position::Top, &by, Some(session), "automove") {
                Ok(_) if from == dest => "already_in_column".to_string(),
                Ok(_) => "moved".to_string(),
                Err(reason) => format!("refused: {reason}"),
            };
        self.feed.movement_decision(ticket, &from, Some(&dest), &outcome);
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
    /// A board tool call is the agent's own word that its session is in a
    /// turn (T-660), refused or not. On the mod road a registered tool fires
    /// no `PreToolUse` and no `PostToolUse`, so a turn spent on board tools
    /// and thinking sent the machine nothing: after a restart T-650's lead
    /// wore `Unknown{DaemonRestarted}` for two and a half minutes, through
    /// three `create_ticket`s and two `start_agent`s, until its reply. The
    /// call may be a subagent's, so it is heard as a nested tool start: it
    /// lifts `Unknown` and an inferred idle, keeps a park's clock, and leaves
    /// a stated idle and a held dialog alone.
    fn hear_agent_call(&mut self, session: uuid::Uuid) {
        let principal = Principal::Automation { rule: "agent_call".into() };
        if matches!(
            authorize(&principal, &Action::Mutate, &Resource::Session { id: session }),
            Decision::Deny { .. }
        ) {
            return;
        }
        let now = now_ms();
        let signal = Signal::ToolStarted { nested: true };
        let Some(change) = self.observe_signal(session, &signal, now, "agent_call") else {
            return;
        };
        if self.apply_change(session, &change, None, Some("agent_call")) {
            self.persist_and_notify();
        }
    }

    fn handle_agent(&mut self, session: uuid::Uuid, cmd: Command) -> Response {
        if !mcp::agent_allows(&cmd) {
            return Response::Err { message: "not available to an agent session".into() };
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "unknown session".into() };
        };
        // Observe-only adopted records have no argv of ours and were never
        // launched with the tool config; a request claiming to be one is not
        // something mesimon started. The predicate is the daemon's one
        // definition of observe-only — adopted AND no argv — not provenance
        // alone: a taken-over external session keeps `Adopted` for life, and
        // its takeover argv carries `--mcp-config` like any spawn's (T-240:
        // the tools were handed out and every call was refused).
        if rec.observe_only() {
            return Response::Err { message: "not a session mesimon spawned".into() };
        }
        if !rec.state.is_live() {
            return Response::Err { message: "session has exited".into() };
        }
        let ticket = rec.ticket;
        // The mod's bridge (T-574) is no tool, so no tier gates it: a column
        // whose agents have tools `off` still has its sessions' mods. It
        // reads the delivery ledger of the caller's own session, which the
        // chokepoint hears as a read of its ticket — an agent is never
        // granted a session resource.
        if let Command::ModNext { ack, pane, speaks } = cmd {
            let by = Principal::Agent { session };
            if let Decision::Deny { reason } =
                authorize(&by, &Action::Read, &Resource::Ticket { id: ticket })
            {
                return Response::Err { message: format!("denied: {reason}") };
            }
            return self.mod_next(session, ack, pane, speaks);
        }
        self.hear_agent_call(session);
        // The column's tier (T-117), against the ticket's column as it
        // stands NOW — the shim listed the tools of the column at spawn, and
        // the model reads the tier it is on in the refusal.
        let tier = self.agent_tools_for(ticket);
        if !mcp::tier_admits(tier, &cmd) {
            let col = self.board.ticket(ticket).map(|t| t.column.clone()).unwrap_or_default();
            return Response::Err {
                message: format!(
                    "not available here: column {col} grants agent tools `{}`",
                    tier.word()
                ),
            };
        }

        match cmd {
            Command::AgentGetTicket => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Read, &Resource::Ticket { id: ticket })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                match self.agent_ticket_view(ticket) {
                    Some(mut view) => {
                        // Its own ticket: the person reads it beside the
                        // ticket, where the key is a lookup (T-715).
                        view.key_about = Some(mesimon_core::mcp::KEY_ABOUT.to_string());
                        // A worker the board's crown started reads who merges
                        // under it (T-599).
                        let crown = self.board.crown_holder().map(|t| t.id);
                        view.under_crown = self
                            .board
                            .sessions
                            .iter()
                            .any(|s| s.id == session && crown.is_some() && s.started_by == crown)
                            .then(|| mesimon_core::mcp::WORKER_UNDER_CROWN.to_string());
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
                    None => no_such_ticket(),
                }
            }
            // Another ticket, by key (T-411): the crown's one read.
            Command::AgentReadTicket { key } => {
                let target = match self.crown_target(ticket, &key, true) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Read, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                match self.agent_ticket_view(target) {
                    Some(mut view) => {
                        view.asked = self.crown_asked_view(ticket, target);
                        view.watched = target != ticket && self.crown_watched.contains(&target);
                        Response::AgentTicket { ticket: view }
                    }
                    None => no_such_ticket(),
                }
            }
            Command::AgentListBoard => {
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } = authorize(&by, &Action::Read, &Resource::Board) {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                Response::AgentBoard { board: self.agent_board_view(ticket) }
            }
            Command::AgentMoveTicket { to_column, idempotency_key, key, before, seen } => {
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
                            seen: None,
                        };
                    }
                }
                // The target: the caller's own ticket, or with a key another
                // ticket — the crown's road (T-411), judged against the
                // ticket as it was READ (`seen`).
                let target = match self.keyed_target(ticket, key.as_deref(), seen.as_deref(), false)
                {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                // Position is priority: `before` lands above a named ticket
                // in the destination; nothing named lands at the top, as
                // every agent move did.
                let pos = match before {
                    None => Position::Top,
                    Some(b) => match self.board.ticket_by_key(&b) {
                        Some(t) => Position::Before(Some(t.id)),
                        None => return Response::Err { message: format!("no such ticket: {b}") },
                    },
                };
                let by = Principal::Agent { session };
                match self.place_ticket(target, &to_column, pos, &by, None, "agent_move") {
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
                        let seen = self.crown_touched(ticket, target, "moved");
                        Response::AgentMoved {
                            column,
                            board_version: self.board_version,
                            replayed: false,
                            seen,
                        }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            // The note tools: the ticket is the binding's — or the key's, for
            // the crown (T-411) — and a note id off it reads as "no such
            // note" inside the handlers.
            Command::AgentReadAttachment { attachment, key } => {
                let target = match key {
                    None => ticket,
                    Some(k) => match self.crown_target(ticket, &k, true) {
                        Ok(t) => t,
                        Err(message) => return Response::Err { message },
                    },
                };
                if let Decision::Deny { reason } = authorize(
                    &Principal::Agent { session },
                    &Action::Read,
                    &Resource::Ticket { id: target },
                ) {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                self.read_attachment(target, attachment)
            }
            Command::AgentReadNote { note, key } => {
                let target = match key {
                    None => ticket,
                    Some(k) => match self.crown_target(ticket, &k, true) {
                        Ok(t) => t,
                        Err(message) => return Response::Err { message },
                    },
                };
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Read, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                self.read_note(target, note)
            }
            Command::AgentWriteNote { note, text, key } => {
                let target = match key {
                    None => ticket,
                    Some(k) => match self.crown_target(ticket, &k, false) {
                        Ok(t) => t,
                        Err(message) => return Response::Err { message },
                    },
                };
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let resp = self.write_note(target, note, text, None, &by);
                if matches!(resp, Response::NoteWritten { .. }) {
                    self.feed.board(by.actor(), "write_note", Some(target));
                    self.crown_touched(ticket, target, "note");
                }
                resp
            }
            // The crown's three writers (T-411). Each resolves its key
            // through the same gate, checks `seen`, does what the human's
            // command does, and answers with the ticket as it stands now —
            // fresh stamp included, so the next edit needs no second read.
            Command::AgentRenameTicket { key, title, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                // Trimmed, unlike the composer's: a model's leading blank
                // is noise, a person's is a choice.
                let title = mesimon_core::board::sanitize_title(title.trim());
                if title.trim().is_empty() {
                    return Response::Err { message: "title is empty".into() };
                }
                if let err @ Response::Err { .. } = self.with_ticket(target, |t| t.title = title) {
                    return err;
                }
                self.feed.board(by.actor(), "rename_ticket", Some(target));
                self.crown_touched(ticket, target, "renamed");
                self.agent_ticket_view(target)
                    .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket })
            }
            Command::AgentSetWorkspace { key, workspace, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let ws = match crown::parse_workspace(&workspace) {
                    Ok(ws) => ws,
                    Err(message) => return Response::Err { message },
                };
                if let err @ Response::Err { .. } = self.set_workspace(target, Some(ws)) {
                    return err;
                }
                self.feed.board(by.actor(), "set_workspace", Some(target));
                self.crown_touched(ticket, target, "workspace");
                self.agent_ticket_view(target)
                    .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket })
            }
            Command::AgentArchiveTicket { key, restore, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), restore) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                // The person's switch (T-590), judged at the call and before
                // anything changes: off, a card leaves the board, and comes
                // back, by a person's hand alone.
                if !self.board.crown_archives {
                    return Response::Err { message: crown_archive_off(&key, restore) };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                // The generic gate says "sleep them first"; the crown's
                // answer names ITS road (T-539): `sleep_agent` for a seat it
                // started, a person for any other.
                if !restore {
                    if let Some(message) = self.crown_archive_refusal(target, &key) {
                        return Response::Err { message };
                    }
                }
                let (resp, word) = if restore {
                    (self.unarchive_ticket(target), "restored")
                } else {
                    (self.archive_ticket(target, &by), "archived")
                };
                if let err @ Response::Err { .. } = resp {
                    return err;
                }
                self.feed.board(
                    by.actor(),
                    if restore { "unarchive_ticket" } else { "archive_ticket" },
                    Some(target),
                );
                self.crown_touched(ticket, target, word);
                self.agent_ticket_view(target)
                    .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket })
            }
            // The crown's start (T-412): an ask to spawn, judged here. The
            // budget and the seat rule are the daemon's; the agent names a
            // ticket and nothing else — no kind, no prompt, no session.
            Command::AgentAskTicket { key, text, seen, plan, deliver } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; ask_agent is for another ticket"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                // A seat to receive the words: a pane or a parked agent. An
                // empty seat is `start_agent`'s (T-412), never a start on
                // the crown's words — the title is the person's prompt.
                let seat = self.seat_of(target);
                if matches!(seat, QueuedSeat::Start(_)) {
                    return Response::Err {
                        message: format!(
                            "{key} has no agent to receive the words; start_agent first"
                        ),
                    };
                }
                if self.board.live_agent(target).is_some_and(|rec| rec.observe_only()) {
                    return Response::Err {
                        message: format!("{key}'s session is external; a person resumes it first"),
                    };
                }
                // A question stands (T-566): words queued now could only wait
                // behind its answer, and the answer is a person's. Refused,
                // so the crown reads the question rather than guessing.
                if self.board.live_agent(target).is_some_and(|rec| rec.state.question_stop()) {
                    return Response::Err {
                        message: format!(
                            "{key}'s agent is asking a question; a person answers it in the pane \
                             or from Remote Control, or answer_agent where the board lets the \
                             crown answer, and the crown reads it with get_ticket (needs_you). \
                             Words queued now would wait behind the answer."
                        ),
                    };
                }
                // A plan stands (T-582): words queued now would wait behind
                // it and the turn it starts. The crown accepts it, or raises
                // its hand for a plan it would change.
                if self.board.live_agent(target).is_some_and(|rec| {
                    rec.state == SessionState::RequiresAction { reason: Reason::Plan }
                }) {
                    return Response::Err {
                        message: format!(
                            "{key}'s agent stopped on a plan; accept_plan accepts it where the \
                             board lets the crown answer, after get_ticket shows the plan \
                             (needs_you), and a plan the crown would change is a person's: \
                             raise_hand names the worker and the change. Words queued now would \
                             wait behind the plan and the turn it starts."
                        ),
                    };
                }
                // Sanitized by subtraction alone, as the person's own words
                // are: nothing is added, and blank words queue nothing.
                let Some(text) = mesimon_core::command::sanitize_prompt(&text) else {
                    return Response::Err { message: "nothing to send".into() };
                };
                // Plan mode (T-434) is a Claude launch flag: the same
                // refusal the person's field would get on a Codex board.
                if let Some(message) = self.plan_refusal(target, plan) {
                    return Response::Err { message };
                }
                // `now` and `immediately` (T-600, T-601) are the person's
                // sends, refused where theirs are: at a dialog, whose keys a
                // paste would answer; with plan mode mid-turn, which restarts
                // the agent; and `immediately` at an agent with no send-now.
                if let (true, QueuedSeat::Pane(id)) = (deliver.at_once(), &seat) {
                    let level = deliver.word();
                    if self.pane_waits_on_you(*id) {
                        return Response::Err {
                            message: format!(
                                "{key}'s agent is at a dialog; words sent {level} would land in \
                                 it. A person answers it in the pane, and ask_agent with deliver \
                                 idle queues the words for its idle."
                            ),
                        };
                    }
                    if plan && !self.session_idle(*id) {
                        return Response::Err {
                            message: format!(
                                "{key}'s agent is mid-turn, and plan mode restarts it; \
                                 ask_agent with deliver idle queues the words for its idle."
                            ),
                        };
                    }
                }
                if deliver == Deliver::Immediately {
                    if let Some(why) = self.immediate_refusal(target) {
                        return Response::Err {
                            message: format!(
                                "{key}'s agent {why}; ask_agent with deliver now sends the words \
                                 at once without it."
                            ),
                        };
                    }
                }
                // The road (T-550): held for a person's send, unless the
                // board lets the crown send and the crown started this agent.
                let held_because = self.crown_ask_hold(ticket, target, &seat);
                let sends = held_because.is_none();
                let replaced =
                    self.queued.iter().any(|q| q.ticket == target && q.by == Some(ticket));
                if let Err(message) = self.park_ask(target, seat, text, Some(ticket), false, plan) {
                    return Response::Err { message };
                }
                let mut held_for_person = !sends;
                if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == target) {
                    q.sends = sends;
                    q.deliver = deliver;
                    // Its agent is on a question (T-565): `park_ask` held
                    // the words, and the send stays a person's — after the
                    // answer, which may change them.
                    held_for_person = q.held_for_person();
                }
                let held_because = held_because.or_else(|| {
                    held_for_person.then(|| {
                        "its agent stopped on a question; a person answers it, then sends the \
                         words (^y on its card)"
                            .to_string()
                    })
                });
                let road = match (held_for_person, deliver) {
                    (true, _) => AskRoad::HeldForPerson,
                    (false, Deliver::Now) => AskRoad::SentNow,
                    (false, Deliver::Immediately) => AskRoad::SentImmediately,
                    (false, Deliver::Idle) => AskRoad::Queued,
                };
                self.feed.board_outcome(
                    "agent",
                    if replaced { "ask_agent_replaced" } else { "ask_agent" },
                    Some(target),
                    road.word(),
                );
                self.crown_touched(ticket, target, "asked");
                match road {
                    // Sent now (T-600): the person's `^y` on the parked words,
                    // made by the board on the crown's say-so — a pane takes
                    // them mid-turn (the mod's `submit`, or the composer's
                    // paste), a parked agent is woken with them. The turn
                    // that takes them is the crown's answer, as after `^y`.
                    // Immediately (T-601) is the same send with Claude Code's
                    // send-now over the words (`send_queued_ask` reads the
                    // entry's level).
                    AskRoad::SentNow | AskRoad::SentImmediately => {
                        if let err @ Response::Err { .. } = self.send_queued_ask(target) {
                            return err;
                        }
                        self.feed.board_outcome(
                            "agent",
                            "ask_agent_sent",
                            Some(target),
                            deliver.word(),
                        );
                        self.crown_touched(ticket, target, "sent");
                    }
                    // An idle agent takes the words now, its touch turning
                    // to `sent`; a busy one when its turn ends, as a
                    // person's queued ask would.
                    AskRoad::Queued => {
                        self.drain_queue();
                    }
                    AskRoad::HeldForPerson => {}
                }
                // The stamp is read after: a wake moves the agent's state,
                // which the stamp covers.
                self.broadcast();
                Response::AgentAsked {
                    key: self.board.ticket(target).map(|t| t.short_key.clone()).unwrap_or(key),
                    replaced,
                    seen: Some(self.seen_token(target)),
                    held_for_person,
                    held_because,
                    road: Some(road),
                }
            }
            // The crown's answer (T-569): a question an agent the crown
            // started stopped on, answered by Remote Control's own
            // screen-verified road. The structure is enforced here and in
            // `crown_dialog_answer`; which questions stay a person's is the
            // crown's judgement, described in the tool and the receipt. The
            // receipt waits for the hook edge: this arm queues the walk and
            // the writer parks the call's reply with it (`answer_waits`).
            Command::AgentAnswerTicket { key, seen, request, index, text, answers } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; answer_agent is for another \
                             ticket's agent"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let key = self.board.ticket(target).map(|t| t.short_key.clone()).unwrap_or(key);
                let answer = match (index, text, answers) {
                    (Some(index), None, None) => mesophon::CrownAnswer::Index(index),
                    (None, Some(text), None) => mesophon::CrownAnswer::Text(text),
                    (None, None, Some(answers)) => mesophon::CrownAnswer::Answers(answers),
                    _ => {
                        return Response::Err {
                            message: "answer_agent takes index, text or answers, one of them"
                                .into(),
                        }
                    }
                };
                match self.crown_dialog_answer(ticket, session, target, &key, &request, answer) {
                    Ok(id) => {
                        self.answer_waits = Some(id);
                        // What a caller that is not the writer loop reads:
                        // the answer is on its way, nothing yet confirmed.
                        Response::AgentAnswered {
                            key,
                            outcome: "awaiting_delivery".into(),
                            reason: None,
                            answer: String::new(),
                            seen: Some(self.seen_token(target)),
                        }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            // The crown's plan accept (T-582): the board's own Enter on the
            // plan dialog's default row (T-420), for a plan a claude the
            // crown started stopped on. The structure is enforced here and in
            // `crown_accept_plan`; which plans stay a person's is the crown's
            // judgement, described in the tool and the receipt. The receipt
            // waits for the hook edge: the writer parks the call's reply
            // with the press (`plan_waits`).
            Command::AgentAcceptPlan { key, seen, request } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; accept_plan is for another \
                             ticket's agent"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let key = self.board.ticket(target).map(|t| t.short_key.clone()).unwrap_or(key);
                match self.crown_accept_plan(ticket, session, target, &key, &request) {
                    Ok(id) => {
                        self.plan_waits = Some(id);
                        // What a caller that is not the writer loop reads:
                        // the press is on its way, nothing yet confirmed.
                        Response::AgentPlanAccepted {
                            key,
                            outcome: "awaiting_delivery".into(),
                            reason: None,
                            seen: Some(self.seen_token(target)),
                        }
                    }
                    Err(message) => Response::Err { message },
                }
            }
            Command::AgentStartTicket { key, seen, plan, tier, workspace } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket, which already runs; start_agent \
                             is for another ticket"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                // Where it works is the crown's to decide on every start
                // (T-583): a shim from before the field sends none.
                let Some(word) = workspace else {
                    return Response::Err {
                        message: "start_agent needs workspace: worktree (the ticket's own \
                                  branch) or shared_checkout; the crown decides where each agent \
                                  it starts works"
                            .into(),
                    };
                };
                let wanted = match crown::parse_workspace(&word) {
                    Ok(ws) => ws,
                    Err(message) => return Response::Err { message },
                };
                let checkout = self.paths.repo_root.to_string_lossy().into_owned();
                let has_worktree = self.worktrees.contains_key(&target);
                let start = match crown::judge(
                    &self.board,
                    ticket,
                    target,
                    wanted,
                    has_worktree,
                    &checkout,
                ) {
                    Ok(start) => start,
                    Err(message) => return Response::Err { message },
                };
                if let Some(message) = self.crown_budget_refusal() {
                    return Response::Err { message };
                }
                // The crown's tier (T-584), judged before anything changes:
                // one it may name, not over a person's own start, and one
                // plan mode can launch on.
                let pick = match tier.as_deref().map(|w| self.crown_tier_pick(target, &key, w)) {
                    None => None,
                    Some(Ok(t)) => Some(t),
                    Some(Err(message)) => return Response::Err { message },
                };
                let plan_refused = match &pick {
                    Some(t) => (plan && !t.provider.has_plan_flag()).then(|| {
                        format!("plan mode is a claude launch flag ∙ {} runs codex", t.name)
                    }),
                    None => self.plan_refusal(target, plan),
                };
                if let Some(message) = plan_refused {
                    return Response::Err { message };
                }
                if let Some(t) = pick {
                    if let Err(message) = self.apply_ticket_tier(target, Some(t.id)) {
                        return Response::Err { message };
                    }
                }
                let (session_started, woken, kind) = match start {
                    // The crown's own parked agent (T-583): the person's `c`,
                    // the conversation kept, in the cwd it was parked in.
                    crown::Start::Wake(id) => {
                        let kind = self
                            .board
                            .sessions
                            .iter()
                            .find(|s| s.id == id)
                            .map(|s| s.kind)
                            .unwrap_or(SessionKind::Claude);
                        let resp = self.resume_session_in(id, false, plan);
                        self.persist_and_notify();
                        match resp {
                            Response::Spawned { .. } => (true, true, kind),
                            Response::Provisioning => (false, true, kind),
                            Response::Err { message } => {
                                return Response::Err { message: format!("{key}: {message}") }
                            }
                            other => {
                                return Response::Err {
                                    message: format!("unexpected wake answer: {other:?}"),
                                }
                            }
                        }
                    }
                    crown::Start::Spawn { apply } => {
                        // As `set_workspace` would, behind the same lock.
                        if apply {
                            if let Response::Err { message } =
                                self.set_workspace(target, Some(wanted))
                            {
                                return Response::Err { message: format!("{key}: {message}") };
                            }
                        }
                        let kind = self.tier_book().start_provider(target).session_kind();
                        match self.spawn_session(target, kind, true, None, Some(ticket), plan) {
                            Response::Spawned { .. } => (true, false, kind),
                            Response::Provisioning => (false, false, kind),
                            Response::Err { message } => return Response::Err { message },
                            other => {
                                return Response::Err {
                                    message: format!("unexpected spawn answer: {other:?}"),
                                }
                            }
                        }
                    }
                };
                self.feed.board(by.actor(), "start_agent", Some(target));
                self.crown_touched(ticket, target, if woken { "woken" } else { "started" });
                // A checkout worker's first delivery is a HEAD past this one;
                // a woken one's next is a HEAD past the wake.
                self.probe_turn(target, ProbeWhy::Baseline);
                let (held, _) = self.crown_seats();
                Response::AgentStarted {
                    key: self.board.ticket(target).map(|t| t.short_key.clone()).unwrap_or(key),
                    session_started,
                    budget_left: self.board.crown_budget.saturating_sub(held.len() as u8),
                    tier: self.tier_book().launch(target, kind).name,
                    woken,
                    workspace: crown::workspace_word(wanted).to_string(),
                }
            }
            // The crown's sleep (T-539): `x` on a card the crown started. The
            // gate is `sleep_one`'s own — idle only, no age floor, since a
            // keyed call is as deliberate as a keypress — and the scope is
            // `started_by`: a person's agent is the person's to park.
            Command::AgentSleepTicket { key, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; sleep_agent is for an agent \
                             the crown started"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let (sid, kind) = match self.crown_sleep_target(target, &key) {
                    Ok(pair) => pair,
                    Err(message) => return Response::Err { message },
                };
                // A working worker is refused in the words a person's `z`
                // reads over the same seat (`still_awake`), behind its key.
                if let Err(why) = self.sleep_one(sid, false) {
                    return Response::Err {
                        message: format!("{key}: {}", mesimon_core::quiet::still_awake(kind, &why)),
                    };
                }
                self.persist_sessions();
                self.feed.board(by.actor(), "sleep_agent", Some(target));
                self.crown_touched(ticket, target, "parked");
                self.agent_ticket_view(target)
                    .map_or_else(no_such_ticket, |ticket| Response::AgentTicket { ticket })
            }
            // The crown's watch (T-712): a ticket a person started, heard of
            // as a worker the crown started is — delivered, finished its
            // turn, raised its hand, merged — with nothing sent to it.
            // Behind the board's `crown_watches`, judged at each call after
            // the key and the stamp resolve; a worker the crown started is
            // refused, since the board already wakes it for that one. The
            // watch ends at the merge (`crownwake::owe_wake`), with
            // `unwatch`, when the ticket leaves the board (`drop_crown_if`),
            // when the row is turned off, and with the crown.
            Command::AgentWatchTicket { key, unwatch, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; watch_ticket is for another \
                             ticket"
                        ),
                    };
                }
                if !self.board.crown_watches {
                    return Response::Err { message: crown_watch_off(&key) };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                let word = if unwatch {
                    if !self.crown_watched.remove(&target) {
                        return Response::Err { message: format!("{key} is not watched") };
                    }
                    "unwatched"
                } else {
                    if self.board.started_by_crown(target) {
                        return Response::Err {
                            message: format!(
                                "{key}'s agent was started by the crown, and the board already \
                                 wakes this session for it; watch_ticket is for a ticket a \
                                 person started"
                            ),
                        };
                    }
                    self.crown_watched.insert(target);
                    "watched"
                };
                self.persist_crown();
                let cmd = if unwatch { "unwatch_ticket" } else { "watch_ticket" };
                self.feed.board(by.actor(), cmd, Some(target));
                self.crown_touched(ticket, target, word);
                match self.agent_ticket_view(target) {
                    Some(mut view) => {
                        view.watched = !unwatch;
                        Response::AgentTicket { ticket: view }
                    }
                    None => no_such_ticket(),
                }
            }
            // The crown's merge (T-613): `m` on a worker's page, for the
            // branches the train will not land. The road is `merge_ticket`'s
            // own — ff-only, refused under a working agent — under the
            // agent principal; what is judged here first is whose merge it
            // is (the board's crown mode) and whether the branch is one a
            // merge can take, each refusal in words naming the next step.
            Command::AgentMergeTicket { key, seen } => {
                let target = match self.keyed_target(ticket, Some(&key), seen.as_deref(), false) {
                    Ok(t) => t,
                    Err(message) => return Response::Err { message },
                };
                if target == ticket {
                    return Response::Err {
                        message: format!(
                            "{key} is this session's own ticket; merge_ticket is for a worker's \
                             branch, and this one is the board's to merge"
                        ),
                    };
                }
                let by = Principal::Agent { session };
                if let Decision::Deny { reason } =
                    authorize(&by, &Action::Mutate, &Resource::Ticket { id: target })
                {
                    return Response::Err { message: format!("denied: {reason}") };
                }
                if let Some(message) = self.crown_merge_refusal(target, &key) {
                    return Response::Err { message };
                }
                match self.merge_ticket(target, &by) {
                    Response::Merge { outcome: MergeOutcome::Merged, detail, .. } => {
                        self.feed.board_outcome(by.actor(), "merge_ticket", Some(target), "merged");
                        let notice = self.notice_after_merge(target, &by);
                        self.crown_touched(ticket, target, "merged");
                        Response::AgentMerged {
                            key: self
                                .board
                                .ticket(target)
                                .map(|t| t.short_key.clone())
                                .unwrap_or(key),
                            detail,
                            notice: notice.to_string(),
                            seen: Some(self.seen_token(target)),
                        }
                    }
                    Response::Merge { outcome: MergeOutcome::NeedsRebase, detail, .. } => {
                        self.feed.board_outcome(
                            by.actor(),
                            "merge_ticket",
                            Some(target),
                            "needs_rebase",
                        );
                        Response::Err { message: format!("{key}: {detail}: {}", rebase_first()) }
                    }
                    Response::Merge { outcome: MergeOutcome::AlreadyMerged, detail, .. } => {
                        Response::Err { message: format!("{key}: {detail}") }
                    }
                    Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
                        self.feed.board_outcome(
                            by.actor(),
                            "merge_ticket",
                            Some(target),
                            "refused",
                        );
                        Response::Err { message: format!("{key}: {detail}") }
                    }
                    other => other,
                }
            }
            Command::AgentCreateTicket {
                title,
                column,
                description,
                tags,
                idempotency_key,
                tier,
                workspace,
            } => {
                // Replay first, for the same reason as a move: a retry after
                // `Connection closed` must not file the same work twice.
                if let Some(key) = &idempotency_key {
                    if let Some(AgentReplay::Created { key: short_key, column, workspace }) =
                        self.agent_replay.get(&(session, key.clone()))
                    {
                        return Response::AgentCreated {
                            key: short_key.clone(),
                            column: column.clone(),
                            board_version: self.board_version,
                            replayed: true,
                            workspace: workspace.clone(),
                        };
                    }
                }
                let by = Principal::Agent { session };
                let filing = AgentFiling { title, column, description, tags, tier, workspace };
                let resp = self.agent_create_ticket(&by, ticket, filing);
                if let (
                    Some(key),
                    Response::AgentCreated { key: short_key, column, workspace, .. },
                ) = (idempotency_key, &resp)
                {
                    self.remember_agent_result(
                        session,
                        key,
                        AgentReplay::Created {
                            key: short_key.clone(),
                            column: column.clone(),
                            workspace: workspace.clone(),
                        },
                    );
                }
                resp
            }
            Command::AgentTagTicket { name, group, remove, key } => {
                let target = match key {
                    None => ticket,
                    Some(k) => match self.crown_target(ticket, &k, false) {
                        Ok(t) => t,
                        Err(message) => return Response::Err { message },
                    },
                };
                let by = Principal::Agent { session };
                match self.agent_tag_ticket(&by, target, &name, group, remove) {
                    Response::AgentTagged { tags, replaced, board_version, .. } => {
                        let seen = self.crown_touched(ticket, target, "tagged");
                        Response::AgentTagged { tags, replaced, board_version, seen }
                    }
                    other => other,
                }
            }
            Command::AgentRaiseHand { reason } => {
                let by = Principal::Agent { session };
                self.agent_raise_hand(&by, ticket, &reason)
            }
            // Unreachable: `agent_allows` above admits exactly seventeen commands.
            _ => Response::Err { message: "not available to an agent session".into() },
        }
    }

    // ------------------------------------------------------------ the crown

    /// The seats the crown's starts hold right now (T-412): the keys of
    /// every ticket on the board whose agent carries `started_by` and still
    /// holds its seat awake (`Board::crown_started`, which skips a parked
    /// agent, T-541, and an archived ticket, T-518), plus the starts parked
    /// behind a worktree provision — those
    /// have no record yet and would otherwise let a burst of worktree
    /// tickets outrun the cap — and the parked agents the crown's own ask
    /// is waiting to wake (T-550). The second value is how many starts are
    /// parked.
    fn crown_seats(&self) -> (Vec<String>, usize) {
        let mut keys: Vec<String> = self
            .board
            .crown_started()
            .iter()
            .filter_map(|s| self.board.ticket(s.ticket).map(|t| t.short_key.clone()))
            .collect();
        let parked: Vec<String> = self
            .pending_spawns
            .iter()
            .filter(|s| s.started_by.is_some())
            .filter_map(|s| self.board.ticket(s.ticket).map(|t| t.short_key.clone()))
            .collect();
        let n = parked.len();
        keys.extend(parked);
        // A wake the crown's own ask will make (T-550): the agent is
        // asleep and out of `crown_started` until the words go, and a
        // start behind it must not take the seat they will need.
        keys.extend(
            self.queued
                .iter()
                .filter(|q| q.sends && matches!(q.seat, QueuedSeat::Wake(_)))
                .filter_map(|q| self.board.ticket(q.ticket).map(|t| t.short_key.clone())),
        );
        keys.sort();
        keys.dedup();
        (keys, n)
    }

    /// Why the crown may not start another agent right now, if it may not:
    /// the cap and the tickets holding it, so the agent can wait for one to
    /// finish, or relay the number to the person who sets it.
    fn crown_budget_refusal(&self) -> Option<String> {
        let budget = self.board.crown_budget;
        let (held, _) = self.crown_seats();
        if held.len() < budget as usize {
            return None;
        }
        Some(if budget == 0 {
            "the crown's spawn budget is 0 on this board: start_agent is off \
             (Settings → Agents → Crown may start … sets it)"
                .to_string()
        } else {
            format!(
                "the crown's spawn budget is spent: {budget} of {budget} crown-started agents \
                 are awake ({}). A seat frees when its agent sleeps (sleep_agent parks an idle \
                 one) or exits, or the person raises the budget in Settings → Agents",
                held.join(", ")
            )
        })
    }

    /// Why the crown's ask for `target` waits on a person's send, or `None`
    /// when the queue delivers it (T-550). In order: the board's switch;
    /// the agent's provenance — the one who started an agent is the one who
    /// steers it (T-539), so a person's agent waits for its person whatever
    /// the switch says; and for a parked agent, the budget. Words to a
    /// sleeping agent wake it, and a wake on the crown's word spends what a
    /// start spends, so it needs a free seat and holds one while it waits
    /// (`crown_seats`). And a wake on the shared checkout waits while
    /// another ticket's agent holds it (T-583): the daemon's own delivery
    /// is held to `start_agent`'s rule, and a person's send is not.
    /// `crown_ticket` is the crown's own, which coordinates rather than holds.
    /// The words are the crown's to relay to the person.
    fn crown_ask_hold(
        &self,
        crown_ticket: ulid::Ulid,
        target: ulid::Ulid,
        seat: &QueuedSeat,
    ) -> Option<String> {
        if !self.board.crown_mode.sends() {
            return Some(
                "this board holds the crown's asks for a person to send (Settings → Agents → \
                 Crown mode: supervised)"
                    .into(),
            );
        }
        if self.board.live_agent(target).is_none_or(|rec| rec.started_by.is_none()) {
            return Some(
                "a person started this agent, and a person sends it words (^y on its card)".into(),
            );
        }
        if let QueuedSeat::Wake(id) = seat {
            let checkout = self.paths.repo_root.to_string_lossy();
            let on_checkout = self.board.sessions.iter().find(|s| s.id == *id).is_some_and(|rec| {
                crown::runs_in(rec, &checkout) == WorkspaceStrategy::SharedCheckout
            });
            if on_checkout {
                let holders =
                    crown::checkout_holders(&self.board, &checkout, &[crown_ticket, target]);
                let by = Principal::Automation { rule: "queued_ask".into() };
                if let Some(who) = crown::checkout_refusal(&by, &holders) {
                    return Some(format!(
                        "its agent is parked on the shared checkout and the words would wake it \
                         there, but {who}; a person's send wakes it, or the ask can be made \
                         again once the checkout is free"
                    ));
                }
            }
        }
        if matches!(seat, QueuedSeat::Wake(_)) {
            let budget = self.board.crown_budget;
            let own = self.board.ticket(target).map(|t| t.short_key.as_str());
            let (held, _) = self.crown_seats();
            let held: Vec<String> = held.into_iter().filter(|k| Some(k.as_str()) != own).collect();
            if held.len() >= budget as usize {
                return Some(format!(
                    "its agent is asleep and the words would wake it, but the crown's budget is \
                     spent ({} of {budget} awake{}); a person's send wakes it, or sleep_agent on \
                     another frees a seat and the ask can be made again",
                    held.len(),
                    if held.is_empty() { String::new() } else { format!(": {}", held.join(", ")) },
                ));
            }
        }
        None
    }

    /// The session `sleep_agent` may park on `target` (T-539), or why none:
    /// the ticket's agent seat, held by a record the crown started, with a
    /// pane to give up. The idle gate is `sleep_one`'s, judged after this,
    /// so a working agent reads the same `still awake — …` words a person's
    /// `z` would. A person's agent is refused by provenance alone, whatever
    /// its state: the one who started it is the one who parks it.
    fn crown_sleep_target(
        &self,
        target: ulid::Ulid,
        key: &str,
    ) -> std::result::Result<(uuid::Uuid, SessionKind), String> {
        let Some(rec) = self.board.live_agent(target) else {
            return Err(format!("{key} has no agent to park"));
        };
        if rec.started_by.is_none() {
            return Err(format!(
                "{key}'s agent was started by a person, and a person parks it (x on its card); \
                 sleep_agent is for an agent the crown started"
            ));
        }
        if !rec.state.has_pane() {
            return Err(format!("{key}'s agent is already asleep"));
        }
        Ok((rec.id, rec.kind))
    }

    /// Why the crown's `archive_ticket` is refused over an awake seat, in
    /// the crown's own words (T-539): the generic gate's "sleep them first"
    /// is a person's instruction, and the crown has one road for a seat it
    /// started and none for a person's. `None` when nothing is awake, or a
    /// shell is — the generic gate then answers as before.
    fn crown_archive_refusal(&self, target: ulid::Ulid, key: &str) -> Option<String> {
        let rec = self.board.live_agent(target).filter(|r| r.state.has_pane())?;
        Some(if rec.started_by.is_some() {
            format!(
                "{key}'s agent is still awake ({}); sleep_agent parks it, then archive_ticket",
                agent_state_word(&rec.state)
            )
        } else {
            format!(
                "{key}'s agent is still awake ({}) and was started by a person, who parks it \
                 (x on its card) before the ticket can be archived",
                agent_state_word(&rec.state)
            )
        })
    }

    /// The ticket a keyed call is about (T-411): the caller's own when the
    /// key names it — a session may always address its own card, crown or
    /// not — else the key's ticket, for the crown alone. An unknown key is
    /// an answer; an uncrowned caller reads how a person grants the crown.
    /// `archived_ok` admits an archived target (a restore's whole point);
    /// otherwise an archived card is off the board and says so.
    fn crown_target(
        &self,
        own: ulid::Ulid,
        key: &str,
        archived_ok: bool,
    ) -> std::result::Result<ulid::Ulid, String> {
        let Some(t) = self.board.ticket_by_key(key) else {
            return Err(format!("no such ticket: {key} (list_board lists every key)"));
        };
        if t.id == own {
            return Ok(own);
        }
        if !self.board.is_crowned(own) {
            return Err(self.crown_refusal(own));
        }
        if t.is_archived() && !archived_ok {
            return Err(format!("{key} is archived; archive_ticket with restore brings it back"));
        }
        Ok(t.id)
    }

    /// `crown_target` plus the freshness check every keyed WRITER makes:
    /// the `seen` stamp `get_ticket` handed out for the target has to match
    /// the ticket as it stands, or the write is refused with the current
    /// state — read-before-write as a check rather than a claim. The
    /// caller's own ticket needs no stamp; a key that names it is an
    /// own-ticket call.
    fn keyed_target(
        &self,
        own: ulid::Ulid,
        key: Option<&str>,
        seen: Option<&str>,
        archived_ok: bool,
    ) -> std::result::Result<ulid::Ulid, String> {
        let Some(key) = key else { return Ok(own) };
        let target = self.crown_target(own, key, archived_ok)?;
        if target == own {
            return Ok(own);
        }
        self.check_seen(target, seen)?;
        Ok(target)
    }

    /// The words an uncrowned agent reads when it reaches for another
    /// ticket: who wears the crown, how a person grants it, and the road
    /// that puts the ask on the card. Transient result data, so it may
    /// instruct — the lint that keeps tool text descriptive does not reach
    /// a refusal, and this one exists to be relayed to the person.
    fn crown_refusal(&self, own: ulid::Ulid) -> String {
        let key = self.board.ticket(own).map(|t| t.short_key.clone()).unwrap_or_default();
        let who = match self.board.crown_holder() {
            Some(h) => format!("{} wears the crown", h.short_key),
            None => "no ticket wears the crown".to_string(),
        };
        format!(
            "not the board's coordinator: {who}. Editing another ticket needs the crown, \
             which only a person grants — on the board, with the cursor on {key}, ^o crowns it. \
             raise_hand puts this request on the card so the person sees it without opening \
             the pane."
        )
    }

    /// An opaque stamp over everything a keyed edit may assume about a
    /// ticket: where it sits, what it says, what it wears, and what its
    /// agent is doing. Recomputed on every read and every check, never
    /// stored, so a daemon restart simply asks for a fresh read.
    fn seen_token(&self, id: ulid::Ulid) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(t) = self.board.ticket(id) {
            t.column.hash(&mut h);
            t.order.hash(&mut h);
            t.title.hash(&mut h);
            for r in &t.tags {
                r.group.hash(&mut h);
                r.name.hash(&mut h);
            }
            for n in &t.notes {
                n.id.to_string().hash(&mut h);
                n.rev.hash(&mut h);
            }
            t.workspace_strategy().word().hash(&mut h);
            t.is_archived().hash(&mut h);
            t.raised.as_ref().map(|r| r.reason.as_str()).hash(&mut h);
            if let Some(s) = self.board.live_agent(id) {
                s.state_word().hash(&mut h);
                s.state_changed_at.hash(&mut h);
            }
        }
        format!("{:016x}", h.finish())
    }

    fn check_seen(&self, id: ulid::Ulid, seen: Option<&str>) -> std::result::Result<(), String> {
        let key = self.board.ticket(id).map(|t| t.short_key.clone()).unwrap_or_default();
        let Some(seen) = seen.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err(format!(
                "seen is required for {key}: read it with get_ticket first and pass the seen stamp it returns"
            ));
        };
        if seen != self.seen_token(id) {
            let col = self.board.ticket(id).map(|t| t.column.clone()).unwrap_or_default();
            let state = self
                .agent_state_view(id)
                .map(|s| s.state)
                .unwrap_or_else(|| "no agent".to_string());
            return Err(format!(
                "{key} changed since it was read (now in {col}, agent {state}); read it again with get_ticket"
            ));
        }
        Ok(())
    }

    /// The card's words about a ticket's agent (T-411): one state word, how
    /// long, and the raised hand's own sentence. Never the pane.
    fn agent_state_view(&self, id: ulid::Ulid) -> Option<AgentStateView> {
        let t = self.board.ticket(id)?;
        let s = self.board.live_agent(id)?;
        let now = mesimon_core::clock::now_ms();
        // The foreground lives in the daemon's map, never on the board's
        // record (T-366): a `!` command running under an idle seat reads
        // `working` here as it does on the card (T-707).
        let mut seat = s.clone();
        seat.foreground = self.foregrounds.get(&s.id).cloned();
        Some(AgentStateView {
            state: seat.state_word().to_string(),
            since_secs: s.state_changed_at.map(|at| now.saturating_sub(at) / 1000),
            raised: t.raised.as_ref().map(|r| r.reason.clone()),
        })
    }

    /// Record that the crown touched `target` (T-411) so the board lights
    /// the card, and hand back the target's fresh stamp. An own-ticket
    /// call is neither: the caller's card already shows its own agent.
    fn crown_touched(
        &mut self,
        own: ulid::Ulid,
        target: ulid::Ulid,
        action: &str,
    ) -> Option<String> {
        if target == own {
            return None;
        }
        let now = mesimon_core::clock::now_ms();
        self.crown_touches.retain(|_, t| now.saturating_sub(t.at_ms) < CROWN_TOUCH_KEPT_MS);
        self.crown_touches.insert(
            target,
            CrownTouch { ticket: target, action: action.to_string(), at_ms: now, from: Some(own) },
        );
        Some(self.seen_token(target))
    }

    /// The crown's ask to `ticket` left the queue unsent (T-568): its next
    /// `get_ticket` on the ticket reads `asked: dropped`, and who.
    fn crown_dropped_ask(&mut self, ticket: ulid::Ulid, crown: ulid::Ulid, by: &'static str) {
        let board = &self.board;
        self.crown_dropped.retain(|t, _| board.ticket(*t).is_some());
        self.crown_dropped.insert(ticket, CrownDrop { crown, by, at_ms: now_ms() });
    }

    /// What the crown on `crown` reads of its last ask to `ticket` (T-568).
    fn crown_asked_view(
        &self,
        crown: ulid::Ulid,
        ticket: ulid::Ulid,
    ) -> Option<mesimon_core::command::AgentAskedView> {
        let drop = self.crown_dropped.get(&ticket).filter(|d| d.crown == crown)?;
        Some(mesimon_core::command::AgentAskedView {
            status: "dropped".into(),
            by: drop.by.into(),
            since_secs: Some(now_ms().saturating_sub(drop.at_ms) / 1000),
        })
    }

    /// The touches still worth drawing, for the snapshot.
    fn recent_crown_touches(&self) -> Vec<CrownTouch> {
        let now = mesimon_core::clock::now_ms();
        let mut out: Vec<CrownTouch> = self
            .crown_touches
            .values()
            .filter(|t| now.saturating_sub(t.at_ms) < CROWN_TOUCH_MS)
            .cloned()
            .collect();
        out.sort_by_key(|t| t.at_ms);
        out
    }

    /// `Command::CrownTicket` (T-411): one ticket wears it, this one now.
    /// Persisted with the board's other scalars, so a restart keeps the
    /// seat; the feed line is the generic dispatch's, with the person as
    /// actor.
    fn crown_ticket(&mut self, id: ulid::Ulid) -> Response {
        if let Err(message) = self.open_ticket(id) {
            return Response::Err { message: message.into() };
        }
        // One level deep by construction (T-412): an agent the crown started
        // can never wear the crown, so no crown-started agent starts agents.
        if self.board.live_agent(id).is_some_and(|s| s.started_by.is_some()) {
            return Response::Err {
                message: "its agent was started by the crown — a crown-started ticket cannot \
                          be crowned (kill or sleep-and-forget that session first)"
                    .into(),
            };
        }
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.crown == Some(id) {
            return Response::Ok;
        }
        // A new crown is owed nothing the old one was (T-414), and the old
        // one's words go nowhere on their own (T-550).
        self.drop_crown_wakes();
        self.hold_crown_sends();
        self.board.crown = Some(id);
        self.persist_columns();
        self.broadcast();
        Response::Ok
    }

    fn uncrown(&mut self) -> Response {
        if self.board.crown.is_none() {
            return Response::Ok;
        }
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        self.drop_crown_wakes();
        self.hold_crown_sends();
        self.board.crown = None;
        self.persist_columns();
        self.broadcast();
        Response::Ok
    }

    /// A crowned ticket leaving the board — deleted, archived — takes the
    /// crown with it: a seat nobody can see is not a seat. Said in the
    /// feed as the daemon's own doing. A watched ticket leaving takes its
    /// watch (T-712): nothing more will happen to it.
    fn drop_crown_if(&mut self, id: ulid::Ulid) {
        if self.crown_watched.remove(&id) {
            self.feed.board("automation", "watch_ended", Some(id));
        }
        if self.board.crown == Some(id) {
            self.drop_crown_wakes();
            self.hold_crown_sends();
            self.board.crown = None;
            self.persist_columns();
            self.feed.board("automation", "uncrown", Some(id));
        }
    }

    /// The crown's asks still waiting to send are held for the person from
    /// here (T-550): the mode went supervised, or the crown left the ticket
    /// that asked. Its authority to send was the mode AND the crown, so either
    /// ending ends the road; the words stay on the card for `^y` or `^u`.
    fn hold_crown_sends(&mut self) {
        for q in self.queued.iter_mut().filter(|q| q.sends) {
            q.sends = false;
        }
    }

    /// What keeps a wake from going out right now, if anything: a person's
    /// ask queued for the crown (theirs goes first, the wake follows as its
    /// own turn), a paste of ours still owed its ack, an empty seat (the
    /// wake starts nobody), or the crown's own session mid-turn. The
    /// checkout-wide quiet test the queue applies is deliberately NOT here:
    /// a crown in the shared checkout would otherwise wait for every worker
    /// it started to fall silent before hearing about the first.
    fn crown_wake_blocked(&self, crown: ulid::Ulid) -> bool {
        if self.queued.iter().any(|q| q.ticket == crown && q.by.is_none()) || self.owed_on(crown) {
            return true;
        }
        match self.seat_of(crown) {
            QueuedSeat::Pane(id) => !self.session_takes_words(id),
            QueuedSeat::Wake(_) => false,
            QueuedSeat::Start(_) => true,
        }
    }

    /// Deliver the owed wakes as one sentence, if the crown can take it
    /// (`crown_wake_blocked`). The road is `deliver` — the queue's own: a
    /// pane is pasted, a parked crown is woken with the words parked for
    /// its first tick. A pane's paste is owed an ack, so the crown is held
    /// in `owed` until its `UserPromptSubmit`, exactly as a queued ask is.
    fn drain_crown_wakes(&mut self) -> bool {
        if self.crown_wakes.is_empty() {
            return false;
        }
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else {
            self.drop_crown_wakes();
            return true;
        };
        if self.crown_wake_blocked(crown) {
            return false;
        }
        let seat = self.seat_of(crown);
        let id = match seat {
            QueuedSeat::Pane(id) | QueuedSeat::Wake(id) => id,
            QueuedSeat::Start(_) => return false,
        };
        let by = Principal::Automation { rule: "crown_wake".into() };
        if authorize(&by, &Action::Mutate, &Resource::Session { id }).denied() {
            self.crown_wakes.clear();
            self.feed.board("automation", "crown_wake_failed", Some(crown));
            return true;
        }
        let text = self.crown_wake_text();
        let ack = Ack { by: "automation", word: "crown_wake_delivered" };
        match self.deliver(crown, seat, text, ack, false, false) {
            Response::Err { message } => {
                eprintln!("mesimon: crown wake failed: {message}");
                self.crown_wakes.clear();
                self.feed.board("automation", "crown_wake_failed", Some(crown));
            }
            _ => {
                self.crown_wakes.clear();
                self.feed.board("automation", "crown_wake_sent", Some(crown));
            }
        }
        true
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
    /// the whole call and leaves no half-filed card behind. The arguments
    /// are the tool's own fields, one each.
    #[allow(clippy::too_many_arguments)]
    fn agent_create_ticket(
        &mut self,
        by: &Principal,
        from: ulid::Ulid,
        filing: AgentFiling,
    ) -> Response {
        let AgentFiling { title, column, description, tags, tier, workspace } = filing;
        // The crown files what it may start, so it says where that work
        // will run (T-583); a worker's ticket is a person's to pick up, and
        // the column's default stands unless it names one.
        let workspace = match workspace.as_deref().map(crown::parse_workspace) {
            Some(Ok(ws)) => Some(ws),
            Some(Err(message)) => return Response::Err { message },
            None if self.board.is_crowned(from) => {
                return Response::Err {
                    message: "the crown files a ticket with its workspace decided: workspace is \
                              worktree (its own branch) or shared_checkout"
                        .into(),
                }
            }
            None => None,
        };
        // No column named means the board's default (T-279): the one chosen
        // in Settings while it still exists, else the first column.
        let Some(column) = column.or_else(|| self.board.landing_column()) else {
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
        let tags = match self.resolve_agent_tags(&tags) {
            Ok(refs) => refs,
            Err(message) => return Response::Err { message },
        };
        // The new ticket's tier (T-584) is the crown's to pick, by the
        // person's words on each: a worker's ticket is an idea for a person
        // to pick up, and its tier is that person's.
        let tier = match tier {
            None => None,
            Some(_) if !self.board.is_crowned(from) => {
                return Response::Err {
                    message: "tier is the crown's to pick; a ticket filed here is for a person \
                              to pick up, and the person picks its tier (^n on its card)"
                        .into(),
                }
            }
            Some(word) => match self.tier_book().resolve_offered(&word) {
                Ok(t) => Some(t.id),
                Err(message) => return Response::Err { message },
            },
        };
        // One mint (T-243): a description the daemon would refuse refuses the
        // ticket with it, so the receipt never has to say "created, but".
        let mint = Mint {
            column: column.clone(),
            title,
            workspace,
            from: Some(from),
            tags,
            note: description.map(|text| (text, Vec::new())),
            tier,
            envelope: None,
        };
        let id = match self.mint_full(by, mint) {
            Ok(id) => id,
            Err(message) => return Response::Err { message },
        };
        self.feed.board(by.actor(), "create_ticket", Some(id));
        // The crown's filing is one of its touches (T-544), so the board
        // strikes the new card from the crown's; any other agent's is not.
        if self.board.is_crowned(from) {
            self.crown_touched(from, id, "created");
        }
        let t = self.board.ticket(id);
        let key = t.map(|t| t.short_key.clone()).unwrap_or_default();
        let workspace =
            t.map(|t| crown::workspace_word(t.workspace_strategy())).unwrap_or_default();
        Response::AgentCreated {
            key,
            column,
            board_version: self.board_version,
            replayed: false,
            workspace: workspace.to_string(),
        }
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
            return no_such_ticket();
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
            if let err @ Response::Err { .. } =
                self.with_ticket(ticket, |t| t.set_tag(def.group, next))
            {
                return err;
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
        Response::AgentTagged { tags, replaced, board_version: self.board_version, seen: None }
    }

    /// `raise_hand`, for an agent (T-107): put the needs-you mark on the
    /// caller's own ticket, with one line saying why.
    ///
    /// A MUTATE on that ticket and nothing else — the card lights, `!N`
    /// counts it, the merge train leaves it alone — and the mark is the
    /// TICKET's, not the session's, so the `Stop` that lands moments after
    /// this call cannot wipe it and a daemon restart cannot forget it.
    ///
    /// Raising a hand that is already up REPLACES the words: an agent that
    /// learns more about what it is stuck on says the newer thing, and a
    /// second card is not what it asked for. Nothing is written and nothing
    /// is broadcast when the words did not change.
    fn agent_raise_hand(&mut self, by: &Principal, ticket: ulid::Ulid, reason: &str) -> Response {
        if let Decision::Deny { reason } =
            authorize(by, &Action::Mutate, &Resource::Ticket { id: ticket })
        {
            return Response::Err { message: format!("denied: {reason}") };
        }
        let Some(t) = self.board.ticket(ticket) else {
            return no_such_ticket();
        };
        // An archived ticket is off the board, so there is no card to light
        // and nothing would ever lower the hand. Said in words rather than
        // stored against a day the ticket comes back.
        if t.is_archived() {
            return Response::Err {
                message: "this ticket is archived — nothing on the board would show the mark"
                    .into(),
            };
        }
        let Some(reason) = mesimon_core::board::sanitize_reason(reason) else {
            return Response::Err { message: "reason is empty once sanitized".into() };
        };
        if t.raised.as_ref().is_some_and(|r| r.reason == reason) {
            return Response::AgentRaised { reason, board_version: self.board_version };
        }
        let raised = mesimon_core::board::Raised {
            at: now_iso(),
            by: by.note_author(),
            reason: reason.clone(),
        };
        if let err @ Response::Err { .. } =
            self.with_ticket(ticket, |t| t.raised = Some(raised.clone()))
        {
            return err;
        }
        self.feed.board(by.actor(), "raise_hand", Some(ticket));
        // A hand on an agent the crown started wakes the crown (T-414) —
        // that a hand went up, never the reason: one agent's words do not
        // start another's turn with no person between (T-413); the crown
        // reads the reason through `get_ticket`.
        self.note_crown_wake(ticket, WakeCause::Raised, None, None);
        Response::AgentRaised { reason, board_version: self.board_version }
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
    /// unmerged worktree does not advertise a `requires_merge` column and
    /// then refuse it.
    fn agent_allowed_columns(&self, id: ulid::Ulid) -> Vec<String> {
        let Some(t) = self.board.ticket(id) else { return Vec::new() };
        if self.agent_tools_for(id) < AgentTools::Full {
            return Vec::new();
        }
        let unmerged = self.ticket_unmerged(id);
        self.board
            .sorted_columns()
            .into_iter()
            .filter(|c| c.name != t.column)
            .filter(|c| !(unmerged && c.settings.requires_merge))
            .map(|c| c.name.clone())
            .collect()
    }

    /// Every column's description (T-467), keyed by name, for `get_ticket`
    /// and `list_board` alike: the user's words on what each column is for,
    /// so an agent choosing between BACKLOG and TODO is not guessing.
    fn agent_column_descriptions(&self) -> std::collections::BTreeMap<String, String> {
        self.board
            .columns
            .iter()
            .filter_map(|c| Some((c.name.clone(), c.settings.description.clone()?)))
            .collect()
    }

    /// The ticket holds a worktree branch that has not landed on the base.
    fn ticket_unmerged(&self, id: ulid::Ulid) -> bool {
        self.worktrees.get(&id).is_some_and(|b| !b.branch.is_empty() && !self.ticket_merged(id))
    }

    /// The ticket's merge state as a word — the same four the `m` flow derives
    /// from git, and a word rather than an enum for the same reason
    /// `WorktreeItem.status` is one.
    /// Who merges `id`'s branch when it is ready (T-613), and the one
    /// clause that decides it — `get_ticket`'s `merge` and the delivered
    /// wake's word. A person's: crown mode supervised, or a ticket whose
    /// words came from outside, which waits for the keyboard whoever asks
    /// (`authorize_execution`). The train's: it is armed, the ticket is on
    /// it and its column reaches a merge, as `train::lane` reads them. The
    /// crown's otherwise, under autonomous mode: the train off, the ticket
    /// out of it by `t`, a column the train stops short of, a raised hand
    /// the train leaves alone.
    fn merge_by(&self, id: ulid::Ulid) -> AgentMergeView {
        let view = |by: &str, why: &str| AgentMergeView { by: by.into(), why: why.into() };
        if !self.board.crown_mode.merges() {
            return view("person", "crown mode is supervised");
        }
        let Some(t) = self.board.ticket(id) else {
            return view("person", "no such ticket");
        };
        if !t.effective_execution_policy().allows_automation() {
            return view("person", "this ticket's words came from outside; a person merges it");
        }
        if !self.train.is_armed() {
            return view("crown", "the train is off");
        }
        if t.manual_merge {
            return view("crown", "out of the train (t)");
        }
        let reach = self.board.column(&t.column).map(|c| c.settings.train).unwrap_or_default();
        if reach != mesimon_core::board::TrainReach::Merge {
            return AgentMergeView {
                by: "crown".into(),
                why: format!(
                    "the train's reach in {} is {}",
                    mesimon_core::text::scrub_text(&t.column),
                    reach.word()
                ),
            };
        }
        if t.hand_raised() {
            return view("crown", "its hand is up, which the train leaves alone");
        }
        view("train", "the train is armed and reaches this column")
    }

    /// Why the crown's `merge_ticket` on `target` is refused before the
    /// merge road is taken (T-613), or `None`: the mode, then the branch as
    /// the flags read it — nothing to merge, merged, behind its base — and
    /// the rebase turn the train asked for, which the train holds for.
    fn crown_merge_refusal(&self, target: ulid::Ulid, key: &str) -> Option<String> {
        if !self.board.crown_mode.merges() {
            return Some(format!(
                "Settings → Agents → Crown mode is supervised: a person merges {key}, m on \
                 the ticket's page"
            ));
        }
        if self.worktrees_barred {
            return Some(self.barred_message("worktrees"));
        }
        let Some(state) = self.merge_state_word(target) else {
            let ws = self.board.ticket(target).map(|t| t.workspace_strategy());
            return Some(match ws {
                Some(WorkspaceStrategy::Worktree) => {
                    format!("{key} has no branch yet: its worktree is not cut")
                }
                _ => format!(
                    "{key} has no branch to merge: its workspace is the shared checkout, whose \
                     commits are on the base the moment they exist"
                ),
            });
        };
        match state {
            "merged" => Some(format!("{key}'s branch is already merged")),
            "clean" => Some(format!("{key}'s branch has no commits to merge yet")),
            "needs_rebase" => Some(format!(
                "{key}'s branch is behind its base ({}): {}",
                self.base_branch.as_deref().unwrap_or("main"),
                rebase_first()
            )),
            _ => {
                let base_tip = self.base_tip_of(target).to_string();
                self.train.in_rebase_turn(target, &base_tip).then(|| {
                    format!(
                        "{key} is in the rebase turn the merge train asked for; the board wakes \
                         this session when that turn ends, and merge_ticket is for after it"
                    )
                })
            }
        }
    }

    /// Paste the merged notice into `id`'s agent after a merge the board
    /// made for a person or the crown (T-613), where the merged-notice pref
    /// is on, as the train's pass does. The word is the receipt's: `sent`,
    /// `off`, `no_pane` or `failed`.
    fn notice_after_merge(&mut self, id: ulid::Ulid, by: &Principal) -> &'static str {
        if !self.merged_notice_wanted() {
            return "off";
        }
        if self.board.pane_target(id).is_none() {
            return "no_pane";
        }
        let req = mesimon_core::command::MergeRequest::MergedNotice;
        let ack = Ack { by: "automation", word: "merge_notice_landed" };
        match self.merge_to_agent(id, req, by, ack) {
            Response::Ok => {
                self.feed.board("automation", "merge_notified", Some(id));
                "sent"
            }
            _ => {
                self.feed.board("automation", "merge_notice_failed", Some(id));
                "failed"
            }
        }
    }

    /// Whether a merge the board makes tells the agent (T-613): the
    /// merged-notice pref. The board that armed the train said it
    /// (`Train::notice`); with no train armed — the crown merges where the
    /// train will not, and a board need not be open — it is read from the
    /// prefs files at the moment it is needed, this board's override first
    /// (T-361), then the machine's, and on by default as the pref is.
    fn merged_notice_wanted(&self) -> bool {
        if self.train.is_armed() {
            return self.train.notice();
        }
        let key = mesimon_core::prefs::PrefKey::MergeTrainNotice.name();
        pref_flag(&self.paths.prefs_file(), key)
            .or_else(|| {
                let machine = crate::paths::state_root().ok()?.join("prefs.json");
                pref_flag(&machine, key)
            })
            .unwrap_or(true)
    }

    /// A person's `m` (T-613): the merge, and for a worker the crown started
    /// the merged notice in the same step where the pref is on, so the
    /// crown's `merged` wake waits for that notice turn (T-596) and the
    /// person's second `m` is not owed — the dialog reads `notified` and
    /// skips its "tell the agent" stage. A person's own worker keeps the
    /// two steps: the one who started it is the one who tells it.
    fn hand_merge(&mut self, id: ulid::Ulid) -> Response {
        let by = Principal::Local;
        match self.merge_ticket(id, &by) {
            Response::Merge { outcome: MergeOutcome::Merged, detail, .. } => {
                let crown_started =
                    self.board.live_agent(id).is_some_and(|rec| rec.started_by.is_some());
                let notified = crown_started && self.notice_after_merge(id, &by) == "sent";
                Response::Merge { outcome: MergeOutcome::Merged, detail, notified }
            }
            other => other,
        }
    }

    fn merge_state_word(&self, id: ulid::Ulid) -> Option<&'static str> {
        let b = self.worktrees.get(&id)?;
        if b.branch.is_empty() {
            return None;
        }
        Some(worktree::merge_word(
            self.wt_agg.get(&id).is_some_and(|a| a.merged),
            self.wt_agg.get(&id).is_some_and(|a| a.needs_rebase),
            self.wt_agg.get(&id).map_or(0, |a| a.ahead),
        ))
    }

    fn agent_ticket_view(&self, id: ulid::Ulid) -> Option<AgentTicketView> {
        let t = self.board.ticket(id)?;
        let workspace = crown::workspace_word(t.workspace_strategy());
        Some(AgentTicketView {
            key: t.short_key.clone(),
            key_about: None,
            title: t.title.clone(),
            column: t.column.clone(),
            workspace: workspace.to_string(),
            branch: self.worktrees.get(&id).map(|b| b.branch.clone()).filter(|b| !b.is_empty()),
            merge_state: self.merge_state_word(id).map(str::to_string),
            repos: self.agent_repo_views(id),
            merge: self.merge_state_word(id).is_some().then(|| self.merge_by(id)),
            // The init script (T-614), on every read: the file's name,
            // whether the checkout has one, what it is for, and how its
            // last run for this ticket went. A stat, nothing more.
            worktree_init: Some(mesimon_core::command::AgentWorktreeInitView {
                script: mesimon_core::workspace::INIT_SCRIPT.into(),
                present: worktree::init_script(&self.paths.repo_root).is_some(),
                about: mesimon_core::mcp::WORKTREE_INIT_ABOUT.into(),
                last: self.worktrees.get(&id).and_then(|b| b.init.as_ref()).map(|r| r.word()),
            }),
            allowed_columns: self.agent_allowed_columns(id),
            column_descriptions: self.agent_column_descriptions(),
            automove: self
                .board
                .column(&t.column)
                .map(|c| AgentAutomoveView {
                    on_working: c.settings.on_working.clone(),
                    on_done: c.settings.on_done.clone(),
                })
                .unwrap_or_default(),
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
            // The summary (T-710), on every read: the rows under every
            // note's `Summary` heading as the card counts them, and the one
            // line that says what the heading does. The bodies are read
            // again here, as the description's is; the parse is the TUI's.
            summary: Some(mesimon_core::command::AgentSummaryView {
                about: mesimon_core::mcp::SUMMARY_ABOUT.into(),
                rows: t
                    .notes
                    .iter()
                    .flat_map(|n| {
                        let body =
                            store::read_note(&self.paths, &t.short_key, n.id).unwrap_or_default();
                        mesimon_core::summary::extract(&body)
                            .into_iter()
                            .map(|r| mesimon_core::command::AgentSummaryRow {
                                note: n.id,
                                text: r.text,
                                done: r.done,
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect(),
            }),
            crowned: self.board.is_crowned(id),
            watched: false,
            crown: self.board.is_crowned(id).then(|| mesimon_core::mcp::CROWN_WAKES.to_string()),
            under_crown: None,
            background: self.agent_background(id),
            state: self.agent_state_view(id),
            needs_you: self.agent_needs_you(id),
            asked: None,
            seen: Some(self.seen_token(id)),
        })
    }

    /// The ticket's agent idle at its composer with background tasks running
    /// (T-599): how many, and since its turn ended. Said, never judged.
    fn agent_background(&self, id: ulid::Ulid) -> Option<AgentBackgroundView> {
        let rec = self.board.live_agent(id)?;
        if rec.state != (SessionState::Idle { stop_reason: StopReason::Background }) {
            return None;
        }
        Some(AgentBackgroundView {
            tasks: u32::try_from(rec.background_tasks.count()).unwrap_or(u32::MAX),
            since_secs: rec.state_changed_at.map(|at| now_ms().saturating_sub(at) / 1000),
        })
    }

    /// What a `needs-you` agent's stop is (T-566): its reason, and on a
    /// question the dialog as Remote Control draws it, gated on the state
    /// as that draw is — a dialog the agent has since left is not shown.
    /// On a plan (T-582), the plan's markdown, for the crown to read before
    /// it accepts. The words are an agent's, leaving for another agent:
    /// scrubbed.
    fn agent_needs_you(&self, id: ulid::Ulid) -> Option<AgentNeedsYouView> {
        let rec = self.board.live_agent(id)?;
        let SessionState::RequiresAction { reason } = rec.state else { return None };
        let dialog = (reason == Reason::Question).then(|| self.control_questions(rec.id)).flatten();
        let plan = (reason == Reason::Plan).then(|| self.control_plan(rec.id)).flatten();
        let scrub = mesimon_core::text::scrub_text;
        Some(AgentNeedsYouView {
            reason: agent_reason_word(reason).to_string(),
            request: dialog
                .map(|(request, _)| request.to_string())
                .or_else(|| plan.map(|(request, _)| request.to_string())),
            plan: plan.map(|(_, markdown)| {
                let text = mesimon_core::text::scrub_lines(markdown);
                let cut = mesimon_core::text::cap_bytes(&text, AGENT_PLAN_MAX_BYTES);
                if cut.len() < text.len() {
                    format!("{cut}…")
                } else {
                    text
                }
            }),
            questions: dialog
                .map(|(_, qs)| qs)
                .unwrap_or_default()
                .iter()
                .map(|q| AgentQuestionView {
                    text: scrub(&q.question),
                    options: q.options.iter().map(|o| scrub(&o.label)).collect(),
                    multi_select: q.multi_select,
                })
                .collect(),
            answerable: dialog.map(|(_, qs)| mesophon::dialog_answerable(qs)),
        })
    }

    /// `person` or `agent` off a ticket's `created_by` (T-253), for a
    /// coordinator triaging what agents filed; a ticket from before the
    /// field says nothing rather than guessing a person.
    fn filed_by(t: &Ticket) -> Option<String> {
        if t.created_by.is_empty() {
            None
        } else if t.agent_created() {
            Some("agent".to_string())
        } else {
            Some("person".to_string())
        }
    }

    /// The board as an agent sees it: columns, and tickets' key/title/column.
    ///
    /// A hand-written projection rather than `Snapshot` with fields removed —
    /// the difference is that this one cannot grow a session field by accident
    /// when the board model does. Archived tickets are excluded because
    /// `column_tickets` is the archived-exclusion chokepoint and an archived
    /// ticket is off the board.
    /// `own` is the caller's ticket: `checkout_held_by` (T-583) names the
    /// OTHER tickets on the checkout, as `start_agent` judges them.
    fn agent_board_view(&self, own: ulid::Ulid) -> AgentBoardView {
        let columns: Vec<String> =
            self.board.sorted_columns().into_iter().map(|c| c.name.clone()).collect();
        let tickets = columns
            .iter()
            .flat_map(|name| {
                self.board.column_tickets(name).into_iter().map(|t| AgentTicketRow {
                    key: t.short_key.clone(),
                    title: t.title.clone(),
                    column: t.column.clone(),
                    by: Self::filed_by(t),
                    state: self.board.live_agent(t.id).map(|s| s.state_word().into()),
                })
            })
            .collect();
        AgentBoardView {
            columns,
            column_descriptions: self.agent_column_descriptions(),
            tickets,
            board_version: self.board_version,
            crown: self.board.crown_holder().map(|t| t.short_key.clone()),
            tiers: self.agent_tier_views(),
            checkout_held_by: crown::checkout_holders(
                &self.board,
                &self.paths.repo_root.to_string_lossy(),
                &[own],
            )
            .into_iter()
            .map(|h| h.key)
            .collect(),
        }
    }

    /// `list_board`'s tiers (T-584): what an agent may name for a ticket it
    /// files or starts (`Book::offered`), each with the person's words on
    /// when to use it. The words are scrubbed again here, because a
    /// hand-edited `tiers.toml` never crossed `save_tier`.
    fn agent_tier_views(&self) -> Vec<mesimon_core::command::AgentTierView> {
        let book = self.tier_book();
        let default = book.default_tier().id;
        book.offered()
            .into_iter()
            .map(|t| mesimon_core::command::AgentTierView {
                is_default: t.id == default,
                description: mesimon_core::tier::sanitize_description(&t.description),
                model: t.model_arg().unwrap_or_default().to_string(),
                id: t.id,
                name: t.name,
                provider: t.provider,
                effort: t.effort,
            })
            .collect()
    }

    /// The one function that moves a ticket between columns.
    ///
    /// Three callers today — the human's `MoveTicket`, `automove`, and the
    /// agent's `move_ticket` — and a fourth when M5 grows column on-enter
    /// actions. They were three separate implementations before T-84, which
    /// meant the DONE gate bound only one of them and nothing could see that
    /// two movers were undoing each other. Everything a move must obey now
    /// lives here, so a new mover obeys it by construction rather than by
    /// somebody remembering. `turn_of` is the session whose own turn edge
    /// made the move (automove), for the move gate's fuse.
    fn place_ticket(
        &mut self,
        id: ulid::Ulid,
        dest: &str,
        pos: Position,
        by: &Principal,
        turn_of: Option<uuid::Uuid>,
        rule: &str,
    ) -> std::result::Result<String, String> {
        let t = self.open_ticket(id).map_err(str::to_string)?;
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
        // M4 DONE gate (author rule 3), now the column's `requires_merge`
        // (T-117): entering means the work landed — an unmerged worktree
        // blocks the move. It lived inside the human's move path until T-84,
        // which would have let an automation route around the one rule that
        // keeps the board from claiming something shipped when git says it
        // did not.
        if self.board.column(dest).is_some_and(|c| c.settings.requires_merge)
            && self.ticket_unmerged(id)
        {
            return Err(format!("worktree unmerged — merge before {dest}"));
        }
        if let Decision::Deny { reason } =
            authorize(by, &Action::MoveTicket, &Resource::Ticket { id })
        {
            return Err(format!("denied: {reason}"));
        }
        if let Decision::Deny { reason } =
            authorize(by, &Action::MoveTicket, &Resource::Column { name: dest.to_string() })
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
            let stamp = now_iso();
            t.remember_column_stay(&stamp);
            t.column = dest.to_string();
            t.order = order;
            // The card's age is time in column, so a column change is the
            // one thing that restarts it (a reorder returned above).
            t.entered_at = Some(stamp);
            let t = t.clone();
            let _ = store::save_ticket(&self.paths, &t);
        }
        self.moves.record(id, &from, dest, by, turn_of, Instant::now());
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
        if let Decision::Deny { reason } =
            authorize(by, &Action::MoveTicket, &Resource::Ticket { id })
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

    /// One worktree binding as the TUI's snapshot carries it, and as a
    /// paired phone's board reads it (T-642).
    fn worktree_item(&self, tid: &ulid::Ulid, b: &worktree::Binding) -> WorktreeItem {
        WorktreeItem {
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
            merged: self.wt_agg.get(tid).is_some_and(|a| a.merged),
            merged_in: self.wt_agg.get(tid).map(|a| a.merged_in.clone()).unwrap_or_default(),
            merged_oid: self.wt_agg.get(tid).map(|a| a.merged_oid.clone()).unwrap_or_default(),
            conflict: !b.branch.is_empty() && self.wt_conflicts.contains(&b.branch),
            ahead: self.wt_agg.get(tid).map_or(0, |a| a.ahead),
            needs_rebase: self.wt_agg.get(tid).is_some_and(|a| a.needs_rebase),
            detail: match &b.status {
                BindingStatus::Error { stage, message } => Some(format!("{stage}: {message}")),
                BindingStatus::Provisioning if self.wt_init.contains(tid) => {
                    Some("init script running".into())
                }
                BindingStatus::Provisioning => {
                    self.wt_progress.get(tid).map(|(d, t)| format!("{d}/{t}"))
                }
                // How the init script went, while it went wrong (T-614).
                BindingStatus::Attached => b.init.as_ref().and_then(|r| r.detail()),
                _ => None,
            },
            path: (b.status == BindingStatus::Attached).then(|| b.path.display().to_string()),
            repos: if b.is_workspace() {
                self.wt_repos
                    .get(tid)
                    .map(|legs| {
                        legs.iter()
                            .map(|l| WorktreeRepoItem {
                                name: l.name.clone(),
                                base: l.base.clone(),
                                ahead: l.ahead,
                                merged: l.merged,
                                needs_rebase: l.needs_rebase,
                                conflict: l.conflict,
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        b.repos
                            .iter()
                            .map(|r| WorktreeRepoItem {
                                name: r.name.clone(),
                                base: r.base.clone(),
                                ahead: 0,
                                merged: false,
                                needs_rebase: false,
                                conflict: false,
                            })
                            .collect()
                    })
            } else {
                Vec::new()
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
        let worktrees = self.worktrees.iter().map(|(tid, b)| self.worktree_item(tid, b)).collect();
        // Standing notices, plus any ticket whose flap fuse is currently
        // blown. The fuse is a real change in how the board behaves — cards
        // stop moving themselves — so it is said out loud rather than left
        // for the user to notice as an absence.
        let pending = self.pending_items();
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
        // A merge the checkout refused, in the same voice and for the same
        // reason (T-289): the train has stopped trying and the card cannot
        // say why — its owed row is 22 cells and this is a sentence. Built
        // from `pending` so the row and the notice can never disagree; one
        // per distinct reason, naming its tickets in board order.
        let mut blocked: Vec<(&str, Vec<String>)> = Vec::new();
        for p in &pending {
            let (Some(detail), Some(t)) = (
                p.text.as_deref().filter(|_| p.action == PendingAction::Merge),
                self.board.ticket(p.ticket),
            ) else {
                continue;
            };
            match blocked.iter_mut().find(|(d, _)| *d == detail) {
                Some((_, keys)) => keys.push(t.short_key.clone()),
                None => blocked.push((detail, vec![t.short_key.clone()])),
            }
        }
        for (detail, keys) in blocked {
            notices.push(Notice::new(
                "merge_train_blocked",
                format!("merge train held for {} — {detail}", keys.join(", ")),
            ));
        }
        if !fused.is_empty() {
            fused.sort();
            notices.push(Notice::new(
                "automation_suspended",
                crate::movegate::suspended_notice(&fused),
            ));
        }
        // The machine's tiers file, quarantined or from a newer build
        // (T-443): a standing advisory like any state file's.
        notices.extend(self.machine_tiers.notice.clone());
        // The foregrounds ride the snapshot's records and nothing else: the
        // board on disk never carries one (T-366).
        // So does the count of an idle agent's background tasks (T-599): the
        // registry is never persisted.
        let mut board = self.board.clone();
        for rec in &mut board.sessions {
            rec.foreground = self.foregrounds.get(&rec.id).cloned();
            rec.tasks_running = (rec.kind.is_agent()
                && rec.state == (SessionState::Idle { stop_reason: StopReason::Background }))
            .then(|| u32::try_from(rec.background_tasks.count()).unwrap_or(u32::MAX));
            // And whether the seat never took a prompt (T-603): one `stat`
            // per idle or parked claude.
            rec.unprompted = self.unprompted_hint(rec);
        }
        let terminals = self
            .terminals
            .iter()
            .map(|(ticket, foreground)| TerminalItem {
                ticket: *ticket,
                foreground: foreground.clone(),
            })
            .collect();
        Response::Board {
            team: self.team_info(),
            mesophon: self.control_info(),
            board,
            terminals,
            grace,
            external: self.external.clone(),
            external_scanning: self.external_scanning,
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
                nested: self.stamped_nested(),
                ..self.git_cache.clone()
            },
            pending,
            automation: self.automation_status(),
            claude_md: self.claude_md.status().for_board(&self.board),
            claude_default_mode: user_default_mode(),
            status_top: self.backend.status_top(),
            crown_touches: self.recent_crown_touches(),
            machine_tiers: self.machine_tiers.tiers.clone(),
            usage: self.usage.view(),
            costs: self.costs.view(now_ms()),
        }
    }

    /// Start the quota reads that are due (T-327), each on a worker under the
    /// machine's lock and launched the way a pane is — the same CLI, the same
    /// sign-in. True when the view moved: another board's daemon wrote the
    /// shared file, or a read started (the dialog says `reading`).
    fn drive_usage(&mut self, now: u64) -> bool {
        let mut changed = self.usage.poll_file(now);
        // The launcher applies the captured shell environment; a first
        // capture still running would hand the probe the daemon's own.
        if self.shell_env_capturing && !self.paths.shell_env_file().exists() {
            return changed;
        }
        for p in self.usage.due(now) {
            let (bin, args) = match p {
                Provider::Claude => (
                    std::env::var("MESIMON_CLAUDE_BIN").unwrap_or_else(|_| "claude".into()),
                    crate::usage::claude_args(),
                ),
                Provider::Codex => (
                    std::env::var("MESIMON_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
                    crate::usage::codex_args(),
                ),
            };
            let mut argv = vec![bin];
            argv.extend(args);
            let argv = self.launch(&argv, &[]);
            let cwd = self.paths.state_dir.clone();
            let lock = crate::usage::shared_file().map(|f| f.with_extension("lock"));
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let outcome = crate::usage::probe(p, &argv, &cwd, lock.as_deref(), now_ms());
                let _ = tx.send(Msg::UsageRead(p, outcome));
            });
            changed = true;
        }
        changed
    }

    fn on_usage_read(&mut self, p: Provider, outcome: crate::usage::Outcome) {
        if self.usage.landed(p, outcome, now_ms()) {
            self.broadcast();
        }
    }

    /// `Command::SetStatusLine` (T-264): the preference reaches tmux — the
    /// live server and the conf the next one reads — and the snapshot says
    /// where it landed so the TUI stops pushing. No server yet is fine: the
    /// conf carries the word until one starts.
    fn set_status_line(&mut self, top: bool) -> Response {
        if let Err(e) = self.backend.set_status_position(top) {
            return Response::Err { message: format!("could not move the status line: {e}") };
        }
        self.broadcast();
        Response::Ok
    }

    /// What mesimon owes each ticket (see `Pending`). Empty until the queued
    /// ask and the merge train land; kept in one place so the snapshot road
    /// forks no git.
    fn pending_items(&self) -> Vec<Pending> {
        let order = self.queue_order();
        let mut out: Vec<Pending> = order
            .iter()
            .map(|&i| &self.queued[i])
            .map(|q| Pending {
                ticket: q.ticket,
                // The seat's own word, so the card can say a session will
                // START rather than that words are queued (T-294).
                action: q.seat.action(),
                waits_on: self.ask_waits_on(q.ticket, &order),
                asking: self.ask_asking(q.ticket, &order),
                text: (!q.text.is_empty()).then(|| q.text.clone()),
                in_flight: false,
                by: q.by.and_then(|c| self.board.ticket(c)).map(|c| c.short_key.clone()),
                sends: q.sends,
                accept_plan: q.accept_plan || q.send_on_accept,
                held: q.held.map(str::to_string),
                plan: q.plan,
                deliver: q.deliver,
            })
            .collect();
        for o in self.owed.values() {
            if o.ack.word == Ack::QUEUED.word {
                out.push(Pending {
                    ticket: o.ticket,
                    action: PendingAction::Ask,
                    waits_on: Vec::new(),
                    asking: Vec::new(),
                    text: None,
                    in_flight: true,
                    by: None,
                    sends: false,
                    accept_plan: false,
                    held: None,
                    plan: false,
                    deliver: Deliver::Idle,
                });
            }
        }
        // The wake the crown is owed (T-414), after the queue's rows so a
        // person's own ask on the crown ticket is the card's row. The text
        // is the sentence as it would go out now, for the ticket page.
        if let Some(crown) =
            self.board.crown_holder().map(|t| t.id).filter(|_| !self.crown_wakes.is_empty())
        {
            let waits_on =
                if self.crown_wake_blocked(crown) { self.keys_of(&[crown]) } else { Vec::new() };
            out.push(Pending {
                ticket: crown,
                action: PendingAction::CrownWake,
                waits_on,
                asking: Vec::new(),
                text: Some(self.crown_wake_text()),
                in_flight: false,
                by: None,
                sends: false,
                accept_plan: false,
                held: None,
                plan: false,
                deliver: Deliver::Idle,
            });
        }
        // What the train will do once its gate is clear — said before it
        // happens, so the card can be watched rather than discovered. The
        // gate is `train_busy`, the same one the pass takes, so the card
        // names exactly what is actually holding it (T-351).
        if self.train.is_armed() && !self.worktrees_barred {
            let plan = self.train_plan();
            let waits_on = self.keys_of(&self.train_busy());
            // Rebases wait for pending merges even when the checkout refuses
            // them: asking against the old base would waste the agent's turn.
            let mut rebase_waits_on = waits_on.clone();
            for key in self.keys_of(&plan.merge) {
                if !rebase_waits_on.contains(&key) {
                    rebase_waits_on.push(key);
                }
            }
            for t in plan.merge {
                let tip = self.wt_agg.get(&t).map(|a| a.tip.clone()).unwrap_or_default();
                out.push(Pending {
                    ticket: t,
                    action: PendingAction::Merge,
                    waits_on: waits_on.clone(),
                    asking: Vec::new(),
                    text: self.train.refusal(t, &tip, self.base_tip_of(t)).map(String::from),
                    in_flight: false,
                    by: None,
                    sends: false,
                    accept_plan: false,
                    held: None,
                    plan: false,
                    deliver: Deliver::Idle,
                });
            }
            for t in plan.rebase {
                out.push(Pending {
                    ticket: t,
                    action: PendingAction::Rebase,
                    waits_on: rebase_waits_on.clone(),
                    asking: Vec::new(),
                    text: None,
                    in_flight: false,
                    by: None,
                    sends: false,
                    accept_plan: false,
                    held: None,
                    plan: false,
                    deliver: Deliver::Idle,
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
                current: r.base_oid == self.base_tip_of(*t),
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
            holding: self.train_holding(),
        }
    }

    /// Whether anything reads the periodic git sample: a subscribed board
    /// (the header) or an armed merge train (it retries a refused merge on
    /// the sample's delta, T-289). The train is owned by a connection and
    /// disarmed when it goes, so a daemon with neither is truly headless.
    fn git_has_reader(&self) -> bool {
        !self.subscribers.is_empty() || self.train.is_armed()
    }

    /// The sampled nested repos with mesimon's own fetch bookkeeping on them
    /// (T-455): fetching now, the newer of `FETCH_HEAD` and our last success,
    /// and our last failure while nothing has fetched the repo since.
    fn stamped_nested(&self) -> Vec<mesimon_core::command::RepoSync> {
        let mut nested = self.git_cache.nested.clone();
        for s in &mut nested {
            s.fetching = self.git_fetching_repos.contains(&s.name);
            if let Some(seen) = self.git_nested_fetch.get(&s.name) {
                s.fetched_at_ms = s.fetched_at_ms.max(seen.ok_at_ms);
                s.fetch_error = seen
                    .error
                    .as_ref()
                    .filter(|(at, _)| *at > s.fetched_at_ms)
                    .map(|(_, e)| e.clone());
            }
        }
        nested
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
        let nested: Vec<mesimon_core::command::RepoSync> =
            if std::mem::take(&mut self.git_fetch_nested_wanted) {
                let compared = self.git_cache.nested.iter();
                compared.filter(|s| s.upstream.is_some() && !s.detached).cloned().collect()
            } else {
                Vec::new()
            };
        self.git_fetching_repos = nested.iter().map(|s| s.name.clone()).collect();
        let repo = self.paths.repo_root.clone();
        // The one nested repo that stands in for the checkout (T-225) is
        // where its branch's remote is configured.
        let fetch_in = crate::gitstatus::branch_dir(&repo, &self.git_cache);
        let branch = self.git_cache.branch.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let own = fetch.then_some((fetch_in, branch));
            let fetched = crate::gitstatus::fetch_pass(&repo, own, &nested);
            let _ = tx.send(Msg::GitSampled(crate::gitstatus::sample(&repo), fetched));
        });
    }

    /// A sample landed. Broadcast only on a visible change: an idle checkout
    /// sampled every 10 s must not repaint every board.
    fn on_git_sampled(
        &mut self,
        sample: mesimon_core::command::RepoGit,
        fetched: crate::gitstatus::Fetched,
    ) {
        self.git_inflight = false;
        let mut changed = std::mem::replace(&mut self.git_fetching, false);
        if !self.git_fetching_repos.is_empty() {
            self.git_fetching_repos.clear();
            changed = true;
        }
        let now = now_ms();
        for (name, verdict) in fetched.nested {
            let seen = self.git_nested_fetch.entry(name).or_default();
            match verdict {
                Ok(()) => {
                    (seen.ok_at_ms, seen.error) = (now, None);
                    // A leg's remote-tracking ref lives in its own repo
                    // (T-267, keyed by leg name), and this fetch can move it.
                    self.upstreams.clear();
                }
                Err(e) => seen.error = Some((now, e)),
            }
            changed = true;
        }
        if let Some(verdict) = fetched.root {
            self.git_last_fetch = Some(Instant::now());
            match verdict {
                Ok(()) => {
                    self.git_fetched_at_ms = now_ms();
                    self.git_fetch_error = None;
                    // A fetch can mint `refs/remotes/origin/HEAD` (git ≥ 2.48
                    // follows the remote's HEAD), which is the first rung of
                    // `default_branch`'s ladder: let it be asked again — and
                    // the same fetch is what moves `origin/main`, so its ref
                    // is re-asked with it (T-267).
                    self.base_branch = None;
                    self.upstreams.clear();
                }
                Err(e) => self.git_fetch_error = Some(e),
            }
            changed = true;
        }
        if sample != self.git_cache {
            // The checkout moved: a commit, a stash, a file written. A merge
            // the checkout REFUSED was refused by that state (T-289), and the
            // `(branch tip, base tip)` pair the refusal is keyed on does not
            // move when the user stashes — which is one of the two things the
            // refusal asks them to do. The sample's own delta is what lets the
            // train try again; the cost of being wrong is one ff-merge that
            // fails without writing anything.
            self.train.forget_refusals();
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

    /// The single write path for `columns.toml`. Barred means a file we
    /// could not read — or one a newer mesimon wrote — is still sitting
    /// there, and writing would destroy the only copy.
    fn persist_columns(&mut self) {
        if self.columns_barred {
            return;
        }
        let _ = self.write_columns();
    }

    /// `columns.toml` written when its text changed since the last write, so
    /// a session-only change costs the board file no fsync.
    fn write_columns(&mut self) -> anyhow::Result<()> {
        store::save_columns_if_changed(&self.paths, &self.board, &mut self.columns_written)
    }

    /// Also the one place `started.json` learns a key (T-441): every change
    /// of a record's conversation persists the sessions, so folding here
    /// misses none — and folds before the bar, which is `sessions.json`'s.
    fn persist_sessions(&mut self) {
        if crate::started::fold(&mut self.started, &self.board.sessions) {
            self.persist_started();
        }
        if self.sessions_barred {
            return;
        }
        // Many tick stages persist on a change that touched no record.
        let _ =
            store::save_sessions_if_changed(&self.paths, &self.board, &mut self.sessions_written);
    }

    /// The single write path for `costs.json` (T-327).
    pub(super) fn persist_costs(&self) {
        if self.costs_barred {
            return;
        }
        let _ = crate::cost::save(&self.paths, &self.costs);
    }

    /// Read what the tickets' transcripts grew since the last pass, on a
    /// worker (T-327). Every transcript a session points at now is learned
    /// first: the ledger keeps it after the record forgets it.
    fn queue_cost_scan(&mut self) {
        self.cost_due = false;
        let learned: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter_map(|r| {
                crate::cost::transcript_of(r)
                    .map(|(p, codex)| (r.ticket, p, codex, r.frames_by_mod()))
            })
            .collect();
        for (ticket, path, codex, by_mod) in learned {
            self.costs.learn(ticket, &path.display().to_string(), codex, by_mod);
        }
        let jobs = self.costs.jobs();
        if jobs.is_empty() {
            return;
        }
        self.cost_scanning = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::CostScanned(crate::cost::scan(jobs, now_ms())));
        });
    }

    /// A pass landed: fold it in, drop the tickets the board no longer has,
    /// save, and say so when a figure moved.
    fn on_cost_scanned(&mut self, done: Vec<crate::cost::Done>) {
        self.cost_scanning = false;
        let live: std::collections::HashSet<ulid::Ulid> =
            self.board.tickets.iter().map(|t| t.id).collect();
        let moved = self.costs.apply(done) | self.costs.keep(&live);
        if moved {
            self.persist_costs();
            self.broadcast();
        }
    }

    /// The single write path for `train.json` (T-635): the train's asks
    /// whose turn is still open, written when that set changed, so a quiet
    /// board writes nothing.
    fn persist_train(&mut self) {
        if self.train_barred {
            return;
        }
        let file = crate::train::TrainFile::of(&self.train);
        let Ok(text) = serde_json::to_string_pretty(&file) else { return };
        let _ = store::write_if_changed(
            &self.paths.train_file(),
            text,
            store::PRIVATE,
            &mut self.train_written,
        );
    }

    /// Read `train.json` back at start (T-635). An ask whose ticket is gone,
    /// or whose ticket has no agent that is working or not yet re-derived
    /// (a Claude reads `Unknown{DaemonRestarted}` until its transcript or a
    /// hook speaks), is dropped; the rest hold as they did, and the first
    /// edge out of a working state settles them (`apply_change`).
    fn restore_train(&mut self, file: crate::train::TrainFile) {
        // No file reads as no asks, which is then not written.
        self.train_written =
            serde_json::to_string_pretty(&crate::train::TrainFile::default_of_schema())
                .unwrap_or_default();
        let asks: Vec<_> = file
            .asks
            .into_iter()
            .filter(|(t, _)| {
                self.board.ticket(*t).is_some()
                    && self.board.sessions.iter().any(|s| {
                        s.ticket == *t
                            && (mesimon_core::quiet::is_working(s)
                                || (s.kind.is_agent()
                                    && matches!(s.state, SessionState::Unknown { .. })))
                    })
            })
            .collect();
        self.train.restore(asks);
        self.persist_train();
    }

    /// The single write path for `started.json`.
    fn persist_started(&self) {
        if self.started_barred {
            return;
        }
        let _ = crate::started::save(&self.paths, &self.started);
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

    /// The single write path for `queue.json` (T-418): the starts and wakes
    /// of the ask queue, in list order — the board order is re-read at every
    /// drain, never stored. A `Pane` entry is never written (see `queued`).
    /// Called after every mutation of `queued`, so the file is the list.
    fn persist_queue(&self) {
        if self.queue_barred {
            return;
        }
        let entries: Vec<crate::askqueue::QueuedEntry> = self
            .queued
            .iter()
            .filter_map(|q| {
                // A held ask (T-413) is memory-only, like a pane ask: the
                // crown's words never wait in a file for a restart to send,
                // whether a person or the queue would send them (T-550).
                if q.by.is_some() || q.held.is_some() {
                    return None;
                }
                let seat = match q.seat {
                    QueuedSeat::Pane(_) => return None,
                    QueuedSeat::Wake(session) => crate::askqueue::PersistedSeat::Wake { session },
                    QueuedSeat::Start(provider) => {
                        crate::askqueue::PersistedSeat::Start { provider }
                    }
                };
                Some(crate::askqueue::QueuedEntry {
                    ticket: q.ticket,
                    seat,
                    text: q.text.clone(),
                    queued_at: q.queued_at,
                    plan: q.plan,
                })
            })
            .collect();
        let _ = crate::askqueue::save(&self.paths, &entries);
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
            archive_bytes: self.archive_bytes,
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

    /// Every ticket in a `reclaim` column (T-117): the set the sleep and
    /// archive offers price and `Z` acts on.
    fn reclaim_tickets(&self) -> std::collections::HashSet<ulid::Ulid> {
        let cols = self.board.reclaim_columns();
        self.board
            .tickets
            .iter()
            .filter(|t| cols.contains(t.column.as_str()))
            .map(|t| t.id)
            .collect()
    }

    /// The header's sleep suggestion: sessions on sleep-safe tickets that
    /// pass the D23 floors RIGHT NOW (same predicate the sleep keys use — the
    /// suggestion never offers what a keystroke would refuse), plus the RSS
    /// they hold. Which columns are sleep-safe is each column's `reclaim`
    /// setting (T-117).
    fn reclaim_figures(&self) -> (u64, usize) {
        let safe = self.reclaim_tickets();
        let now = now_ms();
        let mut bytes = 0u64;
        let mut n = 0usize;
        // The set `reclaim_all` takes — agents only, a shell's sleep is its
        // close (T-366) — priced exactly, so the offer never sells a shell.
        for rec in
            self.board.sessions.iter().filter(|r| safe.contains(&r.ticket) && r.kind.is_agent())
        {
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
    /// corpse's last change that old — and no summary box left unticked
    /// (T-713: the archive refuses one, so the offer never names it). A
    /// board scan plus one small note read per ticket that passed the rest
    /// (a ticket younger than the threshold costs no read), no forks, on the
    /// 1 s bucket after a board change and once a minute otherwise
    /// (`ARCHIVE_TICKS`) — never the RSS bucket, whose no-pane early-return
    /// fires precisely when archive candidates exist.
    fn archive_figures(&self) -> usize {
        self.archive_candidates().len()
    }

    /// The offer's exact candidate set — ArchiveAll takes THIS, nothing
    /// broader (the Z/ReclaimAll rule).
    fn archive_candidates(&self) -> Vec<ulid::Ulid> {
        let now = now_ms();
        let threshold = archive_suggest_ms();
        let reclaim = self.board.archive_columns();
        self.board
            .tickets
            .iter()
            .filter(|t| !t.is_archived() && reclaim.contains(t.column.as_str()))
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
            // Last, so only a ticket the cheap gates passed costs a read.
            .filter(|t| self.open_boxes(t.id) == 0)
            .map(|t| t.id)
            .collect()
    }

    /// The worktrees the archive offer would tear down: a candidate's tree
    /// passing the archive's own reclaim gate, with the bucket's merge
    /// sample standing in for the fresh check a keypress makes (pricing
    /// forks nothing). Unmerged work keeps its tree and frees nothing.
    fn archive_trees(&self, candidates: &[ulid::Ulid]) -> Vec<std::path::PathBuf> {
        candidates
            .iter()
            .filter_map(|id| {
                let b = self.worktrees.get(id)?;
                let merged = self.wt_agg.get(id).is_some_and(|a| a.merged);
                (worktree::reclaim_on_archive(b, merged, 0, self.worktrees_barred)
                    && b.path.is_dir())
                .then(|| b.path.clone())
            })
            .collect()
    }

    /// The archive offer's disk figure (T-679): what is measured of the
    /// trees it would free, and a walk for any not measured yet. A tree off
    /// the offer drops its size, so one back on it is walked again.
    fn price_archive(&mut self, candidates: &[ulid::Ulid]) -> bool {
        let trees = self.archive_trees(candidates);
        self.tree_sizes.retain(|p, _| trees.contains(p));
        let unsized_: Vec<std::path::PathBuf> =
            trees.iter().filter(|p| !self.tree_sizes.contains_key(*p)).cloned().collect();
        if !unsized_.is_empty() && !self.trees_sizing {
            self.trees_sizing = true;
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let sized = unsized_
                    .into_iter()
                    .map(|p| {
                        let mut bytes = 0;
                        crate::resources::tree_bytes(&p, None, &mut bytes);
                        (p, bytes)
                    })
                    .collect();
                let _ = tx.send(Msg::TreesSized(sized));
            });
        }
        let bytes = trees.iter().filter_map(|p| self.tree_sizes.get(p)).sum();
        std::mem::replace(&mut self.archive_bytes, bytes) != bytes
    }

    fn on_trees_sized(&mut self, sized: Vec<(std::path::PathBuf, u64)>) {
        self.trees_sizing = false;
        self.tree_sizes.extend(sized);
        let candidates = self.archive_candidates();
        if self.price_archive(&candidates) {
            self.broadcast();
        }
    }

    /// X: archive every ticket the offer prices. Per-ticket gate re-checked
    /// (a session can wake between pricing and the keypress); one broadcast.
    /// Each ticket goes down `archive_one`, the road `a a` takes, and gets a
    /// feed row of its own: the chokepoint logs a command that names one
    /// ticket, and this one names none (T-481 — the offer left no row at
    /// all, which is how its skipped reclaim went unseen).
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
            if self.archive_one(id, at.clone(), "local".into()) {
                self.feed.board("local", "archive_all", Some(id));
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
    /// The conversation keys the census leaves out, because the drawer lists
    /// what mesimon does not already own: every live record's, and every one
    /// a record mesimon spawned has ever held (T-441). A dead ADOPTED
    /// record's is not here — that conversation was started outside, and the
    /// drawer is how it is imported again.
    fn excluded_conversations(&self) -> std::collections::HashSet<String> {
        let mut keys = self.started.clone();
        keys.extend(
            self.board
                .sessions
                .iter()
                .filter(|session| session.state.is_live() || session.codex_stopping)
                .filter_map(|session| {
                    crate::agents::adapter(session.kind)?.conversation_key(session)
                }),
        );
        keys
    }

    /// Start the census on a worker (T-437). One walk at a time: a rescan
    /// asked for mid-walk is remembered and runs when this one lands, since
    /// the walk in flight excludes the sessions known when it *started*.
    fn rescan_external(&mut self) {
        if self.external_scanning {
            self.external_rescan_wanted = true;
            return;
        }
        self.external_scanning = true;
        let roots = crate::census::repo_roots(&self.paths.repo_root);
        let known = self.excluded_conversations();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut items = Vec::new();
            for provider in [AgentProvider::ClaudeCode, AgentProvider::Codex] {
                let Some(adapter) = crate::agents::adapter(provider.session_kind()) else {
                    continue;
                };
                items.extend(adapter.discover(&roots, &|identity| known.contains(identity)));
            }
            let _ = tx.send(Msg::ExternalScanned(items));
        });
    }

    /// The census landed. Filtered once more against the sessions the board
    /// holds *now* — an import made during the walk must not re-surface —
    /// then broadcast, so an open drawer redraws on it.
    fn on_external_scanned(&mut self, items: Vec<ExternalItem>) {
        self.external_scanning = false;
        let known = self.excluded_conversations();
        self.external =
            items.into_iter().filter(|item| !known.contains(&item.conversation_id)).collect();
        self.external.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then(a.id.cmp(&b.id)));
        if std::mem::take(&mut self.external_rescan_wanted) {
            self.rescan_external();
        }
        self.broadcast();
    }

    fn broadcast(&mut self) {
        self.board_version = self.board_version.wrapping_add(1);
        self.archive_due = true;
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
        self.team_after_broadcast();
        self.control_changed();
    }

    fn persist_and_notify(&mut self) {
        self.persist_columns();
        self.persist_sessions();
        self.broadcast();
    }

    /// Edit one ticket, save it and broadcast; `no such ticket` when it is
    /// not on the board.
    fn with_ticket(&mut self, id: ulid::Ulid, f: impl FnOnce(&mut Ticket)) -> Response {
        let Some(t) = self.board.ticket_mut(id) else { return no_such_ticket() };
        f(t);
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        self.broadcast();
        Response::Ok
    }

    fn create_ticket(
        &mut self,
        by: &Principal,
        column: String,
        title: String,
        workspace: Option<WorkspaceStrategy>,
        tier: Option<String>,
    ) -> Response {
        match self.mint_full(by, Mint { tier, ..Mint::bare(column, title, workspace) }) {
            Ok(id) => Response::Created { id },
            Err(message) => Response::Err { message },
        }
    }

    /// The composer's mint (T-243): tags, description and pictures ride the
    /// one command, so a refusal — a full group, a note past its limit, a
    /// barred store — leaves no half-made ticket, and a Shift+Enter spawns
    /// onto a card that already carries its brief. Tag names are
    /// registered on the fly, the way `set_tag` does for the picker: using a
    /// name is what puts it in the vocabulary, and a name is registered
    /// whether or not the mint then goes through — the picker's own
    /// register-then-wear order.
    pub(super) fn create_ticket_with_note(
        &mut self,
        stream: &Arc<Mutex<UnixStream>>,
        draft: Draft,
    ) -> Response {
        let Draft { column, title, workspace, text, uploads, tags, tier } = draft;
        let mut refs = Vec::with_capacity(tags.len());
        for tag in tags {
            let Some(name) = sanitize_tag(&tag.name) else {
                return Response::Err { message: "empty tag name".into() };
            };
            if !(1..=10).contains(&tag.group) {
                return Response::Err { message: "tag group must be 1-10".into() };
            }
            refs.push(TagRef { group: tag.group, name });
        }
        if text.trim().is_empty() && !uploads.is_empty() {
            return Response::Err { message: "pictures need a description".into() };
        }
        let images = if text.trim().is_empty() {
            Vec::new()
        } else {
            match self.uploads.prepare(&crate::attachments::Owner::stream(stream), &uploads, &text)
            {
                Ok(images) => images,
                Err(e) => {
                    return Response::Err { message: format!("could not save pictures: {e:#}") }
                }
            }
        };
        let mut registered = false;
        for r in &refs {
            registered |= self.board.register_tag(r.group, &r.name).is_ok();
        }
        if registered {
            self.persist_columns();
        }
        let mint = Mint {
            column,
            title,
            workspace,
            from: None,
            tags: refs,
            note: Some((text, images)),
            tier,
            envelope: None,
        };
        match self.mint_full(&Principal::Local, mint) {
            Ok(id) => {
                self.uploads.committed(&uploads);
                Response::Created { id }
            }
            Err(message) => {
                Response::Err { message: format!("could not create ticket: {message}") }
            }
        }
    }

    /// Owner-delegated local adapter intake. The adapter, not this command,
    /// verifies remote messages. Never forwards remote commands or starts work.
    fn import_ticket(
        &mut self,
        by: &Principal,
        column: String,
        content: mesimon_core::content::TicketContent,
        origin: mesimon_core::content::ImportOrigin,
    ) -> Response {
        use mesimon_core::content::{ImportPlacement, PreparedImport};
        let result = (|| -> Result<(store::imports::Receipt, bool)> {
            if let Decision::Deny { reason } =
                authorize(by, &Action::ImportContent, &Resource::Column { name: column.clone() })
            {
                anyhow::bail!("{reason}");
            }
            content.validate()?;
            // Reconciliation precedes destination checks: a committed receipt
            // remains valid after a column rename or original-ticket deletion.
            if let Some(receipt) = store::imports::replay(&self.paths, &origin, &column, &content)?
            {
                return Ok((receipt, false));
            }
            if self.columns_barred {
                anyhow::bail!("{}", self.barred_message("columns"));
            }
            if self.board.column(&column).is_none() {
                anyhow::bail!("no such column: {column}");
            }
            // Reserve before journaling; failure can leave a harmless gap but
            // never lets another ticket reuse a partially accepted display key.
            self.board.next_key =
                self.board.next_key.checked_add(1).context("ticket keys exhausted")?;
            self.write_columns()?;
            let last = self
                .board
                .column_tickets(&column)
                .last()
                .map(|t| t.order.clone())
                .unwrap_or_default();
            let prepared = PreparedImport::prepare(
                by,
                content,
                origin.clone(),
                ImportPlacement {
                    id: ulid::Ulid::new(),
                    short_key: format!(
                        "{}{}",
                        mesimon_core::board::KEY_PREFIX,
                        self.board.next_key
                    ),
                    column,
                    order: fracindex::between(&last, ""),
                    created_at: now_iso(),
                },
                ulid::Ulid::new,
            )?;
            Ok((store::imports::commit(&self.paths, prepared)?, true))
        })();
        match result {
            Ok((receipt, created)) => {
                if self.board.ticket(receipt.id).is_none() {
                    match store::imports::materialized(&self.paths, &receipt) {
                        Ok(Some(ticket)) if ticket.import_origin.as_ref() == Some(&origin) => {
                            self.board.tickets.push(ticket);
                            self.broadcast();
                        }
                        Ok(None) => {} // Accepted original was subsequently deleted.
                        Ok(Some(_)) => {
                            return Response::Err {
                                message:
                                    "import origin differs from accepted ticket; left untouched"
                                        .into(),
                            }
                        }
                        Err(e) => {
                            return Response::Err {
                                message: format!(
                                    "import accepted but local projection needs recovery: {e}"
                                ),
                            }
                        }
                    }
                }
                Response::Imported { id: receipt.id, key: receipt.key, created }
            }
            Err(e) => Response::Err { message: format!("could not import ticket: {e}") },
        }
    }

    /// Copy content only. Publish after all note bodies and metadata are saved;
    /// duplication never provisions a workspace or starts an agent.
    fn duplicate_ticket(&mut self, by: &Principal, id: ulid::Ulid) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let Some(source) = self.board.ticket(id).cloned() else { return no_such_ticket() };
        if source.archived.is_some() {
            return Response::Err { message: "cannot duplicate an archived ticket".into() };
        }
        let mut bodies = Vec::with_capacity(source.notes.len());
        for note in &source.notes {
            match store::read_note(&self.paths, &source.short_key, note.id) {
                Ok(body) => bodies.push(body),
                Err(e) => {
                    return Response::Err { message: format!("could not copy the note: {e}") };
                }
            }
        }
        let attachments = match crate::attachments::collect(&self.paths, &source.short_key) {
            Ok(images) => images,
            Err(e) => return Response::Err { message: format!("could not copy pictures: {e:#}") },
        };
        let column = self.board.column_tickets(&source.column);
        let next = column.iter().position(|t| t.id == id).and_then(|i| column.get(i + 1));
        let order = fracindex::between(&source.order, next.map_or("", |t| t.order.as_str()));
        // Reserve the key durably before any ticket files. A failed copy may
        // leave a gap, but a restart must never reuse a partially written key.
        self.board.next_key += 1;
        if let Err(e) = self.write_columns() {
            return Response::Err { message: format!("could not reserve a ticket key: {e}") };
        }
        let mut ticket = Ticket {
            id: ulid::Ulid::new(),
            short_key: format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
            title: source.title,
            column: source.column,
            order,
            created_at: now_iso(),
            created_by: by.note_author(),
            created_from: None,
            entered_at: Some(now_iso()),
            previous_column: None,
            picked: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: source.execution_policy,
            tier: source.tier,
            import_origin: source.import_origin,
            envelope: None,
            raised: None,
            workspace: source.workspace,
            tags: source.tags,
            notes: source.notes,
            archived: None,
        };
        // Preserve note authorship and order, but give each copy its own identity.
        for note in &mut ticket.notes {
            note.id = ulid::Ulid::new();
        }
        let saved = (|| -> Result<()> {
            for (meta, bytes) in &attachments {
                crate::attachments::save(&self.paths, &ticket.short_key, meta, bytes)?;
            }
            for (note, body) in ticket.notes.iter().zip(bodies) {
                store::save_note(&self.paths, &ticket.short_key, note.id, &body)?;
            }
            store::save_ticket(&self.paths, &ticket)
        })();
        if let Err(e) = saved {
            let cleanup = store::delete_ticket_dir(&self.paths, &ticket.short_key);
            let detail = cleanup
                .err()
                .map(|e| format!("; could not clean up copy: {e}"))
                .unwrap_or_default();
            return Response::Err {
                message: format!("could not duplicate the ticket: {e}{detail}"),
            };
        }
        let id = ticket.id;
        self.board.tickets.push(ticket);
        self.broadcast();
        Response::Created { id }
    }

    /// Append a new ticket to `column` (caller validated the column).
    /// `from` is the ticket the caller was bound to when it asked — an agent's
    /// `create_ticket` — and `None` for a person, who is bound to nothing.
    /// `workspace` is the composer's explicit choice; absent, the column's
    /// own default is stamped onto the ticket (T-117) — the ticket field
    /// stays the truth, so a later change to the column is never
    /// retroactive.
    /// The one place a `Ticket` is built (T-243). Every road that mints —
    /// the thin `CreateTicket`, the composer's `CreateTicketWithNote`, the
    /// agent's `create_ticket`, an adopted external session — passes a
    /// `Mint` through here, so a ticket exists with its title, workspace,
    /// tags, description and pictures, or not at all. Every refusal comes
    /// before anything touches disk; a failed write deletes the ticket's
    /// directory and the board never sees it. Tags arrive as registry
    /// references already spelled by the caller (the human road registers
    /// them on the fly, the agent road resolves them and never registers);
    /// the wearer rule — one tag per group — is judged here, where the
    /// ticket is, instead of mirrored by a client whose picks are on no
    /// ticket yet. Persists and notifies; the caller adds its own feed
    /// line or upload commit.
    fn mint_full(&mut self, by: &Principal, mint: Mint) -> Result<ulid::Ulid, String> {
        let Mint { column, title, workspace, from, tags, note, tier, envelope } = mint;
        // A barred columns.toml means next_key cannot be persisted, so a new
        // ticket's short_key would regress on the next start and save_ticket
        // would write over an existing ticket directory.
        if self.columns_barred {
            return Err(self.barred_message("columns"));
        }
        if self.board.column(&column).is_none() {
            return Err(format!("no such column: {column}"));
        }
        // A title is user text on a card row; scrubbed and bounded here, at
        // the boundary — the composer's own cap is a courtesy a client can lift.
        let title = mesimon_core::board::sanitize_title(&title);
        if title.trim().is_empty() {
            return Err("a ticket needs a title".into());
        }
        for (i, tag) in tags.iter().enumerate() {
            if !(1..=10).contains(&tag.group) {
                return Err("tag group must be 1-10".into());
            }
            if let Some(other) = tags[..i].iter().find(|o| o.group == tag.group) {
                return Err(format!(
                    "one tag per group: {} and {} are both on group {}",
                    other.name, tag.name, tag.group
                ));
            }
        }
        // The composer's tier pick (T-443): a tier this machine resolves,
        // stored as inherit when it is the default. A fresh ticket holds no
        // seat, so any provider's tier is a pick it may make.
        let tier = match tier.as_deref() {
            None => None,
            Some(t) => match self.tier_book().get(t) {
                Some(found) => self.tier_book().stored_pick(&found.id),
                None => return Err(format!("no tier {t} on this machine")),
            },
        };
        let note = match note {
            Some((text, images)) if !text.trim().is_empty() => {
                if let Some(message) = mesimon_core::board::note_size_error(&text) {
                    return Err(message);
                }
                if mesimon_core::board::sanitize_note(&text) != text {
                    return Err("invalid note text".into());
                }
                Some((text, images))
            }
            _ => None,
        };
        // Reserve the key on disk before the ticket exists, so a crash between
        // the two cannot hand the same key to the next mint.
        self.board.next_key += 1;
        if let Err(e) = self.write_columns() {
            return Err(format!("could not reserve a ticket key: {e:#}"));
        }
        let workspace =
            workspace.or_else(|| self.board.column(&column).and_then(|c| c.settings.workspace));
        let last =
            self.board.column_tickets(&column).last().map(|t| t.order.clone()).unwrap_or_default();
        let now = now_iso();
        let author = by.note_author();
        let mut ticket = Ticket {
            id: ulid::Ulid::new(),
            short_key: format!("{}{}", mesimon_core::board::KEY_PREFIX, self.board.next_key),
            title,
            column,
            order: fracindex::between(&last, ""),
            created_at: now.clone(),
            created_by: author.clone(),
            created_from: from,
            entered_at: Some(now.clone()),
            previous_column: None,
            picked: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            tier,
            import_origin: None,
            envelope,
            raised: None,
            workspace,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        };
        for tag in tags {
            ticket.set_tag(tag.group, Some(tag.name));
        }
        if let Some((text, _)) = &note {
            ticket.notes.push(mesimon_core::board::NoteMeta {
                id: ulid::Ulid::new(),
                name: mesimon_core::board::note_name(text),
                rev: 1,
                created_at: now.clone(),
                edited_at: now,
                created_by: author.clone(),
                edited_by: author,
            });
        }
        let saved = (|| -> Result<()> {
            if let Some((text, images)) = &note {
                for (meta, bytes) in images {
                    crate::attachments::save(&self.paths, &ticket.short_key, meta, bytes)?;
                }
                store::save_note(&self.paths, &ticket.short_key, ticket.notes[0].id, text)?;
            }
            store::save_ticket(&self.paths, &ticket)
        })();
        if let Err(error) = saved {
            let cleanup = store::delete_ticket_dir(&self.paths, &ticket.short_key);
            return Err(format!("ticket save failed: {error:#}; cleanup: {cleanup:?}"));
        }
        let id = ticket.id;
        self.board.tickets.push(ticket);
        self.persist_and_notify();
        Ok(id)
    }

    fn delete_ticket(&mut self, id: ulid::Ulid, discard_worktree: bool) -> Response {
        let Some(pos) = self.board.tickets.iter().position(|t| t.id == id) else {
            return no_such_ticket();
        };
        let attachments =
            match crate::attachments::collect(&self.paths, &self.board.tickets[pos].short_key) {
                Ok(images) => images,
                Err(e) => {
                    return Response::Err {
                        message: format!("could not preserve pictures for undo: {e:#}"),
                    }
                }
            };
        // M4 delete gate (defense in depth — the TUI prompts first): an
        // unmerged worktree must be merged or explicitly discarded.
        if !discard_worktree {
            if let Some(b) = self.worktrees.get(&id) {
                if !b.branch.is_empty() && !self.ticket_merged(id) {
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
        self.persist_train();
        self.forget_queued(id, "queued_ask_dropped", "local", "board");
        self.owed.retain(|_, o| o.ticket != id);
        self.drop_crown_if(id);
        // Sessions detach and keep running through the grace band (D21).
        let sessions: Vec<SessionRecord> =
            self.board.sessions.iter().filter(|s| s.ticket == id).cloned().collect();
        // Keep owned Codex cleanup evidence durable until its separate server
        // is stopped, including through a crash during the undo window.
        self.board.sessions.retain(|s| s.ticket != id || s.owns_codex_runtime());
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
                attachments,
            },
        );
        self.persist_and_notify();
        Response::Ok
    }

    /// The ticket's branch has landed in EVERY leg it was cut in (T-368):
    /// the DONE gate's, the delete gate's and the archive's one oracle, and
    /// it must answer as the card does.
    fn ticket_merged(&self, id: ulid::Ulid) -> bool {
        let Some(b) = self.worktrees.get(&id) else { return false };
        if b.branch.is_empty() {
            return false;
        }
        let single_base = self
            .base_branch
            .clone()
            .or_else(|| worktree::default_branch(&self.paths.repo_root).ok())
            .unwrap_or_default();
        let legs = b.legs(&self.paths.repo_root, &single_base);
        legs.iter().all(|leg| {
            // Fresh check on gate paths (the 10 s cache may lag a just-made
            // merge).
            if !leg.base.is_empty() && worktree::is_merged(&leg.repo, &b.branch, &leg.base) {
                return true;
            }
            // A PR squashed into the base is the SAMPLE's answer (T-267):
            // the patch scan behind it is too big for a keypress. An ff
            // merge made here is caught fresh above; a squash made here is a
            // bucket's wait, never a wrong answer. The branch tip is re-read
            // so a commit since the verdict drops it, and the target can
            // only ever gain commits, so a stale tip there cannot turn a
            // merge back into a non-merge. The gates and the card must
            // answer alike: a ticket the board calls merged must not then be
            // refused DONE.
            self.wt_repos
                .get(&id)
                .and_then(|legs| legs.iter().find(|l| l.name == leg.name))
                .and_then(|l| l.content.as_ref())
                .is_some_and(|seen| {
                    seen.merged
                        && !seen.branch_tip.is_empty()
                        && seen.branch_tip == worktree::branch_tip(&leg.repo, &b.branch)
                })
        })
    }

    /// The binding's legs, with the daemon's base branch for a single-repo
    /// one (a workspace leg carries its own).
    fn legs_of(&self, id: ulid::Ulid) -> Vec<worktree::Leg> {
        let base = self.base_branch.clone().unwrap_or_default();
        self.worktrees.get(&id).map(|b| b.legs(&self.paths.repo_root, &base)).unwrap_or_default()
    }

    /// The ticket's base tip as of the last sample (`""` before one).
    fn base_tip_of(&self, id: ulid::Ulid) -> &str {
        self.wt_agg.get(&id).map_or("", |a| a.base_tip.as_str())
    }

    /// The ref a merged PR lands on (`origin/main`) for one leg, asked once
    /// and kept — the value is whether there is one, so a repo with no
    /// remote is not re-asked every bucket. Forgotten with `base_branch`
    /// after a fetch — the one thing that can mint it.
    fn upstream_ref(&mut self, name: &str, repo: &std::path::Path, base: &str) -> Option<String> {
        if !self.upstreams.contains_key(name) {
            self.upstreams.insert(name.to_string(), worktree::upstream_base(repo, base));
        }
        self.upstreams.get(name).cloned().flatten()
    }

    /// `get_ticket`'s per-repo rows on a workspace ticket (T-368); empty on
    /// a single repo.
    fn agent_repo_views(&self, id: ulid::Ulid) -> Vec<AgentRepoView> {
        let Some(b) = self.worktrees.get(&id) else { return Vec::new() };
        if !b.is_workspace() || b.branch.is_empty() {
            return Vec::new();
        }
        let Some(legs) = self.wt_repos.get(&id) else {
            return b
                .repos
                .iter()
                .map(|r| AgentRepoView {
                    name: r.name.clone(),
                    base: r.base.clone(),
                    merge_state: "clean".into(),
                })
                .collect();
        };
        legs.iter()
            .map(|l| AgentRepoView {
                name: l.name.clone(),
                base: l.base.clone(),
                merge_state: worktree::merge_word(l.merged, l.needs_rebase, l.ahead).into(),
            })
            .collect()
    }

    fn set_workspace(&mut self, id: ulid::Ulid, workspace: Option<WorkspaceStrategy>) -> Response {
        if self.board.ticket(id).is_none() {
            return no_such_ticket();
        }
        // Refused here while the bindings file is barred (T-683), so the
        // desk, the crown's `set_workspace`, its `start_agent` and the phone
        // all answer alike — a choice made now would steer the next spawn
        // at a tree nothing could record.
        if self.worktrees_barred {
            return Response::Err { message: self.barred_message("worktrees") };
        }
        // Locked once anything exists that the choice would RELOCATE — a
        // worktree, and an agent standing in a directory this field names.
        //
        // It was any session record at all until T-309 (2026-09-07), and on a
        // board where most tickets have talked to an agent once that is a
        // lock nothing can open: 17 of the author's 44 live tickets were held
        // by it with not one of them provisioned, which is the case the
        // ticket asked for by name ("unless provisioned already"). A parked
        // or finished record relocates nothing — `resume_session` replays the
        // record's OWN cwd and re-resolves only when that directory is gone
        // (T-278) — so the field governs the next spawn, which is what it is
        // for. `has_pane` and not `is_live` is exactly that line: `Sleeping`
        // is a conversation, not a checkout.
        if self.board.sessions.iter().any(|s| s.ticket == id && s.state.has_pane()) {
            return Response::Err {
                message: "workspace locked — an agent is running on this ticket".into(),
            };
        }
        if self.worktrees.contains_key(&id) {
            return Response::Err { message: "workspace locked — worktree exists".into() };
        }
        self.with_ticket(id, |t| t.workspace = workspace)
    }

    /// Set or clear the ticket's tag on one axis. Sanitization happens HERE,
    /// at the boundary, not in the TUI: a tag name is user text that lands on
    /// a card row, and a width hazard there strands cells the diff never
    /// repaints. Not gated on any write bar — `save_ticket` is the deliberate
    /// exception (a ticket file we could not read is absent from
    /// `board.tickets` and so self-bars).
    fn set_tag(&mut self, id: ulid::Ulid, group: u8, name: Option<String>) -> Response {
        if self.board.ticket(id).is_none() {
            return no_such_ticket();
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
        self.with_ticket(id, |t| t.set_tag(group, clean))
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
                notified: false,
            };
        }
        if let Some(ticket) = self.board.ticket(id) {
            if mesimon_core::authorize::authorize_execution(by, ticket.effective_execution_policy())
                .denied()
            {
                return Response::Merge {
                    outcome: MergeOutcome::Refused,
                    detail: "this ticket requires a human to merge it".into(),
                    notified: false,
                };
            }
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
                notified: false,
            };
        };
        if b.branch.is_empty() {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "worktree has no branch yet".into(),
                notified: false,
            };
        }
        let branch = b.branch.clone();
        // Quiet-tickets rule (author): never merge under a working agent.
        // A CLAUDE — a shell is pinned `Running` for the life of its pane
        // (D15), and an ff-merge never touches the worktree it sits in
        // (2026-09-04; it refused every `m` under a `!` shell before).
        let busy = self
            .board
            .sessions
            .iter()
            .any(|s| s.ticket == id && mesimon_core::quiet::is_working(s));
        if busy {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "sessions still working — wait for them to finish".into(),
                notified: false,
            };
        }
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        let Some(base) = self.base_branch.clone() else {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no default branch found".into(),
                notified: false,
            };
        };
        let legs = self.legs_of(id);
        let workspace = self.worktrees.get(&id).is_some_and(|b| b.is_workspace());
        // A branch whose tip never left the creation base is trivially an
        // ancestor of main — merge_check would call it "already merged".
        // Truth: there is nothing to merge yet (dogfood 2026-08-30).
        let touched = legs.iter().any(|leg| {
            let tip = worktree::branch_tip(&leg.repo, &branch);
            !tip.is_empty() && tip != leg.base_oid
        });
        if !touched {
            return Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no commits on the branch yet — nothing to merge".into(),
                notified: false,
            };
        }
        // The gate's own oracle, so `m` never offers to merge — or to rebase
        // — a branch the card already calls merged: a PR squashed into
        // `origin/main` is merged without being an ancestor of anything
        // (T-267), and the ff check below would answer "main moved" there.
        if self.ticket_merged(id) {
            let landed = self
                .wt_agg
                .get(&id)
                .map(|a| &a.merged_in)
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| base.clone());
            return Response::Merge {
                outcome: MergeOutcome::AlreadyMerged,
                detail: format!("{branch} is already in {landed}"),
                notified: false,
            };
        }
        // ff-only policy, leg by leg (T-368): every touched leg is judged
        // before any is moved — a base moved past the branch anywhere is the
        // agent's rebase + tests in its worktree first, and mesimon never
        // mints merge commits. Then each leg fast-forwards into its own
        // base, in order; a refusal midway leaves the ticket half landed and
        // says so, and the next `m` continues from there.
        match worktree::merge_legs(&legs, &branch) {
            worktree::LegMerge::Nothing => Response::Merge {
                outcome: MergeOutcome::Refused,
                detail: "no commits on the branch yet — nothing to merge".into(),
                notified: false,
            },
            worktree::LegMerge::Already => Response::Merge {
                outcome: MergeOutcome::AlreadyMerged,
                detail: format!("{branch} is already in {base}"),
                notified: false,
            },
            worktree::LegMerge::NeedsRebase(moved) => Response::Merge {
                outcome: MergeOutcome::NeedsRebase,
                detail: if workspace {
                    let names: Vec<&str> =
                        moved.iter().map(|(n, _)| worktree::leg_word(n)).collect();
                    format!("{} moved — rebase first", names.join(", "))
                } else {
                    format!("{base} moved — rebase first")
                },
                notified: false,
            },
            worktree::LegMerge::Merged(landed) => {
                self.refresh_worktree_flags();
                // The merge just moved the checkout's branch: the header's
                // `↑` should say so before the next 10 s bucket.
                self.queue_git_sample();
                self.persist_and_notify();
                Response::Merge {
                    outcome: MergeOutcome::Merged,
                    detail: if workspace {
                        format!("{branch} merged into {}", worktree::landed_words(&landed))
                    } else {
                        format!("{branch} merged into {base}")
                    },
                    notified: false,
                }
            }
            worktree::LegMerge::Refused { landed, leg, base: leg_base, error } => {
                if !landed.is_empty() {
                    // Some legs did land: the card must say so now.
                    self.refresh_worktree_flags();
                    self.queue_git_sample();
                    self.persist_and_notify();
                }
                let detail = worktree::merge_refusal_detail(&error, &leg_base);
                Response::Merge {
                    outcome: MergeOutcome::Refused,
                    detail: if workspace {
                        format!("{}: {detail}", worktree::leg_word(&leg))
                    } else {
                        detail
                    },
                    notified: false,
                }
            }
        }
    }

    /// The m flow's inject stages: paste a rebase request or the merged
    /// notice into the ticket's live claude session and submit it (T-5
    /// delivery). Explicit user gesture — the user pressed through the
    /// staged prompt. `ack` is the feed line the paste earns when the agent
    /// takes it: the train names its own, a person's is the plain prompt.
    fn merge_to_agent(
        &mut self,
        id: ulid::Ulid,
        request: mesimon_core::command::MergeRequest,
        by: &Principal,
        ack: Ack,
    ) -> Response {
        if let Decision::Deny { .. } = authorize(by, &Action::Mutate, &Resource::Ticket { id }) {
            return Response::Err { message: "not allowed".into() };
        }
        if self.board.ticket(id).is_some_and(|ticket| {
            mesimon_core::authorize::authorize_execution(by, ticket.effective_execution_policy())
                .denied()
        }) {
            return Response::Err {
                message: "this ticket requires a human to request a merge or rebase".into(),
            };
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
        let mut base = self.base_branch.clone().unwrap_or_else(|| "main".into());
        // On a workspace ticket (T-368) the base is per leg: the sentence
        // names the legs that moved, `main (api), main (web)`, or every
        // touched leg where the sample has not said which yet.
        if b.is_workspace() {
            let legs = self.wt_repos.get(&id).cloned().unwrap_or_default();
            let mut named: Vec<String> = legs
                .iter()
                .filter(|l| l.needs_rebase && l.ahead > 0)
                .map(|l| format!("{} ({})", l.base, worktree::leg_word(&l.name)))
                .collect();
            if named.is_empty() {
                named = legs
                    .iter()
                    .filter(|l| l.touched())
                    .map(|l| format!("{} ({})", l.base, worktree::leg_word(&l.name)))
                    .collect();
            }
            if named.is_empty() {
                named = b
                    .repos
                    .iter()
                    .map(|r| format!("{} ({})", r.base, worktree::leg_word(&r.name)))
                    .collect();
            }
            base = named.join(", ");
        }
        // The board's template, or mesimon's own words where nobody wrote one
        // (T-353). The two facts the sentence is about — which branch, which
        // base — are the only things substituted into it.
        let text =
            self.board.prompts.render(request.prompt(), &[("branch", &branch), ("base", &base)]);
        if let Err(message) = self.paste_to_ticket(id, &text, ack) {
            return Response::Err { message };
        }
        // The turn that takes these words is a merge step: the crown is not
        // woken for it (T-469).
        self.tag_owed(id, TurnAsk::Merge);
        // A delivered rebase ask is remembered against the base tip, by hand
        // or by train: the train does not ask again until the base moves
        // (2026-09-04).
        if matches!(request, mesimon_core::command::MergeRequest::Rebase) {
            self.train.record_ask(
                id,
                self.base_tip_of(id).to_string(),
                now_ms(),
                by.is_human(),
                Instant::now(),
            );
            self.persist_train();
        }
        Response::Ok
    }

    /// Words into the ticket's paned claude — `pane_target`, the one session
    /// `board_enter` focuses — by bracketed paste, then a SEPARATE Enter (a
    /// CR in the same byte burst is absorbed as pasted content, T-5). The
    /// merge flow, a note's nudge and the board's ask all deliver through
    /// here; what differs between them is whose words travel, and `ack` is
    /// the feed line the paste earns when the agent takes it. This is the
    /// one place a paste of ours is entered in `owed` (T-244): a Claude
    /// pane's went in with its Enter and waits `INFLIGHT_MS` for the ack; a
    /// Codex pane's is parked for `drive_codex_inputs`, which pastes on its
    /// own clock, and the record's hold is its clock.
    fn paste_to_ticket(&mut self, ticket: ulid::Ulid, text: &str, ack: Ack) -> Result<(), String> {
        self.paste_at(ticket, text, ack, false)
    }

    /// `paste_to_ticket`, or with `immediately` (T-601) Claude Code's
    /// send-now over the words: a WORKING Claude agent reads them at once,
    /// its running tool call moved to the background, where a plain send
    /// waits for the turn's next step (a paste) or its end (a mod's
    /// `submit`). The words must stand in the composer for the key to send
    /// them: the mod fills the empty box where it speaks `fill`
    /// (`mod_fill`), and the paste road pastes; the keys go once they are
    /// in. An idle agent takes the plain send, which starts its turn at
    /// once anyway. Codex has no such key and takes the plain paste.
    fn paste_at(
        &mut self,
        ticket: ulid::Ulid,
        text: &str,
        ack: Ack,
        immediately: bool,
    ) -> Result<(), String> {
        let Some(rec) = self.board.pane_target(ticket) else {
            return Err("no live agent session on this ticket — start or wake one first".into());
        };
        if rec.observe_only() {
            return Err("external session — resume it to take over before sending a prompt".into());
        }
        let sid = rec.sid16();
        let id = rec.id;
        if rec.kind == SessionKind::Codex {
            if rec.pending_submit || self.parked(id) {
                return Err("a prompt is already waiting for this session".into());
            }
            self.park(
                id,
                ticket,
                Parked { text: text.to_string(), brief: false, title: false },
                ack,
            );
            if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                rec.observation_hold = true;
            }
            return Ok(());
        }
        let immediately = immediately && !self.session_idle(id);
        // Immediately on the mod road (T-601): the words into the empty
        // composer by the mod's `fill`, then the keys, on its `filled`.
        if immediately && self.mod_speaks(id, "fill") && !road::is_command(text) {
            let words = Parked { text: text.to_string(), brief: false, title: false };
            if !self.mod_fill(id, ticket, words, ack) {
                return Err("nothing to send".into());
            }
            return Ok(());
        }
        // A Claude session whose mod is up takes the words by its `submit`
        // (T-575): a turn of its own, held behind a running one, never typed.
        // A slash command is for the box, and is typed into it.
        if !immediately && self.mod_speaks(id, "submit") && !road::is_command(text) {
            let words = Parked { text: text.to_string(), brief: false, title: false };
            if !self.mod_submit(id, ticket, words, ack) {
                return Err("nothing to send".into());
            }
            return Ok(());
        }
        if immediately {
            // The paste, then the send-now in its own call, as an Enter
            // goes in its own (`paste_text`): keys in the paste's burst are
            // read as pasted text.
            self.backend
                .paste_input(&sid, text)
                .and_then(|()| self.backend.send_now(&sid))
                .map_err(|e| format!("could not deliver: {e}"))?;
            self.feed.board("daemon", "prompt_sent_immediately", Some(ticket));
        } else {
            self.backend.paste_text(&sid, text).map_err(|e| format!("could not deliver: {e}"))?;
        }
        self.owed.insert(id, Owed::pasted(ticket, ack, now_ms()));
        Ok(())
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
        self.clients.remove(&conn_key(stream));
        // What this board asked to be read goes with it (T-327).
        self.usage.drop_conn(conn_key(stream));
        // The board that was inside the pane is gone, however it went: give
        // the focus token back. Nothing else ever would — `FocusEnd` comes
        // after a handover this board will not return from.
        if self
            .focus
            .as_ref()
            .is_some_and(|h| h.by.upgrade().is_some_and(|w| Arc::ptr_eq(&w, stream)))
        {
            self.focus = None;
        }
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
                        ahead: self.wt_agg.get(t).map_or(0, |a| a.ahead),
                        merged: self.wt_agg.get(t).is_some_and(|a| a.merged),
                        needs_rebase: self.wt_agg.get(t).is_some_and(|a| a.needs_rebase),
                        conflict: self.wt_conflicts.contains(&b.branch),
                    },
                )
            })
            .collect()
    }

    /// What the train would do, over the cached flags — no git on this road.
    fn train_plan(&self) -> mesimon_core::train::Plan {
        let flags = self.train_flags();
        let base_tip: HashMap<ulid::Ulid, String> =
            self.wt_agg.iter().map(|(t, a)| (*t, a.base_tip.clone())).collect();
        let asked = self.train.asked_tips();
        let fused: std::collections::HashSet<ulid::Ulid> =
            self.train.fused_tickets().copied().collect();
        mesimon_core::train::plan(&mesimon_core::train::Input {
            board: &self.board,
            flags: &flags,
            base_tip: &base_tip,
            asked: &asked,
            fused: &fused,
        })
    }

    /// One pass of the train (2026-09-04), after `refresh_worktree_flags`
    /// on its bucket: ONE action, only while `train_busy` is empty — the
    /// root checkout's own workers and anything mid-rebase, NOT the whole
    /// board since T-351. A
    /// merge first — the first REVIEW candidate in board order, through the
    /// same road a hand `m` takes under `Principal::Automation`, then the
    /// merged notice into its agent if that is on (a turn starts; the next
    /// pass waits for it). Else ONE rebase ask to an idle agent selected by
    /// the planner. Terminal output is not work: idle animations must not
    /// hold the train behind a pane-silence check. A refused merge is
    /// remembered per tip pair so a dirty main is not retried every bucket.
    /// Pending merges hold further rebase asks until they land or leave the
    /// train; the next rebase should include the commits they will add.
    fn train_pass(&mut self) -> bool {
        if !self.train.is_armed()
            || self.worktrees_barred
            || self.worktrees.is_empty()
            || self.base_branch.is_none()
        {
            return false;
        }
        if !self.train_busy().is_empty() {
            return false;
        }
        let plan = self.train_plan();
        let by = Principal::Automation { rule: mesimon_core::train::RULE.into() };
        let mut refused = false;
        for t in plan.merge.iter().copied() {
            // The tip the flags were sampled at, like `base_tip` beside it:
            // the refusal memory is keyed on the pair, and the snapshot road
            // (`pending_items`) reads the same map, so neither forks git.
            let tip = self.wt_agg.get(&t).map(|a| a.tip.clone()).unwrap_or_default();
            if self.train.refusal(t, &tip, self.base_tip_of(t)).is_some() {
                continue;
            }
            match self.merge_ticket(t, &by) {
                Response::Merge { outcome: MergeOutcome::Merged, .. } => {
                    self.feed.board("automation", "merge_train_merged", Some(t));
                    if self.train.notice() {
                        if self.board.pane_target(t).is_some() {
                            let req = mesimon_core::command::MergeRequest::MergedNotice;
                            let ack = Ack { by: "automation", word: "merge_train_notice_landed" };
                            match self.merge_to_agent(t, req, &by, ack) {
                                Response::Ok => {
                                    self.feed.board("automation", "merge_train_notified", Some(t));
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
                Response::Merge { outcome: MergeOutcome::Refused, detail, .. } => {
                    let base_tip = self.base_tip_of(t).to_string();
                    self.train.refuse(t, tip, base_tip, detail);
                    self.feed.board("automation", "merge_train_refused:merge", Some(t));
                    refused = true;
                }
                // NeedsRebase / AlreadyMerged: the cached flags lagged git;
                // the refresh that ran before this pass will not next time.
                _ => return true,
            }
        }
        if !plan.merge.is_empty() {
            // Includes remembered refusals. Keep trying other merge candidates,
            // but do not rebase more branches onto a base we cannot yet advance.
            // A new refusal must be broadcast even though no merge landed.
            return refused;
        }
        let Some(t) = plan.rebase.first().copied() else { return false };
        let was_fused = self.train.is_fused(t);
        let ack = Ack { by: "automation", word: "merge_train_rebase_landed" };
        match self.merge_to_agent(t, mesimon_core::command::MergeRequest::Rebase, &by, ack) {
            Response::Ok => {
                self.feed.board("automation", "merge_train_rebase_asked", Some(t));
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
            return no_such_ticket();
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
        rev: Option<u64>,
        by: &Principal,
    ) -> Response {
        use mesimon_core::board::{note_name, note_size_error, sanitize_note, NoteMeta};
        // Too long is refused before anything is touched (T-328): the
        // existing note stays whole and the receipt says why. The cap inside
        // `sanitize_note` never fires past this line; it is the floor.
        if let Some(message) = note_size_error(&text) {
            return Response::Err { message };
        }
        let text = sanitize_note(&text);
        let blank = text.trim().is_empty();
        let Some(t) = self.board.ticket(ticket) else {
            return no_such_ticket();
        };
        if let Some(id) = note {
            let Some(meta) = t.note(id) else {
                return Response::Err { message: "no such note".into() };
            };
            // The revision the writer read is not the one on disk (T-696):
            // someone wrote in between, and a whole-file write would take
            // their words away. The phone road's `note_gate` is this rule.
            if rev.is_some_and(|r| r != meta.rev) {
                return Response::Err {
                    message: "the note changed since you read it ∙ reopen".into(),
                };
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
            return no_such_ticket();
        };
        let Some(meta) = t.note(note) else {
            return Response::Err { message: "no such note".into() };
        };
        let name = mesimon_core::text::scrub_text(&meta.name);
        let note_id = note.to_string();
        let text = self.board.prompts.render(
            mesimon_core::prompts::AgentPrompt::NoteUpdated,
            &[("note", &name), ("id", &note_id)],
        );
        match self.paste_to_ticket(ticket, &text, Ack::PROMPT) {
            Ok(()) => Response::Ok,
            Err(message) => Response::Err { message },
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
    #[allow(clippy::too_many_arguments)]
    fn prompt_session(
        &mut self,
        ticket: ulid::Ulid,
        text: String,
        queued: bool,
        immediately: bool,
        accept_plan: bool,
        plan: bool,
        tier: Option<String>,
    ) -> Response {
        if self.board.live_agent(ticket).is_some_and(|rec| rec.observe_only()) {
            return Response::Err {
                message: "external session — resume it to take over before sending a prompt".into(),
            };
        }
        // The ask field's tier pick (T-443) lands on the ticket before the
        // seat is read: an empty seat starts on it, a parked one wakes on
        // it, and a pane owes the switch these words then ride.
        if let Some(pick) = tier {
            if let Err(message) = self.apply_ticket_tier(ticket, Some(pick)) {
                return Response::Err { message };
            }
            self.persist_sessions();
        }
        let seat = self.seat_of(ticket);
        // A pane that owes a switch takes its words through the relaunch,
        // and the relaunch waits for idle: sent now at a working pane, the
        // words queue behind the turn instead of landing on the old tier.
        let queued = queued
            || matches!(seat, QueuedSeat::Pane(id) if self.tier_owed(id) && !self.session_idle(id));
        // Blank in, nothing out: an empty paste would press Enter on a turn
        // the user never wrote. An EMPTY SEAT is the one exception (T-294):
        // there the Enter lands on the ticket title the spawn types, which
        // is a turn the user did write — it is the composed start, asked for
        // through the same field.
        // An `accept plan` (T-420) is about a dialog, and only a pane shows
        // one: off a pane the flag is nothing. On a pane it is always a
        // QUEUED entry, even with the dialog up right now — the dialog may
        // not have painted yet when the hook lands the `≡` (PreToolUse
        // fires before it renders), so the press is the clock's
        // (`service_plan_accepts`), one tick away, and the card says
        // `accepting plan` until the harness confirms.
        let accept_plan = accept_plan && matches!(seat, QueuedSeat::Pane(_));
        // Plan mode (T-434) is about the NEXT turn's launch; an accept is
        // about a plan that already exists, so beside it the flag is
        // nothing. Claude only — a launch flag Codex does not have — and on
        // a live pane only an IDLE one: there is no keystroke that sets the
        // mode absolutely, so the pane is parked and woken with the flag,
        // which `sleep_one` refuses mid-turn. Sent now, that refusal is the
        // person's answer; queued, the drain waits for idle as it always
        // has and the relaunch is the delivery.
        let plan = plan && !accept_plan;
        if let Some(message) = self.plan_refusal(ticket, plan) {
            return Response::Err { message };
        }
        if plan && !queued {
            if let QueuedSeat::Pane(id) = seat {
                if !self.session_idle(id) {
                    return Response::Err {
                        message: format!(
                            "{} is mid-turn ∙ plan mode restarts it, so queue the ask for its idle",
                            mesimon_core::keymap::AGENT_WORD
                        ),
                    };
                }
            }
        }
        let text = match (mesimon_core::command::sanitize_prompt(&text), &seat) {
            (Some(text), _) => text,
            (None, QueuedSeat::Start(_)) => String::new(),
            // "Accept the plan, ask nothing": the entry carries the flag
            // and no words, and goes the moment the press is in.
            (None, QueuedSeat::Pane(_)) if accept_plan => String::new(),
            // A seat that never took a prompt (T-603): blank is its brief.
            (None, QueuedSeat::Pane(id) | QueuedSeat::Wake(id)) if self.unprompted(*id) => {
                self.forget_queued(ticket, "queued_ask_dropped", "local", "person");
                return self.send_brief(ticket, seat, plan);
            }
            (None, _) => return Response::Err { message: "nothing to send".into() },
        };
        if queued || accept_plan {
            return self.enqueue_ask(ticket, seat, text, accept_plan, plan);
        }
        // Immediately (T-601) is Claude Code's send-now: refused at an agent
        // that has none, and at a dialog, whose keys it would press.
        if immediately {
            if let Some(why) = self.immediate_refusal(ticket) {
                return Response::Err {
                    message: format!("{} {why}", mesimon_core::keymap::AGENT_WORD),
                };
            }
            if let QueuedSeat::Pane(id) = seat {
                if self.pane_waits_on_you(id) {
                    return Response::Err {
                        message: mesimon_core::command::ANSWER_IN_PANE_FIRST.into(),
                    };
                }
            }
        }
        // Sending now while an ask waits is the user talking to the agent
        // ahead of it: the waiting words are theirs to drop, and they just
        // did (the TUI's status says so).
        self.forget_queued(ticket, "queued_ask_dropped", "local", "person");
        self.deliver(ticket, seat, text, Ack::PROMPT, plan, immediately)
    }

    /// Why Claude Code's send-now (T-601) cannot reach this ticket's agent,
    /// or `None`: the seat is `Board::send_now_seat`, the predicate the
    /// TUI offers the `immediately` stop by (T-685), so a refusal here is
    /// one the field never offered. Two ways to fall short: the seat's
    /// provider has no such key (Codex), or there is no pane for the key to
    /// cut into — a parked agent or an empty seat, whose words a wake or a
    /// start takes whole, as `now` would send them.
    fn immediate_refusal(&self, ticket: ulid::Ulid) -> Option<&'static str> {
        if self.board.send_now_seat(ticket).is_some() {
            return None;
        }
        Some(if self.tier_book().seat_provider(ticket).has_send_now() {
            "has no turn for a send-now to cut into ∙ now delivers the words"
        } else {
            "has no send-now (Claude Code's alone)"
        })
    }

    /// Why a plan-mode ask cannot reach this ticket (T-434), or `None`. The
    /// flag is `--permission-mode plan`, Claude Code's; a Codex session, or
    /// an empty seat on a Codex board, has no launch flag for its plan mode.
    /// The seat's provider is `Book::seat_provider`, the one the TUI's
    /// `plan_able` reads (T-685).
    fn plan_refusal(&self, ticket: ulid::Ulid, plan: bool) -> Option<String> {
        if !plan {
            return None;
        }
        (!self.tier_book().seat_provider(ticket).has_plan_flag())
            .then(|| "plan mode is a claude launch flag ∙ this seat runs codex".to_string())
    }

    /// Ask every seat in a column — paned, parked or EMPTY (T-405). The one
    /// road (`deliver`) puts the words in front of each: a pane is pasted
    /// into, a parked agent is woken with them held, and a ticket with no
    /// agent starts one on them. Queued prompts wait for idle; shared
    /// checkouts are serialized in board order. Park the batch before
    /// draining so one receipt covers the gesture.
    ///
    /// T-378 skipped the empty seats — "a column is not a place to spawn N
    /// claudes from one key" — to keep the plural the same idea as the
    /// singular. T-379 then made the SINGULAR start on an empty seat, so
    /// that refusal was the odd one out and this is it lifted. What holds a
    /// burst back is the queued default (`drain_queue` takes one per quiet
    /// checkout per pass) and `spawn_gate`, not a refusal here.
    fn prompt_column(
        &mut self,
        column: &str,
        text: String,
        queued: bool,
        accept_plan: bool,
    ) -> Response {
        if self.board.column(column).is_none() {
            return Response::Err { message: format!("no such column: {column}") };
        }
        let ids: Vec<ulid::Ulid> = self.board.column_tickets(column).iter().map(|t| t.id).collect();
        // The seats the flag reaches (T-429): a pane on its dialog or known
        // to be planning. Every other seat takes the words as the ordinary
        // column ask, and off a pane the flag is nothing, as it is for the
        // single ask.
        let accepts: Vec<ulid::Ulid> = if accept_plan {
            ids.iter().copied().filter(|t| self.plan_able_seat(*t).is_some()).collect()
        } else {
            Vec::new()
        };
        // Blank in, nothing out — with the EMPTY SEAT exception `prompt_session`
        // already makes (T-294): there the Enter lands on the ticket title the
        // spawn types, which is a turn the user did write. So a blank column ask
        // reaches the empty seats and skips every agent already sitting in one;
        // with no empty seat to take it, it is the refusal it always was. A
        // blank ACCEPT is the other exception: "accept the plans, ask nothing".
        let words = mesimon_core::command::sanitize_prompt(&text);
        if words.is_none()
            && accepts.is_empty()
            && !ids.iter().any(|t| matches!(self.seat_of(*t), QueuedSeat::Start(_)))
        {
            return Response::Err { message: "nothing to send".into() };
        }
        self.feed.board("local", "prompt_column", None);
        let (mut sent, mut woke, mut started, mut skipped, mut failed) = (0, 0, 0, 0, 0);
        let mut parked: Vec<(ulid::Ulid, &'static str)> = Vec::new();
        let mut accepting = 0;
        for ticket in ids {
            let external = self.board.live_agent(ticket).is_some_and(|rec| rec.observe_only());
            let seat = self.seat_of(ticket);
            let starts = matches!(seat, QueuedSeat::Start(_));
            let accept = accepts.contains(&ticket);
            if external || (words.is_none() && !starts && !accept) {
                skipped += 1;
                continue;
            }
            let word = seat.word();
            let text = words.clone().unwrap_or_default();
            // An accept is always a QUEUED entry (T-420): the press is the
            // clock's, one per quiet checkout (`service_plan_accepts`), so
            // the first goes now and the rest follow as each implementation
            // ends. The column's toggle does not reach it.
            if accept {
                match self.park_ask(ticket, seat, text, None, true, false) {
                    Ok(()) => accepting += 1,
                    Err(message) => {
                        eprintln!("mesimon: column accept could not park: {message}");
                        failed += 1;
                    }
                }
                continue;
            }
            // A worktree ticket's checkout is its own, so there is nobody to
            // wait for and `park_ask` refuses a start there outright. It goes
            // now whatever the column's toggle says — the single ask's rule
            // since T-294, which never offers the toggle on that seat.
            let now_anyway = starts && !self.shared_checkout(ticket);
            if queued && !now_anyway {
                match self.park_ask(ticket, seat, text, None, false, false) {
                    Ok(()) => parked.push((ticket, word)),
                    Err(message) => {
                        eprintln!("mesimon: column ask could not park: {message}");
                        failed += 1;
                    }
                }
                continue;
            }
            self.forget_queued(ticket, "queued_ask_dropped", "local", "person");
            match self.deliver(ticket, seat, text, Ack::PROMPT, false, false) {
                Response::Err { message } => {
                    eprintln!("mesimon: column ask failed: {message}");
                    self.feed.board("local", "prompt_column_failed", Some(ticket));
                    failed += 1;
                }
                // `Provisioning` counts with the starts and the wakes: the
                // session is owed, parked behind its worktree, not refused.
                _ if word == "wake" => {
                    self.feed.board("local", "prompt_column_woke", Some(ticket));
                    woke += 1;
                }
                _ if word == "start" => {
                    self.feed.board("local", "prompt_column_started", Some(ticket));
                    started += 1;
                }
                _ => {
                    self.feed.board("local", "prompt_column_sent", Some(ticket));
                    sent += 1;
                }
            }
        }
        let mut still_queued = 0;
        // An accept's press is the 1 s bucket's (`service_plan_accepts`);
        // the rows show now.
        if accepting > 0 && parked.is_empty() {
            self.broadcast();
        }
        if !parked.is_empty() {
            self.drain_queue();
            self.broadcast();
            // The drain sent at most one per quiet checkout; the receipt
            // reads what it did the way `enqueue_ask` does — the in-flight
            // marker for a paste, the record for a wake or a start.
            for (ticket, word) in parked {
                if self.queued.iter().any(|q| q.ticket == ticket) {
                    still_queued += 1;
                } else if self.ask_in_flight(ticket) {
                    sent += 1;
                } else if word != "ask" && self.board.live_agent(ticket).is_some() {
                    if word == "wake" {
                        woke += 1;
                    } else {
                        started += 1;
                    }
                } else {
                    failed += 1;
                }
            }
        }
        Response::Asked {
            sent,
            woke,
            started,
            queued: still_queued,
            skipped,
            failed,
            accepts: accepting,
        }
    }

    /// Where the ticket's claude is, for a prompt: in a pane, parked, or not
    /// there at all. The same three answers `prompt_session` routes on and
    /// `drain_queue` re-checks at delivery — one function, so a queued ask
    /// and a sent one can never disagree about what "the ticket's claude"
    /// means. `has_pane` before `is_live`: a parked session is live and has
    /// no process to type at.
    fn seat_of(&self, ticket: ulid::Ulid) -> QueuedSeat {
        if let Some(id) = self.prompt_target(ticket) {
            return QueuedSeat::Pane(id);
        }
        // Live and paneless is exactly Sleeping (`has_pane` excludes only
        // `Exited` and `Sleeping`), so this arm is the parked claude, and
        // `prompt_sleeping`'s own filter is the belt under it.
        match self.board.live_agent(ticket) {
            Some(rec) => QueuedSeat::Wake(rec.id),
            None => QueuedSeat::Start(self.tier_book().start_provider(ticket)),
        }
    }

    /// Put the user's words in front of this ticket's claude, whatever seat
    /// it is in — the one road, taken by a send-now `PromptSession` and by
    /// `drain_queue` when a checkout goes quiet (T-294). A pane is pasted
    /// into, a parked claude is woken with the words parked for its first
    /// tick, and an EMPTY seat starts one: the title is typed as always and
    /// the words ride under the brief.
    fn deliver(
        &mut self,
        ticket: ulid::Ulid,
        seat: QueuedSeat,
        text: String,
        ack: Ack,
        plan: bool,
        immediately: bool,
    ) -> Response {
        match seat {
            // Plan mode on a pane (T-434) is a relaunch, never a paste — and
            // so is a tier switch the seat owes (T-443), once it is idle:
            // the words ride the relaunch. Owed but mid-turn, a send-now
            // pastes on the tier it has; `prompt_session` queues its own.
            QueuedSeat::Pane(id) if plan || (self.tier_owed(id) && self.session_idle(id)) => {
                self.relaunch(ticket, id, Some(text), plan, "local")
            }
            // `ack` is a pane's alone: a wake and a start park their words
            // as the user's prompt (`park`), whichever road asked.
            // Immediately (T-601) is Claude Code's send-now over the words;
            // a wake or a start below is a launch, which takes them first.
            QueuedSeat::Pane(_) => match self.paste_at(ticket, &text, ack, immediately) {
                // The board's own picture of the session is now a turn behind:
                // the record still says `Idle` until the agent's `UserPromptSubmit`
                // hook lands, and that is the hook's to say, not ours. What we
                // broadcast is the feed entry above — the card catches up when
                // the agent does, the same way it does for a prompt typed in the
                // pane.
                Ok(()) => Response::Ok,
                Err(message) => Response::Err { message },
            },
            QueuedSeat::Wake(_) => self.prompt_sleeping(ticket, text, plan),
            // The provider the start was accepted with, whatever changed
            // since; the ticket's tier gives it a model only where the two
            // agree (`Book::launch`, T-443).
            QueuedSeat::Start(provider) => {
                let words = (!text.is_empty()).then_some(text);
                self.spawn_session(ticket, provider.session_kind(), true, words, None, plan)
            }
        }
    }

    // ------------------------------------------------------------ queued asks

    /// Tickets holding a CHECKOUT: a working claude with that cwd
    /// (`quiet::working_tickets` — a shell never counts), a paste of ours
    /// still owed its ack, and the sessions of a deleted ticket riding out
    /// the grace band there, judged by their frozen state (conservative:
    /// they keep running for 30 s in the same tree).
    fn checkout_holders(&self, cwd: &str) -> Vec<ulid::Ulid> {
        self.working(Some(cwd))
    }

    /// The tickets holding a queued ask back: `checkout_holders` on its
    /// checkout, except that its OWN agent idle at its composer with
    /// background tasks running does not (T-599, `quiet::holds_against_words`):
    /// Claude Code takes a prompt there, and whether the wait is the work or
    /// a loop that never ends is the agent's to say. Another ticket's agent
    /// in the same checkout still holds it, tasks and all.
    fn ask_holders(&self, q: &QueuedAsk) -> Vec<ulid::Ulid> {
        let own = q.ticket;
        self.working_by(Some(&q.cwd), &|s| {
            if s.ticket == own {
                mesimon_core::quiet::holds_against_words(s)
            } else {
                mesimon_core::quiet::is_working(s)
            }
        })
    }

    /// The merge train's gate (T-351). It was `working(None)` — every working
    /// ticket anywhere — until the user asked why three grinding worktrees
    /// should hold up a fourth ticket's merge. They should not.
    ///
    /// An ff-merge writes exactly ONE working tree, and only sometimes: the
    /// ROOT checkout's, when the base is what is checked out there. Otherwise
    /// `worktree::ff_merge` is `git push . <branch>:refs/heads/<base>`, a ref
    /// update that opens no file. A worktree agent mid-turn is untouched by
    /// another ticket's merge either way, so waiting for it bought nothing.
    /// We hold for the root checkout's workers whatever it has checked out:
    /// over-cautious at worst, and the alternative is reading the checkout's
    /// branch on a road whose whole point is that it forks no git (T-289).
    ///
    /// The one board-wide wait that survives: a ticket IN ITS REBASE TURN at
    /// this base tip — asked by us at it, turn still running. Advancing the
    /// base under it lands its rebase on a stale one and earns it a fresh
    /// ask, and six of those in two hours suspend the train for that ticket.
    /// The hold lasts the whole turn, not the git step (T-435): the words
    /// end "run the tests … before we merge", so the turn runs on after
    /// `needs_rebase` clears, and in that window the ticket was neither
    /// mid-rebase nor a merge candidate — the pass fell through and asked
    /// the next behind ticket onto the same tip. Three REVIEW tickets
    /// behind one hand merge were asked 31 s apart, and each merge then
    /// re-asked the rest: N(N+1)/2 turns for N. One ask outstanding per
    /// tip is the rule; the turn ending settles it (`apply_change` →
    /// `Train::settle`), so a LATER turn at the same tip is a bystander
    /// (T-351) and not a hold, and the merge that follows moves the tip.
    fn train_busy(&self) -> Vec<ulid::Ulid> {
        let mut out = self.checkout_holders(&self.paths.repo_root.to_string_lossy());
        for t in self.working(None) {
            if out.contains(&t) {
                continue;
            }
            if self.train.in_rebase_turn(t, self.base_tip_of(t)) {
                out.push(t);
            }
        }
        out
    }

    /// The working tickets, on one checkout (`Some(cwd)`) or the whole board.
    fn working(&self, cwd: Option<&str>) -> Vec<ulid::Ulid> {
        self.working_by(cwd, &mesimon_core::quiet::is_working)
    }

    /// `working` under another reading of the word — the plan accept's
    /// (T-429) — over the same three ledgers: the board's sessions, the owed
    /// pastes, and the grace band's frozen records.
    fn working_by(
        &self,
        cwd: Option<&str>,
        working: &dyn Fn(&mesimon_core::board::SessionRecord) -> bool,
    ) -> Vec<ulid::Ulid> {
        let owed: std::collections::HashSet<ulid::Ulid> =
            self.owed.values().map(|o| o.ticket).collect();
        let mut out = mesimon_core::quiet::working_tickets_by(&self.board, &owed, cwd, working);
        for g in self.grace.values() {
            if !out.contains(&g.ticket.id)
                && g.sessions.iter().any(|s| cwd.is_none_or(|c| s.cwd == c) && working(s))
            {
                out.push(g.ticket.id);
            }
        }
        out
    }

    /// Tickets holding a checkout AGAINST A PLAN ACCEPT (T-429):
    /// `checkout_holders` minus the sessions parked on their own plan
    /// dialog (`quiet::holds_against_accept` — a dialog writes nothing, and
    /// three agents on three dialogs would otherwise hold the checkout
    /// against each other forever), plus every ticket whose press is IN
    /// FLIGHT: its record stays at `Plan` until the harness confirms the
    /// Enter, and from the Enter on it is a writer. A worktree ticket's
    /// checkout is its own, so there the list is only ever itself.
    fn accept_holders(&self, cwd: &str) -> Vec<ulid::Ulid> {
        let mut out = self.working_by(Some(cwd), &mesimon_core::quiet::holds_against_accept);
        for s in &self.board.sessions {
            if s.cwd == cwd && self.plan_accept.contains_key(&s.id) && !out.contains(&s.ticket) {
                out.push(s.ticket);
            }
        }
        out
    }

    /// The pane seat of a ticket that can take an `accept plan`, for the
    /// column's press (T-429).
    /// A ticket on the board and not archived, or the refusal that says
    /// which it is not.
    fn open_ticket(&self, id: ulid::Ulid) -> std::result::Result<&Ticket, &'static str> {
        match self.board.ticket(id) {
            None => Err("no such ticket"),
            Some(t) if t.is_archived() => Err(TICKET_ARCHIVED),
            Some(t) => Ok(t),
        }
    }

    fn plan_able_seat(&self, ticket: ulid::Ulid) -> Option<uuid::Uuid> {
        self.board.plan_seat(ticket).map(|s| s.id)
    }

    fn keys_of(&self, ids: &[ulid::Ulid]) -> Vec<String> {
        ids.iter().filter_map(|t| self.board.ticket(*t)).map(|t| t.short_key.clone()).collect()
    }

    /// Park one prompt per ticket until its target is idle and its checkout
    /// is quiet. A second ask replaces the words; a quiet target sends now.
    /// Empty shared-checkout seats retain the queued-start behavior.
    fn enqueue_ask(
        &mut self,
        ticket: ulid::Ulid,
        seat: QueuedSeat,
        text: String,
        accept_plan: bool,
        plan: bool,
    ) -> Response {
        let word = seat.word();
        if let Err(message) = self.park_ask(ticket, seat, text, None, accept_plan, plan) {
            return Response::Err { message };
        }
        self.drain_queue();
        self.broadcast();
        if let Some(q) = self.queued.iter().find(|q| q.ticket == ticket) {
            let held = q.held.map(str::to_string);
            let order = self.queue_order();
            Response::Queued {
                behind: self.ask_waits_on(ticket, &order),
                asking: self.ask_asking(ticket, &order),
                held,
            }
        } else if self.ask_in_flight(ticket) {
            Response::Ok
        } else if word != "ask" || plan {
            // A start or a wake delivered on the spot — or a pane relaunched
            // into plan mode (T-434), which is a wake — holds the checkout
            // through its own record (`Spawning` + an owed Enter), so there
            // is no in-flight marker to look for — the session is the receipt.
            match self.board.live_agent(ticket) {
                Some(rec) => Response::Spawned { id: rec.id, fresh: false },
                None => Response::Err { message: "could not deliver".into() },
            }
        } else {
            Response::Err { message: "could not deliver".into() }
        }
    }

    /// Put one ask in the queue without draining it: the checks, the
    /// push-or-replace and its feed line, and nothing else — so a column's
    /// worth of asks (T-378) can be parked in one pass and drained once,
    /// with one broadcast, instead of N of each. `enqueue_ask` is this plus
    /// the drain and the receipt.
    ///
    /// `by` is the crown ticket when the words are its agent's (T-413): the
    /// entry is then HELD for a person's send — until the crown's handler
    /// sets `sends` (T-550) — and it may replace only the crown's own
    /// earlier ask — a person's queued words are never overwritten by an
    /// agent's. A person's ask replaces either.
    fn park_ask(
        &mut self,
        ticket: ulid::Ulid,
        seat: QueuedSeat,
        text: String,
        by: Option<ulid::Ulid>,
        accept_plan: bool,
        plan: bool,
    ) -> Result<(), String> {
        let t = self.open_ticket(ticket).map_err(str::to_string)?;
        if by.is_some() && self.queued.iter().any(|q| q.ticket == ticket && q.by.is_none()) {
            return Err(format!(
                "{} already has a person's ask queued; it goes first",
                t.short_key
            ));
        }
        // An empty worktree has no resolved checkout yet. Starting it does
        // not interrupt a turn; provisioning retains its existing path.
        if !self.shared_checkout(ticket) && matches!(seat, QueuedSeat::Start(_)) {
            return Err("start the worktree session before queueing a follow-up".into());
        }
        // The checkout the delivery will land in: the target's own cwd where
        // there is a session, else the shared root a spawn would resolve to
        // (`resolve_spawn_cwd`, a pure read for this strategy).
        let cwd = match &seat {
            QueuedSeat::Pane(id) | QueuedSeat::Wake(id) => {
                match self.board.sessions.iter().find(|s| s.id == *id) {
                    Some(rec) => rec.cwd.clone(),
                    None => return Err("no such session".into()),
                }
            }
            QueuedSeat::Start(_) => self.paths.repo_root.display().to_string(),
        };
        // The PTY budget is deliberately NOT consulted here: a start that
        // would be refused for resources now may be fine when its turn comes,
        // and `spawn_session` says so at delivery either way.
        if let Some(id) =
            self.queued.iter().find(|q| q.ticket == ticket).and_then(|q| match q.seat {
                QueuedSeat::Pane(id) => Some(id),
                _ => None,
            })
        {
            self.control_cancel(id);
        }
        let now = now_ms();
        let word = seat.word();
        // Parked while its agent is ALREADY on a question (T-565): the hold
        // `apply_change` puts on words queued before the stop, at once —
        // the stop has no edge left to come. Every road parks here: the
        // field, the column, the phone's Queue and the crown's ask, whose
        // handler may then set `sends` (T-550); `held` outranks it.
        let held = match seat {
            QueuedSeat::Pane(id) if self.session_asking(id) => Some("agent asked"),
            _ => None,
        };
        // The feed names the author and never the words: a person's ask is
        // `queued_ask`; the crown's is `ask_agent` with actor `agent`,
        // written by its handler with the road the words took (T-600).
        let fresh = format!("queued_{word}");
        // A person's words over the crown's (T-568): the crown's are dropped,
        // and its `get_ticket` says so. The crown's own new ask supersedes
        // whatever became of its last.
        match by {
            Some(_) => {
                self.crown_dropped.remove(&ticket);
            }
            None => {
                if let Some(crown) =
                    self.queued.iter().find(|q| q.ticket == ticket).and_then(|q| q.by)
                {
                    self.crown_dropped_ask(ticket, crown, "person");
                }
            }
        }
        if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == ticket) {
            q.text = text;
            // Editing queued words does not reinterpret the accepted start
            // after a project provider switch.
            if !matches!((&q.seat, &seat), (QueuedSeat::Start(_), QueuedSeat::Start(_))) {
                q.seat = seat;
            }
            q.cwd = cwd;
            q.by = by;
            // The crown's road is its handler's to set again (T-550).
            q.sends = false;
            // Re-queued by hand: the person read the words again, so a
            // hold is answered — unless the question is still up, whose
            // answer may change them yet — and the flag is whatever the
            // field said.
            q.accept_plan = accept_plan;
            q.send_on_accept = false;
            q.held = held;
            q.plan = plan && !accept_plan;
            // So is the crown's level (T-600, T-601).
            q.deliver = Deliver::Idle;
            if by.is_none() {
                self.feed.board("local", "queued_ask_replaced", Some(ticket));
            }
        } else {
            self.queued.push(QueuedAsk {
                ticket,
                seat,
                cwd,
                text,
                queued_at: now,
                by,
                sends: false,
                accept_plan,
                send_on_accept: false,
                held,
                plan: plan && !accept_plan,
                deliver: Deliver::Idle,
            });
            if by.is_none() {
                self.feed.board("local", &fresh, Some(ticket));
            }
        }
        if held.is_some() {
            self.feed.board("automation", "queued_ask_held_question", Some(ticket));
        }
        self.persist_queue();
        Ok(())
    }

    /// Is the ticket's checkout the shared one — `SharedCheckout` by
    /// strategy and no worktree bound to it? Only this kind of empty seat
    /// can queue a start before its checkout is resolved.
    fn shared_checkout(&self, ticket: ulid::Ulid) -> bool {
        self.board.ticket(ticket).is_some_and(|t| {
            t.workspace_strategy() == WorkspaceStrategy::SharedCheckout
                && !self.worktrees.contains_key(&ticket)
        })
    }

    /// The queued asks in BOARD order — column order, then row order, the
    /// merge train's walk (`train::plan`) — so the user sorts the queue by
    /// sorting the cards (T-263, user: "so that user can sort while items
    /// are queued"). Read at every drain and every snapshot, never stored:
    /// FIFO was the first shape, and it made the order invisible and
    /// unchangeable. A ticket the board no longer lists sorts last; the
    /// sweep drops it. Built once per pass and handed to `ask_waits_on` and
    /// `ask_asking` (T-688): a snapshot asks those per queued ask.
    fn queue_order(&self) -> Vec<usize> {
        let mut rank: HashMap<ulid::Ulid, (usize, usize)> = HashMap::new();
        for (ci, col) in self.board.sorted_columns().iter().enumerate() {
            for (ri, t) in self.board.column_tickets(&col.name).iter().enumerate() {
                rank.insert(t.id, (ci, ri));
            }
        }
        let mut idx: Vec<usize> = (0..self.queued.len()).collect();
        idx.sort_by_key(|&i| rank.get(&self.queued[i].ticket).copied().unwrap_or((usize::MAX, 0)));
        idx
    }

    /// What a parked ask waits on, as keys: the tickets working in its
    /// checkout, then the asks queued AHEAD of it there in board order —
    /// so the card reads `queued ∙ after T-3 +1` and a card moved up its
    /// column watches the count fall. Empty for a ticket with no ask.
    /// `order` is this pass's `queue_order`.
    fn ask_waits_on(&self, ticket: ulid::Ulid, order: &[usize]) -> Vec<String> {
        self.keys_of(&self.ask_waits_on_ids(ticket, order))
    }

    /// `ask_waits_on`, as the tickets themselves.
    fn ask_waits_on_ids(&self, ticket: ulid::Ulid, order: &[usize]) -> Vec<ulid::Ulid> {
        let Some(q) = self.queued.iter().find(|q| q.ticket == ticket) else {
            return Vec::new();
        };
        // A held ask (T-413, T-420) waits on a person, not the checkout.
        if q.held_for_person() {
            return Vec::new();
        }
        // A flagged ask (T-429) waits on the accept's holders — the tickets
        // working in its checkout, a dialog not among them, a press in
        // flight among them — and on the flagged asks ahead of it there
        // whose dialog is up, since `service_plan_accepts` presses those
        // first. Its own press in flight lists itself, which the card
        // reads as `accepting plan`.
        if q.accept_plan || q.send_on_accept {
            let mut ids = self.accept_holders(&q.cwd);
            for &i in order {
                let ahead = &self.queued[i];
                if ahead.ticket == ticket {
                    break;
                }
                let at_plan = matches!(ahead.seat, QueuedSeat::Pane(id)
                    if self.session_at_plan(id));
                if ahead.accept_plan
                    && ahead.held.is_none()
                    && ahead.cwd == q.cwd
                    && at_plan
                    && !ids.contains(&ahead.ticket)
                {
                    ids.push(ahead.ticket);
                }
            }
            return ids;
        }
        let mut ids = self.ask_holders(q);
        if !self.queued_target_ready(q) && !ids.contains(&ticket) {
            ids.push(ticket);
        }
        for &i in order {
            let ahead = &self.queued[i];
            if ahead.ticket == ticket {
                break;
            }
            if !ahead.held_for_person() && ahead.cwd == q.cwd && !ids.contains(&ahead.ticket) {
                ids.push(ahead.ticket);
            }
        }
        ids
    }

    /// The keys among what the ask waits on whose agent is on a question
    /// (T-565) — a turn that ends only when a person answers, so the card
    /// says `after T-3's answer` rather than `after T-3`. A held ask waits
    /// on nobody's turn; its own key is listed while its own agent asks,
    /// so the card can say the answer comes before the send.
    fn ask_asking(&self, ticket: ulid::Ulid, order: &[usize]) -> Vec<String> {
        let Some(q) = self.queued.iter().find(|q| q.ticket == ticket) else {
            return Vec::new();
        };
        let ids: Vec<ulid::Ulid> = if q.held_for_person() {
            match q.seat {
                QueuedSeat::Pane(id) if self.session_asking(id) => vec![ticket],
                _ => Vec::new(),
            }
        } else {
            self.ask_waits_on_ids(ticket, order)
                .into_iter()
                .filter(|t| self.ticket_asking(*t))
                .collect()
        };
        self.keys_of(&ids)
    }

    /// Is this session's agent on a question (T-565,
    /// `attention::on_question`): stated so, or stated so last and demoted
    /// by nothing but the stale clock.
    fn session_asking(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().find(|s| s.id == id).is_some_and(|s| self.rec_asking(s))
    }

    /// Does this pane's agent wait on a person — a dialog up, or a question
    /// the stale clock demoted (T-565)? A paste there lands in the dialog as
    /// its answer, so a send (`send_queued_ask`, T-420) and a phone's steer
    /// (T-568) are refused.
    fn pane_waits_on_you(&self, id: uuid::Uuid) -> bool {
        self.session_asking(id)
            || self
                .board
                .sessions
                .iter()
                .any(|s| s.id == id && matches!(s.state, SessionState::RequiresAction { .. }))
    }

    fn rec_asking(&self, rec: &SessionRecord) -> bool {
        attention::on_question(&rec.state, self.machines.get(&rec.id))
    }

    /// Is any agent of this ticket on a question (`session_asking`)?
    fn ticket_asking(&self, ticket: ulid::Ulid) -> bool {
        self.board
            .sessions
            .iter()
            .any(|s| s.ticket == ticket && s.kind.is_agent() && self.rec_asking(s))
    }

    /// Paste the TOPMOST waiting ask of every QUIET checkout — one per
    /// checkout per pass, since the paste itself makes it busy again
    /// (`owed`), and in board order (`queue_order`), so the next one
    /// goes when this one's agent has acked and settled. Runs from
    /// `apply_change` (the EndTurn settle and the shutdown flush both come
    /// through it), on the 1 s bucket, and at enqueue. A target that is not
    /// the pane it was queued at is dropped, not redirected.
    fn queued_target_ready(&self, q: &QueuedAsk) -> bool {
        match q.seat {
            // A relaunch (plan mode, a tier switch owed) ends the pane and
            // its tasks with it, so it waits for a true idle (T-599).
            QueuedSeat::Pane(id) if q.plan || self.tier_owed(id) => self.session_idle(id),
            QueuedSeat::Pane(id) => self.session_takes_words(id),
            // A Codex record wakes only once its runtime confirmed the stop
            // (a tier switch parks one and queues its words here, T-443);
            // trying earlier is a refusal, and the words would be lost.
            QueuedSeat::Wake(id) => {
                !self.board.sessions.iter().any(|s| s.id == id && s.codex_stopping)
            }
            QueuedSeat::Start(_) => true,
        }
    }

    /// Is this pane's session between turns — idle, not parked in the
    /// background, and not working by any of `quiet`'s signs? What a
    /// relaunch waits for (a tier switch, plan mode); a paste of words reads
    /// the wider `session_takes_words` (T-599).
    fn session_idle(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().any(|s| {
            s.id == id
                && matches!(s.state, SessionState::Idle { stop_reason } if stop_reason != StopReason::Background)
                && !mesimon_core::quiet::is_working(s)
        })
    }

    /// Will this pane's session take words now — `session_idle`, or idle at
    /// its composer while background tasks run (T-599,
    /// `quiet::holds_against_words`)? What a queued ask and the crown's wake
    /// read. A tier switch or a plan relaunch keeps `session_idle`: those
    /// end the pane, and the tasks with it.
    fn session_takes_words(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().any(|s| {
            s.id == id
                && matches!(s.state, SessionState::Idle { .. })
                && !mesimon_core::quiet::holds_against_words(s)
        })
    }

    /// Is this session parked on its plan dialog — the `≡`?
    fn session_at_plan(&self, id: uuid::Uuid) -> bool {
        self.board.sessions.iter().any(|s| {
            s.id == id && s.state == (SessionState::RequiresAction { reason: Reason::Plan })
        })
    }

    fn drain_queue(&mut self) -> bool {
        let mut seen: Vec<String> = Vec::new();
        let mut take: Vec<usize> = Vec::new();
        for i in self.queue_order() {
            // A held ask (T-413) is a person's to send; it neither goes nor
            // takes the checkout's turn from the ask behind it. The same
            // for one the daemon held on a question (T-420). The crown's
            // ask the board lets it send (T-550) goes like a person's.
            if self.queued[i].held_for_person() {
                continue;
            }
            let cwd = self.queued[i].cwd.clone();
            let quiet = !seen.contains(&cwd)
                && self.ask_holders(&self.queued[i]).is_empty()
                && !self.checkout_unresolved(&cwd)
                && self.queued_target_ready(&self.queued[i]);
            seen.push(cwd);
            if quiet {
                take.push(i);
            }
        }
        // Highest index first, so the lower indexes stay valid as entries
        // leave; which checkout pastes first does not matter.
        take.sort_unstable_by(|a, b| b.cmp(a));
        let mut changed = false;
        for i in take {
            let QueuedAsk { ticket, seat, text, plan, by, .. } = self.queued.remove(i);
            changed = true;
            let word = seat.word();
            if !self.seat_stands(ticket, &seat) {
                self.feed.board("automation", "queued_ask_dropped_target_gone", Some(ticket));
                if let Some(crown) = by {
                    self.crown_dropped_ask(ticket, crown, "board");
                }
                continue;
            }
            // A pane's paste is owed an ack, so the checkout is held by its
            // `owed` entry until it lands (`Ack::QUEUED`: the card says
            // `queued ∙ sending`). A wake and a start hold it through their
            // own record — `Spawning` and an owed Enter are both WORKING —
            // and their entry is the user's prompt, so the card shows the
            // launching arc instead. A pane relaunched into plan mode
            // (T-434) is a wake in all but its seat word.
            let sent = match seat {
                seat @ QueuedSeat::Pane(pane) if !plan && !self.tier_owed(pane) => {
                    match self.deliver_queued_ask(ticket, seat, text, Ack::QUEUED, false, false) {
                        Response::Ok => true,
                        _ => {
                            self.feed.board("automation", "queued_ask_failed", Some(ticket));
                            false
                        }
                    }
                }
                seat => match self.deliver(ticket, seat, text, Ack::PROMPT, plan, false) {
                    Response::Err { message } => {
                        eprintln!("mesimon: queued {word} failed: {message}");
                        self.feed.board(
                            "automation",
                            &format!("queued_{word}_failed"),
                            Some(ticket),
                        );
                        false
                    }
                    _ => true,
                },
            };
            if !sent {
                continue;
            }
            match by {
                // The crown's words, sent on the board's say-so (T-550): the
                // feed names the crown's agent as the actor, the card lights
                // `♛ sent`, and the turn that takes them is the crown's
                // answer (T-469), as after a person's `^y`.
                Some(crown) => {
                    self.feed.board("agent", "ask_agent_sent", Some(ticket));
                    self.tag_owed(ticket, TurnAsk::Crown(crown));
                    self.crown_touched(crown, ticket, "sent");
                }
                None => {
                    self.feed.board("automation", &format!("queued_{word}_sent"), Some(ticket));
                }
            }
        }
        if changed {
            self.persist_queue();
        }
        changed
    }

    /// Does an agent on this checkout sit at `Unknown` with a pane — the
    /// state every session of ours has right after a daemon restart, before
    /// the transcript tail or a hook says what it is doing? `is_working`
    /// does not count it (the reconcile resolves it within a tick), but a
    /// RESTORED start (T-418) may be judged on the very first tick, and a
    /// checkout that cannot be proved quiet is not quiet — the whole point
    /// of the queue is never to be a second writer in one index.
    fn checkout_unresolved(&self, cwd: &str) -> bool {
        self.board.sessions.iter().any(|s| {
            s.cwd == cwd
                && s.kind.is_agent()
                && s.state.has_pane()
                && matches!(s.state, SessionState::Unknown { .. })
        })
    }

    /// Is the seat an entry was queued at still the seat it named? A pane
    /// must be the same pane, a parked claude the same record (woken by hand
    /// in the meantime is fine — same session, same conversation), and a
    /// `Start` needs the seat still EMPTY, since a claude somebody started
    /// there is not one to start beside. A seat that changed drops the
    /// entry; nothing is ever redirected.
    fn seat_stands(&self, ticket: ulid::Ulid, seat: &QueuedSeat) -> bool {
        match seat {
            QueuedSeat::Pane(id) => self.prompt_target(ticket) == Some(*id),
            QueuedSeat::Wake(id) => self.board.live_agent(ticket).map(|s| s.id) == Some(*id),
            QueuedSeat::Start(_) => self.board.live_agent(ticket).is_none(),
        }
    }

    /// Drop the asks whose ticket or seat is gone: the ticket deleted or
    /// archived, the pane dead or parked, a different session in the seat,
    /// or — for a queued start — a claude somebody started there by hand
    /// (`seat_stands`). The explicit cancels (`kill_session`, `sleep_one`,
    /// `delete_ticket`, a send-now) name their reason; this is the net.
    fn sweep_queue(&mut self) -> bool {
        let stale: Vec<ulid::Ulid> = self
            .queued
            .iter()
            .filter(|q| {
                let ticket_gone = self.board.ticket(q.ticket).is_none_or(|t| t.is_archived());
                ticket_gone || !self.seat_stands(q.ticket, &q.seat)
            })
            .map(|q| q.ticket)
            .collect();
        for t in &stale {
            self.forget_queued(*t, "queued_ask_dropped_target_gone", "automation", "board");
        }
        !stale.is_empty()
    }

    /// Drop the ticket's queued ask, unsent. `dropped_by` is what the
    /// crown's `get_ticket` says of its own words (T-568): `person` when a
    /// person acted on them (replaced, took back, talked past), `board` when
    /// the seat they waited at went (the agent slept or ended, the ticket
    /// left).
    fn forget_queued(
        &mut self,
        ticket: ulid::Ulid,
        why: &str,
        actor: &str,
        dropped_by: &'static str,
    ) -> bool {
        let before = self.queued.len();
        let queued = self.queued.iter().find(|q| q.ticket == ticket);
        let session = queued.and_then(|q| match q.seat {
            QueuedSeat::Pane(id) => Some(id),
            _ => None,
        });
        if let Some(crown) = queued.and_then(|q| q.by) {
            self.crown_dropped_ask(ticket, crown, dropped_by);
        }
        self.queued.retain(|q| q.ticket != ticket);
        if let Some(id) = session {
            self.control_cancel(id);
        }
        if self.queued.len() != before {
            self.feed.board(actor, why, Some(ticket));
            self.persist_queue();
            return true;
        }
        false
    }

    fn deliver_queued_ask(
        &mut self,
        ticket: ulid::Ulid,
        seat: QueuedSeat,
        text: String,
        ack: Ack,
        plan: bool,
        immediately: bool,
    ) -> Response {
        let id = match seat {
            QueuedSeat::Pane(id) => Some(id),
            _ => None,
        };
        if let Some(id) = id {
            if !self.control_delivery_allowed(id) {
                return Response::Err { message: "prompt authorization expired".into() };
            }
            let paired = self.control_delivery_principal(id);
            let action = if paired.is_some() { Action::PromptExisting } else { Action::Mutate };
            let principal = paired.unwrap_or(Principal::Automation { rule: "queued_ask".into() });
            if authorize(&principal, &action, &Resource::Session { id }).denied() {
                self.control_cancel(id);
                return Response::Err { message: "prompt not authorized".into() };
            }
        }
        let response = self.deliver(ticket, seat, text, ack, plan, immediately);
        if let Some(id) = id {
            if self.control_delivery_principal(id).is_some() && self.parked(id) {
                if let Some(rec) = self.board.sessions.iter_mut().find(|r| r.id == id) {
                    rec.pending_prefill = false;
                }
            }
            if matches!(response, Response::Err { .. }) {
                self.control_cancel(id);
            } else if !self.parked(id) {
                self.control_submitted(id);
            }
        }
        response
    }

    /// The agent of `session` stopped on a question: hold the ask queued at
    /// its pane (T-420). A crown's held ask is held already, and one it
    /// sends (T-550) is held like a person's — the answer may change it
    /// too; an ask in flight has left the queue. Idempotent, so a
    /// re-asserted state costs nothing.
    fn hold_queued_on_question(&mut self, session: uuid::Uuid) {
        let Some(q) = self.queued.iter_mut().find(|q| {
            matches!(q.seat, QueuedSeat::Pane(id) if id == session) && !q.held_for_person()
        }) else {
            return;
        };
        q.held = Some("agent asked");
        let ticket = q.ticket;
        self.feed.board("automation", "queued_ask_held_question", Some(ticket));
    }

    /// Press Enter on the plan dialog `session`'s pane is showing, at the
    /// row the harness highlights by default (T-420). Refuses, naming why,
    /// when the pane is not on a dialog it recognises or the cursor is not
    /// on that row — the safe side is the dialog left up. The harness
    /// confirms the press through its own hooks; `settle_plan_accepts` is
    /// the deadline on that.
    fn accept_plan(&mut self, session: uuid::Uuid, actor: &str) -> Result<(), &'static str> {
        let ticket = self.press_plan(session)?;
        self.feed.board(actor, "plan_accepted", Some(ticket));
        Ok(())
    }

    /// `accept_plan`'s press without its feed line, for a road that writes
    /// its own (the crown's, T-582): the checks, the screen, the Enter, the
    /// deadline. `Ok` is the ticket pressed for.
    fn press_plan(&mut self, session: uuid::Uuid) -> Result<ulid::Ulid, &'static str> {
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Err("no such session");
        };
        if rec.state != (SessionState::RequiresAction { reason: Reason::Plan }) {
            return Err("no plan waiting");
        }
        if self.plan_accept.contains_key(&session) {
            return Err("accepting the plan");
        }
        let sid = rec.sid16();
        let kind = rec.kind;
        let ticket = rec.ticket;
        let screen = self
            .backend
            .capture_input_screen(&sid)
            .map_err(|_| crate::plan_dialog::NOT_RECOGNISED)?;
        match kind {
            SessionKind::Claude => crate::plan_dialog::claude_at_default(&screen.lines)?,
            SessionKind::Codex => crate::plan_dialog::codex_at_default(&screen)?,
            _ => return Err("no plan waiting"),
        }
        self.backend.send_enter(&sid).map_err(|_| "could not press enter")?;
        self.plan_accept.insert(session, now_ms() + PLAN_ACCEPT_CONFIRM_MS);
        self.plan_accept_tries.remove(&session);
        Ok(ticket)
    }

    /// The flagged asks (T-420): a pane at `Plan` with an `accept_plan`
    /// entry queued gets the press, once. The dialog paints after the hook
    /// that lands the `≡`, so a miss is retried on the next pass, up to
    /// `PLAN_ACCEPT_TRIES`; then the flag goes and the ask waits as an
    /// unflagged one does. An entry with no words leaves the queue on the
    /// press — "accept the plan, ask nothing" — and one with words keeps
    /// waiting for the idle that follows the accepted turn.
    ///
    /// The press obeys the checkout (T-429): one flagged ask per QUIET
    /// checkout per pass, in board order, the way `drain_queue` pastes —
    /// an accepted plan is an implementation starting, and two of those in
    /// one index is what the queue exists to prevent. Quiet is
    /// `accept_holders`' word: a dialog holds nothing, a press in flight
    /// does, and a checkout that cannot be proved quiet is not. The rule
    /// gates WHEN the press goes, never whether the dialog is recognised —
    /// "never a blind Enter" is `accept_plan`'s, unchanged. Worktree
    /// tickets have their own checkout and press on their own clock.
    fn service_plan_accepts(&mut self, _now: u64) -> bool {
        let mut seen: Vec<String> = Vec::new();
        let mut due: Vec<(ulid::Ulid, uuid::Uuid)> = Vec::new();
        for i in self.queue_order() {
            let q = &self.queued[i];
            if !q.accept_plan || q.held.is_some() {
                continue;
            }
            let QueuedSeat::Pane(id) = q.seat else { continue };
            if self.plan_accept.contains_key(&id) || !self.session_at_plan(id) {
                continue;
            }
            if seen.contains(&q.cwd) {
                continue;
            }
            let quiet = self.accept_holders(&q.cwd).is_empty() && !self.checkout_unresolved(&q.cwd);
            if !quiet {
                continue;
            }
            seen.push(q.cwd.clone());
            due.push((q.ticket, id));
        }
        let mut changed = false;
        for (ticket, id) in due {
            let principal = Principal::Automation { rule: "queued_ask".into() };
            if authorize(&principal, &Action::Mutate, &Resource::Session { id }).denied() {
                continue;
            }
            match self.accept_plan(id, "automation") {
                Ok(()) => {
                    if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == ticket) {
                        q.accept_plan = false;
                        // Words ride the confirmation (`settle_plan_accepts`);
                        // none, and the accept was the whole ask.
                        q.send_on_accept = !q.text.is_empty();
                        if q.text.is_empty() {
                            self.queued.retain(|q| q.ticket != ticket);
                        }
                    }
                    changed = true;
                }
                Err(why) => {
                    let tries = self.plan_accept_tries.entry(id).or_insert(0);
                    *tries += 1;
                    if *tries >= PLAN_ACCEPT_TRIES {
                        self.plan_accept_tries.remove(&id);
                        if let Some(q) = self.queued.iter_mut().find(|q| q.ticket == ticket) {
                            q.accept_plan = false;
                            if q.text.is_empty() {
                                self.queued.retain(|q| q.ticket != ticket);
                            }
                        }
                        eprintln!("mesimon: plan accept on {ticket} gave up: {why}");
                        self.feed.board("automation", "plan_accept_unrecognised", Some(ticket));
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// A press in flight is confirmed by the harness moving the record off
    /// `Plan` (T-420) — Claude's `PostToolUse ExitPlanMode`, Codex's next
    /// turn — and forgotten then. Past the deadline still at `Plan`, the feed
    /// says so and the `≡` stays: the card never claims what the hooks did
    /// not see.
    fn settle_plan_accepts(&mut self, now: u64) -> bool {
        let mut changed = false;
        let pressed: Vec<(uuid::Uuid, u64)> =
            self.plan_accept.iter().map(|(k, v)| (*k, *v)).collect();
        for (id, deadline) in pressed {
            let at_plan = self.board.sessions.iter().any(|s| {
                s.id == id && s.state == (SessionState::RequiresAction { reason: Reason::Plan })
            });
            if !at_plan {
                self.plan_accept.remove(&id);
                changed |= self.send_after_accept(id);
            } else if now >= deadline {
                self.plan_accept.remove(&id);
                let ticket = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket);
                self.feed.board("automation", "plan_accept_unconfirmed", ticket);
                // The words stay, as an ordinary ask: they wait for idle.
                if let Some(q) = self
                    .queued
                    .iter_mut()
                    .find(|q| matches!(q.seat, QueuedSeat::Pane(s) if s == id) && q.send_on_accept)
                {
                    q.send_on_accept = false;
                }
                changed = true;
            }
        }
        changed
    }

    /// The accepted plan's turn has begun (the record left `Plan`): the
    /// words flagged to ride it go in now (T-420). A Claude pane takes a
    /// paste mid-turn and shows it to the agent at its next step, which is
    /// the point — "main moved since the plan started" is worth nothing
    /// after the implementation. Into a live turn only: a record that left
    /// `Plan` for anything but `Running`/`Idle` (a death, a park) keeps the
    /// words queued for the sweep to judge.
    fn send_after_accept(&mut self, session: uuid::Uuid) -> bool {
        let Some(i) = self.queued.iter().position(|q| {
            matches!(q.seat, QueuedSeat::Pane(s) if s == session) && q.send_on_accept
        }) else {
            return false;
        };
        let live = self.board.sessions.iter().any(|s| {
            s.id == session && matches!(s.state, SessionState::Running | SessionState::Idle { .. })
        });
        if !live {
            self.queued[i].send_on_accept = false;
            return true;
        }
        let QueuedAsk { ticket, seat, text, .. } = self.queued.remove(i);
        if !self.seat_stands(ticket, &seat) {
            self.feed.board("automation", "queued_ask_dropped_target_gone", Some(ticket));
            return true;
        }
        match self.deliver_queued_ask(ticket, seat, text, Ack::QUEUED, false, false) {
            Response::Ok => {
                self.feed.board("automation", "queued_ask_sent_after_plan", Some(ticket));
            }
            _ => self.feed.board("automation", "queued_ask_failed", Some(ticket)),
        }
        true
    }

    fn send_queued_ask(&mut self, ticket: ulid::Ulid) -> Response {
        let Some(i) = self.queued.iter().position(|q| q.ticket == ticket) else {
            return Response::Err { message: "nothing queued on this ticket".into() };
        };
        // A pane on a dialog takes a paste as an ANSWER (T-420): the words
        // would land in the question, or the plan's revise row. The person
        // answers in the pane first; the ask keeps waiting. A question the
        // stale clock demoted to `Unknown` is still up (T-565).
        if let QueuedSeat::Pane(id) = self.queued[i].seat {
            if self.pane_waits_on_you(id) {
                return Response::Err {
                    message: mesimon_core::command::ANSWER_IN_PANE_FIRST.into(),
                };
            }
        }
        let q = self.queued.remove(i);
        self.persist_queue();
        if !self.seat_stands(ticket, &q.seat) {
            if let Some(crown) = q.by {
                self.crown_dropped_ask(ticket, crown, "board");
            }
            self.broadcast();
            return Response::Err { message: "queued session changed".into() };
        }
        let immediately = q.deliver == Deliver::Immediately;
        let response =
            self.deliver_queued_ask(ticket, q.seat, q.text, Ack::QUEUED, q.plan, immediately);
        // The crown's words, sent by a person: the turn that takes them is
        // the crown's answer (T-469).
        if let (Some(crown), false) = (q.by, matches!(response, Response::Err { .. })) {
            self.tag_owed(ticket, TurnAsk::Crown(crown));
        }
        self.broadcast();
        response
    }

    /// Remove and return the current words on the single writer.
    fn take_queued_ask(&mut self, ticket: ulid::Ulid) -> Response {
        let Some(q) = self.queued.iter().find(|q| q.ticket == ticket) else {
            return Response::Err { message: "nothing queued on this ticket".into() };
        };
        let text = q.text.clone();
        self.drop_queued_ask(ticket);
        Response::PromptTakenBack { text }
    }

    fn drop_queued_ask(&mut self, ticket: ulid::Ulid) -> Response {
        if self.forget_queued(ticket, "queued_ask_dropped", "local", "person") {
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
    /// (`arm_owed`). `pending_submit` is set for the same
    /// reason the composer sets it — the launching arc on the card is how the
    /// user watches the ask land — and the seat is still ONE claude: a wake
    /// re-enters the record, it never mints a second.
    fn prompt_sleeping(&mut self, ticket: ulid::Ulid, text: String, plan: bool) -> Response {
        self.wake_with(ticket, Parked { text, brief: false, title: false }, plan)
    }

    /// A person's plain wake that came up fresh (T-603): the conversation
    /// never took a prompt, so there was none to resume, and the new pane's
    /// composer is empty where the plain start's held the title. The title
    /// is typed again, never submitted, as the start typed it and as the
    /// hook-set relaunch does (`relaunch_on_hooks`). A wake that carries
    /// words owes them and types nothing.
    fn retype_title(&mut self, resp: &Response) {
        let Response::Spawned { id, fresh: true } = resp else { return };
        if self.owed.contains_key(id) {
            return;
        }
        let Some(rec) =
            self.board.sessions.iter().find(|s| s.id == *id && s.kind == SessionKind::Claude)
        else {
            return;
        };
        let Some(title) =
            self.board.ticket(rec.ticket).map(|t| t.title.trim()).filter(|t| !t.is_empty())
        else {
            return;
        };
        let _ = self.backend.send_text(&rec.sid16(), &format!("{title} "));
    }

    /// Wake the ticket's parked claude and hold `words` for its first tick:
    /// an ask (`prompt_sleeping`), or the brief of a conversation that never
    /// took a prompt (T-603).
    fn wake_with(&mut self, ticket: ulid::Ulid, words: Parked, plan: bool) -> Response {
        let Some(id) = self
            .board
            .live_agent(ticket)
            .filter(|s| matches!(s.state, SessionState::Sleeping))
            .map(|s| s.id)
        else {
            return Response::Err {
                message: "no live agent session on this ticket — start or wake one first".into(),
            };
        };
        let resp = self.resume_session_in(id, false, plan);
        match &resp {
            Response::Spawned { .. } => self.park(id, ticket, words, Ack::PROMPT),
            // The worktree is being rebuilt under the wake (T-278): the
            // words ride the parked resume and land when it replays.
            Response::Provisioning => {
                if let Some(r) = self.pending_resumes.iter_mut().find(|r| r.session == id) {
                    r.prompt = Some(words.text);
                    r.brief = words.brief;
                    r.plan = plan;
                }
            }
            _ => {}
        }
        resp
    }

    fn restore_ticket(&mut self, id: ulid::Ulid) -> Response {
        let Some(mut g) = self.grace.remove(&id) else {
            return Response::Err { message: "grace window expired".into() };
        };
        for (meta, bytes) in &g.attachments {
            if let Err(e) = crate::attachments::save(&self.paths, &g.ticket.short_key, meta, bytes)
            {
                self.grace.insert(id, g);
                return Response::Err { message: format!("could not restore pictures: {e:#}") };
            }
        }
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
        for session in g.sessions {
            if !self.board.sessions.iter().any(|existing| existing.id == session.id) {
                self.board.sessions.push(session);
            }
        }
        self.persist_and_notify();
        Response::Ok
    }

    /// Archive: off the board, everything kept (ticket file, sleeping
    /// sessions) — and the worktree too, unless its work has landed
    /// (`reclaim_on_archive`). Gated on the ticket holding no pane —
    /// archive means everything is already asleep.
    fn archive_ticket(&mut self, id: ulid::Ulid, by: &Principal) -> Response {
        match self.board.ticket(id) {
            None => return no_such_ticket(),
            Some(t) if t.is_archived() => {
                return Response::Err { message: "already archived".into() }
            }
            Some(_) => {}
        }
        if self.board.ticket_awake_sessions(id) > 0 {
            return Response::Err { message: "sessions still awake — sleep them first".into() };
        }
        if let Some(message) = open_boxes_refusal(self.open_boxes(id)) {
            return Response::Err { message };
        }
        self.archive_one(id, now_iso(), by.note_author());
        // Re-price now — a taken offer must not linger until the next bucket.
        self.archive_cache = self.archive_figures();
        self.broadcast();
        Response::Ok
    }

    /// The boxes still unticked in the ticket's `Summary` sections (T-713),
    /// over every note, read from disk as `get_ticket` reads them. What the
    /// card counts as left to do: a ticket that holds one is not finished,
    /// so the archive refuses it and the offer never prices it. A snooze is
    /// a return and is not asked. A note that cannot be read counts nothing.
    fn open_boxes(&self, id: ulid::Ulid) -> usize {
        let Some(t) = self.board.ticket(id) else { return 0 };
        t.notes
            .iter()
            .filter_map(|n| store::read_note(&self.paths, &t.short_key, n.id).ok())
            .map(|body| {
                mesimon_core::summary::Count::of(&mesimon_core::summary::extract(&body)).open()
            })
            .sum()
    }

    /// One ticket off the board, whichever gesture asked — `a a` or the
    /// offer's X, which once set the flag on its own and skipped the reclaim
    /// (T-481: 49 landed worktrees outlived their archive, ~126 GB of
    /// `target/`). The caller has judged the gates; this is the rest, so the
    /// two cannot drift again: the flag on disk, the crown (a crowned ticket
    /// leaving the board takes it along, T-411 — a snooze is the one archive
    /// that keeps it, and does not come here), the reclaim. False when the
    /// ticket is not on the board.
    fn archive_one(&mut self, id: ulid::Ulid, at: String, by: String) -> bool {
        let Some(t) = self.board.ticket_mut(id) else { return false };
        t.archived = Some(Archived { at, by, until: None, needs_you: false });
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        self.drop_crown_if(id);
        self.reclaim_on_archive(id);
        true
    }

    /// The archive is where the disk went to hide (T-278, 2026-09-06: 14 of
    /// the board's 16 worktrees belonged to archived tickets whose branches
    /// were already on main, ~2 GB of `target/` each). So an archived
    /// ticket's worktree is torn down when its work has LANDED — merged by
    /// the same oracle the card and the DONE gate answer with
    /// (`ticket_merged`: an ancestor of the base, or the sample's patch-id
    /// verdict, so a squashed PR counts) — and nothing on the ticket has a
    /// pane, which the archive gate already holds. It goes down the delete's
    /// own road (`process_teardowns`: after the reaper, the T-273 terminal
    /// killed first, single `--force`, `branch -d` — never `-D`, which stays
    /// behind the user's discard). Unmerged work keeps its worktree exactly
    /// as before: the archive stays reversible for work that has not landed.
    /// A snooze takes none of this — a snooze is a return.
    fn reclaim_on_archive(&mut self, id: ulid::Ulid) {
        let Some(b) = self.worktrees.get(&id) else { return };
        let merged = !b.branch.is_empty() && self.ticket_merged(id);
        let awake = self.board.ticket_awake_sessions(id);
        if !worktree::reclaim_on_archive(b, merged, awake, self.worktrees_barred) {
            return;
        }
        if self.pending_teardown.iter().any(|t| t.ticket == id) || self.tearing_down.contains(&id) {
            return;
        }
        self.pending_teardown.push(Teardown {
            ticket: id,
            why: TeardownWhy::Archived,
            sids: vec![],
        });
    }

    /// The archive's reclaim, for the trees it never reached (T-481): the
    /// offer's X skipped it until then, and the queue it feeds is memory
    /// only, so an archive that raced a restart lost its teardown. At start
    /// and whenever a worktree sample lands, an archived ticket (never a
    /// snooze — a snooze is a return) whose tree still stands on disk and
    /// whose sample reads merged goes to `reclaim_on_archive`, and its fresh
    /// `ticket_merged` and pane gates decide. The sample is the prefilter,
    /// so an unmerged archived tree costs no fork per bucket; a tree already
    /// gone is left alone, so the squash-merged branch `branch -d` refuses
    /// (`Evicted`, kept on purpose) is not re-queued, and re-logged, every
    /// bucket. A ticket whose `!` terminal is open waits for it to close:
    /// nobody asked for this teardown just now, and it would kill that shell.
    fn reclaim_archived(&mut self) {
        let due: Vec<ulid::Ulid> = self
            .worktrees
            .iter()
            .filter(|(id, b)| {
                let archived = self
                    .board
                    .ticket(**id)
                    .and_then(|t| t.archived.as_ref())
                    .is_some_and(|a| a.until.is_none());
                archived
                    && self.wt_agg.get(*id).is_some_and(|a| a.merged)
                    && b.path.is_dir()
                    && !self.terminals.contains_key(&Some(**id))
            })
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.reclaim_on_archive(id);
        }
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
    /// or a shell with a live child) holds the ticket on
    /// the board with nothing on it touched, in words that name it.
    /// `wake_snoozed` on the tick wheel is the other half; the sessions
    /// stay asleep when the ticket returns, and `c` wakes them.
    fn snooze_ticket(&mut self, id: ulid::Ulid, until: u64, needs_you: bool) -> Response {
        match self.board.ticket(id) {
            None => return no_such_ticket(),
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
                return Response::Err { message: mesimon_core::quiet::still_awake(*kind, &why) };
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
        let resp = self.with_ticket(id, |t| {
            t.woke_at = None;
            t.archived = Some(Archived {
                at,
                by: "local".into(),
                until: Some(format!("@{until}")),
                needs_you,
            });
        });
        self.archive_cache = self.archive_figures();
        resp
    }

    /// The cursor rested on a ticket a snooze woke lit: the mark comes off.
    /// A no-op — no write, no broadcast — on a ticket that wears none, so
    /// the TUI can send it whenever it likes.
    fn seen_ticket(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            Some(t) if !t.is_woke() => Response::Ok,
            _ => self.with_ticket(id, |t| t.woke_at = None),
        }
    }

    /// The person opened a ticket's page (T-497): a phone's ticket nobody had
    /// picked up is picked up at the desk. A no-op — no write, no broadcast —
    /// on any other ticket, as `seen_ticket` is.
    fn opened_ticket(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            Some(t) if !t.awaits_pickup() => Response::Ok,
            _ => self.with_ticket(id, |t| {
                t.picked = Some(PickedUp { at: now_iso(), by: PICKED_AT_DESK.into() })
            }),
        }
    }

    /// An agent started on a phone's ticket (T-497): picked up, if nobody
    /// had. Saves the ticket and says whether it changed; the caller
    /// broadcasts.
    fn picked_by_agent(&mut self, ticket: ulid::Ulid) -> bool {
        let Some(t) = self.board.ticket_mut(ticket).filter(|t| t.awaits_pickup()) else {
            return false;
        };
        t.picked = Some(PickedUp { at: now_iso(), by: PICKED_BY_AGENT.into() });
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        true
    }

    /// The person is done with a raised hand (T-107): the mark comes off.
    /// A no-op — no write, no broadcast — on a ticket holding none, so the
    /// TUI can send it on every departure from a ticket page.
    fn lower_hand(&mut self, id: ulid::Ulid) -> Response {
        match self.board.ticket(id) {
            Some(t) if !t.hand_raised() => Response::Ok,
            _ => self.with_ticket(id, |t| t.raised = None),
        }
    }

    /// The other road down: a turn reached the ticket's agent, so whatever it
    /// was waiting for, it has been given. Returns whether anything changed —
    /// the caller is already inside a hook turn and owns the persist and the
    /// broadcast.
    ///
    /// Two callers, one idea — the hand comes down when the NEXT turn starts.
    /// `UserPromptSubmit` is the road a typed line takes; `apply_change`'s
    /// `Idle{EndTurn}` → `Running` is the road a `!` bash command takes, which
    /// fires no prompt hook at all (T-311, and see T-228 for the measurement).
    /// The same edge from a held dialog into `Running` is the answer given in
    /// the pane (T-611): the turn goes on, so no next turn ever starts.
    ///
    /// The daemon cannot tell its own paste's ack from a line the user typed
    /// (the queued ask states the same limit), so a prompt mesimon delivers
    /// also lowers the hand. The one delivery that would have mattered — the
    /// merge train's notice — cannot reach a raised hand at all: `train::plan`
    /// skips the ticket for as long as one is up.
    fn lower_hand_on(&mut self, ticket: ulid::Ulid) -> bool {
        let Some(t) = self.board.ticket_mut(ticket) else { return false };
        if t.raised.take().is_none() {
            return false;
        }
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        true
    }

    /// Take a ticket off the merge train, or put it back (T-227). The flag
    /// is the ticket's and the planner reads it, so the next `pending_items`
    /// already lists nothing for it; a person's gesture on the ticket also
    /// clears the train's memory of it, the way a hand `m` does. A no-op
    /// when nothing changes — no write, no broadcast.
    fn set_manual_merge(&mut self, id: ulid::Ulid, on: bool) -> Response {
        match self.board.ticket(id) {
            None => return no_such_ticket(),
            Some(t) if t.manual_merge == on => return Response::Ok,
            Some(_) => {}
        }
        self.train.hand_touched(id);
        self.with_ticket(id, |t| t.manual_merge = on)
    }

    /// `Command::SetCrownBudget` (T-412): the cap on crown-started seats.
    /// Lowering it below what is held stops the next start and kills
    /// nothing — the seats already paid for run out on their own.
    fn set_crown_budget(&mut self, budget: u8) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.crown_budget != budget {
            self.board.crown_budget = budget;
            self.persist_and_notify();
        }
        Response::Ok
    }

    /// `Command::SetCrownMode` (T-610, over T-550's and T-569's switches):
    /// whether the crown's asks to the agents it started go by the queue,
    /// and whether it may answer a question or accept a plan one of them
    /// stopped on, and is woken by either. Supervised means nothing more
    /// goes out on the crown's word: an ask still waiting to send is held
    /// for the person from here, and an answer still walking its keys stops
    /// at its next step (`state_changed`, `crown_answer_allowed`); a wake
    /// already owed is still said, and the crown's answer is then refused in
    /// words naming this row. Autonomous releases nothing already held —
    /// those words were left for the person to read, and the crown may ask
    /// again.
    fn set_crown_mode(&mut self, mode: CrownMode) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.crown_mode != mode {
            self.board.crown_mode = mode;
            if !mode.sends() {
                self.hold_crown_sends();
            }
            self.persist_and_notify();
        }
        Response::Ok
    }

    /// `Command::SetCrownArchives` (T-590): whether the crown's
    /// `archive_ticket` archives and restores. Judged at each call, so an
    /// off holds from the crown's next one.
    fn set_crown_archives(&mut self, on: bool) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.crown_archives != on {
            self.board.crown_archives = on;
            self.persist_and_notify();
        }
        Response::Ok
    }

    /// `Command::SetCrownWatches` (T-712): whether the crown's `watch_ticket`
    /// watches. Judged at each call, and an off ends every watch standing:
    /// the row is the person's stop, not a gate on the next call alone.
    fn set_crown_watches(&mut self, on: bool) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.crown_watches != on {
            self.board.crown_watches = on;
            if !on && !self.crown_watched.is_empty() {
                self.crown_watched.clear();
                self.feed.board("automation", "crown_watch_dropped", self.board.crown);
            }
            self.persist_and_notify();
        }
        Response::Ok
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
        // A person who turned it OFF has answered the question the offer
        // asks: the chip does not come back to ask it again. Settings still
        // turns it on, which is what makes the stamp affordable here too.
        if !on {
            self.board.claude_md_ignored = true;
        }
        self.persist_and_notify();
        Response::Ok
    }

    /// One of the three sentences mesimon types into an agent's box (T-353).
    /// A person's row in Settings — `mcp::agent_allows` denies the command —
    /// and `columns.toml`'s, so it returns early under the bar like the
    /// switches above it.
    ///
    /// The text is sanitized HERE and stored sanitized, through
    /// `sanitize_template` — the ask sanitizer's one-line twin: the bytes on
    /// disk are the bytes the tty will receive, and a template cannot carry
    /// a CR that would split one prompt into two turns, nor a newline (a
    /// typed ask may since T-380; a template is one line by law). Blank in
    /// — which is what an emptied field sends — is mesimon's own words back,
    /// and writes nothing.
    fn set_agent_prompt(
        &mut self,
        which: mesimon_core::prompts::AgentPrompt,
        text: Option<String>,
    ) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let text = text.as_deref().and_then(mesimon_core::prompts::sanitize_template);
        if self.board.prompts.custom(which) == text.as_deref() {
            return Response::Ok;
        }
        self.board.prompts.set(which, text);
        self.persist_and_notify();
        Response::Ok
    }

    /// The default column (T-279): where an agent's `create_ticket` lands a
    /// card that names no column. `None` is the first column again. A
    /// person's row in Settings — `mcp::agent_allows` denies the command —
    /// and `columns.toml`'s, so it returns early under the bar like the two
    /// switches above. The next `create_ticket` reads it; nothing already on
    /// the board moves.
    fn set_default_column(&mut self, column: Option<&str>) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if self.board.default_column.as_deref() == column {
            return Response::Ok;
        }
        if let Err(message) = self.board.set_default_column(column) {
            return Response::Err { message };
        }
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

    // ---- the column lifecycle (T-117) -----------------------------------
    //
    // Every one of these writes `columns.toml`, so every one returns early
    // under the bar (`set_mcp_tools`'s reason) — except `sort_column`, which
    // writes ticket files only, and those self-bar. The feed line is the
    // wire name (`Command::meta`), with the person as actor.

    fn add_column(&mut self, name: String, after: Option<String>) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let Some(name) = mesimon_core::board::sanitize_column_name(&name) else {
            return Response::Err { message: "a column needs a name".into() };
        };
        if let Err(message) = self.board.add_column(name, after.as_deref()) {
            return Response::Err { message };
        }
        self.persist_and_notify();
        Response::Ok
    }

    /// The name is the foreign key, so this is a transaction: every ticket
    /// in the column (archived too), every other column's rule naming it,
    /// the move gate's memory and the grace band's held tickets — a restore
    /// pushes the held clone back verbatim, and one holding the old name
    /// would land invisible.
    fn rename_column(&mut self, name: &str, to: &str) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let Some(to) = mesimon_core::board::sanitize_column_name(to) else {
            return Response::Err { message: "a column needs a name".into() };
        };
        if name == to {
            return Response::Ok;
        }
        let touched = match self.board.rename_column(name, &to) {
            Ok(t) => t,
            Err(message) => return Response::Err { message },
        };
        for id in touched {
            if let Some(t) = self.board.ticket(id) {
                let _ = store::save_ticket(&self.paths, t);
            }
        }
        self.moves.rename_column(name, &to);
        for g in self.grace.values_mut() {
            if g.ticket.column == name {
                g.ticket.column = to.clone();
            }
        }
        self.persist_and_notify();
        Response::Ok
    }

    fn delete_column(&mut self, name: &str) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        // A ticket in the grace band counts as live: `restore_ticket` pushes
        // it back into the column it was deleted from.
        let held = self.grace.values().filter(|g| g.ticket.column == name).count();
        if held > 0 {
            return Response::Err {
                message:
                    "a ticket just deleted from it can still come back — wait for the undo band"
                        .into(),
            };
        }
        let cleared = match self.board.delete_column(name) {
            Ok(c) => c,
            Err(message) => return Response::Err { message },
        };
        for (col, field) in cleared {
            self.feed.board("local", &format!("column_rule_cleared:{field}"), None);
            let _ = col;
        }
        self.persist_and_notify();
        Response::Ok
    }

    fn reorder_column(&mut self, name: &str, before: Option<String>) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        if let Err(message) = self.board.reorder_column(name, before.as_deref()) {
            return Response::Err { message };
        }
        self.persist_and_notify();
        Response::Ok
    }

    fn set_column_settings(
        &mut self,
        name: &str,
        settings: mesimon_core::board::ColumnSettings,
    ) -> Response {
        if self.columns_barred {
            return Response::Err { message: self.barred_message("columns") };
        }
        let Some(col) = self.board.column(name) else {
            return Response::Err { message: format!("no such column: {name}") };
        };
        // The description is text agents read (T-467): scrubbed and capped
        // on the way in, so the bytes on disk are the bytes a tool returns.
        let description = settings
            .description
            .as_deref()
            .and_then(mesimon_core::board::sanitize_column_description);
        let settings = mesimon_core::board::ColumnSettings { description, ..settings };
        if col.settings == settings {
            return Response::Ok;
        }
        let offers_changed = col.settings.offers() != settings.offers();
        if let Err(message) = self.board.set_column_settings(name, settings) {
            return Response::Err { message };
        }
        if offers_changed {
            // Changing eligibility should not leave the old offer standing
            // until the next RSS sample. Reuse the latest byte measurements.
            self.reclaim_cache = self.reclaim_figures();
        }
        self.persist_and_notify();
        Response::Ok
    }

    /// One-shot. `needs_you` is what the header's `!N` counts: the attention
    /// queue's tickets and the woken ones.
    fn sort_column(&mut self, column: &str, by: mesimon_core::board::SortBy) -> Response {
        if self.board.column(column).is_none() {
            return Response::Err { message: format!("no such column: {column}") };
        }
        let needs_you = self.board.needs_you_tickets();
        let touched = self.board.sort_column(column, by, &needs_you);
        for id in touched {
            if let Some(t) = self.board.ticket(id) {
                let _ = store::save_ticket(&self.paths, t);
            }
        }
        self.broadcast();
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
            let needs_you = self
                .board
                .ticket(id)
                .and_then(|t| t.archived.as_ref())
                .is_some_and(|a| a.needs_you);
            if self.unarchive(id, Some(Position::Top), needs_you).is_none() {
                continue;
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
            None => return no_such_ticket(),
            Some(t) if !t.is_archived() => return Response::Err { message: "not archived".into() },
            Some(_) => {}
        }
        match self.unarchive(id, None, false) {
            Some(()) => {
                self.broadcast();
                Response::Ok
            }
            None => no_such_ticket(),
        }
    }

    /// The one road back from the archive, for a restore and a snooze's wake
    /// alike. The ticket lands in its own column where that still exists,
    /// else the first (fixed template today; policies in M5); `land` says
    /// where in it — `None` keeps the order it left with, `Some(Top)` is a
    /// wake, fresh news that outranks what was there. `entered_at` restamps
    /// whenever the ticket moves (a new column, or a landing asked for), and
    /// `needs_you` lights it. The move gate and the train forget it: what
    /// they remembered — a reversal to refuse, a blown fuse — describes a
    /// board from before it left, and applying it to the first move back
    /// would be a refusal nobody could explain. Saved, never broadcast: the
    /// callers' clocks differ. `column_tickets` never lists an archived
    /// ticket, so the order is computed against the board it rejoins.
    fn unarchive(&mut self, id: ulid::Ulid, land: Option<Position>, needs_you: bool) -> Option<()> {
        let t = self.board.ticket(id)?;
        let col = if self.board.columns.iter().any(|c| c.name == t.column) {
            t.column.clone()
        } else {
            self.board.sorted_columns().first()?.name.clone()
        };
        let moved = col != t.column;
        let order = match &land {
            Some(pos) => self.order_within(&col, id, pos),
            None if moved => self.order_within(&col, id, &Position::Before(None)),
            None => t.order.clone(),
        };
        self.moves.forget(id);
        self.train.forget(id);
        self.persist_train();
        let stamp = now_iso();
        let t = self.board.ticket_mut(id)?;
        t.archived = None;
        t.column = col;
        t.order = order;
        if land.is_some() || moved {
            t.entered_at = Some(stamp.clone());
        }
        if needs_you {
            t.woke_at = Some(stamp);
        }
        let t = t.clone();
        let _ = store::save_ticket(&self.paths, &t);
        Some(())
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
                    if s.holds_process() && !s.observe_only() {
                        if s.kind == SessionKind::Codex {
                            let by =
                                Principal::Automation { rule: "deleted_session_cleanup".into() };
                            if !matches!(
                                authorize(&by, &Action::Mutate, &Resource::Session { id: s.id }),
                                Decision::Deny { .. }
                            ) {
                                self.kill_session(s.id);
                            }
                        } else {
                            let _ = self.backend.signal_session(&s.sid16());
                        }
                        self.reaping.insert(s.sid16(), Instant::now() + REAP_GRACE);
                        sids.push(s.sid16());
                    }
                }
                // M4: worktree teardown waits for the reaper — never remove a
                // directory a live process still has as cwd (12 §12.6.5).
                if self.worktrees.contains_key(&id) {
                    self.pending_teardown.push(Teardown {
                        ticket: id,
                        why: TeardownWhy::Deleted { discard: g.discard_worktree },
                        sids,
                    });
                }
            }
        }
        self.broadcast();
    }

    /// Teardown transaction (12 §12.6.1), once every pane of the ticket left
    /// the reaper: unlock → remove --force (single force; NEVER -f -f) →
    /// branch -d if merged, -D only under the user's discard confirmation.
    /// An archived ticket's (T-278) is judged again here — still archived,
    /// still merged (a terminal standing in the tree could have committed
    /// meanwhile) — and keeps its binding as `Evicted` while the branch
    /// survives (`branch -d` refuses a squash-merged one), so a restore
    /// replays the same branch; a branch that went takes the binding with
    /// it, and the next spawn provisions fresh.
    fn process_teardowns(&mut self) {
        if self.pending_teardown.is_empty() {
            return;
        }
        let ready: Vec<usize> = self
            .pending_teardown
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                !t.sids.iter().any(|s| self.reaping.contains_key(s))
                    && !self.board.sessions.iter().any(|s| s.ticket == t.ticket && s.codex_stopping)
                    // One flight per tree: a second ask (a delete after an
                    // archive) waits for the first to land and is judged
                    // against what is left.
                    && !self.tearing_down.contains(&t.ticket)
            })
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
            let Teardown { ticket, why, .. } = self.pending_teardown.remove(i);
            let Some(b) = self.worktrees.get(&ticket).cloned() else { continue };
            let merged = !b.branch.is_empty() && self.ticket_merged(ticket);
            let archived = matches!(why, TeardownWhy::Archived);
            if archived {
                let still_archived = self.board.ticket(ticket).is_some_and(|t| t.is_archived());
                if !still_archived || !merged {
                    // Restored before its turn, or no longer landed: the
                    // archive's rule no longer holds, and the tree stays.
                    continue;
                }
            }
            if b.path.is_dir() {
                // The worktree's terminal (T-273) stands in the directory
                // about to go: it is no session of the ticket, so the reaper
                // never saw it. Killed here, first — never remove a live cwd.
                let _ = self.backend.kill_session(&terminal_name(Some(ticket)));
            }
            // The gates are judged here, on the writer, with the board in
            // hand; the removal itself goes to a worker (T-561). `worktree
            // remove --force` unlinks every file of the tree — a `target/`
            // of a few GB is seconds — and `archive_all` queues one per
            // ticket for the same tick, so done here it was the whole
            // board waiting on a snapshot for 5–13 s (the journal's
            // `process_teardowns` lines). Every leg, then the container
            // (T-368): a merged leg's branch goes with `-d`, a discard's
            // with `-D`; unmerged without discard keeps the branch (commits
            // survive). On a workspace each leg is judged on its own, so a
            // landed leg's branch goes while an unmerged sibling's stays.
            let discard = matches!(why, TeardownWhy::Deleted { discard: true });
            let workspace = b.is_workspace();
            let branch = b.branch.clone();
            let repo_root = self.paths.repo_root.clone();
            let tx = self.tx.clone();
            self.tearing_down.insert(ticket);
            std::thread::spawn(move || {
                let started = Instant::now();
                let branch_kept = worktree::teardown(&repo_root, &b, &|leg| {
                    let leg_merged = if workspace {
                        !leg.base.is_empty() && worktree::is_merged(&leg.repo, &branch, &leg.base)
                    } else {
                        merged
                    };
                    if leg_merged {
                        Some(false)
                    } else if discard {
                        Some(true)
                    } else {
                        None
                    }
                });
                let _ = tx.send(Msg::TornDown {
                    ticket,
                    archived,
                    branch_kept,
                    took: started.elapsed(),
                });
            });
        }
    }

    /// A worker finished removing a tree (T-561): the bookkeeping the
    /// teardown turn used to do after `worktree::teardown` returned. An
    /// archived ticket whose branch `branch -d` refused (squash-merged)
    /// keeps its binding as `Evicted`, so a restore replays the same
    /// branch; a branch that went takes the binding with it. A spawn or a
    /// wake that arrived while the tree was going parked behind it
    /// (`resolve_spawn_cwd`), and provisions now — the replay is
    /// `on_provisioned`'s, as for any other provision. The flags take the
    /// worker road, as `on_provisioned`'s do.
    fn on_torn_down(
        &mut self,
        ticket: ulid::Ulid,
        archived: bool,
        branch_kept: bool,
        took: Duration,
    ) {
        self.tearing_down.remove(&ticket);
        let key = self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        self.journal.line(&format!("torn down {key}: {} ms", took.as_millis()));
        if archived && branch_kept {
            if let Some(b) = self.worktrees.get_mut(&ticket) {
                b.status = BindingStatus::Evicted;
                b.locked = false;
            }
            self.feed.board("automation", "worktree_torn_down:branch_kept", Some(ticket));
        } else {
            if archived {
                self.feed.board("automation", "worktree_torn_down", Some(ticket));
            }
            self.worktrees.remove(&ticket);
            self.wt_agg.remove(&ticket);
            self.wt_repos.remove(&ticket);
            self.wt_progress.remove(&ticket);
            self.wt_fresh.remove(&ticket);
        }
        self.persist_worktrees();
        if self.pending_spawns.iter().any(|s| s.ticket == ticket)
            || self.pending_resumes.iter().any(|r| r.ticket == ticket)
        {
            self.queue_provision(ticket);
        }
        self.queue_worktree_flags();
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
            None,
            "move_ticket",
        ) {
            Ok(_) => Response::Ok,
            Err(message) => Response::Err { message },
        }
    }

    /// `started_by` is the crown's ticket when the crown asked (T-412) and
    /// `None` for every start a person or a column rule made; it lands on
    /// the record and is what the spawn budget counts.
    fn spawn_session(
        &mut self,
        ticket: ulid::Ulid,
        kind: SessionKind,
        submit_prompt: bool,
        prompt: Option<String>,
        started_by: Option<ulid::Ulid>,
        plan: bool,
    ) -> Response {
        // An archived ticket must not grow a live pane no board surface shows.
        if let Err(message) = self.open_ticket(ticket) {
            return Response::Err { message: message.into() };
        }
        // A joined board has no checkout here (T-215): nothing to run in.
        if self.team_content_only() {
            return Response::Err {
                message: "this board has no repository on this machine — open it where the code is"
                    .into(),
            };
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
        if kind.is_agent() {
            if self.pending_resumes.iter().any(|pending| pending.ticket == ticket) {
                return Response::Err {
                    message: "an agent resume is already provisioning on this ticket".into(),
                };
            }
            if let Some(held) = self.board.live_agent(ticket) {
                if held.codex_stopping {
                    return Response::Err {
                        message: "agent is still stopping — wait for its server cleanup before starting another".into(),
                    };
                }
                let verb =
                    if matches!(held.state, SessionState::Sleeping) { "wake" } else { "focus" };
                return Response::Err {
                    message: format!("ticket already has an agent session — {verb} it instead"),
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
                if !self.pending_spawns.iter().any(|s| {
                    s.ticket == ticket && (s.kind == kind || (s.kind.is_agent() && kind.is_agent()))
                }) {
                    self.pending_spawns.push(PendingSpawn {
                        ticket,
                        kind,
                        submit_prompt,
                        prompt,
                        started_by,
                        plan,
                    });
                }
                self.persist_and_notify();
                return Response::Provisioning;
            }
            Err(message) => return Response::Err { message },
        };
        let id = uuid::Uuid::new_v4();
        // The road (T-574), decided once and stamped below with the argv it
        // shaped.
        let pick = self.launch_road(kind, id);
        let road = pick.road;
        let spec = if let Some(adapter) = crate::agents::adapter(kind) {
            match adapter
                .start(&self.launch_context(id, ticket, kind, &cwd, plan, &pick), &id.to_string())
            {
                Ok(spec) => spec,
                Err(message) => return Response::Err { message },
            }
        } else {
            LaunchSpec::plain(vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())])
        };
        let argv = spec.argv;
        // Claude enters Spawning; the SessionStart hook flips it to Running.
        // Bash has no hook surface — a live pane is all "running" means (D15).
        let state = match kind {
            SessionKind::Claude | SessionKind::Codex => SessionState::Spawning,
            SessionKind::Bash => SessionState::Running,
        };
        let mut rec =
            SessionRecord::new(id, kind, ticket, argv.clone(), cwd.display().to_string(), state);
        rec.state_changed_at = Some(now_ms());
        rec.codex_generation = spec.generation;
        rec.started_by = started_by;
        rec.road = road;
        rec.native = pick.native();
        if kind == SessionKind::Codex {
            rec.agent_preview_path =
                Some(crate::agents::codex::preview_path(&self.paths, id).display().to_string());
            rec.pending_prefill = true;
        }
        let launch = self.launch(&argv, &self.launch_vars(ticket, &cwd, id, kind, &pick));
        match self.backend.spawn(&rec.sid16(), &cwd, &launch) {
            Ok(pane) => rec.pane_key = Some(pane),
            Err(e) => return Response::Err { message: format!("spawn failed: {e}") },
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
        // `SessionStart` frame instead (`arm_owed`, `settle_owed`).
        //
        // The title rides argv nowhere: `claude <title>` would dispatch a
        // title that happens to name a subcommand ("doctor", "update") to that
        // subcommand instead, silently, and `--` does not shield it (measured
        // 2026-08-31). Keystrokes have no such vocabulary.
        //
        // A Claude launch on the mod road whose prompt mesimon submits types
        // nothing (T-575): the title, the brief and the words go down the
        // mod's bridge as one `submit` the moment it polls, as the person's
        // own words, so neither Claude's raw mode nor its paste detection
        // nor its `<pasted_content>` wrapping (T-588) is in the way.
        let by_mod = submit_prompt && kind == SessionKind::Claude && road == Road::Mod;
        if kind.is_agent() {
            if let Some(title) =
                self.board.ticket(ticket).map(|t| t.title.trim()).filter(|t| !t.is_empty())
            {
                if kind == SessionKind::Codex
                    || by_mod
                    || self.backend.send_text(&rec.sid16(), &format!("{title} ")).is_ok()
                {
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
                    // Parked EMPTY and read at paste time (T-117): a
                    // description written between the spawn and
                    // `SessionStart` is the brief too.
                    if submit_prompt {
                        // `prompt` is the ask a Shift+Enter carried into an
                        // empty seat (T-294) — parked BESIDE the brief, not
                        // instead of it: the agent gets the ticket's own
                        // words and then the user's. Empty is the ordinary
                        // composed spawn, whose prompt is the title and the
                        // description.
                        let text = prompt.unwrap_or_default();
                        let mut owed = Owed::launch(
                            ticket,
                            Parked { text, brief: true, title: by_mod },
                            Ack::PROMPT,
                        );
                        owed.mod_road = by_mod;
                        self.owed.insert(id, owed);
                    }
                }
            }
        }
        self.machines.insert(id, Machine::new(rec.state.clone(), now_ms()));
        self.board.sessions.push(rec);
        self.stamp_tier(id);
        if kind.is_agent() {
            self.picked_by_agent(ticket);
        }
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
                Some(b)
                    if b.status == BindingStatus::Attached
                        && !self.tearing_down.contains(&ticket) =>
                {
                    Ok(Some(b.path.clone()))
                }
                _ => Err("no worktree bound to this ticket — adopt one first".into()),
            },
            WorkspaceStrategy::Worktree => {
                // Barred bindings refuse the spawn here, on the one road every
                // spawn takes (T-683) — the desk's `c`, the phone's start, the
                // crown's, the queue's `Start` and the pending-spawn replay —
                // because `persist_worktrees` would never record the tree
                // `queue_provision` cuts, and an unrecorded tree is an orphan
                // nothing can reclaim (D26). Three callers checked and two
                // did not; now none has to.
                if self.worktrees_barred {
                    return Err(self.barred_message("worktrees"));
                }
                // On a workspace root (repositories nested one level under
                // it) the provision cuts one worktree per nested repo
                // (T-368, `queue_provision` asks the census).
                if self.tearing_down.contains(&ticket) {
                    // The tree is being removed on a worker (T-561): the
                    // binding still reads `Attached`, but its directory is
                    // going. Park; `on_torn_down` provisions afresh.
                    return Ok(None);
                }
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
            repos: Vec::new(),
            init: None,
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
        // A workspace root (T-368) gets a leg per census repo. The census is
        // asked here (1.5 ms) rather than read off the last sample, so a
        // spawn before the boot sample lands is judged the same way.
        let census = crate::gitstatus::census(&repo);
        let is_meta = repo.join(".git").exists();
        // The init script's road (T-614): the pane's own launcher, so the
        // script sees the environment the agent will, and the seam.
        let launcher = self.init_launcher();
        let init_timeout = worktree::init_timeout();
        std::thread::spawn(move || {
            let started = Instant::now();
            let ptx = tx.clone();
            let result = match evicted {
                Some(prior) => worktree::provision_existing(&repo, ticket, &prior),
                None if census.is_empty() => {
                    worktree::provision(&repo, &root, ticket, &key, &title)
                }
                None => worktree::provision_workspace(
                    &repo,
                    &root,
                    &census,
                    is_meta,
                    ticket,
                    &key,
                    &title,
                    &mut |done, total| {
                        let _ = ptx.send(Msg::ProvisionProgress(ticket, done, total));
                    },
                ),
            };
            let took = started.elapsed();
            // The init script, once per leg that has one, after the tree is
            // cut and before the parked spawn replays (T-614). Its time is
            // not the provision's: the journal keeps the two apart.
            let mut inits = Vec::new();
            if let Ok(b) = &result {
                let vars = vec![
                    ("MESIMON_TICKET".to_string(), key.clone()),
                    ("MESIMON_WORKTREE_BRANCH".to_string(), b.branch.clone()),
                ];
                for leg in b.legs(&repo, "") {
                    if worktree::init_script(&leg.repo).is_none() {
                        continue;
                    }
                    let _ = ptx.send(Msg::ProvisionInit(ticket));
                    if let Some(r) = worktree::run_init(&launcher, &leg, &vars, init_timeout) {
                        inits.push(r);
                    }
                }
            }
            let _ = tx.send(Msg::Provisioned(ticket, result, took, inits));
        });
    }

    fn on_provision_progress(&mut self, ticket: ulid::Ulid, done: u32, total: u32) {
        if self.worktrees.get(&ticket).is_some_and(|b| b.status == BindingStatus::Provisioning) {
            self.wt_progress.insert(ticket, (done, total));
            self.broadcast();
        }
    }

    /// The init script started in a provisioning tree (T-614): the page
    /// says so while it runs, so a long seed reads as what it is.
    fn on_provision_init(&mut self, ticket: ulid::Ulid) {
        if self.worktrees.get(&ticket).is_some_and(|b| b.status == BindingStatus::Provisioning)
            && self.wt_init.insert(ticket)
        {
            self.broadcast();
        }
    }

    /// The init script's launcher (T-614): `mesimon exec --env <file>`,
    /// the pane's own prefix, so the script runs in the environment the
    /// agent will and the captured secrets never ride a command line.
    fn init_launcher(&self) -> Vec<String> {
        vec![
            self.self_exe.display().to_string(),
            "exec".into(),
            "--env".into(),
            self.paths.shell_env_file().display().to_string(),
        ]
    }

    fn on_provisioned(
        &mut self,
        ticket: ulid::Ulid,
        result: std::result::Result<Binding, (String, String)>,
        took: Duration,
        inits: Vec<worktree::InitReport>,
    ) {
        self.wt_progress.remove(&ticket);
        self.wt_init.remove(&ticket);
        match result {
            Ok(mut b) => {
                // The journal keeps the cost (T-368): a workspace's legs are
                // cut one after another, and this line is how the number on
                // a real workspace is read — the single-repo line beside it
                // is the baseline.
                let key =
                    self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
                self.journal.line(&format!(
                    "provisioned {key}: {} in {} ms",
                    mesimon_core::workspace::repos_word(b.repos.len().max(1)),
                    took.as_millis()
                ));
                // The init script's runs (T-614): the journal keeps each
                // leg's word and its output under the ticket's key, the
                // feed one `worktree_init` line per leg with the exit code
                // and the seconds, and the binding the worst run — a
                // failure outranks the legs that went well — until the
                // next provisioning. Nothing here holds the spawn below.
                for r in &inits {
                    let leg = if r.leg.is_empty() { String::new() } else { format!(" {}", r.leg) };
                    self.journal.line(&format!(
                        "init {key}{leg}: {} ({})",
                        r.run.word(),
                        mesimon_core::workspace::INIT_SCRIPT
                    ));
                    for line in r.output.lines() {
                        self.journal.line(&format!("  {key}{leg} | {line}"));
                    }
                    let outcome = if r.leg.is_empty() {
                        r.run.word()
                    } else {
                        format!("{}: {}", r.leg, r.run.word())
                    };
                    self.feed.board_outcome("daemon", "worktree_init", Some(ticket), &outcome);
                }
                b.init = inits.iter().fold(None, |worst, r| match worst {
                    Some(w) if !mesimon_core::workspace::InitRun::ok(&w) => Some(w),
                    _ => Some(r.run.clone()),
                });
                self.worktrees.insert(ticket, b);
                // The replay is timed as a stage (T-430): what is left on
                // this turn is `spawn_session`'s tmux work, and the
                // slow-turn line names it so the number is read off the
                // journal, not guessed.
                let replay = Instant::now();
                let (pending, rest): (Vec<PendingSpawn>, Vec<PendingSpawn>) =
                    self.pending_spawns.drain(..).partition(|s| s.ticket == ticket);
                self.pending_spawns = rest;
                for s in pending {
                    // A failed replay has no client waiting on it — leave a
                    // feed trace (the TUI's parked focus intent surfaces the
                    // "attached but no session" outcome to the user).
                    let kind = s.kind;
                    if let Response::Err { message } = self.spawn_session(
                        s.ticket,
                        kind,
                        s.submit_prompt,
                        s.prompt,
                        s.started_by,
                        s.plan,
                    ) {
                        eprintln!("mesimon: parked spawn replay failed ({kind:?}): {message}");
                        self.feed.board("daemon", "spawn_replay_failed", Some(s.ticket));
                    }
                }
                // A wake parked behind the rebuild (T-278) replays the same
                // way; the words a Shift+Enter parked with it land as
                // `prompt_sleeping` would have landed them.
                let (resumes, rest): (Vec<PendingResume>, Vec<PendingResume>) =
                    self.pending_resumes.drain(..).partition(|r| r.ticket == ticket);
                self.pending_resumes = rest;
                for r in resumes {
                    match self.resume_session_in(r.session, r.confirm, r.plan) {
                        Response::Spawned { .. } => {
                            if let Some(text) = r.prompt {
                                let words = Parked { text, brief: r.brief, title: r.brief };
                                self.park(r.session, r.ticket, words, Ack::PROMPT);
                            }
                        }
                        Response::Err { message } => {
                            eprintln!("mesimon: parked wake replay failed: {message}");
                            self.feed.board("daemon", "resume_replay_failed", Some(ticket));
                        }
                        _ => {}
                    }
                }
                self.note_stage("spawn_replay", replay.elapsed());
            }
            Err((stage, message)) => {
                self.pending_spawns.retain(|s| s.ticket != ticket);
                self.pending_resumes.retain(|r| r.ticket != ticket);
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
        // The flags take the worker road (T-430). A binding just cut is
        // trivially fresh — not merged, nothing ahead, no rebase owed — and
        // that is exactly what an absent entry reads as (`unwrap_or` on
        // every `wt_*` map; the snapshot's `repos` falls back to the
        // binding's own legs). Nothing on this turn reads the flags: the
        // lock and the card's mark read `status`. The synchronous road
        // was one `compute_repo_flags` round per census repo on the writer,
        // 12 rounds on a 12-leg workspace, while every keypress waited.
        let flags = Instant::now();
        self.queue_worktree_flags();
        self.note_stage("queue_worktree_flags", flags.elapsed());
        self.persist_and_notify();
    }

    /// Take the worktree lock when a session starts in it (12 §12.3.2).
    fn lock_worktree(&mut self, ticket: ulid::Ulid, session: uuid::Uuid) {
        let Some(b) = self.worktrees.get_mut(&ticket) else { return };
        if b.status != BindingStatus::Attached || b.locked {
            return;
        }
        let key = self.board.ticket(ticket).map(|t| t.short_key.clone()).unwrap_or_default();
        // Every leg (T-368): a lock taken in each repository the branch is
        // checked out in; one refused releases the ones taken.
        let legs = b.legs(&self.paths.repo_root, "");
        let mut taken = Vec::new();
        for leg in &legs {
            if worktree::lock(&leg.repo, &leg.path, &key, session, std::process::id()).is_ok() {
                taken.push(leg);
            } else {
                for done in taken {
                    let _ = worktree::unlock(&done.repo, &done.path);
                }
                return;
            }
        }
        if let Some(b) = self.worktrees.get_mut(&ticket) {
            b.locked = true;
        }
        self.persist_worktrees();
    }

    /// The bindings as `compute_repo_flags` wants them: one query per
    /// repository a leg lives in, holding every ticket's leg there (T-368).
    /// A single-repo board is one query on the root — the same `2 + n`
    /// forks it always was. Empty when there is nothing to judge, which is
    /// also when the flags are cleared rather than sampled. The upstream of
    /// a leg is asked here where the cache has no answer, on the writer,
    /// which is what the tick's road avoids by passing `None` and asking on
    /// the worker. A single-repo leg rides with an empty base when the
    /// cache is empty — the worker fills it from `default_branch` — where a
    /// workspace leg without one is a broken record and is skipped: with
    /// the leg skipped instead (before T-430) a fetch, which empties the
    /// cache, left the worker road with no query and no sample, and the
    /// flags stood still until a synchronous road happened to run. `only`
    /// narrows it to one ticket's legs: the crown's turn probe (T-469).
    fn wt_queries(
        &mut self,
        ask_upstream: bool,
        only: Option<ulid::Ulid>,
    ) -> Vec<worktree::RepoQuery> {
        let base = self.base_branch.clone().unwrap_or_default();
        let mut queries: Vec<worktree::RepoQuery> = Vec::new();
        let tickets: Vec<ulid::Ulid> = self
            .worktrees
            .keys()
            .copied()
            .filter(|t| only.is_none_or(|o| o == *t) && !self.tearing_down.contains(t))
            .collect();
        for t in tickets {
            let b = &self.worktrees[&t];
            if b.branch.is_empty() {
                continue;
            }
            let branch = b.branch.clone();
            let workspace = b.is_workspace();
            for leg in b.legs(&self.paths.repo_root, &base) {
                if leg.base.is_empty() && workspace {
                    continue;
                }
                let seen = self
                    .wt_repos
                    .get(&t)
                    .and_then(|legs| legs.iter().find(|l| l.name == leg.name))
                    .and_then(|l| l.content.clone());
                let input = worktree::FlagInput {
                    ticket: t,
                    branch: branch.clone(),
                    base_oid: leg.base_oid.clone(),
                    seen,
                };
                if let Some(q) =
                    queries.iter_mut().find(|q| q.name == leg.name && q.base == leg.base)
                {
                    q.inputs.push(input);
                    continue;
                }
                let upstream = if ask_upstream {
                    self.upstream_ref(&leg.name, &leg.repo, &leg.base)
                } else {
                    self.upstreams.get(&leg.name).cloned().flatten()
                };
                queries.push(worktree::RepoQuery {
                    name: leg.name.clone(),
                    repo: leg.repo.clone(),
                    base: leg.base.clone(),
                    upstream,
                    inputs: vec![input],
                });
            }
        }
        queries
    }

    /// merged/ahead/needs-rebase/conflict flags, NOW, on the writer thread —
    /// for the roads that must read them fresh in the same turn: startup,
    /// a merge just made, a binding torn down. A binding just attached is
    /// not one of them (T-430): fresh is what an absent entry reads as. The tick
    /// never takes this road (T-216): with thirteen bindings it was 53 git
    /// forks, ~0.5 s, every 10 s, and every keypress in that window waited
    /// on it — it asks `queue_worktree_flags` instead. `2 + n` forks per
    /// repository since the same change (`worktree::compute_flags`).
    fn refresh_worktree_flags(&mut self) {
        self.wt_gen = self.wt_gen.wrapping_add(1);
        if self.worktrees.is_empty() {
            self.wt_agg.clear();
            self.wt_repos.clear();
            self.wt_conflicts.clear();
            self.wt_fresh.clear();
            return;
        }
        if self.base_branch.is_none() {
            self.base_branch = worktree::default_branch(&self.paths.repo_root).ok();
        }
        if self.base_branch.is_none() {
            return;
        }
        let queries = self.wt_queries(true, None);
        let samples = worktree::compute_repo_flags(&queries);
        self.wt_seq = self.wt_seq.wrapping_add(1);
        // The callers of this road broadcast on their own terms.
        let _ = self.absorb_worktree_flags(samples, self.wt_seq, None);
    }

    /// The tick's road: the same sample on a worker, landing as
    /// `Msg::WorktreeFlags`. One at a time — a sample still out when the
    /// next bucket comes round is simply the one that will land. The base
    /// branch is resolved on the worker too when the cache is empty (a fetch
    /// empties it), so its own forks leave the writer as well.
    fn queue_worktree_flags(&mut self) {
        if self.wt_inflight || self.worktrees.is_empty() {
            return;
        }
        self.wt_inflight = true;
        self.wt_seq = self.wt_seq.wrapping_add(1);
        self.wt_inflight_seq = self.wt_seq;
        let gen = self.wt_gen;
        let repo = self.paths.repo_root.clone();
        let base = self.base_branch.clone();
        let cached: HashMap<String, Option<String>> = self.upstreams.clone();
        // Single-repo legs need the base to make a query at all; with the
        // cache empty the worker resolves it and the queries are built here
        // against that answer once it lands. A workspace leg carries its
        // own base and is queried either way.
        let mut queries = self.wt_queries(false, None);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let base = match base.or_else(|| worktree::default_branch(&repo).ok()) {
                Some(b) => b,
                None => {
                    let _ = tx.send(Msg::WorktreeFlags(gen, Vec::new()));
                    return;
                }
            };
            for q in &mut queries {
                if q.base.is_empty() {
                    q.base = base.clone();
                }
                // The upstream ref leaves the writer thread with the base,
                // and on the same terms: asked here when the cache has no
                // answer yet.
                if !cached.contains_key(&q.name) {
                    q.upstream = worktree::upstream_base(&q.repo, &q.base);
                }
            }
            let samples = worktree::compute_repo_flags(&queries);
            let _ = tx.send(Msg::WorktreeFlags(gen, samples));
        });
    }

    /// A worker's sample landed. Stale (a synchronous refresh ran since it
    /// started) means dropped: the flags on hand are newer than it. Fresh
    /// means absorbed, then the train's pass on it — exactly what the tick
    /// did in one turn before the sample left the writer thread.
    fn on_worktree_flags(&mut self, gen: u64, samples: Vec<worktree::RepoSample>) {
        self.wt_inflight = false;
        if gen != self.wt_gen || samples.is_empty() {
            return;
        }
        if self.base_branch.is_none() {
            if let Some(root) = samples.iter().find(|s| s.name.is_empty()) {
                self.base_branch = Some(root.base.clone());
            }
        }
        for s in &samples {
            self.upstreams.entry(s.name.clone()).or_insert_with(|| {
                // What the worker resolved is what the cache learns; the
                // upstream it used is not on the sample, so it is asked once
                // more here — one fork per leg name, once per fetch.
                let repo = if s.name.is_empty() {
                    self.paths.repo_root.clone()
                } else {
                    self.paths.repo_root.join(&s.name)
                };
                worktree::upstream_base(&repo, &s.base)
            });
        }
        let changed = self.absorb_worktree_flags(samples, self.wt_inflight_seq, None);
        // A merge that lands after the archive (a PR squashed later, a
        // `git merge` by hand) is one only the sample can see (T-481).
        self.reclaim_archived();
        let acted = self.train_pass();
        if acted {
            self.persist_sessions();
        }
        // Turns waiting on this look are judged on it (T-678), after the
        // train's pass, so a merge or a rebase ask it just made holds them.
        let settled = self.settle_looks();
        // A merge made somewhere else — a squash on a forge, a `git pull` in
        // another terminal — moves no session and fires no hook, so the
        // sample's own delta is the only thing that can tell the board.
        if changed || acted || settled {
            self.broadcast();
        }
    }

    /// Take a sample's answers, for the bindings still here — per leg, then
    /// folded into the ticket's one answer (`worktree::aggregate`, T-368) —
    /// and release the lock of any attached binding whose last session is
    /// gone (the one git fork left on this road, and a rare one).
    ///
    /// `seq` is the number the sample started under: a ticket already read
    /// by a newer one is left as it is (T-678). `only` is a ticket's own
    /// look, which queried that ticket's repositories alone, so of the
    /// branches checked out twice it can speak for that ticket's branch
    /// and no other.
    fn absorb_worktree_flags(
        &mut self,
        samples: Vec<worktree::RepoSample>,
        seq: u64,
        only: Option<ulid::Ulid>,
    ) -> bool {
        // Whether any of it is NEWS — what the board would draw differently.
        // The tick's road broadcasts on that and nothing else: a fetch that
        // lands a merge moves no session and fires no hook, so without this
        // the mark waited for the next thing to happen (T-267).
        let mut conflicts: Vec<String> = match only.and_then(|t| self.worktrees.get(&t)) {
            Some(b) => {
                let mine = b.branch.clone();
                let mut kept: Vec<String> =
                    self.wt_conflicts.iter().filter(|c| **c != mine).cloned().collect();
                if samples.iter().any(|s| s.flags.conflicts.contains(&mine)) {
                    kept.push(mine);
                }
                kept
            }
            None if only.is_some() => self.wt_conflicts.clone(),
            None => Vec::new(),
        };
        if only.is_none() {
            for s in &samples {
                for c in &s.flags.conflicts {
                    if !conflicts.contains(c) {
                        conflicts.push(c.clone());
                    }
                }
            }
        }
        let mut changed = self.wt_conflicts != conflicts;
        self.wt_conflicts = conflicts;
        // Per ticket, its legs in binding order.
        let base = self.base_branch.clone().unwrap_or_default();
        let tickets: Vec<ulid::Ulid> = self
            .worktrees
            .keys()
            .copied()
            .filter(|t| only.is_none_or(|o| o == *t))
            .filter(|t| self.wt_fresh.get(t).is_none_or(|f| *f <= seq))
            .collect();
        for t in tickets {
            let b = &self.worktrees[&t];
            if b.branch.is_empty() {
                continue;
            }
            let branch = b.branch.clone();
            let mut legs: Vec<worktree::RepoFlags> = Vec::new();
            for leg in b.legs(&self.paths.repo_root, &base) {
                let Some(s) = samples.iter().find(|s| s.name == leg.name && s.base == leg.base)
                else {
                    // Not sampled this pass: carry the old leg forward.
                    if let Some(old) = self
                        .wt_repos
                        .get(&t)
                        .and_then(|old| old.iter().find(|l| l.name == leg.name))
                    {
                        legs.push(old.clone());
                    }
                    continue;
                };
                let Some(f) = s.flags.flags.iter().find(|f| f.ticket == t) else { continue };
                // The memo is the sampler's own working note, never the
                // board's.
                legs.push(worktree::RepoFlags::of(&leg, s, f, &branch));
            }
            if legs.is_empty() {
                continue;
            }
            let a = worktree::aggregate(&legs);
            let was = self.wt_agg.get(&t).map(|w| w.merged);
            // Unmerged to merged is a landing, whoever made it (T-527). A
            // first reading is one only for a branch the crown was already
            // told of, or whose delivery is held for the train (T-554): the
            // first sample comes a slow bucket after the cut, and a restart,
            // which forgets what the crown heard, must not re-hear every
            // branch merged before it.
            let owed = self.crown_heard.get(&t).is_some_and(|h| h.awaits_merge());
            if a.merged
                && self.board.crown.is_some()
                && (was == Some(false) || (was.is_none() && owed))
            {
                self.crown_landed.push(t);
            }
            changed |= self.wt_agg.get(&t) != Some(&a);
            self.wt_agg.insert(t, a);
            let same_legs = self.wt_repos.get(&t).is_some_and(|old| {
                old.len() == legs.len()
                    && old.iter().zip(&legs).all(|(o, n)| {
                        (o.ahead, o.merged, o.needs_rebase, o.conflict)
                            == (n.ahead, n.merged, n.needs_rebase, n.conflict)
                    })
            });
            changed |= !same_legs;
            self.wt_repos.insert(t, legs);
            self.wt_fresh.insert(t, seq);
        }
        let tickets: Vec<ulid::Ulid> = self.worktrees.keys().copied().collect();
        for tid in tickets {
            let (locked, attached) = {
                let b = &self.worktrees[&tid];
                (b.locked, b.status == BindingStatus::Attached)
            };
            // Release the lock once the last session on the ticket is gone.
            if locked && attached {
                let live = self
                    .board
                    .sessions
                    .iter()
                    .any(|s| s.ticket == tid && (s.state.is_live() || s.codex_stopping));
                if !live {
                    let legs = self.legs_of(tid);
                    let released =
                        legs.iter().all(|leg| worktree::unlock(&leg.repo, &leg.path).is_ok());
                    if released {
                        if let Some(b) = self.worktrees.get_mut(&tid) {
                            b.locked = false;
                        }
                    }
                }
            }
        }
        changed
    }

    fn kill_session(&mut self, id: uuid::Uuid) -> Response {
        let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) else {
            return Response::Err { message: "no such session".into() };
        };
        let owned = !rec.observe_only();
        let reap = (owned && rec.holds_process()).then(|| rec.sid16());
        // Kill on a live session ends the process; the conversation survives
        // and its corpse stays on the ticket rail. Kill on an already-dead
        // record is the rail's dismissal gesture — the one exit the rail hides.
        let reason = if rec.state.is_live() { ExitReason::Killed } else { ExitReason::Dismissed };
        rec.state = SessionState::Exited { reason };
        if rec.kind == SessionKind::Codex && reap.is_some() {
            rec.codex_stopping = true;
            rec.observation_hold = true;
        }
        rec.pending_submit = false;
        rec.pending_prefill = false;
        rec.codex_submit_sent = false;
        self.owed.remove(&id);
        self.codex_input_due.remove(&id);
        self.codex_ready.remove(&id);
        rec.waiting_since = None;
        rec.detail = None;
        let (id, state, ticket) = (rec.id, rec.state.clone(), rec.ticket);
        self.machines.insert(id, Machine::new(state, now_ms()));
        if let Some(sid) = reap {
            let _ = self.backend.signal_session(&sid);
            self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        }
        // The user ended the session an ask was waiting for.
        self.forget_queued(ticket, "queued_ask_dropped", "local", "board");
        self.persist_and_notify();
        Response::Ok
    }

    /// 19 §4 tier 2: mint an observe-only record for a discovered foreign
    /// session — no process, no tmux, no hooks. Tier-0 state comes from the
    /// tail poller; the census preview seeds the card detail.
    fn attach_external(
        &mut self,
        by: &Principal,
        selector: uuid::Uuid,
        ticket: Option<ulid::Ulid>,
    ) -> std::result::Result<uuid::Uuid, String> {
        if let Some(t) = ticket {
            if self.board.ticket(t).is_none() {
                return Err("no such ticket".into());
            }
            // An accepted start/resume owns the seat before its pane exists.
            // Adoption must not displace it while the worktree is provisioning.
            if self
                .pending_spawns
                .iter()
                .any(|pending| pending.ticket == t && pending.kind.is_agent())
                || self.pending_resumes.iter().any(|pending| pending.ticket == t)
            {
                return Err(
                    "an agent start or resume is already provisioning on this ticket".into()
                );
            }
        }
        let item = self.external.iter().find(|item| item.id == selector).cloned();
        let provider = item.as_ref().map(|item| item.provider).unwrap_or(AgentProvider::ClaudeCode);
        let identity = item
            .as_ref()
            .map(|item| item.conversation_id.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| selector.to_string());
        let kind = provider.session_kind();
        let adapter = crate::agents::adapter(kind).expect("provider is an agent");
        let matches_identity = |record: &SessionRecord| {
            record.kind == kind && adapter.conversation_key(record).as_ref() == Some(&identity)
        };
        if self
            .board
            .sessions
            .iter()
            .any(|record| matches_identity(record) && record.holds_agent_seat())
        {
            return Err("session already on the board".into());
        }
        if ticket.is_some_and(|ticket| self.board.live_agent(ticket).is_some()) {
            return Err("ticket already has a live agent session".into());
        }
        if let Some(id) = self
            .board
            .sessions
            .iter()
            .filter(|record| matches_identity(record) && !record.state.is_live())
            .max_by_key(|record| record.state_changed_at.unwrap_or(0))
            .map(|record| record.id)
        {
            if let Some(ticket) = ticket {
                if let Some(record) = self.board.sessions.iter_mut().find(|record| record.id == id)
                {
                    record.ticket = ticket;
                }
            }
            self.external.retain(|item| item.id != selector);
            return Ok(id);
        }
        let Some(item) = item else {
            return Err("unknown external session — reopen the drawer to rescan".into());
        };
        self.external.retain(|item| item.id != selector);
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
                    .unwrap_or_else(|| item.id.to_string()[..8].to_string());
                let title: String = title.chars().take(48).collect();
                self.mint_full(by, Mint::bare(column, title, None))?
            }
        };
        let id = uuid::Uuid::new_v4();
        let mut rec = SessionRecord::new(
            id,
            kind,
            ticket,
            vec![], // no process of ours — the discriminator the tail poller keys on
            item.cwd.clone(),
            SessionState::unknown(),
        );
        rec.provenance = Provenance::Adopted;
        match provider {
            AgentProvider::ClaudeCode => rec.claude_session_id = identity.parse().ok(),
            AgentProvider::Codex => rec.codex_thread_id = Some(identity),
        }
        rec.transcript_path = Some(item.transcript_path.clone());
        rec.confidence = Confidence::Low;
        rec.state_changed_at = Some(now_ms());
        rec.detail = item.preview.clone();
        self.machines.insert(id, Machine::restore(rec.state.clone(), Confidence::Low, now_ms()));
        self.board.sessions.push(rec);
        Ok(id)
    }

    /// How far up the tool ladder a claude on `ticket` reaches (T-117): the
    /// board's switch off is `Off`, else its column's `agent_tools` as it
    /// stands NOW — read at spawn for what the shim lists and at every call
    /// for what the daemon admits, so a hand move to a `read` column narrows
    /// a live session and a move back widens it. A ticket whose column is
    /// gone (a hand edit) reads `Full`, what a new column gets.
    fn agent_tools_for(&self, ticket: ulid::Ulid) -> AgentTools {
        if !self.board.mcp_tools {
            return AgentTools::Off;
        }
        self.board
            .ticket(ticket)
            .and_then(|t| self.board.column(&t.column))
            .map(|c| c.settings.agent_tools)
            .unwrap_or_default()
    }

    fn launch_context<'a>(
        &'a self,
        id: uuid::Uuid,
        ticket: ulid::Ulid,
        kind: SessionKind,
        cwd: &'a std::path::Path,
        plan: bool,
        pick: &bridge::Pick,
    ) -> LaunchContext<'a> {
        LaunchContext {
            paths: &self.paths,
            cwd,
            session: id,
            tools: self.agent_tools_for(ticket),
            brief: self.board.system_prompt,
            plan,
            tier: self.tier_book().launch(ticket, kind),
            column: self
                .board
                .ticket(ticket)
                .and_then(|t| self.board.column(&t.column))
                .map(|c| c.settings.clone())
                .unwrap_or_default(),
            // The laid mod when the launch's road is the mod (T-574);
            // T-573's research seam names another folder over it.
            road: if pick.folder.is_some() {
                mesimon_core::road::Road::Mod
            } else {
                mesimon_core::road::Road::Hooks
            },
            mod_dir: mod_dir().or_else(|| pick.folder.clone()),
            hook_set: pick.hook_set,
        }
    }

    fn resume_argv(
        &self,
        rec: &SessionRecord,
        plan: bool,
        pick: &bridge::Pick,
    ) -> std::result::Result<LaunchSpec, String> {
        let context = self.launch_context(
            rec.id,
            rec.ticket,
            rec.kind,
            std::path::Path::new(&rec.cwd),
            plan,
            pick,
        );
        crate::agents::adapter(rec.kind)
            .ok_or_else(|| "shells do not have agent conversations".to_string())?
            .resume(&context, rec)
    }

    /// Board constraints shared by wake and automatic inactivity parking.
    fn resume_board_guard(&self, rec: &SessionRecord) -> Option<String> {
        if self
            .pending_spawns
            .iter()
            .any(|pending| pending.ticket == rec.ticket && pending.kind.is_agent())
            || self
                .pending_resumes
                .iter()
                .any(|pending| pending.ticket == rec.ticket && pending.session != rec.id)
        {
            return Some(
                "another agent start or resume is already provisioning on this ticket".into(),
            );
        }
        if self.board.ticket(rec.ticket).is_some_and(|t| t.is_archived()) {
            return Some(TICKET_ARCHIVED.into());
        }
        if self.board.sessions.iter().any(|other| {
            other.id != rec.id && other.ticket == rec.ticket && other.holds_agent_seat()
        }) {
            return Some("ticket already has a live agent session — focus it instead".into());
        }
        None
    }

    /// A provider supplies conversation identity and external ownership;
    /// the board enforces its one-writer policy across records.
    fn resume_guard(&self, rec: &SessionRecord, confirm: bool) -> Option<Response> {
        let adapter = crate::agents::adapter(rec.kind)?;
        let identity = adapter.conversation_key(rec);
        if self.board.sessions.iter().any(|other| {
            other.id != rec.id
                && other.kind == rec.kind
                && other.holds_process()
                && identity.is_some()
                && adapter.conversation_key(other) == identity
        }) {
            return Some(Response::Err {
                message: "conversation already running under Mesimon".into(),
            });
        }
        if !confirm {
            if let Some(owner) = adapter.external_owner(rec) {
                // The daemon's OWN previous pane, still going down after the
                // sleep's SIGTERM, keeps its pid file live for as long as its
                // exit hooks run — and `x x` (sleep, wake) lands inside that.
                // That process is not "elsewhere": the wake's kill-session
                // finishes what the sleep began, and its exit frames are
                // stragglers `straggler_death` drops (T-381). Read as
                // elsewhere, the refusal made the user confirm a kill of
                // their own session on every visit to the ticket.
                let own = owner.pid.is_some() && owner.pid == self.own_pane_pid(rec);
                if !own {
                    return Some(Response::NeedsConfirm {
                        message: format!("running elsewhere ({owner}) — resuming would interleave transcripts; resume again to override"),
                    });
                }
            }
        }
        None
    }

    /// This only establishes absence of known owners. An explicit person must
    /// separately acknowledge descendants that the crashed runtime could not audit.
    fn unverified_cleanup_resume_eligible(
        &self,
        rec: &SessionRecord,
    ) -> Result<(u64, crate::agents::codex::RecoveryLaunchTarget), String> {
        let generation =
            rec.codex_generation.ok_or_else(|| "Codex cleanup identity is missing".to_string())?;
        if rec.kind != SessionKind::Codex || rec.argv.is_empty() {
            return Err("Codex cleanup recovery requires an owned runtime".into());
        }
        if !std::path::Path::new(&rec.cwd).is_dir() {
            return Err("Codex cleanup is unverified and its checkout is missing; recovery cannot recreate it while unknown child processes may remain".into());
        }
        let panes = self
            .backend
            .panes()
            .map_err(|e| format!("Codex cleanup: cannot verify private pane absence: {e}"))?;
        if panes.as_deref().is_some_and(|p| {
            p.iter().any(|pane| pane.session_name == rec.sid16() && !pane.pane_dead)
        }) {
            return Err("Codex is still stopping; its native pane remains live".into());
        }
        // `None` is a server that did not answer. The stale socket left by a
        // dead server is safe only after a bounded endpoint probe positively
        // excludes its listener. A server that answered with no pane has
        // said the pane is gone — and a `-D` server lives on between its
        // sessions (T-690), so that answer is an everyday one, not a dead
        // server's silence.
        if panes.is_none() {
            crate::agents::codex::recovery_endpoint_absent(&self.paths.tmux_sock())?;
        }
        let target = crate::agents::codex::recovery_launch_target(&self.paths, rec)?;
        let mut owner_record = rec.clone();
        if let crate::agents::codex::RecoveryLaunchTarget::Exact(identity) = &target {
            owner_record.codex_thread_id = Some(identity.clone());
        }
        if let Some(owner) = crate::agents::adapter(rec.kind)
            .and_then(|adapter| adapter.external_owner(&owner_record))
        {
            return Err(format!(
                "Codex is still stopping; known conversation owner remains ({owner})"
            ));
        }
        crate::agents::codex::recovery_owner_absent(&self.paths, rec)?;
        Ok((generation, target))
    }

    /// Takeover / wake: spawn `claude --resume` under this record's sid16.
    fn resume_session(&mut self, id: uuid::Uuid, confirm: bool) -> Response {
        self.resume_session_with_cleanup_ack(id, confirm, false, false)
    }

    /// A wake in plan mode (T-434): `--permission-mode plan` on this launch,
    /// the column's word again on the next.
    /// A resume on a road no person's second press can follow — a wake, a
    /// prompt, the crown, a replay — where a typed offer is a plain refusal.
    fn resume_session_in(&mut self, id: uuid::Uuid, confirm: bool, plan: bool) -> Response {
        match self.resume_session_with_cleanup_ack(id, confirm, false, plan) {
            Response::NeedsConfirm { message } => Response::Err { message },
            resp => resp,
        }
    }

    fn resume_session_with_cleanup_ack(
        &mut self,
        id: uuid::Uuid,
        confirm: bool,
        human_resume: bool,
        plan: bool,
    ) -> Response {
        let Some(mut rec) = self.board.sessions.iter().find(|s| s.id == id).cloned() else {
            return Response::Err { message: "no such session".into() };
        };
        if !rec.kind.is_agent() {
            return Response::Err { message: "only agent sessions resume".into() };
        }
        let mut startup_retry = false;
        let cleanup_generation = if rec.codex_stopping {
            if !human_resume {
                return Response::Err {
                    message: "Codex is still stopping; automatic wake cannot acknowledge unverified cleanup".into(),
                };
            }
            let eligible = self.unverified_cleanup_resume_eligible(&rec);
            let (generation, target) = match eligible {
                Ok(target) => target,
                Err(message) => {
                    self.cleanup_resume_offers.remove(&id);
                    return Response::Err { message };
                }
            };
            if !confirm || self.cleanup_resume_offers.get(&id) != Some(&generation) {
                self.cleanup_resume_offers.insert(id, generation);
                return Response::NeedsConfirm {
                    message: format!("Codex cleanup is unverified: the known pane and runtime are absent, but unknown child processes may remain. Check the checkout and resume again to acknowledge this risk and {}", match target {
                        crate::agents::codex::RecoveryLaunchTarget::Exact(_) => "resume the exact conversation",
                        crate::agents::codex::RecoveryLaunchTarget::RetryStartup => "retry startup; recorded evidence proves no conversation selection was forwarded",
                    }),
                };
            }
            match target {
                crate::agents::codex::RecoveryLaunchTarget::Exact(identity) => {
                    rec.codex_thread_id = Some(identity)
                }
                crate::agents::codex::RecoveryLaunchTarget::RetryStartup => startup_retry = true,
            }
            Some(generation)
        } else {
            self.cleanup_resume_offers.remove(&id);
            None
        };
        // Git creates the worktree directory before post-checkout finishes. Keep
        // the accepted resume behind that transaction even once its cwd exists.
        if self.pending_resumes.iter().any(|pending| pending.session == id) {
            return Response::Provisioning;
        }
        if let Some(message) = self.resume_board_guard(&rec) {
            return Response::Err { message };
        }
        if rec.state.has_pane() && !matches!(rec.state, SessionState::Unknown { .. }) {
            // Live states keep their pane; resuming over it would double-run.
            if !rec.argv.is_empty() {
                return Response::Err { message: "session is live — focus it instead".into() };
            }
        }
        if let Some(refusal) = self.resume_guard(&rec, confirm) {
            return refusal;
        }
        let adapter = crate::agents::adapter(rec.kind).expect("agent kind checked above");
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
        let fresh = (adapter.capabilities().resume
            == crate::agents::ResumePolicy::FreshWhenHistoryMissing
            && adapter.history_missing(&rec))
        .then(uuid::Uuid::new_v4);
        // Preserve the unacknowledged old generation before adapter preparation
        // writes the configuration for its replacement. This is evidence of a
        // human risk acceptance, never a fabricated cleanup acknowledgement.
        let cleanup_evidence = if cleanup_generation.is_some() {
            if let Some(message) = self.spawn_gate() {
                return Response::Err { message };
            }
            match crate::agents::codex::retain_unverified_cleanup(&self.paths, &rec) {
                Ok(path) => Some(path),
                Err(message) => return Response::Err { message },
            }
        } else {
            None
        };
        // A wake re-decides the road (T-574), as it re-reads the tier.
        let pick = self.launch_road(rec.kind, rec.id);
        let road = pick.road;
        let spec = if startup_retry {
            match adapter.start(
                &self.launch_context(
                    rec.id,
                    rec.ticket,
                    rec.kind,
                    std::path::Path::new(&rec.cwd),
                    plan,
                    &pick,
                ),
                &rec.id.to_string(),
            ) {
                Ok(spec) => spec,
                Err(message) => return Response::Err { message },
            }
        } else {
            match fresh {
                Some(new_id) => {
                    match adapter.start(
                        &self.launch_context(
                            rec.id,
                            rec.ticket,
                            rec.kind,
                            std::path::Path::new(&rec.cwd),
                            plan,
                            &pick,
                        ),
                        &new_id.to_string(),
                    ) {
                        Ok(a) => a,
                        Err(message) => return Response::Err { message },
                    }
                }
                None => match self.resume_argv(&rec, plan, &pick) {
                    Ok(a) => a,
                    Err(message) => return Response::Err { message },
                },
            }
        };
        let argv = spec.argv;
        let (sid, mut cwd, ticket) =
            (rec.sid16(), std::path::PathBuf::from(rec.cwd.clone()), rec.ticket);
        let strategy = self
            .board
            .ticket(ticket)
            .map(|t| t.workspace_strategy())
            .unwrap_or(mesimon_core::board::DEFAULT_WORKSPACE);
        // M4: never silently relocate an agent — a removed worktree/cwd is an
        // explicit refusal, not a fallback into the main checkout. The one
        // road that is not a relocation (T-278): a worktree ticket whose tree
        // the archive reclaimed gets it rebuilt the way a first spawn does —
        // the same branch where it survived, a fresh one off the base where
        // `branch -d` took it — and the wake is parked behind the build.
        if !cwd.is_dir() {
            if strategy != WorkspaceStrategy::Worktree {
                return Response::Err {
                    message: format!(
                        "session's directory is gone ({}) — cannot resume",
                        cwd.display()
                    ),
                };
            }
            match self.resolve_spawn_cwd(ticket) {
                Ok(Some(p)) => cwd = p,
                Ok(None) => {
                    if !self.pending_resumes.iter().any(|r| r.session == id) {
                        self.pending_resumes.push(PendingResume {
                            ticket,
                            session: id,
                            confirm,
                            prompt: None,
                            brief: false,
                            plan,
                        });
                    }
                    self.persist_and_notify();
                    return Response::Provisioning;
                }
                Err(message) => return Response::Err { message },
            }
        }
        if let Some(message) = self.spawn_gate() {
            return Response::Err { message };
        }
        self.reaping.remove(&sid); // a fresh pane must not meet a stale reap
        let _ = self.backend.kill_session(&sid); // clear any dead remain-on-exit pane
        let launch = self.launch(&argv, &self.launch_vars(ticket, &cwd, id, rec.kind, &pick));
        let pane = match self.backend.spawn(&sid, &cwd, &launch) {
            Ok(pane) => pane,
            Err(e) => {
                if let (Some(evidence), Some(prepared_generation)) =
                    (cleanup_evidence.as_deref(), spec.generation)
                {
                    if let Err(rollback) = crate::agents::codex::restore_unverified_cleanup(
                        &self.paths,
                        &rec,
                        evidence,
                        prepared_generation,
                    ) {
                        return Response::Err {
                        message: format!("resume spawn failed: {e}; original cleanup remains unverified and configuration rollback failed: {rollback}"),
                    };
                    }
                }
                return Response::Err { message: format!("resume spawn failed: {e}") };
            }
        };
        let now = now_ms();
        let resumed_thread = rec.codex_thread_id.clone();
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.pane_key = Some(pane);
            rec.argv = argv;
            rec.road = road;
            rec.native = pick.native();
            rec.codex_generation = spec.generation;
            rec.codex_plan_dialog_seen = false;
            rec.codex_plan_dismissed_turn = None;
            rec.codex_observed_seq = 0;
            rec.codex_pending_seq = None;
            rec.codex_stopping = false;
            if rec.kind == SessionKind::Codex {
                rec.codex_thread_id = resumed_thread;
                rec.observation_hold = true;
                rec.agent_preview_path =
                    Some(crate::agents::codex::preview_path(&self.paths, id).display().to_string());
            }
            rec.cwd = cwd.display().to_string();
            rec.state = SessionState::Spawning;
            rec.state_changed_at = Some(now);
            rec.waiting_since = None;
            rec.confidence = Confidence::High;
            // The new pane carries no prefill (resume restores the
            // conversation, and the prompt is already in it), so an Enter
            // owed by the old one is stale — never carry it across.
            rec.pending_submit = false;
            rec.codex_submit_sent = false;
            rec.pending_prefill = false;
            self.owed.remove(&id);
            if let Some(new_id) = fresh {
                // Point the record at the conversation it is actually hosting
                // now. The stale path would otherwise send the NEXT resume
                // back to the id that had no transcript — the dead end, one
                // wake later.
                rec.claude_session_id = Some(new_id);
                rec.transcript_path = None;
            }
        }
        self.cleanup_resume_offers.remove(&id);
        if let (Some(generation), Some(evidence)) = (cleanup_generation, cleanup_evidence) {
            self.feed.hook_event(
                &id.to_string(),
                "CleanupUnverifiedResume",
                Some(&format!(
                    "Local user acknowledged unknown child processes may remain from generation {generation}; retained evidence: {}",
                    evidence.display()
                )),
            );
        }
        self.recovery.remove(&id); // reset provider cursors for the new launch
        self.machines.insert(id, Machine::new(SessionState::Spawning, now));
        self.stamp_tier(id);
        let agent_on = self.board.sessions.iter().find(|s| s.id == id && s.kind.is_agent());
        if let Some(ticket) = agent_on.map(|s| s.ticket) {
            if self.picked_by_agent(ticket) {
                self.broadcast();
            }
        }
        Response::Spawned { id, fresh: fresh.is_some() || startup_retry }
    }

    /// Automatic sleep is deliberately narrower than a user's sleep gesture:
    /// a High-confidence idle Claude, timed from its settled state
    /// transition, by the rule that is due (`Board::idle_park_due`): the
    /// ticket's column's own `sleep_after_minutes` over any idle at its
    /// prompt (T-543), the only timer since T-610.
    fn park_inactive(&mut self, now: u64) -> bool {
        let minute = inactivity_minute_ms();
        let candidates: Vec<_> = self
            .board
            .sessions
            .iter()
            .filter_map(|rec| {
                let (rule, timeout) = self.board.idle_park_due(rec, now, minute)?;
                Some((rec, timeout, rule))
            })
            .filter(|&(rec, _, _)| {
                self.board.ticket(rec.ticket).is_some_and(|t| t.raised.is_none())
                    // A `!` command running in the composer (T-707) is the
                    // person at work; never park the pane under it.
                    && !self.foregrounds.contains_key(&rec.id)
                    && !self.pending_resumes.iter().any(|pending| pending.session == rec.id)
                    && !self.owed_on(rec.ticket)
                    && !self.queued.iter().any(|q| q.ticket == rec.ticket)
                    && self.machines.get(&rec.id).is_some_and(|m| {
                        let view = m.view();
                        view.state == rec.state
                            && view.pending.is_none()
                            && view.manual_compaction_prior.is_none()
                    })
            })
            .map(|(rec, timeout, rule)| (rec.id, timeout, rule))
            .collect();
        let mut changed = false;
        for (id, timeout, rule) in candidates {
            let by = Principal::Automation { rule: rule.into() };
            let rec =
                self.board.sessions.iter().find(|rec| rec.id == id).expect("candidate exists");
            if !matches!(
                authorize(&by, &Action::Mutate, &Resource::Session { id }),
                Decision::Allow
            ) || self.resume_board_guard(rec).is_some()
                || self.resume_guard(rec, false).is_some()
            {
                continue;
            }
            // A person attached to the pane and typing in it within the
            // timeout holds it awake (T-543): a column's minute is short
            // enough to land while someone reads the answer, and the sleep
            // would close the terminal under them. One `list-clients` fork,
            // only for a session already due; a client quiet past the
            // timeout is someone who walked away, and the park goes on.
            let quiet_secs = self.backend.client_quiet_secs(&rec.sid16()).ok().flatten();
            if quiet_secs.is_some_and(|secs| secs.saturating_mul(1000) < timeout) {
                continue;
            }
            let adapter = crate::agents::adapter(rec.kind).expect("Claude adapter");
            // Preserve the exact conversation, using the same history and identity
            // predicates as resume. A fresh-start fallback is not an automatic park.
            if adapter.history_missing(rec) || adapter.conversation_key(rec).is_none() {
                continue;
            }
            let ticket = rec.ticket;
            if self.sleep_one(id, false).is_ok() {
                self.feed.board("automation", rule, Some(ticket));
                changed = true;
            }
        }
        changed
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
        // The kind × state clause is core's (`quiet::sleep_eligible`), shared
        // with the TUI's pre-judgement; what follows is what only the daemon
        // can see.
        mesimon_core::quiet::sleep_eligible(rec.kind, &rec.state).map_err(str::to_owned)?;
        if enforce_floor && rec.kind == SessionKind::Codex && rec.observation_hold {
            return Err("Codex observation has not proved this session quiet".into());
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
        let (sid, transcript) = (rec.sid16(), rec.transcript_path.clone());

        // A shell's pane IS its record (T-366, the user: "sleep of an adopted
        // shell kills it, remove it from the records"). There is no
        // conversation to park and a woken one would be a different shell
        // wearing the same row, so `x` on a shell CLOSES it: the pane goes
        // through the same kill ladder as a parked agent's, and the record
        // goes with it — the rail, the archive gate and the worktree lock
        // (released by the reaper's pass once no live session remains) all
        // stop counting a shell that is gone.
        if rec.kind == SessionKind::Bash {
            let ticket = rec.ticket;
            self.board.sessions.retain(|s| s.id != id);
            self.machines.remove(&id);
            self.recovery.remove(&id);
            self.foregrounds.remove(&id);
            let _ = self.backend.signal_session(&sid);
            self.reaping.insert(sid, Instant::now() + REAP_GRACE);
            self.journal.line(&format!("shell closed: session {id} on ticket {ticket}"));
            return Ok(());
        }

        // B-A22: the conversation belongs to Claude's own store and this is
        // our snapshot of it — the same copy `park_on_exit` makes, now in the
        // same silence. It used to also assert the copy held a user+assistant
        // pair and, failing that, park the record with `resume may lose
        // context` in its detail: a warning the user asked for the removal of
        // (2026-09-07). It was wrong twice over. `resume_session` does not
        // read this copy — it replays argv with `--resume` against Claude's
        // OWN store — so a thin snapshot costs the wake nothing, and where
        // there is genuinely no conversation to resume the wake mints a fresh
        // one under a new uuid rather than losing anything. The other sleep
        // road never said it, so one gesture answered two ways.
        self.park_record(id, transcript.as_deref(), now);
        let _ = self.backend.signal_session(&sid);
        self.reaping.insert(sid, Instant::now() + REAP_GRACE);
        // The user parked the claude an ask was waiting for: a queued ask
        // needs an awake pane, and waking it later against their gesture is
        // not what they asked for.
        if let Some(t) = self.board.sessions.iter().find(|s| s.id == id).map(|s| s.ticket) {
            self.forget_queued(t, "queued_ask_dropped", "local", "board");
        }
        Ok(())
    }

    /// The park both sleep roads share: the conversation snapshotted (B-A22:
    /// it belongs to the agent's own store, and this is our copy), the
    /// record parked with its Codex input forgotten, and the machine
    /// re-minted `Sleeping` so the pane's death echo cannot undo it.
    fn park_record(&mut self, id: uuid::Uuid, transcript: Option<&str>, now: u64) {
        if let Some(t) = transcript {
            let dir = self.paths.transcripts_dir();
            if std::fs::create_dir_all(&dir).is_ok() {
                let _ = std::fs::copy(t, dir.join(format!("{id}.jsonl")));
            }
        }
        if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
            rec.state = SessionState::Sleeping;
            if rec.kind == SessionKind::Codex {
                rec.codex_stopping = true;
                rec.observation_hold = true;
                rec.pending_submit = false;
                rec.pending_prefill = false;
                rec.codex_submit_sent = false;
                rec.codex_pending_seq = None;
                self.owed.remove(&id);
                self.codex_input_due.remove(&id);
                self.codex_ready.remove(&id);
            }
            rec.confidence = Confidence::High;
            rec.waiting_since = None;
            rec.state_changed_at = Some(now);
            // Cleared, never merely left: a `RequiresAction` detail that
            // outlived its state would otherwise ride the parked record —
            // `apply_change` wipes it outside the attention states and this
            // road does not go through `apply_change`.
            rec.detail = None;
        }
        self.machines.insert(id, Machine::new(SessionState::Sleeping, now));
        self.recovery.remove(&id);
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
        if !rec.kind.is_agent()
            || rec.argv.is_empty()
            || !matches!(rec.state, SessionState::Exited { reason: ExitReason::UserQuit })
        {
            return false;
        }
        let adapter = crate::agents::adapter(rec.kind).expect("agent kind checked above");
        if adapter.history_missing(rec) || adapter.conversation_key(rec).is_none() {
            return false;
        }
        let (sid, transcript) = (rec.sid16(), rec.transcript_path.clone());

        let now = now_ms();
        self.park_record(id, transcript.as_deref(), now);
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
            return Response::Err { message: TICKET_ARCHIVED.into() };
        }
        match rec.kind {
            SessionKind::Claude | SessionKind::Codex => self.resume_session_in(id, false, false),
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
                let pane = match self.backend.spawn(&sid, &cwd, &launch) {
                    Ok(pane) => pane,
                    Err(e) => return Response::Err { message: format!("wake spawn failed: {e}") },
                };
                let now = now_ms();
                if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                    rec.pane_key = Some(pane);
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
        // still in play (2026-08-30 rescope; the columns' `reclaim`, T-117).
        let safe = self.reclaim_tickets();
        let candidates: Vec<uuid::Uuid> = self
            .board
            .sessions
            .iter()
            .filter(|r| safe.contains(&r.ticket))
            // Agents only: a shell's sleep is its close (T-366), and a bulk
            // gesture priced in freed memory must not silently end shells.
            .filter(|r| {
                matches!(
                    (r.kind, &r.state),
                    (SessionKind::Claude | SessionKind::Codex, SessionState::Idle { .. })
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
            // Removing the terminal must not kill the supervisor before it
            // proves its separate app-server and tool process group stopped.
            if self.board.sessions.iter().any(|s| s.sid16() == sid && s.codex_stopping) {
                continue;
            }
            self.reaping.remove(&sid);
            let _ = self.backend.kill_session(&sid);
        }
    }

    /// T-357: a Codex record whose ticket a person deleted, whose pane is
    /// gone and whose runtime never confirmed cleanup. `delete_ticket` keeps
    /// such a record on purpose — its `codex_stopping` is the evidence that a
    /// separate app-server may still own the checkout — and the snapshot loop
    /// releases it when the runtime reports `stopped`. A runtime that crashed
    /// before it could (the dogfood case: `No space left on device` at
    /// startup) never reports anything, and with the ticket gone no gesture
    /// on the board reaches the record: it held a laptop awake and a shared
    /// checkout "working" for two days. This runs the whole rung of positive
    /// evidence a human resume relies on — pane absent, tmux endpoint absent
    /// when tmux answers nothing, no live conversation owner, no listener on
    /// either runtime socket, no same-user process naming the runtime's
    /// config or sockets, and a complete inventory that saw this daemon —
    /// off the writer thread, and `on_codex_orphans_checked` drops the record
    /// only on a clean verdict. Lost evidence still never means done: a check
    /// that cannot prove absence refuses, and the record stays, re-checked
    /// every `CODEX_ORPHAN_RETRY`. The person's deletion of the ticket, past
    /// its undo window, is the acknowledgement a resume would have asked for.
    ///
    /// T-405 widened it past the deleted ticket, which was never the part
    /// that mattered. A record whose ticket still stands is reachable by
    /// gesture, but no gesture CLEARS the flag — `kill_session` sets it again
    /// on a corpse (the rail's dismissal), and only a resume or the runtime's
    /// own `stopped` ever lowers it. So a crashed runtime on a live ticket
    /// owns the shared checkout for good, and every queued ask on that board
    /// waits forever on a card that shows nothing working (dogfooded: seven
    /// such records on one board, six of them `Exited`, a column's worth of
    /// queued starts behind them). The evidence bar is unchanged and is
    /// already the harsher act's — it is what lets this DELETE an orphan — so
    /// a live ticket's record merely loses the flag and stays on its rail.
    fn sweep_codex_orphans(&mut self) {
        if self.codex_orphan_checking {
            return;
        }
        let now = Instant::now();
        let stale_at = now_ms().saturating_sub(codex_cleanup_stale_ms());
        let candidates: Vec<SessionRecord> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.kind == SessionKind::Codex
                    && !s.argv.is_empty()
                    && s.codex_stopping
                    && !s.state.has_pane()
                    // A deleted ticket inside its undo window is still the
                    // person's to bring back, record and all.
                    && !self.grace.contains_key(&s.ticket)
                    // A live ticket's cleanup gets the clock; a deleted
                    // ticket was already acknowledged by hand (T-405). No
                    // stamp is lost evidence, and lost evidence never means
                    // done — the record keeps its claim, as T-357 has it.
                    && (self.board.ticket(s.ticket).is_none()
                        || s.state_changed_at.is_some_and(|at| at <= stale_at))
                    && self.codex_orphan_due.get(&s.id).is_none_or(|(due, _)| *due <= now)
            })
            .cloned()
            .collect();
        if candidates.is_empty() {
            return;
        }
        for rec in &candidates {
            let said = self.codex_orphan_due.remove(&rec.id).and_then(|(_, why)| why);
            self.codex_orphan_due.insert(rec.id, (now + CODEX_ORPHAN_RETRY, said));
        }
        // Pane absence is read here: the backend belongs to the writer. A
        // server that answered with no pane is an answer (T-690: a `-D`
        // server lives between its sessions); only no server at all sends
        // the thread to the endpoint probe.
        let Ok(panes) = self.backend.panes() else { return };
        let tmux_answered_nothing = panes.is_none();
        let live: std::collections::HashSet<String> = panes
            .unwrap_or_default()
            .iter()
            .filter(|p| !p.pane_dead)
            .map(|p| p.session_name.clone())
            .collect();
        let tmux_sock = self.paths.tmux_sock();
        let paths = self.paths.clone();
        let tx = self.tx.clone();
        self.codex_orphan_checking = true;
        std::thread::spawn(move || {
            let results = candidates
                .into_iter()
                .map(|rec| {
                    let verdict = (|| {
                        if live.contains(&rec.sid16()) {
                            return Err("its native pane remains live".to_string());
                        }
                        if tmux_answered_nothing {
                            crate::agents::codex::recovery_endpoint_absent(&tmux_sock)?;
                        }
                        if let Some(owner) = crate::agents::adapter(rec.kind)
                            .and_then(|adapter| adapter.external_owner(&rec))
                        {
                            return Err(format!("known conversation owner remains ({owner})"));
                        }
                        crate::agents::codex::recovery_owner_absent(&paths, &rec)
                    })();
                    (rec.id, rec.codex_generation, verdict)
                })
                .collect();
            let _ = tx.send(Msg::CodexOrphansChecked(results));
        });
    }

    /// The verdicts of `sweep_codex_orphans`, re-judged on the writer: the
    /// record must still be what the check looked at — same generation,
    /// still stopping, still without a pane, and its ticket not inside the
    /// undo window — or the verdict is stale and dropped. A clean verdict
    /// drops a record whose ticket is gone and, where the ticket still
    /// stands (T-405), clears the cleanup flags and leaves the corpse on
    /// its rail: the checkout is released either way.
    fn on_codex_orphans_checked(
        &mut self,
        results: Vec<(uuid::Uuid, Option<u64>, std::result::Result<(), String>)>,
    ) {
        self.codex_orphan_checking = false;
        let mut dirty = false;
        for (id, generation, verdict) in results {
            let by = Principal::Automation { rule: "codex_orphan_cleanup".into() };
            if matches!(
                authorize(&by, &Action::Mutate, &Resource::Session { id }),
                Decision::Deny { .. }
            ) {
                continue;
            }
            let Some(rec) = self.board.sessions.iter().find(|s| s.id == id) else { continue };
            let still_stuck = rec.kind == SessionKind::Codex
                && rec.codex_stopping
                && !rec.state.has_pane()
                && rec.codex_generation == generation
                && !self.grace.contains_key(&rec.ticket);
            if !still_stuck {
                continue;
            }
            let orphan = self.board.ticket(rec.ticket).is_none();
            match verdict {
                Ok(()) if orphan => {
                    self.journal.line(&format!(
                        "codex orphan released: session {id} — ticket deleted, pane gone, no known runtime owner remains"
                    ));
                    self.feed.hook_event(
                        &id.to_string(),
                        "CodexOrphanReleased",
                        Some("no_known_owner"),
                    );
                    self.board.sessions.retain(|s| s.id != id);
                    self.machines.remove(&id);
                    self.codex_ready.remove(&id);
                    self.codex_native_ready.remove(&id);
                    self.codex_input_due.remove(&id);
                    self.owed.remove(&id);
                    self.cleanup_resume_offers.remove(&id);
                    self.codex_orphan_due.remove(&id);
                    dirty = true;
                }
                // The ticket still stands, so the record is its rail's to
                // show: only the cleanup flags go, and with them the claim
                // on the checkout (`quiet::is_working`) and, for a corpse,
                // the agent seat (`holds_agent_seat`).
                Ok(()) => {
                    self.journal.line(&format!(
                        "codex cleanup released: session {id} — pane gone, no known runtime owner remains"
                    ));
                    self.feed.hook_event(
                        &id.to_string(),
                        "CodexCleanupReleased",
                        Some("no_known_owner"),
                    );
                    if let Some(rec) = self.board.sessions.iter_mut().find(|s| s.id == id) {
                        rec.codex_stopping = false;
                        rec.observation_hold = false;
                    }
                    self.codex_ready.remove(&id);
                    self.codex_native_ready.remove(&id);
                    self.codex_input_due.remove(&id);
                    self.codex_orphan_due.remove(&id);
                    dirty = true;
                }
                Err(why) => {
                    if let Some((_, said)) = self.codex_orphan_due.get_mut(&id) {
                        if said.as_deref() != Some(why.as_str()) {
                            self.journal.line(&format!(
                                "codex orphan kept: session {id} — {why}; rechecked every {}s",
                                CODEX_ORPHAN_RETRY.as_secs()
                            ));
                            *said = Some(why);
                        }
                    }
                }
            }
        }
        if dirty {
            self.persist_and_notify();
        }
    }

    /// The token as it stands. A holder whose connection has gone is no
    /// holder: `on_client_gone` releases it, and this is the second guard —
    /// a `Weak` that no longer upgrades answers the same way.
    fn focus_held(&self) -> Option<Focus> {
        self.focus.as_ref().filter(|h| h.by.strong_count() > 0).map(|h| h.what)
    }

    fn focus_start(&mut self, session: uuid::Uuid, by: &Arc<Mutex<UnixStream>>) -> Response {
        if let Some(holder) = self.focus_held() {
            if holder != Focus::Session(session) {
                return Response::Err { message: "another session is focused".into() };
            }
        }
        let Some(rec) = self.board.sessions.iter().find(|s| s.id == session) else {
            return Response::Err { message: "no such session".into() };
        };
        if matches!(rec.state, SessionState::Sleeping) {
            return Response::Err { message: "asleep — wake it first".into() };
        }
        if rec.observe_only() {
            // Observe-only: no pane, no hooks, no input (19 §4 tier 2).
            return Response::Err { message: "external session — resume it to take over".into() };
        }
        self.focus = Some(FocusHold { what: Focus::Session(session), by: Arc::downgrade(by) });
        let sid16 = rec.sid16();
        let kind = rec.kind;
        let argv = self.backend.attach_argv(&sid16);
        // Breadcrumb leaf: the session's own name when the agent set one
        // (OSC-0 pane title; tmux reports the hostname when it never did),
        // else the kind word.
        let kind_word = kind.word();
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
    /// TUI header: ` mesimon › project !N › T-12 ticket title `. The needs-you
    /// count uses terminal yellow (the 16-colour attn of 06 §2.7, both
    /// flavors) popped out of the reversed bar; tmux chrome is backend-owned
    /// display, not the wire — the daemon still never styles a wire string.
    fn refresh_status_line(&mut self) {
        let Some(focus) = self.focus_held() else { return };
        // The terminal has no session on the board: the breadcrumb names
        // the ticket whose worktree it stands in, or nothing at the root.
        let (focused, terminal_ticket) = match focus {
            Focus::Session(id) => (Some(id), None),
            Focus::Terminal { ticket } => (None, ticket),
        };
        let repo = tmux_text(
            &self
                .paths
                .repo_root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            32,
        );
        let title = focused
            .and_then(|f| self.board.sessions.iter().find(|s| s.id == f))
            .map(|s| s.ticket)
            .or(terminal_ticket)
            .and_then(|t| self.board.ticket(t))
            .map(|t| ticket_crumb(&t.short_key, &t.title))
            .unwrap_or_default();
        let queue = mesimon_core::attention::attention_queue(&self.board);
        // Sessions in the attention set plus tickets a snooze woke lit —
        // the header chip's number, so the two never disagree.
        let needs_you = self.board.needs_you_count();
        // The user is looking at this pane: a `!1` that means "the session
        // you're inside" is noise, so the chip only shows when somewhere
        // ELSE needs them too.
        let only_self =
            needs_you == 1 && queue.first().is_some_and(|s| focused.is_some_and(|f| s.id == f));
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
        let line = status_left(&repo, &attn, &title, &self.focus_label);
        if self.last_status_left.as_deref() != Some(&line)
            && self.backend.set_status_left(&line).is_ok()
        {
            self.last_status_left = Some(line);
        }
    }

    /// `!` (T-273): the project's terminal — the user's own shell in the
    /// checkout root, or in a ticket's attached worktree, on the private tmux
    /// server, persistent. Not a `SessionRecord`: a session belongs to a
    /// ticket and every card, rail and quiet gate reads it as one, and the
    /// terminal is a place to stand, not work on a ticket. The gate session
    /// is the precedent — a named tmux session the board never lists. Alive
    /// means reused (a `git pull` in flight is never lost; the pane outlives
    /// the TUI and the daemon like every session does); a dead pane (`exit`
    /// typed, `remain-on-exit`) is killed and respawned.
    fn open_terminal(
        &mut self,
        ticket: Option<ulid::Ulid>,
        by: &Arc<Mutex<UnixStream>>,
    ) -> Response {
        let want = Focus::Terminal { ticket };
        if let Some(holder) = self.focus_held() {
            if holder != want {
                return Response::Err { message: "another session is focused".into() };
            }
        }
        // A worktree ticket lands in its worktree; any other ticket page is
        // the checkout's. A worktree still being made is refused in words,
        // never the root by surprise.
        let (cwd, vars) = match ticket {
            None => (self.paths.repo_root.clone(), Vec::new()),
            Some(id) => match self.worktrees.get(&id) {
                Some(b) if b.status == BindingStatus::Attached => {
                    (b.path.clone(), self.session_vars(id, &b.path))
                }
                Some(_) => return Response::Err { message: "worktree not ready yet".into() },
                None => (self.paths.repo_root.clone(), Vec::new()),
            },
        };
        let name = terminal_name(ticket);
        let alive = self
            .backend
            .snapshot()
            .map(|s| s.iter().any(|p| p.session_name == name && !p.pane_dead))
            .unwrap_or(false);
        if !alive {
            let _ = self.backend.kill_session(&name);
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let launch = self.launch(&[shell], &vars);
            if let Err(e) = self.backend.spawn(&name, &cwd, &launch) {
                return Response::Err { message: format!("terminal spawn failed: {e}") };
            }
        }
        // Listed now, not on the next poll: the ghost row (T-366) is on the
        // rail the moment the handover returns.
        if let std::collections::btree_map::Entry::Vacant(e) = self.terminals.entry(ticket) {
            e.insert(None);
            self.broadcast();
        }
        self.focus = Some(FocusHold { what: want, by: Arc::downgrade(by) });
        self.focus_label = "terminal".to_string();
        self.refresh_status_line();
        Response::Attach { argv: self.backend.attach_argv(&name) }
    }

    /// `Command::AdoptTerminal` (T-366): the ticket's `!` terminal becomes a
    /// shell session of the ticket. The pane is RENAMED to the new record's
    /// `sid16` — the shell keeps its history and its processes, and from
    /// here every session road (`pane_tail`, sleep, wake, the reaper, the
    /// pane-died hook, reconcile) finds it by that name with no other
    /// change. The record is what a `SpawnSession { Bash }` would have made:
    /// `[$SHELL]` in the same directory, `Running`, provenance `Spawned`
    /// (the pane WAS spawned by this daemon through `launch`; `Adopted` is
    /// the external drawer's word and badges `external`).
    fn adopt_terminal(&mut self, ticket: ulid::Ulid) -> Response {
        if let Err(message) = self.open_ticket(ticket) {
            return Response::Err { message: message.into() };
        }
        if self.team_content_only() {
            return Response::Err {
                message: "this board has no repository on this machine — open it where the code is"
                    .into(),
            };
        }
        // The user is inside it: adopting under their feet would change the
        // token's target while a client holds it.
        if self.focus_held() == Some(Focus::Terminal { ticket: Some(ticket) }) {
            return Response::Err {
                message: "terminal is attached — return to the board first".into(),
            };
        }
        // Exact name, alive, in a fresh snapshot — `rename-session -t`
        // prefix-matches on a miss, so the check is the guard.
        let name = terminal_name(Some(ticket));
        let alive = self
            .backend
            .snapshot()
            .map(|s| s.iter().any(|p| p.session_name == name && !p.pane_dead))
            .unwrap_or(false);
        if !alive {
            self.terminals.remove(&Some(ticket));
            return Response::Err {
                message: "no terminal to adopt — open one with ! first".into(),
            };
        }
        let cwd = match self.worktrees.get(&ticket) {
            Some(b) if b.status == BindingStatus::Attached => b.path.clone(),
            _ => self.paths.repo_root.clone(),
        };
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let id = uuid::Uuid::new_v4();
        let now = now_ms();
        let mut rec = SessionRecord::new(
            id,
            SessionKind::Bash,
            ticket,
            vec![shell],
            cwd.display().to_string(),
            SessionState::Running,
        );
        rec.state_changed_at = Some(now);
        if let Err(e) = self.backend.rename_session(&name, &rec.sid16()) {
            return Response::Err { message: format!("terminal adopt failed: {e}") };
        }
        rec.pane_key = self.backend.pane_key(&rec.sid16()).ok();
        if let Some(fg) = self.terminals.remove(&Some(ticket)).flatten() {
            self.foregrounds.insert(id, fg);
        }
        self.machines.insert(id, Machine::new(SessionState::Running, now));
        self.board.sessions.push(rec);
        self.lock_worktree(ticket, id);
        self.persist_and_notify();
        Response::Spawned { id, fresh: false }
    }

    fn gate_status(&mut self) -> Response {
        if self.paths.gate_file().is_file() {
            return Response::Gate { passed: true, attach_argv: None };
        }
        // Create (idempotently) the gate session the first-run ceremony attaches to (D20).
        let argv = vec![self.self_exe.display().to_string(), "detach-guide".into()];
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
/// The ticket's crumb on the focused status line (T-428): the key, bold, then
/// the title — `T-12 fix the thing`. The key is how a ticket is named in a
/// prompt, a note or a commit (the ticket page's chip carries it, T-233),
/// and inside a pane it is the one word that says which ticket this
/// session is on when three worktrees share a title word. The title alone
/// keeps its 48-char cap; the key rides outside it.
fn ticket_crumb(key: &str, title: &str) -> String {
    let title = tmux_text(title, 48);
    if title.is_empty() {
        format!("#[bold]{}#[nobold]", tmux_text(key, 16))
    } else {
        format!("#[bold]{}#[nobold] {title}", tmux_text(key, 16))
    }
}

/// The focused status line: ` mesimon › repo › T-12 title › leaf `. The
/// root terminal stands on no ticket (T-489), so its ticket crumb is empty
/// and drops out with its separator — ` mesimon › repo › terminal `, never
/// an empty `› ›` segment.
fn status_left(repo: &str, attn: &str, ticket: &str, leaf: &str) -> String {
    let ticket = if ticket.is_empty() { String::new() } else { format!(" › {ticket}") };
    format!(" mesimon › #[bold]{repo}#[nobold]{attn}{ticket} › #[bold]{leaf}#[nobold] ")
}

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
/// The slow bucket, in ticks — the worktree flags, the train, the CLAUDE.md
/// sample and the checkout's git sample: `RSS_TICKS` unless
/// `MESIMON_WT_REFRESH_TICKS` says otherwise — a test seam, since an e2e
/// cannot wait 10 s a step.
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

/// Ten-second sweep: a column's shortest timer is a minute (T-543), and a
/// sweep that judges nothing but the records in memory until one is due
/// costs nothing between. Shortened only by the subprocess test harness.
fn inactivity_park_ticks() -> u64 {
    std::env::var("MESIMON_INACTIVITY_PARK_TICKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(40)
}

/// Tests can compress a minute without changing the persisted setting's units.
fn inactivity_minute_ms() -> u64 {
    std::env::var("MESIMON_INACTIVITY_MINUTE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(60_000)
}

/// Test seam only — e2e cannot wait out the real 60 s floor.
fn codex_cleanup_stale_ms() -> u64 {
    std::env::var("MESIMON_CODEX_CLEANUP_STALE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(CODEX_CLEANUP_STALE_MS)
}

/// Test seam only — e2e cannot wait out the real 60 s floor.
fn sleep_min_age_ms() -> u64 {
    std::env::var("MESIMON_SLEEP_MIN_AGE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SLEEP_MIN_AGE_MS)
}

/// Test seam only — e2e cannot wait out the real 30 minutes a worker sits
/// idle with background tasks before the crown is told (T-599).
fn linger_ms() -> u64 {
    std::env::var("MESIMON_LINGER_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(crownwake::LINGER_MS)
}

/// Test seam only — e2e cannot wait out the real 30 s composer wait.
fn composer_wait_ms() -> u64 {
    std::env::var("MESIMON_COMPOSER_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(COMPOSER_WAIT_MS)
}

/// Test seam only — e2e cannot wait out the real 10 s bridge wait (T-575).
fn mod_bridge_wait_ms() -> u64 {
    std::env::var("MESIMON_MOD_BRIDGE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MOD_BRIDGE_WAIT_MS)
}

/// Test seam only — e2e cannot wait out the real hour.
/// The archive's refusal over unticked summary boxes (T-713), one wording
/// for every road: `a a`, the crown's `archive_ticket`.
fn open_boxes_refusal(open: usize) -> Option<String> {
    match open {
        0 => None,
        1 => Some("1 summary box is unticked — tick it or take it out first".into()),
        n => Some(format!("{n} summary boxes are unticked — tick them or take them out first")),
    }
}

fn archive_suggest_ms() -> u64 {
    std::env::var("MESIMON_ARCHIVE_SUGGEST_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ARCHIVE_SUGGEST_MS)
}

/// The process table — pid, parent, age and name — for `bash_mode_foreground`
/// (T-707). `ps axo` is BSD and procps alike; an unreadable table is empty,
/// never an error: a missing `ps` only means no `!` command is ever seen.
fn process_rows() -> Vec<mesimon_core::board::ProcRow> {
    let Ok(out) =
        std::process::Command::new("ps").args(["axo", "pid=,ppid=,etime=,comm="]).output()
    else {
        return Vec::new();
    };
    mesimon_core::board::parse_ps_rows(&String::from_utf8_lossy(&out.stdout))
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

/// `created_at`'s `@<unix secs>` stamp as epoch ms; None for anything else
/// (an unparsable stamp never feeds the archive suggestion).
fn created_at_ms(created_at: &str) -> Option<u64> {
    created_at.strip_prefix('@').and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000)
}

/// How much of the description rides `get_ticket`. The whole of it is one
/// `read_note` away; this keeps a routine call from carrying 32 KiB.
const AGENT_DESCRIPTION_MAX_BYTES: usize = 4096;
/// The longest plan `get_ticket.needs_you.plan` carries (T-582): the
/// projection's own bound on the tool input it was read from.
const AGENT_PLAN_MAX_BYTES: usize = 16 * 1024;

fn now_iso() -> String {
    // Seconds precision is enough for created_at; avoid a chrono dependency.
    format!("@{}", now_secs())
}

/// T-573's research seam: the plugin folder a Claude launch loads with
/// `--plugin-dir`, or nothing. Off by default; a spike, not a feature.
fn mod_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("MESIMON_MOD_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// The rig's seam (T-598), never a user's: `MESIMON_RIG_NO_FLAGS=1` launches
/// every command (panes and the road probe alike) with Claude Code's flags
/// service off, `DISABLE_GROWTHBOOK=1`, so each flag reads its built-in
/// default. It is the only local pin of `tengu_plugin_hooks_modules`, whose
/// default is on (measured on 2.1.288: the local override hooks are stubbed
/// out of the public build), and it pins every other flag to its default
/// with it. `ci/rig.py --flags-off` sets it, so the rig's acceptance can run
/// while the remote flag has mods off.
fn rig_no_flags() -> bool {
    std::env::var("MESIMON_RIG_NO_FLAGS").as_deref() == Ok("1")
}

#[cfg(test)]
mod status_line_tests {
    use super::{status_left, ticket_crumb, tmux_text};

    /// The root terminal has no ticket, so the line skips the ticket crumb
    /// and its separator rather than draw an empty segment (T-489).
    #[test]
    fn the_status_line_drops_an_empty_ticket_crumb() {
        assert_eq!(
            status_left("mesimon", "", "", "terminal"),
            " mesimon › #[bold]mesimon#[nobold] › #[bold]terminal#[nobold] "
        );
        assert_eq!(
            status_left("mesimon", "", "#[bold]T-4#[nobold] fix", "claude"),
            " mesimon › #[bold]mesimon#[nobold] › #[bold]T-4#[nobold] fix › #[bold]claude#[nobold] "
        );
    }

    /// The status-line crumb names the ticket by key, then title (T-428):
    /// the key is bold, the title is capped and scrubbed the way it was.
    #[test]
    fn the_ticket_crumb_leads_with_the_key() {
        assert_eq!(ticket_crumb("T-428", "show the id"), "#[bold]T-428#[nobold] show the id");
        assert_eq!(
            ticket_crumb("T-1", ""),
            "#[bold]T-1#[nobold]",
            "no trailing space for an empty title"
        );
        let long = "x".repeat(60);
        assert_eq!(ticket_crumb("T-9", &long), format!("#[bold]T-9#[nobold] {}", "x".repeat(48)));
        // A title cannot open a tmux style: `#` doubles, quotes vanish.
        assert_eq!(ticket_crumb("T-2", "a #[fg=red] 'b'"), "#[bold]T-2#[nobold] a ##[fg=red] b");
        assert_eq!(tmux_text("T-2", 16), "T-2");
    }
}

#[cfg(test)]
mod crown_archive_tests {
    use super::crown_archive_off;

    /// T-590: while the board's switch is off, the crown reads the row a
    /// person turns and its own road — the move to DONE for an archive, a
    /// person for a restore.
    #[test]
    fn the_crown_s_archive_names_the_row_while_off() {
        assert_eq!(
            crown_archive_off("T-5", false),
            "Settings → Agents → Crown archives tickets is off; move T-5 to DONE instead, or a \
             person archives"
        );
        assert_eq!(
            crown_archive_off("T-5", true),
            "Settings → Agents → Crown archives tickets is off; a person restores T-5"
        );
    }
}

#[cfg(test)]
mod permission_release_tests {
    use super::releases_permission;
    use crate::ingest::HookFrame;
    use mesimon_core::road::Road;
    use serde_json::json;

    fn frame(event: &str, road: Road, payload: serde_json::Value) -> HookFrame {
        HookFrame {
            session: uuid::Uuid::nil().to_string(),
            event: event.into(),
            reason: None,
            pane: None,
            road,
            payload,
        }
    }

    /// T-581: the mod's hold in `classic.PermissionRequest` runs on until
    /// the daemon closes `mesimon approve`'s wait, and the person answering
    /// the dialog in the pane is the `PostToolUse` edge the mod relays, read
    /// exactly as the hook set's.
    #[test]
    fn the_session_s_own_edge_ends_the_wait_on_either_road() {
        for road in [Road::Hooks, Road::Mod] {
            for event in
                ["PostToolUse", "PostToolUseFailure", "Stop", "UserPromptSubmit", "SessionEnd"]
            {
                assert!(releases_permission(&frame(event, road, json!({}))), "{event} {road:?}");
            }
            // A subagent's tool, or a frame that says nothing of the dialog.
            assert!(!releases_permission(&frame("PostToolUse", road, json!({"agent_id": "a7"}))));
            for event in ["PreToolUse", "PermissionRequest", "Notification", "ModUsage"] {
                assert!(!releases_permission(&frame(event, road, json!({}))), "{event}");
            }
        }
    }
}
