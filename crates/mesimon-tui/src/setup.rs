//! Setup (T-816): the first time a board opens, one question in a
//! person's words, how they work, with the practice board under it showing
//! what the answer does. Everything else takes its recommended setting
//! (T-1123): mesimon merges for you, an agent brings a behind pull request
//! up to date, the pull request board runs to merged, and every ticket gets
//! its own worktree.
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
    MergeInto, SessionKind, SessionRecord, SessionState, StopReason, Template,
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
}

/// One answer on a page: the word the code reads, and the words it shows.
pub(crate) struct Opt {
    pub key: &'static str,
    pub label: String,
    pub detail: String,
}

/// What the folder is, learned as Setup opens.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facts {
    /// The forge `origin` is on, and whether its CLI is signed in there;
    /// `None` until the probe answers, or where there is no such remote.
    pub forge: Option<(Forge, bool)>,
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

impl Setup {
    /// Setup over the board at `repo`; `probe` asks the forge off this
    /// thread (false in tests, which never run a CLI).
    pub fn new(
        theme: &Theme,
        prefs: crate::prefs::Prefs,
        repo: PathBuf,
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
            facts: Facts::default(),
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

    /// The pages, in order: how you work, then the last page.
    pub fn asks(&self) -> Vec<Ask> {
        vec![Ask::Work]
    }

    /// The page's question, `None` on the last page.
    pub fn ask(&self) -> Option<Ask> {
        self.asks().get(self.at).copied()
    }

    pub fn ready(&self) -> bool {
        self.at >= self.asks().len()
    }

    /// The forge's own words for a pull request and its site.
    fn words(&self) -> (&'static str, &'static str, &'static str) {
        match self.facts.forge() {
            Forge::GitHub => ("pull request", "PR", "GitHub"),
            Forge::GitLab => ("merge request", "MR", "GitLab"),
        }
    }

    /// The step's name in the row of steps.
    pub fn rail(&self, ask: Ask) -> String {
        match ask {
            Ask::Work => "How you work".into(),
        }
    }

    pub fn question(&self) -> String {
        match self.ask() {
            None => "Your board is ready".into(),
            Some(Ask::Work) => "How do you usually work?".into(),
        }
    }

    /// The line under the last page's question: what the board does
    /// without being asked.
    pub fn status(&self) -> Option<String> {
        if !self.ready() {
            return None;
        }
        let fix = if self.team() {
            format!("agents keep their {}s up to date", self.words().1)
        } else {
            "merged into main for you".to_string()
        };
        Some(format!("{fix} ∙ each ticket in its own copy"))
    }

    pub fn options(&self) -> Vec<Opt> {
        let (noun, _, site) = self.words();
        let o = |key, label: &str, detail: &str| Opt {
            key,
            label: label.to_string(),
            detail: detail.to_string(),
        };
        match self.ask() {
            None => {
                vec![o("board", "Open the board", ""), o("tour", "Take the guided tour first", "")]
            }
            Some(Ask::Work) => vec![
                o("alone", "On my own", "finished work goes straight into main"),
                o("team", &format!("With {noun}s"), &format!("work goes to {site} for review first")),
                o("unsure", "I'm not sure", "start simple: straight into main, no reviews"),
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
    /// board (to merged) otherwise.
    fn template(&self) -> Template {
        if self.team() {
            Template::PullRequests
        } else {
            Template::Kanban
        }
    }

    /// The answers, as the daemon and the TUI take them.
    pub fn outcome(&self) -> Outcome {
        Outcome {
            template: self.template(),
            noun: self.words().1,
            merge_into: if self.team() { MergeInto::PullRequests } else { MergeInto::Main },
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
        let (_, short, site) = self.words();
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

    fn setup() -> Setup {
        Setup::new(
            &Theme::new(Flavor::Graphite, Profile::TrueColor)
                .with_underline_colors(true)
                .with_block_gaps(false),
            Default::default(),
            "/practice".into(),
            false,
        )
        .expect("setup")
    }

    /// Every page, every answer under the cursor, on any first answer and
    /// either forge: the board holds still, every agent on it says
    /// something, and nothing reaches past the practice board.
    #[test]
    fn every_page_holds_still_and_speaks() {
        for forge in [Forge::GitHub, Forge::GitLab] {
            for work in [0, 1, 2] {
                let mut s = setup();
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

    /// How you work is the only question (T-1123): whatever the answer,
    /// the next page is the last, and nothing on it is a question.
    #[test]
    fn how_you_work_is_the_only_question() {
        for work in [0, 1, 2] {
            let mut s = setup();
            s.cursor = work;
            assert_eq!(s.asks(), [Ask::Work]);
            s.next().expect("next");
            assert!(s.ready(), "answer {work}");
        }
        let mut s = setup();
        s.facts.forge = Some((Forge::GitLab, true));
        assert_eq!(s.options()[1].label, "With merge requests");
    }

    /// What `s` applies: a plain board straight into main, worktrees and
    /// auto merge as the board starts.
    #[test]
    fn skipping_is_the_defaults() {
        let mut s = setup();
        s.skip().expect("skip");
        assert!(s.ready());
        assert!(s.skipped(Ask::Work));
        assert_eq!(
            s.outcome(),
            Outcome {
                template: Template::Kanban,
                noun: "PR",
                merge_into: MergeInto::Main,
                tour: false,
            }
        );
    }

    /// Not sure is on your own: straight into main, no pull requests.
    #[test]
    fn not_sure_goes_straight_into_main() {
        let mut s = setup();
        s.cursor = 2;
        s.next().expect("unsure");
        assert!(s.ready());
        let o = s.outcome();
        assert_eq!((o.template, o.merge_into), (Template::Kanban, MergeInto::Main));
    }

    /// Pull requests get the recommended pull request board, to merged,
    /// and the last page says what the board does without being asked.
    #[test]
    fn the_answers_become_the_outcome() {
        let mut s = setup();
        s.facts.forge = Some((Forge::GitLab, false));
        s.cursor_step(true).expect("team");
        s.next().expect("work");
        assert!(s.ready());
        assert!(s.status().is_some_and(|l| l.contains("MRs up to date")), "{:?}", s.status());
        s.cursor_step(true).expect("tour");
        let o = s.outcome();
        assert_eq!(o.template, Template::PullRequests);
        assert_eq!(o.noun, "MR");
        assert_eq!(o.merge_into, MergeInto::PullRequests);
        assert!(o.tour);
        // Back keeps the answer.
        s.back().expect("back");
        assert_eq!(s.options()[s.cursor].key, "team");
        assert!(s.status().is_none(), "only the last page states it");
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
        let mut s = setup();
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
