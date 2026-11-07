//! The launch the ticket page watches (T-1156): from the press to the
//! agent's box, what the board is doing — the worktree being made, its init
//! script, the agent starting — with the seconds each took as this client
//! saw them. Draw-side memory, like the shin's clock: a client that opens
//! the page mid-launch lists only the steps it saw begin, and never guesses
//! a duration it did not watch.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use mesimon_core::board::{SessionRecord, SessionState, StopReason};

use super::App;

/// What this client has seen of each ticket's launch.
#[derive(Default)]
pub(crate) struct Launches {
    seen: RefCell<HashMap<ulid::Ulid, Seen>>,
    /// Tickets last drawn with no agent at all: a launch that begins from
    /// here is the letter's to wake.
    seats: RefCell<HashSet<ulid::Ulid>>,
}

#[derive(Clone, Copy)]
struct Seen {
    worktree: Option<(Instant, Option<Instant>)>,
    init: Option<(Instant, Option<Instant>)>,
    agent: Option<Instant>,
    fresh: bool,
    /// When the list gave way to the agent's box: the ready hop's clock.
    ended: Option<Instant>,
}

/// One row of the launch list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Under way, for this many whole seconds.
    Doing(u64),
    /// Over, in this many.
    Done(u64),
    /// Over, and how it went wrong: the init script's own words
    /// (`InitRun::detail`), `init failed ∙ exit 2`.
    Failed(String),
    /// Not begun.
    Todo,
}

/// The launch list, as the page draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Launch {
    /// The worktree being made, and its branch: absent on a ticket that
    /// starts in the checkout, or one whose worktree this client did not
    /// see being made.
    pub worktree: Option<(Step, String)>,
    /// The init script: absent where none ran while this client watched.
    pub init: Option<Step>,
    pub agent: Step,
    /// What becomes of the words in the box: `true` when mesimon sends
    /// them once the agent is ready (a start with an ask, a brief), `false`
    /// when the title waits there for the person's Enter.
    pub sends: bool,
    /// The launch began on a ticket with no agent: the rail's letter wakes.
    pub fresh: bool,
}

fn secs(from: Instant, to: Instant) -> u64 {
    to.saturating_duration_since(from).as_secs()
}

/// An agent on its way to its box: the pane up and the hook not yet
/// heard, or heard with the words mesimon owes it still unsent.
pub(crate) fn booting(rec: &SessionRecord) -> bool {
    rec.kind.is_agent()
        && (rec.state == SessionState::Spawning
            || (rec.state == SessionState::Idle { stop_reason: StopReason::Unknown }
                && rec.pending_submit))
}

impl App {
    /// A spawn parked behind its worktree (M4): the binding queued or being
    /// made, and no agent on the ticket yet. The seat's press is spent.
    pub(crate) fn launch_parked(&self, ticket: ulid::Ulid) -> bool {
        self.board.live_agent(ticket).is_none()
            && self
                .wt_item(ticket)
                .is_some_and(|w| matches!(w.status.as_str(), "queued" | "provisioning"))
    }

    /// The ticket's launch as the page draws it, `None` once the agent is
    /// at its box (or before anything was pressed). Called by the draw: it
    /// stamps each step's start and end as it first sees them.
    pub(crate) fn launch(&self, ticket: ulid::Ulid) -> Option<Launch> {
        let now = Instant::now();
        let rec = self.board.live_agent(ticket);
        let parked = self.launch_parked(ticket);
        let boot = rec.is_some_and(booting);
        let mut seen = self.launches.seen.borrow_mut();
        if !parked && !boot {
            if let Some(s) = seen.get_mut(&ticket) {
                s.ended.get_or_insert(now);
            }
            if rec.is_none() {
                seen.remove(&ticket);
                self.launches.seats.borrow_mut().insert(ticket);
            }
            return None;
        }
        let from_seat = self.launches.seats.borrow_mut().remove(&ticket);
        let fresh = from_seat
            || rec.is_none()
            || rec.is_some_and(|r| r.typed_title.is_some() && r.transcript_path.is_none());
        let s = seen.entry(ticket).or_insert(Seen {
            worktree: None,
            init: None,
            agent: None,
            fresh,
            ended: None,
        });
        if s.ended.is_some() {
            *s = Seen { worktree: None, init: None, agent: None, fresh, ended: None };
        }
        let wt = self.wt_item(ticket);
        let making = wt.is_some_and(|w| matches!(w.status.as_str(), "queued" | "provisioning"));
        let init_running = making && wt.and_then(|w| w.detail.as_deref()) == Some(INIT_RUNNING);
        if making {
            s.worktree.get_or_insert((now, None));
        }
        if init_running {
            s.init.get_or_insert((now, None));
        }
        // A step ends the first frame something after it is seen.
        if let Some((_, end @ None)) = &mut s.worktree {
            if init_running || !making {
                *end = Some(now);
            }
        }
        if let Some((_, end @ None)) = &mut s.init {
            if !init_running {
                *end = Some(now);
            }
        }
        if boot {
            s.agent.get_or_insert(now);
        }
        let worktree = s.worktree.map(|(a, e)| {
            let step = e.map_or(Step::Doing(secs(a, now)), |e| Step::Done(secs(a, e)));
            (step, wt.map(|w| w.branch.clone()).unwrap_or_default())
        });
        let failed = wt
            .filter(|w| w.status == "attached")
            .and_then(|w| w.detail.clone())
            .filter(|d| d.starts_with("init "));
        let init = s.init.map(|(a, e)| match (e, failed) {
            (None, _) => Step::Doing(secs(a, now)),
            (Some(_), Some(words)) => Step::Failed(words),
            (Some(e), None) => Step::Done(secs(a, e)),
        });
        let agent = s.agent.map_or(Step::Todo, |a| Step::Doing(secs(a, now)));
        // A parked start sends words unless it is the `enter` road, whose
        // press also asked for the pane (`pending_spawn_focus`).
        let sends = rec.map_or_else(
            || !self.pending_spawn_focus.is_some_and(|(t, ..)| t == ticket),
            |r| r.pending_submit,
        );
        Some(Launch { worktree, init, agent, sends, fresh: s.fresh })
    }

    /// The ms since `ticket`'s launch gave way to its agent's box, while the
    /// ready hop still plays; `None` after, or for a launch this client did
    /// not watch.
    pub(crate) fn launch_ready_ms(&self, ticket: ulid::Ulid) -> Option<u64> {
        let seen = self.launches.seen.borrow();
        let s = seen.get(&ticket).filter(|s| s.fresh)?;
        let ms = s.ended?.elapsed().as_millis() as u64;
        (ms < crate::creature::ready_ms()).then_some(ms)
    }
}

/// The binding's detail while its init script runs (`server.rs`'s
/// `on_provision_init`).
pub(crate) const INIT_RUNNING: &str = "init script running";
