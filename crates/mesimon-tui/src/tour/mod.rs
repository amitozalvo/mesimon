//! The guided tour (T-763): the menu's `Guided tour` row opens a dialog
//! with the lessons down the left and, beside them, a practice board
//! playing the one under the cursor.
//!
//! The practice board is the board: a second [`App`] on a toy daemon
//! (`toy.rs`) that keeps its world in memory. A lesson is that world's
//! first state and the keys a person would press, pressed on a clock, so
//! what the tour shows is what the keys do today and cannot drift from
//! them: a binding that changes changes its lesson with it, and a lesson
//! whose keys stop working stops working in `cargo ut`
//! (`every_lesson_plays_to_its_end`). Where an agent has a part, the
//! lesson plays it by editing the world, as the hooks would.
//!
//! Enter hands the practice board the keys (where the lesson has something
//! to try): every key is then its own, as on the real board, Esc and its
//! menu included, until it quits: q there, or its menu's last row, gives
//! the keys back to the lessons. Nothing it does leaves it: the
//! toy writes no file and starts no process, and its copy and opener are
//! the practice board's (`App::practice`).

pub(crate) mod lessons;
pub(crate) mod toy;

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use mesimon_core::keymap::Key;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Terminal;

use crate::app::App;
use crate::theme::Theme;

pub(crate) use lessons::{Lesson, LESSONS};
use toy::{Shared, Toy, World};

/// How long a pressed key's effect stays on screen before the next one.
const KEY_MS: u64 = 900;
/// One letter of typed words.
const LETTER_MS: u64 = 70;
/// How long the last frame holds before the lesson plays again.
const HOLD_MS: u64 = 3_000;

/// One beat of a lesson.
pub(crate) enum Step {
    /// A key, as a person presses it, then a pause to see what it did.
    Key(Key),
    /// Words typed a letter at a time.
    Type(&'static str),
    /// Hold still.
    Wait(u64),
    /// The world moving by itself: an agent's turn, a forge, a person
    /// elsewhere. What the hooks or the daemon would have done.
    Do(fn(&mut World)),
    /// The line under the board from here on.
    Say(&'static str),
}

/// A lesson's playback: which step is next and when.
struct Player {
    step: usize,
    /// Letters of a `Type` step already typed.
    typed: usize,
    due: u64,
    /// The last frame went up at this time; the replay starts after it.
    ended: Option<u64>,
}

pub(crate) struct Tour {
    /// The lesson under the cursor.
    pub idx: usize,
    pub sandbox: Box<App>,
    world: Shared,
    player: Option<Player>,
    /// The person has the practice board's keys.
    pub trying: bool,
    /// The lessons whose try was done, this opening.
    pub done: Vec<usize>,
    /// The caption under the board.
    pub caption: &'static str,
    /// The board's flavor, color profile and terminal rendering capabilities:
    /// the practice board uses the same rendering choices.
    theme: Theme,
    prefs: crate::prefs::Prefs,
    /// Where the practice board is drawn before it is laid on the dialog.
    pub(crate) canvas: RefCell<Option<Terminal<TestBackend>>>,
}

impl Tour {
    pub fn new(theme: &Theme, prefs: crate::prefs::Prefs, idx: usize) -> Result<Self> {
        let world = Rc::new(RefCell::new(World::new()));
        let sandbox = Box::new(App::practice(
            Box::new(Toy(world.clone())),
            theme.variant(theme.flavor),
            &prefs,
        )?);
        let mut tour = Self {
            idx,
            sandbox,
            world,
            player: None,
            trying: false,
            done: Vec::new(),
            caption: "",
            theme: theme.variant(theme.flavor),
            prefs,
            canvas: RefCell::new(None),
        };
        tour.start(idx, mesimon_core::clock::now_ms())?;
        Ok(tour)
    }

    pub fn lesson(&self) -> &'static Lesson {
        &LESSONS[self.idx.min(LESSONS.len() - 1)]
    }

    /// Lay the lesson's world out fresh and play it from its first step.
    pub fn start(&mut self, idx: usize, now: u64) -> Result<()> {
        self.idx = idx.min(LESSONS.len() - 1);
        self.trying = false;
        self.reset()?;
        self.caption = "";
        self.player = Some(Player { step: 0, typed: 0, due: now + KEY_MS / 2, ended: None });
        Ok(())
    }

    /// A fresh world and a fresh board over it, cursor on the first card.
    fn reset(&mut self) -> Result<()> {
        let mut world = World::new();
        (self.lesson().seed)(&mut world);
        world.changed = false;
        *self.world.borrow_mut() = world;
        *self.sandbox = App::practice(
            Box::new(Toy(self.world.clone())),
            self.theme.variant(self.theme.flavor),
            &self.prefs,
        )?;
        Ok(())
    }

    /// Hand the practice board the keys: the lesson's world fresh, its
    /// playback stopped, the ask under the board.
    pub fn try_it(&mut self) -> Result<()> {
        let Some(ask) = self.lesson().ask else { return Ok(()) };
        self.player = None;
        self.reset()?;
        self.sandbox.practice_copies(true);
        self.trying = true;
        self.caption = ask;
        Ok(())
    }

    /// Give the keys back and play the lesson again.
    pub fn stop_trying(&mut self, now: u64) -> Result<()> {
        self.start(self.idx, now)
    }

    /// A key while the practice board has them. `false` when the key gave
    /// them back: the practice board quit.
    pub fn key(&mut self, code: KeyCode, mods: KeyModifiers, now: u64) -> Result<bool> {
        self.sandbox.handle_key(code, mods)?;
        if std::mem::take(&mut self.sandbox.quit) {
            self.stop_trying(now)?;
            return Ok(false);
        }
        self.settle()?;
        Ok(true)
    }

    pub fn paste(&mut self, text: &str) -> Result<()> {
        self.sandbox.on_paste(text)?;
        self.settle()
    }

    /// The board fetches what changed, and a try that reached its goal is
    /// marked done.
    fn settle(&mut self) -> Result<()> {
        if std::mem::take(&mut self.world.borrow_mut().changed) {
            self.sandbox.refresh()?;
        }
        if self.trying && !self.done.contains(&self.idx) {
            let met =
                self.lesson().done.is_some_and(|done| done(&self.world.borrow(), &self.sandbox));
            if met {
                self.done.push(self.idx);
            }
        }
        Ok(())
    }

    /// Is this lesson's try done, this opening?
    pub fn is_done(&self, idx: usize) -> bool {
        self.done.contains(&idx)
    }

    /// Something on the practice board wants the next frame sooner than the
    /// spinner's cadence.
    pub fn animating(&self) -> bool {
        self.sandbox.animating()
            || self.player.as_ref().is_some_and(|p| {
                p.ended.is_none() && matches!(self.lesson().show.get(p.step), Some(Step::Type(_)))
            })
    }

    /// Play every step that is due by `now`. True when anything changed.
    pub fn tick(&mut self, now: u64) -> Result<bool> {
        let mut dirty = self.sandbox.pump()?;
        let show = self.lesson().show;
        while let Some(p) = self.player.as_mut() {
            if now < p.due {
                break;
            }
            if let Some(ended) = p.ended {
                if now >= ended + HOLD_MS {
                    self.start(self.idx, now)?;
                    dirty = true;
                    continue;
                }
                break;
            }
            let Some(step) = show.get(p.step) else {
                p.ended = Some(now);
                p.due = now + HOLD_MS;
                break;
            };
            dirty = true;
            match step {
                Step::Key(k) => {
                    p.step += 1;
                    p.due = now + KEY_MS;
                    let (code, mods) = crate::keys::from_key(*k);
                    self.sandbox.handle_key(code, mods)?;
                }
                Step::Type(words) => {
                    let Some(c) = words.chars().nth(p.typed) else {
                        p.step += 1;
                        p.typed = 0;
                        p.due = now + KEY_MS / 2;
                        continue;
                    };
                    p.typed += 1;
                    p.due = now + LETTER_MS;
                    let mods =
                        if c.is_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
                    self.sandbox.handle_key(KeyCode::Char(c), mods)?;
                }
                Step::Wait(ms) => {
                    p.step += 1;
                    p.due = now + ms;
                }
                Step::Do(change) => {
                    p.step += 1;
                    change(&mut self.world.borrow_mut());
                }
                Step::Say(words) => {
                    p.step += 1;
                    self.caption = words;
                }
            }
            self.settle()?;
        }
        if self.trying {
            self.settle()?;
        }
        Ok(dirty)
    }

    /// Play the lesson through to its last frame at once, for the tests:
    /// whether its try's goal held on any beat of the way.
    #[cfg(test)]
    pub fn play_out(&mut self) -> Result<bool> {
        let mut now = 0;
        let mut met = false;
        self.start(self.idx, now)?;
        while self.player.as_ref().is_some_and(|p| p.ended.is_none()) {
            now = self.player.as_ref().map_or(now, |p| p.due);
            self.tick(now)?;
            met |= self.lesson().done.is_some_and(|d| d(&self.world.borrow(), &self.sandbox));
        }
        Ok(met)
    }

    /// The practice board's world, for the tests.
    #[cfg(test)]
    pub fn world(&self) -> std::cell::Ref<'_, World> {
        self.world.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};

    fn tour(idx: usize) -> Tour {
        Tour::new(
            &Theme::new(Flavor::Graphite, Profile::TrueColor)
                .with_underline_colors(true)
                .with_block_gaps(false),
            Default::default(),
            idx,
        )
        .expect("tour")
    }

    #[test]
    fn every_lesson_plays_to_its_end() {
        for (i, lesson) in LESSONS.iter().enumerate() {
            let mut t = tour(i);
            let met = t.play_out().expect(lesson.title);
            // What the lesson plays is what its try asks for: playing it is
            // doing it, and nothing it presses reaches past the board.
            assert!(met || lesson.done.is_none(), "{}: playing it does not do it", lesson.title);
            assert!(t.world().refused.is_empty(), "{}: {:?}", lesson.title, t.world().refused);
            assert!(!t.caption.is_empty(), "{}: nothing said under the board", lesson.title);
        }
    }

    /// `MESIMON_TOUR_DUMP=<n> cargo test -p mesimon-tui dump_a_lesson --
    /// --nocapture` prints lesson n's practice board after every beat: how
    /// a lesson is written against what the board really does.
    #[test]
    fn dump_a_lesson() {
        let Ok(n) = std::env::var("MESIMON_TOUR_DUMP") else { return };
        let idx: usize = n.parse().expect("a lesson number");
        let mut t = tour(idx);
        let mut term = Terminal::new(TestBackend::new(90, 22)).expect("canvas");
        let mut now = 0;
        t.start(idx, now).expect("start");
        let mut beat = 0;
        while t.player.as_ref().is_some_and(|p| p.ended.is_none()) {
            let p = t.player.as_ref().expect("player");
            let typing = matches!(t.lesson().show.get(p.step), Some(Step::Type(_))) && p.typed > 0;
            now = p.due;
            t.tick(now).expect("tick");
            if typing {
                continue;
            }
            term.draw(|g| crate::ui::draw(g, &t.sandbox)).expect("draw");
            let buf = term.backend().buffer();
            println!(
                "---- beat {beat}: {} | status: {} | refused: {:?}",
                t.caption,
                t.sandbox.status,
                t.world().refused
            );
            for y in 0..buf.area.height {
                let row: String =
                    (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect();
                println!("{}", row.trim_end());
            }
            beat += 1;
        }
    }

    #[test]
    fn a_fresh_try_is_not_done_before_a_key() {
        for (i, lesson) in LESSONS.iter().enumerate() {
            let Some(done) = lesson.done else { continue };
            let mut t = tour(i);
            t.try_it().expect("try");
            assert!(t.trying);
            assert!(!done(&t.world(), &t.sandbox), "{}: done before it was tried", lesson.title);
        }
    }

    #[test]
    fn esc_on_the_practice_board_is_its_menu() {
        let i = LESSONS.iter().position(|l| l.ask.is_some()).expect("a lesson to try");
        let mut t = tour(i);
        t.try_it().expect("try");
        let kept = t.key(KeyCode::Esc, KeyModifiers::NONE, 0).expect("key");
        assert!(kept && t.trying);
        assert!(matches!(t.sandbox.mode, crate::app::Mode::Menu { .. }));
    }

    #[test]
    fn q_on_the_practice_board_goes_back_to_the_lessons_and_quits_nothing() {
        let i = LESSONS.iter().position(|l| l.ask.is_some()).expect("a lesson to try");
        let mut t = tour(i);
        t.try_it().expect("try");
        let kept = t.key(KeyCode::Char('q'), KeyModifiers::NONE, 0).expect("key");
        assert!(!kept, "q gave the keys back");
        assert!(!t.trying);
        assert!(!t.sandbox.quit);
    }

    #[test]
    fn the_practice_menu_offers_no_tour_and_a_way_back() {
        let t = tour(0);
        let rows = mesimon_core::keymap::menu_items(&t.sandbox.ctx());
        assert!(!rows.iter().any(|m| m.verb == mesimon_core::keymap::Verb::Tour));
        let quit = rows.iter().find(|m| m.verb == mesimon_core::keymap::Verb::Quit).expect("row");
        assert_eq!((quit.label)(&t.sandbox.ctx()), "Back to the tour");
    }
}
