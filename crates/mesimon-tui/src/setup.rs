//! Setup (T-816): the first time a board opens, a few questions in a
//! person's words, one page each, with the practice board under them
//! showing what the answer does.
//!
//! The practice board is the tour's (`tour::toy`): the real board drawn on
//! a toy daemon that keeps its world in memory. Each page lays it out from
//! the answers already given and the one under the cursor, and holds still
//! (T-850): a card or two where the answer puts them, their agents at work
//! and saying what an agent would say there, and nothing moving, since a
//! person who has never seen mesimon cannot follow a board in motion.
//! Nothing on it leaves the toy; the answers reach the daemon once, as
//! `Command::ApplySetup`, when the last page is left.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::Result;
use mesimon_core::board::{
    MergeInto, SessionKind, SessionRecord, SessionState, StopReason, Template, WorkspaceStrategy,
};
use mesimon_core::command::PrItem;
use mesimon_core::forge::Forge;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::theme::Theme;
use crate::tour::toy::{Shared, Toy, World};

/// One page's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ask {
    /// On your own, with pull requests, or not sure (on your own).
    Work,
    /// On your own: does the train merge into main.
    Merge,
    /// With pull requests: does the agent bring a behind PR up to date.
    Fix,
    /// With pull requests: which columns follow one.
    Board,
    /// A folder of several repositories: a worktree per ticket.
    Projects,
}

/// One answer on a page: the word the code reads, and the words it shows.
pub(crate) struct Opt {
    pub key: &'static str,
    pub label: String,
    pub detail: String,
    /// Marked `(recommended)` beside the label.
    pub recommended: bool,
}

/// What the folder is, learned as Setup opens.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    /// The forge `origin` is on, and whether its CLI is signed in there;
    /// `None` until the probe answers, or where there is no such remote.
    pub forge: Option<(Forge, bool)>,
    /// The probe answered.
    pub probed: bool,
    /// The folder's own repositories, where it is a workspace of them.
    pub repos: usize,
}

impl Facts {
    fn forge(&self) -> Forge {
        self.forge.map_or(Forge::GitHub, |(f, _)| f)
    }
}

/// What Setup asks the daemon for, and what the TUI does itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub template: Template,
    pub noun: &'static str,
    pub merge_into: MergeInto,
    pub workspace: WorkspaceStrategy,
    /// This board's auto merge, where the answer turned it off.
    pub auto_merge_off: bool,
    /// Open the guided tour after.
    pub tour: bool,
}

pub(crate) struct Setup {
    /// The page: an index into `asks()`, or its length for the last page.
    pub at: usize,
    /// The answer under the cursor on this page.
    pub cursor: usize,
    chosen: HashMap<Ask, &'static str>,
    skipped: HashSet<Ask>,
    pub facts: Facts,
    probe: Option<mpsc::Receiver<Option<(Forge, bool)>>>,
    pub sandbox: Box<App>,
    world: Shared,
    theme: Theme,
    prefs: crate::prefs::Prefs,
    /// Where the practice board is drawn before it is laid on the page.
    pub(crate) canvas: RefCell<Option<Terminal<TestBackend>>>,
}

const LOGIN: &str = "Fix the login redirect";
const DARK: &str = "Dark mode toggle";
const AVATARS: &str = "Cache the avatar images";

impl Setup {
    /// Setup over the board at `repo`; `probe` asks the forge off this
    /// thread (false in tests, which never run a CLI).
    pub fn new(
        theme: &Theme,
        prefs: crate::prefs::Prefs,
        repo: PathBuf,
        repos: usize,
        probe: bool,
    ) -> Result<Self> {
        let world = Rc::new(RefCell::new(World::new()));
        let sandbox = Box::new(App::practice(
            Box::new(Toy(world.clone())),
            theme.variant(theme.flavor),
            &prefs,
        )?);
        let probe = probe.then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(mesimon_daemon::forge::setup_probe(&repo));
            });
            rx
        });
        let mut s = Self {
            at: 0,
            cursor: 0,
            chosen: HashMap::new(),
            skipped: HashSet::new(),
            facts: Facts { repos, probed: probe.is_none(), ..Default::default() },
            probe,
            sandbox,
            world,
            theme: theme.variant(theme.flavor),
            prefs,
            canvas: RefCell::new(None),
        };
        s.stage()?;
        Ok(s)
    }

    /// The pages, in order, as the answers so far shape them.
    pub fn asks(&self) -> Vec<Ask> {
        let mut out = vec![Ask::Work];
        if self.team() {
            out.extend([Ask::Fix, Ask::Board]);
        } else {
            out.push(Ask::Merge);
        }
        if self.facts.repos >= 2 {
            out.push(Ask::Projects);
        }
        out
    }

    /// The page's question, `None` on the last page.
    pub fn ask(&self) -> Option<Ask> {
        self.asks().get(self.at).copied()
    }

    pub fn ready(&self) -> bool {
        self.at >= self.asks().len()
    }

    /// The forge's own words for a pull request and its site.
    fn words(&self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self.facts.forge() {
            Forge::GitHub => ("pull request", "PR", "GitHub", "gh"),
            Forge::GitLab => ("merge request", "MR", "GitLab", "glab"),
        }
    }

    /// The step's name in the row of steps.
    pub fn rail(&self, ask: Ask) -> String {
        match ask {
            Ask::Work => "How you work".into(),
            Ask::Merge => "Merging".into(),
            Ask::Fix => format!("{}s", self.words().1),
            Ask::Board => "Board".into(),
            Ask::Projects => "Projects".into(),
        }
    }

    pub fn question(&self) -> String {
        let (noun, ..) = self.words();
        match self.ask() {
            None => "Your board is ready".into(),
            Some(Ask::Work) => "How do you usually work?".into(),
            Some(Ask::Merge) => {
                "When an agent's work is ready, should mesimon merge it into main?".into()
            }
            Some(Ask::Fix) => {
                format!("If a {noun} falls behind main, should its agent bring it up to date?")
            }
            Some(Ask::Board) => "Pick your board".into(),
            Some(Ask::Projects) => {
                format!(
                    "This folder has {} projects. Give each ticket its own copy?",
                    self.facts.repos
                )
            }
        }
    }

    /// The line under the question, where the page has a fact to state.
    pub fn status(&self) -> Option<(String, bool)> {
        if self.ask() != Some(Ask::Fix) {
            return None;
        }
        let (_, _, site, cli) = self.words();
        Some(match self.facts.forge {
            _ if !self.facts.probed => (format!("looking for {site}…"), true),
            None => ("no GitHub or GitLab remote here yet".into(), false),
            Some((_, true)) => (format!("{site} ✓ signed in with {cli}"), true),
            Some((_, false)) => (format!("{site} isn't signed in ∙ run {cli} auth login"), false),
        })
    }

    pub fn options(&self) -> Vec<Opt> {
        let (_, short, site, _) = self.words();
        let nouns = format!("{}s", self.words().0);
        let o = |key, label: &str, detail: &str| Opt {
            key,
            label: label.to_string(),
            detail: detail.to_string(),
            recommended: false,
        };
        let best =
            |key, label: &str, detail: &str| Opt { recommended: true, ..o(key, label, detail) };
        let board = |t: Template, recommended: bool| -> Opt {
            let names: Vec<String> =
                t.columns(short).into_iter().map(|(n, _)| say_column(&n, short)).collect();
            Opt {
                key: template_key(t),
                label: names.join(" → "),
                detail: String::new(),
                recommended,
            }
        };
        match self.ask() {
            None => {
                vec![o("board", "Open the board", ""), o("tour", "Take the guided tour first", "")]
            }
            Some(Ask::Work) => vec![
                o("alone", "On my own", "finished work goes straight into main"),
                Opt {
                    key: "team",
                    label: format!("With {nouns}"),
                    detail: format!("work goes to {site} for review first"),
                    recommended: false,
                },
                o("unsure", "I'm not sure", "start simple: straight into main, no reviews"),
            ],
            Some(Ask::Merge) => vec![
                best("yes", "Yes, once it reaches Review", ""),
                o("no", "No, I'll press m on the card", ""),
            ],
            Some(Ask::Fix) => vec![best("yes", "Yes", ""), o("no", "No, I'll do it myself", "")],
            Some(Ask::Board) => vec![
                board(Template::PullRequests, true),
                board(Template::PullRequestsProduction, false),
            ],
            Some(Ask::Projects) => vec![
                best("own", "Yes", "agents never get in each other's way"),
                o("shared", "No", "tickets take turns in one copy"),
            ],
        }
    }

    /// Was this page left on its default?
    pub fn skipped(&self, ask: Ask) -> bool {
        self.skipped.contains(&ask)
    }

    /// Was this page answered, either way?
    pub fn answered(&self, ask: Ask) -> bool {
        self.chosen.contains_key(&ask) || self.skipped.contains(&ask)
    }

    /// The answer a page holds: the cursor's on this page, the chosen one
    /// on a page passed, the first (the default) on any other.
    fn pick(&self, ask: Ask) -> &'static str {
        if self.ask() == Some(ask) {
            if let Some(o) = self.options().get(self.cursor) {
                return o.key;
            }
        }
        if let Some(k) = self.chosen.get(&ask) {
            return k;
        }
        match ask {
            Ask::Work => "alone",
            Ask::Merge | Ask::Fix => "yes",
            Ask::Board => "prs",
            Ask::Projects => "own",
        }
    }

    /// The answer to the first page is pull requests: under the cursor
    /// while it is on screen, the one given after. Not sure is on your own.
    fn team(&self) -> bool {
        if self.at == 0 {
            self.cursor == 1
        } else {
            self.chosen.get(&Ask::Work).copied() == Some("team")
        }
    }

    /// The board's columns: a plain board on your own, the pull request
    /// board otherwise, as the Board page has it.
    fn template(&self) -> Template {
        if !self.team() {
            return Template::Kanban;
        }
        match self.pick(Ask::Board) {
            "prod" => Template::PullRequestsProduction,
            _ => Template::PullRequests,
        }
    }

    /// The answers, as the daemon and the TUI take them.
    pub fn outcome(&self) -> Outcome {
        let team = self.team();
        let off = if team { self.pick(Ask::Fix) == "no" } else { self.pick(Ask::Merge) == "no" };
        Outcome {
            template: self.template(),
            noun: self.words().1,
            merge_into: if team { MergeInto::PullRequests } else { MergeInto::Main },
            workspace: if self.facts.repos >= 2 && self.pick(Ask::Projects) == "shared" {
                WorkspaceStrategy::SharedCheckout
            } else {
                WorkspaceStrategy::Worktree
            },
            auto_merge_off: off,
            tour: self.ready() && self.cursor == 1,
        }
    }

    // ---- the keys -----------------------------------------------------

    pub fn cursor_step(&mut self, down: bool) -> Result<()> {
        let n = self.options().len();
        if n < 2 {
            return Ok(());
        }
        self.cursor = if down { (self.cursor + 1) % n } else { (self.cursor + n - 1) % n };
        self.stage()
    }

    /// Enter: this page's answer, then the next page.
    pub fn next(&mut self) -> Result<()> {
        if let Some(ask) = self.ask() {
            let key = self.pick(ask);
            self.chosen.insert(ask, key);
            self.skipped.remove(&ask);
            self.go(self.at + 1)?;
        }
        Ok(())
    }

    /// `s`: this page on its default, then the next page.
    pub fn skip(&mut self) -> Result<()> {
        if let Some(ask) = self.ask() {
            self.chosen.remove(&ask);
            self.skipped.insert(ask);
            self.go(self.at + 1)?;
        }
        Ok(())
    }

    /// `h`: the page before, its answer under the cursor.
    pub fn back(&mut self) -> Result<()> {
        if self.at > 0 {
            self.go(self.at - 1)?;
        }
        Ok(())
    }

    /// A page by its place in the row of steps, its answer under the
    /// cursor.
    pub fn go(&mut self, at: usize) -> Result<()> {
        self.at = at.min(self.asks().len());
        self.cursor = 0;
        if let Some(k) = self.ask().and_then(|a| self.chosen.get(&a).copied()) {
            self.cursor = self.options().iter().position(|o| o.key == k).unwrap_or(0);
        }
        self.stage()
    }

    // ---- the clock ----------------------------------------------------

    /// The practice board's own clock, and the probe's answer when it
    /// lands.
    pub fn tick(&mut self) -> Result<bool> {
        let mut dirty = self.sandbox.pump()?;
        if let Some(rx) = &self.probe {
            if let Ok(answer) = rx.try_recv() {
                self.facts.forge = answer;
                self.facts.probed = true;
                self.probe = None;
                self.stage()?;
                dirty = true;
            }
        }
        Ok(dirty)
    }

    /// Something on the page wants frames sooner than the spinner's.
    pub fn animating(&self) -> bool {
        self.sandbox.animating()
    }

    /// The page's board: a fresh world laid out from the answers, a fresh
    /// board over it.
    fn stage(&mut self) -> Result<()> {
        let mut world = World::new();
        self.lay(&mut world);
        world.changed = false;
        *self.world.borrow_mut() = world;
        *self.sandbox = App::practice(
            Box::new(Toy(self.world.clone())),
            self.theme.variant(self.theme.flavor),
            &self.prefs,
        )?;
        self.sandbox.rich_keys = true;
        self.sandbox.peek_all = true;
        for (path, text) in &self.world.borrow().peeks {
            self.sandbox.peek_cache.stage(path, text);
        }
        Ok(())
    }

    // ---- the pages' boards --------------------------------------------

    /// What the page shows: the board its answers make, and on it the
    /// card or two the question is about, where the answer under the
    /// cursor puts them, their agents saying what they would say there.
    fn lay(&self, w: &mut World) {
        let (_, short, site, _) = self.words();
        let f = self.facts.forge();
        let _ = w.board.apply_template(self.template(), short);
        w.board.merge_into = if self.team() { MergeInto::PullRequests } else { MergeInto::Main };
        match self.ask() {
            Some(Ask::Work) if self.team() => {
                let a = w.ticket(&nth(w, 2), LOGIN);
                w.worktree(a, 2);
                w.prs.push(pr_of(f, a, 412, "open", "running"));
                agent(w, a, idle(), &format!("Opened {short} #412 on {site} for review."));
            }
            Some(Ask::Work) => {
                let a = w.ticket(&nth(w, 1), LOGIN);
                w.worktree(a, 1);
                agent(w, a, SessionState::Running, "Fixing it in my own copy of the code.");
            }
            Some(Ask::Merge) => {
                if self.pick(Ask::Merge) == "yes" {
                    let a = w.ticket(&nth(w, 3), LOGIN);
                    w.worktree(a, 2);
                    w.merged(a);
                    agent(w, a, idle(), "Done, tests pass. It's in main now.");
                } else {
                    let a = w.ticket(&nth(w, 2), LOGIN);
                    w.worktree(a, 2);
                    agent(w, a, idle(), "Done, tests pass. Over to you.");
                }
            }
            Some(Ask::Fix) => {
                let d = w.ticket(&nth(w, 3), DARK);
                w.worktree(d, 1);
                w.merged(d);
                let a = w.ticket(&nth(w, 2), LOGIN);
                w.worktree(a, 2);
                let mut mine = pr_of(f, a, 412, "open", "passed");
                mine.conflict = true;
                w.prs = vec![pr_of(f, d, 411, "merged", "passed"), mine];
                for wt in w.worktrees.iter_mut().filter(|t| t.ticket == a) {
                    wt.conflict = true;
                    wt.needs_rebase = true;
                }
                if self.pick(Ask::Fix) == "yes" {
                    let says = format!("main moved on; updating {short} #412.");
                    agent(w, a, SessionState::Running, &says);
                } else {
                    let says = format!("{short} #412 conflicts with main now.");
                    agent(w, a, idle(), &says);
                }
            }
            Some(Ask::Board) => {
                if self.template() == Template::PullRequestsProduction {
                    let c = w.ticket(&nth(w, 4), AVATARS);
                    let mut p = pr_of(f, c, 410, "merged", "passed");
                    p.deployed = vec!["production".into()];
                    w.prs.push(p);
                }
                let d = w.ticket(&nth(w, 3), DARK);
                w.prs.push(pr_of(f, d, 411, "merged", "passed"));
                let a = w.ticket(&nth(w, 2), LOGIN);
                w.worktree(a, 2);
                w.prs.push(pr_of(f, a, 412, "open", "passed"));
                agent(w, a, idle(), &format!("{short} #412 is open; checks pass."));
            }
            Some(Ask::Projects) => {
                let col = nth(w, 1);
                let a = w.ticket(&col, LOGIN);
                if self.pick(Ask::Projects) == "own" {
                    let c = w.ticket(&col, AVATARS);
                    for id in [a, c] {
                        w.worktree(id, 1);
                    }
                    agent(w, a, SessionState::Running, "Working in my own copy.");
                    agent(w, c, SessionState::Running, "In my own copy too.");
                } else {
                    let c = w.ticket(&nth(w, 0), AVATARS);
                    for id in [a, c] {
                        if let Some(t) = w.board.ticket_mut(id) {
                            t.workspace = Some(WorkspaceStrategy::SharedCheckout);
                        }
                    }
                    agent(w, a, SessionState::Running, "Working in the folder itself.");
                }
            }
            None => {
                w.ticket(&nth(w, 0), LOGIN);
            }
        }
    }
}

/// An agent on `ticket` in `state`, saying `words`. Placed by hand: the
/// toy's `session` runs the column's automove, and a page's cards stay
/// where it laid them.
fn agent(w: &mut World, ticket: ulid::Ulid, state: SessionState, words: &str) {
    w.board.sessions.push(SessionRecord::new(
        uuid::Uuid::from_u128(ticket.0),
        SessionKind::Claude,
        ticket,
        vec!["claude".into()],
        "/practice".into(),
        state,
    ));
    w.says(ticket, words);
}

/// A PR on `forge`, numbered as the forge numbers it.
fn pr_of(forge: Forge, ticket: ulid::Ulid, number: u64, state: &str, pipeline: &str) -> PrItem {
    let (word, url) = match forge {
        Forge::GitHub => ("github", format!("https://github.com/acme/web/pull/{number}")),
        Forge::GitLab => {
            ("gitlab", format!("https://gitlab.com/acme/web/-/merge_requests/{number}"))
        }
    };
    PrItem {
        ticket,
        forge: word.into(),
        repo: "acme/web".into(),
        number,
        url,
        state: state.into(),
        pipeline: pipeline.into(),
        review: "approved".into(),
        ..Default::default()
    }
}

/// A column's name in a sentence: `To do`, `In progress`, `PR open`.
fn say_column(name: &str, noun: &str) -> String {
    if name == "TODO" {
        return "To do".into();
    }
    let words: Vec<String> =
        name.split(' ').map(|w| if w == noun { w.to_string() } else { w.to_lowercase() }).collect();
    let mut out = words.join(" ");
    if let Some(first) = out.get(0..1).map(str::to_uppercase) {
        out.replace_range(0..1, &first);
    }
    out
}

fn template_key(t: Template) -> &'static str {
    match t {
        Template::Kanban => "kanban",
        Template::PullRequests => "prs",
        Template::PullRequestsProduction => "prod",
    }
}

fn idle() -> SessionState {
    SessionState::Idle { stop_reason: StopReason::EndTurn }
}

/// The board's `i`-th column's name, clamped.
fn nth(w: &World, i: usize) -> String {
    let cols = w.board.sorted_columns();
    cols.get(i.min(cols.len().saturating_sub(1))).map(|c| c.name.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};

    fn setup(repos: usize) -> Setup {
        Setup::new(
            &Theme::new(Flavor::Graphite, Profile::TrueColor)
                .with_underline_colors(true)
                .with_block_gaps(false),
            Default::default(),
            "/practice".into(),
            repos,
            false,
        )
        .expect("setup")
    }

    /// Every page, every answer under the cursor, on any first answer and
    /// either forge: the board holds still, every agent on it says
    /// something, and nothing reaches past the practice board.
    #[test]
    fn every_page_holds_still_and_speaks() {
        for repos in [0, 6] {
            for forge in [Forge::GitHub, Forge::GitLab] {
                for work in [0, 1, 2] {
                    let mut s = setup(repos);
                    s.facts.forge = Some((forge, true));
                    s.cursor = work;
                    s.next().expect("next");
                    for at in 0..=s.asks().len() {
                        s.go(at).expect("go");
                        for cursor in 0..s.options().len() {
                            s.cursor = cursor;
                            s.stage().expect("stage");
                            let w = s.world.borrow();
                            let here = format!("{:?} {cursor}", s.ask());
                            assert!(w.refused.is_empty(), "{here}: {:?}", w.refused);
                            assert!(
                                w.board.tickets.iter().all(|t| t.previous_column.is_none()),
                                "{here}: a card arrived from somewhere"
                            );
                            for r in &w.board.sessions {
                                let said = r.transcript_path.as_ref().and_then(|p| w.peeks.get(p));
                                assert!(said.is_some(), "{here}: an agent with nothing to say");
                            }
                        }
                    }
                }
            }
        }
    }

    /// The pages follow the first answer: on your own (or not sure) asks
    /// about merging, pull requests about falling behind and their board,
    /// and a folder of projects asks about them.
    #[test]
    fn the_pages_follow_how_you_work() {
        let mut s = setup(0);
        assert_eq!(s.asks(), [Ask::Work, Ask::Merge]);
        s.cursor_step(true).expect("down");
        assert_eq!(s.asks(), [Ask::Work, Ask::Fix, Ask::Board]);
        s.cursor_step(true).expect("down");
        assert_eq!(s.options()[s.cursor].key, "unsure");
        assert_eq!(s.asks(), [Ask::Work, Ask::Merge], "not sure is on your own");
        s.cursor_step(false).expect("up");
        s.next().expect("next");
        s.next().expect("next");
        assert_eq!(s.ask(), Some(Ask::Board));
        let opts = s.options();
        assert_eq!(opts.len(), 2, "only the pull request boards");
        assert!(opts[0].recommended && !opts[1].recommended);
        assert_eq!(opts[0].label, "To do → In progress → PR open → Merged");
        s.facts.forge = Some((Forge::GitLab, true));
        assert_eq!(s.options()[0].label, "To do → In progress → MR open → Merged");
        assert_eq!(setup(6).asks()[2], Ask::Projects);
    }

    /// Each question's recommended answer is its default, and marked.
    #[test]
    fn the_default_is_the_recommended_answer() {
        let mut s = setup(6);
        s.cursor = 1;
        s.next().expect("team");
        for _ in 0..3 {
            let opts = s.options();
            assert!(opts[0].recommended, "{:?}", s.ask());
            assert_eq!(opts.iter().filter(|o| o.recommended).count(), 1, "{:?}", s.ask());
            s.next().expect("next");
        }
        assert!(s.ready());
        assert!(s.options().iter().all(|o| !o.recommended), "the last page recommends nothing");
        let mut s = setup(0);
        assert!(s.options().iter().all(|o| !o.recommended), "how you work is the person's");
        s.next().expect("alone");
        assert!(s.options()[0].recommended, "merging for you");
    }

    /// What `s` on every page applies: a plain board, worktrees, auto
    /// merge kept.
    #[test]
    fn skipping_everything_is_the_defaults() {
        let mut s = setup(0);
        while !s.ready() {
            s.skip().expect("skip");
        }
        assert!(s.ready());
        assert!(s.skipped(Ask::Work) && s.skipped(Ask::Merge));
        assert_eq!(
            s.outcome(),
            Outcome {
                template: Template::Kanban,
                noun: "PR",
                merge_into: MergeInto::Main,
                workspace: WorkspaceStrategy::Worktree,
                auto_merge_off: false,
                tour: false,
            }
        );
    }

    /// Not sure is on your own: straight into main, no pull requests.
    #[test]
    fn not_sure_goes_straight_into_main() {
        let mut s = setup(0);
        s.cursor = 2;
        s.next().expect("unsure");
        assert_eq!(s.ask(), Some(Ask::Merge));
        s.skip().expect("skip");
        assert!(s.ready());
        let o = s.outcome();
        assert_eq!((o.template, o.merge_into), (Template::Kanban, MergeInto::Main));
        assert!(!o.auto_merge_off);
    }

    /// Each answer lands where the daemon and the TUI read it.
    #[test]
    fn the_answers_become_the_outcome() {
        let mut s = setup(6);
        s.facts.forge = Some((Forge::GitLab, false));
        s.cursor_step(true).expect("team");
        s.next().expect("work");
        s.cursor_step(true).expect("no fix");
        s.next().expect("fix");
        s.cursor_step(true).expect("production");
        s.next().expect("board");
        s.cursor_step(true).expect("shared");
        s.next().expect("projects");
        assert!(s.ready());
        s.cursor_step(true).expect("tour");
        let o = s.outcome();
        assert_eq!(o.template, Template::PullRequestsProduction);
        assert_eq!(o.noun, "MR");
        assert_eq!(o.merge_into, MergeInto::PullRequests);
        assert_eq!(o.workspace, WorkspaceStrategy::SharedCheckout);
        assert!(o.auto_merge_off && o.tour);
        // Back keeps every answer.
        s.back().expect("back");
        assert_eq!(s.ask(), Some(Ask::Projects));
        s.back().expect("back");
        assert_eq!(s.options()[s.cursor].key, "prod");
    }

    /// `MESIMON_SETUP_DUMP=<page>[,<cursor>[,team]] cargo test -p mesimon-tui
    /// dump_a_page -- --nocapture` prints the page's practice board.
    #[test]
    fn dump_a_page() {
        let Ok(arg) = std::env::var("MESIMON_SETUP_DUMP") else { return };
        let parts: Vec<&str> = arg.split(',').collect();
        let at: usize = parts[0].parse().expect("a page number");
        let cursor: usize = parts.get(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let team = parts.get(2) == Some(&"team");
        let mut s = setup(0);
        s.facts.forge = Some((Forge::GitHub, true));
        s.cursor = usize::from(team);
        s.next().expect("next");
        s.go(at).expect("go");
        s.cursor = cursor;
        s.stage().expect("stage");
        let mut term = Terminal::new(TestBackend::new(110, 16)).expect("canvas");
        term.draw(|g| crate::ui::draw(g, &s.sandbox)).expect("draw");
        let buf = term.backend().buffer();
        println!("---- {:?} {cursor}", s.ask());
        for y in 0..buf.area.height {
            let row: String =
                (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
            println!("{}", row.trim_end());
        }
    }
}
