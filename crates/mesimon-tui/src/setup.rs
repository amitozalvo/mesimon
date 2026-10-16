//! Setup (T-816): the first time a board opens, a few questions in a
//! person's words, one page each, with the practice board under them
//! playing what the answer does.
//!
//! The practice board is the tour's (`tour::toy`): the real board drawn on
//! a toy daemon that keeps its world in memory. Each page is a scene on it,
//! laid out from the answers already given and the one under the cursor,
//! so the board grows with the answers rather than changing under them.
//! Nothing a scene does leaves the toy; the answers reach the daemon once,
//! as `Command::ApplySetup`, when the last page is left.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::Result;
use mesimon_core::board::{
    AgentTools, MergeInto, Raised, SessionKind, SessionState, StopReason, Template,
    WorkspaceStrategy,
};
use mesimon_core::command::PrItem;
use mesimon_core::forge::Forge;
use mesimon_core::keymap::Key;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::Terminal;

use crate::app::App;
use crate::theme::{Flavor, Profile, Theme};
use crate::tour::toy::{Shared, Toy, World};

/// One page's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ask {
    /// On your own, or with pull requests.
    Work,
    /// On your own: does the train merge into main.
    Merge,
    /// With pull requests: does the agent bring a behind PR up to date.
    Fix,
    /// The board's columns.
    Board,
    /// A folder of several repositories: a worktree per ticket.
    Projects,
    /// What an agent may do on its card.
    Agents,
    /// The agent brief, shown.
    Brief,
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
    pub agent_tools: AgentTools,
    /// This board's auto merge, where the answer turned it off.
    pub auto_merge_off: bool,
    /// Open the guided tour after.
    pub tour: bool,
}

/// One beat of a scene.
enum Beat {
    /// Hold still.
    Wait(u64),
    /// The world moving: an agent, the forge, as the hooks would.
    Do(Box<dyn Fn(&mut World)>),
    /// A key, as a person presses it.
    Key(Key),
    /// Words typed a letter at a time.
    Type(&'static str),
}

const KEY_MS: u64 = 700;
const LETTER_MS: u64 = 55;
const HOLD_MS: u64 = 2_600;

struct Player {
    step: usize,
    typed: usize,
    due: u64,
    ended: Option<u64>,
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
    beats: Vec<Beat>,
    player: Player,
    theme: (Flavor, Profile),
    prefs: crate::prefs::Prefs,
    /// Where the practice board is drawn before it is laid on the page.
    pub(crate) canvas: RefCell<Option<Terminal<TestBackend>>>,
}

const LOGIN: &str = "Fix the login redirect";
const DARK: &str = "Dark mode toggle";
const AVATARS: &str = "Cache the avatar images";
const COOKIE: &str = "Session cookie expires after 5 minutes";

impl Setup {
    /// Setup over the board at `repo`; `probe` asks the forge off this
    /// thread (false in tests, which never run a CLI).
    pub fn new(
        theme: (Flavor, Profile),
        prefs: crate::prefs::Prefs,
        repo: PathBuf,
        repos: usize,
        probe: bool,
    ) -> Result<Self> {
        let world = Rc::new(RefCell::new(World::new()));
        let sandbox = Box::new(App::practice(
            Box::new(Toy(world.clone())),
            Theme::new(theme.0, theme.1),
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
            beats: Vec::new(),
            player: Player { step: 0, typed: 0, due: 0, ended: None },
            theme,
            prefs,
            canvas: RefCell::new(None),
        };
        s.replay(mesimon_core::clock::now_ms())?;
        Ok(s)
    }

    /// The pages, in order, as the answers so far shape them.
    pub fn asks(&self) -> Vec<Ask> {
        let mut out = vec![Ask::Work, if self.team() { Ask::Fix } else { Ask::Merge }, Ask::Board];
        if self.facts.repos >= 2 {
            out.push(Ask::Projects);
        }
        out.extend([Ask::Agents, Ask::Brief]);
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
            Ask::Agents => "Agents".into(),
            Ask::Brief => "Brief".into(),
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
            Some(Ask::Agents) => "What can an agent do on its own card?".into(),
            Some(Ask::Brief) => "Every agent mesimon starts here gets these lines first:".into(),
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
        };
        let board = |t: Template| -> Opt {
            let names: Vec<String> =
                t.columns(short).into_iter().map(|(n, _)| say_column(&n, short)).collect();
            Opt { key: template_key(t), label: names.join(" → "), detail: String::new() }
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
                },
            ],
            Some(Ask::Merge) => {
                vec![
                    o("yes", "Yes, once it reaches Review", ""),
                    o("no", "No, I'll press m on the card", ""),
                ]
            }
            Some(Ask::Fix) => vec![o("yes", "Yes", ""), o("no", "No, I'll do it myself", "")],
            Some(Ask::Board) => {
                let order = if self.team() {
                    [Template::PullRequests, Template::PullRequestsProduction, Template::Kanban]
                } else {
                    [Template::Kanban, Template::PullRequests, Template::PullRequestsProduction]
                };
                order.into_iter().map(board).collect()
            }
            Some(Ask::Projects) => vec![
                o("own", "Yes", "agents never get in each other's way"),
                o("shared", "No", "tickets take turns in one copy"),
            ],
            Some(Ask::Agents) => vec![
                o("full", "Everything", "write notes, move the card, file new tickets"),
                o("notes", "Write notes and ask you", ""),
                o("read", "Only read it", ""),
            ],
            Some(Ask::Brief) => vec![o("on", "Continue", "")],
        }
    }

    /// The brief, verbatim, on its page.
    pub fn text(&self) -> Option<&'static str> {
        (self.ask() == Some(Ask::Brief)).then_some(mesimon_core::brief::TEXT)
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
            Ask::Board => {
                if self.team() {
                    "prs"
                } else {
                    "kanban"
                }
            }
            Ask::Projects => "own",
            Ask::Agents => "full",
            Ask::Brief => "on",
        }
    }

    /// Does the board see this page's answer yet: a page passed, or the one
    /// on screen. The last page sees them all.
    fn known(&self, ask: Ask) -> bool {
        self.ready() || self.answered(ask) || self.ask() == Some(ask)
    }

    /// The answer to the first page: under the cursor while it is on
    /// screen, the one given after.
    fn team(&self) -> bool {
        if self.at == 0 {
            self.cursor == 1
        } else {
            self.chosen.get(&Ask::Work).copied() == Some("team")
        }
    }

    fn template(&self) -> Template {
        match self.pick(Ask::Board) {
            "prs" => Template::PullRequests,
            "prod" => Template::PullRequestsProduction,
            _ => Template::Kanban,
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
            agent_tools: match self.pick(Ask::Agents) {
                "notes" => AgentTools::Annotate,
                "read" => AgentTools::Read,
                _ => AgentTools::Full,
            },
            auto_merge_off: off,
            tour: self.ready() && self.cursor == 1,
        }
    }

    // ---- the keys -----------------------------------------------------

    pub fn cursor_step(&mut self, down: bool, now: u64) -> Result<()> {
        let n = self.options().len();
        if n < 2 {
            return Ok(());
        }
        let was = self.cursor;
        self.cursor = if down { (self.cursor + 1) % n } else { (self.cursor + n - 1) % n };
        if self.at == 0 && was != self.cursor {
            // How you work decides the board's first answer.
            self.chosen.remove(&Ask::Board);
        }
        self.replay(now)
    }

    /// Enter: this page's answer, then the next page.
    pub fn next(&mut self, now: u64) -> Result<()> {
        if let Some(ask) = self.ask() {
            let key = self.pick(ask);
            self.chosen.insert(ask, key);
            self.skipped.remove(&ask);
            self.go(self.at + 1, now)?;
        }
        Ok(())
    }

    /// `s`: this page on its default, then the next page.
    pub fn skip(&mut self, now: u64) -> Result<()> {
        if let Some(ask) = self.ask() {
            if ask == Ask::Work {
                self.chosen.remove(&Ask::Board);
            }
            self.chosen.remove(&ask);
            self.skipped.insert(ask);
            self.go(self.at + 1, now)?;
        }
        Ok(())
    }

    /// Esc: every page not answered on its default, and the last page.
    pub fn skip_rest(&mut self, now: u64) -> Result<()> {
        for ask in self.asks().into_iter().skip(self.at) {
            if !self.chosen.contains_key(&ask) {
                self.skipped.insert(ask);
            }
        }
        let last = self.asks().len();
        self.go(last, now)
    }

    /// `h`: the page before, its answer under the cursor.
    pub fn back(&mut self, now: u64) -> Result<()> {
        if self.at > 0 {
            self.go(self.at - 1, now)?;
        }
        Ok(())
    }

    /// A page by its place in the row of steps, its answer under the
    /// cursor.
    pub fn go(&mut self, at: usize, now: u64) -> Result<()> {
        self.at = at.min(self.asks().len());
        self.cursor = 0;
        if let Some(k) = self.ask().and_then(|a| self.chosen.get(&a).copied()) {
            self.cursor = self.options().iter().position(|o| o.key == k).unwrap_or(0);
        }
        self.replay(now)
    }

    // ---- the clock ----------------------------------------------------

    /// The probe's answer, when it lands, and every beat due by `now`.
    pub fn tick(&mut self, now: u64) -> Result<bool> {
        let mut dirty = self.sandbox.pump()?;
        if let Some(rx) = &self.probe {
            if let Ok(answer) = rx.try_recv() {
                self.facts.forge = answer;
                self.facts.probed = true;
                self.probe = None;
                self.replay(now)?;
                dirty = true;
            }
        }
        while now >= self.player.due {
            if let Some(ended) = self.player.ended {
                if now >= ended + HOLD_MS {
                    self.replay(now)?;
                    dirty = true;
                    continue;
                }
                break;
            }
            let Some(beat) = self.beats.get(self.player.step) else {
                self.player.ended = Some(now);
                self.player.due = now + HOLD_MS;
                break;
            };
            dirty = true;
            match beat {
                Beat::Wait(ms) => {
                    self.player.step += 1;
                    self.player.due = now + ms;
                }
                Beat::Do(change) => {
                    self.player.step += 1;
                    change(&mut self.world.borrow_mut());
                }
                Beat::Key(k) => {
                    self.player.step += 1;
                    self.player.due = now + KEY_MS;
                    let (code, mods) = crate::keys::from_key(*k);
                    self.sandbox.handle_key(code, mods)?;
                }
                Beat::Type(words) => {
                    let Some(c) = words.chars().nth(self.player.typed) else {
                        self.player.step += 1;
                        self.player.typed = 0;
                        self.player.due = now + KEY_MS / 2;
                        continue;
                    };
                    self.player.typed += 1;
                    self.player.due = now + LETTER_MS;
                    let (code, _) = crate::keys::from_key(Key::Char(c));
                    self.sandbox.handle_key(code, KeyModifiers::NONE)?;
                }
            }
            self.settle()?;
        }
        Ok(dirty)
    }

    /// Something on the page wants frames sooner than the spinner's.
    pub fn animating(&self) -> bool {
        self.sandbox.animating()
            || (self.player.ended.is_none()
                && matches!(self.beats.get(self.player.step), Some(Beat::Type(_))))
    }

    /// The board fetches what changed, and the agents' words reach it.
    fn settle(&mut self) -> Result<()> {
        let changed = std::mem::take(&mut self.world.borrow_mut().changed);
        if changed {
            for (path, text) in &self.world.borrow().peeks {
                self.sandbox.peek_cache.stage(path, text);
            }
            self.sandbox.refresh()?;
        }
        Ok(())
    }

    /// The page's scene from its first frame: a fresh world laid out from
    /// the answers, a fresh board over it.
    fn replay(&mut self, now: u64) -> Result<()> {
        let mut world = World::new();
        self.lay(&mut world);
        let beats = self.scene();
        world.changed = false;
        *self.world.borrow_mut() = world;
        *self.sandbox = App::practice(
            Box::new(Toy(self.world.clone())),
            Theme::new(self.theme.0, self.theme.1),
            &self.prefs,
        )?;
        self.sandbox.rich_keys = true;
        self.sandbox.peek_all = true;
        for (path, text) in &self.world.borrow().peeks {
            self.sandbox.peek_cache.stage(path, text);
        }
        self.beats = beats;
        self.player = Player { step: 0, typed: 0, due: now + KEY_MS, ended: None };
        Ok(())
    }

    // ---- the scenes ---------------------------------------------------

    /// What the page plays on the board.
    fn scene(&self) -> Vec<Beat> {
        match self.ask() {
            Some(Ask::Merge) => self.merging(),
            Some(Ask::Fix) => self.behind(),
            Some(Ask::Projects) => self.projects(),
            Some(Ask::Agents) => self.agents(),
            Some(Ask::Brief) => self.brief(),
            Some(Ask::Work) | Some(Ask::Board) | None => self.journey(),
        }
    }

    /// The board every scene starts from: the template once it is known,
    /// the forge's workflow once it is, and the one ticket.
    fn lay(&self, w: &mut World) {
        let t = if self.known(Ask::Board) { self.template() } else { Template::Kanban };
        let _ = w.board.apply_template(t, self.words().1);
        w.board.merge_into = if self.team() { MergeInto::PullRequests } else { MergeInto::Main };
        let start = match self.ask() {
            Some(Ask::Merge) | Some(Ask::Fix) => 2,
            Some(Ask::Agents) => 1,
            _ => 0,
        };
        let col = self.col(w, start);
        let a = w.ticket(&col, LOGIN);
        if self.ask() == Some(Ask::Fix) {
            let b = w.ticket(&col, DARK);
            w.session(b, SessionKind::Claude, idle());
            w.worktree(b, 1);
            w.prs.push(pr_of(self.facts.forge(), b, 411, "open", "passed"));
        }
        if self.ask() == Some(Ask::Projects) {
            w.ticket(&self.col(w, 0), AVATARS);
        }
        if start > 0 {
            w.worktree(a, 1);
            let state =
                if self.ask() == Some(Ask::Agents) { SessionState::Running } else { idle() };
            // Placed by hand: the toy's automove would move the card.
            w.board.sessions.push(mesimon_core::board::SessionRecord::new(
                uuid::Uuid::from_u128(7),
                SessionKind::Claude,
                a,
                vec!["claude".into()],
                "/practice".into(),
                state,
            ));
        }
        if self.team() && start == 2 {
            w.prs.push(pr_of(self.facts.forge(), a, 412, "open", "passed"));
        }
    }

    /// The board's `i`-th column by name: the scenes say "the column after
    /// working", whatever this template calls it.
    fn col(&self, w: &World, i: usize) -> String {
        let cols = w.board.sorted_columns();
        cols.get(i.min(cols.len().saturating_sub(1))).map(|c| c.name.clone()).unwrap_or_default()
    }

    /// The ticket from TODO to where this board's work ends, as far as the
    /// answers so far reach.
    fn journey(&self) -> Vec<Beat> {
        let team = self.team();
        let ends = self.known(Ask::Merge) || self.known(Ask::Fix);
        let merges = if team { true } else { self.pick(Ask::Merge) == "yes" };
        let template = if self.known(Ask::Board) { self.template() } else { Template::Kanban };
        let pr_board = template != Template::Kanban;
        let prod = template == Template::PullRequestsProduction;
        let f = self.facts.forge();
        let mut b = vec![
            Beat::Wait(600),
            Beat::Do(Box::new(|w| {
                if let Some(a) = w.find(LOGIN) {
                    w.worktree(a, 0);
                    w.session(a, SessionKind::Claude, SessionState::Running);
                }
            })),
            Beat::Wait(1_400),
        ];
        if team {
            b.push(Beat::Do(Box::new(move |w| {
                if let Some(a) = w.find(LOGIN) {
                    w.worktree(a, 2);
                    w.prs = vec![pr_of(f, a, 412, "open", "running")];
                    w.changed = true;
                    if pr_board {
                        let to = nth(w, 2);
                        w.arrive(a, &to);
                    }
                }
            })));
            b.push(Beat::Wait(1_300));
        }
        b.push(Beat::Do(Box::new(|w| {
            if let Some(a) = w.find(LOGIN) {
                w.worktree(a, 2);
                w.session(a, SessionKind::Claude, idle());
            }
        })));
        b.push(Beat::Wait(1_200));
        if team {
            b.push(Beat::Do(Box::new(move |w| {
                if let Some(a) = w.find(LOGIN) {
                    w.prs = vec![pr_of(f, a, 412, "open", "passed")];
                    w.changed = true;
                }
            })));
            b.push(Beat::Wait(1_300));
        }
        if !ends || !merges {
            return b;
        }
        b.push(Beat::Do(Box::new(move |w| {
            if let Some(a) = w.find(LOGIN) {
                if team {
                    w.prs = vec![pr_of(f, a, 412, "merged", "passed")];
                }
                w.merged(a);
                let to = nth(w, 3);
                w.arrive(a, &to);
            }
        })));
        b.push(Beat::Wait(1_300));
        if prod && team {
            b.push(Beat::Do(Box::new(move |w| {
                if let Some(a) = w.find(LOGIN) {
                    let mut p = pr_of(f, a, 412, "merged", "passed");
                    p.deployed = vec!["production".into()];
                    w.prs = vec![p];
                    let to = nth(w, 4);
                    w.arrive(a, &to);
                }
            })));
            b.push(Beat::Wait(1_300));
        }
        b
    }

    /// Merging into main: the card in Review lands in Done, or waits for
    /// the person's `m`, the cursor on it.
    fn merging(&self) -> Vec<Beat> {
        let mut b = vec![Beat::Key(Key::Char('l')), Beat::Key(Key::Char('l')), Beat::Wait(900)];
        if self.pick(Ask::Merge) == "yes" {
            b.push(Beat::Do(Box::new(|w| {
                if let Some(a) = w.find(LOGIN) {
                    w.merged(a);
                    let to = nth(w, 3);
                    w.arrive(a, &to);
                }
            })));
            b.push(Beat::Wait(1_600));
        } else {
            b.push(Beat::Wait(2_400));
        }
        b
    }

    /// A PR falling behind: another ticket's merges, this one conflicts,
    /// and its agent brings it up to date, or it waits for the person.
    fn behind(&self) -> Vec<Beat> {
        let f = self.facts.forge();
        let fix = self.pick(Ask::Fix) == "yes";
        let mut b = vec![Beat::Key(Key::Char('l')), Beat::Key(Key::Char('l')), Beat::Wait(800)];
        b.push(Beat::Do(Box::new(move |w| {
            let (Some(a), Some(d)) = (w.find(LOGIN), w.find(DARK)) else { return };
            w.prs = vec![pr_of(f, d, 411, "merged", "passed"), pr_of(f, a, 412, "open", "passed")];
            w.merged(d);
            let to = nth(w, 3);
            w.arrive(d, &to);
        })));
        b.push(Beat::Wait(1_300));
        b.push(Beat::Do(Box::new(move |w| {
            let (Some(a), Some(d)) = (w.find(LOGIN), w.find(DARK)) else { return };
            let mut mine = pr_of(f, a, 412, "open", "passed");
            mine.conflict = true;
            w.prs = vec![pr_of(f, d, 411, "merged", "passed"), mine];
            for wt in w.worktrees.iter_mut().filter(|t| t.ticket == a) {
                wt.conflict = true;
                wt.needs_rebase = true;
            }
            w.changed = true;
        })));
        b.push(Beat::Wait(1_500));
        if !fix {
            b.push(Beat::Wait(1_500));
            return b;
        }
        b.push(Beat::Do(Box::new(|w| {
            if let Some(a) = w.find(LOGIN) {
                w.board.sessions.iter_mut().filter(|s| s.ticket == a).for_each(|s| {
                    s.state = SessionState::Running;
                });
                w.says(a, "Rebasing onto main and fixing the conflict…");
            }
        })));
        b.push(Beat::Wait(1_800));
        b.push(Beat::Do(Box::new(move |w| {
            let (Some(a), Some(d)) = (w.find(LOGIN), w.find(DARK)) else { return };
            w.prs = vec![pr_of(f, d, 411, "merged", "passed"), pr_of(f, a, 412, "open", "running")];
            for wt in w.worktrees.iter_mut().filter(|t| t.ticket == a) {
                wt.conflict = false;
                wt.needs_rebase = false;
            }
            w.board.sessions.iter_mut().filter(|s| s.ticket == a).for_each(|s| {
                s.state = idle();
            });
            w.says(a, "Rebased on main; one conflict in router.ts, the tests pass. Pushed.");
        })));
        b.push(Beat::Wait(1_500));
        b.push(Beat::Do(Box::new(move |w| {
            let (Some(a), Some(d)) = (w.find(LOGIN), w.find(DARK)) else { return };
            w.prs = vec![pr_of(f, d, 411, "merged", "passed"), pr_of(f, a, 412, "open", "passed")];
            w.changed = true;
        })));
        b.push(Beat::Wait(1_500));
        b
    }

    /// Several projects: two agents at once, each in its own copy, or the
    /// second waiting its turn.
    fn projects(&self) -> Vec<Beat> {
        let own = self.pick(Ask::Projects) == "own";
        vec![
            Beat::Wait(700),
            Beat::Do(Box::new(move |w| {
                for title in [LOGIN, AVATARS] {
                    let Some(id) = w.find(title) else { continue };
                    if own {
                        w.worktree(id, 1);
                    } else if let Some(t) = w.board.ticket_mut(id) {
                        t.workspace = Some(WorkspaceStrategy::SharedCheckout);
                    }
                    if own || title == LOGIN {
                        w.session(id, SessionKind::Claude, SessionState::Running);
                    }
                }
            })),
            Beat::Wait(2_600),
        ]
    }

    /// An agent finds a bug, the person asks it to file a ticket, and the
    /// tools decide what happens.
    fn agents(&self) -> Vec<Beat> {
        let tools = self.pick(Ask::Agents);
        vec![
            Beat::Key(Key::Char('l')),
            Beat::Wait(700),
            Beat::Do(Box::new(|w| {
                if let Some(a) = w.find(LOGIN) {
                    w.board.sessions.iter_mut().filter(|s| s.ticket == a).for_each(|s| {
                        s.state = idle();
                    });
                    w.says(
                        a,
                        "Fixed. I also found a bug: the session cookie expires after 5 minutes.",
                    );
                }
            })),
            Beat::Wait(1_600),
            Beat::Key(Key::ShiftEnter),
            Beat::Type("file a ticket to fix it"),
            Beat::Key(Key::Enter),
            Beat::Wait(1_200),
            Beat::Do(Box::new(move |w| {
                let Some(a) = w.find(LOGIN) else { return };
                match tools {
                    "full" => {
                        w.ticket("TODO", COOKIE);
                        let key = w.board.next_key;
                        w.says(a, &format!("Filed T-{key}: {COOKIE}."));
                    }
                    "notes" => {
                        if let Some(t) = w.board.ticket_mut(a) {
                            t.raised = Some(Raised {
                                at: format!("@{}", mesimon_core::clock::now_secs()),
                                by: "agent:practice".into(),
                                reason: "I'm not able to file tickets on this board.".into(),
                            });
                        }
                        w.says(a, "I put the bug in my notes on this ticket instead.");
                    }
                    _ => w.says(a, "I'm not able to file tickets on this board."),
                }
                w.board.sessions.iter_mut().filter(|s| s.ticket == a).for_each(|s| {
                    s.state = idle();
                });
            })),
            Beat::Wait(2_200),
        ]
    }

    /// The brief at work: the agent reads its ticket before anything else.
    fn brief(&self) -> Vec<Beat> {
        vec![
            Beat::Wait(700),
            Beat::Do(Box::new(|w| {
                if let Some(a) = w.find(LOGIN) {
                    w.session(a, SessionKind::Claude, SessionState::Running);
                    w.says(a, "I'll read the ticket first.");
                }
            })),
            Beat::Wait(1_900),
            Beat::Do(Box::new(|w| {
                if let Some(a) = w.find(LOGIN) {
                    w.says(
                        a,
                        "The notes say the old /signin links must keep working. Starting there.",
                    );
                }
            })),
            Beat::Wait(2_000),
        ]
    }
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

    fn setup(repos: usize) -> Setup {
        Setup::new(
            (Flavor::Graphite, Profile::TrueColor),
            Default::default(),
            "/practice".into(),
            repos,
            false,
        )
        .expect("setup")
    }

    /// Play the page's scene to its last beat; true when it got there.
    fn play_out(s: &mut Setup) -> bool {
        let mut now = 1;
        s.replay(now).expect("replay");
        for _ in 0..500 {
            if s.player.ended.is_some() {
                return true;
            }
            now = s.player.due.max(now);
            s.tick(now).expect("tick");
        }
        false
    }

    /// Every page, every answer under the cursor, on either workflow and
    /// either forge: its scene plays to its end, and nothing it presses
    /// reaches past the practice board.
    #[test]
    fn every_page_plays_to_its_end() {
        for repos in [0, 6] {
            for forge in [Forge::GitHub, Forge::GitLab] {
                for work in [0, 1] {
                    let mut s = setup(repos);
                    s.facts.forge = Some((forge, true));
                    s.cursor = work;
                    let pages = {
                        s.next(0).expect("next");
                        s.asks().len()
                    };
                    for at in 1..=pages {
                        s.go(at, 0).expect("go");
                        for cursor in 0..s.options().len() {
                            s.cursor = cursor;
                            assert!(play_out(&mut s), "{:?} {cursor}: never ended", s.ask());
                            let w = s.world.borrow();
                            assert!(
                                w.refused.is_empty(),
                                "{:?} {cursor}: {:?}",
                                s.ask(),
                                w.refused
                            );
                        }
                    }
                }
            }
        }
    }

    /// The pages follow the first answer, and a folder of projects asks
    /// about them.
    #[test]
    fn the_pages_follow_how_you_work() {
        let mut s = setup(0);
        assert_eq!(s.asks(), [Ask::Work, Ask::Merge, Ask::Board, Ask::Agents, Ask::Brief]);
        s.cursor_step(true, 0).expect("down");
        assert_eq!(s.asks(), [Ask::Work, Ask::Fix, Ask::Board, Ask::Agents, Ask::Brief]);
        s.next(0).expect("next");
        s.next(0).expect("next");
        assert_eq!(s.ask(), Some(Ask::Board));
        assert_eq!(s.options()[0].key, "prs", "pull requests preselect their board");
        assert_eq!(s.options()[0].label, "To do → In progress → PR open → Merged");
        s.facts.forge = Some((Forge::GitLab, true));
        assert_eq!(s.options()[0].label, "To do → In progress → MR open → Merged");
        assert_eq!(setup(6).asks()[3], Ask::Projects);
    }

    /// What Esc on the first page applies: today's board, worktrees, every
    /// tool, the brief, auto merge kept.
    #[test]
    fn skipping_everything_is_the_defaults() {
        let mut s = setup(0);
        s.skip_rest(0).expect("skip");
        assert!(s.ready());
        assert!(s.skipped(Ask::Work) && s.skipped(Ask::Brief));
        assert_eq!(
            s.outcome(),
            Outcome {
                template: Template::Kanban,
                noun: "PR",
                merge_into: MergeInto::Main,
                workspace: WorkspaceStrategy::Worktree,
                agent_tools: AgentTools::Full,
                auto_merge_off: false,
                tour: false,
            }
        );
    }

    /// Each answer lands where the daemon and the TUI read it.
    #[test]
    fn the_answers_become_the_outcome() {
        let mut s = setup(6);
        s.facts.forge = Some((Forge::GitLab, false));
        s.cursor_step(true, 0).expect("team");
        s.next(0).expect("work");
        s.cursor_step(true, 0).expect("no fix");
        s.next(0).expect("fix");
        s.cursor_step(true, 0).expect("production");
        s.next(0).expect("board");
        s.cursor_step(true, 0).expect("shared");
        s.next(0).expect("projects");
        s.cursor_step(true, 0).expect("notes");
        s.next(0).expect("agents");
        assert!(s.options().len() == 1 && s.text() == Some(mesimon_core::brief::TEXT));
        s.next(0).expect("brief");
        assert!(s.ready());
        s.cursor_step(true, 0).expect("tour");
        let o = s.outcome();
        assert_eq!(o.template, Template::PullRequestsProduction);
        assert_eq!(o.noun, "MR");
        assert_eq!(o.merge_into, MergeInto::PullRequests);
        assert_eq!(o.workspace, WorkspaceStrategy::SharedCheckout);
        assert_eq!(o.agent_tools, AgentTools::Annotate);
        assert!(o.auto_merge_off && o.tour);
        // Back keeps every answer.
        s.back(0).expect("back");
        assert_eq!(s.ask(), Some(Ask::Brief));
        s.back(0).expect("back");
        assert_eq!(s.options()[s.cursor].key, "notes");
    }

    /// `MESIMON_SETUP_DUMP=<page>[,<cursor>[,team]] cargo test -p mesimon-tui
    /// dump_a_page -- --nocapture` prints the page's practice board after
    /// every beat.
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
        s.next(0).expect("next");
        s.go(at, 0).expect("go");
        s.cursor = cursor;
        s.replay(0).expect("replay");
        let mut term = Terminal::new(TestBackend::new(110, 16)).expect("canvas");
        let mut now = 0;
        let mut beat = 0;
        while s.player.ended.is_none() && beat < 80 {
            let typing =
                matches!(s.beats.get(s.player.step), Some(Beat::Type(_))) && s.player.typed > 0;
            now = s.player.due.max(now);
            s.tick(now).expect("tick");
            if typing {
                continue;
            }
            term.draw(|g| crate::ui::draw(g, &s.sandbox)).expect("draw");
            let buf = term.backend().buffer();
            println!("---- beat {beat} {:?} | status: {}", s.ask(), s.sandbox.status);
            for y in 0..buf.area.height {
                let row: String =
                    (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
                println!("{}", row.trim_end());
            }
            beat += 1;
        }
    }
}
