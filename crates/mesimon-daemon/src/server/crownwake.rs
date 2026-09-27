//! What wakes the crown (T-414, narrowed by T-469): three ticket events, not
//! every `Stop`.
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
//! 3. **raised its hand** — `raise_hand`, as before.
//!
//! A turn that took mesimon's own merge-flow words (`m`'s or the train's
//! rebase ask, the merged notice) is a merge step: the person already knows,
//! and the crown learns on its next `get_ticket`. Every wake line carries
//! what changed since the crown last heard (`merge_state needs_rebase →
//! ahead`, `column REVIEW`), so the obvious costs no tool call.

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

/// What the crown is woken for, in the precedence two events on one worker
/// coalesce by: a hand outranks an answer, an answer a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum WakeCause {
    Delivered,
    Answered,
    Raised,
}

impl WakeCause {
    /// The clause after the ticket in the sentence.
    fn clause(self) -> &'static str {
        match self {
            WakeCause::Delivered => "delivered",
            WakeCause::Answered => "answered your ask",
            WakeCause::Raised => "raised its hand",
        }
    }

    /// The feed's word.
    fn word(self) -> &'static str {
        match self {
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

/// What changed between what the crown last heard and now, in words:
/// `merge_state needs_rebase → ahead`, `ahead 1 → 3`, `commit 1a2b3c4`,
/// `column REVIEW`. With nothing heard before, the state itself.
pub(super) fn delta(before: Option<&Told>, now: &Told) -> Vec<String> {
    let mut out = Vec::new();
    match (before.and_then(|b| b.merge), now.merge) {
        (Some(was), Some(is)) if was != is => out.push(format!("merge_state {was} → {is}")),
        (None, Some(is)) => out.push(format!("merge_state {is}")),
        _ => {}
    }
    if let Some(b) = before.filter(|b| b.merge.is_some() && now.merge.is_some()) {
        if b.ahead != now.ahead {
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
    Branch {
        tip: String,
        merge: &'static str,
        ahead: u32,
    },
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
                Found::Branch {
                    merge: worktree::merge_word(a.merged, a.needs_rebase, a.ahead),
                    tip: a.tip,
                    ahead: a.ahead,
                }
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
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let found = look.run(worker);
            let _ = tx.send(Msg::TurnProbed(TurnProbe { worker, crown, why, found }));
        });
    }

    /// A probe landed: judge it against the worker's baseline, move the
    /// baseline, and owe the crown a wake when the verdict says so.
    pub(super) fn on_turn_probed(&mut self, p: TurnProbe) {
        if self.board.crown_holder().map(|t| t.id) != Some(p.crown) {
            return;
        }
        let Some(column) = self.board.ticket(p.worker).map(|t| t.column.clone()) else {
            return;
        };
        let now = match p.found {
            Found::Branch { tip, merge, ahead } => Told { tip, merge: Some(merge), ahead, column },
            Found::Checkout { head } => Told { tip: head, merge: None, ahead: 0, column },
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
        if let Some(cause) = verdict(before.as_ref(), &now, answered, merge_step) {
            let from = heard.told.replace(now.clone());
            self.note_crown_wake(p.worker, cause, from, Some(now));
        }
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
        if let Some(w) = self.crown_wakes.iter_mut().find(|w| w.worker == worker) {
            w.cause = w.cause.max(cause);
            if w.to.is_none() {
                w.from = from;
            }
            if to.is_some() {
                w.to = to;
            }
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
        let merged = branch("bbb", "merged", 0, "DONE");
        assert_eq!(verdict(Some(&first), &merged, false, false), None, "a merge is not news");
        // New work on top is a new delivery; so is a branch the base moved
        // past, which still has something to merge.
        let more = branch("ccc", "ahead", 2, "REVIEW");
        assert_eq!(verdict(Some(&first), &more, false, false), Some(WakeCause::Delivered));
        let behind = branch("ddd", "needs_rebase", 1, "REVIEW");
        assert_eq!(verdict(None, &behind, false, false), Some(WakeCause::Delivered));
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
    }
}
