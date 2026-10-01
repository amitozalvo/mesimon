//! What wakes the crown (T-414, narrowed by T-469, T-527): four ticket
//! events, not every `Stop`.
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
//! 2. **answered your ask** — the turn that took the crown's `ask_agent`
//!    words ended, whatever it left behind;
//! 3. **raised its hand** — `raise_hand`, as before;
//! 4. **merged** — a worker it started has its branch read `merged` by the
//!    worktree flags (`hear_merges`), however it got there: `m`, the train,
//!    or a `git merge` in a terminal. A shared-checkout worker has no branch,
//!    so nothing to read; its commits are on the base the moment they exist.
//!
//! A turn that took mesimon's own merge-flow words (`m`'s or the train's
//! rebase ask, the merged notice) is a merge step: the person already knows,
//! and the crown learns on its next `get_ticket` — or from the merge itself,
//! which the flags see and the words do not. Every wake line carries what
//! changed since the crown last heard (`merge_state needs_rebase → ahead`,
//! `column REVIEW`), so the obvious costs no tool call.
//!
//! A delivery the armed merge train will take is held (T-554): the crown
//! hears it once, at the merge, as the delivery with `merged` in its delta
//! — or the moment the train will not take it after all (`hear_deferred`).

use super::*;

/// Why the words a turn ran on were sent. Stamped on the paste's owed entry
/// (`tag_owed`) and moved to `Daemon::turn_asks` by its ack, so the turn
/// that TOOK the words knows it at its end — not the one that happened to
/// be running when they were pasted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnAsk {
    /// The crown's `ask_agent` words, sent by a person's `^y`.
    Crown(ulid::Ulid),
    /// A merge-flow sentence: a rebase ask or the merged notice, by `m` or
    /// by the train.
    Merge,
}

/// A worker's work at one turn's end: the baseline the next turn is judged
/// against and the two sides of a wake's delta.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Told {
    /// The branch tip on a worktree (legs joined), HEAD on a checkout.
    pub(super) tip: String,
    /// `merge_state`'s word; `None` on a checkout, which has no branch.
    pub(super) merge: Option<&'static str>,
    /// Commits ahead of the base; 0 on a checkout.
    pub(super) ahead: u32,
    pub(super) column: String,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BranchLook {
    pub(super) tip: String,
    pub(super) base_tip: String,
    pub(super) ahead: u32,
    pub(super) merged: bool,
    pub(super) needs_rebase: bool,
    pub(super) conflict: bool,
}

impl BranchLook {
    fn told(&self, column: String) -> Told {
        Told {
            tip: self.tip.clone(),
            merge: Some(worktree::merge_word(self.merged, self.needs_rebase, self.ahead)),
            ahead: self.ahead,
            column,
        }
    }
}

/// What the crown is woken for, in the precedence two events on one worker
/// coalesce by: a hand outranks an answer, an answer a delivery, and a
/// delivery a merge — a merge folded into any other line is said in its
/// delta (`merge_state merged`), so a delivery and its merge between two
/// crown turns are one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WakeCause {
    Merged,
    Delivered,
    Answered,
    Raised,
}

impl WakeCause {
    /// The clause after the ticket in the sentence.
    fn clause(self) -> &'static str {
        match self {
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered your ask",
            WakeCause::Raised => "raised its hand",
        }
    }

    /// The feed's word.
    fn word(self) -> &'static str {
        match self {
            WakeCause::Merged => "merged",
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered",
            WakeCause::Raised => "raised",
        }
    }
}

/// One thing the crown has yet to hear about: keyed by worker, so a second
/// event before delivery raises the cause (`WakeCause`'s order) and moves
/// `to` rather than adding a clause. `from` stays what the crown last heard.
pub(super) struct CrownWake {
    pub(super) worker: ulid::Ulid,
    cause: WakeCause,
    from: Option<Told>,
    to: Option<Told>,
}

impl CrownWake {
    /// A second event on the same worker before delivery: the stronger
    /// cause, the latest work, and `from` kept unless nothing was described
    /// yet — the delta runs from what the crown last heard to now.
    fn fold(&mut self, cause: WakeCause, from: Option<Told>, to: Option<Told>) {
        self.cause = self.cause.max(cause);
        if self.to.is_none() {
            self.from = from;
        }
        if to.is_some() {
            self.to = to;
        }
    }
}

/// Whether a turn's end wakes the crown, and why. `before` is the worker's
/// last baseline (`None` after a restart, or before the first look), `now`
/// its work as the turn left it. An answer always wakes; a merge step never
/// does; otherwise only something mergeable at a tip not seen before —
/// a second idle with nothing new is silent.
pub(super) fn verdict(
    before: Option<&Told>,
    now: &Told,
    answered: bool,
    merge_step: bool,
) -> Option<WakeCause> {
    if answered {
        return Some(WakeCause::Answered);
    }
    if merge_step || !now.mergeable() {
        return None;
    }
    before.is_none_or(|b| b.tip != now.tip).then_some(WakeCause::Delivered)
}

/// What a turn's end does once the merge train is counted (T-554).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    Wake(WakeCause),
    /// Held for the train: no wake now, and `told` stays where it was.
    Hold,
    Silent,
}

/// `verdict`'s answer with the merge train in it (T-554). A delivery the
/// train will land is held, and so is one held before whose new turn left
/// it still the train's; a held delivery the train will not take after all
/// comes due as the delivery it was. `takes` asks the train about the
/// branch as this turn left it, and is asked only then: an answer, a hand
/// and a silent turn with nothing held are `verdict`'s alone.
pub(super) fn with_train(
    cause: Option<WakeCause>,
    held: bool,
    takes: impl FnOnce() -> bool,
) -> Due {
    match cause {
        Some(WakeCause::Delivered) => {}
        Some(other) => return Due::Wake(other),
        None if held => {}
        None => return Due::Silent,
    }
    if takes() {
        Due::Hold
    } else {
        Due::Wake(WakeCause::Delivered)
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
/// keeps its count, and neither is news beside the word.
pub(super) fn delta(before: Option<&Told>, now: &Told) -> Vec<String> {
    let mut out = Vec::new();
    match (before.and_then(|b| b.merge), now.merge) {
        (Some(was), Some(is)) if was != is => out.push(format!("merge_state {was} → {is}")),
        (None, Some(is)) => out.push(format!("merge_state {is}")),
        (Some(_), Some("merged")) if before.is_some_and(|b| b.tip != now.tip) => {
            out.push("merge_state merged".into());
        }
        _ => {}
    }
    if let Some(b) = before.filter(|b| b.merge.is_some() && now.merge.is_some()) {
        if b.ahead != now.ahead && now.merge != Some("merged") {
            out.push(format!("ahead {} → {}", b.ahead, now.ahead));
        }
    }
    if now.merge.is_none() && before.is_none_or(|b| b.tip != now.tip) && !now.tip.is_empty() {
        out.push(format!("commit {}", now.tip.get(..7).unwrap_or(&now.tip)));
    }
    if before.is_none_or(|b| b.column != now.column) {
        out.push(format!("column {}", mesimon_core::text::scrub_text(&now.column)));
    }
    out
}

/// One worker's two baselines. `judged` is the work as the last looked-at
/// turn left it: a wake's novelty is judged against it, so a merge step's
/// rebased tip is not news on the next idle. `told` is the work as the last
/// wake about it described it: a wake's delta runs from there, so a column
/// a silent turn moved is still said.
#[derive(Clone, Debug, Default)]
pub(super) struct Heard {
    judged: Option<Told>,
    told: Option<Told>,
    /// A delivery held for the merge train (T-554): the branch as the turn
    /// that delivered it, or the latest turn since, left it. `told` does
    /// not move while it is held, so the merge reads as a tip the crown
    /// never heard of.
    deferred: Option<BranchLook>,
    /// Turn probes out for this worker. While one is, the train is not
    /// judged on what came before it: the turn that just ended may have
    /// moved the branch.
    looks: u32,
}

impl Heard {
    /// The crown is owed this worker's merge: a wake described its work,
    /// or a delivery is held for the train.
    pub(super) fn awaits_merge(&self) -> bool {
        self.told.is_some() || self.deferred.is_some()
    }
}

/// What a probe was for.
#[derive(Clone, Copy, Debug)]
pub(super) enum ProbeWhy {
    /// A worker's turn ended, having taken these words.
    Turn(Option<TurnAsk>),
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
                let Some(base) = base.or_else(|| worktree::default_branch(&repo).ok()) else {
                    return Found::Unknown;
                };
                for q in &mut queries {
                    if q.base.is_empty() {
                        q.base = base.clone();
                    }
                }
                let samples = worktree::compute_repo_flags(&queries);
                let legs: Vec<worktree::RepoFlags> = binding
                    .legs(&repo, &base)
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
    /// The worker's claude was started by THIS crown: a worker left over
    /// from an earlier crown wakes nobody.
    fn started_by_crown(&self, worker: ulid::Ulid, crown: ulid::Ulid) -> bool {
        self.board
            .sessions
            .iter()
            .any(|s| s.ticket == worker && s.holds_agent_seat() && s.started_by == Some(crown))
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
    /// left. Only while a crown is worn, never for the crown's own ticket,
    /// and only for a worker this crown started or a turn that took its
    /// ask.
    pub(super) fn turn_ended(&mut self, worker: ulid::Ulid, end_turn: bool) {
        let asked = self.turn_asks.remove(&worker);
        if end_turn {
            self.probe_turn(worker, ProbeWhy::Turn(asked));
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
        let answered = matches!(why, ProbeWhy::Turn(Some(TurnAsk::Crown(c))) if c == crown);
        if !answered && !self.started_by_crown(worker, crown) {
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
        self.crown_heard.entry(worker).or_default().looks += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let found = look.run(worker);
            let _ = tx.send(Msg::TurnProbed(TurnProbe { worker, crown, why, found }));
        });
    }

    /// A probe landed: judge it against the worker's baseline, move the
    /// baseline, and owe the crown a wake when the verdict says so. A
    /// delivery the merge train will take is held instead (T-554), and one
    /// already held is judged again on what this turn left: a rebase the
    /// train asked for and the agent could not finish comes due here.
    pub(super) fn on_turn_probed(&mut self, p: TurnProbe) {
        if self.board.crown_holder().map(|t| t.id) != Some(p.crown) {
            return;
        }
        if let Some(h) = self.crown_heard.get_mut(&p.worker) {
            h.looks = h.looks.saturating_sub(1);
        }
        let Some(column) = self.board.ticket(p.worker).map(|t| t.column.clone()) else {
            return;
        };
        let (now, branch) = match p.found {
            Found::Branch(look) => (look.told(column), Some(look)),
            Found::Checkout { head } => (Told { tip: head, merge: None, ahead: 0, column }, None),
            Found::Unknown => {
                // Git could not say what the turn left; an answer is still
                // an answer.
                if matches!(p.why, ProbeWhy::Turn(Some(TurnAsk::Crown(c))) if c == p.crown) {
                    self.note_crown_wake(p.worker, WakeCause::Answered, None, None);
                }
                return;
            }
        };
        let heard = self.crown_heard.entry(p.worker).or_default();
        let asked = match p.why {
            ProbeWhy::Baseline => {
                heard.judged.get_or_insert(now);
                return;
            }
            ProbeWhy::Turn(asked) => asked,
        };
        let answered = matches!(asked, Some(TurnAsk::Crown(c)) if c == p.crown);
        let merge_step = asked == Some(TurnAsk::Merge);
        let before = heard.judged.replace(now.clone());
        let held = heard.deferred.is_some();
        let cause = verdict(before.as_ref(), &now, answered, merge_step);
        // A checkout has no branch for the train: its delivery wakes.
        let takes = || branch.as_ref().is_some_and(|look| self.train_takes(p.worker, look));
        let cause = match with_train(cause, held, takes) {
            Due::Wake(cause) => cause,
            Due::Silent => return,
            Due::Hold => {
                if !held {
                    self.feed.board("automation", "crown_wake_deferred", Some(p.worker));
                }
                self.crown_heard.entry(p.worker).or_default().deferred = branch;
                return;
            }
        };
        let heard = self.crown_heard.entry(p.worker).or_default();
        heard.deferred = None;
        let from = heard.told.replace(now.clone());
        if held {
            // Held, so the claim was judged when it was: the seat may have
            // gone since.
            self.owe_crown_wake(p.crown, p.worker, cause, from, Some(now));
        } else {
            self.note_crown_wake(p.worker, cause, from, Some(now));
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
        if !self.train.is_armed() || self.worktrees_barred || self.base_branch.is_none() {
            return false;
        }
        let Some(t) = self.board.ticket(worker).filter(|t| !t.is_archived()) else {
            return false;
        };
        let Some(binding) = self.worktrees.get(&worker) else { return false };
        if self.working(None).contains(&worker) {
            return true;
        }
        let flags = mesimon_core::train::WtFlags {
            attached: binding.status == BindingStatus::Attached,
            ahead: look.ahead,
            merged: look.merged,
            needs_rebase: look.needs_rebase,
            conflict: look.conflict,
        };
        let asked = self.train.asked_tips();
        let reading = mesimon_core::train::Reading {
            reach: self.board.column(&t.column).map(|c| c.settings.train).unwrap_or_default(),
            flags: &flags,
            seat: mesimon_core::train::seat(&self.board, worker),
            base_tip: &look.base_tip,
            asked: asked.get(&worker).map(String::as_str),
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
    fn sampled(&self, worker: ulid::Ulid) -> Option<BranchLook> {
        let branch = &self.worktrees.get(&worker)?.branch;
        Some(BranchLook {
            tip: self.wt_tip.get(&worker)?.clone(),
            base_tip: self.base_tip_of(worker).to_string(),
            ahead: self.wt_ahead.get(&worker).copied().unwrap_or(0),
            merged: self.wt_merged.get(&worker).copied().unwrap_or(false),
            needs_rebase: self.wt_needs_rebase.get(&worker).copied().unwrap_or(false),
            conflict: self.wt_conflicts.contains(branch),
        })
    }

    /// Deliveries held for the merge train that it will not take after all
    /// (T-554), on the tick: the train was disarmed, the merge was refused,
    /// the rebase it asked for left the branch behind the same base tip,
    /// the fuse blew, a person took the ticket off the train or out of its
    /// columns, or parked its agent. Each wakes the crown as the delivery
    /// it was, its delta running to now. A held branch that reads merged
    /// here was missed by `hear_merges` and is said the same way. Judged on
    /// the train's own sample once that has reached the held tip — before
    /// then the sample predates the turn and the turn's look is the truth —
    /// and not at all while a probe is out. A ticket gone from the board
    /// takes its held delivery with it.
    pub(super) fn hear_deferred(&mut self) -> bool {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let held: Vec<(ulid::Ulid, BranchLook)> = self
            .crown_heard
            .iter()
            .filter(|(_, h)| h.looks == 0)
            .filter_map(|(w, h)| Some((*w, h.deferred.clone()?)))
            .collect();
        let mut woke = false;
        for (worker, look) in held {
            let Some(column) =
                self.board.ticket(worker).filter(|t| !t.is_archived()).map(|t| t.column.clone())
            else {
                if let Some(h) = self.crown_heard.get_mut(&worker) {
                    h.deferred = None;
                }
                continue;
            };
            let seen = self.sampled(worker).filter(|s| s.tip == look.tip).unwrap_or(look);
            if self.train_takes(worker, &seen) {
                continue;
            }
            let now = seen.told(column);
            let heard = self.crown_heard.entry(worker).or_default();
            heard.deferred = None;
            let from = heard.told.replace(now.clone());
            self.owe_crown_wake(crown, worker, WakeCause::Delivered, from, Some(now));
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
    /// that read the branch just before it is not a second delivery.
    pub(super) fn hear_merges(&mut self) -> bool {
        if self.crown_landed.is_empty() {
            return false;
        }
        let landed = std::mem::take(&mut self.crown_landed);
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return false };
        let mut woke = false;
        for worker in landed {
            if self.wt_merged.get(&worker) != Some(&true) {
                continue;
            }
            let Some(column) = self.board.ticket(worker).map(|t| t.column.clone()) else {
                continue;
            };
            let now = Told {
                tip: self.wt_tip.get(&worker).cloned().unwrap_or_default(),
                merge: Some("merged"),
                ahead: self.wt_ahead.get(&worker).copied().unwrap_or(0),
                column,
            };
            let heard = self.crown_heard.get(&worker).cloned().unwrap_or_default();
            // A delivery held for the train is this crown's to hear (T-554),
            // whatever became of the seat since.
            let started = worker != crown
                && (heard.deferred.is_some() || self.started_by_crown(worker, crown));
            let Some(cause) = merge_verdict(started, &heard, &now) else { continue };
            let heard = self.crown_heard.entry(worker).or_default();
            heard.judged = Some(now.clone());
            heard.deferred = None;
            let from = heard.told.replace(now.clone());
            self.owe_crown_wake(crown, worker, cause, from, Some(now));
            woke = true;
        }
        woke
    }

    /// Record that the crown is owed a wake about `worker`, and try to
    /// deliver it now. Only while the crown is worn, and — an answer aside,
    /// which the crown asked for — only for an agent THIS crown started.
    /// The feed line names both tickets and the cause, never the sentence;
    /// the crown's card lights `woke` the way it lights for the crown's own
    /// touches.
    pub(super) fn note_crown_wake(
        &mut self,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
    ) {
        let Some(crown) = self.board.crown_holder().map(|t| t.id) else { return };
        if worker == crown {
            return;
        }
        if cause != WakeCause::Answered && !self.started_by_crown(worker, crown) {
            return;
        }
        self.owe_crown_wake(crown, worker, cause, from, to);
    }

    /// `note_crown_wake` past its guards: for a caller that judged the
    /// crown's claim on the worker itself.
    fn owe_crown_wake(
        &mut self,
        crown: ulid::Ulid,
        worker: ulid::Ulid,
        cause: WakeCause,
        from: Option<Told>,
        to: Option<Told>,
    ) {
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            w.fold(cause, from, to);
        } else {
            self.crown_wakes.push(CrownWake { worker, cause, from, to });
        }
        self.feed.crown_wake(crown, worker, cause.word());
        self.crown_touched(worker, crown, "woke");
        self.drain_crown_wakes();
        self.broadcast();
    }

    /// The crown left, or another ticket took it: whatever it was owed goes
    /// with it, said once in the feed, and what it was told goes too — a
    /// new crown has heard nothing.
    pub(super) fn drop_crown_wakes(&mut self) {
        self.crown_heard.clear();
        self.crown_landed.clear();
        if self.crown_wakes.is_empty() {
            return;
        }
        self.crown_wakes.clear();
        self.feed.board("automation", "crown_wake_dropped", self.board.crown);
    }

    /// The sentence the crown receives: the board's `crown_wake` template
    /// over every wake owed, in the order they happened, each with what
    /// changed in brackets. A worker's title and a column's name are user
    /// text headed for another process, so they cross `scrub_text`. A worker
    /// no longer on the board contributes nothing. A hand's reason never
    /// rides it (T-414): the crown reads it through `get_ticket`.
    pub(super) fn crown_wake_text(&self) -> String {
        let mut events: Vec<String> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        for w in &self.crown_wakes {
            let Some(t) = self.board.ticket(w.worker) else { continue };
            let title = mesimon_core::text::scrub_text(&t.title);
            let mut event = format!("{} \"{title}\" {}", t.short_key, w.cause.clause());
            let changed = w.to.as_ref().map(|to| delta(w.from.as_ref(), to)).unwrap_or_default();
            if !changed.is_empty() {
                event.push_str(&format!(" ({})", changed.join(", ")));
            }
            events.push(event);
            keys.push(t.short_key.clone());
        }
        self.board.prompts.render(
            mesimon_core::prompts::AgentPrompt::CrownWake,
            &[("events", &events.join("; ")), ("keys", &keys.join(", "))],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(tip: &str, merge: &'static str, ahead: u32, column: &str) -> Told {
        Told { tip: tip.into(), merge: Some(merge), ahead, column: column.into() }
    }

    fn checkout(head: &str, column: &str) -> Told {
        Told { tip: head.into(), merge: None, ahead: 0, column: column.into() }
    }

    /// The ticket's own case: once per delivery. A second idle at the same
    /// tip is silent, and so is an idle with nothing to merge.
    #[test]
    fn a_second_idle_with_nothing_new_is_silent() {
        let first = branch("aaa", "ahead", 1, "REVIEW");
        assert_eq!(verdict(None, &first, false, false), Some(WakeCause::Delivered));
        assert_eq!(verdict(Some(&first), &first, false, false), None, "same tip");
        let fresh = branch("base", "clean", 0, "IN PROGRESS");
        assert_eq!(verdict(None, &fresh, false, false), None, "nothing to merge");
        // A turn's end at a merged branch is not a delivery: the merge is
        // heard where the flags read it (`merge_verdict`).
        let merged = branch("bbb", "merged", 0, "DONE");
        assert_eq!(verdict(Some(&first), &merged, false, false), None, "not a delivery");
        // New work on top is a new delivery; so is a branch the base moved
        // past, which still has something to merge.
        let more = branch("ccc", "ahead", 2, "REVIEW");
        assert_eq!(verdict(Some(&first), &more, false, false), Some(WakeCause::Delivered));
        let behind = branch("ddd", "needs_rebase", 1, "REVIEW");
        assert_eq!(verdict(None, &behind, false, false), Some(WakeCause::Delivered));
    }

    /// T-554: a delivery the armed merge train will land is held, with no
    /// wake, and one already held is judged again by the next turn. Still
    /// the train's (the rebase it asked for went through): held. Not the
    /// train's any more (that rebase left the branch behind the same tip,
    /// or the train is off): the delivery comes due. An answer is never
    /// held, and a silent turn with nothing held never asks the train.
    #[test]
    fn a_delivery_the_train_will_take_is_held() {
        let behind = branch("aaa", "needs_rebase", 1, "REVIEW");
        let cause = verdict(None, &behind, false, false);
        assert_eq!(with_train(cause, false, || true), Due::Hold);
        assert_eq!(with_train(cause, false, || false), Due::Wake(WakeCause::Delivered));
        // The train's rebase ask: a merge step, silent by `verdict`.
        let rebased = branch("bbb", "ahead", 1, "REVIEW");
        let cause = verdict(Some(&behind), &rebased, false, true);
        assert_eq!(cause, None);
        assert_eq!(with_train(cause, true, || true), Due::Hold, "the train merges it next");
        assert_eq!(
            with_train(cause, true, || false),
            Due::Wake(WakeCause::Delivered),
            "the train gave up"
        );
        assert_eq!(with_train(None, false, || panic!("the train was asked")), Due::Silent);
        assert_eq!(
            with_train(Some(WakeCause::Answered), true, || panic!("the train was asked")),
            Due::Wake(WakeCause::Answered)
        );
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
        assert_eq!(verdict(Some(&before), &rebased, true, false), Some(WakeCause::Answered));
        assert_eq!(verdict(Some(&before), &before, true, false), Some(WakeCause::Answered));
        assert_eq!(verdict(Some(&before), &rebased, false, true), None);
        assert_eq!(verdict(Some(&before), &rebased, true, true), Some(WakeCause::Answered));
    }

    /// A shared checkout delivers by HEAD: a new one wakes, the same one
    /// does not, and with no baseline (a restart) any HEAD does.
    #[test]
    fn a_checkout_delivers_on_a_new_head() {
        let start = checkout("1111111aaaa", "IN PROGRESS");
        let after = checkout("2222222bbbb", "REVIEW");
        assert_eq!(verdict(Some(&start), &start, false, false), None);
        assert_eq!(verdict(Some(&start), &after, false, false), Some(WakeCause::Delivered));
        assert_eq!(verdict(None, &after, false, false), Some(WakeCause::Delivered));
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

    #[test]
    fn coalesced_causes_keep_the_strongest() {
        assert!(WakeCause::Raised > WakeCause::Answered);
        assert!(WakeCause::Answered > WakeCause::Delivered);
        assert!(WakeCause::Delivered > WakeCause::Merged);
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
        let h = Heard { deferred: Some(BranchLook::default()), ..heard(Some(rebased), None) };
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
}
