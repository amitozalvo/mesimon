//! Small, local games in the board's unused cells (T-823). The rendered
//! cards are solid terrain, never erased bricks. No game calls a transport,
//! edits a board, or writes a preference. Live terrain changes preserve
//! the round, moving only pieces that would be covered by the board.
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use mesimon_core::keymap::Verb;
use ratatui::{buffer::Buffer, layout::Rect, widgets::Paragraph, Frame};

use crate::app::App;

type Point = (i32, i32);

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
            for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
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

    fn paddle_lanes(&self) -> Vec<(i32, i32)> {
        let row = self.area.height as i32 - 1;
        let mut runs = Vec::new();
        let mut start = None;
        for x in 0..=self.area.width as i32 {
            let open = (0..7).all(|dy| self.free((x, row - dy)));
            match (start, open) {
                (None, true) => start = Some(x),
                (Some(a), false) => {
                    if x - a >= 14 {
                        runs.push((a, x));
                    }
                    start = None;
                }
                _ => {}
            }
        }
        runs
    }

    fn in_lane(&self, lane: (i32, i32)) -> Self {
        let mut field = self.clone();
        for (i, wall) in field.walls.iter_mut().enumerate() {
            let x = (i % field.area.width as usize) as i32;
            *wall |= x < lane.0 || x >= lane.1;
        }
        field
    }

    fn card_at(&self, p: Point) -> Option<ulid::Ulid> {
        let x = p.0 + self.area.x as i32;
        let y = p.1 + self.area.y as i32;
        self.cards
            .iter()
            .find(|(_, r)| {
                x >= r.x as i32 && x < r.right() as i32 && y >= r.y as i32 && y < r.bottom() as i32
            })
            .map(|(id, _)| *id)
    }
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Breaker,
    Snake,
}

#[derive(Clone, Debug, PartialEq)]
struct Snake {
    body: Vec<Point>,
    dir: Point,
    turns: VecDeque<Point>,
    food: Option<Point>,
}

#[derive(Clone, Debug, PartialEq)]
struct Breaker {
    ball: (f64, f64),
    velocity: (f64, f64),
    paddle: i32,
    lane: (i32, i32),
    row: i32,
    bricks: Vec<Point>,
    hit_cards: HashSet<ulid::Ulid>,
    combo: u32,
    lives: u32,
    level: u32,
    launched: bool,
}

#[derive(Debug)]
pub(crate) struct Arcade {
    kind: Kind,
    field: Field,
    snake: Option<Snake>,
    breaker: Option<Breaker>,
    paused: bool,
    over: bool,
    ready: bool,
    message: &'static str,
    score: u32,
    best: [u32; 2],
    rng: u64,
    last_step: Instant,
    attention: HashSet<ulid::Ulid>,
}

impl Arcade {
    pub(crate) fn new(attention: HashSet<ulid::Ulid>) -> Self {
        Self {
            kind: Kind::Breaker,
            field: Field::default(),
            snake: None,
            breaker: None,
            paused: true,
            over: false,
            ready: false,
            message: "space to launch",
            score: 0,
            best: [0; 2],
            rng: mesimon_core::clock::now_ms().max(1),
            last_step: Instant::now(),
            attention,
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_for_test(&mut self, seed: u64) {
        self.rng = seed.max(1);
        if self.kind == Kind::Snake {
            self.food();
        }
    }

    pub(crate) fn playing(&self) -> bool {
        self.ready && !self.paused && !self.over
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
        let first = self.snake.is_none() && self.breaker.is_none();
        let resized = self.field.area != field.area;
        let previous = std::mem::replace(&mut self.field, field);
        if first {
            self.place();
        } else {
            self.ready = match self.kind {
                Kind::Snake => self.adapt_snake(&previous),
                Kind::Breaker => self.adapt_breaker(),
            };
        }
        if (first || resized)
            && !matches!(self.message, "a ticket needs you" | "board disconnected")
        {
            self.pause(if first { "space to play" } else { "board resized; space to continue" });
        }
    }

    fn adapt_snake(&mut self, previous: &Field) -> bool {
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

    fn adapt_breaker(&mut self) -> bool {
        let Some(mut breaker) = self.breaker.clone() else { return false };
        // Keep the current lane if it still fits. Newly opened space must
        // not move the paddle or reset the ball just because it is wider.
        let lanes = self.field.paddle_lanes();
        if !lanes.iter().any(|&(a, b)| a <= breaker.lane.0 && b >= breaker.lane.1) {
            let Some(lane) = lanes.into_iter().min_by_key(|&(a, b)| {
                let ball_x = breaker.ball.0.round() as i32;
                (
                    breaker.launched && !(a..b).contains(&ball_x),
                    (breaker.paddle - breaker.paddle.clamp(a, b - 5)).abs(),
                )
            }) else {
                return false;
            };
            breaker.lane = lane;
        }
        breaker.row = self.field.area.height as i32 - 1;
        breaker.paddle = breaker.paddle.clamp(breaker.lane.0, breaker.lane.1 - 5);
        let terrain = self.field.in_lane(breaker.lane);
        let ball = (breaker.ball.0.round() as i32, breaker.ball.1.round() as i32);
        if !breaker.launched {
            breaker.ball = ((breaker.paddle + 2) as f64, (breaker.row - 1) as f64);
        } else if !terrain.free(ball) || ball.1 >= breaker.row {
            let Some(p) = terrain
                .points()
                .filter(|&p| p.1 < breaker.row && terrain.free(p))
                .min_by_key(|&(x, y)| (x - ball.0).abs() + (y - ball.1).abs())
            else {
                return false;
            };
            breaker.ball = (p.0 as f64, p.1 as f64);
        }
        let start = (breaker.ball.0.round() as i32, breaker.ball.1.round() as i32);
        let reachable: HashSet<_> = terrain.component(start, &[]).into_iter().collect();
        let fits = |(x, y): Point| {
            y < breaker.row - 3 && (0..3).all(|dx| reachable.contains(&(x + dx, y)))
        };
        // Preserve every unbroken target. Only covered or disconnected
        // targets move; none are regenerated, scored, or silently removed.
        let mut bricks: Vec<_> = breaker.bricks.iter().copied().filter(|&p| fits(p)).collect();
        for &old in breaker.bricks.iter().filter(|&&p| !fits(p)) {
            let Some(p) = terrain
                .points()
                .filter(|&p| fits(p))
                .filter(|&(x, y)| !bricks.iter().any(|&(bx, by)| y == by && (x - bx).abs() < 3))
                .min_by_key(|&(x, y)| (x - old.0).abs() + (y - old.1).abs())
            else {
                return false;
            };
            bricks.push(p);
        }
        breaker.bricks = bricks;
        self.breaker = Some(breaker);
        true
    }

    fn place(&mut self) {
        self.ready = false;
        match self.kind {
            Kind::Snake => {
                let cells = self.field.largest_component();
                // Require some room to turn, not a one-cell corridor with
                // food at its far end and no possible return journey.
                let centre =
                    (self.field.area.width as i32 / 2, self.field.area.height as i32 * 2 / 3);
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
            Kind::Breaker => {
                let row = self.field.area.height as i32 - 1;
                if let Some((a, b)) =
                    self.field.paddle_lanes().into_iter().max_by_key(|(a, b)| b - a)
                {
                    let (lives, level) =
                        self.breaker.as_ref().map_or((3, 1), |b| (b.lives, b.level));
                    let paddle = (a + b - 5) / 2;
                    self.breaker = Some(Breaker {
                        ball: ((paddle + 2) as f64, (row - 1) as f64),
                        velocity: (18.0, -10.0),
                        paddle,
                        lane: (a, b),
                        row,
                        bricks: Vec::new(),
                        hit_cards: HashSet::new(),
                        combo: 0,
                        lives,
                        level,
                        launched: false,
                    });
                    self.ready = true;
                    self.bricks();
                }
                // Keep lives and wave while the terminal has too little
                // space. A later resize must not grant a fresh round.
            }
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
            self.over = true;
            self.message = "all snacks collected! r for another round";
        }
    }

    fn bricks(&mut self) {
        let Some(b) = &self.breaker else { return };
        let start = (b.ball.0.round() as i32, b.ball.1.round() as i32);
        // A crowded board can leave only one paddle lane. Keep targets
        // in the part the ball can reach without leaving that lane.
        let terrain = self.field.in_lane(b.lane);
        let reachable: HashSet<_> = terrain.component(start, &[]).into_iter().collect();
        let mut bricks = Vec::new();
        // Separate little bricks in free space, including below a full
        // upper board. Cards remain bumpers throughout every wave.
        for y in 0..b.row.saturating_sub(3) {
            if y % 3 != b.level as i32 % 3 {
                continue;
            }
            for x in 1..self.field.area.width as i32 - 2 {
                if x % 7 == 1 && (0..3).all(|dx| reachable.contains(&(x + dx, y))) {
                    bricks.push((x, y));
                }
            }
        }
        // A handful of targets leaves the live board calm enough to read.
        // Spread them through the candidate rows instead of filling one.
        let stride = bricks.len().div_ceil(18).max(1);
        self.breaker.as_mut().expect("breaker exists while placing bricks").bricks =
            bricks.into_iter().step_by(stride).collect();
    }

    pub(crate) fn key(&mut self, verb: Verb) {
        match verb {
            Verb::GameBreaker | Verb::GameSnake => {
                let kind = if verb == Verb::GameBreaker { Kind::Breaker } else { Kind::Snake };
                if kind != self.kind {
                    self.kind = kind;
                    self.restart();
                }
            }
            Verb::GameRestart => self.restart(),
            Verb::GamePause if self.ready && !self.over => {
                self.paused = !self.paused;
                self.message = if self.paused { "paused" } else { "" };
                self.last_step = Instant::now();
                if let Some(b) = &mut self.breaker {
                    b.launched = true;
                }
            }
            Verb::CursorLeft | Verb::CursorRight | Verb::CursorUp | Verb::CursorDown => {
                let dir = match verb {
                    Verb::CursorLeft => (-1, 0),
                    Verb::CursorRight => (1, 0),
                    Verb::CursorUp => (0, -1),
                    _ => (0, 1),
                };
                if self.kind == Kind::Snake && !self.over {
                    if let Some(s) = &mut self.snake {
                        let prev = s.turns.back().copied().unwrap_or(s.dir);
                        if s.turns.len() < 2 && dir != prev && dir != (-prev.0, -prev.1) {
                            s.turns.push_back(dir);
                        }
                    }
                } else if let Some(b) = &mut self.breaker {
                    b.paddle = (b.paddle + dir.0 * 3).clamp(b.lane.0, b.lane.1 - 5);
                    if !b.launched {
                        b.ball.0 = (b.paddle + 2) as f64;
                    }
                }
            }
            _ => {}
        }
    }

    fn restart(&mut self) {
        self.score = 0;
        self.over = false;
        self.breaker = None;
        self.snake = None;
        self.place();
        self.pause("space to play");
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if !self.playing() {
            self.last_step = now;
            return;
        }
        let step_ms = match self.kind {
            Kind::Breaker => 20,
            Kind::Snake => {
                let horizontal = self.snake.as_ref().is_some_and(|s| s.dir.1 == 0);
                let speed = 170_u64.saturating_sub((self.score / 10) as u64 * 4).max(85);
                if horizontal {
                    speed
                } else {
                    speed * 2
                }
            }
        };
        if now.saturating_duration_since(self.last_step) < Duration::from_millis(step_ms) {
            return;
        }
        // A slow snapshot, suspended process or idle tab never produces
        // a catch-up burst. Time lost to work cannot consume game lives.
        self.last_step = now;
        match self.kind {
            Kind::Snake => self.step_snake(),
            Kind::Breaker => self.step_breaker(step_ms as f64 / 1000.0),
        }
        let idx = usize::from(self.kind == Kind::Snake);
        self.best[idx] = self.best[idx].max(self.score);
    }

    fn step_snake(&mut self) {
        let Some(s) = &mut self.snake else { return };
        if let Some(d) = s.turns.pop_front() {
            s.dir = d;
        }
        let p = add(s.body[0], s.dir);
        let eat = s.food == Some(p);
        let occupied = s.body.len() - usize::from(!eat);
        if !self.field.free(p) || s.body[..occupied].contains(&p) {
            self.over = true;
            self.message = "round over; r to restart";
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

    fn step_breaker(&mut self, dt: f64) {
        let Some(b) = &mut self.breaker else { return };
        let old = b.ball;
        let next = (old.0 + b.velocity.0 * dt, old.1 + b.velocity.1 * dt);
        let point = |p: (f64, f64)| (p.0.round() as i32, p.1.round() as i32);
        if b.velocity.1 > 0.0 && next.1 >= b.row as f64 - 0.5 {
            if next.0 >= b.paddle as f64 - 0.5 && next.0 <= (b.paddle + 4) as f64 + 0.5 {
                // Where the ball meets the paddle aims the rebound. Even
                // the middle has a small sideways drift: no vertical trap.
                let offset = ((next.0 - (b.paddle + 2) as f64) / 2.5).clamp(-1.0, 1.0);
                let offset =
                    if offset.abs() < 0.15 { 0.15 * b.velocity.0.signum() } else { offset };
                let speed = (10.0 + b.level as f64).min(18.0);
                b.velocity = (offset * speed * 2.0, -speed);
                b.ball.1 = b.row as f64 - 0.6;
                b.hit_cards.clear();
            } else {
                b.lives -= 1;
                b.combo = 0;
                b.launched = false;
                b.ball = ((b.paddle + 2) as f64, (b.row - 1) as f64);
                if b.lives == 0 {
                    self.over = true;
                    self.message = "round over; r to restart";
                } else {
                    self.pause("ball lost; space to launch");
                }
            }
            return;
        }
        let hit = point(next);
        let brick = b.bricks.iter().position(|&(x, y)| hit.1 == y && (x..x + 3).contains(&hit.0));
        let lane_wall = |p: Point| p.0 < b.lane.0 || p.0 >= b.lane.1;
        let wall = !self.field.free(hit) || lane_wall(hit);
        if wall || brick.is_some() {
            if let Some(i) = brick {
                b.bricks.remove(i);
                b.combo += 1;
                self.score += 10 * b.combo.min(5);
            } else if let Some(id) = self.field.card_at(hit) {
                if b.hit_cards.insert(id) {
                    b.combo += 1;
                    self.score += 5 * b.combo.min(5);
                }
            }
            let xpoint = point((next.0, old.1));
            let xhit = !self.field.free(xpoint) || lane_wall(xpoint);
            let yhit = !self.field.free(point((old.0, next.1)));
            if xhit {
                b.velocity.0 = -b.velocity.0;
            }
            if yhit || !xhit || brick.is_some() {
                b.velocity.1 = -b.velocity.1;
            }
            if brick.is_some() && b.bricks.is_empty() {
                b.level += 1;
                b.ball = ((b.paddle + 2) as f64, (b.row - 1) as f64);
                b.launched = false;
                self.bricks();
                self.pause("wave cleared! space for the next");
            }
        } else {
            b.ball = next;
        }
    }

    fn heading(&self) -> String {
        let idx = usize::from(self.kind == Kind::Snake);
        let name = match self.kind {
            Kind::Snake => "Ticket Snake".to_string(),
            Kind::Breaker => self.breaker.as_ref().map_or("Board Breaker".into(), |b| {
                format!(
                    "Board Breaker  {} lives  wave {}  x{}",
                    b.lives,
                    b.level,
                    b.combo.clamp(1, 5)
                )
            }),
        };
        let message =
            if !self.ready { "more empty space needed; resize or esc" } else { self.message };
        format!(" {name}  {} pts  best {}  {message}", self.score, self.best[idx])
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
        let mut put = |p: Point, glyph: &str| {
            paint(f.buffer_mut(), &game.field, p, glyph, app.theme.base());
        };
        match game.kind {
            Kind::Snake => {
                if let Some(s) = &game.snake {
                    for (i, &p) in s.body.iter().enumerate() {
                        put(p, if i == 0 { "@" } else { "o" });
                    }
                    if let Some(p) = s.food {
                        put(p, "*");
                    }
                }
            }
            Kind::Breaker => {
                if let Some(b) = &game.breaker {
                    for &(x, y) in &b.bricks {
                        for dx in 0..3 {
                            put((x + dx, y), "=");
                        }
                    }
                    for dx in 0..5 {
                        put((b.paddle + dx, b.row), "=");
                    }
                    put((b.ball.0.round() as i32, b.ball.1.round() as i32), "o");
                }
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

    fn game(kind: Kind) -> Arcade {
        let mut g = Arcade::new(HashSet::new());
        g.kind = kind;
        g.rng = 17;
        g.sync(field(60, 20));
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
    fn snake_buffers_two_corners_but_cannot_reverse() {
        let mut g = game(Kind::Snake);
        g.key(Verb::CursorLeft);
        assert!(g.snake.as_ref().unwrap().turns.is_empty());
        g.key(Verb::CursorUp);
        g.key(Verb::CursorLeft);
        g.key(Verb::CursorDown);
        assert_eq!(g.snake.as_ref().unwrap().turns.len(), 2);
        g.step_snake();
        assert_eq!(g.snake.as_ref().unwrap().dir, (0, -1));
        g.step_snake();
        assert_eq!(g.snake.as_ref().unwrap().dir, (-1, 0));
        assert!(!g.over);
    }

    #[test]
    fn snake_eats_grows_and_scores_then_hits_a_wall() {
        let mut g = game(Kind::Snake);
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(5, 5), (4, 5), (3, 5)];
        s.food = Some((6, 5));
        g.step_snake();
        assert_eq!(g.score, 10);
        assert_eq!(g.snake.as_ref().unwrap().body.len(), 4);
        let wall = g.field.index((7, 5)).unwrap();
        g.field.walls[wall] = true;
        g.step_snake();
        assert!(g.over);
    }

    #[test]
    fn snake_can_enter_vacating_tail_but_not_its_body() {
        let mut g = game(Kind::Snake);
        let s = g.snake.as_mut().unwrap();
        s.body = vec![(5, 5), (5, 6), (6, 6), (6, 5)];
        s.dir = (1, 0);
        s.food = Some((12, 12));
        g.step_snake();
        assert!(!g.over);
        g.snake.as_mut().unwrap().dir = (-1, 0);
        g.step_snake();
        assert!(g.over);
    }

    #[test]
    fn food_stays_in_the_snakes_reachable_component() {
        let mut g = game(Kind::Snake);
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
        for kind in [Kind::Snake, Kind::Breaker] {
            for (w, h) in [(0, 0), (1, 1), (80, 20)] {
                let mut g = game(kind);
                let mut f = field(w, h);
                f.walls.fill(true);
                g.sync(f);
                g.key(Verb::GamePause);
                g.tick(Instant::now() + Duration::from_secs(20));
                assert!(!g.playing());
                assert!(g.heading().contains("more empty space"));
            }
        }
    }

    #[test]
    fn moving_cards_leave_unaffected_rounds_running_and_pieces_unchanged() {
        for kind in [Kind::Breaker, Kind::Snake] {
            let mut g = game(kind);
            g.score = 120;
            g.key(Verb::GamePause);
            if let Some(b) = &mut g.breaker {
                b.lives = 2;
                b.level = 3;
                b.combo = 4;
                b.hit_cards.insert(ulid::Ulid(7));
                b.bricks = vec![(25, 5)];
            }
            if let Some(s) = &mut g.snake {
                s.body.push(add(*s.body.last().unwrap(), (-1, 0)));
                s.food = Some((35, 12));
                s.turns.push_back((0, -1));
            }
            let snake = g.snake.clone();
            let breaker = g.breaker.clone();
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
                assert_eq!(g.breaker, breaker);
                assert_eq!(g.last_step, last_step);
            }
        }
    }

    #[test]
    fn a_card_covering_the_snake_moves_its_whole_body_without_resetting_it() {
        let mut g = game(Kind::Snake);
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
        g.step_snake();
        assert!(!g.over);
    }

    #[test]
    fn a_card_entering_the_next_snake_step_cannot_cause_an_instant_loss() {
        let mut g = game(Kind::Snake);
        g.key(Verb::GamePause);
        let s = g.snake.as_ref().unwrap();
        let next = add(s.body[0], s.dir);
        let mut f = g.field.clone();
        let i = f.index(next).unwrap();
        f.walls[i] = true;
        g.sync(f);
        g.step_snake();
        assert!(g.playing());
        assert!(!g.over);
    }

    #[test]
    fn covered_food_moves_to_a_reachable_cell_without_moving_the_snake() {
        let mut g = game(Kind::Snake);
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
    fn covered_breaker_pieces_move_without_relaunching_or_restoring_targets() {
        let mut g = game(Kind::Breaker);
        g.key(Verb::GamePause);
        g.score = 120;
        let b = g.breaker.as_mut().unwrap();
        b.ball = (10.0, 8.0);
        b.bricks = vec![(10, 4), (30, 4)];
        b.lives = 2;
        b.combo = 4;
        b.hit_cards.insert(ulid::Ulid(7));
        let before = b.clone();
        let mut f = g.field.clone();
        for p in [(10, 8), (10, 4)] {
            let i = f.index(p).unwrap();
            f.walls[i] = true;
        }
        g.sync(f);
        let b = g.breaker.as_ref().unwrap();
        assert!(g.playing());
        assert!(g.field.free((b.ball.0.round() as i32, b.ball.1.round() as i32)));
        assert_eq!(b.velocity, before.velocity);
        assert_eq!(b.paddle, before.paddle);
        assert_eq!(b.lane, before.lane);
        assert_eq!(b.lives, before.lives);
        assert_eq!(b.level, before.level);
        assert_eq!(b.combo, before.combo);
        assert_eq!(b.hit_cards, before.hit_cards);
        assert!(b.launched);
        assert_eq!(b.bricks.len(), before.bricks.len());
        assert!(b.bricks.contains(&(30, 4)));
        assert!(b.bricks.iter().all(|&(x, y)| (0..3).all(|dx| g.field.free((x + dx, y)))));
        assert_eq!(g.score, 120);
    }

    #[test]
    fn temporary_crowding_preserves_both_games_and_resumes_when_space_returns() {
        for kind in [Kind::Breaker, Kind::Snake] {
            let mut g = game(kind);
            g.key(Verb::GamePause);
            let snake = g.snake.clone();
            let breaker = g.breaker.clone();
            let original = g.field.clone();
            let mut crowded = original.clone();
            crowded.walls.fill(true);
            g.sync(crowded);
            assert!(!g.playing());
            assert_eq!(g.snake, snake);
            assert_eq!(g.breaker, breaker);
            g.sync(original);
            assert!(g.playing());
            assert_eq!(g.snake, snake);
            assert_eq!(g.breaker, breaker);
        }
    }

    #[test]
    fn a_card_covering_the_paddle_changes_its_lane_without_losing_the_ball() {
        let mut g = game(Kind::Breaker);
        g.key(Verb::GamePause);
        let b = g.breaker.as_mut().unwrap();
        b.ball = (10.0, 8.0);
        b.bricks = vec![(10, 4), (30, 4)];
        let before = b.clone();
        let mut f = g.field.clone();
        let i = f.index((before.paddle, before.row)).unwrap();
        f.walls[i] = true;
        g.sync(f);
        let b = g.breaker.as_ref().unwrap();
        assert!(g.playing());
        assert_ne!(b.lane, before.lane);
        assert!(b.paddle >= b.lane.0 && b.paddle + 5 <= b.lane.1);
        assert_eq!(b.ball, before.ball);
        assert_eq!(b.velocity, before.velocity);
        assert_eq!(b.lives, before.lives);
        assert_eq!(b.bricks.len(), before.bricks.len());
    }

    #[test]
    fn resizing_pauses_without_resetting_and_card_motion_never_unpauses() {
        for kind in [Kind::Snake, Kind::Breaker] {
            let mut g = game(kind);
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
    }

    #[test]
    fn new_attention_pauses_even_when_the_count_stays_the_same() {
        let mut g = game(Kind::Snake);
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
        let mut g = game(Kind::Snake);
        g.key(Verb::GamePause);
        let before = g.snake.as_ref().unwrap().body[0];
        g.tick(Instant::now() + Duration::from_secs(60));
        assert_eq!(g.snake.as_ref().unwrap().body[0], add(before, (1, 0)));
        assert!(!g.over);
    }

    #[test]
    fn breaker_hits_bricks_and_keeps_cards_intact() {
        let mut g = game(Kind::Breaker);
        let b = g.breaker.as_mut().unwrap();
        b.bricks = vec![(10, 5), (25, 5)];
        b.ball = (11.0, 5.6);
        b.velocity = (0.0, -10.0);
        g.step_breaker(0.02);
        assert_eq!(g.score, 10);
        assert_eq!(g.breaker.as_ref().unwrap().bricks, vec![(25, 5)]);
        assert!(g.breaker.as_ref().unwrap().velocity.1 > 0.0);
        let rect = Rect::new(10, 3, 10, 2);
        g.field.cards.push((ulid::Ulid(1), rect));
        for x in 10..20 {
            for y in 3..5 {
                let i = g.field.index((x, y)).unwrap();
                g.field.walls[i] = true;
            }
        }
        let original = g.field.clone();
        let b = g.breaker.as_mut().unwrap();
        b.ball = (11.0, 4.6);
        b.velocity = (0.0, -10.0);
        g.step_breaker(0.02);
        assert_eq!(g.score, 20);
        assert_eq!(g.field, original);
    }

    #[test]
    fn paddle_aims_the_rebound_and_missing_costs_exactly_one_life() {
        let mut g = game(Kind::Breaker);
        let b = g.breaker.as_mut().unwrap();
        b.ball = ((b.paddle + 4) as f64, b.row as f64 - 0.6);
        b.velocity = (2.0, 10.0);
        g.step_breaker(0.02);
        let b = g.breaker.as_mut().unwrap();
        assert!(b.velocity.0 > 10.0 && b.velocity.1 < 0.0);
        b.ball = (0.0, b.row as f64 - 0.6);
        b.velocity = (0.0, 10.0);
        g.step_breaker(0.02);
        assert_eq!(g.breaker.as_ref().unwrap().lives, 2);
        assert!(!g.playing());
        g.tick(Instant::now() + Duration::from_secs(10));
        assert_eq!(g.breaker.as_ref().unwrap().lives, 2);
    }

    #[test]
    fn clearing_a_wave_raises_the_level_and_waits_for_launch() {
        let mut g = game(Kind::Breaker);
        let b = g.breaker.as_mut().unwrap();
        b.bricks = vec![(10, 5)];
        b.ball = (11.0, 5.6);
        b.velocity = (0.0, -10.0);
        g.step_breaker(0.02);
        let b = g.breaker.as_ref().unwrap();
        assert_eq!(b.level, 2);
        assert!(!b.bricks.is_empty());
        assert!(!g.playing());
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
    fn temporary_loss_of_space_keeps_lives_and_attention_visible() {
        let mut g = game(Kind::Breaker);
        g.breaker.as_mut().unwrap().lives = 1;
        g.observe(HashSet::from([ulid::Ulid(1)]), false);
        g.sync(field(3, 3));
        g.sync(field(60, 20));
        assert_eq!(g.breaker.as_ref().unwrap().lives, 1);
        assert_eq!(g.message, "a ticket needs you");
        assert!(!g.playing());
    }

    #[test]
    fn breaker_can_clear_a_real_wave_with_aimed_rebounds() {
        let mut g = game(Kind::Breaker);
        // Follow the falling ball with alternating offsets, exercising the
        // real generated field rather than placing the ball on each brick.
        for step in 0..20_000 {
            let b = g.breaker.as_mut().unwrap();
            if b.level > 1 {
                break;
            }
            let aim = if step / 180 % 2 == 0 { 1 } else { -1 };
            b.paddle = (b.ball.0.round() as i32 - 2 + aim).clamp(b.lane.0, b.lane.1 - 5);
            g.step_breaker(0.02);
            assert!(!g.over, "autoplayer lost all three balls");
        }
        assert!(g.breaker.as_ref().unwrap().level > 1, "wave trapped: {:?}", g.breaker);
    }
    #[test]
    fn a_narrow_paddle_lane_has_no_unreachable_targets_or_uncatchable_side_falls() {
        let mut g = game(Kind::Breaker);
        let b = g.breaker.as_mut().unwrap();
        b.lane = (20, 40);
        b.paddle = 28;
        b.ball = (20.0, 17.0);
        b.velocity = (-20.0, -10.0);
        g.bricks();
        assert!(g.breaker.as_ref().unwrap().bricks.iter().all(|&(x, _)| (20..38).contains(&x)));
        g.step_breaker(0.04);
        let b = g.breaker.as_ref().unwrap();
        assert!(b.ball.0 >= 20.0 && b.velocity.0 > 0.0);
    }
}
