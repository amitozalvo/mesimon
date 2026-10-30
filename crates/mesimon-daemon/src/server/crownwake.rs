//! What wakes the crown (T-414, narrowed by T-469, T-527, T-554, widened by
//! T-591): five ticket events, not every `Stop`.
//!
//! A worker's turn ending is how its process breathes, not what happened to
//! its ticket. One landing used to be three wakes — the delivery, the rebase
//! the crown itself asked for, and the merged notice a person's `m` sent —
//! and each is a crown turn. So a turn's end is looked at, off the writer
//! thread (`probe_turn`), and the crown hears only of:
//!
//! 1. **delivered** — a worker it started ends a turn with something new to
//!    merge: its branch ahead of the base at a tip the crown has not heard
//!    of, or, in a shared checkout, a HEAD it has not heard of;
//! 2. **finished its turn** (T-591) — a worker it started ends a turn with
//!    nothing new to merge and nothing pending on its ticket: no words
//!    queued for it or on their way, no hand up, no merge the train will
//!    make. A worker whose answer is words — notes, a review, a test — is
//!    done with what it was asked, and the crown decides what comes next;
//! 3. **ended its turn after your ask went in** — a turn the crown's
//!    `ask_agent` words, an answer or a plan's accept rode has ended,
//!    whatever it left behind; an ask's line names it and its word then
//!    (`crownask`, T-938);
//! 4. **raised its hand** — `raise_hand`, as before, and **asks a
//!    question** — a worker it started stopped on `AskUserQuestion`, where
//!    the board lets the crown answer (T-569); and **hit a usage limit**
//!    (T-802) — its turn ended on the account's limit, which no turn of
//!    its own ends: a person does, and the crown stops waiting on it;
//! 5. **merged** — a worker it started has its branch read `merged` by the
//!    worktree flags (`hear_merges`), however it got there: `m`, the train,
//!    or a `git merge` in a terminal. A shared-checkout worker has no branch,
//!    so nothing to read; its commits are on the base the moment they exist.
//!
//! And one that is no worker's at all (T-777): **could not start** — a
//! start the crown asked for (`start_agent`) whose worktree could not be
//! cut, a repository with no commits yet the common case. The start's
//! receipt said `session_started: false` while the tree was being cut, so
//! without this line the crown waited on a worker that would never come.
//! The line carries the card's own words, which end on the fix.
//!
//! And one that is no event at all (T-599): **lingering** — a worker it
//! started has sat idle at its composer with background tasks running for
//! `LINGER_MS` (30 min) with no foreground turn. Once per stretch, and the
//! weakest news: the wait may be the work (a long suite, a rig run) or a
//! loop that never ends, and time cannot tell which, so the board decides
//! nothing — it says so, and the crown asks the worker.
//!
//! A turn that took mesimon's own merge-flow words (`m`'s or the train's
//! rebase ask, the merged notice) is a merge step: the person already knows,
//! and the crown learns on its next `get_ticket` — or from the merge itself,
//! which the flags see and the words do not. Every wake line carries what
//! changed since the crown last heard (`merge_state needs_rebase → ahead`,
//! `column REVIEW`), so the obvious costs no tool call.
//!
//! **What is pending on a ticket** (T-596) is one list, read by
//! `Daemon::pending_on` wherever a wake is judged, and one table of what
//! each entry does to a wake (`due`). The next gap is a line here, not a
//! new rule:
//!
//! - **the train lane** — the armed merge train will land the branch from
//!   here (T-554): a delivery or a finished turn is held, and the crown
//!   hears it once, at the merge, as the delivery with `merged` in its
//!   delta — or the moment the train will not take it after all
//!   (`hear_deferred`), as what was held;
//! - **a merge step in flight** — the merge flow's words (`m`'s or the
//!   train's rebase ask, the merged notice) pasted and not yet taken, or
//!   the turn that took them still running: a merge, and a delivery, is
//!   held until that turn is over (`hear_stepped`), so the crown hears a
//!   landing when its worker is finished entirely — merged, notified, and
//!   its turn ended — and its line says `and finished its turn`;
//! - **words on their way** — queued for the agent (a person's, or the
//!   crown's held for `^y` or sent by the queue), pasted and not yet
//!   taken, or a launch's that never reached it (`unsent`: the seat's
//!   resend, or the one a restart makes, T-722): a finished turn is silent,
//!   and the turn those words run is judged on its own end (T-591);
//! - **a turn running**: a finished turn is silent (T-591);
//! - **its hand up, a question or a plan** — each its own wake: a finished
//!   turn is silent.
//!
//! An answer, a hand, a question and a plan are never held: the crown
//! asked, or someone waits on it. A delivery is never silenced, only
//! delayed: words queued behind it do not hold it (T-591). A merge with no
//! step in flight — no live agent, a merge the train could not notify, a
//! person's `m` whose notice is not sent yet — is heard at once.
//!
//! **Whose worker (T-712).** Every event above is heard for a worker a
//! crown started (`started_by_crown`) — the crown worn now, whichever
//! ticket wore it at the start (T-927): the seat owns its workers, and a
//! crown moved to another ticket hears them — and — behind the board's
//! `crown_watches` — for a ticket the crown watches (`watch_ticket`,
//! `Daemon::crown_watched`): a person's worker the crown was told to wait
//! for. A ticket the crown holds out of the merge train (`hold_ticket`,
//! `Daemon::crown_held`) is heard as a watched one, with no switch: the
//! hold's receipt promises its delivery. A watched ticket is heard delivered, finished, raised, limited and merged,
//! and the watch ends at the merge; its question and its plan stay the
//! person's (the crown cannot answer a person's agent), and so does its
//! lingering: the crown's words do reach a watched agent (T-734), but a
//! person's agent's background tasks are the person's to end. Behind the
//! same switch, a ticket with no agent that carries a pull request is heard
//! for the forge's news and its merge (`unattended_pr`, T-890): the crown's
//! own work, named on the ticket it was for, has no worker to be heard by.
//! `crown_hears` is the one claim.
//!
//! **A turn's end waits for its forge read (T-773).** A turn that pushed or
//! opened a PR owes a forge read (T-761's look and burst); its wake —
//! delivered, finished, answered — is held in `Heard::forging` until that
//! read lands or `FORGE_WAIT_MS` pass, so the crown's `get_ticket` lists the
//! PR the turn opened, and the line says the opening (`opened !2`), which
//! wakes no one by itself. A board no forge reads, or a turn whose look
//! found nothing to read, holds nothing.

use super::crownexpect::{overrun_clause, Overrun};
use super::*;
use serde::{Deserialize, Serialize};

/// Why the words a turn ran on were sent. Stamped on the paste's owed entry
/// (`tag_owed`) and moved to `Daemon::turn_asks` by its ack, so the turn
/// that TOOK the words knows it at its end — not the one that happened to
/// be running when they were pasted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnAsk {
    /// The crown's answer to a worker's question or its plan accepted
    /// (T-569, T-582), and, in a `crown.json` from before T-938, its
    /// `ask_agent` words.
    Crown(ulid::Ulid),
    /// The crown's `ask_agent` words, by the ask's id (T-938): handed to
    /// the turn running them by the agent's own edge (`crownask`), and
    /// `said` set at that turn's end to the ask's word then, which the
    /// wake carries as it was.
    Asked { crown: ulid::Ulid, ask: ulid::Ulid, said: Said },
    /// A merge-flow sentence: a rebase ask or the merged notice, by `m` or
    /// by the train.
    Merge,
}

impl TurnAsk {
    /// The crown the turn answers, for a crown's turn of either kind.
    pub(super) fn crown(self) -> Option<ulid::Ulid> {
        match self {
            TurnAsk::Crown(c) | TurnAsk::Asked { crown: c, .. } => Some(c),
            TurnAsk::Merge => None,
        }
    }
}

/// The crown ask's word at the end of a turn it rode (T-938): `taken` where
/// its words started that turn, `handed` where they went into one already
/// running, `unknown` where a restart lost what became of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Said {
    /// The words went in and no edge of the agent's named them (a paste,
    /// a fill): the turn ended after them, and that is all it says.
    Sent,
    Handed,
    Taken,
    Unknown,
}

impl Said {
    fn word(self) -> &'static str {
        match self {
            Said::Sent => "sent",
            Said::Handed => "handed",
            Said::Taken => "taken",
            Said::Unknown => "unknown",
        }
    }

    /// A receipt of the agent's named the words.
    fn received(self) -> bool {
        matches!(self, Said::Handed | Said::Taken)
    }
}

/// The ask a turn-end wake was owed for, as it stood then (T-938): the line
/// names this one, never whichever ask is latest when the wake is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct AskSaid {
    pub(super) ask: ulid::Ulid,
    pub(super) said: Said,
}

impl AskSaid {
    pub(super) fn of(asked: Option<TurnAsk>) -> Option<AskSaid> {
        match asked? {
            TurnAsk::Asked { ask, said, .. } => Some(AskSaid { ask, said }),
            _ => None,
        }
    }
}

/// A worker's work at one turn's end: the baseline the next turn is judged
/// against and the two sides of a wake's delta.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ToldOnDisk", into = "ToldOnDisk")]
pub(super) struct Told {
    /// The branch tip on a worktree (legs joined), HEAD on a checkout.
    pub(super) tip: String,
    /// `merge_state`'s word; `None` on a checkout, which has no branch.
    /// Skipped only to keep serde's borrow rule off a `'static`: the field
    /// crosses as `ToldOnDisk`'s.
    #[serde(skip)]
    pub(super) merge: Option<&'static str>,
    /// Commits ahead of the base, the legs summed; 0 on a checkout.
    pub(super) ahead: u32,
    /// Each leg's own `ahead` by its word (`root`, `api`), on a branch of
    /// more than one leg alone (T-772): a sum names no repository and
    /// matches no forge's count, so a multi-leg delta says the legs.
    pub(super) legs: Vec<(String, u32)>,
    pub(super) column: String,
    /// The forge's words when a wake is about its edge (T-739, T-744):
    /// `pipeline passed !1, review approved !1`. `None` where no forge spoke.
    pub(super) forge: Option<String>,
}

/// `Told` in `crown.json` (T-602): the merge word as written, read back as
/// one of `worktree::merge_word`'s four — a word this build does not know
/// reads as no branch, which only widens what the next wake says.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct ToldOnDisk {
    tip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge: Option<String>,
    ahead: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    legs: Vec<(String, u32)>,
    column: String,
    /// T-739 wrote it `pipeline`.
    #[serde(skip_serializing_if = "Option::is_none", alias = "pipeline")]
    forge: Option<String>,
}

impl From<ToldOnDisk> for Told {
    fn from(d: ToldOnDisk) -> Told {
        let merge = d.merge.and_then(|w| {
            ["merged", "needs_rebase", "ahead", "clean"].into_iter().find(|k| *k == w)
        });
        Told { tip: d.tip, merge, ahead: d.ahead, legs: d.legs, column: d.column, forge: d.forge }
    }
}

impl From<Told> for ToldOnDisk {
    fn from(t: Told) -> ToldOnDisk {
        ToldOnDisk {
            tip: t.tip,
            merge: t.merge.map(str::to_string),
            ahead: t.ahead,
            legs: t.legs,
            column: t.column,
            forge: t.forge,
        }
    }
}

impl Told {
    /// Something a merge would take: a branch ahead and not yet in, or, on
    /// a checkout, any HEAD at all — its commits are already where they go.
    fn mergeable(&self) -> bool {
        match self.merge {
            Some(word) => self.ahead > 0 && word != "merged",
            None => !self.tip.is_empty(),
        }
    }
}

/// A worktree branch as one look saw it, legs folded: what a wake says of
/// it, and what the merge train is judged on (T-554). A turn's own look or
/// the train's last sample, whichever has seen the newer tip.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct BranchLook {
    pub(super) tip: String,
    pub(super) base_tip: String,
    pub(super) ahead: u32,
    /// `Told::legs`: each leg's `ahead`, on more than one leg alone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) legs: Vec<(String, u32)>,
    pub(super) merged: bool,
    pub(super) needs_rebase: bool,
    pub(super) conflict: bool,
}

/// `Told::legs` out of a branch's legs: each leg's word and `ahead`, empty
/// on a single leg, whose `ahead` is the branch's.
pub(super) fn leg_aheads(legs: &[worktree::RepoFlags]) -> Vec<(String, u32)> {
    if legs.len() < 2 {
        return Vec::new();
    }
    legs.iter().map(|l| (worktree::leg_word(&l.name).to_string(), l.ahead)).collect()
}

impl BranchLook {
    fn told(&self, column: String) -> Told {
        Told {
            tip: self.tip.clone(),
            merge: Some(worktree::merge_word(self.merged, self.needs_rebase, self.ahead)),
            ahead: self.ahead,
            legs: self.legs.clone(),
            column,
            forge: None,
        }
    }
}

/// What the crown is woken for, in the precedence two events on one worker
/// coalesce by: a question outranks a plan (T-582) and a plan a hand
/// (T-569) — all three wait on someone, and a question or a plan is a turn
/// frozen on it — a hand an answer, an answer a delivery, a delivery a
/// merge — a merge folded into any other line is said in its delta
/// (`merge_state merged`), so a delivery and its merge between two crown
/// turns are one line — and a merge a finished turn (T-591), the weakest
/// news: every other line about the worker says what it left. A merge told
/// alone does not say its turn ended (T-1004: let go on no proven end past
/// a restart), so a finished turn folding into a merge or a delivery owed
/// keeps its end in the line (`finished`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WakeCause {
    /// Idle with background tasks for `LINGER_MS` (T-599): the weakest.
    Lingering,
    /// A turn the crown armed with `expect_secs` has run past it, the agent
    /// still working (T-973): above lingering, which it never meets (one is
    /// a turn, the other a park), and under a finished turn, which says the
    /// same turn ended.
    Overrun,
    Finished,
    /// Its pull request has news from the forge (T-739, T-744): a pipeline
    /// failed or passed, a job waits on a person, the merge reached an
    /// environment, a conflict came or went, a review landed, a PR merged
    /// that moves no worktree flag (T-757). Above a
    /// finished turn, since each is a decision someone may take; under a
    /// merge and a delivery, whose delta carries the forge's words anyway.
    /// T-739's crown.json spelled it `pipeline`.
    #[serde(alias = "pipeline")]
    Forge,
    Merged,
    Delivered,
    Answered,
    /// Its turn ended on an error (T-936): a server's, a refused request, a
    /// model gone. Above an answer, since the answer's turn is the one that
    /// failed, and under a limit, which a person ends.
    Failed,
    /// Its turn ended on a usage limit (T-802): above an answer, since the
    /// answer's turn is the one the limit cut, and under a hand, which a
    /// worker raised on purpose.
    Limited,
    Raised,
    Planned,
    Asked,
    /// A start the crown asked for whose worktree could not be cut (T-777).
    /// The strongest: no agent exists, so nothing else about the worker
    /// is news, and the crown is the one who must act.
    Unstarted,
    /// A person promoted the board's integration branch to main (T-954).
    Promotion,
}

impl WakeCause {
    /// The clause after the ticket in the sentence.
    fn clause(self) -> &'static str {
        match self {
            WakeCause::Lingering => "has been idle with background tasks",
            WakeCause::Overrun => "has run its turn past what it expected",
            WakeCause::Finished => "finished its turn",
            WakeCause::Forge => "has news from the forge",
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            // What a turn's end proves (T-938): the ask's own word rides
            // the brackets, and the clause names no ask it cannot tie.
            WakeCause::Answered => "ended its turn after your ask went in",
            WakeCause::Failed => "ended its turn on an error",
            WakeCause::Limited => "hit a usage limit",
            WakeCause::Raised => "raised its hand",
            // Never the plan's words (T-414's rule for a hand's reason):
            // the crown reads them with `get_ticket`.
            WakeCause::Planned => "stops on a plan",
            // Never the question's words (T-414's rule for a hand's
            // reason): the crown reads them with `get_ticket`.
            WakeCause::Asked => "asks a question",
            WakeCause::Unstarted => "could not start",
            WakeCause::Promotion => "integration branch promoted to main",
        }
    }

    /// The feed's word.
    fn word(self) -> &'static str {
        match self {
            WakeCause::Lingering => "lingering",
            WakeCause::Overrun => "overrun",
            WakeCause::Finished => "finished",
            WakeCause::Forge => "forge",
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered",
            WakeCause::Failed => "failed",
            WakeCause::Limited => "limited",
            WakeCause::Raised => "raised",
            WakeCause::Planned => "planned",
            WakeCause::Asked => "asked",
            WakeCause::Unstarted => "unstarted",
            WakeCause::Promotion => "promotion",
        }
    }
}

/// One thing the crown has yet to hear about: keyed by worker, so a second
/// event before delivery raises the cause (`WakeCause`'s order) and moves
/// `to` rather than adding a clause. `from` stays what the crown last heard.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CrownWake {
    pub(super) worker: ulid::Ulid,
    cause: WakeCause,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from: Option<Told>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<Told>,
    /// Held until the merge step's turn was over (T-596): the line says the
    /// worker finished its turn after the merge.
    #[serde(default)]
    finished: bool,
    /// A lingering wake's numbers (T-599), as the stretch stood when it
    /// was owed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    linger: Option<Linger>,
    /// Owed across a daemon restart (T-602): the line says `after a
    /// restart`, so the crown knows it may be late.
    #[serde(default)]
    pub(super) late: bool,
    /// Why a start could not be made (T-777): the card's words for the
    /// worktree that was not cut, said in the line's brackets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unstarted: Option<String>,
    /// The failed turn this wake is about (T-936), as the board saw it fail:
    /// its reason said first in the line's brackets, never read off a later
    /// state, and its identity what a refined reason must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failed: Option<FailedTurn>,
    /// The crown ask the turn rode, as it stood at the turn's end (T-938).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ask: Option<AskSaid>,
    /// An overrun wake's numbers (T-973), as the turn stood when it was owed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    overrun: Option<Overrun>,
    /// The wake says a merge (T-1071), as `Owed::merge`: kept on a landing
    /// held for its merge step, which may also be a delivery whose branch
    /// the step left unmerged. Written only when set.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    merge: bool,
    /// Board-level promotion detail; the worker key is the crown ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    promotion: Option<String>,
}

/// One failed turn (T-936): the seat, the turn (Codex's own turn id; on
/// Claude, the moment the board saw the failure), and what it ended on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FailedTurn {
    pub(super) session: uuid::Uuid,
    pub(super) turn: String,
    pub(super) reason: mesimon_core::board::FailReason,
}

/// One ticket's line from a forge pass (`hear_forge`): its words, whether
/// they were read inside its recovery window (T-937), and whether they say
/// a PR's merge (T-1071).
pub(super) struct ForgeNews {
    pub(super) worker: ulid::Ulid,
    pub(super) words: String,
    pub(super) late: bool,
    pub(super) merged: bool,
}

/// What rides a wake beside its cause and delta, as `owe_wake` folds it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Owed {
    pub(super) finished: bool,
    pub(super) linger: Option<Linger>,
    pub(super) late: bool,
    pub(super) ask: Option<AskSaid>,
    pub(super) overrun: Option<Overrun>,
    /// The wake says a merge (T-1071): the flags' landing, the forge's
    /// `merged !N`, or a landing held for its merge step that was one. What
    /// ends a watch; the branch's standing `merged` does not.
    pub(super) merge: bool,
}

/// What a sentence to the crown tells it (`Daemon::telling`), kept until
/// the crown has it (`Daemon::told`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct Telling {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    columns: Vec<(ulid::Ulid, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    raised: Vec<ulid::Ulid>,
    #[serde(default)]
    news: String,
}

/// The crown's wake pasted into a Codex crown's box (T-918) and not yet
/// entered: the Enter is held while a person is in the pane, and a box they
/// changed is theirs. Counted told at the Enter (or a turn begun on it) and
/// owed again if the box never sends it. Kept in `crown.json`, so the next
/// daemon knows the words in the box for its own (`resume_crown_entering`)
/// and never waits on them as a person's draft.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Entering {
    pub(super) crown: ulid::Ulid,
    /// The crown's Codex seat.
    pub(super) session: uuid::Uuid,
    /// The sentence pasted.
    pub(super) text: String,
    pub(super) wakes: Vec<CrownWake>,
    pub(super) telling: Telling,
    /// A sample since the paste found a person in the pane: a box that no
    /// longer reads as the wake is their edit. With nobody seen, nobody
    /// but mesimon wrote it.
    #[serde(default)]
    pub(super) person_came: bool,
    /// The mod's `submit` frame a Claude crown's wake went down as (T-936),
    /// and the pane it went to: told on that frame's `entered` from that
    /// seat and pane alone, owed again on its refusal. None for a Codex box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) frame: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pane: Option<String>,
    /// When a submitted wake's frame stops being waited for (T-936): the
    /// window its owed entry had, kept here, where no prompt erases it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) expires_ms: Option<u64>,
    /// Parked on a crown seat a wake or a relaunch is bringing up (T-936):
    /// told at its delivery edge, the launch's own receipt, or once its
    /// words go down the mod as a frame (`frame` then).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) parked: bool,
}

/// Whether a crown wake's failed delivery still holds the drain (T-936):
/// `hold` is the seat it failed on and that seat's state's moment, `seat`
/// the current crown's seat and its moment now. Only that same seat, not
/// yet moved: another crown's seat, or this one after its state moved, is
/// tried again.
pub(super) fn failed_hold(
    hold: Option<(uuid::Uuid, Option<u64>)>,
    seat: Option<(uuid::Uuid, Option<u64>)>,
) -> bool {
    hold.is_some() && hold == seat
}

/// How long a tmux client attached to the crown's pane holds its wake after
/// tmux last recorded activity from it (T-945): ten minutes. It bounds this
/// hold alone, not the wake's delay: a draft, an unknown reading or another
/// hold can last longer. A terminal left on the crown's pane held a wake for
/// eleven hours (2026-10-24); whether ten minutes is the right value needs
/// observed wake durations.
pub(super) const PERSON_IDLE_MS: u64 = 10 * 60 * 1000;

/// Why a tmux client attached to the crown's pane holds its wake (T-945):
/// tmux recorded activity from it inside the bound, or its reading cannot
/// be trusted (missing, past the arithmetic, or after the sample), and
/// an untrusted reading holds as the attachment alone did before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClientHold {
    Recent,
    Unknown,
}

/// What holds the crown's wake for a person (T-918): a client in its pane,
/// or words left in its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PersonHold {
    Client(ClientHold),
    Draft,
}

impl PersonHold {
    /// The crown's wake row's word for the hold.
    pub(super) fn word(self) -> &'static str {
        match self {
            PersonHold::Client(ClientHold::Recent) => wake_when_you_leave(),
            PersonHold::Client(ClientHold::Unknown) => "idle unknown ∙ leave",
            PersonHold::Draft => "after your draft",
        }
    }
}

/// `leave or 10m idle`, the bound's own duration in it: the release a person
/// has and tmux's activity record, never a person's last key. The card draws
/// `crown wakes` over it where the whole row does not fit (`App::card_owed`).
fn wake_when_you_leave() -> &'static str {
    static WORDS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| format!("leave or {} idle", span_words(person_idle_ms())))
}

/// `10m`, `5s`, `1500ms`: a bound in the largest unit it fills whole.
fn span_words(ms: u64) -> String {
    if ms.is_multiple_of(60_000) {
        format!("{}m", ms / 60_000)
    } else if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// The bound, shortened by an e2e (`MESIMON_PERSON_IDLE_MS`). Read once, so
/// the row's words and the hold agree; a zero or unreadable value is
/// refused and the default stands, never a hold turned off.
pub(super) fn person_idle_ms() -> u64 {
    static MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MS.get_or_init(|| idle_bound(std::env::var("MESIMON_PERSON_IDLE_MS").ok().as_deref()))
}

fn idle_bound(seam: Option<&str>) -> u64 {
    seam.and_then(|v| v.trim().parse().ok()).filter(|ms| *ms > 0).unwrap_or(PERSON_IDLE_MS)
}

/// Does an attached client hold the crown's wake (T-945), from tmux's
/// `#{session_activity}` (whole epoch seconds) and the sample's own time:
/// `Recent` until `bound` past the end of the second the stamp names (the
/// fraction it drops is counted as activity), `Unknown` for a missing
/// stamp, one whose second begins after the sample's time (a clock stepped
/// back, or input in a later second than the sample began, which the next
/// sample reads) or one past the arithmetic, and `None` once the bound has
/// gone by.
pub(super) fn client_hold(activity: Option<u64>, at_ms: u64, bound_ms: u64) -> Option<ClientHold> {
    let Some(secs) = activity else { return Some(ClientHold::Unknown) };
    let Some(stamp) = secs.checked_mul(1000) else { return Some(ClientHold::Unknown) };
    if stamp > at_ms {
        return Some(ClientHold::Unknown);
    }
    let Some(expiry) = stamp.checked_add(1000).and_then(|s| s.checked_add(bound_ms)) else {
        return Some(ClientHold::Unknown);
    };
    (at_ms < expiry).then_some(ClientHold::Recent)
}

/// Whether a mod report about `frame` from `session` is the crown's
/// submitted wake (T-936): the attempt's own frame, from its seat, while
/// that seat has a live pane and it is the pane the frame went to. A killed
/// record keeps its pane key, and a relaunched one has another.
pub(super) fn submitted_fence(
    e: &Entering,
    rec: Option<&SessionRecord>,
    session: uuid::Uuid,
    frame: &str,
) -> bool {
    let Some(rec) = rec.filter(|r| r.id == session && r.state.has_pane()) else { return false };
    e.session == session && e.frame.as_deref() == Some(frame) && rec.pane_key == e.pane
}

/// A worker idle with background tasks (T-599): how many, and for how long
/// in minutes, when the board noticed. Never the tasks' words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Linger {
    pub(super) tasks: usize,
    pub(super) minutes: u64,
}

/// When a record's stretch idle on tasks began, after the edge `from` →
/// `to` at `now` (T-944): stamped on entering a park on tasks, kept while
/// the park moves between `Background` and `Monitoring` (the demote, or a
/// task's class changing), gone on leaving it — a foreground turn ends the
/// stretch. `kept` is the stamp so far; `None` from a park is a stretch
/// the daemon did not see begin, dated from this edge.
pub(super) fn tasks_stretch(
    from: &SessionState,
    to: &SessionState,
    now: u64,
    kept: Option<u64>,
) -> Option<u64> {
    if !to.on_tasks() {
        None
    } else if from.on_tasks() {
        kept.or(Some(now))
    } else {
        Some(now)
    }
}

/// A worker due the lingering wake (T-599, T-944): a Claude seat the crown
/// started, on another ticket, idle on tasks (`SessionState::on_tasks`)
/// for `after` since its stretch began (`since`, else the state's own
/// stamp). How many tasks is the registry's; it is empty after a restart,
/// and the clause then says no number.
pub(super) fn linger_due(
    s: &SessionRecord,
    crown: ulid::Ulid,
    since: Option<u64>,
    now: u64,
    after: u64,
) -> Option<Linger> {
    if s.kind != SessionKind::Claude
        || s.ticket == crown
        || s.started_by.is_none()
        || !s.state.on_tasks()
    {
        return None;
    }
    let age = now.saturating_sub(since.or(s.state_changed_at)?);
    (age >= after).then(|| Linger { tasks: s.background_tasks.count(), minutes: age / 60_000 })
}

/// How long a worker sits idle with background tasks before the crown is
/// told (T-599). Not a verdict: the crown is told, and nothing else moves.
pub(super) const LINGER_MS: u64 = 30 * 60 * 1000;

/// How much of a failed start's reason rides the wake line (T-777): the
/// card's sentence whole, a later stage's git text cut where it runs long.
const UNSTARTED_MAX_BYTES: usize = 240;

/// The lingering clause: `has been idle with 3 background tasks for 30
/// min`, the count left out where the registry lost it (a restart starts
/// it empty), the hours said past two of them.
pub(super) fn lingering_clause(l: Linger) -> String {
    let tasks = match l.tasks {
        0 => "background tasks".to_string(),
        1 => "1 background task".to_string(),
        n => format!("{n} background tasks"),
    };
    let time = if l.minutes >= 120 {
        format!("{} h", l.minutes / 60)
    } else {
        format!("{} min", l.minutes)
    };
    format!("has been idle with {tasks} for {time}")
}

impl CrownWake {
    fn says_merge(&self) -> bool {
        self.cause == WakeCause::Merged
            || self.merge
            || self.to.as_ref().is_some_and(|to| {
                to.merge == Some("merged")
                    && self
                        .from
                        .as_ref()
                        .is_some_and(|from| from.merge != Some("merged") || from.tip != to.tip)
            })
    }

    /// A wake can cover a held merge's fact, but never another answer or
    /// forge edge merely because the branch still reads merged.
    fn only_merge_at(&self, tip: &str) -> bool {
        !self.finished && self.merge_fact_at(tip)
    }

    fn merge_fact_at(&self, tip: &str) -> bool {
        matches!(self.cause, WakeCause::Merged | WakeCause::Delivered)
            && self
                .to
                .as_ref()
                .is_some_and(|to| to.merge == Some("merged") && to.tip == tip && to.forge.is_none())
            && self.ask.is_none()
            && self.linger.is_none()
            && self.overrun.is_none()
            && self.failed.is_none()
            && self.unstarted.is_none()
            && self.promotion.is_none()
    }

    fn awaiting_forge_only(&self) -> bool {
        self.cause == WakeCause::Forge && self.to.as_ref().is_none_or(|to| to.forge.is_none())
    }

    /// The clause or delta actually names this branch's merge. `to` alone
    /// is not enough: a held landing already advanced the internal baseline,
    /// so an answer from merged to merged may say no merge at all.
    fn reports_merge(&self) -> bool {
        !matches!(self.cause, WakeCause::Promotion | WakeCause::Unstarted)
            && self.to.as_ref().is_some_and(|to| {
                to.merge == Some("merged")
                    && (self.cause == WakeCause::Merged
                        || self
                            .from
                            .as_ref()
                            .is_none_or(|from| from.merge != Some("merged") || from.tip != to.tip))
            })
    }

    /// A ticket read carries the worker's turn/ask and branch state. Keep
    /// material alerts and forge edges conservatively: their historical
    /// details need not be in a later ticket view, even at the same tip.
    fn turn_news(&self) -> bool {
        matches!(
            self.cause,
            WakeCause::Finished | WakeCause::Answered | WakeCause::Delivered | WakeCause::Merged
        ) && self.to.as_ref().is_none_or(|to| to.forge.is_none())
            && self.linger.is_none()
            && self.overrun.is_none()
            && self.failed.is_none()
            && self.unstarted.is_none()
            && self.promotion.is_none()
    }

    /// A wake owed with what rides it (`Owed`); a start's or a failure's
    /// words are set by the road that owes it.
    fn new(
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
        owed: Owed,
    ) -> CrownWake {
        let Owed { finished, linger, late, ask, overrun, merge } = owed;
        CrownWake {
            worker,
            cause,
            from,
            to,
            finished,
            linger,
            late,
            unstarted: None,
            failed: None,
            ask,
            overrun,
            merge,
            promotion: None,
        }
    }

    /// `fold`, with what rides the second event: a flag either set stays
    /// set, a lingering wake's numbers stay the first's, and the latest
    /// ask and overrun win.
    fn absorb(&mut self, cause: WakeCause, from: Option<Told>, to: Option<Told>, owed: Owed) {
        let Owed { finished, linger, late, ask, overrun, merge } = owed;
        self.fold(cause, from, to);
        self.finished |= finished;
        self.merge |= merge;
        self.linger = self.linger.or(linger);
        self.late |= late;
        self.ask = ask.or(self.ask);
        self.overrun = overrun.or(self.overrun);
    }

    /// A second event on the same worker before delivery: the stronger
    /// cause, the latest work, and `from` kept unless nothing was described
    /// yet — the delta runs from what the crown last heard to now. The
    /// forge's words are edges, not a state (T-744): a later fold keeps
    /// what the earlier ones said, so a pass and a review between two crown
    /// turns are both in the line.
    fn fold(&mut self, cause: WakeCause, from: Option<Told>, to: Option<Told>) {
        self.cause = self.cause.max(cause);
        if self.to.is_none() {
            self.from = from;
        }
        if let Some(mut to) = to {
            let said = self.to.as_mut().and_then(|t| t.forge.take());
            to.forge = match (said, to.forge.take()) {
                (Some(a), Some(b)) if a != b => Some(format!("{a}, {b}")),
                (a, b) => b.or(a),
            };
            self.to = Some(to);
        }
    }

    /// The wake as it is sent (T-785): its `column` read now, as the
    /// crown's `get_ticket` will read it, not as it stood when the wake was
    /// owed — a wake held for the crown's turn waited while the crown, or a
    /// person, moved the card. The edges in it stay as heard.
    pub(super) fn sent_at(&self, column: &str) -> CrownWake {
        let mut w = self.clone();
        if let Some(to) = w.to.as_mut() {
            column.clone_into(&mut to.column);
        }
        w
    }

    /// The clause after the ticket: the cause's, and for a landing held
    /// until its merge step's turn was over (T-596), that the turn ended —
    /// the crown's close-out (`sleep_agent`) goes through from here.
    fn clause(&self) -> String {
        if let (WakeCause::Lingering, Some(l)) = (self.cause, self.linger) {
            return lingering_clause(l);
        }
        if let (WakeCause::Overrun, Some(o)) = (self.cause, self.overrun) {
            return overrun_clause(o);
        }
        // An ask no receipt named (T-938): the turn ended after it went out,
        // and that is all the end proves.
        if let (WakeCause::Answered, Some(a)) = (self.cause, self.ask) {
            if !a.said.received() {
                return format!("ended a turn after ask {} was sent; receipt unconfirmed", a.ask);
            }
        }
        let clause = self.cause.clause();
        if self.finished && matches!(self.cause, WakeCause::Merged | WakeCause::Delivered) {
            format!("{clause} and finished its turn")
        } else {
            clause.to_string()
        }
    }

    /// What the line says in brackets: what changed from what the crown
    /// last heard to now, and for a finished turn (T-591) first that it
    /// left nothing new to merge — never a checkout's HEAD, which that turn
    /// did not move. Nothing at all where git could not say. A wake owed
    /// across a restart says so last (T-602).
    fn changed(&self) -> Vec<String> {
        if let Some(detail) = &self.promotion {
            return vec![detail.clone()];
        }
        if let (WakeCause::Unstarted, Some(why)) = (self.cause, &self.unstarted) {
            return vec![why.clone()];
        }
        let failed = self.failed.as_ref().filter(|_| self.cause == WakeCause::Failed);
        let mut out: Vec<String> = self
            .ask
            .filter(|a| self.cause == WakeCause::Answered && a.said.received())
            .map(|a| format!("ask {}: {}", a.ask, a.said.word()))
            .into_iter()
            .collect();
        out.extend(match &self.to {
            None => Vec::new(),
            Some(to) if self.cause != WakeCause::Finished => delta(self.from.as_ref(), to),
            Some(to) => {
                let mut out = vec!["nothing new to merge".to_string()];
                out.extend(changes(self.from.as_ref(), to, false));
                out
            }
        });
        if let Some(reason) = failed {
            out.insert(0, mesimon_core::board::agent_fail_word(reason.reason).to_string());
        }
        if self.late {
            out.push("after a restart".to_string());
        }
        out
    }
}

/// What a turn's end says about the worker, and why. `before` is the
/// worker's last baseline (`None` after a restart, or before the first
/// look), `now` its work as the turn left it, and `fresh` that a turn ran
/// since the last end judged. An answer always wakes; a merge step never
/// does; something mergeable at a tip not seen before is a delivery; and
/// any other fresh turn finished (T-591) — "nothing new" is no reason for
/// silence, and the crown decides what comes next. What is pending on the
/// ticket is `due`'s to weigh.
pub(super) fn verdict(
    before: Option<&Told>,
    now: &Told,
    answered: bool,
    merge_step: bool,
    fresh: bool,
) -> Option<WakeCause> {
    if answered {
        return Some(WakeCause::Answered);
    }
    if merge_step {
        return None;
    }
    if now.mergeable() && before.is_none_or(|b| b.tip != now.tip) {
        return Some(WakeCause::Delivered);
    }
    fresh.then_some(WakeCause::Finished)
}

/// A state the restart's look may not judge (T-756): the transcript tail's
/// guess at Low, or the stale clock's. After a restart a turn the last
/// daemon saw open re-derives from the tail until a hook re-asserts, and a
/// worker between two tool calls reads idle there; with its branch ahead,
/// that idle read as a delivery the turn had not made. A hook's state is
/// High, and the close T-663 dates in the gap is Medium: both are ends.
pub(super) fn guessed(s: &SessionRecord) -> bool {
    matches!(s.confidence, Confidence::Low | Confidence::Stale)
}

/// An agent not yet re-derived after a restart (T-776): `Unknown` until a
/// hook or the transcript speaks, which says nothing of a turn's end. The
/// train's ask is held open for it (`restore_train`), so a delivery held
/// for the train stays held: released, it told the crown `delivered` a
/// second after the restart for a worker whose rebase turn was still on.
pub(super) fn rederiving(s: &SessionRecord) -> bool {
    matches!(s.state, SessionState::Unknown { .. })
}

/// Whether this state change is a stop the crown is woken for, and why:
/// an agent entering `RequiresAction{Question}` (T-569) or
/// `RequiresAction{Plan}` (T-582) on a board whose crown mode lets it
/// answer — autonomous unless the person chose supervised (T-610). A
/// claude's since T-569, a codex's since T-846, when its runtime carries
/// the request the board answers by keys. Supervised, the person is the
/// one to wake, and the card's needs-you already does. A secret, a form or
/// a permission is never the crown's, so none of them wakes it. Whose agent
/// it is — one a crown started — is `note_crown_wake`'s to judge.
pub(super) fn asks_the_crown(
    mode: CrownMode,
    kind: SessionKind,
    change: &Change,
) -> Option<WakeCause> {
    if change.from == change.to {
        return None;
    }
    stop_cause(mode, kind, &change.to)
}

/// `asks_the_crown`'s judgement of the stop alone: a question or a plan the
/// crown is woken for at `state`. A new dialog that replaces the one an
/// agent is stopped on, with no state edge between them, asks it too
/// (T-943): a plan sent back is replanned and stopped on again inside the
/// attention settle, so the record never leaves `RequiresAction{Plan}`.
pub(super) fn stop_cause(
    mode: CrownMode,
    kind: SessionKind,
    state: &SessionState,
) -> Option<WakeCause> {
    if !mode.takes_stop(kind, state) {
        return None;
    }
    match state {
        SessionState::RequiresAction { reason: Reason::Question } => Some(WakeCause::Asked),
        SessionState::RequiresAction { reason: Reason::Plan } => Some(WakeCause::Planned),
        _ => None,
    }
}

/// Whether this state change is an agent stopping on a usage limit (T-802),
/// whatever the crown mode: nothing the crown sends ends it, but a crown
/// that does not hear it waits on a turn that will not come, since the
/// limit is no turn's end. Whose agent it is is `note_crown_wake`'s.
pub(super) fn limits_the_crown(kind: SessionKind, change: &Change) -> Option<WakeCause> {
    let quota = SessionState::RequiresAction { reason: Reason::QuotaResume };
    (kind.is_agent() && change.from != change.to && change.to == quota)
        .then_some(WakeCause::Limited)
}

/// A Claude turn ending on an error (T-936), as `hear_failure` takes it:
/// the reason, and whether it is a failure of its own. A Claude turn's
/// hooks arrive one by one, a prompt between two turns, so a change into
/// `Failed` is one: a repeated hook changes nothing. One from another
/// failure is the same failure read again (T-659's transcript class), which
/// refines its reason and wakes nobody; and only a hook's or a proven
/// close's (High, Medium) is a failure the board saw: the restart's
/// re-derivation at Low reads one already told, or one nobody saw. A Codex
/// seat is polled, and two turns can fail between two polls into the same
/// state: `codex_failure` judges it by its turn.
pub(super) fn failure_edge(kind: SessionKind, change: &Change) -> Option<Failure> {
    let SessionState::Failed { reason } = change.to else { return None };
    if !matches!(kind, SessionKind::Claude) || change.from == change.to {
        return None;
    }
    if matches!(change.from, SessionState::Failed { .. }) {
        return Some(Failure { reason, new: false });
    }
    matches!(change.confidence, Confidence::High | Confidence::Medium)
        .then_some(Failure { reason, new: true })
}

/// A Codex snapshot that reads `Failed` (T-936), judged by its turn id: a
/// turn other than the last one the seat reported failed (`last`) is a
/// failure of its own, even where the state it leaves reads as the last one
/// did (two turns failed between two polls); the same turn read in another
/// failure is that failure refined; and the same turn read again, a
/// restart's included, is nothing. A snapshot with no turn id is judged by
/// its change, as a Claude seat's.
pub(super) fn codex_failure(
    state: &SessionState,
    turn: &str,
    last: Option<&str>,
    change: Option<&Change>,
) -> Option<Failure> {
    let SessionState::Failed { reason } = *state else { return None };
    let from_failed = change.map(|c| matches!(c.from, SessionState::Failed { .. }));
    let new = Some(Failure { reason, new: true });
    let refined = Some(Failure { reason, new: false });
    match from_failed {
        _ if !turn.is_empty() && last != Some(turn) => new,
        Some(true) => refined,
        Some(false) if turn.is_empty() => new,
        _ => None,
    }
}

/// `failure_edge`'s and `codex_failure`'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Failure {
    pub(super) reason: mesimon_core::board::FailReason,
    /// A failure of its own; false for the same one with its reason refined.
    pub(super) new: bool,
}

/// What is pending on a worker's ticket (T-596): the list in the module's
/// doctrine, one field per line of it, read by `Daemon::pending_on` where
/// a wake is judged and weighed by `due`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Pending {
    /// The armed merge train will land the branch from here (T-554),
    /// judged on the branch as a look saw it. A merged branch has none.
    pub(super) train: bool,
    /// The merge flow's words pasted and not yet taken, or the turn that
    /// took them still running.
    pub(super) merge_step: bool,
    /// Other words for the agent: queued, pasted and not yet taken, or a
    /// launch's that never reached it (`unsent`, T-722).
    pub(super) words: bool,
    /// A turn running on the agent, other than a question or a plan.
    pub(super) turn: bool,
    /// Its hand up.
    pub(super) hand: bool,
    /// A question or a plan the agent stopped on (T-569, T-582).
    pub(super) dialog: bool,
}

impl Pending {
    /// The worker is not done with what it was asked: a turn that ended
    /// with nothing new is not yet news (T-591). The train is not in it —
    /// a finish the train will take is held for it, not silenced.
    fn busy(&self) -> bool {
        self.merge_step || self.words || self.turn || self.hand || self.dialog
    }
}

/// What a wake does once what is pending on its ticket is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    Wake(WakeCause),
    /// Held for the train as this cause (T-554): no wake now, and `told`
    /// stays where it was.
    Hold(WakeCause),
    /// Held as this cause until the merge step in flight is over (T-596):
    /// `told` moves now, so the landing is heard once, and `hear_stepped`
    /// says it.
    Step(WakeCause),
    Silent,
}

/// The table (T-596): `verdict`'s or `merge_verdict`'s answer, with what is
/// pending on the ticket in it. A finished turn on a busy ticket is
/// silent. A delivery or a finished turn (T-591) the train will land is
/// held, and so is one held before (`held`, as what it was held) whose new
/// turn left it still the train's; held, the stronger of the two is kept,
/// so a delivery the crown has not heard of is never said as a finish. One
/// the train will not take after all comes due as that. A merge, or a
/// delivery, while a merge step is in flight waits for that turn's end.
/// Anything else wakes now.
pub(super) fn due(cause: Option<WakeCause>, held: Option<WakeCause>, pending: &Pending) -> Due {
    let cause = cause.filter(|c| *c != WakeCause::Finished || !pending.busy());
    let due = match cause {
        Some(WakeCause::Delivered | WakeCause::Finished | WakeCause::Merged) => cause.max(held),
        Some(other) => return Due::Wake(other),
        None => held,
    };
    match due {
        None => Due::Silent,
        Some(c @ (WakeCause::Delivered | WakeCause::Finished)) if pending.train => Due::Hold(c),
        Some(c @ (WakeCause::Delivered | WakeCause::Merged)) if pending.merge_step => Due::Step(c),
        Some(c) => Due::Wake(c),
    }
}

/// Whether a worker's branch reading `merged` wakes the crown (T-527), and
/// as what. `now` is the work as merged, `heard` what the crown has heard of
/// the worker. Only a worker a crown started (`started`) — the crown's
/// own ticket is never one, since no crown starts itself — and never a
/// checkout, which has no branch to land. A merge the crown was already
/// told of at this tip is silent, which is also what keeps a reading that
/// flaps `merged → ahead → merged` to one wake. Otherwise: the crown was
/// woken for this delivery (the tip it was told of, or the one the last
/// looked-at turn left — the merge flow's own rebase of it) and hears
/// `merged`; or it never heard of this tip, and the merge is the delivery
/// line with `merged` in its delta.
pub(super) fn merge_verdict(started: bool, heard: &Heard, now: &Told) -> Option<WakeCause> {
    if !started || now.merge != Some("merged") {
        return None;
    }
    let told = heard.told.as_ref();
    if told.is_some_and(|t| t.merge == Some("merged") && t.tip == now.tip) {
        return None;
    }
    let known = told.is_some_and(Told::mergeable)
        && [told, heard.judged.as_ref()].into_iter().flatten().any(|t| t.tip == now.tip);
    Some(if known { WakeCause::Merged } else { WakeCause::Delivered })
}

/// What changed between what the crown last heard and now, in words:
/// `merge_state needs_rebase → ahead`, `ahead 1 → 3`, `commit 1a2b3c4`,
/// `column REVIEW`. With nothing heard before, the state itself. A branch
/// merged again at a new tip says `merge_state merged` once more, and a
/// merged branch's `ahead` is not said: an ff merge reads 0 and a squash
/// keeps its count, and neither is news beside the word. A branch of more
/// than one leg says each leg that moved (`ahead api 1 → 2`), never a bare
/// sum (T-772), and its first `ahead` names the legs ahead (`merge_state
/// ahead api 1, web 1`, T-782).
pub(super) fn delta(before: Option<&Told>, now: &Told) -> Vec<String> {
    changes(before, now, true)
}

/// `delta`, with a checkout's new HEAD said only where `head` is: a turn
/// that moved no work did not make it (T-591).
fn changes(before: Option<&Told>, now: &Told, head: bool) -> Vec<String> {
    let mut out = Vec::new();
    match (before.and_then(|b| b.merge), now.merge) {
        (Some(was), Some(is)) if was != is => out.push(format!("merge_state {was} → {is}")),
        (None, Some("ahead")) if !now.legs.is_empty() => {
            let legs: Vec<String> = now
                .legs
                .iter()
                .filter(|(_, a)| *a > 0)
                .map(|(n, a)| format!("{} {a}", mesimon_core::text::scrub_text(n)))
                .collect();
            out.push(format!("merge_state ahead {}", legs.join(", ")));
        }
        (None, Some(is)) => out.push(format!("merge_state {is}")),
        (Some(_), Some("merged")) if before.is_some_and(|b| b.tip != now.tip) => {
            out.push("merge_state merged".into());
        }
        _ => {}
    }
    if let Some(b) = before.filter(|b| b.merge.is_some() && now.merge.is_some()) {
        if now.merge != Some("merged") {
            out.extend(ahead_words(b, now));
        }
    }
    if head && now.merge.is_none() && before.is_none_or(|b| b.tip != now.tip) && !now.tip.is_empty()
    {
        out.push(format!("commit {}", now.tip.get(..7).unwrap_or(&now.tip)));
    }
    if before.is_none_or(|b| b.column != now.column) {
        out.push(format!("column {}", mesimon_core::text::scrub_text(&now.column)));
    }
    // The forge's words (T-739, T-744): each is an edge a pass saw, never
    // a state, so a second `review approved` after a dismissal is said again.
    if let Some(p) = &now.forge {
        out.push(mesimon_core::text::scrub_text(p));
    }
    out
}

/// How far ahead the branch moved since `b`: the count on a single leg;
/// each leg that moved where both sides know the same legs (`ahead api
/// 1 → 2`); else the sum with the leg count (`ahead 2 → 3 over 2 repos`),
/// for a baseline from before T-772 or a leg cut since.
fn ahead_words(b: &Told, now: &Told) -> Vec<String> {
    if now.legs.is_empty() {
        return if b.ahead == now.ahead {
            Vec::new()
        } else {
            vec![format!("ahead {} → {}", b.ahead, now.ahead)]
        };
    }
    let was = |name: &str| b.legs.iter().find(|(n, _)| n == name).map(|(_, a)| *a);
    if b.legs.len() == now.legs.len() && now.legs.iter().all(|(n, _)| was(n).is_some()) {
        return now
            .legs
            .iter()
            .filter_map(|(n, a)| {
                let from = was(n)?;
                (from != *a)
                    .then(|| format!("ahead {} {from} → {a}", mesimon_core::text::scrub_text(n)))
            })
            .collect();
    }
    if b.ahead == now.ahead {
        return Vec::new();
    }
    vec![format!("ahead {} → {} over {} repos", b.ahead, now.ahead, now.legs.len())]
}

/// One worker's two baselines. `judged` is the work as the last looked-at
/// turn left it: a wake's novelty is judged against it, so a merge step's
/// rebased tip is not news on the next idle. `told` is the work as the last
/// wake about it described it: a wake's delta runs from there, so a column
/// a silent turn moved is still said.
///
/// Kept in `crown.json` (T-602) with the wakes owed, so a restart neither
/// loses a wake nor says one twice.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Heard {
    #[serde(skip_serializing_if = "Option::is_none")]
    judged: Option<Told>,
    #[serde(skip_serializing_if = "Option::is_none")]
    told: Option<Told>,
    /// A turn's end held for the merge train (T-554, T-591). `told` does
    /// not move while it is held, so the merge reads as a tip the crown
    /// never heard of.
    #[serde(skip_serializing_if = "Option::is_none")]
    deferred: Option<Held>,
    /// A landing held until the merge step in flight is over (T-596), the
    /// line as it will be said; `told` already moved to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    stepped: Option<CrownWake>,
    /// A turn's end's wake waiting for the forge read that turn owes
    /// (T-773), the line as it will be said; `told` already moved to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    forging: Option<Forging>,
    /// An opening a read saw while this worker's turn probe was out
    /// (T-773): the turn's wake says it.
    #[serde(skip)]
    opening: Option<String>,
    /// What the turn probes out for this worker were asked for (T-602):
    /// a probe sent on the way down never lands, so the restart's look
    /// judges the turn it was for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) unjudged: Option<Unjudged>,
    /// Turn probes out for this worker. While one is, the train is not
    /// judged on what came before it: the turn that just ended may have
    /// moved the branch.
    #[serde(skip)]
    looks: u32,
    /// Read back from `crown.json` and not looked at since (T-602): a wake
    /// owed now may be late, and says `after a restart`.
    #[serde(skip)]
    pub(super) restored: bool,
}

/// A raised hand the crown was told of, kept for its reason (T-922). The
/// telling lowers the hand (T-861), and the wake's sentence names the hand
/// and never its words (T-414), so without this the crown that was woken to
/// read the reason found none. Not a mark: nothing counts it and nothing
/// lights for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ToldHand {
    /// `Raised::reason`, as the worker gave it.
    pub(super) reason: String,
    /// When the wake that named it reached the crown.
    pub(super) told_ms: u64,
}

/// A turn's end the crown's ledger has not judged yet (T-602): what its
/// probe was for, kept until the look lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Unjudged {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) asked: Option<TurnAsk>,
    pub(super) fresh: bool,
}

impl Unjudged {
    /// Two ends owed one look: a turn ran if either says so, and the
    /// crown's ask is kept over a merge step, as `mark_turn` keeps it.
    pub(super) fn join(self, other: Unjudged) -> Unjudged {
        let asked = match (self.asked, other.asked) {
            (Some(a), _) | (_, Some(a)) if a.crown().is_some() => Some(a),
            (a, b) => a.or(b),
        };
        Unjudged { asked, fresh: self.fresh || other.fresh }
    }
}

impl Heard {
    /// The crown is owed this worker's merge: a wake described its work,
    /// or a delivery is held for the train.
    pub(super) fn awaits_merge(&self) -> bool {
        self.told.is_some() || self.deferred.is_some()
    }

    /// Hold a landing — a merge, or a delivery — for the merge step in
    /// flight on the worker (T-596): both baselines move to it now, so the
    /// crown hears it once, and the line waits in `stepped`. A second one
    /// before the step is over folds in, as two wakes owed to a working
    /// crown do. True the first time. `late` is the caller's own reading
    /// that the news is owed across a restart (a forge read's, T-937): it
    /// rides the held line, a fold's included, beside `restored`.
    fn hold_for_step(
        &mut self,
        worker: ulid::Ulid,
        cause: WakeCause,
        now: Told,
        late: bool,
    ) -> bool {
        self.judged = Some(now.clone());
        self.deferred = None;
        // A merge (T-1071) where the branch reads merged at a tip the crown
        // was not told merged: a delivery held while a step runs, its branch
        // still ahead, is none, and nor is a step that never landed.
        let merge = now.merge == Some("merged")
            && !self.told.as_ref().is_some_and(|t| t.merge == Some("merged") && t.tip == now.tip);
        let from = self.told.replace(now.clone());
        if let Some(w) = self.stepped.as_mut() {
            w.absorb(cause, from, Some(now), Owed { late, merge, ..Owed::default() });
            return false;
        }
        let owed = Owed { finished: true, late: self.restored || late, merge, ..Owed::default() };
        self.stepped = Some(CrownWake::new(worker, cause, from, Some(now), owed));
        true
    }

    /// Read back from `crown.json` (T-1004): a landing held for its merge
    /// step is told after the restart, at that step's end, and says so.
    pub(super) fn restored_step(&mut self) {
        if let Some(w) = self.stepped.as_mut() {
            w.late = true;
        }
    }

    /// A merge step's mark taken on an edge that proves no end (T-1004,
    /// round 2): a guess, a dialog, a seat gone. The held landing will say
    /// the merge alone, and the turn's real end, if one comes, says it
    /// finished.
    fn unproven_step(&mut self) {
        if let Some(w) = self.stepped.as_mut() {
            w.finished = false;
        }
    }

    /// The merge step is over: the landing it held, with the column the
    /// turn left the card in (`None`: the ticket is gone, and takes what
    /// was held with it), which is also what the crown has now heard.
    fn step_over(&mut self, column: Option<String>) -> Option<CrownWake> {
        let mut wake = self.stepped.take()?;
        self.land(&mut wake.to, column?);
        Some(wake)
    }

    /// The column a wake's branch is read in, set on the wake and on what
    /// the crown was told of that same tip, which it has now heard.
    fn land(&mut self, to: &mut Option<Told>, column: String) {
        if let Some(to) = to.as_mut() {
            if let Some(told) = self.told.as_mut().filter(|t| t.tip == to.tip) {
                told.column.clone_from(&column);
            }
            to.column = column;
        }
    }
}

/// A turn's end held for the merge train: what it would have woken the
/// crown for — a delivery, or a finished turn (T-591) — and the branch as
/// that turn, or the latest turn since, left it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Held {
    cause: WakeCause,
    look: BranchLook,
}

/// A turn's end's wake held for the forge read the turn owes (T-773), and
/// when the hold began.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Forging {
    wake: CrownWake,
    since_ms: u64,
}

/// How long a turn's end's wake waits for its forge read (T-773). A burst
/// read is one CLI call per repository, under `forge::BURST_MS`'s floor;
/// past this the crown hears the turn without it, and the PR rides
/// `get_ticket` once the read lands.
pub(super) const FORGE_WAIT_MS: u64 = 15_000;

/// Whether a turn's end's wake still waits (T-773): a read is `awaited`
/// (`forge::State::awaited`) and the hold began under `FORGE_WAIT_MS` ago.
pub(super) fn forge_holds(awaited: bool, since_ms: u64, now_ms: u64) -> bool {
    awaited && now_ms.saturating_sub(since_ms) < FORGE_WAIT_MS
}

/// Put the forge's words first in a wake's: an opening comes before what
/// the read says of the PR it opened.
fn say_first(to: &mut Option<Told>, words: String) {
    if let Some(to) = to.as_mut() {
        to.forge = Some(match to.forge.take() {
            Some(rest) => format!("{words}, {rest}"),
            None => words,
        });
    }
}

/// What a probe was for.
#[derive(Clone, Copy, Debug)]
pub(super) enum ProbeWhy {
    /// A worker's turn ended, having taken these words. `fresh`: a turn
    /// ran since the last end looked at (`Daemon::turns_open`), so this
    /// end is a finished turn and not an idle re-entered (T-591).
    Turn { asked: Option<TurnAsk>, fresh: bool },
    /// The first look at a worker after a daemon restart (T-602), for the
    /// turn the last daemon did not judge, if any: what changed while no
    /// daemon was looking — a merge among it — is said once, `after a
    /// restart`.
    Restart { asked: Option<TurnAsk>, fresh: bool },
    /// The crown just started a checkout worker: its HEAD now is the line
    /// the first turn's commits are judged against.
    Baseline,
}

/// Where a probe looks, gathered on the writer and answered on a worker.
enum Look {
    /// A worktree ticket: its binding and one query per repository its legs
    /// live in, the base resolved on the worker where the cache has none.
    Branch {
        binding: Box<Binding>,
        repo: std::path::PathBuf,
        base: Option<String>,
        selected_target: Option<String>,
        queries: Vec<worktree::RepoQuery>,
    },
    /// Anything else: the checkout the worker runs in.
    Checkout(std::path::PathBuf),
}

/// The work as git saw it; the column is added on landing.
#[derive(Clone, Debug)]
pub(super) enum Found {
    Branch(BranchLook),
    Checkout {
        head: String,
    },
    /// Git could not say: not a repository, no base, a branch gone.
    Unknown,
}

/// A probe's answer, landing as `Msg::TurnProbed`.
pub(super) struct TurnProbe {
    worker: ulid::Ulid,
    /// The crown the probe was asked for; another one wearing it by the
    /// time it lands is owed nothing.
    crown: ulid::Ulid,
    why: ProbeWhy,
    gen: u64,
    found: Found,
}

impl Look {
    /// Run the look: the same flags sample the tick takes, for one ticket's
    /// legs, or one `rev-parse` of the checkout's HEAD.
    fn run(self, worker: ulid::Ulid) -> Found {
        match self {
            Look::Branch { binding, repo, base, selected_target, mut queries } => {
                // The root's default branch for a single-repo leg; a
                // workspace leg carries its own, so a folder root with no
                // branch is looked at all the same (T-754).
                let base = base.or_else(|| {
                    repo.join(".git")
                        .exists()
                        .then(|| worktree::default_branch(&repo).ok())
                        .flatten()
                });
                for q in &mut queries {
                    if q.base.is_empty() {
                        if let Some(b) = &base {
                            q.base = b.clone();
                        }
                    }
                }
                queries.retain(|q| !q.base.is_empty());
                if queries.is_empty() {
                    return Found::Unknown;
                }
                let samples = worktree::compute_repo_flags(&queries);
                let legs: Vec<worktree::RepoFlags> = binding
                    .legs_with_target(&repo, &base.unwrap_or_default(), selected_target.as_deref())
                    .iter()
                    .filter_map(|leg| {
                        let s =
                            samples.iter().find(|s| s.name == leg.name && s.base == leg.base)?;
                        let f = s.flags.flags.iter().find(|f| f.ticket == worker)?;
                        Some(worktree::RepoFlags::of(leg, s, f, &binding.branch))
                    })
                    .collect();
                if legs.is_empty() {
                    return Found::Unknown;
                }
                let a = worktree::aggregate(&legs);
                Found::Branch(BranchLook {
                    tip: a.tip,
                    base_tip: a.base_tip,
                    ahead: a.ahead,
                    legs: leg_aheads(&legs),
                    merged: a.merged,
                    needs_rebase: a.needs_rebase,
                    conflict: a.conflict,
                })
            }
            Look::Checkout(cwd) => {
                let head = crate::git::git(&cwd)
                    .args(["rev-parse", "--verify", "--quiet", "HEAD"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                if head.is_empty() {
                    Found::Unknown
                } else {
                    Found::Checkout { head }
                }
            }
        }
    }
}

impl Daemon {
    /// The worker's claude was started by a crown, `crown` wearing it now:
    /// what an earlier crown started is this one's (T-927), as its
    /// `ask_agent` and `sleep_agent` already took it. The board's own
    /// predicate, which the person's banners read too (T-678).
    fn started_by_crown(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.board.crown_holder().is_some_and(|t| t.id == crown)
            && self.board.started_by_crown(worker)
    }

    /// THIS crown watches the ticket (T-712): `watch_ticket` put it on the
    /// list, and `crown` still wears it. The person's banners do not read
    /// this one: a watched ticket is a person's, and its news stays theirs
    /// too.
    fn watched_by_crown(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.board.crown_holder().is_some_and(|t| t.id == crown)
            && self.crown_watched.contains(&worker)
    }

    /// THIS crown holds the ticket out of the merge train (`hold_ticket`,
    /// T-919), `crown` still wearing it: the receipt said its delivery
    /// wakes this session, so it does, whoever started its agent (T-927).
    /// A hold is heard as a watch is; a person's `t` makes it the person's,
    /// and the crown's hearing goes with it.
    fn held_by_crown(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.board.crown_holder().is_some_and(|t| t.id == crown)
            && self.crown_held.contains(&worker)
    }

    /// The crown is owed the worker's news: a crown started the worker, it
    /// watches the ticket (T-712) or holds it out of the train (T-927), or
    /// the ticket has no agent and carries a pull request
    /// (`unattended_pr`, T-890). The one claim every wake but an answer
    /// makes — `note_crown_wake` narrows a watch and a hold to what a
    /// person's agent gives the crown to act on. Never a ticket off the
    /// board (T-1076): an archived or snoozed ticket has no turn to end,
    /// so its news is the forge's and the flags', which the board still
    /// reads for its card and its reclaim and tells no crown.
    fn crown_hears(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.board.ticket(worker).is_some_and(|t| !t.is_archived())
            && (self.started_by_crown(worker, crown)
                || self.watched_by_crown(worker, crown)
                || self.held_by_crown(worker, crown)
                || self.unattended_pr(worker, crown))
    }

    /// Forge news owed on a ticket archived or snoozed since (T-1076): a
    /// wake queued, restored from `crown.json` or given back by a box the
    /// crown lost (`lose_entering`, which the drain runs first) that is the
    /// forge's alone is not told, as `crown_hears` would not
    /// owe it now. A wake whose cause is a turn's, a merge or a hand stays;
    /// one the crown's box already holds is told.
    pub(super) fn drop_archived_forge(&mut self) -> bool {
        let board = &self.board;
        let off = |w: &CrownWake| {
            w.cause == WakeCause::Forge && board.ticket(w.worker).is_some_and(|t| t.is_archived())
        };
        if !self.crown_wakes.iter().any(off) {
            return false;
        }
        self.crown_wakes.retain(|w| !off(w));
        true
    }

    /// A pull request on a ticket with no agent (T-890): the crown did the
    /// work itself and named the PR on the ticket it was for, or the
    /// worker's seat went before the PR landed. No agent on the board is
    /// owed the forge's news for it, so the crown is, as for a ticket it
    /// watches, behind the same switch (`crown_watches`), and never for its
    /// own ticket. A parked agent holds its seat, so its ticket is heard by
    /// provenance as before.
    fn unattended_pr(&self, ticket: ulid::Ulid, crown: ulid::Ulid) -> bool {
        ticket != crown
            && self.board.crown_watches
            && self.board.crown_holder().is_some_and(|t| t.id == crown)
            && self.board.live_agent(ticket).is_none()
            && self.forge.carries_pr(ticket)
    }

    /// The words just pasted or parked on `ticket`'s claude were asked for
    /// this reason: stamped on the owed entry, so its ack — the words
    /// reaching the agent — is what marks the turn. The crown's ask wins
    /// over a merge step on the same entry.
    pub(super) fn tag_owed(&mut self, ticket: ulid::Ulid, ask: TurnAsk) {
        if let Some(o) = self.owed.values_mut().find(|o| o.ticket == ticket) {
            if o.asked.is_none() || ask.crown().is_some() {
                o.asked = Some(ask);
            }
        }
    }

    /// The owed words reached the agent: the turn now running is theirs.
    pub(super) fn mark_turn(&mut self, ticket: ulid::Ulid, ask: TurnAsk) {
        // The turn the crown's ask begins is the one its `expect_secs`
        // times (T-973).
        if let TurnAsk::Asked { ask: id, .. } = ask {
            self.expect_began(ticket, Some(id), None, now_ms());
        }
        match ask {
            TurnAsk::Crown(_) | TurnAsk::Asked { .. } => {
                self.turn_asks.insert(ticket, ask);
            }
            TurnAsk::Merge => {
                self.turn_asks.entry(ticket).or_insert(ask);
            }
        }
    }

    /// A worker's turn ended (`apply_change`): what the turn was asked for
    /// goes with it, and on an `EndTurn` a probe goes out to see what it
    /// left, carrying whether a turn ran since the last one — taken here,
    /// so an idle re-entered with no turn between finishes nothing (T-591).
    /// Only while a crown is worn, never for the crown's own ticket, and
    /// only for a worker a crown started, a ticket it watches or holds, or
    /// a turn that took its ask.
    pub(super) fn turn_ended(&mut self, worker: ulid::Ulid, end_turn: bool) {
        // A worker the restart has yet to look at (T-602): this end is that
        // look when its state is committed — a hook's, or the close in the
        // gap T-663 proves at Medium. The tail's guess at Low is no end at
        // all (T-756): it read a worker mid-turn as idle, its branch ahead
        // as a delivery, and woke the crown a second after the restart. The
        // look, the turn and its asks wait for an end that is one.
        let seat = self.board.live_agent(worker);
        if self.crown_recheck.contains_key(&worker) && seat.is_some_and(guessed) {
            return;
        }
        // Only a proven end settles the crown's ask (T-938): a dialog mid-turn,
        // a guess or an observation lost (a wait gone stale `Unknown`) is no
        // end, and the ask the turn rides stays with it for the end that is.
        // A turn a hook says failed or was interrupted is over too (round 4):
        // its ask settles, or it would block the crown's next one for good.
        // Probes, looks and `turns_open` still take only `end_turn`.
        let dialog = seat.is_some_and(|s| matches!(s.state, SessionState::RequiresAction { .. }));
        let ended = end_turn
            || seat.is_some_and(|s| {
                matches!(
                    s.state,
                    SessionState::Failed { .. }
                        | SessionState::Idle { stop_reason: StopReason::Interrupted }
                ) && !guessed(s)
            });
        // A person's Esc ends the turn the crown's ask rode at whatever
        // confidence the board read it (T-1025): on the mod road an
        // interrupt fires no `Stop`, and the engine's own record of it
        // (`[Request interrupted by user]`, the tail's Low hint) was the one
        // end the board ever saw, so a `handed` ask kept its mark and
        // blocked the crown's next ask for six minutes. The guessed-restart
        // return above still comes first; the merge step's unproven hold
        // (T-1004) and the probes keep `ended`.
        let interrupted = seat.is_some_and(|s| {
            s.state == SessionState::Idle { stop_reason: StopReason::Interrupted }
        });
        let settles = ended || interrupted;
        // Read before it is taken, so an edge that keeps the ask, or finds
        // none, leaves the ledger unmarked (T-839).
        let asked = self.turn_asks.get(&worker).copied();
        let keep = (dialog || !settles) && matches!(asked, Some(TurnAsk::Asked { .. }));
        if asked.is_some() && !keep {
            self.turn_asks.remove(&worker);
        }
        // A merge step's mark taken on no end (T-1004, round 2) loses the
        // fact that its turn was never seen to end, so the held landing
        // keeps it.
        if (dialog || !ended) && asked == Some(TurnAsk::Merge) {
            self.unproven_step(worker);
        }
        // The ask kept for the end that is one is this edge's no longer
        // (T-974, round 1): a look or a probe here answers nothing with it,
        // or the real end would answer it a second time.
        let asked = if keep {
            None
        } else if dialog || !settles {
            asked
        } else {
            self.crown_ask_turn_ended(worker, asked)
        };
        // Only a turn's end that is one says the turn finished (T-731): a
        // guessed idle read a model still thinking as done. A proven close
        // in the gap is Medium (T-663).
        if let Some(owed) = self.crown_recheck.remove(&worker) {
            let fresh = end_turn && self.turns_open.remove(&worker);
            let u = owed.join(Unjudged { asked, fresh });
            self.probe_turn(worker, ProbeWhy::Restart { asked: u.asked, fresh: u.fresh });
            return;
        }
        if end_turn {
            let fresh = self.turns_open.remove(&worker);
            self.probe_turn(worker, ProbeWhy::Turn { asked, fresh });
        }
    }

    /// Workers the restored ledger names (T-602), looked at once on the
    /// tick when their agent is settled — idle, parked, on a dialog, or
    /// gone — with the turn the last daemon left unjudged, if any. One
    /// still working, `Unknown` until a hook or the transcript speaks, or
    /// settled only by the tail's guess (T-756), is left to its turn's end
    /// (`turn_ended`); past `RECHECK_MS` it is
    /// let go, its merge is the flags' to hear, and what is owed from then
    /// on is current news: no line says `after a restart` any more.
    pub(super) fn hear_restored(&mut self) -> bool {
        if self.crown_recheck_until == 0 {
            return false;
        }
        let expired = now_ms() >= self.crown_recheck_until;
        let settled = |s: &SessionRecord| {
            !s.pending_submit
                && !guessed(s)
                && !matches!(
                    s.state,
                    SessionState::Unknown { .. }
                        | SessionState::Spawning
                        | SessionState::Running
                        | SessionState::Idle { stop_reason: StopReason::Background }
                )
        };
        let due: Vec<ulid::Ulid> = self
            .crown_recheck
            .keys()
            .copied()
            .filter(|w| self.crown_heard.get(w).is_none_or(|h| h.looks == 0))
            .filter(|w| expired || self.board.live_agent(*w).is_none_or(settled))
            .collect();
        for worker in due {
            let Some(owed) = self.crown_recheck.remove(&worker) else { continue };
            if self.board.live_agent(worker).is_some_and(|s| !settled(s)) {
                // Past the window and still working: its end judges it.
                continue;
            }
            // At a dialog the turn is still open (T-974, round 1): the
            // crown's ask it rides and the open turn stay for the end that
            // is one, as `turn_ended` keeps them, and this look is a
            // baseline that answers nothing. A dialog is never an end.
            let dialog = self
                .board
                .live_agent(worker)
                .is_some_and(|s| matches!(s.state, SessionState::RequiresAction { .. }));
            let (asked, fresh) = if dialog {
                let kept = matches!(self.turn_asks.get(&worker), Some(TurnAsk::Asked { .. }));
                (if kept { None } else { self.turn_asks.remove(&worker) }, false)
            } else {
                (self.turn_asks.remove(&worker), self.turns_open.remove(&worker))
            };
            // A look that is no committed end (a dialog, a seat parked or
            // gone) takes a merge step's mark without proving its turn over
            // (T-1004, round 2).
            let ended = self.board.live_agent(worker).is_some_and(|s| {
                matches!(
                    s.state,
                    SessionState::Idle {
                        stop_reason: StopReason::EndTurn | StopReason::Interrupted
                    } | SessionState::Failed { .. }
                )
            });
            if asked == Some(TurnAsk::Merge) && (dialog || !ended) {
                self.unproven_step(worker);
            }
            let u = owed.join(Unjudged { asked, fresh });
            self.probe_turn(worker, ProbeWhy::Restart { asked: u.asked, fresh: u.fresh });
        }
        if expired && self.crown_recheck.is_empty() {
            for h in self.crown_heard.values_mut().filter(|h| h.looks == 0) {
                h.restored = false;
            }
            self.crown_recheck_until = 0;
        }
        false
    }

    /// What is pending on the worker's ticket now (T-596): the module's
    /// list, each line read once. The train is judged on `look`, the branch
    /// as a turn's look or the train's sample saw it; with none — a merged
    /// branch — it has nothing to take. The merge flow's words are a merge
    /// step from their paste (`tag_owed`) to the end of the turn that took
    /// them (`mark_turn`, taken by `turn_ended`); any other words are words.
    pub(super) fn pending_on(&self, worker: ulid::Ulid, look: Option<&BranchLook>) -> Pending {
        let agent = self.board.live_agent(worker);
        let working = agent.is_some_and(mesimon_core::quiet::is_working);
        let dialog = agent.is_some_and(|s| {
            matches!(
                s.state,
                SessionState::RequiresAction { reason: Reason::Question | Reason::Plan }
            )
        });
        let owed = |merge: bool| {
            self.owed
                .values()
                .any(|o| o.ticket == worker && (o.asked == Some(TurnAsk::Merge)) == merge)
        };
        // The turn that took the merge flow's words runs until an end that is
        // one (T-1004): across a restart its seat reads `Unknown`, then the
        // tail's guess, neither of them working, and the merge's wake went
        // out a second after the restart while the notice's turn ran. The
        // restart's look bounds that reading: a seat still unproven when it
        // settles (`RECHECK_MS`) lets the step go, as an idle one sends no
        // hook to end it, and `hear_stepped` then tells the merge alone.
        let stepping = agent.is_some_and(|s| {
            working
                || (s.state.has_pane()
                    && (rederiving(s) || guessed(s))
                    && self.crown_recheck.contains_key(&worker))
        });
        Pending {
            train: look.is_some_and(|look| self.train_takes(worker, look)),
            merge_step: owed(true)
                || (stepping && self.turn_asks.get(&worker) == Some(&TurnAsk::Merge)),
            // A launch's words that reached no agent (T-570) are still owed
            // to it, by the seat's resend or the restart's (T-722).
            words: owed(false)
                || self.queued.iter().any(|q| q.ticket == worker)
                || agent.is_some_and(|s| s.unsent_words().is_some()),
            turn: working && !dialog,
            hand: self.board.ticket(worker).is_some_and(|t| t.hand_raised()),
            dialog,
        }
    }

    /// Send a look at the worker's work to a thread: the flags sample for a
    /// worktree ticket, HEAD for a checkout. The writer forks nothing; the
    /// answer lands as `Msg::TurnProbed`.
    pub(super) fn probe_turn(&mut self, worker: ulid::Ulid, why: ProbeWhy) {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return };
        if worker == crown {
            return;
        }
        let answered = match why {
            ProbeWhy::Turn { asked, .. } | ProbeWhy::Restart { asked, .. } => {
                asked.and_then(TurnAsk::crown) == Some(crown)
            }
            ProbeWhy::Baseline => false,
        };
        if !answered && !self.crown_hears(worker, crown) {
            return;
        }
        let branch = self.worktrees.get(&worker).filter(|b| !b.branch.is_empty()).cloned();
        let look = match branch {
            // A worktree's first turn is judged by `ahead`, which starts at
            // zero: it needs no baseline.
            Some(_) if matches!(why, ProbeWhy::Baseline) => return,
            Some(binding) => Look::Branch {
                queries: self.wt_queries(false, Some(worker)),
                binding: Box::new(binding),
                repo: self.paths.repo_root.clone(),
                base: self.worktree_base_cached(),
                selected_target: self.configured_worktree_base(),
            },
            None => {
                let Some(cwd) = self.board.live_agent(worker).map(|s| s.cwd.clone()) else {
                    return;
                };
                Look::Checkout(std::path::PathBuf::from(cwd))
            }
        };
        let heard = self.crown_heard.entry(worker).or_default();
        heard.looks += 1;
        if let ProbeWhy::Turn { asked, fresh } | ProbeWhy::Restart { asked, fresh } = why {
            let u = Unjudged { asked, fresh };
            heard.unjudged = Some(heard.unjudged.map_or(u, |o| o.join(u)));
        }
        let tx = self.tx.clone();
        let gen = self.merge_target_gen;
        std::thread::spawn(move || {
            let found = look.run(worker);
            let _ = tx.send(Msg::TurnProbed(TurnProbe { worker, crown, why, gen, found }));
        });
    }

    /// A probe landed: judge it against the worker's baseline, move the
    /// baseline, and owe the crown a wake when the verdict says so, weighed
    /// against what is pending on the ticket (`due`). A delivery or a
    /// finished turn the merge train will take is held instead (T-554,
    /// T-591), and one already held is judged again on what this turn
    /// left: a rebase the train asked for and the agent could not finish
    /// comes due here. What is pending is judged as the look lands, after
    /// the queue has had the turn's end.
    pub(super) fn on_turn_probed(&mut self, p: TurnProbe) {
        let worker = p.worker;
        if p.gen != self.merge_target_gen {
            if let Some(h) = self.crown_heard.get_mut(&worker) {
                h.looks = h.looks.saturating_sub(1);
            }
            if self.board.crown == Some(p.crown) {
                self.probe_turn(worker, p.why);
            }
            return;
        }
        self.judge_probe(p);
        // Looked at since the restart (T-602): what is owed from here is
        // current, and every turn it was asked for is judged. The feed says
        // so once per worker per start (T-937), the fact an e2e waits on
        // before news it expects said as current.
        let mut looked = false;
        if let Some(h) = self.crown_heard.get_mut(&worker) {
            looked = std::mem::take(&mut h.restored);
            if h.looks == 0 {
                h.unjudged = None;
            }
        }
        if looked {
            self.feed.board("automation", "crown_restart_looked", Some(worker));
        }
    }

    fn judge_probe(&mut self, p: TurnProbe) {
        if let Some(h) = self.crown_heard.get_mut(&p.worker) {
            h.looks = h.looks.saturating_sub(1);
        }
        if self.board.crown_holder().map(|t| t.id) != Some(p.crown) {
            return;
        }
        let Some(column) = self.board.ticket(p.worker).map(|t| t.column.clone()) else {
            return;
        };
        let (asked, fresh, restart) = match p.why {
            ProbeWhy::Turn { asked, fresh } => (asked, fresh, false),
            ProbeWhy::Restart { asked, fresh } => (asked, fresh, true),
            ProbeWhy::Baseline => (None, false, false),
        };
        let answered = asked.and_then(TurnAsk::crown) == Some(p.crown);
        let said = AskSaid::of(asked);
        let merge_step = asked == Some(TurnAsk::Merge);
        let (now, branch) = match p.found {
            Found::Branch(look) => (look.told(column), Some(look)),
            Found::Checkout { head } => (
                Told { tip: head, merge: None, ahead: 0, legs: Vec::new(), column, forge: None },
                None,
            ),
            Found::Unknown => {
                // Git could not say what the turn left — nothing seen is
                // nothing to merge — but an answer is still an answer, and
                // a finished turn still finished.
                let cause = verdict(None, &Told::default(), answered, merge_step, fresh);
                if let Due::Wake(cause) = due(cause, None, &self.pending_on(p.worker, None)) {
                    if self.crown_claims(p.worker, p.crown, cause) {
                        self.owe_turn_end(p.crown, p.worker, cause, None, None, said);
                    }
                }
                return;
            }
        };
        let started = self.crown_hears(p.worker, p.crown);
        let heard = self.crown_heard.entry(p.worker).or_default();
        if matches!(p.why, ProbeWhy::Baseline) {
            heard.judged.get_or_insert(now);
            return;
        }
        // A merge no daemon was there to read (T-602) is the restart's to
        // say, judged as the flags would have (T-527) before the baseline
        // moves to it.
        let landed = if restart {
            merge_verdict(started || heard.deferred.is_some(), heard, &now)
        } else {
            None
        };
        let before = heard.judged.replace(now.clone());
        let held = heard.deferred.as_ref().map(|h| h.cause);
        let cause = verdict(before.as_ref(), &now, answered, merge_step, fresh).max(landed);
        // A checkout has no branch for the train: its delivery wakes.
        let cause = match due(cause, held, &self.pending_on(p.worker, branch.as_ref())) {
            Due::Wake(cause) => cause,
            Due::Silent => return,
            Due::Hold(cause) => {
                if held.is_none() {
                    self.feed.board("automation", "crown_wake_deferred", Some(p.worker));
                }
                self.crown_heard.entry(p.worker).or_default().deferred =
                    branch.map(|look| Held { cause, look });
                return;
            }
            Due::Step(cause) => {
                // Held, so the claim is judged now, when the turn's look did.
                if held.is_some() || self.crown_hears(p.worker, p.crown) {
                    self.hold_for_step(p.worker, cause, now, false);
                }
                return;
            }
        };
        let heard = self.crown_heard.entry(p.worker).or_default();
        heard.deferred = None;
        let from = heard.told.replace(now.clone());
        // Held, so the claim was judged when it was: the seat may have gone
        // since.
        if held.is_some() || self.crown_claims(p.worker, p.crown, cause) {
            self.owe_turn_end(p.crown, p.worker, cause, from, Some(now), said);
        }
    }

    /// A turn's end's wake (T-773): owed now, or held for the forge read
    /// the turn owes, which `hear_forged` lets go. An opening a read saw
    /// before this look landed is said first in its forge words.
    fn owe_turn_end(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        mut to: Option<Told>,
        ask: Option<AskSaid>,
    ) {
        let heard = self.crown_heard.entry(worker).or_default();
        if let Some(words) = heard.opening.take() {
            say_first(&mut to, words);
        }
        if !self.forge.awaited(worker) {
            self.owe_wake(crown, worker, cause, from, to, Owed { ask, ..Owed::default() });
            return;
        }
        let heard = self.crown_heard.entry(worker).or_default();
        if let Some(f) = heard.forging.as_mut() {
            if f.wake.awaiting_forge_only() {
                // The older turn was observed. This end is new news, with
                // its own forge wait; it must not inherit the placeholder's
                // stronger Forge cause and be discarded as an empty read.
                f.wake = CrownWake::new(
                    worker,
                    cause,
                    from,
                    to,
                    Owed { ask, late: heard.restored, ..Owed::default() },
                );
                f.since_ms = now_ms();
                return;
            }
            f.wake.absorb(cause, from, to, Owed { ask, ..Owed::default() });
            return;
        }
        let owed = Owed { late: heard.restored, ask, ..Owed::default() };
        let wake = CrownWake::new(worker, cause, from, to, owed);
        heard.forging = Some(Forging { wake, since_ms: now_ms() });
        self.feed.board("automation", "crown_wake_deferred:forge", Some(worker));
    }

    /// Turn ends held for their forge read (T-773) whose read landed, or
    /// that waited `FORGE_WAIT_MS`: each is owed as it was held, without
    /// draining, so a forge wake the same read brings folds into it and
    /// the crown hears one line; the caller drains. `openings` are the
    /// reads' `opened !2`, per ticket: said on a held wake, kept for a
    /// turn whose probe is still out or that is still running (T-782: a
    /// read mid-turn, the minute's, can bind the PR before the turn ends,
    /// and the look after it then owes no read), and otherwise said by no
    /// one. True when one was let go.
    pub(super) fn hear_forged(&mut self, openings: HashMap<ulid::Ulid, String>) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        for (worker, words) in openings {
            let running = self.turns_open.contains(&worker) && self.crown_hears_turn(worker);
            if running {
                self.crown_heard.entry(worker).or_default();
            }
            let Some(h) = self.crown_heard.get_mut(&worker) else { continue };
            if let Some(f) = h.forging.as_mut() {
                say_first(&mut f.wake.to, words);
            } else if h.looks > 0 || running {
                h.opening = Some(match h.opening.take() {
                    Some(was) => format!("{was}, {words}"),
                    None => words,
                });
            }
        }
        let now = now_ms();
        let over: Vec<ulid::Ulid> = self
            .crown_heard
            .iter()
            .filter(|(w, h)| {
                h.forging
                    .as_ref()
                    .is_some_and(|f| !forge_holds(self.forge.awaited(**w), f.since_ms, now))
            })
            .map(|(w, _)| *w)
            .collect();
        let mut woke = false;
        for worker in over {
            let Some(h) = self.crown_heard.get_mut(&worker) else { continue };
            let Some(Forging { wake, .. }) = h.forging.take() else { continue };
            // A keyed observation consumed the turn, but kept the slot for
            // the forge read still out. With no new opening, it owes nothing.
            if wake.awaiting_forge_only() {
                continue;
            }
            let Some(column) = self.board.ticket(worker).map(|t| t.column.clone()) else {
                continue;
            };
            // The column the read's arrival left the card in.
            let mut to = wake.to;
            h.land(&mut to, column);
            let owed =
                Owed { finished: wake.finished, late: wake.late, ask: wake.ask, ..Owed::default() };
            self.enter_wake(crown, worker, wake.cause, wake.from, to, owed);
            woke = true;
        }
        woke
    }

    /// A landing that waits for the merge step in flight on its worker
    /// (T-596), said in the feed the first time. The claim on the worker is
    /// the caller's to have judged.
    fn hold_for_step(&mut self, worker: ulid::Ulid, cause: WakeCause, now: Told, late: bool) {
        if self.crown_heard.entry(worker).or_default().hold_for_step(worker, cause, now, late) {
            self.feed.board("automation", "crown_wake_deferred:merge_step", Some(worker));
        }
    }

    /// `Heard::unproven_step` on the worker's landing held for its step.
    fn unproven_step(&mut self, worker: ulid::Ulid) {
        if let Some(h) = self.crown_heard.get_mut(&worker) {
            h.unproven_step();
        }
    }

    /// Whether the merge train will land this branch from here (T-554),
    /// judged on `look`: it is armed, and the ticket is in a turn — the
    /// rebase the train asked for, or any other, whose own end is judged
    /// again — or on one of the train's two lists by the planner's own
    /// predicate (`train::lane`), with no refusal of this merge on record.
    /// "Will it", not "has it": the train acts on its next sample, seconds
    /// after the turn's end.
    pub(super) fn train_takes(&self, worker: ulid::Ulid, look: &BranchLook) -> bool {
        if !self.train.is_armed() || self.worktrees_barred || !self.base_known_for(worker) {
            return false;
        }
        let Some(t) = self.board.ticket(worker).filter(|t| !t.is_archived()) else {
            return false;
        };
        let Some(binding) = self.worktrees.get(&worker) else { return false };
        if self.board.live_agent(worker).is_some_and(rederiving)
            || self.working(None).contains(&worker)
        {
            return true;
        }
        let flags = mesimon_core::train::WtFlags {
            attached: binding.status == BindingStatus::Attached,
            ahead: look.ahead,
            merged: look.merged,
            needs_rebase: look.needs_rebase,
            conflict: look.conflict,
            // On a board that merges through pull requests the forge merges
            // and the train asks only a conflict's rebase (T-810), so the
            // crown hears the delivery.
            has_pr: self.forge.has_pr(worker),
            pr_conflict: self.forge.pr_conflict(worker),
        };
        let asked = self.train.asked_tips();
        // A look the merge outran (T-1037): taken at the tip that merged,
        // before the merge, it lands after `on_merged` moved the card on.
        // It is judged in the column the merge found the ticket in, so a
        // delivery the train took is held for the landing's wake there too.
        let column = match self.merged_out_of.get(&worker) {
            Some((from, tip)) if !look.merged && *tip == look.tip => from.as_str(),
            _ => t.column.as_str(),
        };
        let reading = mesimon_core::train::Reading {
            merge_into: self.board.merge_into.clone(),
            reach: self.board.column(column).map(|c| c.settings.train).unwrap_or_default(),
            flags: &flags,
            seat: self.train_seat(worker),
            base_tip: &look.base_tip,
            asked: asked.get(&worker).map(String::as_str),
            conflict_asked: self.train.conflict_asked().contains(&worker),
            fused: self.train.is_fused(worker),
        };
        match mesimon_core::train::lane(t, &reading) {
            Some(mesimon_core::train::Lane::Merge) => {
                self.train.refusal(worker, &look.tip, &look.base_tip).is_none()
            }
            Some(mesimon_core::train::Lane::Rebase) => true,
            None => false,
        }
    }

    /// The branch as the train's last sample saw it, folded like a look.
    pub(super) fn sampled(&self, worker: ulid::Ulid) -> Option<BranchLook> {
        let branch = &self.worktrees.get(&worker)?.branch;
        Some(BranchLook {
            tip: self.wt_agg.get(&worker)?.tip.clone(),
            base_tip: self.base_tip_of(worker).to_string(),
            ahead: self.wt_agg.get(&worker).map_or(0, |a| a.ahead),
            legs: self.wt_repos.get(&worker).map(|l| leg_aheads(l)).unwrap_or_default(),
            merged: self.wt_agg.get(&worker).is_some_and(|a| a.merged),
            needs_rebase: self.wt_agg.get(&worker).is_some_and(|a| a.needs_rebase),
            conflict: self.wt_conflicts.contains(branch),
        })
    }

    /// Deliveries and finished turns held for the merge train that it will
    /// not take after all (T-554, T-591), on the tick: the train was
    /// disarmed, the merge was refused, the rebase it asked for left the
    /// branch behind the same base tip, the fuse blew, a person took the
    /// ticket off the train or out of its columns, or parked its agent. Each
    /// wakes the crown as what it was held as, its delta running to now. A
    /// held branch that reads merged here was missed by `hear_merges` and is
    /// said the same way. Judged on the train's own sample once that has
    /// reached the held tip — before then the sample predates the turn and
    /// the turn's look is the truth — and not at all while a probe is out,
    /// nor before the restart's look (T-776): a hold read back from
    /// `crown.json` is the restart's look's to judge, at a turn's end that
    /// is one, as `hear_merges` waits for it.
    /// A ticket gone from the board takes what was held with it.
    pub(super) fn hear_deferred(&mut self) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let held: Vec<(ulid::Ulid, Held)> = self
            .crown_heard
            .iter()
            .filter(|(w, h)| h.looks == 0 && !self.crown_recheck.contains_key(w))
            .filter_map(|(w, h)| Some((*w, h.deferred.clone()?)))
            .collect();
        let mut woke = false;
        for (worker, Held { cause, look }) in held {
            let Some(column) =
                self.board.ticket(worker).filter(|t| !t.is_archived()).map(|t| t.column.clone())
            else {
                if let Some(h) = self.crown_heard.get_mut(&worker) {
                    h.deferred = None;
                }
                continue;
            };
            let seen = self.sampled(worker).filter(|s| s.tip == look.tip).unwrap_or(look);
            let now = seen.told(column);
            match due(None, Some(cause), &self.pending_on(worker, Some(&seen))) {
                Due::Wake(cause) => {
                    let heard = self.crown_heard.entry(worker).or_default();
                    heard.deferred = None;
                    let from = heard.told.replace(now.clone());
                    self.owe_crown_wake(crown, worker, cause, from, Some(now), false);
                    woke = true;
                }
                Due::Step(cause) => self.hold_for_step(worker, cause, now, false),
                Due::Hold(_) | Due::Silent => {}
            }
        }
        woke
    }

    /// Landings held for a merge step (T-596) whose step is over: the words
    /// were taken and their turn ended, or they never reached the agent (a
    /// paste given up on, a pane that died), or its mark survives to the
    /// release, a seat the restart's look never saw end (T-1004), on the
    /// tick. Each wakes the crown as it was held, saying the worker finished
    /// its turn unless no end was proven (a surviving mark, or one taken on
    /// no end: `unproven_step`), with the
    /// column the turn left the card in. Not while a probe is out: the
    /// turn that just ended is still being looked at. A ticket gone from
    /// the board takes what was held with it.
    pub(super) fn hear_stepped(&mut self) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let over: Vec<ulid::Ulid> = self
            .crown_heard
            .iter()
            .filter(|(_, h)| h.stepped.is_some() && h.looks == 0)
            .map(|(w, _)| *w)
            .filter(|w| !self.pending_on(*w, None).merge_step)
            .collect();
        let mut woke = false;
        for worker in over {
            // A surviving mark (T-1004): the step is let go with its mark
            // still on, so no end took it and the seat was never seen to end
            // (the restart's look bounded it). The board knows the merge and
            // not the end, so the line says the merge alone, and the turn
            // stays open for its real end to say it finished. The mark goes,
            // or it would read that end, or the next turn's, as this merge's
            // step and silence it.
            let surviving = self.turn_asks.get(&worker) == Some(&TurnAsk::Merge);
            if surviving {
                self.turn_asks.remove(&worker);
            }
            let column = self.board.ticket(worker).map(|t| t.column.clone());
            let Some(heard) = self.crown_heard.get_mut(&worker) else { continue };
            let Some(mut wake) = heard.step_over(column) else { continue };
            wake.finished &= !surviving;
            let held = Owed {
                finished: wake.finished,
                late: wake.late,
                ask: wake.ask,
                merge: wake.merge,
                ..Owed::default()
            };
            self.owe_wake(crown, worker, wake.cause, wake.from, wake.to, held);
            woke = true;
        }
        woke
    }

    /// Branches the worktree flags just read `merged` (`crown_landed`,
    /// filled by `absorb_worktree_flags`): the one place `m`, the train and
    /// a merge made in a terminal all end. Heard on the tick rather than
    /// inside the refresh that saw them, so the line says the column the
    /// merge flow left the card in; a reading that fell back before the
    /// tick was no merge. Both baselines move to the merge, so a turn probe
    /// that read the branch just before it is not a second delivery. A
    /// merge whose notice is on its way to the worker, or whose notice turn
    /// is running, waits for that turn to end (T-596): the train pastes it
    /// in the pass that merged, so it is on its way by this tick.
    pub(super) fn hear_merges(&mut self) -> bool {
        if self.crown_landed.is_empty() {
            return false;
        }
        let landed = std::mem::take(&mut self.crown_landed);
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let mut woke = false;
        for worker in landed {
            // The restart's first look at the worker says a merge made while
            // no daemon was looking (T-602); this reading waits for it.
            let looking = self.crown_recheck.contains_key(&worker)
                || self.crown_heard.get(&worker).is_some_and(|h| h.restored && h.looks > 0);
            if looking {
                self.crown_landed.push(worker);
                continue;
            }
            if !self.wt_agg.get(&worker).is_some_and(|a| a.merged) {
                continue;
            }
            let Some(column) = self.board.ticket(worker).map(|t| t.column.clone()) else {
                continue;
            };
            let now = Told {
                tip: self.wt_agg.get(&worker).map(|a| a.tip.clone()).unwrap_or_default(),
                merge: Some("merged"),
                ahead: self.wt_agg.get(&worker).map_or(0, |a| a.ahead),
                legs: self.wt_repos.get(&worker).map(|l| leg_aheads(l)).unwrap_or_default(),
                column,
                forge: None,
            };
            let heard = self.crown_heard.get(&worker).cloned().unwrap_or_default();
            // A delivery held for the train is this crown's to hear (T-554),
            // whatever became of the seat since.
            let started =
                worker != crown && (heard.deferred.is_some() || self.crown_hears(worker, crown));
            // A merge read while this crown does not hear the worker (T-936)
            // still moves the baselines it has: they would read it as news
            // once the crown hears the worker again (a watch, a new hold, a
            // restart's look). Only the baselines: nothing is told, no hand
            // comes down, and no entry is made for a worker never heard.
            if !started {
                if let Some(h) = self.crown_heard.get_mut(&worker) {
                    h.judged = Some(now.clone());
                    h.told = Some(now);
                }
                continue;
            }
            let cause = merge_verdict(started, &heard, &now);
            match due(cause, None, &self.pending_on(worker, None)) {
                Due::Step(cause) => self.hold_for_step(worker, cause, now, false),
                Due::Wake(cause) => {
                    let heard = self.crown_heard.entry(worker).or_default();
                    heard.judged = Some(now.clone());
                    heard.deferred = None;
                    let from = heard.told.replace(now.clone());
                    let owed = Owed { merge: true, ..Owed::default() };
                    self.owe_wake(crown, worker, cause, from, Some(now), owed);
                    woke = true;
                }
                Due::Hold(_) | Due::Silent => {}
            }
            if let Some(h) = self.crown_heard.get_mut(&worker) {
                h.restored = false;
            }
        }
        woke
    }

    /// Record that the crown is owed a wake about `worker`, and try to
    /// deliver it now. Only while the crown is worn, and — an answer aside,
    /// which the crown asked for — only for an agent a crown started, or
    /// a ticket it watches (T-712) or holds (T-927), whose delivery, finished turn, hand,
    /// usage limit (T-802) and merge are the crown's to hear and whose question or plan is not: a
    /// person's agent is answered by the person, and its card's needs-you
    /// already calls them. The feed line names both tickets and the cause,
    /// never the sentence; the crown's card lights `woke` the way it lights
    /// for the crown's own touches.
    pub(super) fn note_crown_wake(
        &mut self,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
    ) {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return };
        if self.crown_claims(worker, crown, cause) {
            self.owe_crown_wake(crown, worker, cause, from, to, false);
        }
    }

    /// A successful person promotion wakes the current crown with the exact
    /// target and repository legs that advanced.
    pub(super) fn note_promotion_wake(&mut self, target: &str, legs: &[String]) {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return };
        let detail = format!("{target} → main ({})", legs.join(", "));
        self.enter_wake(crown, crown, WakeCause::Promotion, None, None, Owed::default());
        if let Some(wake) = self.crown_wakes.iter_mut().find(|wake| wake.worker == crown) {
            wake.cause = WakeCause::Promotion;
            wake.promotion = Some(detail);
        }
        self.drain_crown_wakes(None);
        self.broadcast();
    }

    /// `note_crown_wake`'s guards: never the crown's own ticket, and, an
    /// answer aside, a worker a crown started, or a watched or held
    /// ticket's news.
    pub(super) fn crown_claims(
        &self,
        worker: ulid::Ulid,
        crown: ulid::Ulid,
        cause: WakeCause,
    ) -> bool {
        if worker == crown {
            return false;
        }
        if cause == WakeCause::Answered || self.started_by_crown(worker, crown) {
            return true;
        }
        let watched_news = matches!(
            cause,
            WakeCause::Delivered
                | WakeCause::Finished
                | WakeCause::Merged
                | WakeCause::Raised
                | WakeCause::Limited
                | WakeCause::Failed
        );
        watched_news && (self.watched_by_crown(worker, crown) || self.held_by_crown(worker, crown))
    }

    /// A turn ended on `ticket`'s agent that the crown hears (T-773): its
    /// look is taken, train or not, for the forge read the wake waits on.
    pub(super) fn crown_hears_turn(&self, ticket: ulid::Ulid) -> bool {
        self.board
            .crown_holder()
            .map(|t| t.id)
            .is_some_and(|crown| ticket != crown && self.crown_hears(ticket, crown))
    }

    /// `note_crown_wake` past its guards: for a caller that judged the
    /// crown's claim on the worker itself. `finished`: the wake waited for
    /// its merge step's turn to end (T-596), and a fold keeps saying so.
    fn owe_crown_wake(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
        finished: bool,
    ) {
        self.owe_wake(crown, worker, cause, from, to, Owed { finished, ..Owed::default() });
    }

    /// `owe_crown_wake`, with what rides the wake beside its cause: a
    /// lingering wake's numbers (T-599), and whether it was owed across a
    /// restart (T-602) — so is any wake about a worker no look has reached
    /// since one.
    pub(super) fn owe_wake(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
        owed: Owed,
    ) {
        self.enter_wake(crown, worker, cause, from, to, owed);
        self.drain_crown_wakes(None);
        self.broadcast();
    }

    /// `owe_wake` without the delivery: the wake is entered and said in the
    /// feed, and the caller drains.
    fn enter_wake(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
        owed: Owed,
    ) {
        let late = owed.late || self.crown_heard.get(&worker).is_some_and(|h| h.restored);
        let owed = Owed { late, ..owed };
        let ask = owed.ask;
        // A wake that says a merge (T-1071): one its caller says is, or a
        // branch reading merged where the crown last heard it unmerged, or
        // merged at another tip. A branch that stays merged is no merge: a
        // later PR's news on it said nothing new of the branch, and ended
        // the watch before that PR's own merge.
        let landed = CrownWake::new(worker, cause, from.clone(), to.clone(), owed).says_merge();
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            // A proven end folding into merge news owed ahead of it (T-1004,
            // round 3): the merge was told alone, its turn not seen to end,
            // and this is that end. Not the other way: merge news after a
            // finished turn is no end of its own.
            if cause == WakeCause::Finished
                && matches!(w.cause, WakeCause::Merged | WakeCause::Delivered)
            {
                w.finished = true;
            }
            w.absorb(cause, from, to, owed);
        } else {
            self.crown_wakes.push(CrownWake::new(worker, cause, from, to, owed));
        }
        // A watch ends at the merge (T-712): the wait it was for is over,
        // and the line that says so is its last.
        if landed {
            self.end_merge_watch(worker);
        }
        let named = ask.filter(|_| cause == WakeCause::Answered);
        let named = named.map(|a| format!("{} {}", a.ask, a.said.word()));
        self.feed.crown_wake(crown, worker, cause.word(), named);
        self.crown_mark(worker, crown, "woke");
    }

    /// A start the crown asked for could not be made (T-777): its worktree
    /// failed to cut, and `why` is the card's words for it. The crown hears
    /// it at once, with nothing pending to weigh, since no agent exists.
    pub(super) fn hear_unstarted(&mut self, crown: ulid::Ulid, worker: ulid::Ulid, why: String) {
        if worker == crown {
            return;
        }
        let why = mesimon_core::text::scrub_text(&why);
        let why = mesimon_core::text::cap_bytes(&why, UNSTARTED_MAX_BYTES).to_string();
        self.expect_unstarted(worker);
        self.owe_said(crown, worker, WakeCause::Unstarted, |w| w.unstarted = Some(why));
    }

    /// `owe_wake` for news with nothing before it, whose words `say` sets
    /// on the wake once it is entered.
    fn owe_said(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        say: impl FnOnce(&mut CrownWake),
    ) {
        self.enter_wake(crown, worker, cause, None, None, Owed::default());
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            say(w);
        }
        self.drain_crown_wakes(None);
        self.broadcast();
    }

    /// A worker's turn ended on an error (T-936): the crown that hears it
    /// is woken once per failed turn (`session`, `turn`), the reason as the
    /// board saw it on the wake. A refined reason moves the one still owed
    /// for that same turn, and wakes nobody. A failure is no turn's end the
    /// board looks at: it says nothing delivered or finished.
    pub(super) fn hear_failure(
        &mut self,
        worker: ulid::Ulid,
        session: uuid::Uuid,
        turn: String,
        failure: Failure,
    ) {
        let failed = FailedTurn { session, turn, reason: failure.reason };
        if !failure.new {
            let same = |f: &FailedTurn| f.session == failed.session && f.turn == failed.turn;
            if let Some(w) = self
                .crown_wakes
                .iter_mut()
                .find(|w| w.worker == worker && w.failed.as_ref().is_some_and(same))
            {
                w.failed = Some(failed);
            }
            return;
        }
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return };
        if !self.crown_claims(worker, crown, WakeCause::Failed) {
            return;
        }
        self.owe_said(crown, worker, WakeCause::Failed, |w| w.failed = Some(failed));
    }

    /// Workers the crown started that have sat idle at their composer with
    /// background tasks for `LINGER_MS` with no foreground turn (T-599), on
    /// the tick: the crown is woken once per stretch, `has been idle with 3
    /// background tasks for 30 min`, and nothing else happens — no merge,
    /// no park, no kill. A stretch ends only with a foreground turn
    /// (`apply_change` forgets the record on `Running`), so a task the
    /// agent arms or ends without a turn does not make a second wake.
    /// Either park on tasks is one (T-944): a background command's park
    /// reads `Monitoring` after `attention::PARK_STALE_MS`, and the stretch
    /// is timed from the turn's end (`tasks_since`), not from that demote.
    pub(super) fn hear_lingering(&mut self) -> bool {
        let sessions = &self.board.sessions;
        let known = |id: &uuid::Uuid| sessions.iter().any(|s| s.id == *id);
        // Asked before it is written: a `&mut` marks the ledger (T-839).
        if !self.lingered.iter().all(known) {
            self.lingered.retain(|id| known(id));
        }
        // A record that left its park by a road other than `apply_change`
        // (a sleep, a death) leaves no stretch behind.
        let parked = |id: &uuid::Uuid| sessions.iter().any(|s| s.id == *id && s.state.on_tasks());
        self.tasks_since.retain(|id, _| parked(id));
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let now = now_ms();
        let after = super::linger_ms();
        let due: Vec<(uuid::Uuid, ulid::Ulid, Linger)> = self
            .board
            .sessions
            .iter()
            .filter(|s| !self.lingered.contains(&s.id))
            .filter_map(|s| {
                let since = self.tasks_since.get(&s.id).copied();
                linger_due(s, crown, since, now, after).map(|l| (s.id, s.ticket, l))
            })
            .collect();
        for (id, worker, linger) in &due {
            self.lingered.insert(*id);
            let owed = Owed { linger: Some(*linger), ..Owed::default() };
            self.owe_wake(crown, *worker, WakeCause::Lingering, None, None, owed);
        }
        !due.is_empty()
    }

    /// A worker's pull request has news (T-739, T-744): the forge pass saw
    /// an edge — a pipeline failed or passed, a job waits, a deployment, a
    /// conflict, a review — and the crown hears it for a worker it started
    /// or watches, through the same table (`due`) as a merge. One line per
    /// ticket per pass, every edge in it: `pipeline passed !1, review
    /// approved !1`. What the crown does with it is its brief's to say.
    /// A pass that also flipped the branch to merged (the forge's merged
    /// clause, T-769) says the merge in this line, as `merged`, and moves
    /// both baselines to it as `hear_merges` does: the flags' wake on the
    /// tick then finds it told and folds into it. News read inside the
    /// ticket's recovery window (`late`, `forge::State::restored`, T-937)
    /// says `after a restart`, held for a step or not; the look-based
    /// `restored` of T-602 still adds the same words in `enter_wake`.
    /// `merged`: the words say a PR's merge (T-1071), which ends a watch.
    pub(super) fn hear_forge(&mut self, news: Vec<ForgeNews>) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let mut woke = false;
        for ForgeNews { worker, words, late, merged } in news {
            if worker == crown || !self.crown_hears(worker, crown) {
                continue;
            }
            let Some(column) = self.board.ticket(worker).map(|t| t.column.clone()) else {
                continue;
            };
            let agg = self.wt_agg.get(&worker);
            let now = Told {
                tip: agg.map(|a| a.tip.clone()).unwrap_or_default(),
                merge: agg.map(|a| worktree::merge_word(a.merged, a.needs_rebase, a.ahead)),
                ahead: agg.map_or(0, |a| a.ahead),
                legs: self.wt_repos.get(&worker).map(|l| leg_aheads(l)).unwrap_or_default(),
                column,
                forge: Some(words),
            };
            let landing = self.crown_landed.contains(&worker)
                && now.merge == Some("merged")
                && !self
                    .crown_heard
                    .get(&worker)
                    .and_then(|h| h.told.as_ref())
                    .is_some_and(|t| t.merge == Some("merged") && t.tip == now.tip);
            let cause = if landing { WakeCause::Merged } else { WakeCause::Forge };
            match due(Some(cause), None, &self.pending_on(worker, None)) {
                Due::Step(cause) => self.hold_for_step(worker, cause, now, late),
                Due::Wake(cause) => {
                    let heard = self.crown_heard.entry(worker).or_default();
                    if landing {
                        heard.judged = Some(now.clone());
                        heard.deferred = None;
                    }
                    let from = heard.told.replace(now.clone());
                    let owed = Owed { late, merge: landing || merged, ..Owed::default() };
                    self.owe_wake(crown, worker, cause, from, Some(now), owed);
                    woke = true;
                }
                Due::Hold(_) | Due::Silent => {}
            }
        }
        woke
    }

    /// What the wakes about to be sent tell the crown (T-918): each card's
    /// column as it stands (`CrownWake::sent_at`), which the next wake's
    /// delta runs from (T-785, a column said is not said again), the hands
    /// they are about, and the card's line. Taken with the sentence and
    /// applied by `told` once the crown has it: at the paste on Claude's
    /// roads, at the Enter on a Codex box (`Daemon::crown_wake_entering`).
    pub(super) fn telling(&self) -> Telling {
        let columns = self
            .crown_wakes
            .iter()
            .filter(|w| w.to.is_some())
            .filter_map(|w| Some((w.worker, self.board.ticket(w.worker)?.column.clone())))
            .collect();
        let raised = self
            .crown_wakes
            .iter()
            .filter(|w| w.cause == WakeCause::Raised)
            .map(|w| w.worker)
            .collect();
        Telling { columns, raised, news: self.crown_wake_news() }
    }

    /// The crown has the sentence `telling` was taken with: its columns
    /// are the baseline, the hands it was about come down (T-861) — the
    /// crown holds the worker's question from here: it answers the worker,
    /// which lowers the hand anyway, or raises its own, and a `!` that
    /// stood on past that was two cards asking a person one thing. Only a
    /// wake whose cause IS the hand: a question or a plan folded over it
    /// (`WakeCause`'s order) comes down with its dialog (T-611), and a hand
    /// folded over a delivery is still the hand.
    ///
    /// The reason stays with the crown that was told (T-922): the sentence
    /// sent it to `get_ticket` for it, and the hand it would have read
    /// there is the one coming down here. A hand somebody lowered before
    /// the sentence went in keeps nothing: that person dealt with it.
    pub(super) fn told(&mut self, crown: ulid::Ulid, telling: Telling, wakes: &[CrownWake]) {
        for wake in wakes.iter().filter(|wake| wake.reports_merge()) {
            if let Some(to) = wake.to.as_ref().filter(|to| to.merge == Some("merged")) {
                self.consume_merge_news(wake.worker, &to.tip);
            }
        }
        let Telling { columns, raised, news } = telling;
        for (worker, column) in columns {
            if let Some(told) = self.crown_heard.get_mut(&worker).and_then(|h| h.told.as_mut()) {
                told.column = column;
            }
        }
        let now = now_ms();
        for worker in raised {
            let reason = self.board.ticket(worker).and_then(|t| t.raised.as_ref());
            if let Some(reason) = reason.map(|r| r.reason.clone()) {
                self.crown_hands.insert(worker, ToldHand { reason, told_ms: now });
            }
            self.lower_hand_on(worker);
        }
        self.feed.board("automation", "crown_wake_sent", Some(crown));
        self.crown_said(crown, &news);
    }

    fn end_merge_watch(&mut self, worker: ulid::Ulid) {
        if self.crown_watched.remove(&worker) {
            self.hold_unwatched_sends(worker);
            self.feed.board("automation", "watch_ended", Some(worker));
        }
    }

    /// Consume only news already present on the writer thread. Later
    /// events, including another answer at the same tip, remain news.
    /// Retry copies lose it too, without changing words already in flight.
    fn consume_crown_news(&mut self, worker: ulid::Ulid, covers: impl Fn(&CrownWake) -> bool) {
        let mut landed = false;
        let mut keep = |w: &CrownWake| !(w.worker == worker && covers(w));
        self.crown_wakes.retain(&mut keep);
        if let Some(h) = self.crown_heard.get_mut(&worker) {
            if h.stepped.as_ref().is_some_and(|w| !keep(w)) {
                landed |= h.stepped.as_ref().is_some_and(CrownWake::says_merge);
                h.stepped = None;
            }
            if let Some(f) = h.forging.as_mut().filter(|f| !keep(&f.wake)) {
                landed |= f.wake.says_merge();
                // The turn is known; the outstanding forge read is not.
                // Keep its slot so a later opening still has a wake to ride.
                let mut to = f.wake.to.clone();
                if let Some(to) = to.as_mut() {
                    to.forge = None;
                }
                f.wake = CrownWake::new(worker, WakeCause::Forge, to.clone(), to, Owed::default());
            }
        }
        if let Some(e) = self.crown_wake_entering.as_mut() {
            e.wakes.retain(&mut keep);
        }
        for e in self.crown_wakes_taken.iter_mut() {
            e.wakes.retain(&mut keep);
        }
        if landed {
            self.end_merge_watch(worker);
        }
    }

    fn consume_merge_news(&mut self, worker: ulid::Ulid, tip: &str) {
        self.consume_crown_news(worker, |w| w.only_merge_at(tip));
        let mut landed = false;
        if let Some(h) = self.crown_heard.get_mut(&worker) {
            // A stepped wake's `finished` predicts its own notice turn's
            // end. Unlike a queued wake, no later Finished folds here: due
            // holds only Delivered/Merged for a step. Consuming the merge
            // can remove this hold, but not a new completion queued later.
            if h.stepped.as_ref().is_some_and(|w| w.merge_fact_at(tip)) {
                landed = h.stepped.take().is_some_and(|w| w.says_merge());
            }
            // A delivery still held for the train has not become a merged
            // wake yet. Its work is covered by this merge too; a different
            // tip or a later finished turn is not.
            if h.deferred
                .as_ref()
                .is_some_and(|held| held.cause == WakeCause::Delivered && held.look.tip == tip)
            {
                h.deferred = None;
            }
        }
        if landed {
            self.end_merge_watch(worker);
        }
    }

    /// A merge receipt handles only its merge, never unrelated turn,
    /// failure, dialog or forge news folded alongside it.
    pub(super) fn crown_observed_merge(&mut self, worker: ulid::Ulid) {
        let Some(now) = self
            .sampled(worker)
            .filter(|look| look.merged)
            .and_then(|look| Some(look.told(self.board.ticket(worker)?.column.clone())))
        else {
            return;
        };
        self.consume_merge_news(worker, &now.tip);
        self.crown_landed.retain(|w| *w != worker);
        self.end_merge_watch(worker);
        self.crown_heard.entry(worker).or_default().told = Some(now);
        self.persist_crown();
    }

    /// A successful keyed ticket read observes this ticket's pending turn
    /// and branch news (T-1036). Board listings and narrow writes do not.
    pub(super) fn crown_observed(&mut self, worker: ulid::Ulid, merge: Option<&str>) {
        if self.board.crown.is_none() || self.board.crown == Some(worker) {
            return;
        }
        self.consume_crown_news(worker, |w| {
            w.turn_news()
                && (merge == Some("merged")
                    || w.to.as_ref().is_none_or(|to| to.merge != Some("merged")))
        });
        let now = self
            .sampled(worker)
            .filter(|look| look.merged && merge == Some("merged"))
            .and_then(|look| Some(look.told(self.board.ticket(worker)?.column.clone())));
        if now.as_ref().is_some_and(|t| t.merge == Some("merged"))
            && self.crown_landed.contains(&worker)
        {
            self.crown_landed.retain(|w| *w != worker);
            self.end_merge_watch(worker);
        }
        if now.is_some() || self.crown_heard.contains_key(&worker) {
            let h = self.crown_heard.entry(worker).or_default();
            h.deferred = None;
            if let Some(now) = now {
                h.told = Some(now);
            }
        }
        self.persist_crown();
    }

    /// The wakes a Codex box took and never entered (T-918): owed again,
    /// ahead of any owed since, which fold into them as a second event on
    /// the same worker does. Nothing was told, so nothing is undone.
    pub(super) fn owe_again(&mut self, wakes: Vec<CrownWake>) {
        restore_wakes(&mut self.crown_wakes, wakes);
    }

    /// The crown left, or another ticket took it: whatever it was owed goes
    /// with it, said once in the feed, and what it was told goes too — a
    /// new crown has heard nothing.
    pub(super) fn drop_crown_wakes(&mut self) {
        // A failed delivery's hold was the old seat's (T-936).
        self.crown_wake_failed_on = None;
        self.crown_heard.clear();
        self.crown_landed.clear();
        self.crown_recheck.clear();
        self.crown_watched.clear();
        self.crown_hands.clear();
        // The outgoing crown's expectations (T-973, round 1): a turn it
        // timed wakes no other crown, and is kept in no other's ledger.
        if !self.crown_expects.is_empty() {
            self.crown_expects.clear();
        }
        // A Codex box holding the wake for its Enter (T-918): the seat is
        // no crown's now, so nothing presses it. The words stay in the box
        // as they are, for whoever is at it.
        let entering = self.crown_wake_entering.take().map(|e| e.session);
        if let Some(id) =
            entering.filter(|id| self.owed.get(id).is_some_and(|o| o.ack.is_crown_wake()))
        {
            self.codex_input_due.remove(&id);
            self.control_cancel(id);
            self.drop_owed(id);
        }
        // Taken wakes (T-990) were told this crown; they are kept for no
        // other.
        if !self.crown_wakes_taken.is_empty() {
            self.crown_wakes_taken.clear();
        }
        if self.crown_wakes.is_empty() && entering.is_none() {
            return;
        }
        self.crown_wakes.clear();
        self.feed.board("automation", "crown_wake_dropped", self.board.crown);
    }

    /// The worker's agent was given something (T-922): its next turn began,
    /// so the hand the crown was told of is answered or overtaken, and its
    /// reason is no longer the worker's ask. Also the worker leaving the
    /// board (archive, snooze, delete). Writes nothing for a worker that
    /// kept none.
    pub(super) fn forget_told_hand(&mut self, worker: ulid::Ulid) {
        if self.crown_hands.contains_key(&worker) {
            self.crown_hands.remove(&worker);
        }
    }

    /// What the crown on `crown` reads of `worker`'s told hand (T-922):
    /// only the board's crown, and never on its own ticket.
    pub(super) fn told_hand_view(
        &self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
    ) -> Option<mesimon_core::command::AgentToldHandView> {
        if self.board.crown != Some(crown) || worker == crown {
            return None;
        }
        let hand = self.crown_hands.get(&worker)?;
        Some(mesimon_core::command::AgentToldHandView {
            reason: mesimon_core::text::scrub_text(&hand.reason),
            told_secs: Some(now_ms().saturating_sub(hand.told_ms) / 1000),
        })
    }

    /// The sentence the crown receives: the board's `crown_wake` template
    /// over every wake owed, in the order they happened, each with what
    /// changed in brackets. A worker's title and a column's name are user
    /// text headed for another process, so they cross `scrub_text`. A worker
    /// no longer on the board contributes nothing. A hand's reason never
    /// rides it (T-414): the crown reads it through `get_ticket`. A delivered
    /// branch that is not yet merged says who merges it (T-613, `merge:
    /// crown`), read as the sentence goes — the board's mode and train as
    /// they stand now, not as they stood when the wake was owed.
    pub(super) fn crown_wake_text(&self) -> String {
        let (events, keys) = self.crown_wake_events(true);
        self.board.prompts.render(
            mesimon_core::prompts::AgentPrompt::CrownWake,
            &[("events", &events.join("; ")), ("keys", &keys.join(", "))],
        )
    }

    /// The same events for the crown's card (T-783), without the template
    /// and without the titles: the board shows each worker's card already,
    /// and one line under a title has no room to spell another.
    pub(super) fn crown_wake_news(&self) -> String {
        self.crown_wake_events(false).0.join("; ")
    }

    /// Every wake owed, in the order they happened, each with what changed
    /// in brackets, and the keys they name. `titled` spells each worker's
    /// title after its key, as the crown's sentence does.
    fn crown_wake_events(&self, titled: bool) -> (Vec<String>, Vec<String>) {
        let mut events: Vec<String> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for w in &self.crown_wakes {
            let Some(t) = self.board.ticket(w.worker) else { continue };
            let w = &w.sent_at(&t.column);
            let mut event = if titled {
                let title = mesimon_core::text::scrub_text(&t.title);
                format!("{} \"{title}\" {}", t.short_key, w.clause())
            } else {
                format!("{} {}", t.short_key, w.clause())
            };
            let mut changed = w.changed();
            let unmerged_branch =
                w.to.as_ref().is_some_and(|to| to.merge.is_some_and(|m| m != "merged"));
            if w.cause == WakeCause::Delivered && unmerged_branch {
                let word = format!("merge: {}", self.merge_by(w.worker).by);
                let at =
                    changed.iter().position(|c| c.starts_with("merge_state")).map_or(0, |i| i + 1);
                changed.insert(at, word);
            }
            if !changed.is_empty() {
                event.push_str(&format!(" ({})", changed.join(", ")));
            }
            events.push(event);
            keys.push(t.short_key.clone());
        }
        (events, keys)
    }
}

/// Put returned wakes before those owed meanwhile, folding each worker's
/// newer news into its returned wake. Kept separate from delivery so the
/// complete return fold can be checked without a live Codex box.
fn restore_wakes(owed: &mut Vec<CrownWake>, wakes: Vec<CrownWake>) {
    let since = std::mem::replace(owed, wakes);
    for w in since {
        if let Some(first) = owed.iter_mut().find(|o| o.worker == w.worker) {
            first.absorb(
                w.cause,
                w.from,
                w.to,
                Owed {
                    finished: w.finished,
                    linger: w.linger,
                    late: w.late,
                    ask: w.ask,
                    overrun: w.overrun,
                    merge: w.merge,
                },
            );
            first.unstarted = w.unstarted.or(first.unstarted.take());
            first.failed = w.failed.or(first.failed.take());
            first.promotion = w.promotion.or(first.promotion.take());
        } else {
            owed.push(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_probe_from_the_previous_merge_target_cannot_owe_news() {
        let mut b = watched_bed("stale-target-probe", None);
        b.d.board.sessions.clear(); // No live seat to reprobe in this fixture.
        b.d.board.merge_into = MergeInto::Branch("target-b".into());
        b.d.crown_heard.get_mut(&b.worker).unwrap().looks = 1;
        b.d.on_turn_probed(TurnProbe {
            worker: b.worker,
            crown: b.crown,
            why: ProbeWhy::Turn { asked: None, fresh: true },
            gen: b.d.merge_target_gen.wrapping_sub(1),
            found: Found::Branch(BranchLook::default()),
        });
        assert!(b.d.crown_wakes.is_empty());
        let heard = &b.d.crown_heard[&b.worker];
        assert_eq!(heard.looks, 0);
        assert!(heard.told.is_none());
    }

    #[test]
    fn a_turn_probe_keeps_its_delivery_across_an_ordinary_flag_refresh() {
        let mut b = watched_bed("same-target-probe", None);
        b.d.board.sessions.clear();
        b.d.crown_heard.get_mut(&b.worker).unwrap().looks = 1;
        let gen = b.d.merge_target_gen;
        b.d.wt_gen = b.d.wt_gen.wrapping_add(1); // A merge refreshed flags meanwhile.
        b.d.on_turn_probed(TurnProbe {
            worker: b.worker,
            crown: b.crown,
            why: ProbeWhy::Turn { asked: None, fresh: true },
            gen,
            found: Found::Branch(BranchLook {
                tip: "captured-worker-commit".into(),
                ahead: 1,
                ..BranchLook::default()
            }),
        });
        let wake = owed_for(&b.d, b.worker).expect("the captured delivery is still news");
        assert_eq!(wake.cause, WakeCause::Delivered);
        assert_eq!(wake.to.as_ref().unwrap().tip, "captured-worker-commit");
        assert_eq!(b.d.crown_heard[&b.worker].looks, 0);
    }

    /// A Codex box gives back X's wake after newer news for X was owed.
    /// The return keeps both snapshots' facts and the queue's order.
    #[test]
    fn a_returned_wake_keeps_the_later_ask_and_overrun() {
        let worker = ulid::Ulid(1);
        let first_linger = Linger { tasks: 2, minutes: 30 };
        let mut first = CrownWake::new(
            worker,
            WakeCause::Lingering,
            Some(branch("old", "clean", 0, "TODO")),
            Some(branch("first", "ahead", 1, "REVIEW")),
            Owed { linger: Some(first_linger), ..Owed::default() },
        );
        first.unstarted = Some("old start failure".into());
        first.failed = Some(FailedTurn {
            session: uuid::Uuid::from_u128(1),
            turn: "old turn".into(),
            reason: mesimon_core::board::FailReason::Unknown,
        });
        let ask = AskSaid { ask: ulid::Ulid(2), said: Said::Taken };
        let overrun = Overrun { expected_secs: 30, running_secs: 60 };
        let latest = branch("latest", "ahead", 2, "REVIEW");
        let mut newer = CrownWake::new(
            worker,
            WakeCause::Answered,
            first.to.clone(),
            Some(latest.clone()),
            Owed {
                finished: true,
                linger: Some(Linger { tasks: 3, minutes: 45 }),
                late: true,
                ask: Some(ask),
                overrun: Some(overrun),
                merge: false,
            },
        );
        newer.unstarted = Some("new start failure".into());
        let failed = FailedTurn {
            session: uuid::Uuid::from_u128(2),
            turn: "new turn".into(),
            reason: mesimon_core::board::FailReason::Server,
        };
        newer.failed = Some(failed.clone());
        let other = CrownWake::new(ulid::Ulid(3), WakeCause::Raised, None, None, Owed::default());
        let mut owed = vec![other, newer];
        restore_wakes(&mut owed, vec![first]);

        assert_eq!(owed.iter().map(|w| w.worker).collect::<Vec<_>>(), vec![worker, ulid::Ulid(3)]);
        let w = &owed[0];
        assert_eq!(w.ask, Some(ask), "the later ask survives the box's return");
        assert_eq!(w.overrun, Some(overrun), "the later overrun survives the box's return");
        assert_eq!(w.linger, Some(first_linger), "the first lingering snapshot stays");
        assert!(w.finished && w.late, "both sticky flags survive");
        assert_eq!(w.cause, WakeCause::Answered);
        assert_eq!(w.from.as_ref().unwrap().tip, "old");
        assert_eq!(w.to, Some(latest));
        assert_eq!(w.unstarted.as_deref(), Some("new start failure"));
        assert_eq!(w.failed, Some(failed));
        assert!(w.changed().contains(&format!("ask {}: taken", ask.ask)));

        // Empty newer fields cannot erase facts from the returned wake.
        // Nonempty ask/overrun fields replace the previous snapshots.
        let mut newer = CrownWake::new(worker, WakeCause::Finished, None, None, Owed::default());
        let returned = std::mem::replace(&mut owed, vec![newer.clone()]);
        restore_wakes(&mut owed, returned);
        assert_eq!(owed[0].ask, Some(ask));
        assert_eq!(owed[0].overrun, Some(overrun));
        assert_eq!(owed[0].unstarted.as_deref(), Some("new start failure"));
        assert_eq!(owed[0].failed.as_ref().unwrap().turn, "new turn");
        let next_ask = AskSaid { ask: ulid::Ulid(4), said: Said::Handed };
        let next_overrun = Overrun { expected_secs: 40, running_secs: 80 };
        newer.ask = Some(next_ask);
        newer.overrun = Some(next_overrun);
        let returned = std::mem::replace(&mut owed, vec![newer]);
        restore_wakes(&mut owed, returned);
        assert_eq!(owed[0].ask, Some(next_ask));
        assert_eq!(owed[0].overrun, Some(next_overrun));
    }

    /// T-938: the turn-end wake says what the end proves and names the ask
    /// it was owed for, as it stood then. A fold keeps the latest owed; a
    /// wake for an answer or a plan names none.
    #[test]
    fn a_turn_end_wake_names_its_own_ask_and_says_no_more_than_the_end() {
        let old = ulid::Ulid::from_parts(1, 1);
        let mut w = CrownWake::new(
            ulid::Ulid::from_parts(1, 7),
            WakeCause::Answered,
            None,
            None,
            Owed { ask: Some(AskSaid { ask: old, said: Said::Handed }), ..Owed::default() },
        );
        assert_eq!(w.clause(), "ended its turn after your ask went in");
        assert_eq!(w.changed(), vec![format!("ask {old}: handed")]);
        let json = serde_json::to_string(&w).unwrap();
        let back: CrownWake = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ask, w.ask, "the snapshot rides crown.json: {json}");
        // No receipt named the words: the line claims the turn's end alone.
        for said in [Said::Sent, Said::Unknown] {
            w.ask = Some(AskSaid { ask: old, said });
            assert_eq!(
                w.clause(),
                format!("ended a turn after ask {old} was sent; receipt unconfirmed")
            );
            assert!(w.changed().is_empty(), "no second word for it: {:?}", w.changed());
        }
        // An answer's turn carries no ask; nor does a v1 wake.
        w.ask = None;
        assert!(w.changed().is_empty());
        assert_eq!(AskSaid::of(Some(TurnAsk::Crown(old))), None);
        let newer = ulid::Ulid::from_parts(1, 2);
        let said = AskSaid::of(Some(TurnAsk::Asked { crown: old, ask: newer, said: Said::Taken }));
        assert_eq!(said, Some(AskSaid { ask: newer, said: Said::Taken }));
        // Joined with a merge step, the crown's ask is kept.
        let joined = Unjudged { asked: Some(TurnAsk::Merge), fresh: true }.join(Unjudged {
            asked: Some(TurnAsk::Asked { crown: old, ask: newer, said: Said::Handed }),
            fresh: false,
        });
        assert_eq!(joined.asked.and_then(TurnAsk::crown), Some(old));
    }

    fn branch(tip: &str, merge: &'static str, ahead: u32, column: &str) -> Told {
        Told {
            tip: tip.into(),
            merge: Some(merge),
            ahead,
            legs: Vec::new(),
            column: column.into(),
            forge: None,
        }
    }

    fn checkout(head: &str, column: &str) -> Told {
        Told {
            tip: head.into(),
            merge: None,
            ahead: 0,
            legs: Vec::new(),
            column: column.into(),
            forge: None,
        }
    }

    const DELIVERED: Option<WakeCause> = Some(WakeCause::Delivered);
    const FINISHED: Option<WakeCause> = Some(WakeCause::Finished);
    const MERGED: Option<WakeCause> = Some(WakeCause::Merged);
    const IDLE: Pending = Pending {
        train: false,
        merge_step: false,
        words: false,
        turn: false,
        hand: false,
        dialog: false,
    };
    const TRAIN: Pending = Pending { train: true, ..IDLE };
    const STEP: Pending = Pending { merge_step: true, ..IDLE };

    /// T-756: a worker mid-turn across a restart reads `Unknown`, then the
    /// tail's guess, idle at Low, between two of its tool calls. With its
    /// branch ahead that idle would be a delivery, so the restart's look
    /// waits; the close T-663 dates in the gap, or the hook's `Stop`, is
    /// the turn's end, and the same branch is the delivery.
    #[test]
    fn the_restart_s_look_waits_for_an_end_that_is_one() {
        use mesimon_core::attention::{Machine, Signal, TailHint};
        use mesimon_core::board::{SessionKind, UnknownReason};
        let restarted = || {
            Machine::restore(
                SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
                Confidence::High,
                0,
            )
        };
        let record = |m: &Machine| {
            let mut r = SessionRecord::new(
                uuid::Uuid::from_u128(1),
                SessionKind::Claude,
                ulid::Ulid::from_parts(1, 1),
                vec![],
                "/tmp".into(),
                m.state().clone(),
            );
            r.confidence = m.confidence();
            r
        };
        let stop = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: false,
            monitoring_tasks: false,
            teammates: 0,
        };
        let ahead = branch("bbb", "needs_rebase", 2, "IN PROGRESS");
        let before = branch("aaa", "ahead", 1, "IN PROGRESS");
        assert_eq!(verdict(Some(&before), &ahead, false, false, false), DELIVERED, "if looked");

        for hint in [TailHint::TurnComplete, TailHint::StaleQuiet] {
            let mut m = restarted();
            let c = m.apply(&Signal::TranscriptHint { kind: hint }, 1_000).unwrap();
            assert!(matches!(c.to, SessionState::Idle { .. }), "{hint:?}");
            assert!(guessed(&record(&m)), "{hint:?}: the tail's guess holds the look");
            m.apply(&Signal::ToolCompleted { nested: false }, 2_000).unwrap();
            // Leaving `Running` settles before it commits.
            if m.apply(&stop, 3_000).is_none() {
                m.tick(10_000).unwrap();
            }
            assert_eq!(m.state(), &SessionState::Idle { stop_reason: StopReason::EndTurn });
            assert!(!guessed(&record(&m)), "{hint:?}: the hook's Stop is the end");
        }

        let mut m = restarted();
        m.apply(&Signal::TranscriptTurnEnded, 1_000).unwrap();
        assert_eq!(m.confidence(), Confidence::Medium);
        assert!(!guessed(&record(&m)), "a close dated in the gap is the end");

        let mut m = restarted();
        m.apply(&stop, 1_000).unwrap();
        assert!(!guessed(&record(&m)), "a hook re-asserting after the restart");

        // T-776: before the tail or a hook speaks the record is `Unknown`
        // at High, which `guessed` does not catch; it is no end either, and
        // holds a delivery held for the train until one commits.
        let m = restarted();
        assert!(!guessed(&record(&m)));
        assert!(rederiving(&record(&m)), "Unknown holds the train's hold");
        let mut m = restarted();
        m.apply(&Signal::TranscriptTurnEnded, 1_000).unwrap();
        assert!(!rederiving(&record(&m)), "a close dated in the gap");
        let mut m = restarted();
        m.apply(&stop, 1_000).unwrap();
        assert!(!rederiving(&record(&m)), "the hook's Stop");
    }

    /// Once per delivery (T-469): a second idle at the same tip delivers
    /// nothing, and neither does an idle with nothing to merge. Each is a
    /// finished turn (T-591) when a turn ran since the last end looked at,
    /// and silent when none did. A delivery is never gated on it.
    #[test]
    fn a_second_idle_with_nothing_new_delivers_nothing() {
        let first = branch("aaa", "ahead", 1, "REVIEW");
        assert_eq!(verdict(None, &first, false, false, false), DELIVERED);
        assert_eq!(verdict(None, &first, false, false, true), DELIVERED);
        assert_eq!(verdict(Some(&first), &first, false, false, false), None, "same tip");
        assert_eq!(verdict(Some(&first), &first, false, false, true), FINISHED, "same tip, fresh");
        let fresh = branch("base", "clean", 0, "IN PROGRESS");
        assert_eq!(verdict(None, &fresh, false, false, false), None, "nothing to merge");
        assert_eq!(verdict(None, &fresh, false, false, true), FINISHED, "nothing to merge, fresh");
        // A turn's end at a merged branch is not a delivery: the merge is
        // heard where the flags read it (`merge_verdict`).
        let merged = branch("bbb", "merged", 0, "DONE");
        assert_eq!(verdict(Some(&first), &merged, false, false, false), None, "not a delivery");
        assert_eq!(verdict(Some(&first), &merged, false, false, true), FINISHED);
        // New work on top is a new delivery; so is a branch the base moved
        // past, which still has something to merge.
        let more = branch("ccc", "ahead", 2, "REVIEW");
        assert_eq!(verdict(Some(&first), &more, false, false, false), DELIVERED);
        let behind = branch("ddd", "needs_rebase", 1, "REVIEW");
        assert_eq!(verdict(None, &behind, false, false, false), DELIVERED);
    }

    /// T-591's own case: a worker the crown started is done with a turn that
    /// left nothing new, and the crown hears it finished — once, since the
    /// same idle re-entered with no turn between is not fresh again. A
    /// merge step stays silent however fresh it reads; the crown hears at
    /// the merge. Nothing pending, it wakes.
    /// A worker idle with background tasks (T-599): the weakest news, said
    /// in numbers, never the tasks' words.
    #[test]
    fn a_lingering_line_says_how_many_and_how_long() {
        let l = |tasks, minutes| Linger { tasks, minutes };
        assert_eq!(lingering_clause(l(3, 30)), "has been idle with 3 background tasks for 30 min");
        assert_eq!(lingering_clause(l(1, 45)), "has been idle with 1 background task for 45 min");
        assert_eq!(lingering_clause(l(0, 600)), "has been idle with background tasks for 10 h");
        assert!(WakeCause::Lingering < WakeCause::Finished, "below a finished turn");
        let mut w = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Lingering,
            None,
            None,
            Owed { linger: Some(l(3, 30)), ..Owed::default() },
        );
        assert_eq!(w.clause(), "has been idle with 3 background tasks for 30 min");
        assert!(w.changed().is_empty(), "no delta: nothing about the work moved");
        // A stronger event on the same worker says itself.
        w.fold(WakeCause::Raised, None, None);
        assert_eq!(w.clause(), "raised its hand");
        // A lingering wake is never held: nothing pending on the ticket
        // weighs it.
        let pending = Pending { train: true, words: true, turn: true, ..IDLE };
        assert_eq!(
            due(Some(WakeCause::Lingering), None, &pending),
            Due::Wake(WakeCause::Lingering)
        );
    }

    /// A background command's park through the real machine (T-944): the
    /// Stop parks it at `Background`, the park clock demotes it to
    /// `Monitoring` with no frame, and the stretch's stamp is the same
    /// before and after; the notification's foreground turn ends it, and
    /// the next park is a new stretch.
    #[test]
    fn a_park_on_tasks_keeps_its_stretch_across_the_demote() {
        use mesimon_core::attention::{Machine, Signal, PARK_STALE_MS, SETTLE_MS};
        let parked_on_command = Signal::Stop {
            stop_hook_active: false,
            has_agent_id: false,
            blocking_tasks: true,
            monitoring_tasks: true,
            teammates: 0,
        };
        let mut since = None;
        let mut m = Machine::new(SessionState::Running, 0);
        let mut step = |c: mesimon_core::attention::Change, at: u64| {
            since = tasks_stretch(&c.from, &c.to, at, since);
            (c.to, since)
        };
        assert!(m.apply(&parked_on_command, 1_000).is_none(), "a leave settles");
        let parked = 1_000 + SETTLE_MS;
        let (to, at_park) = step(m.tick(parked).expect("parks"), parked);
        assert_eq!(to, SessionState::Idle { stop_reason: StopReason::Background });
        assert_eq!(at_park, Some(parked));
        let stale = parked + PARK_STALE_MS;
        let (to, at_demote) = step(m.tick(stale).expect("the park goes stale"), stale);
        assert_eq!(to, SessionState::Idle { stop_reason: StopReason::Monitoring });
        assert_eq!(at_demote, at_park, "the demote keeps the stretch's stamp");
        let turn = stale + 60_000;
        let (to, after_turn) =
            step(m.apply(&Signal::UserPromptSubmit, turn).expect("a turn"), turn);
        assert_eq!(to, SessionState::Running);
        assert_eq!(after_turn, None, "a foreground turn ends the stretch");
        assert!(m.apply(&parked_on_command, turn + 1_000).is_none());
        let again = turn + 1_000 + SETTLE_MS;
        let (_, next) = step(m.tick(again).expect("parks again"), again);
        assert_eq!(next, Some(again), "a new stretch");
        // Between the two parks either way, and none outside them.
        let bg = SessionState::Idle { stop_reason: StopReason::Background };
        let mon = SessionState::Idle { stop_reason: StopReason::Monitoring };
        let end = SessionState::Idle { stop_reason: StopReason::EndTurn };
        assert_eq!(tasks_stretch(&mon, &bg, 9, Some(5)), Some(5));
        assert_eq!(tasks_stretch(&bg, &mon, 9, None), Some(9), "unseen: dated here");
        assert_eq!(tasks_stretch(&end, &mon, 9, Some(5)), Some(9), "entered afresh");
        assert_eq!(tasks_stretch(&mon, &end, 9, Some(5)), None);
        assert_eq!(tasks_stretch(&mon, &SessionState::Sleeping, 9, Some(5)), None);
    }

    /// Who is due the lingering wake (T-599, T-944): a crown-started Claude
    /// seat idle on tasks under either park, aged from its stretch, not
    /// from the demote's restamp. Nobody else, and nobody early.
    #[test]
    fn a_worker_lingers_on_either_park_from_its_stretch() {
        use mesimon_core::board::{SessionKind, SessionRecord};
        let crown = ulid::Ulid::from_parts(1, 1);
        let worker = ulid::Ulid::from_parts(2, 2);
        const MIN: u64 = 60_000;
        let after = 30 * MIN;
        let now = 100 * MIN;
        let rec = |kind, ticket, state: SessionState| {
            let mut r = SessionRecord::new(
                uuid::Uuid::from_u128(7),
                kind,
                ticket,
                vec![],
                "/tmp".into(),
                state,
            );
            r.started_by = Some(crown);
            r.state_changed_at = Some(now - after);
            r.background_tasks.record(
                "cmd1",
                Some(mesimon_core::background::BACKGROUND_COMMAND),
                None,
                mesimon_core::background::Transition::Started,
                None,
            );
            r
        };
        let bg = SessionState::Idle { stop_reason: StopReason::Background };
        let mon = SessionState::Idle { stop_reason: StopReason::Monitoring };
        let due = |r: &SessionRecord, since| linger_due(r, crown, since, now, after);
        for park in [bg.clone(), mon.clone()] {
            let r = rec(SessionKind::Claude, worker, park.clone());
            assert_eq!(due(&r, None), Some(Linger { tasks: 1, minutes: 30 }), "{park:?}");
            let mut early = r.clone();
            early.state_changed_at = Some(now - after + 1);
            assert_eq!(due(&early, None), None, "{park:?}: inside the grace");
            let mut unasked = r.clone();
            unasked.started_by = None;
            assert_eq!(due(&unasked, None), None, "{park:?}: not the crown's start");
            let own = rec(SessionKind::Claude, crown, park.clone());
            assert_eq!(due(&own, None), None, "{park:?}: the crown's own seat");
            let codex = rec(SessionKind::Codex, worker, park.clone());
            assert_eq!(due(&codex, None), None, "{park:?}: a Claude seat alone");
        }
        for not_parked in [
            SessionState::Idle { stop_reason: StopReason::EndTurn },
            SessionState::Running,
            SessionState::Sleeping,
        ] {
            let r = rec(SessionKind::Claude, worker, not_parked.clone());
            assert_eq!(due(&r, Some(0)), None, "{not_parked:?}");
        }
        // Demoted three minutes ago, parked thirty-three: due, and said in
        // the stretch's minutes. Dated from the demote it would be early.
        let mut demoted = rec(SessionKind::Claude, worker, mon);
        demoted.state_changed_at = Some(now - 3 * MIN);
        assert_eq!(due(&demoted, None), None, "the restamp alone is early");
        assert_eq!(due(&demoted, Some(now - 33 * MIN)), Some(Linger { tasks: 1, minutes: 33 }));
        // The registry lost its tasks (a restart): no number is invented.
        let mut lost = demoted.clone();
        lost.background_tasks.clear();
        let l = due(&lost, Some(now - 33 * MIN)).expect("due");
        assert_eq!(lingering_clause(l), "has been idle with background tasks for 33 min");
    }

    #[test]
    fn a_finished_turn_with_nothing_new_wakes_once() {
        let idle = branch("base", "clean", 0, "REVIEW");
        let cause = verdict(None, &idle, false, false, true);
        assert_eq!(due(cause, None, &IDLE), Due::Wake(WakeCause::Finished));
        // The same idle again: `turns_open` was taken by the first end.
        assert_eq!(verdict(Some(&idle), &idle, false, false, false), None);
        assert_eq!(due(None, None, &TRAIN), Due::Silent);
        // The train's rebase ask or the merged notice.
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        assert_eq!(verdict(Some(&rebased), &rebased, false, true, true), None, "merge step");
        // A checkout is the same: nothing new is a HEAD already judged.
        let head = checkout("1111111aaaa", "REVIEW");
        assert_eq!(verdict(Some(&head), &head, false, false, true), FINISHED);
        assert_eq!(verdict(Some(&head), &head, false, false, false), None);
        // An answer outranks it: the crown asked.
        assert_eq!(verdict(Some(&idle), &idle, true, false, true), Some(WakeCause::Answered));
    }

    /// T-554, T-591: a delivery the armed merge train will land is held,
    /// with no wake, and one already held is judged again by the next turn.
    /// Still the train's (the rebase it asked for went through): held. Not
    /// the train's any more (that rebase left the branch behind the same
    /// tip, or the train is off): the delivery comes due. A finished turn
    /// on a branch the train will take is held the same way and comes due
    /// as a finish; over a held delivery it is the delivery, which the
    /// crown has not heard of. An answer is never held, and a silent turn
    /// with nothing held is silent whatever the train would do.
    #[test]
    fn a_delivery_the_train_will_take_is_held() {
        let behind = branch("aaa", "needs_rebase", 1, "REVIEW");
        let cause = verdict(None, &behind, false, false, true);
        assert_eq!(due(cause, None, &TRAIN), Due::Hold(WakeCause::Delivered));
        assert_eq!(due(cause, None, &IDLE), Due::Wake(WakeCause::Delivered));
        // The train's rebase ask: a merge step, silent by `verdict`.
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        let cause = verdict(Some(&behind), &rebased, false, true, true);
        assert_eq!(cause, None);
        assert_eq!(
            due(cause, DELIVERED, &TRAIN),
            Due::Hold(WakeCause::Delivered),
            "the train merges it next"
        );
        assert_eq!(due(cause, DELIVERED, &IDLE), Due::Wake(WakeCause::Delivered), "it gave up");
        assert_eq!(due(None, None, &TRAIN), Due::Silent);
        assert_eq!(
            due(Some(WakeCause::Answered), DELIVERED, &TRAIN),
            Due::Wake(WakeCause::Answered)
        );
        // Delivered and heard, then a turn that left the same tip: held for
        // the train as a finish, and a finish if the train gives up.
        let cause = verdict(Some(&rebased), &rebased, false, false, true);
        assert_eq!(cause, FINISHED);
        assert_eq!(due(cause, None, &TRAIN), Due::Hold(WakeCause::Finished));
        assert_eq!(due(None, FINISHED, &IDLE), Due::Wake(WakeCause::Finished));
        // A finish over a held delivery is that delivery, held or due.
        assert_eq!(due(cause, DELIVERED, &TRAIN), Due::Hold(WakeCause::Delivered));
        assert_eq!(due(cause, DELIVERED, &IDLE), Due::Wake(WakeCause::Delivered));
        // A delivery over a held finish is the delivery.
        assert_eq!(due(DELIVERED, FINISHED, &IDLE), Due::Wake(WakeCause::Delivered));
        // What the train is judged on is what the line would say.
        let look = BranchLook {
            tip: "aaa".into(),
            base_tip: "m1".into(),
            ahead: 1,
            needs_rebase: true,
            ..BranchLook::default()
        };
        assert_eq!(look.told("REVIEW".into()), behind);
    }

    /// The crown's own ask always comes back, whatever the turn left; the
    /// merge flow's words never wake it, even at a new tip.
    #[test]
    fn an_answer_always_wakes_and_a_merge_step_never_does() {
        let before = branch("aaa", "needs_rebase", 1, "REVIEW");
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        for fresh in [false, true] {
            let answered = Some(WakeCause::Answered);
            assert_eq!(verdict(Some(&before), &rebased, true, false, fresh), answered);
            assert_eq!(verdict(Some(&before), &before, true, false, fresh), answered);
            assert_eq!(verdict(Some(&before), &rebased, false, true, fresh), None);
            assert_eq!(verdict(Some(&before), &rebased, true, true, fresh), answered);
        }
    }

    /// A shared checkout delivers by HEAD: a new one wakes, the same one
    /// does not, and with no baseline (a restart) any HEAD does.
    #[test]
    fn a_checkout_delivers_on_a_new_head() {
        let start = checkout("1111111aaaa", "IN PROGRESS");
        let after = checkout("2222222bbbb", "REVIEW");
        assert_eq!(verdict(Some(&start), &start, false, false, false), None);
        assert_eq!(verdict(Some(&start), &after, false, false, false), DELIVERED);
        assert_eq!(verdict(None, &after, false, false, false), DELIVERED);
    }

    /// A finished turn's line (T-591) says first that it left nothing new
    /// to merge, then what changed since the crown last heard — and never a
    /// checkout's HEAD, which the turn did not move. Where git could not
    /// say, the line says the finish alone.
    #[test]
    fn a_finished_line_says_nothing_new_to_merge() {
        let wake = |from: Option<Told>, to: Option<Told>| {
            CrownWake::new(ulid::Ulid::nil(), WakeCause::Finished, from, to, Owed::default())
        };
        let head = checkout("1234567890abcdef", "REVIEW");
        assert_eq!(
            wake(None, Some(head.clone())).changed(),
            ["nothing new to merge", "column REVIEW"]
        );
        assert_eq!(
            wake(Some(head.clone()), Some(head.clone())).changed(),
            ["nothing new to merge"]
        );
        let idle = branch("base", "clean", 0, "REVIEW");
        assert_eq!(
            wake(None, Some(idle)).changed(),
            ["nothing new to merge", "merge_state clean", "column REVIEW"]
        );
        let heard = branch("aaa", "ahead", 1, "IN PROGRESS");
        let moved = branch("aaa", "needs_rebase", 1, "REVIEW");
        assert_eq!(
            wake(Some(heard), Some(moved)).changed(),
            ["nothing new to merge", "merge_state ahead → needs_rebase", "column REVIEW"]
        );
        assert!(wake(None, None).changed().is_empty());
        assert_eq!(WakeCause::Finished.clause(), "finished its turn");
        assert_eq!(WakeCause::Finished.word(), "finished");
        // Folded into a delivery, the line is the delivery's, and names the
        // HEAD it delivered.
        let mut w = wake(None, Some(checkout("1111111aaaa", "REVIEW")));
        w.fold(WakeCause::Delivered, None, Some(checkout("2222222bbbb", "REVIEW")));
        assert_eq!(w.changed(), ["commit 2222222", "column REVIEW"]);
    }

    /// A branch of two legs says each leg that moved, by its word, never
    /// the sum (T-772, lab finding 19); a baseline that knew no legs (an
    /// older `crown.json`, a leg cut since) says the sum with the count;
    /// one leg says the count as it always did. The legs cross `crown.json`.
    #[test]
    fn a_multi_leg_delta_names_each_leg_that_moved() {
        let legs = |tip: &str, root: u32, api: u32| Told {
            legs: vec![("root".into(), root), ("api".into(), api)],
            ..branch(tip, "ahead", root + api, "REVIEW")
        };
        let (before, after) = (legs("a+b", 1, 1), legs("a+c", 1, 2));
        assert_eq!(delta(Some(&before), &after), ["ahead api 1 → 2"]);
        assert_eq!(
            delta(Some(&before), &legs("c+d", 3, 4)),
            ["ahead root 1 → 3", "ahead api 1 → 4"]
        );
        // The same total, moved between legs, is still news.
        assert_eq!(
            delta(Some(&before), &legs("c+d", 0, 2)),
            ["ahead root 1 → 0", "ahead api 1 → 2"]
        );
        // The first word names the legs ahead (T-782, lab finding 2).
        assert_eq!(
            delta(None, &legs("a+b", 1, 1)),
            ["merge_state ahead root 1, api 1", "column REVIEW"]
        );
        assert_eq!(delta(None, &legs("a+b", 0, 2)), ["merge_state ahead api 2", "column REVIEW"]);
        let unlegged = branch("a+b", "ahead", 2, "REVIEW");
        assert_eq!(delta(Some(&unlegged), &legs("a+c", 1, 5)), ["ahead 2 → 6 over 2 repos"]);
        assert!(delta(Some(&unlegged), &legs("a+c", 1, 1)).is_empty());
        let one = branch("a", "ahead", 1, "REVIEW");
        assert_eq!(delta(Some(&one), &branch("b", "ahead", 2, "REVIEW")), ["ahead 1 → 2"]);
        let back: Told = serde_json::from_str(&serde_json::to_string(&after).unwrap()).unwrap();
        assert_eq!(back, after);
        assert!(!serde_json::to_string(&one).unwrap().contains("legs"), "one leg writes none");
    }

    #[test]
    fn the_delta_says_what_changed_since_the_crown_last_heard() {
        let before = branch("aaa", "needs_rebase", 1, "REVIEW");
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        assert_eq!(delta(Some(&before), &rebased), vec!["merge_state needs_rebase → ahead"]);
        let more = branch("ccc", "ahead", 3, "REVIEW");
        assert_eq!(delta(Some(&rebased), &more), vec!["ahead 1 → 3"]);
        // Heard nothing yet: the state itself, and the column.
        assert_eq!(delta(None, &before), vec!["merge_state needs_rebase", "column REVIEW"]);
        let moved = branch("aaa", "needs_rebase", 1, "DONE");
        assert_eq!(delta(Some(&before), &moved), vec!["column DONE"]);
        assert!(delta(Some(&before), &before).is_empty());
        // A checkout names its new HEAD, short.
        let head = checkout("1234567890abcdef", "REVIEW");
        assert_eq!(delta(None, &head), vec!["commit 1234567", "column REVIEW"]);
        assert!(delta(Some(&head), &head).is_empty());
        // A column is the user's words, headed for another process.
        let odd = branch("aaa", "needs_rebase", 1, "RE\u{1b}[2JVIEW");
        let line = delta(Some(&before), &odd).join(", ");
        assert!(!line.contains('\u{1b}'), "{line:?}");
    }

    /// T-569, T-582: a question or a plan wakes the crown under the switch
    /// and not without, only on the edge into it, from a claude and, since
    /// T-846, a codex; never from a shell.
    #[test]
    fn a_question_or_a_plan_wakes_the_crown_only_under_the_switch() {
        let question = SessionState::RequiresAction { reason: Reason::Question };
        let change = |from: SessionState, to: SessionState| Change {
            from,
            to,
            attention_added: false,
            confidence: Confidence::High,
        };
        let asked = change(SessionState::Running, question.clone());
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Claude, &asked),
            Some(WakeCause::Asked)
        );
        assert_eq!(
            asks_the_crown(CrownMode::Supervised, SessionKind::Claude, &asked),
            None,
            "off: the person's"
        );
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Codex, &asked),
            Some(WakeCause::Asked),
            "a codex's question is carried by its runtime and walked by keys (T-846)"
        );
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Bash, &asked),
            None,
            "a shell has no dialog"
        );
        assert_eq!(
            asks_the_crown(
                CrownMode::Autonomous,
                SessionKind::Claude,
                &change(question.clone(), question)
            ),
            None
        );
        let plan = SessionState::RequiresAction { reason: Reason::Plan };
        let planned = change(SessionState::Running, plan.clone());
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Claude, &planned),
            Some(WakeCause::Planned)
        );
        assert_eq!(
            asks_the_crown(CrownMode::Supervised, SessionKind::Claude, &planned),
            None,
            "off: the person's"
        );
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Codex, &planned),
            Some(WakeCause::Planned),
            "a codex plan is the T-420 press, for the crown too (T-846)"
        );
        assert_eq!(
            asks_the_crown(CrownMode::Autonomous, SessionKind::Claude, &change(plan.clone(), plan)),
            None
        );
        for reason in
            [Reason::Secret, Reason::Elicitation, Reason::Permission, Reason::Auth, Reason::Trust]
        {
            let stop = change(SessionState::Running, SessionState::RequiresAction { reason });
            assert_eq!(
                asks_the_crown(CrownMode::Autonomous, SessionKind::Claude, &stop),
                None,
                "{reason:?} is a person's"
            );
        }
        let answered = change(
            SessionState::RequiresAction { reason: Reason::Question },
            SessionState::Running,
        );
        assert_eq!(asks_the_crown(CrownMode::Autonomous, SessionKind::Claude, &answered), None);
        // The wake carries the cause, never the plan's words.
        assert_eq!(WakeCause::Planned.clause(), "stops on a plan");
    }

    /// T-802: a usage limit wakes the crown whatever its mode, on the edge
    /// into it, from any agent; a person's prompt leaving it wakes nothing,
    /// and nothing is owed on the ticket for it to wait out.
    #[test]
    fn a_usage_limit_wakes_the_crown_on_its_edge() {
        let quota = SessionState::RequiresAction { reason: Reason::QuotaResume };
        let change = |from: SessionState, to: SessionState| Change {
            from,
            to,
            attention_added: false,
            confidence: Confidence::High,
        };
        let limited = change(SessionState::Running, quota.clone());
        for kind in [SessionKind::Claude, SessionKind::Codex] {
            assert_eq!(limits_the_crown(kind, &limited), Some(WakeCause::Limited), "{kind:?}");
        }
        assert_eq!(limits_the_crown(SessionKind::Bash, &limited), None, "a shell has no quota");
        let again = change(quota.clone(), quota.clone());
        assert_eq!(limits_the_crown(SessionKind::Claude, &again), None);
        let left = change(quota.clone(), SessionState::Running);
        assert_eq!(limits_the_crown(SessionKind::Claude, &left), None);
        let busy = Pending { turn: true, words: true, ..Pending::default() };
        assert_eq!(due(Some(WakeCause::Limited), None, &busy), Due::Wake(WakeCause::Limited));
        assert_eq!(WakeCause::Limited.clause(), "hit a usage limit");
    }

    /// T-936: a turn ending on an error wakes the crown on the edge into
    /// it, from any agent, at a hook's confidence; a repeated hook changes
    /// nothing, a failure read again refines its reason and wakes nobody,
    /// and the restart's re-derivation at Low is no failure the board saw.
    /// The line says the reason the board saw, first; a failure says
    /// nothing delivered or finished, and nothing pending holds it.
    #[test]
    fn a_failed_turn_wakes_the_crown_on_its_edge() {
        use mesimon_core::board::FailReason;
        let failed = |reason| SessionState::Failed { reason };
        let change = |from: SessionState, to: SessionState, confidence| Change {
            from,
            to,
            attention_added: false,
            confidence,
        };
        let server = failed(FailReason::Server);
        let fell = change(SessionState::Running, server.clone(), Confidence::High);
        assert_eq!(
            failure_edge(SessionKind::Claude, &fell),
            Some(Failure { reason: FailReason::Server, new: true })
        );
        assert_eq!(failure_edge(SessionKind::Bash, &fell), None, "a shell's turn is no agent's");
        assert_eq!(failure_edge(SessionKind::Codex, &fell), None, "a Codex turn is its own");
        let proven = change(SessionState::Running, server.clone(), Confidence::Medium);
        assert!(failure_edge(SessionKind::Claude, &proven).is_some_and(|f| f.new));
        let guessed = change(
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
            server.clone(),
            Confidence::Low,
        );
        assert_eq!(failure_edge(SessionKind::Claude, &guessed), None, "a restart's guess");
        let again = change(server.clone(), server.clone(), Confidence::High);
        assert_eq!(failure_edge(SessionKind::Claude, &again), None, "a repeated hook");
        let refined = change(failed(FailReason::Unknown), server.clone(), Confidence::Medium);
        assert_eq!(
            failure_edge(SessionKind::Claude, &refined),
            Some(Failure { reason: FailReason::Server, new: false })
        );
        let left = change(server.clone(), SessionState::Running, Confidence::High);
        assert_eq!(failure_edge(SessionKind::Claude, &left), None);

        let busy = Pending { turn: true, words: true, ..Pending::default() };
        assert_eq!(due(Some(WakeCause::Failed), None, &busy), Due::Wake(WakeCause::Failed));
        let mut wake =
            CrownWake::new(ulid::Ulid::new(), WakeCause::Failed, None, None, Owed::default());
        wake.failed = Some(FailedTurn {
            session: uuid::Uuid::nil(),
            turn: "turn-a".into(),
            reason: FailReason::Server,
        });
        assert_eq!(wake.clause(), "ended its turn on an error");
        assert_eq!(wake.changed(), ["server error"]);
        // A delivery owed before it keeps its delta beside the reason.
        let ahead = branch("bbb", "ahead", 1, "REVIEW");
        wake.fold(WakeCause::Delivered, None, Some(ahead));
        assert_eq!(wake.cause, WakeCause::Failed);
        assert_eq!(wake.changed()[0], "server error");
        wake.late = true;
        assert_eq!(wake.changed().last().map(String::as_str), Some("after a restart"));
        // A stronger cause folded over it says its own line, not the reason.
        wake.fold(WakeCause::Raised, None, None);
        assert!(!wake.changed().iter().any(|c| c == "server error"));
    }

    /// T-936: a Codex seat is polled, so its failure is judged by its turn
    /// id. Two turns that failed between two polls into the same state are
    /// two failures with no change between them; the same turn read again,
    /// a restart's `Unknown` edge into it included, is nothing; the same
    /// turn in another failure is that failure refined; a snapshot with no
    /// turn id falls back to its change.
    #[test]
    fn a_codex_failure_is_its_turn() {
        use mesimon_core::board::FailReason;
        let failed = |reason| SessionState::Failed { reason };
        let change = |from: SessionState, to: SessionState| Change {
            from,
            to,
            attention_added: false,
            confidence: Confidence::High,
        };
        let server = failed(FailReason::Server);
        let new = Some(Failure { reason: FailReason::Server, new: true });
        let into = change(SessionState::Running, server.clone());
        assert_eq!(codex_failure(&server, "turn-a", None, Some(&into)), new, "the first");
        assert_eq!(codex_failure(&server, "turn-b", Some("turn-a"), None), new, "no poll between");
        assert_eq!(codex_failure(&server, "turn-b", Some("turn-b"), None), None, "read again");
        let restart = change(
            SessionState::Unknown { reason: UnknownReason::DaemonRestarted },
            server.clone(),
        );
        assert_eq!(codex_failure(&server, "turn-b", Some("turn-b"), Some(&restart)), None);
        let refined = change(failed(FailReason::Unknown), server.clone());
        assert_eq!(
            codex_failure(&server, "turn-b", Some("turn-b"), Some(&refined)),
            Some(Failure { reason: FailReason::Server, new: false })
        );
        assert_eq!(codex_failure(&server, "turn-c", Some("turn-b"), Some(&refined)), new);
        assert_eq!(codex_failure(&server, "", None, Some(&into)), new, "no turn id: its change");
        assert_eq!(codex_failure(&server, "", None, None), None);
        assert_eq!(codex_failure(&SessionState::Running, "turn-d", None, None), None);
    }

    /// T-936, the fourth read: the fence refuses a report about the
    /// attempt's own frame and pane key from a seat with no pane (killed:
    /// the record keeps its key), and one from a replacement pane, and
    /// takes it only from the live pane the frame went to.
    #[test]
    fn a_wake_report_from_a_dead_or_replaced_pane_settles_nothing() {
        let session = uuid::Uuid::from_u128(7);
        let e = Entering {
            crown: ulid::Ulid::from_parts(1, 1),
            session,
            text: "T-2 raised its hand".into(),
            wakes: Vec::new(),
            telling: Telling::default(),
            person_came: false,
            frame: Some("01FRAME".into()),
            pane: Some("100:%3".into()),
            expires_ms: Some(1),
            parked: false,
        };
        let mut rec = SessionRecord::new(
            session,
            SessionKind::Claude,
            ulid::Ulid::from_parts(1, 2),
            Vec::new(),
            "/".into(),
            SessionState::Idle { stop_reason: StopReason::EndTurn },
        );
        rec.pane_key = Some("100:%3".into());
        assert!(submitted_fence(&e, Some(&rec), session, "01FRAME"), "its live pane");
        assert!(!submitted_fence(&e, Some(&rec), session, "01OTHER"), "another frame");
        assert!(!submitted_fence(&e, Some(&rec), uuid::Uuid::from_u128(8), "01FRAME"));
        rec.state = SessionState::Exited { reason: mesimon_core::board::ExitReason::Killed };
        assert!(!submitted_fence(&e, Some(&rec), session, "01FRAME"), "a dead seat, same key");
        rec.state = SessionState::Idle { stop_reason: StopReason::EndTurn };
        rec.pane_key = Some("200:%0".into());
        assert!(!submitted_fence(&e, Some(&rec), session, "01FRAME"), "a replacement pane");
        assert!(!submitted_fence(&e, None, session, "01FRAME"), "no seat");
    }

    /// T-936, the fourth read: a failed delivery holds the drain for the
    /// current crown's seat alone, until that seat's state moves; crown B's
    /// seat is never held by crown A's failure.
    #[test]
    fn a_failed_wake_holds_only_the_seat_it_failed_on() {
        let (a, b) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let hold = Some((a, Some(10)));
        assert!(failed_hold(hold, Some((a, Some(10)))), "the same seat, not moved");
        assert!(!failed_hold(hold, Some((a, Some(11)))), "the seat moved");
        assert!(!failed_hold(hold, Some((b, Some(10)))), "another crown's seat");
        assert!(!failed_hold(hold, None), "no seat");
        assert!(!failed_hold(None, Some((a, Some(10)))), "no hold");
    }

    /// T-945: an attached client holds the crown's wake for the bound past
    /// the end of the second its activity stamp names, and no longer; a
    /// reading the arithmetic cannot trust holds as the attachment did.
    #[test]
    fn an_attached_client_holds_the_wake_for_the_bound_after_its_activity() {
        let bound = 600_000;
        let s = 1_792_850_000_u64;
        let stamp = s * 1000;
        assert_eq!(client_hold(Some(s), stamp, bound), Some(ClientHold::Recent), "just now");
        // The stamp drops the second's fraction: activity at s.999 is still
        // `s`, so the bound runs from the second's end.
        let expiry = stamp + 1000 + bound;
        assert_eq!(client_hold(Some(s), expiry - 1, bound), Some(ClientHold::Recent), "a ms short");
        assert_eq!(client_hold(Some(s), expiry, bound), None, "the bound gone by");
        assert_eq!(client_hold(Some(s), expiry + 3_600_000, bound), None, "hours idle");
        // Renewed a ms before the bound: the new stamp holds again.
        assert_eq!(
            client_hold(Some(s + 600), expiry - 1, bound),
            Some(ClientHold::Recent),
            "renewed before the bound"
        );
        // A stamp after the sample is no reading of it, by a millisecond or
        // by half a second; the stamp's own millisecond is the sample's.
        assert_eq!(client_hold(Some(s), stamp, bound), Some(ClientHold::Recent), "the stamp's ms");
        assert_eq!(client_hold(Some(s), stamp - 1, bound), Some(ClientHold::Unknown), "1 ms ahead");
        assert_eq!(client_hold(Some(s + 1), stamp + 500, bound), Some(ClientHold::Unknown));
        assert_eq!(client_hold(None, expiry, bound), Some(ClientHold::Unknown), "no reading");
        assert_eq!(
            client_hold(Some(s + 5), stamp, bound),
            Some(ClientHold::Unknown),
            "a stamp past the sample: a clock stepped back"
        );
        assert_eq!(
            client_hold(Some(u64::MAX), u64::MAX, bound),
            Some(ClientHold::Unknown),
            "past the arithmetic"
        );
        assert_eq!(
            client_hold(Some(u64::MAX / 1000), u64::MAX, bound),
            Some(ClientHold::Unknown),
            "the expiry past the arithmetic"
        );
    }

    /// T-945: the bound's seam is refused at zero or unreadable, never a
    /// hold turned off, and the row's words carry the bound.
    #[test]
    fn the_idle_bound_refuses_a_zero_seam_and_names_itself() {
        assert_eq!(idle_bound(None), PERSON_IDLE_MS);
        assert_eq!(idle_bound(Some("0")), PERSON_IDLE_MS);
        assert_eq!(idle_bound(Some("soon")), PERSON_IDLE_MS);
        assert_eq!(idle_bound(Some("-5")), PERSON_IDLE_MS);
        assert_eq!(idle_bound(Some("5000")), 5000);
        assert_eq!(span_words(PERSON_IDLE_MS), "10m");
        assert_eq!(span_words(5000), "5s");
        assert_eq!(span_words(90_000), "90s");
        assert_eq!(span_words(1500), "1500ms");
        assert_eq!(PersonHold::Draft.word(), "after your draft");
        assert_eq!(PersonHold::Client(ClientHold::Unknown).word(), "idle unknown ∙ leave");
        assert!(PersonHold::Client(ClientHold::Recent).word().starts_with("leave or "));
    }

    #[test]
    fn coalesced_causes_keep_the_strongest() {
        assert!(WakeCause::Asked > WakeCause::Planned);
        assert!(WakeCause::Planned > WakeCause::Raised);
        assert!(WakeCause::Raised > WakeCause::Limited);
        assert!(WakeCause::Limited > WakeCause::Failed);
        assert!(WakeCause::Failed > WakeCause::Answered);
        assert!(WakeCause::Answered > WakeCause::Delivered);
        assert!(WakeCause::Delivered > WakeCause::Merged);
        assert!(WakeCause::Merged > WakeCause::Finished);
    }

    /// T-785, lab run 3's A: a forge wake owed with the card in PROD
    /// waits for the crown's turn, the crown moves the card to DONE, and
    /// the line that goes out says DONE, as `get_ticket` does then. The
    /// edge stays as heard; a card sent where the crown last heard it says
    /// no column. `forge_e2e` pins `telling`, the other half.
    #[test]
    fn a_wake_says_the_column_it_is_sent_from() {
        let before = branch("aaa", "merged", 0, "REVIEW");
        let mut owed = branch("aaa", "merged", 0, "PROD");
        owed.forge = Some("deployed to production !11".into());
        let wake = CrownWake::new(
            ulid::Ulid::new(),
            WakeCause::Forge,
            Some(before.clone()),
            Some(owed),
            Owed::default(),
        );
        assert_eq!(wake.changed(), ["column PROD", "deployed to production !11"]);
        assert_eq!(wake.sent_at("DONE").changed(), ["column DONE", "deployed to production !11"]);
        assert_eq!(wake.sent_at("REVIEW").changed(), ["deployed to production !11"]);
    }

    fn heard(judged: Option<Told>, told: Option<Told>) -> Heard {
        Heard { judged, told, ..Heard::default() }
    }

    /// A held delivery (T-554) is heard once, at its merge: the crown was
    /// never told of the tip, so the line is the delivery with `merged` in
    /// its delta — whatever the train's rebase did to the tip on the way.
    #[test]
    fn a_held_delivery_is_heard_at_its_merge() {
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        let held = Held { cause: WakeCause::Delivered, look: BranchLook::default() };
        let h = Heard { deferred: Some(held), ..heard(Some(rebased), None) };
        assert!(h.awaits_merge());
        assert!(!heard(None, None).awaits_merge());
        let landed = branch("bbb", "merged", 0, "DONE");
        assert_eq!(merge_verdict(true, &h, &landed), Some(WakeCause::Delivered));
        assert_eq!(delta(h.told.as_ref(), &landed), vec!["merge_state merged", "column DONE"]);
    }

    /// T-527's own case: the crown was woken for the delivery, then the
    /// branch lands — one `merged` line saying so, whoever merged it. The
    /// merge flow's own rebase of that delivery is the same landing.
    #[test]
    fn a_merge_after_its_delivery_is_one_merged_line() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let landed = branch("aaa", "merged", 0, "REVIEW");
        let h = heard(Some(delivered.clone()), Some(delivered.clone()));
        assert_eq!(merge_verdict(true, &h, &landed), Some(WakeCause::Merged));
        // An ff merge reads `ahead 0`; the word says it, the count is noise.
        assert_eq!(delta(Some(&delivered), &landed), vec!["merge_state ahead → merged"]);
        // Told of the delivery behind the base; `m` asked for the rebase (a
        // merge step, judged but not told) and merged the rebased tip.
        let behind = branch("aaa", "needs_rebase", 1, "REVIEW");
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        let landed = branch("bbb", "merged", 0, "DONE");
        let h = heard(Some(rebased), Some(behind.clone()));
        assert_eq!(merge_verdict(true, &h, &landed), Some(WakeCause::Merged));
        assert_eq!(
            delta(Some(&behind), &landed),
            vec!["merge_state needs_rebase → merged", "column DONE"]
        );
    }

    /// A tip the crown never heard of, delivered and merged between two of
    /// its turns, is ONE line: the delivery, with `merged` in its delta.
    #[test]
    fn a_merge_the_crown_never_heard_of_is_its_delivery() {
        let landed = branch("aaa", "merged", 0, "REVIEW");
        // Nothing heard at all (a restart, or a merge inside the turn).
        assert_eq!(merge_verdict(true, &Heard::default(), &landed), Some(WakeCause::Delivered));
        assert_eq!(delta(None, &landed), vec!["merge_state merged", "column REVIEW"]);
        // A merge step judged it, but no wake ever told the crown.
        let h = heard(Some(branch("aaa", "ahead", 1, "REVIEW")), None);
        assert_eq!(merge_verdict(true, &h, &landed), Some(WakeCause::Delivered));
        // Told of older work; newer commits landed before any turn ended.
        let older = branch("000", "ahead", 1, "REVIEW");
        let h = heard(Some(older.clone()), Some(older));
        assert_eq!(merge_verdict(true, &h, &landed), Some(WakeCause::Delivered));
        // The delivery still waiting on a working crown takes the merge in:
        // one line, the delivery's, its delta running to the merge.
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let mut w = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Delivered,
            None,
            Some(delivered.clone()),
            Owed::default(),
        );
        let h = heard(Some(delivered.clone()), Some(delivered.clone()));
        let cause = merge_verdict(true, &h, &landed).unwrap();
        w.fold(cause, Some(delivered), Some(landed));
        assert_eq!(w.cause, WakeCause::Delivered);
        assert_eq!(
            delta(w.from.as_ref(), w.to.as_ref().unwrap()),
            vec!["merge_state merged", "column REVIEW"]
        );
        // Merged once, then new work merged again: the word is said again.
        let first = branch("aaa", "merged", 0, "DONE");
        let again = branch("ccc", "merged", 0, "DONE");
        assert_eq!(delta(Some(&first), &again), vec!["merge_state merged"]);
    }

    /// One wake per merge: a second reading at `merged` is silent, and so
    /// is one that flapped back and forth at the same tip.
    #[test]
    fn a_merge_already_told_is_silent() {
        let landed = branch("aaa", "merged", 0, "DONE");
        let h = heard(Some(landed.clone()), Some(landed.clone()));
        assert_eq!(merge_verdict(true, &h, &landed), None);
        // New work after it is a delivery; that one's merge is news again.
        let more = branch("bbb", "ahead", 1, "REVIEW");
        let h = heard(Some(more.clone()), Some(more));
        assert_eq!(
            merge_verdict(true, &h, &branch("bbb", "merged", 0, "DONE")),
            Some(WakeCause::Merged)
        );
    }

    /// A shared checkout has no branch to land, and a worker this crown did
    /// not start — the crown's own ticket among them — never wakes it.
    #[test]
    fn a_checkout_or_an_unstarted_worker_has_no_merge_to_hear() {
        let head = checkout("1111111aaaa", "REVIEW");
        let h = heard(Some(head.clone()), Some(head.clone()));
        assert_eq!(merge_verdict(true, &h, &head), None);
        assert_eq!(merge_verdict(true, &Heard::default(), &head), None);
        let landed = branch("aaa", "merged", 0, "DONE");
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let h = heard(Some(delivered.clone()), Some(delivered));
        assert_eq!(merge_verdict(false, &h, &landed), None);
        assert_eq!(merge_verdict(false, &Heard::default(), &landed), None);
    }

    /// The pending list (T-596), entry by entry: anything but the train
    /// makes a finished turn silent — the turn those words run, the hand,
    /// the question or the plan is its own news — and the train holds it.
    /// None of them silences a delivery, a merge waits only for the merge
    /// flow's own words, and nothing holds an answer, a hand, a question or
    /// a plan.
    #[test]
    fn the_pending_list_weighs_every_wake() {
        let busy = [
            STEP,
            Pending { words: true, ..IDLE },
            Pending { turn: true, ..IDLE },
            Pending { hand: true, ..IDLE },
            Pending { dialog: true, ..IDLE },
        ];
        for p in &busy {
            assert_eq!(due(FINISHED, None, p), Due::Silent, "{p:?}");
        }
        for p in &busy[1..] {
            assert_eq!(due(DELIVERED, None, p), Due::Wake(WakeCause::Delivered), "{p:?}");
            assert_eq!(due(MERGED, None, p), Due::Wake(WakeCause::Merged), "{p:?}");
        }
        assert_eq!(due(DELIVERED, None, &STEP), Due::Step(WakeCause::Delivered));
        assert_eq!(due(MERGED, None, &STEP), Due::Step(WakeCause::Merged));
        assert_eq!(due(FINISHED, None, &TRAIN), Due::Hold(WakeCause::Finished));
        assert_eq!(due(MERGED, None, &TRAIN), Due::Wake(WakeCause::Merged), "no train's");
        // The train first: it will merge, and the merge is heard then.
        let both = Pending { train: true, merge_step: true, ..IDLE };
        assert_eq!(due(DELIVERED, None, &both), Due::Hold(WakeCause::Delivered));
        let causes = [WakeCause::Answered, WakeCause::Raised, WakeCause::Planned, WakeCause::Asked];
        for cause in causes {
            for p in busy.iter().chain([&TRAIN, &IDLE]) {
                assert_eq!(due(Some(cause), DELIVERED, p), Due::Wake(cause), "{cause:?} {p:?}");
            }
        }
    }

    /// T-596's own case: the train merged a delivery the crown was told of
    /// and pasted the merged notice. While the notice is on its way or its
    /// turn runs, the merge is held — both baselines moved, so another
    /// reading at `merged` is silent — and when that turn is over it is one
    /// line, `merged and finished its turn`, in the column the turn left.
    #[test]
    fn a_merge_with_its_notice_turn_is_heard_at_that_turns_end_once() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let landed = branch("aaa", "merged", 0, "REVIEW");
        let mut h = heard(Some(delivered.clone()), Some(delivered));
        let cause = merge_verdict(true, &h, &landed);
        assert_eq!(cause, MERGED);
        assert_eq!(due(cause, None, &STEP), Due::Step(WakeCause::Merged));
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed.clone(), false));
        assert_eq!(merge_verdict(true, &h, &landed), None, "the flags again, mid-turn");
        let wake = h.step_over(Some("DONE".into())).expect("the held merge");
        assert_eq!(wake.clause(), "merged and finished its turn");
        assert_eq!(wake.changed(), ["merge_state ahead → merged", "column DONE"]);
        // Once: nothing more is held, and the crown has heard the landing
        // in the column the line said.
        assert!(h.step_over(Some("DONE".into())).is_none());
        assert_eq!(merge_verdict(true, &h, &branch("aaa", "merged", 0, "DONE")), None);
        assert!(delta(h.told.as_ref(), &branch("aaa", "merged", 0, "DONE")).is_empty());
        // Folded into a stronger line owed to a working crown, the turn is
        // that line's to say.
        let mut w = wake.clone();
        w.fold(WakeCause::Raised, None, None);
        assert_eq!(w.clause(), "raised its hand");
    }

    /// A merge with no step in flight — no live agent, a merge the train
    /// could not notify, a person's `m` whose notice is not sent yet — is
    /// heard at once, as before, and says nothing of a turn.
    #[test]
    fn a_merge_with_no_notice_turn_is_heard_at_once() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let landed = branch("aaa", "merged", 0, "REVIEW");
        let h = heard(Some(delivered.clone()), Some(delivered.clone()));
        let cause = merge_verdict(true, &h, &landed);
        assert_eq!(due(cause, None, &IDLE), Due::Wake(WakeCause::Merged));
        let wake = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Merged,
            Some(delivered),
            Some(landed),
            Owed::default(),
        );
        assert_eq!(wake.clause(), "merged");
    }

    /// T-554's held delivery, landed by the train with its notice: the
    /// merge reads a tip the crown never heard of, so the line is the
    /// delivery with `merged` in its delta — held, like any landing, until
    /// the notice turn is over, and said once. A ticket gone from the board
    /// by then takes what was held with it.
    #[test]
    fn a_held_delivery_landed_by_the_train_is_heard_after_the_notice_turn() {
        let behind = branch("aaa", "needs_rebase", 1, "REVIEW");
        let held = Held { cause: WakeCause::Delivered, look: BranchLook::default() };
        let mut h = Heard { deferred: Some(held), ..heard(Some(behind), None) };
        let landed = branch("bbb", "merged", 0, "REVIEW");
        let cause = merge_verdict(true, &h, &landed);
        assert_eq!(cause, DELIVERED);
        assert_eq!(due(cause, None, &STEP), Due::Step(WakeCause::Delivered));
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Delivered, landed.clone(), false));
        assert!(h.deferred.is_none(), "the train's part is done");
        assert!(h.awaits_merge());
        assert_eq!(merge_verdict(true, &h, &landed), None, "heard once");
        let wake = h.step_over(Some("DONE".into())).expect("the held delivery");
        assert_eq!(wake.clause(), "delivered and finished its turn");
        assert_eq!(wake.changed(), ["merge_state merged", "column DONE"]);
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed, false));
        assert!(h.step_over(None).is_none(), "the ticket is gone");
        assert!(h.stepped.is_none());
    }

    /// A forge read's `late` (T-937) rides a landing held for its merge
    /// step, newly held or folded into one held already, after the
    /// restart's look cleared `restored`: the step's release says
    /// `after a restart`. One held with neither says nothing of a restart.
    #[test]
    fn a_forge_read_s_late_rides_a_landing_held_for_its_step() {
        let landed = branch("bbb", "merged", 0, "REVIEW");
        let mut h = heard(None, None);
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed.clone(), true));
        let wake = h.step_over(Some("DONE".into())).expect("held");
        assert!(wake.late);
        assert_eq!(wake.changed().last().map(String::as_str), Some("after a restart"));
        // Held by a live reading first, then the restored read's news folds
        // in: the fold keeps the mark.
        let mut h = heard(None, None);
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Delivered, landed.clone(), false));
        assert!(!h.stepped.as_ref().unwrap().late);
        assert!(!h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed.clone(), true));
        let wake = h.step_over(Some("DONE".into())).expect("held");
        assert!(wake.late, "the fold keeps the read's mark");
        assert_eq!(wake.changed().last().map(String::as_str), Some("after a restart"));
        // Neither the read nor the ledger says restart: nothing does.
        let mut h = heard(None, None);
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed, false));
        let wake = h.step_over(Some("DONE".into())).expect("held");
        assert!(!wake.changed().iter().any(|w| w == "after a restart"));
    }

    /// A watched person's ticket (T-712) through a landing held for its
    /// merge step (T-596), as `hold_for_step` takes it from `hear_merges`,
    /// `hear_forge`, `hear_deferred` and a turn's end, and `hear_stepped`
    /// lets it go (T-1071): the bed's agent runs no step, so the release
    /// is the step's end.
    fn watched_bed(name: &str, told: Option<Told>) -> crate::server::crownask::ledger_tests::Bed {
        let mut b = crate::server::crownask::ledger_tests::bed(name);
        b.d.board.sessions[0].started_by = None;
        b.d.crown_watched.insert(b.worker);
        b.d.crown_heard.insert(b.worker, heard(told.clone(), told));
        b
    }

    /// The wake owed for the bed's worker, if any.
    fn owed_for(d: &crate::server::Daemon, worker: ulid::Ulid) -> Option<&CrownWake> {
        d.crown_wakes.iter().find(|w| w.worker == worker)
    }

    #[test]
    fn an_answer_reporting_the_merge_consumes_its_held_merge_wake() {
        let mut b = watched_bed("answered-merge", Some(branch("aaa", "ahead", 1, "REVIEW")));
        let w = b.worker;
        let merged = branch("aaa", "merged", 1, "DONE");
        b.d.hold_for_step(w, WakeCause::Merged, merged.clone(), false);
        b.d.enter_wake(b.crown, w, WakeCause::Answered, None, Some(merged), Owed::default());
        let telling = b.d.telling();
        let wakes = std::mem::take(&mut *b.d.crown_wakes);
        let seat = uuid::Uuid::from_u128(1036);
        let mut entering = in_box(b.crown, seat, "100:%2", wakes);
        entering.telling = telling;
        *b.d.crown_wake_entering = Some(entering);
        b.d.wake_entered(seat);
        assert!(!b.d.hear_stepped(), "the answered turn already heard this merge");
        assert!(b.d.crown_wakes.is_empty());
        assert!(!b.d.crown_watched.contains(&w), "observing the merge still ends its watch");
    }

    #[test]
    fn a_keyed_read_consumes_old_news_but_keeps_other_workers_and_later_events() {
        let mut b = watched_bed("observed-read", None);
        let w = b.worker;
        sampled_branch(&mut b, "aaa", true);
        let other = ulid::Ulid(3);
        b.d.enter_wake(b.crown, w, WakeCause::Answered, None, None, Owed::default());
        b.d.enter_wake(b.crown, other, WakeCause::Finished, None, None, Owed::default());
        b.d.hold_for_step(w, WakeCause::Merged, branch("aaa", "merged", 1, "DONE"), false);
        // Constructing a board view alone observes no ticket's news.
        let mut view = b.d.agent_ticket_view(w).unwrap();
        assert!(owed_for(&b.d, w).is_some());
        b.d.crown_reads(&mut view, w);
        assert!(owed_for(&b.d, w).is_none());
        assert!(!b.d.hear_stepped());
        assert!(owed_for(&b.d, other).is_some());
        assert!(!b.d.crown_watched.contains(&w));
        // A later answer, even at the same tip, and a later merge are new.
        b.d.enter_wake(
            b.crown,
            w,
            WakeCause::Answered,
            None,
            Some(branch("aaa", "merged", 1, "DONE")),
            Owed::default(),
        );
        b.d.hold_for_step(w, WakeCause::Merged, branch("bbb", "merged", 1, "DONE"), false);
        assert!(b.d.hear_stepped());
        let wake = owed_for(&b.d, w).unwrap();
        assert_eq!(wake.cause, WakeCause::Answered);
        assert_eq!(wake.to.as_ref().unwrap().tip, "bbb");
    }

    #[test]
    fn narrow_write_receipts_do_not_consume_unseen_news() {
        for (i, action) in ["noted", "renamed", "tagged"].into_iter().enumerate() {
            for cause in [
                WakeCause::Answered,
                WakeCause::Failed,
                WakeCause::Asked,
                WakeCause::Limited,
                WakeCause::Forge,
                WakeCause::Merged,
            ] {
                let mut b = watched_bed(&format!("wr-{i}-{cause:?}"), None);
                let w = b.worker;
                b.d.enter_wake(b.crown, w, cause, None, None, Owed::default());
                b.d.hold_for_step(w, WakeCause::Merged, branch("aaa", "merged", 1, "DONE"), false);
                let write = b.d.receipt_write(w, None);
                b.d.crown_touched(b.crown, w, action, write);
                assert_eq!(owed_for(&b.d, w).unwrap().cause, cause, "{action}");
                assert!(b.d.hear_stepped(), "a narrow receipt did not say the merge either");
            }
        }
    }

    #[test]
    fn a_delayed_merge_acknowledgment_keeps_new_answers_forge_news_and_other_tips() {
        for cause in [WakeCause::Answered, WakeCause::Forge, WakeCause::Merged] {
            let mut b = watched_bed(&format!("late-ack-{cause:?}"), None);
            let w = b.worker;
            sampled_branch(&mut b, "aaa", true);
            let old = CrownWake::new(
                w,
                WakeCause::Answered,
                None,
                Some(branch("aaa", "merged", 1, "DONE")),
                Owed::default(),
            );
            let seat = uuid::Uuid::from_u128(1036);
            *b.d.crown_wake_entering = Some(in_box(b.crown, seat, "100:%9", vec![old]));
            let mut view = b.d.agent_ticket_view(w).unwrap();
            b.d.crown_reads(&mut view, w);
            let mut newer =
                branch(if cause == WakeCause::Merged { "bbb" } else { "aaa" }, "merged", 1, "DONE");
            if cause == WakeCause::Forge {
                newer.forge = Some("pipeline failed #43".into());
            }
            b.d.enter_wake(b.crown, w, cause, None, Some(newer.clone()), Owed::default());
            b.d.wake_entered(seat);
            let wake = owed_for(&b.d, w).unwrap();
            assert_eq!(wake.cause, cause);
            assert_eq!(wake.to.as_ref(), Some(&newer));
        }
    }

    #[test]
    fn a_read_of_a_turn_waiting_for_the_forge_keeps_news_that_arrives_afterward() {
        for opening in [false, true] {
            let mut b = watched_bed(&format!("observed-forge-{opening}"), None);
            let w = b.worker;
            let wake = CrownWake::new(
                w,
                WakeCause::Answered,
                None,
                Some(branch("aaa", "ahead", 1, "REVIEW")),
                Owed::default(),
            );
            b.d.crown_heard.get_mut(&w).unwrap().forging = Some(Forging { wake, since_ms: 0 });
            let mut view = b.d.agent_ticket_view(w).unwrap();
            b.d.crown_reads(&mut view, w);
            let openings =
                if opening { HashMap::from([(w, "opened #43".into())]) } else { HashMap::new() };
            assert_eq!(b.d.hear_forged(openings), opening);
            if opening {
                let wake = owed_for(&b.d, w).unwrap();
                assert_eq!(wake.cause, WakeCause::Forge);
                assert_eq!(wake.to.as_ref().unwrap().forge.as_deref(), Some("opened #43"));
            } else {
                assert!(owed_for(&b.d, w).is_none());
            }
        }
    }

    #[test]
    fn review_a_new_finished_turn_rearms_an_observed_forge_slot() {
        let mut b = watched_bed("rv-forge-end", None);
        let w = b.worker;
        b.d.forge.remotes.insert(
            "/r".into(),
            Some(mesimon_core::forge::Remote { host: "github.com".into(), path: "o/r".into() }),
        );
        b.d.forge.looking.insert(w);
        let state = branch("aaa", "ahead", 1, "REVIEW");
        b.d.owe_turn_end(b.crown, w, WakeCause::Finished, None, Some(state.clone()), None);
        let mut view = b.d.agent_ticket_view(w).unwrap();
        b.d.crown_reads(&mut view, w);
        b.d.owe_turn_end(b.crown, w, WakeCause::Finished, Some(state.clone()), Some(state), None);
        b.d.forge.looking.remove(&w);
        assert!(b.d.hear_forged(HashMap::new()), "the new end survives a read with no opening");
        assert_eq!(owed_for(&b.d, w).unwrap().cause, WakeCause::Finished);
    }

    #[test]
    fn review_a_delayed_merge_ack_keeps_a_new_finished_turn_folded_into_its_merge() {
        let mut b = watched_bed("rv-merge-end", None);
        let w = b.worker;
        let merged = branch("aaa", "merged", 1, "DONE");
        let old =
            CrownWake::new(w, WakeCause::Answered, None, Some(merged.clone()), Owed::default());
        let seat = uuid::Uuid::from_u128(1036);
        *b.d.crown_wake_entering = Some(in_box(b.crown, seat, "100:%9", vec![old]));
        b.d.enter_wake(b.crown, w, WakeCause::Merged, None, Some(merged.clone()), Owed::default());
        b.d.enter_wake(
            b.crown,
            w,
            WakeCause::Finished,
            Some(merged.clone()),
            Some(merged),
            Owed::default(),
        );
        assert!(owed_for(&b.d, w).unwrap().finished);
        b.d.wake_entered(seat);
        assert!(
            owed_for(&b.d, w)
                .is_some_and(|wake| wake.finished || wake.cause == WakeCause::Finished),
            "the old merge's acknowledgment cannot consume the later end"
        );
    }

    #[test]
    fn a_wakes_internal_merged_baseline_is_not_a_report_of_the_merge() {
        let mut b = watched_bed("merge-unsaid", None);
        let w = b.worker;
        let merged = branch("aaa", "merged", 1, "DONE");
        b.d.hold_for_step(w, WakeCause::Merged, merged.clone(), false);
        let answer = CrownWake::new(
            w,
            WakeCause::Answered,
            Some(merged.clone()),
            Some(merged),
            Owed::default(),
        );
        assert!(answer.changed().is_empty(), "the answer carries no merge fact");
        b.d.told(b.crown, Telling::default(), &[answer]);
        assert!(b.d.hear_stepped(), "the unreported merge is still owed");
    }

    #[test]
    fn a_ticket_view_that_does_not_report_merged_keeps_the_pending_merge() {
        let mut b = watched_bed("merge-unread", None);
        let w = b.worker;
        sampled_branch(&mut b, "aaa", true);
        b.d.hold_for_step(w, WakeCause::Merged, branch("aaa", "merged", 1, "DONE"), false);
        // A later open PR can replace merge_state's standing merged word.
        let mut view = b.d.agent_ticket_view(w).unwrap();
        view.merge_state = Some("open PR #43".into());
        b.d.crown_reads(&mut view, w);
        assert!(b.d.hear_stepped(), "an internal merged flag was not returned");
    }

    #[test]
    fn an_observed_wake_is_not_restored_from_a_lost_box() {
        let mut b = watched_bed("observed-return", None);
        let w = b.worker;
        let other = ulid::Ulid(3);
        let gone = uuid::Uuid::from_u128(1036);
        let wakes = vec![wake(w, WakeCause::Answered), wake(other, WakeCause::Finished)];
        *b.d.crown_wake_entering = Some(in_box(b.crown, gone, "100:%9", wakes.clone()));
        b.d.crown_wakes_taken.push(in_box(b.crown, gone, "100:%9", wakes));
        let mut view = b.d.agent_ticket_view(w).unwrap();
        b.d.crown_reads(&mut view, w);
        b.d.drain_crown_wakes(None);
        assert!(owed_for(&b.d, w).is_none());
        assert!(owed_for(&b.d, other).is_some());
    }

    fn sampled_branch(b: &mut crate::server::crownask::ledger_tests::Bed, tip: &str, merged: bool) {
        b.d.worktrees.insert(
            b.worker,
            Binding {
                path: b.d.paths.repo_root.clone(),
                branch: "msmn/worker".into(),
                base_oid: "base".into(),
                branch_oid: tip.into(),
                status: BindingStatus::Attached,
                locked: false,
                repos: Vec::new(),
                init: None,
            },
        );
        b.d.wt_agg.insert(
            b.worker,
            worktree::Aggregate { tip: tip.into(), merged, ahead: 1, ..Default::default() },
        );
    }

    #[test]
    fn a_read_before_the_merge_tick_hears_that_merge_but_not_a_later_one() {
        let mut b = watched_bed("read-merge-tick", None);
        let w = b.worker;
        sampled_branch(&mut b, "aaa", false);
        let mut view = b.d.agent_ticket_view(w).unwrap();
        b.d.crown_reads(&mut view, w);
        sampled_branch(&mut b, "aaa", true);
        b.d.crown_landed.push(w);
        assert!(b.d.hear_merges(), "a read before the merge cannot consume it");
        assert!(owed_for(&b.d, w).is_some());

        b.d.crown_watched.insert(w);
        sampled_branch(&mut b, "bbb", true);
        b.d.crown_landed.push(w);
        let mut view = b.d.agent_ticket_view(w).unwrap();
        b.d.crown_reads(&mut view, w);
        assert!(!b.d.hear_merges(), "the read carried the merge before its wake was composed");
        assert!(owed_for(&b.d, w).is_none());
        assert!(!b.d.crown_watched.contains(&w));
    }

    #[test]
    fn a_merge_receipt_consumes_its_merge_but_preserves_unrelated_news() {
        for cause in [
            WakeCause::Merged,
            WakeCause::Answered,
            WakeCause::Failed,
            WakeCause::Asked,
            WakeCause::Limited,
            WakeCause::Forge,
        ] {
            let mut b = watched_bed(&format!("mrc-{cause:?}"), None);
            let w = b.worker;
            sampled_branch(&mut b, "aaa", true);
            let mut to = branch("aaa", "merged", 1, "DONE");
            if cause == WakeCause::Forge {
                to.forge = Some("pipeline failed #43".into());
            }
            b.d.enter_wake(b.crown, w, cause, None, Some(to), Owed::default());
            b.d.hold_for_step(w, WakeCause::Merged, branch("aaa", "merged", 1, "DONE"), false);
            let write = b.d.receipt_write(w, None);
            b.d.crown_touched(b.crown, w, "merged", write);
            assert!(!b.d.hear_stepped());
            assert_eq!(
                owed_for(&b.d, w).map(|w| w.cause),
                (cause != WakeCause::Merged).then_some(cause)
            );
        }
    }

    #[test]
    fn observing_an_old_queued_merge_does_not_end_a_later_watch() {
        let mut b = watched_bed("read-rewatch", None);
        let w = b.worker;
        sampled_branch(&mut b, "aaa", true);
        b.d.enter_wake(
            b.crown,
            w,
            WakeCause::Merged,
            None,
            Some(branch("aaa", "merged", 1, "DONE")),
            Owed::default(),
        );
        assert!(!b.d.crown_watched.contains(&w));
        b.d.crown_watched.insert(w);
        let mut view = b.d.agent_ticket_view(w).unwrap();
        b.d.crown_reads(&mut view, w);
        assert!(b.d.crown_watched.contains(&w), "the queued merge already ended its watch");
        assert!(owed_for(&b.d, w).is_none());
    }

    #[test]
    fn an_observed_merge_consumes_only_its_delivery_still_held_for_the_train() {
        for (i, (cause, tip)) in [
            (WakeCause::Delivered, "aaa"),
            (WakeCause::Delivered, "bbb"),
            (WakeCause::Finished, "aaa"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut b = watched_bed(&format!("read-train-{i}"), None);
            let w = b.worker;
            sampled_branch(&mut b, "aaa", true);
            b.d.crown_heard.get_mut(&w).unwrap().deferred = Some(Held {
                cause,
                look: BranchLook { tip: tip.into(), ahead: 1, ..Default::default() },
            });
            b.d.crown_observed_merge(w);
            assert_eq!(
                b.d.crown_heard[&w].deferred.is_none(),
                cause == WakeCause::Delivered && tip == "aaa"
            );
            assert!(!b.d.crown_watched.contains(&w), "the successful merge ends its watch");
        }
    }

    #[test]
    fn a_ticket_read_keeps_material_details_not_proven_by_its_current_state() {
        for cause in [WakeCause::Failed, WakeCause::Asked, WakeCause::Limited, WakeCause::Forge] {
            let mut b = watched_bed(&format!("alert-{cause:?}"), None);
            let w = b.worker;
            b.d.enter_wake(b.crown, w, cause, None, None, Owed::default());
            let mut view = b.d.agent_ticket_view(w).unwrap();
            b.d.crown_reads(&mut view, w);
            assert_eq!(owed_for(&b.d, w).unwrap().cause, cause);
        }
    }

    /// The forge's news on the bed's worker, as a pass hands it on.
    fn forge_news(worker: ulid::Ulid, words: &str, late: bool) -> ForgeNews {
        ForgeNews { worker, words: words.into(), late, merged: false }
    }

    /// The bed's worker off the board: archived, or snoozed until later.
    fn archive(d: &mut crate::server::Daemon, worker: ulid::Ulid, snooze: bool) {
        let until = snooze.then(|| format!("@{}", now_ms() / 1000 + 3600));
        let archived = mesimon_core::board::Archived {
            at: "@1".into(),
            by: "local".into(),
            until,
            needs_you: false,
        };
        d.board.ticket_mut(worker).unwrap().archived = Some(archived);
    }

    /// A PR on the bed's worker, as the forge's last read bound it.
    fn bind_pr(d: &mut crate::server::Daemon, worker: ulid::Ulid) {
        use mesimon_core::forge::{Forge, Pipeline, Pr, PrState};
        let pr = Pr {
            forge: Forge::GitHub,
            repo: "o/r".into(),
            number: 43,
            title: String::new(),
            url: "https://github.com/o/r/pull/43".into(),
            state: PrState::Open,
            head_oid: "h".into(),
            head_ref: "b".into(),
            merge_oid: String::new(),
            conflict: false,
            review: Default::default(),
            pipeline: Pipeline::Passed,
            pipeline_run: 0,
            updated_ms: 1,
            merged_ms: 0,
            deployed: Vec::new(),
            landed_tips: Vec::new(),
            reviewed: false,
        };
        d.forge.sets.insert(worker, vec![pr]);
    }

    /// An archived or snoozed ticket's forge news wakes no crown (T-1076),
    /// after a restart too, on every claim the crown has on it: one it
    /// started (its agent parked, as an archive leaves it), one it watches,
    /// and an unattended PR. The same ticket in DONE, unarchived, still
    /// wakes it.
    #[test]
    fn forge_news_on_a_ticket_off_the_board_wakes_no_crown() {
        type Claim = fn(&mut crate::server::crownask::ledger_tests::Bed);
        let started: Claim = |b| {
            b.d.board.sessions[0].started_by = Some(b.crown);
            b.d.board.sessions[0].state = SessionState::Sleeping;
        };
        let watched: Claim = |b| {
            b.d.board.sessions[0].started_by = None;
            b.d.crown_watched.insert(b.worker);
        };
        let unattended: Claim = |b| {
            b.d.board.sessions.clear();
            b.d.board.crown_watches = true;
            bind_pr(&mut b.d, b.worker);
        };
        for (name, claim) in [("started", started), ("watched", watched), ("pr", unattended)] {
            for off in [None, Some(false), Some(true)] {
                let mut b = crate::server::crownask::ledger_tests::bed(&format!("arch-{name}"));
                let w = b.worker;
                claim(&mut b);
                b.d.board.ticket_mut(w).unwrap().column = "DONE".into();
                if let Some(snooze) = off {
                    archive(&mut b.d, w, snooze);
                }
                let woke = b.d.hear_forge(vec![forge_news(w, "pipeline passed #43", true)]);
                let owed = owed_for(&b.d, w).map(|o| o.cause);
                if off.is_some() {
                    assert!(!woke && owed.is_none(), "{name}, {off:?}: {owed:?}");
                } else {
                    assert!(woke, "{name}: DONE and on the board");
                    assert_eq!(owed, Some(WakeCause::Forge), "{name}");
                }
            }
        }
    }

    /// The forge's merge of an archived ticket's branch (its flags flip on
    /// the merged clause) tells no crown, and moves the baselines the crown
    /// has, as for any worker it does not hear (T-936).
    #[test]
    fn an_archived_ticket_s_merge_wakes_no_crown() {
        let mut b = watched_bed("arch-merge", Some(branch("aaa", "ahead", 1, "DONE")));
        let w = b.worker;
        archive(&mut b.d, w, false);
        b.d.wt_agg.insert(
            w,
            crate::worktree::Aggregate { merged: true, tip: "aaa".into(), ..Default::default() },
        );
        b.d.crown_landed.push(w);
        assert!(!b.d.hear_merges());
        assert!(owed_for(&b.d, w).is_none());
        let told = b.d.crown_heard.get(&w).and_then(|h| h.told.as_ref());
        assert_eq!(told.and_then(|t| t.merge), Some("merged"), "the baseline moved");
    }

    /// Forge news owed while the ticket was on the board and still queued
    /// when it left (T-1076) is not told: owed, restored or given back by a
    /// box, the drain drops it. A turn's wake on the same ticket stays, and
    /// news on a ticket still on the board goes as before.
    #[test]
    fn forge_news_queued_before_an_archive_is_not_told_after_it() {
        let mut b = watched_bed("arch-queued", None);
        let (w, other) = (b.worker, ulid::Ulid(3));
        b.d.crown_watched.insert(other);
        b.d.hear_forge(vec![forge_news(w, "pipeline passed #43", false)]);
        b.d.hear_forge(vec![forge_news(other, "pipeline failed #44", false)]);
        assert_eq!(owed_for(&b.d, w).map(|o| o.cause), Some(WakeCause::Forge));
        archive(&mut b.d, w, false);
        assert!(b.d.drain_crown_wakes(None), "the drop is a change");
        assert!(owed_for(&b.d, w).is_none(), "queued");
        assert!(owed_for(&b.d, other).is_some(), "on the board");
        assert!(!b.d.drain_crown_wakes(None), "nothing more to drop");
        // Given back by a Codex box, or restored from `crown.json`.
        let back = CrownWake::new(w, WakeCause::Forge, None, None, Owed::default());
        b.d.owe_again(vec![back]);
        b.d.drain_crown_wakes(None);
        assert!(owed_for(&b.d, w).is_none(), "given back");
        // A turn's wake is not the forge's.
        let turn = CrownWake::new(w, WakeCause::Delivered, None, None, Owed::default());
        b.d.owe_again(vec![turn]);
        b.d.drain_crown_wakes(None);
        assert_eq!(owed_for(&b.d, w).map(|o| o.cause), Some(WakeCause::Delivered));
    }

    /// A wake the crown's box held, submitted down the mod with a window
    /// still open, on `session`'s pane `pane`.
    fn in_box(
        crown: ulid::Ulid,
        session: uuid::Uuid,
        pane: &str,
        wakes: Vec<CrownWake>,
    ) -> Entering {
        Entering {
            crown,
            session,
            text: "the wake".into(),
            wakes,
            telling: Telling::default(),
            person_came: false,
            frame: Some("F1".into()),
            pane: Some(pane.into()),
            expires_ms: Some(now_ms() + 3_600_000),
            parked: false,
        }
    }

    fn wake(worker: ulid::Ulid, cause: WakeCause) -> CrownWake {
        CrownWake::new(worker, cause, None, None, Owed::default())
    }

    /// A box the crown lost gives its wakes back inside the drain
    /// (`lose_entering`: a submitted wake whose pane went, a taken one
    /// whose pane went), and an archived ticket's forge news among them is
    /// dropped in that same drain, before delivery reads the owed list
    /// (T-1076, round 2: the drop ran first and the give-back after it).
    /// The rest come back owed as before, a turn's wake on the archived
    /// ticket included, and a box whose pane stands keeps its words.
    #[test]
    fn forge_news_a_lost_box_gives_back_is_dropped_in_the_same_drain() {
        let gone = uuid::Uuid::from_u128(99);
        // Submitted, its pane gone: owed again, and the archived one dropped.
        let mut b = watched_bed("arch-lost", None);
        let (w, other, crown) = (b.worker, ulid::Ulid(3), b.crown);
        archive(&mut b.d, w, false);
        let wakes = vec![wake(w, WakeCause::Forge), wake(other, WakeCause::Forge)];
        *b.d.crown_wake_entering = Some(in_box(crown, gone, "100:%9", wakes));
        assert!(b.d.drain_crown_wakes(None), "a box lost and a wake dropped");
        assert!(b.d.crown_wake_entering.is_none());
        assert!(owed_for(&b.d, w).is_none(), "not resent: dropped before delivery");
        assert_eq!(owed_for(&b.d, other).map(|o| o.cause), Some(WakeCause::Forge), "on the board");
        // Taken by the box, its pane gone since: the same.
        let mut b = watched_bed("arch-taken", None);
        let w = b.worker;
        archive(&mut b.d, w, false);
        b.d.crown_wakes_taken.push(in_box(crown, gone, "100:%9", vec![wake(w, WakeCause::Forge)]));
        assert!(b.d.drain_crown_wakes(None));
        assert!(b.d.crown_wakes_taken.is_empty());
        assert!(owed_for(&b.d, w).is_none(), "taken, then lost");
        // A turn's wake on the archived ticket comes back owed.
        let mut b = watched_bed("arch-lost-turn", None);
        let w = b.worker;
        archive(&mut b.d, w, false);
        *b.d.crown_wake_entering =
            Some(in_box(crown, gone, "100:%9", vec![wake(w, WakeCause::Delivered)]));
        b.d.drain_crown_wakes(None);
        assert_eq!(owed_for(&b.d, w).map(|o| o.cause), Some(WakeCause::Delivered));
        // The box still on its pane: its words are the crown's, and stay.
        let mut b = watched_bed("arch-boxed", None);
        let w = b.worker;
        archive(&mut b.d, w, false);
        let seat = uuid::Uuid::from_u128(1);
        let mut rec = mesimon_core::board::SessionRecord::new(
            seat,
            mesimon_core::board::SessionKind::Claude,
            crown,
            vec!["claude".into()],
            "/".into(),
            SessionState::Running,
        );
        rec.pane_key = Some("100:%2".into());
        b.d.board.sessions.push(rec);
        *b.d.crown_wake_entering =
            Some(in_box(crown, seat, "100:%2", vec![wake(w, WakeCause::Forge)]));
        assert!(!b.d.drain_crown_wakes(None), "nothing lost, nothing dropped");
        let held = b.d.crown_wake_entering.as_ref().map(|e| e.wakes.len());
        assert_eq!(held, Some(1), "the box's words stay");
        assert!(owed_for(&b.d, w).is_none());
    }

    /// A merge held for its step ends the watch once, at the release, and
    /// not before it: the crown has not heard it yet.
    #[test]
    fn a_watched_landing_held_for_its_step_ends_the_watch_at_its_release() {
        let mut b = watched_bed("step-merge", Some(branch("aaa", "ahead", 1, "REVIEW")));
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Merged, branch("aaa", "merged", 1, "REVIEW"), false);
        assert!(b.d.crown_watched.contains(&w), "held: nothing said yet");
        assert!(owed_for(&b.d, w).is_none());
        assert!(b.d.hear_stepped());
        assert!(!b.d.crown_watched.contains(&w), "the merge, said, ends it");
        let wake = owed_for(&b.d, w).expect("the landing");
        assert!(wake.merge && wake.changed().contains(&"merge_state ahead → merged".into()));
        assert!(!b.d.hear_stepped(), "said once");
        // Nothing heard of the worker before, the step's merge lands it.
        let mut b = watched_bed("step-merge-fresh", None);
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Delivered, branch("bbb", "merged", 1, "REVIEW"), false);
        assert!(b.d.hear_stepped());
        assert!(!b.d.crown_watched.contains(&w), "a delivery the step merged");
    }

    /// A delivery held while a merge step ran (`due`'s `Step` for
    /// `Delivered`), whose branch the step left unmerged — a refused or
    /// failed merge — is told at the release and leaves the watch standing.
    /// So does one on a branch merged before at the tip the crown heard.
    #[test]
    fn an_unmerged_delivery_released_after_its_step_keeps_the_watch() {
        let mut b = watched_bed("step-ahead", Some(branch("aaa", "ahead", 1, "REVIEW")));
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Delivered, branch("bbb", "ahead", 2, "REVIEW"), false);
        assert!(b.d.hear_stepped());
        let wake = owed_for(&b.d, w).expect("the delivery is told");
        assert!(!wake.merge);
        assert!(b.d.crown_watched.contains(&w), "no merge, the watch stands");
        // Nothing heard before, still ahead.
        let mut b = watched_bed("step-ahead-fresh", None);
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Delivered, branch("bbb", "ahead", 2, "REVIEW"), false);
        assert!(b.d.hear_stepped());
        assert!(b.d.crown_watched.contains(&w), "nothing heard, nothing merged");
        // Merged at the tip the crown was told merged: no merge since.
        let merged = branch("aaa", "merged", 1, "REVIEW");
        let mut b = watched_bed("step-same", Some(merged.clone()));
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Delivered, merged, false);
        assert!(b.d.hear_stepped());
        assert!(b.d.crown_watched.contains(&w), "merged before, at this tip");
    }

    /// A merge folded into a held delivery keeps the fact, whichever came
    /// first, and so does a delivery after it: the step's release is one
    /// line and ends the watch.
    #[test]
    fn a_merge_folded_into_a_held_delivery_ends_the_watch() {
        let mut b = watched_bed("step-fold", Some(branch("aaa", "ahead", 1, "REVIEW")));
        let w = b.worker;
        b.d.hold_for_step(w, WakeCause::Delivered, branch("bbb", "ahead", 2, "REVIEW"), false);
        b.d.hold_for_step(w, WakeCause::Merged, branch("bbb", "merged", 2, "REVIEW"), false);
        b.d.hold_for_step(w, WakeCause::Delivered, branch("ccc", "ahead", 1, "REVIEW"), false);
        assert!(b.d.hear_stepped());
        assert!(owed_for(&b.d, w).is_some_and(|wake| wake.merge));
        assert!(!b.d.crown_watched.contains(&w));
    }

    /// The fact rides `crown.json` (T-1004's held landing, T-1071): a held
    /// merge read back after a restart ends the watch at its release, a held
    /// unmerged delivery read back does not, and a held landing written
    /// before the field reads as none and is written as it was.
    #[test]
    fn a_held_landing_keeps_its_merge_across_a_restart() {
        let mut h = heard(None, Some(branch("aaa", "ahead", 1, "REVIEW")));
        assert!(h.hold_for_step(
            ulid::Ulid::nil(),
            WakeCause::Merged,
            branch("aaa", "merged", 1, "REVIEW"),
            false
        ));
        let text = serde_json::to_string(&h).unwrap();
        assert!(text.contains("\"merge\":true"), "{text}");
        let back = restarted(&h);
        assert!(back.stepped.as_ref().is_some_and(|w| w.merge));
        let mut b = watched_bed("step-restart", None);
        let w = b.worker;
        b.d.crown_heard.insert(w, back);
        assert!(b.d.hear_stepped());
        assert!(!b.d.crown_watched.contains(&w), "the restored merge ends it");
        // An unmerged delivery: nothing written, nothing read back.
        let mut h = heard(None, Some(branch("aaa", "ahead", 1, "REVIEW")));
        assert!(h.hold_for_step(
            ulid::Ulid::nil(),
            WakeCause::Delivered,
            branch("bbb", "ahead", 2, "REVIEW"),
            false
        ));
        let text = serde_json::to_string(&h).unwrap();
        assert!(!text.contains("\"merge\":true") && !text.contains("\"merge\":false"), "{text}");
        let mut b = watched_bed("step-restart-ahead", None);
        let w = b.worker;
        b.d.crown_heard.insert(w, restarted(&h));
        assert!(b.d.hear_stepped());
        assert!(b.d.crown_watched.contains(&w), "the restored delivery keeps it");
    }

    /// A worker's ledger as `crown.json` carries it through a restart
    /// (T-602), read back marked restored.
    fn restarted(h: &Heard) -> Heard {
        let mut back: Heard = serde_json::from_str(&serde_json::to_string(h).unwrap()).unwrap();
        back.restored = true;
        back
    }

    /// The baseline round-trips: both sides of it, a hold for the train and
    /// a landing held for its step, with the merge word read back as the
    /// same `&'static str` the judging compares. A word this build does not
    /// know reads as no branch.
    #[test]
    fn the_baseline_round_trips() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let mut h = heard(Some(delivered.clone()), Some(delivered.clone()));
        h.deferred = Some(Held { cause: WakeCause::Finished, look: BranchLook::default() });
        h.unjudged = Some(Unjudged { asked: Some(TurnAsk::Merge), fresh: true });
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, delivered.clone(), false));
        h.looks = 2;
        let back = restarted(&h);
        assert_eq!(back.judged, h.judged);
        assert_eq!(back.told, h.told);
        assert_eq!(back.unjudged, h.unjudged);
        assert_eq!(back.looks, 0, "no probe of the last daemon's is out");
        let stepped = back.stepped.as_ref().expect("the held landing");
        assert_eq!((stepped.cause, stepped.finished), (WakeCause::Merged, true));
        let checkout = Heard { told: Some(checkout("1111111aaaa", "REVIEW")), ..Heard::default() };
        assert_eq!(restarted(&checkout).told.unwrap().merge, None);
        let odd: Told =
            serde_json::from_str(r#"{"tip":"a","merge":"rebasing","ahead":1}"#).unwrap();
        assert_eq!(odd.merge, None);
        // An answer owed through a turn end and a merge step on the same
        // look is still an answer.
        let crown = Unjudged { asked: Some(TurnAsk::Crown(ulid::Ulid::nil())), fresh: false };
        let step = Unjudged { asked: Some(TurnAsk::Merge), fresh: true };
        assert_eq!(step.join(crown), Unjudged { asked: crown.asked, fresh: true });
    }

    /// T-602's own case: the crown heard the delivery, the daemon went
    /// down, the branch merged in the window. The restart's look says the
    /// merge — once: the baseline moves to it, and the flags' reading after
    /// is silent.
    #[test]
    fn a_merge_in_the_restart_window_is_heard_once_after_it() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let mut h = restarted(&heard(Some(delivered.clone()), Some(delivered.clone())));
        assert!(h.awaits_merge(), "the flags' first reading is owed it");
        let landed = branch("aaa", "merged", 0, "REVIEW");
        let cause = verdict(h.judged.as_ref(), &landed, false, false, false)
            .max(merge_verdict(true, &h, &landed));
        assert_eq!(due(cause, None, &IDLE), Due::Wake(WakeCause::Merged));
        let wake = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Merged,
            h.told.replace(landed.clone()),
            Some(landed.clone()),
            Owed { late: h.restored, ..Owed::default() },
        );
        h.judged = Some(landed.clone());
        assert_eq!(wake.changed(), ["merge_state ahead → merged", "after a restart"]);
        assert_eq!(merge_verdict(true, &h, &landed), None, "the flags after it");
        // Through a second restart, still heard.
        assert_eq!(merge_verdict(true, &restarted(&h), &landed), None);
    }

    /// What the crown heard before the restart is silent after it: the
    /// same tip is no delivery, and a merge it was told of is no merge.
    #[test]
    fn a_wake_already_heard_is_silent_after_a_restart() {
        let delivered = branch("aaa", "ahead", 1, "REVIEW");
        let h = restarted(&heard(Some(delivered.clone()), Some(delivered.clone())));
        let cause = verdict(h.judged.as_ref(), &delivered, false, false, false)
            .max(merge_verdict(true, &h, &delivered));
        assert_eq!(due(cause, None, &IDLE), Due::Silent);
        let landed = branch("aaa", "merged", 0, "DONE");
        let h = restarted(&heard(Some(landed.clone()), Some(landed.clone())));
        let cause = verdict(h.judged.as_ref(), &landed, false, false, false)
            .max(merge_verdict(true, &h, &landed));
        assert_eq!(due(cause, None, &IDLE), Due::Silent);
        // A turn that ran across the restart and left the same tip did
        // finish (T-591), and says so once.
        assert_eq!(verdict(h.judged.as_ref(), &landed, false, false, true), FINISHED);
    }

    /// A delivery held for the train (T-554) survives the restart as the
    /// hold it was, not re-judged: the merge after it is the delivery, with
    /// `merged` in its delta, heard once.
    #[test]
    fn a_held_delivery_is_heard_at_its_merge_after_a_restart() {
        let behind = branch("aaa", "needs_rebase", 1, "REVIEW");
        let held = Held { cause: WakeCause::Delivered, look: BranchLook::default() };
        let h = restarted(&Heard { deferred: Some(held), ..heard(Some(behind), None) });
        assert!(h.awaits_merge());
        let landed = branch("bbb", "merged", 0, "DONE");
        let cause = merge_verdict(true, &h, &landed);
        assert_eq!(cause, DELIVERED);
        assert_eq!(due(cause, None, &IDLE), Due::Wake(WakeCause::Delivered));
        let mut wake = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Delivered,
            h.told.clone(),
            Some(landed),
            Owed::default(),
        );
        assert_eq!(wake.changed(), ["merge_state merged", "column DONE"]);
        // Owed across the restart, it says so; a fold keeps saying it.
        wake.late = true;
        wake.fold(WakeCause::Raised, None, None);
        assert_eq!(wake.changed(), ["merge_state merged", "column DONE", "after a restart"]);
        let back: CrownWake = serde_json::from_str(&serde_json::to_string(&wake).unwrap()).unwrap();
        assert_eq!((back.cause, back.late), (WakeCause::Raised, true));
    }

    /// T-744: a forge wake says its edges' words, every one a pass saw
    /// between two crown turns, ranks over a finished turn and under a
    /// merge, and comes back from `crown.json` — T-739's spelling included.
    #[test]
    fn a_forge_wake_says_every_edge_and_round_trips() {
        assert!(WakeCause::Forge > WakeCause::Finished);
        assert!(WakeCause::Merged > WakeCause::Forge);
        assert_eq!(WakeCause::Forge.word(), "forge");
        assert_eq!(WakeCause::Forge.clause(), "has news from the forge");
        let before = branch("aaa", "ahead", 1, "REVIEW");
        let news = |words: &str| Told { forge: Some(words.into()), ..before.clone() };
        assert_eq!(delta(Some(&before), &news("pipeline passed !1")), ["pipeline passed !1"]);
        // An edge is news each time it happens: a second approval after a
        // dismissal is said again.
        let approved = news("review approved !1");
        assert_eq!(delta(Some(&approved), &approved), ["review approved !1"]);
        let mut wake = CrownWake::new(
            ulid::Ulid::nil(),
            WakeCause::Forge,
            Some(before.clone()),
            Some(news("pipeline passed !1")),
            Owed::default(),
        );
        wake.fold(WakeCause::Forge, None, Some(news("review approved !1")));
        assert_eq!(wake.changed(), ["pipeline passed !1, review approved !1"]);
        // A delivery folded over it keeps the forge's words in its delta.
        wake.fold(WakeCause::Delivered, None, Some(branch("bbb", "ahead", 2, "REVIEW")));
        assert_eq!(wake.cause, WakeCause::Delivered);
        assert_eq!(wake.changed(), ["ahead 1 → 2", "pipeline passed !1, review approved !1"]);
        let back: CrownWake = serde_json::from_str(&serde_json::to_string(&wake).unwrap()).unwrap();
        assert_eq!(back.to, wake.to);
        let old: CrownWake = serde_json::from_str(
            r#"{"worker":"00000000000000000000000000","cause":"pipeline",
                "to":{"tip":"a","ahead":1,"column":"REVIEW","pipeline":"failed #4: lint"}}"#,
        )
        .unwrap();
        assert_eq!(old.cause, WakeCause::Forge);
        assert_eq!(old.to.and_then(|t| t.forge).as_deref(), Some("failed #4: lint"));
    }

    /// T-773: a turn's end waits for a forge read only where one may be
    /// owed. No forge: nothing. A look that found nothing pushed: nothing.
    /// A push: held until a read begun after the turn's end lands, or
    /// `FORGE_WAIT_MS` pass. The opening is said first in the line.
    #[test]
    fn a_turn_end_waits_for_its_forge_read_only_where_one_is_owed() {
        use mesimon_core::forge::Remote;
        let t = ulid::Ulid::new();
        // No forge reads this board: the look out holds nothing.
        let mut s = crate::forge::State::default();
        s.remotes.insert("/r".into(), None);
        s.looking.insert(t);
        assert!(!forge_holds(s.awaited(t), 5_000, 5_000), "no forge");
        // A forge: while the look is out, it may find a push.
        let mut s = crate::forge::State::default();
        s.remotes
            .insert("/r".into(), Some(Remote { host: "github.com".into(), path: "o/r".into() }));
        s.looking.insert(t);
        assert!(forge_holds(s.awaited(t), 5_000, 5_000), "the look is out");
        // It found nothing pushed and nothing opened: no read is owed.
        s.looking.remove(&t);
        assert!(!forge_holds(s.awaited(t), 5_000, 5_000), "nothing pushed");
        // It found a push: held until a read begun after the turn's end.
        s.owed.insert(t, 5_000);
        assert!(forge_holds(s.awaited(t), 5_000, 6_000), "pushed");
        s.answer([t].iter(), 4_000);
        assert!(forge_holds(s.awaited(t), 5_000, 6_000), "a read begun before answers nothing");
        assert!(!forge_holds(s.awaited(t), 5_000, 5_000 + FORGE_WAIT_MS), "15 s pass");
        s.answer([t].iter(), 5_100);
        assert!(!forge_holds(s.awaited(t), 5_000, 6_000), "the read landed");
        // The line says the opening, before what the read says of the PR.
        let mut to = Some(branch("aaa", "ahead", 1, "REVIEW"));
        say_first(&mut to, "opened !2".into());
        let mut wake = CrownWake::new(
            t,
            WakeCause::Delivered,
            Some(branch("000", "clean", 0, "IN PROGRESS")),
            to,
            Owed::default(),
        );
        let passed = Told {
            forge: Some("pipeline passed !2".into()),
            ..branch("aaa", "ahead", 1, "REVIEW")
        };
        wake.fold(WakeCause::Forge, None, Some(passed));
        assert_eq!(wake.cause, WakeCause::Delivered);
        assert_eq!(
            wake.changed(),
            [
                "merge_state clean → ahead",
                "ahead 0 → 1",
                "column REVIEW",
                "opened !2, pipeline passed !2"
            ]
        );
    }
}
