//! The idle snake (T-823, T-870): a snake in the board's unused cells that
//! comes out on its own after an hour with no key and plays itself, until
//! `^g` hands it to the person or `esc` stops it. The rendered cards are
//! solid terrain, never erased. Nothing here calls a transport, edits a
//! board or writes a preference. Live terrain changes preserve the round,
//! moving only pieces the board would cover.
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use mesimon_core::keymap::Verb;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Paragraph, Frame};

use crate::app::App;

/// How long the board waits for a key, a paste or a click before the snake
/// comes out (`App::idle_after`'s default).
pub(crate) const IDLE_AFTER: Duration = Duration::from_secs(3600);

/// How long a round the cpu lost stays on the board before it starts over.
const REPLAY_AFTER: Duration = Duration::from_secs(2);

type Point = (i32, i32);

const DIRS: [Point; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Field {
    area: Rect,
    walls: Vec<bool>,
    cards: Vec<(ulid::Ulid, Rect)>,
}

impl Field {
    fn index(&self, (x, y): Point) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.area.width as i32 && y < self.area.height as i32)
            .then(|| y as usize * self.area.width as usize + x as usize)
    }

    fn free(&self, p: Point) -> bool {
        self.index(p).is_some_and(|i| !self.walls[i])
    }

    fn points(&self) -> impl Iterator<Item = Point> + '_ {
        (0..self.area.height as i32).flat_map(|y| (0..self.area.width as i32).map(move |x| (x, y)))
    }

    fn component(&self, start: Point, avoid: &[Point]) -> Vec<Point> {
        let mut seen = vec![false; self.walls.len()];
        let mut todo = VecDeque::from([start]);
        let mut out = Vec::new();
        while let Some(p) = todo.pop_front() {
            let Some(i) = self.index(p) else { continue };
            if seen[i] || !self.free(p) || avoid.contains(&p) {
                continue;
            }
            seen[i] = true;
            out.push(p);
            for d in DIRS {
                todo.push_back(add(p, d));
            }
        }
        out
    }

    fn largest_component(&self) -> Vec<Point> {
        let mut visited = HashSet::new();
        let mut largest = Vec::new();
        for p in self.points() {
            if !self.free(p) || visited.contains(&p) {
                continue;
            }
            let part = self.component(p, &[]);
            visited.extend(part.iter().copied());
            if part.len() > largest.len() {
                largest = part;
            }
        }
        largest
    }

    /// The shortest walk from `from` to `to` through free cells none of
    /// `blocked` holds (`from` itself may be): the cells after `from`, `to`
    /// last. `Some(empty)` when they are one cell.
    fn path(&self, from: Point, to: Point, blocked: &[Point]) -> Option<Vec<Point>> {
        let start = self.index(from)?;
        let mut shut = vec![false; self.walls.len()];
        for &p in blocked {
            if let Some(i) = self.index(p) {
                shut[i] = true;
            }
        }
        let mut came: Vec<Option<usize>> = vec![None; self.walls.len()];
        let mut seen = vec![false; self.walls.len()];
        seen[start] = true;
        let mut todo = VecDeque::from([from]);
        while let Some(p) = todo.pop_front() {
            if p == to {
                let mut out = Vec::new();
                let mut at = self.index(p)?;
                while at != start {
                    let w = self.area.width as usize;
                    out.push(((at % w) as i32, (at / w) as i32));
                    at = came[at]?;
                }
                out.reverse();
                return Some(out);
            }
            let here = self.index(p)?;
            for d in DIRS {
                let n = add(p, d);
                let Some(i) = self.index(n) else { continue };
                if seen[i] || self.walls[i] || shut[i] {
                    continue;
                }
                seen[i] = true;
                came[i] = Some(here);
                todo.push_back(n);
            }
        }
        None
    }
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

#[derive(Clone, Debug, PartialEq)]
struct Snake {
    body: Vec<Point>,
    dir: Point,
    turns: VecDeque<Point>,
    food: Option<Point>,
}

impl Snake {
    /// The cpu's next direction: the shortest way to the food when the
    /// snake can still reach its own tail after eating there (so the
    /// snack never walls it in), else the step that leaves the longest way
    /// round to the tail (room to keep moving), else any free step. The
    /// neck is in every blocked set, so no answer reverses.
    fn cpu_dir(&self, field: &Field) -> Option<Point> {
        let head = self.body[0];
        let len = self.body.len();
        let dir_to = |p: Point| (p.0 - head.0, p.1 - head.1);
        if let Some(food) = self.food {
            if let Some(path) = field.path(head, food, &self.body[..len - 1]) {
                if let Some(&first) = path.first() {
                    // The body once the walk is done and the snack eaten:
                    // the walk, head first, then what is left of today's.
                    let mut body: Vec<Point> = path.iter().rev().copied().collect();
                    body.extend_from_slice(&self.body);
                    body.truncate(len + 1);
                    let tail = body[body.len() - 1];
                    if field.path(body[0], tail, &body[..body.len() - 1]).is_some() {
                        return Some(dir_to(first));
                    }
                }
            }
        }
        // Chase the tail, the long way: the current heading first, so a
        // tie keeps the snake straight.
        let mut dirs = DIRS.to_vec();
        dirs.sort_by_key(|&d| d != self.dir);
        let mut best: Option<(usize, Point)> = None;
        for d in &dirs {
            let p = add(head, *d);
            if !field.free(p) || self.body[..len - 1].contains(&p) {
                continue;
            }
            let mut body = vec![p];
            body.extend_from_slice(&self.body[..len - 1]);
            let tail = body[body.len() - 1];
            if let Some(path) = field.path(p, tail, &body[..body.len() - 1]) {
                if best.is_none_or(|(n, _)| path.len() > n) {
                    best = Some((path.len(), *d));
                }
            }
        }
        if let Some((_, d)) = best {
            return Some(d);
        }
        dirs.into_iter().find(|&d| {
            let p = add(head, d);
            field.free(p) && !self.body[..len - 1].contains(&p)
        })
    }
}

#[derive(Debug)]
pub(crate) struct Arcade {
    field: Field,
    snake: Option<Snake>,
    /// The cpu holds the snake; `^g` flips it.
    cpu: bool,
    paused: bool,
    over: bool,
    /// When the round was lost, for the cpu's replay.
    over_at: Option<Instant>,
    ready: bool,
    message: &'static str,
    score: u32,
    best: u32,
    rng: u64,
    last_step: Instant,
    attention: HashSet<ulid::Ulid>,
}

impl Arcade {
    /// A snake the cpu plays, from the first frame.
    pub(crate) fn new(attention: HashSet<ulid::Ulid>) -> Self {
        Self {
            field: Field::default(),
            snake: None,
            cpu: true,
            paused: true,
            over: false,
            over_at: None,
            ready: false,
            message: "",
            score: 0,
            best: 0,
            rng: mesimon_core::clock::now_ms().max(1),
            last_step: Instant::now(),
            attention,
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_for_test(&mut self, seed: u64) {
        self.rng = seed.max(1);
        self.food();
    }

    pub(crate) fn playing(&self) -> bool {
        self.ready && !self.paused && !self.over
    }

    /// Whether the cpu holds the snake (the steering keys are nobody's).
    pub(crate) fn cpu(&self) -> bool {
        self.cpu
    }

    pub(crate) fn pause(&mut self, message: &'static str) {
        if !self.over {
            self.paused = true;
            self.message = message;
        }
    }

    pub(crate) fn observe(&mut self, attention: HashSet<ulid::Ulid>, disconnected: bool) {
        if !attention.is_subset(&self.attention) {
            self.pause("a ticket needs you");
        }
        self.attention = attention;
        if disconnected {
            self.pause("board disconnected");
        }
    }

    /// The cpu's turn of the clock: held while `hold` says why (the
    /// terminal unfocused, too small, the help card), playing otherwise —
    /// it waits for no key — and a lost round replayed after a moment. A
    /// person's snake is driven by their keys alone.
    pub(crate) fn drive(&mut self, hold: Option<&'static str>, now: Instant) {
        if !self.cpu {
            return;
        }
        if self.over {
            if self.over_at.is_some_and(|t| now.saturating_duration_since(t) >= REPLAY_AFTER) {
                self.restart(now);
            }
            return;
        }
        match hold {
            Some(why) => self.pause(why),
            None if self.paused && self.ready => {
                self.paused = false;
                self.message = "";
                self.last_step = now;
            }
            None => {}
        }
    }

    fn random(&mut self, len: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % len.max(1) as u64) as usize
    }

    fn sync(&mut self, field: Field) {
        if self.field == field {
            return;
        }
        let first = self.snake.is_none();
        let resized = self.field.area != field.area;
        let previous = std::mem::replace(&mut self.field, field);
        if first {
            self.place();
        } else {
            self.ready = self.adapt(&previous);
        }
        // The cpu's snake waits for nothing; a person's waits for their
        // space, and a hold that names a ticket keeps its words.
        if (first || resized)
            && !self.cpu
            && !matches!(self.message, "a ticket needs you" | "board disconnected")
        {
            self.pause(if first { "space to play" } else { "board resized; space to continue" });
        }
    }

    fn adapt(&mut self, previous: &Field) -> bool {
        let Some(mut snake) = self.snake.clone() else { return false };
        let head = snake.body[0];
        let next = add(head, snake.turns.front().copied().unwrap_or(snake.dir));
        // Moving cards must not kill the snake on the very next step.
        // Existing walls remain the player's responsibility.
        let protect_next = previous.free(next) && !self.field.free(next);
        let fits = |offset| {
            snake.body.iter().all(|&p| self.field.free(add(p, offset)))
                && ((offset == (0, 0) && !protect_next) || self.field.free(add(next, offset)))
        };
        if !fits((0, 0)) {
            let Some(offset) = self
                .field
                .points()
                .map(|p| (p.0 - head.0, p.1 - head.1))
                .filter(|&offset| fits(offset))
                .min_by_key(|&(x, y)| x.abs() + y.abs())
            else {
                return false;
            };
            for p in &mut snake.body {
                *p = add(*p, offset);
            }
        }
        let cells = self.field.component(snake.body[0], &snake.body[1..]);
        if !snake.food.is_some_and(|p| cells.contains(&p) && !snake.body.contains(&p)) {
            let choices: Vec<_> = cells.into_iter().filter(|p| !snake.body.contains(p)).collect();
            if choices.is_empty() {
                return false;
            }
            snake.food = Some(choices[self.random(choices.len())]);
        }
        self.snake = Some(snake);
        true
    }

    fn place(&mut self) {
        self.ready = false;
        let cells = self.field.largest_component();
        // Require some room to turn, not a one-cell corridor with food at
        // its far end and no possible return journey.
        let centre = (self.field.area.width as i32 / 2, self.field.area.height as i32 * 2 / 3);
        let start = cells
            .iter()
            .filter(|&&(x, y)| {
                (-3..=4).all(|dx| (-2..=2).all(|dy| self.field.free((x + dx, y + dy))))
            })
            .min_by_key(|&&(x, y)| (x - centre.0).abs() + (y - centre.1).abs())
            .copied();
        if let Some(p) = start.filter(|_| cells.len() >= 40) {
            self.snake = Some(Snake {
                body: vec![p, add(p, (-1, 0)), add(p, (-2, 0))],
                dir: (1, 0),
                turns: VecDeque::new(),
                food: None,
            });
            self.ready = true;
            self.food();
        } else {
            self.snake = None;
        }
    }

    fn food(&mut self) {
        let Some(s) = &self.snake else { return };
        let cells: Vec<_> = self
            .field
            .component(s.body[0], &s.body[1..])
            .into_iter()
            .filter(|p| !s.body.contains(p))
            .collect();
        let idx = self.random(cells.len());
        self.snake.as_mut().expect("snake exists while placing food").food =
            cells.get(idx).copied();
        if cells.is_empty() {
            self.lose("all snacks collected!");
        }
    }

    fn lose(&mut self, why: &'static str) {
        self.over = true;
        self.over_at = Some(Instant::now());
        self.message = if self.cpu { why } else { "round over ∙ space for another" };
    }

    pub(crate) fn key(&mut self, verb: Verb) {
        match verb {
            // The handover, either way: whoever takes the snake takes it
            // as it is, running or paused (the cpu's clock lifts a pause).
            Verb::GameTake => {
                self.cpu = !self.cpu;
                if let Some(s) = &mut self.snake {
                    s.turns.clear();
                }
                if self.over {
                    self.message =
                        if self.cpu { "round over" } else { "round over ∙ space for another" };
                } else if !self.cpu && self.paused {
                    self.message = "space to play";
                }
            }
            Verb::GamePause if !self.cpu && self.over => self.restart(Instant::now()),
            Verb::GamePause if !self.cpu && self.ready => {
                self.paused = !self.paused;
                self.message = if self.paused { "paused" } else { "" };
                self.last_step = Instant::now();
            }
            Verb::CursorLeft | Verb::CursorRight | Verb::CursorUp | Verb::CursorDown
                if !self.cpu && !self.over =>
            {
                let dir = match verb {
                    Verb::CursorLeft => (-1, 0),
                    Verb::CursorRight => (1, 0),
                    Verb::CursorUp => (0, -1),
                    _ => (0, 1),
                };
                if let Some(s) = &mut self.snake {
                    let prev = s.turns.back().copied().unwrap_or(s.dir);
                    if s.turns.len() < 2 && dir != prev && dir != (-prev.0, -prev.1) {
                        s.turns.push_back(dir);
                    }
                }
            }
            _ => {}
        }
    }

    fn restart(&mut self, now: Instant) {
        self.score = 0;
        self.over = false;
        self.over_at = None;
        self.snake = None;
        self.place();
        self.last_step = now;
        if self.cpu {
            self.paused = false;
            self.message = "";
        } else {
            self.pause("space to play");
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if !self.playing() {
            self.last_step = now;
            return;
        }
        let horizontal = self.snake.as_ref().is_some_and(|s| s.dir.1 == 0);
        let speed = 170_u64.saturating_sub((self.score / 10) as u64 * 4).max(85);
        let step_ms = if horizontal { speed } else { speed * 2 };
        if now.saturating_duration_since(self.last_step) < Duration::from_millis(step_ms) {
            return;
        }
        // A slow snapshot, suspended process or idle tab never produces
        // a catch-up burst. Time lost to work cannot consume a round.
        self.last_step = now;
        self.step();
        self.best = self.best.max(self.score);
    }

    fn step(&mut self) {
        let Some(s) = &mut self.snake else { return };
        if self.cpu {
            if let Some(d) = s.cpu_dir(&self.field) {
                s.dir = d;
            }
        } else if let Some(d) = s.turns.pop_front() {
            s.dir = d;
        }
        let p = add(s.body[0], s.dir);
        let eat = s.food == Some(p);
        let occupied = s.body.len() - usize::from(!eat);
        if !self.field.free(p) || s.body[..occupied].contains(&p) {
            self.lose("round over");
            return;
        }
        s.body.insert(0, p);
        if eat {
            self.score += 10;
            self.food();
        } else {
            s.body.pop();
        }
    }

    fn heading(&self) -> String {
        let message = if !self.ready {
            "more empty space needed; resize or esc"
        } else if self.cpu && self.playing() {
            "cpu plays"
        } else {
            self.message
        };
        format!(" Ticket Snake  {} pts  best {}  {message}", self.score, self.best)
    }
}

/// Capture terrain before drawing any effects. Protect the entire card,
/// including spaces within prose and wide-glyph continuation cells, and
/// every other nonblank cell. Game glyphs use the normal neutral ink only.
pub(crate) fn draw(f: &mut Frame, app: &App, area: Rect, heading: Rect) {
    let mut game = app.arcade.borrow_mut();
    let Some(game) = game.as_mut() else { return };
    let area =
        Rect::new(area.x, area.y.saturating_add(2), area.width, area.height.saturating_sub(2));
    let cards = {
        let spots = app.spots.borrow();
        spots
            .cards
            .iter()
            .chain(&spots.trails)
            .map(|c| (c.id, Rect::new(c.x, c.y, c.width, c.rows)))
            .collect()
    };
    let mut field =
        Field { area, walls: vec![false; area.width as usize * area.height as usize], cards };
    // Each card's cells first, once per card (T-838: asking `card_at` of
    // every cell was every cell against every card, per frame), then
    // every other inked cell.
    for (_, r) in &field.cards {
        let r = r.intersection(area);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let i = (y - area.y) as usize * area.width as usize + (x - area.x) as usize;
                field.walls[i] = true;
            }
        }
    }
    for y in 0..area.height {
        for x in 0..area.width {
            let i = y as usize * area.width as usize + x as usize;
            if !field.walls[i] {
                field.walls[i] = f.buffer_mut()[(area.x + x, area.y + y)].symbol() != " ";
            }
        }
    }
    game.sync(field);
    if game.ready {
        if let Some(s) = &game.snake {
            for (i, &p) in s.body.iter().enumerate() {
                paint(
                    f.buffer_mut(),
                    &game.field,
                    p,
                    if i == 0 { "@" } else { "o" },
                    app.theme.base(),
                );
            }
            if let Some(p) = s.food {
                paint(f.buffer_mut(), &game.field, p, "*", app.theme.base());
            }
        }
    }
    f.render_widget(Paragraph::new(game.heading()).style(app.theme.dim1()), heading);
}

fn paint(buffer: &mut Buffer, field: &Field, p: Point, glyph: &str, style: ratatui::style::Style) {
    if field.free(p) {
        let cell = &mut buffer[(field.area.x + p.0 as u16, field.area.y + p.1 as u16)];
        if cell.symbol() == " " {
            cell.set_symbol(glyph).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(w: u16, h: u16) -> Field {
        Field {
            area: Rect::new(0, 0, w, h),
            walls: vec![false; w as usize * h as usize],
            cards: vec![],
        }
    }

    /// A snake a person holds, ready and paused on a 60x20 field.
    fn game() -> Arcade {
        let mut g = Arcade::new(HashSet::new());
        g.rng = 17;
        g.sync(field(60, 20));
        g.key(Verb::GameTake);
        g
    }

    /// The cpu's snake, playing on a 60x20 field.
    fn cpu_game() -> Arcade {
        let mut g = Arcade::new(HashSet::new());
        g.rng = 17;
        g.sync(field(60, 20));
        g.drive(None, Instant::now());
        g
    }

    #[test]
    fn edges_are_solid_including_negative_coordinates() {
        let f = field(30, 12);
        for p in [(-1, 0), (0, -1), (30, 3), (3, 12)] {
            assert!(!f.free(p));
        }
        assert!(f.free((29, 11)));
    }

    #[test]
    fn a_new_snake_is_the_cpus_and_plays_without_a_key() {
        let g = cpu_game();
        assert!(g.cpu());
        assert!(g.playing());
        assert_eq!(g.heading(), " Ticket Snake  0 pts  best 0  cpu plays");
    }

    #[test]
    fn the_handover_gives_a_person_the_running_snake_and_the_cpu_takes_it_back() {
        let mut g = cpu_game();
        g.key(Verb::GameTake);
        assert!(!g.cpu());
        assert!(g.playing(), "the person gets the snake as it runs");
        g.key(Verb::GamePause);
        assert!(!g.playing());
        assert_eq!(g.message, "paused");
        g.key(Verb::GameTake);
        assert!(g.cpu());
        assert!(!g.playing(), "as it was left");
        g.drive(None, Instant::now());
        assert!(g.playing(), "and the clock lifts the pause");
    }

    #[test]
    fn the_cpu_survives_a_long_open_round_and_eats() {
        let mut g = cpu_game();
        let mut best_len = 3;
        for _ in 0..4000 {
            g.step();
            assert!(!g.over, "the cpu lost at {} pts: {:?}", g.score, g.snake);
            best_len = best_len.max(g.snake.as_ref().unwrap().body.len());
        }
        assert!(g.score >= 100, "{} pts", g.score);
        assert!(best_len >= 13);
    }

    #[test]
    fn the_cpu_survives_around_cards() {
        let mut g = cpu_game();
        let mut f = field(60, 20);
        for (x0, y0) in [(5, 3), (25, 8), (45, 2), (15, 14)] {
            f.cards.push((ulid::Ulid(x0 as u128), Rect::new(x0, y0, 10, 4)));
            for x in x0..x0 + 10 {
                for y in y0..y0 + 4 {
                    let i = f.index((x as i32, y as i32)).unwrap();
                    f.walls[i] = true;
                }
            }
        }
        g.sync(f);
        assert!(g.playing());
        for _ in 0..3000 {
            g.step();
            assert!(!g.over, "the cpu lost at {} pts: {:?}", g.score, g.snake);
        }
        assert!(g.score >= 50, "{} pts", g.score);
    }

    #[test]
    fn the_cpu_never_reverses_into_its_neck() {
        let mut g = cpu_game();
        for _ in 0..500 {
            let s = g.snake.as_ref().unwrap();
            let before = s.dir;
            g.step();
            let after = g.snake.as_ref().unwrap().dir;
            assert_ne!(after, (-before.0, -before.1));
        }
    }

    #[test]
    fn a_lost_cpu_round_replays_after_a_moment_from_zero() {
        let mut g = cpu_game();
        g.score = 70;
        g.best = 70;
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(0, 5), (1, 5), (2, 5)];
        s.dir = (-1, 0);
        s.food = None;
        // Boxed in: nothing free around the head.
        for p in [(0, 4), (0, 6)] {
            let i = g.field.index(p).unwrap();
            g.field.walls[i] = true;
        }
        g.step();
        assert!(g.over);
        assert_eq!(g.message, "round over");
        let t = g.over_at.unwrap();
        g.drive(None, t + Duration::from_millis(500));
        assert!(g.over, "not yet");
        g.drive(None, t + REPLAY_AFTER);
        assert!(!g.over);
        assert!(g.playing());
        assert_eq!(g.score, 0);
        assert_eq!(g.best, 70, "the best survives the replay");
    }

    #[test]
    fn a_persons_lost_round_waits_for_space() {
        let mut g = game();
        g.key(Verb::GamePause);
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(0, 5), (1, 5), (2, 5)];
        s.dir = (-1, 0);
        g.step();
        assert!(g.over);
        assert_eq!(g.message, "round over ∙ space for another");
        g.drive(None, Instant::now() + Duration::from_secs(10));
        assert!(g.over, "the clock replays nobody's round but the cpu's");
        g.key(Verb::GamePause);
        assert!(!g.over);
        assert!(!g.playing());
        assert_eq!(g.message, "space to play");
    }

    #[test]
    fn a_hold_pauses_the_cpu_and_its_lifting_resumes() {
        let mut g = cpu_game();
        g.drive(Some("paused while away"), Instant::now());
        assert!(!g.playing());
        assert_eq!(g.message, "paused while away");
        g.drive(None, Instant::now());
        assert!(g.playing());
        // A person's pause is theirs: a lifted hold does not undo it.
        g.key(Verb::GameTake);
        g.key(Verb::GamePause);
        g.drive(None, Instant::now());
        assert!(!g.playing());
    }

    #[test]
    fn snake_buffers_two_corners_but_cannot_reverse() {
        let mut g = game();
        g.key(Verb::CursorLeft);
        assert!(g.snake.as_ref().unwrap().turns.is_empty());
        g.key(Verb::CursorUp);
        g.key(Verb::CursorLeft);
        g.key(Verb::CursorDown);
        assert_eq!(g.snake.as_ref().unwrap().turns.len(), 2);
        g.step();
        assert_eq!(g.snake.as_ref().unwrap().dir, (0, -1));
        g.step();
        assert_eq!(g.snake.as_ref().unwrap().dir, (-1, 0));
        assert!(!g.over);
    }

    #[test]
    fn the_cpus_snake_takes_no_steering() {
        let mut g = cpu_game();
        g.key(Verb::CursorUp);
        assert!(g.snake.as_ref().unwrap().turns.is_empty());
        g.key(Verb::GamePause);
        assert!(g.playing());
    }

    #[test]
    fn snake_eats_grows_and_scores_then_hits_a_wall() {
        let mut g = game();
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(5, 5), (4, 5), (3, 5)];
        s.food = Some((6, 5));
        g.step();
        assert_eq!(g.score, 10);
        assert_eq!(g.snake.as_ref().unwrap().body.len(), 4);
        let wall = g.field.index((7, 5)).unwrap();
        g.field.walls[wall] = true;
        g.step();
        assert!(g.over);
    }

    #[test]
    fn snake_can_enter_vacating_tail_but_not_its_body() {
        let mut g = game();
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(5, 5), (5, 6), (6, 6), (6, 5)];
        s.dir = (1, 0);
        s.food = Some((12, 12));
        g.step();
        assert!(!g.over);
        g.snake.as_mut().unwrap().dir = (-1, 0);
        g.step();
        assert!(g.over);
    }

    #[test]
    fn food_stays_in_the_snakes_reachable_component() {
        let mut g = game();
        for y in 0..20 {
            let i = g.field.index((30, y)).unwrap();
            g.field.walls[i] = true;
        }
        g.snake.as_mut().unwrap().body = vec![(5, 5), (4, 5), (3, 5)];
        for _ in 0..100 {
            g.food();
            let s = g.snake.as_ref().unwrap();
            let p = s.food.unwrap();
            assert!(p.0 < 30 && g.field.free(p) && !s.body.contains(&p));
        }
    }

    #[test]
    fn tiny_and_filled_boards_are_inert_not_panics() {
        for (w, h) in [(0, 0), (1, 1), (80, 20)] {
            let mut g = cpu_game();
            let mut f = field(w, h);
            f.walls.fill(true);
            g.sync(f);
            g.drive(None, Instant::now());
            g.tick(Instant::now() + Duration::from_secs(20));
            assert!(!g.playing());
            assert!(g.heading().contains("more empty space"));
        }
    }

    #[test]
    fn moving_cards_leave_unaffected_rounds_running_and_pieces_unchanged() {
        let mut g = game();
        g.score = 120;
        g.key(Verb::GamePause);
        if let Some(s) = &mut g.snake {
            s.body.push(add(*s.body.last().unwrap(), (-1, 0)));
            s.food = Some((35, 12));
            s.turns.push_back((0, -1));
        }
        let snake = g.snake.clone();
        let last_step = g.last_step;
        for y in 0..10 {
            let mut f = field(60, 20);
            f.cards.push((ulid::Ulid(1), Rect::new(0, y, 4, 2)));
            for dx in 0..4 {
                for dy in 0..2 {
                    let i = f.index((dx, y as i32 + dy)).unwrap();
                    f.walls[i] = true;
                }
            }
            g.sync(f);
            assert!(g.playing());
            assert_eq!(g.score, 120);
            assert_eq!(g.snake, snake);
            assert_eq!(g.last_step, last_step);
        }
    }

    #[test]
    fn a_card_covering_the_snake_moves_its_whole_body_without_resetting_it() {
        let mut g = game();
        g.key(Verb::GamePause);
        g.score = 80;
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(8, 8), (7, 8), (7, 9), (6, 9), (5, 9)];
        s.turns.push_back((0, -1));
        s.food = Some((40, 10));
        let before = s.clone();
        let mut f = g.field.clone();
        let i = f.index((8, 8)).unwrap();
        f.walls[i] = true;
        g.sync(f);
        assert!(g.playing());
        let s = g.snake.as_ref().unwrap();
        let offset = (s.body[0].0 - before.body[0].0, s.body[0].1 - before.body[0].1);
        assert_eq!(s.body, before.body.iter().map(|&p| add(p, offset)).collect::<Vec<_>>());
        assert!(s.body.iter().all(|&p| g.field.free(p)));
        assert_eq!(s.dir, before.dir);
        assert_eq!(s.turns, before.turns);
        assert_eq!(s.food, before.food);
        assert_eq!(g.score, 80);
        g.step();
        assert!(!g.over);
    }

    #[test]
    fn a_card_entering_the_next_snake_step_cannot_cause_an_instant_loss() {
        let mut g = game();
        g.key(Verb::GamePause);
        let s = g.snake.as_ref().unwrap();
        let next = add(s.body[0], s.dir);
        let mut f = g.field.clone();
        let i = f.index(next).unwrap();
        f.walls[i] = true;
        g.sync(f);
        g.step();
        assert!(g.playing());
        assert!(!g.over);
    }

    #[test]
    fn covered_food_moves_to_a_reachable_cell_without_moving_the_snake() {
        let mut g = game();
        g.key(Verb::GamePause);
        let before = g.snake.clone().unwrap();
        let mut f = g.field.clone();
        let i = f.index(before.food.unwrap()).unwrap();
        f.walls[i] = true;
        g.sync(f);
        let s = g.snake.as_ref().unwrap();
        assert!(g.playing());
        assert_eq!(s.body, before.body);
        assert_ne!(s.food, before.food);
        assert!(g.field.component(s.body[0], &s.body[1..]).contains(&s.food.unwrap()));
    }

    #[test]
    fn temporary_crowding_preserves_the_round_and_resumes_when_space_returns() {
        for cpu in [true, false] {
            let mut g = if cpu { cpu_game() } else { game() };
            if !cpu {
                g.key(Verb::GamePause);
            }
            let snake = g.snake.clone();
            let original = g.field.clone();
            let mut crowded = original.clone();
            crowded.walls.fill(true);
            g.sync(crowded);
            assert!(!g.playing());
            assert_eq!(g.snake, snake);
            g.sync(original);
            assert!(g.playing());
            assert_eq!(g.snake, snake);
        }
    }

    #[test]
    fn resizing_pauses_a_person_without_resetting_and_card_motion_never_unpauses() {
        let mut g = game();
        g.key(Verb::GamePause);
        let snake = g.snake.clone();
        g.sync(field(0, 0));
        assert!(!g.playing());
        g.sync(field(60, 20));
        assert_eq!(g.snake, snake);
        assert!(!g.playing());
        assert_eq!(g.message, "board resized; space to continue");
        let mut f = g.field.clone();
        f.walls[4] = true;
        g.sync(f);
        assert!(!g.playing());
    }

    #[test]
    fn resizing_holds_the_cpu_only_until_the_next_tick_of_its_clock() {
        let mut g = cpu_game();
        let snake = g.snake.clone();
        g.sync(field(0, 0));
        g.sync(field(60, 20));
        assert_eq!(g.snake, snake);
        assert!(g.playing(), "no key to wait for");
        assert_ne!(g.message, "board resized; space to continue");
    }

    #[test]
    fn new_attention_pauses_even_when_the_count_stays_the_same() {
        let mut g = game();
        g.observe(HashSet::from([ulid::Ulid(1)]), false);
        g.key(Verb::GamePause);
        g.observe(HashSet::from([ulid::Ulid(1)]), false);
        assert!(g.playing());
        g.observe(HashSet::from([ulid::Ulid(2)]), false);
        assert!(!g.playing());
        assert_eq!(g.message, "a ticket needs you");
        g.key(Verb::GamePause);
        g.observe(HashSet::from([ulid::Ulid(2)]), true);
        assert!(!g.playing());
    }

    #[test]
    fn long_stalls_do_not_fast_forward_a_round() {
        let mut g = game();
        g.key(Verb::GamePause);
        let before = g.snake.as_ref().unwrap().body[0];
        g.tick(Instant::now() + Duration::from_secs(60));
        assert_eq!(g.snake.as_ref().unwrap().body[0], add(before, (1, 0)));
        assert!(!g.over);
    }

    #[test]
    fn game_ink_never_replaces_existing_cells() {
        let f = field(30, 12);
        let mut buffer = Buffer::empty(f.area);
        buffer[(4, 4)].set_symbol("A");
        let cell = buffer[(4, 4)].clone();
        paint(&mut buffer, &f, (4, 4), "@", ratatui::style::Style::default());
        assert_eq!(buffer[(4, 4)], cell);
        paint(&mut buffer, &f, (5, 4), "@", ratatui::style::Style::default());
        assert_eq!(buffer[(5, 4)].symbol(), "@");
    }

    #[test]
    fn temporary_loss_of_space_keeps_attention_visible() {
        let mut g = game();
        g.observe(HashSet::from([ulid::Ulid(1)]), false);
        g.sync(field(3, 3));
        g.sync(field(60, 20));
        assert_eq!(g.message, "a ticket needs you");
        assert!(!g.playing());
    }

    #[test]
    fn a_path_is_the_cells_after_the_start_and_respects_the_blocked() {
        let f = field(10, 5);
        assert_eq!(f.path((0, 0), (0, 0), &[]), Some(vec![]));
        assert_eq!(f.path((0, 0), (2, 0), &[]), Some(vec![(1, 0), (2, 0)]));
        // The start may be blocked (it is the snake's own head); a cell on
        // the way may not.
        assert_eq!(f.path((0, 0), (2, 0), &[(0, 0)]), Some(vec![(1, 0), (2, 0)]));
        assert_eq!(f.path((0, 0), (2, 0), &[(1, 0)]).map(|p| p.len()), Some(4));
        let mut shut = f.clone();
        for y in 0..5 {
            let i = shut.index((5, y)).unwrap();
            shut.walls[i] = true;
        }
        assert_eq!(shut.path((0, 0), (9, 0), &[]), None);
    }
}
