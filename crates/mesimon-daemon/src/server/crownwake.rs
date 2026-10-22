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
//! 3. **answered your ask** — the turn that took the crown's `ask_agent`
//!    words ended, whatever it left behind;
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
//! **Whose worker (T-712).** Every event above is heard for a worker THIS
//! crown started (`started_by_crown`), and — behind the board's
//! `crown_watches` — for a ticket the crown watches (`watch_ticket`,
//! `Daemon::crown_watched`): a person's worker the crown was told to wait
//! for. A watched ticket is heard delivered, finished, raised, limited and merged,
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

use super::*;
use serde::{Deserialize, Serialize};

/// Why the words a turn ran on were sent. Stamped on the paste's owed entry
/// (`tag_owed`) and moved to `Daemon::turn_asks` by its ack, so the turn
/// that TOOK the words knows it at its end — not the one that happened to
/// be running when they were pasted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TurnAsk {
    /// The crown's `ask_agent` words, sent by a person's `^y`.
    Crown(ulid::Ulid),
    /// A merge-flow sentence: a rebase ask or the merged notice, by `m` or
    /// by the train.
    Merge,
}

/// The crown's words that went into a running turn and were not acked
/// inside `INFLIGHT_MS` (T-600): a `now` ask, or a person's `^y`, at a
/// worker in a long turn. Claude Code takes them either way, and the mark
/// their ack would have set (`mark_turn`) is kept for the turn that does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LateAsk {
    pub(super) ask: TurnAsk,
    /// Down the mod as a `submit`: held until the running turn ends and
    /// run as a turn of its own, whose `UserPromptSubmit` takes the mark.
    /// A paste is shown to the running turn at its next step, so with no
    /// ack before that turn's end, the turn took it.
    pub(super) by_mod: bool,
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
/// news: every other line about the worker already says its turn ended,
/// and says what it left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WakeCause {
    /// Idle with background tasks for `LINGER_MS` (T-599): the weakest.
    Lingering,
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
}

impl WakeCause {
    /// The clause after the ticket in the sentence.
    fn clause(self) -> &'static str {
        match self {
            WakeCause::Lingering => "has been idle with background tasks",
            WakeCause::Finished => "finished its turn",
            WakeCause::Forge => "has news from the forge",
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered your ask",
            WakeCause::Limited => "hit a usage limit",
            WakeCause::Raised => "raised its hand",
            // Never the plan's words (T-414's rule for a hand's reason):
            // the crown reads them with `get_ticket`.
            WakeCause::Planned => "stops on a plan",
            // Never the question's words (T-414's rule for a hand's
            // reason): the crown reads them with `get_ticket`.
            WakeCause::Asked => "asks a question",
            WakeCause::Unstarted => "could not start",
        }
    }

    /// The feed's word.
    fn word(self) -> &'static str {
        match self {
            WakeCause::Lingering => "lingering",
            WakeCause::Finished => "finished",
            WakeCause::Forge => "forge",
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered",
            WakeCause::Limited => "limited",
            WakeCause::Raised => "raised",
            WakeCause::Planned => "planned",
            WakeCause::Asked => "asked",
            WakeCause::Unstarted => "unstarted",
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
}

/// What rides a wake beside its cause and delta, as `owe_wake` folds it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Owed {
    finished: bool,
    linger: Option<Linger>,
    late: bool,
}

/// A worker idle with background tasks (T-599): how many, and for how long
/// in minutes, when the board noticed. Never the tasks' words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Linger {
    pub(super) tasks: usize,
    pub(super) minutes: u64,
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
        if let (WakeCause::Unstarted, Some(why)) = (self.cause, &self.unstarted) {
            return vec![why.clone()];
        }
        let mut out = match &self.to {
            None => Vec::new(),
            Some(to) if self.cause != WakeCause::Finished => delta(self.from.as_ref(), to),
            Some(to) => {
                let mut out = vec!["nothing new to merge".to_string()];
                out.extend(changes(self.from.as_ref(), to, false));
                out
            }
        };
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
/// it is — one THIS crown started — is `note_crown_wake`'s to judge.
pub(super) fn asks_the_crown(
    mode: CrownMode,
    kind: SessionKind,
    change: &Change,
) -> Option<WakeCause> {
    if change.from == change.to || !mode.takes_stop(kind, &change.to) {
        return None;
    }
    match change.to {
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
/// the worker. Only a worker THIS crown started (`started`) — the crown's
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
            (Some(a @ TurnAsk::Crown(_)), _) | (_, Some(a @ TurnAsk::Crown(_))) => Some(a),
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
    /// crown do. True the first time.
    fn hold_for_step(&mut self, worker: ulid::Ulid, cause: WakeCause, now: Told) -> bool {
        self.judged = Some(now.clone());
        self.deferred = None;
        let from = self.told.replace(now.clone());
        if let Some(w) = self.stepped.as_mut() {
            w.fold(cause, from, Some(now));
            return false;
        }
        self.stepped = Some(CrownWake {
            worker,
            cause,
            from,
            to: Some(now),
            finished: true,
            linger: None,
            late: self.restored,
            unstarted: None,
        });
        true
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
    found: Found,
}

impl Look {
    /// Run the look: the same flags sample the tick takes, for one ticket's
    /// legs, or one `rev-parse` of the checkout's HEAD.
    fn run(self, worker: ulid::Ulid) -> Found {
        match self {
            Look::Branch { binding, repo, base, mut queries } => {
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
                    .legs(&repo, &base.unwrap_or_default())
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
    /// The worker's claude was started by THIS crown, `crown` still wearing
    /// it: a worker left over from an earlier crown wakes nobody. The
    /// board's own predicate, which the person's banners read too (T-678).
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

    /// The crown is owed the worker's news: it started the worker, it
    /// watches the ticket (T-712), or the ticket has no agent and carries
    /// a pull request (`unattended_pr`, T-890). The one claim every wake
    /// but an answer makes — `note_crown_wake` narrows a watch to what a
    /// person's agent gives the crown to act on.
    fn crown_hears(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.started_by_crown(worker, crown)
            || self.watched_by_crown(worker, crown)
            || self.unattended_pr(worker, crown)
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
            if o.asked.is_none() || matches!(ask, TurnAsk::Crown(_)) {
                o.asked = Some(ask);
            }
        }
    }

    /// Words that carried `ask` went in and their ack did not come inside
    /// the window (T-600): the mark waits for the turn that takes them.
    pub(super) fn ask_unacked(&mut self, ticket: ulid::Ulid, ask: TurnAsk, by_mod: bool) {
        self.late_asks.insert(ticket, LateAsk { ask, by_mod });
    }

    /// A prompt reached `ticket`'s agent with nothing owed: the words that
    /// outlived their window, if any, are what this turn runs on.
    pub(super) fn late_ask_acked(&mut self, ticket: ulid::Ulid) {
        if let Some(late) = self.late_asks.remove(&ticket) {
            self.mark_turn(ticket, late.ask);
        }
    }

    /// The owed words reached the agent: the turn now running is theirs.
    pub(super) fn mark_turn(&mut self, ticket: ulid::Ulid, ask: TurnAsk) {
        match ask {
            TurnAsk::Crown(_) => {
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
    /// only for a worker this crown started or a turn that took its ask.
    pub(super) fn turn_ended(&mut self, worker: ulid::Ulid, end_turn: bool) {
        // A worker the restart has yet to look at (T-602): this end is that
        // look when its state is committed — a hook's, or the close in the
        // gap T-663 proves at Medium. The tail's guess at Low is no end at
        // all (T-756): it read a worker mid-turn as idle, its branch ahead
        // as a delivery, and woke the crown a second after the restart. The
        // look, the turn and its asks wait for an end that is one.
        if self.crown_recheck.contains_key(&worker)
            && self.board.live_agent(worker).is_some_and(guessed)
        {
            return;
        }
        let pasted = self.late_asks.get(&worker).is_some_and(|l| !l.by_mod);
        let late = if pasted { self.late_asks.remove(&worker).map(|l| l.ask) } else { None };
        let asked = self.turn_asks.remove(&worker).or(late);
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
            let asked = self.turn_asks.remove(&worker);
            let fresh = self.turns_open.remove(&worker);
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
        Pending {
            train: look.is_some_and(|look| self.train_takes(worker, look)),
            merge_step: owed(true)
                || (working && self.turn_asks.get(&worker) == Some(&TurnAsk::Merge)),
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
        let answered = matches!(
            why,
            ProbeWhy::Turn { asked: Some(TurnAsk::Crown(c)), .. }
                | ProbeWhy::Restart { asked: Some(TurnAsk::Crown(c)), .. } if c == crown
        );
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
                base: self.base_branch.clone(),
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
        std::thread::spawn(move || {
            let found = look.run(worker);
            let _ = tx.send(Msg::TurnProbed(TurnProbe { worker, crown, why, found }));
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
        self.judge_probe(p);
        // Looked at since the restart (T-602): what is owed from here is
        // current, and every turn it was asked for is judged.
        if let Some(h) = self.crown_heard.get_mut(&worker) {
            h.restored = false;
            if h.looks == 0 {
                h.unjudged = None;
            }
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
        let answered = matches!(asked, Some(TurnAsk::Crown(c)) if c == p.crown);
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
                        self.owe_turn_end(p.crown, p.worker, cause, None, None);
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
                    self.hold_for_step(p.worker, cause, now);
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
            self.owe_turn_end(p.crown, p.worker, cause, from, Some(now));
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
    ) {
        let heard = self.crown_heard.entry(worker).or_default();
        if let Some(words) = heard.opening.take() {
            say_first(&mut to, words);
        }
        if !self.forge.awaited(worker) {
            self.owe_crown_wake(crown, worker, cause, from, to, false);
            return;
        }
        let heard = self.crown_heard.entry(worker).or_default();
        if let Some(f) = heard.forging.as_mut() {
            f.wake.fold(cause, from, to);
            return;
        }
        let late = heard.restored;
        let wake = CrownWake {
            worker,
            cause,
            from,
            to,
            finished: false,
            linger: None,
            late,
            unstarted: None,
        };
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
            let Some(column) = self.board.ticket(worker).map(|t| t.column.clone()) else {
                continue;
            };
            // The column the read's arrival left the card in.
            let mut to = wake.to;
            h.land(&mut to, column);
            let owed = Owed { finished: wake.finished, linger: None, late: wake.late };
            self.enter_wake(crown, worker, wake.cause, wake.from, to, owed);
            woke = true;
        }
        woke
    }

    /// A landing that waits for the merge step in flight on its worker
    /// (T-596), said in the feed the first time. The claim on the worker is
    /// the caller's to have judged.
    fn hold_for_step(&mut self, worker: ulid::Ulid, cause: WakeCause, now: Told) {
        if self.crown_heard.entry(worker).or_default().hold_for_step(worker, cause, now) {
            self.feed.board("automation", "crown_wake_deferred:merge_step", Some(worker));
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
        let reading = mesimon_core::train::Reading {
            merge_into: self.board.merge_into,
            reach: self.board.column(&t.column).map(|c| c.settings.train).unwrap_or_default(),
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
                Due::Step(cause) => self.hold_for_step(worker, cause, now),
                Due::Hold(_) | Due::Silent => {}
            }
        }
        woke
    }

    /// Landings held for a merge step (T-596) whose step is over: the words
    /// were taken and their turn ended, or they never reached the agent (a
    /// paste given up on, a pane that died), on the tick. Each wakes the
    /// crown as it was held, saying the worker finished its turn, with the
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
            let column = self.board.ticket(worker).map(|t| t.column.clone());
            let Some(heard) = self.crown_heard.get_mut(&worker) else { continue };
            let Some(wake) = heard.step_over(column) else { continue };
            let held = Owed { finished: true, linger: None, late: wake.late };
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
            let cause = merge_verdict(started, &heard, &now);
            match due(cause, None, &self.pending_on(worker, None)) {
                Due::Step(cause) => self.hold_for_step(worker, cause, now),
                Due::Wake(cause) => {
                    let heard = self.crown_heard.entry(worker).or_default();
                    heard.judged = Some(now.clone());
                    heard.deferred = None;
                    let from = heard.told.replace(now.clone());
                    self.owe_crown_wake(crown, worker, cause, from, Some(now), false);
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
    /// which the crown asked for — only for an agent THIS crown started, or
    /// a ticket it watches (T-712), whose delivery, finished turn, hand,
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

    /// `note_crown_wake`'s guards: never the crown's own ticket, and, an
    /// answer aside, a worker it started, or a watched ticket's news.
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
        );
        watched_news && self.watched_by_crown(worker, crown)
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
    fn owe_wake(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
        owed: Owed,
    ) {
        self.enter_wake(crown, worker, cause, from, to, owed);
        self.drain_crown_wakes();
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
        let Owed { finished, linger, late } = owed;
        let late = late || self.crown_heard.get(&worker).is_some_and(|h| h.restored);
        let landed =
            cause == WakeCause::Merged || to.as_ref().is_some_and(|t| t.merge == Some("merged"));
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            w.fold(cause, from, to);
            w.finished |= finished;
            w.linger = w.linger.or(linger);
            w.late |= late;
        } else {
            self.crown_wakes.push(CrownWake {
                worker,
                cause,
                from,
                to,
                finished,
                linger,
                late,
                unstarted: None,
            });
        }
        // A watch ends at the merge (T-712): the wait it was for is over,
        // and the line that says so is its last.
        if landed && self.crown_watched.remove(&worker) {
            self.hold_unwatched_sends(worker);
            self.feed.board("automation", "watch_ended", Some(worker));
        }
        self.feed.crown_wake(crown, worker, cause.word());
        self.crown_touched(worker, crown, "woke");
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
        self.enter_wake(crown, worker, WakeCause::Unstarted, None, None, Owed::default());
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            w.unstarted = Some(why);
        }
        self.drain_crown_wakes();
        self.broadcast();
    }

    /// Workers the crown started that have sat idle at their composer with
    /// background tasks for `LINGER_MS` with no foreground turn (T-599), on
    /// the tick: the crown is woken once per stretch, `has been idle with 3
    /// background tasks for 30 min`, and nothing else happens — no merge,
    /// no park, no kill. A stretch ends only with a foreground turn
    /// (`apply_change` forgets the record on `Running`), so a task the
    /// agent arms or ends without a turn does not make a second wake.
    pub(super) fn hear_lingering(&mut self) -> bool {
        let sessions = &self.board.sessions;
        let known = |id: &uuid::Uuid| sessions.iter().any(|s| s.id == *id);
        // Asked before it is written: a `&mut` marks the ledger (T-839).
        if !self.lingered.iter().all(known) {
            self.lingered.retain(|id| known(id));
        }
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let now = now_ms();
        let after = super::linger_ms();
        let due: Vec<(uuid::Uuid, ulid::Ulid, Linger)> = self
            .board
            .sessions
            .iter()
            .filter(|s| {
                s.kind == SessionKind::Claude
                    && s.ticket != crown
                    && s.started_by == Some(crown)
                    && s.state == (SessionState::Idle { stop_reason: StopReason::Background })
                    && !self.lingered.contains(&s.id)
            })
            .filter_map(|s| {
                let age = now.saturating_sub(s.state_changed_at?);
                (age >= after).then_some((
                    s.id,
                    s.ticket,
                    Linger { tasks: s.background_tasks.count(), minutes: age / 60_000 },
                ))
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
    /// tick then finds it told and folds into it.
    pub(super) fn hear_forge(&mut self, news: Vec<(ulid::Ulid, String)>) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let mut woke = false;
        for (worker, words) in news {
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
                Due::Step(cause) => self.hold_for_step(worker, cause, now),
                Due::Wake(cause) => {
                    let heard = self.crown_heard.entry(worker).or_default();
                    if landing {
                        heard.judged = Some(now.clone());
                        heard.deferred = None;
                    }
                    let from = heard.told.replace(now.clone());
                    self.owe_crown_wake(crown, worker, cause, from, Some(now), false);
                    woke = true;
                }
                Due::Hold(_) | Due::Silent => {}
            }
        }
        woke
    }

    /// The wakes just sent told the crown each card's column as it stands
    /// (`CrownWake::sent_at`), so that is what the next wake's delta runs
    /// from (T-785): a column said is not said again.
    pub(super) fn told_columns(&mut self) {
        for w in self.crown_wakes.iter().filter(|w| w.to.is_some()) {
            let Some(column) = self.board.ticket(w.worker).map(|t| t.column.clone()) else {
                continue;
            };
            if let Some(told) = self.crown_heard.get_mut(&w.worker).and_then(|h| h.told.as_mut()) {
                told.column = column;
            }
        }
    }

    /// The crown left, or another ticket took it: whatever it was owed goes
    /// with it, said once in the feed, and what it was told goes too — a
    /// new crown has heard nothing.
    pub(super) fn drop_crown_wakes(&mut self) {
        self.crown_heard.clear();
        self.crown_landed.clear();
        self.crown_recheck.clear();
        self.crown_watched.clear();
        if self.crown_wakes.is_empty() {
            return;
        }
        self.crown_wakes.clear();
        self.feed.board("automation", "crown_wake_dropped", self.board.crown);
    }

    /// The hands the sentence just told the crown of come down (T-861):
    /// the crown holds the worker's question from here — it answers the
    /// worker, which lowers the hand anyway, or raises its own — and a `!`
    /// that stood on past that was two cards asking a person one thing.
    /// Only a wake whose cause IS the hand: a question or a plan folded
    /// over it (`WakeCause`'s order) comes down with its dialog (T-611),
    /// and a hand folded over a delivery is still the hand.
    pub(super) fn lower_told_hands(&mut self) {
        let told: Vec<ulid::Ulid> = self
            .crown_wakes
            .iter()
            .filter(|w| w.cause == WakeCause::Raised)
            .map(|w| w.worker)
            .collect();
        for worker in told {
            self.lower_hand_on(worker);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut w = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Lingering,
            from: None,
            to: None,
            finished: false,
            linger: Some(l(3, 30)),
            late: false,
            unstarted: None,
        };
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
        let wake = |from: Option<Told>, to: Option<Told>| CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Finished,
            from,
            to,
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
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

    #[test]
    fn coalesced_causes_keep_the_strongest() {
        assert!(WakeCause::Asked > WakeCause::Planned);
        assert!(WakeCause::Planned > WakeCause::Raised);
        assert!(WakeCause::Raised > WakeCause::Limited);
        assert!(WakeCause::Limited > WakeCause::Answered);
        assert!(WakeCause::Answered > WakeCause::Delivered);
        assert!(WakeCause::Delivered > WakeCause::Merged);
        assert!(WakeCause::Merged > WakeCause::Finished);
    }

    /// T-785, lab run 3's A: a forge wake owed with the card in PROD
    /// waits for the crown's turn, the crown moves the card to DONE, and
    /// the line that goes out says DONE, as `get_ticket` does then. The
    /// edge stays as heard; a card sent where the crown last heard it says
    /// no column. `forge_e2e` pins `told_columns`, the other half.
    #[test]
    fn a_wake_says_the_column_it_is_sent_from() {
        let before = branch("aaa", "merged", 0, "REVIEW");
        let mut owed = branch("aaa", "merged", 0, "PROD");
        owed.forge = Some("deployed to production !11".into());
        let wake = CrownWake {
            worker: ulid::Ulid::new(),
            cause: WakeCause::Forge,
            from: Some(before.clone()),
            to: Some(owed),
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
        let mut w = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Delivered,
            from: None,
            to: Some(delivered.clone()),
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed.clone()));
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
        let wake = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Merged,
            from: Some(delivered),
            to: Some(landed),
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Delivered, landed.clone()));
        assert!(h.deferred.is_none(), "the train's part is done");
        assert!(h.awaits_merge());
        assert_eq!(merge_verdict(true, &h, &landed), None, "heard once");
        let wake = h.step_over(Some("DONE".into())).expect("the held delivery");
        assert_eq!(wake.clause(), "delivered and finished its turn");
        assert_eq!(wake.changed(), ["merge_state merged", "column DONE"]);
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, landed));
        assert!(h.step_over(None).is_none(), "the ticket is gone");
        assert!(h.stepped.is_none());
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
        assert!(h.hold_for_step(ulid::Ulid::nil(), WakeCause::Merged, delivered.clone()));
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
        let wake = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Merged,
            from: h.told.replace(landed.clone()),
            to: Some(landed.clone()),
            finished: false,
            linger: None,
            late: h.restored,
            unstarted: None,
        };
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
        let mut wake = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Delivered,
            from: h.told.clone(),
            to: Some(landed),
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
        let mut wake = CrownWake {
            worker: ulid::Ulid::nil(),
            cause: WakeCause::Forge,
            from: Some(before.clone()),
            to: Some(news("pipeline passed !1")),
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
        let mut wake = CrownWake {
            worker: t,
            cause: WakeCause::Delivered,
            from: Some(branch("000", "clean", 0, "IN PROGRESS")),
            to,
            finished: false,
            linger: None,
            late: false,
            unstarted: None,
        };
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
