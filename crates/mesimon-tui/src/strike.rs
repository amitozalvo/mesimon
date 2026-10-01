//! The crown's lightning (T-544): a bolt from the crowned card to the card
//! the crown just touched, and the clock the touched card's landing reads.
//!
//! A touch off the snapshot (`CrownTouch`) becomes a `Strike` the moment
//! this board first sees it, so every phase counts from what the person saw
//! and not from the daemon's stamp. The leader runs from the crown's `♛` to
//! the touched card in `LEADER_MS`, a stepped leader that jumps rather than
//! slides; the bolt lands, the return stroke burns the whole channel white,
//! flickers once, and the channel cools into the ground by `BOLT_MS`. The
//! card's title lands on the same clock (`Theme::crown_land`): the word for
//! what was done appears as the bolt arrives, not before it.
//!
//! The bolt is drawn in braille, two dots by four to a cell, and never over
//! a letter: its dots go in the board's blank cells, not in the gap between
//! two words, and every cell its channel crosses takes a faint glow of the
//! crown's tint behind whatever is written there — so it reads unbroken,
//! passing behind the cards it crosses, and every title stays legible.
//! Drawn over the letters, or between them, a title read as corrupted
//! rather than struck. It arcs over the board rather than running straight,
//! so a crown and a card on one row are joined over the row, and it lands
//! from above on the struck title's first letter, where the landing's light
//! starts. Neither the struck title's row nor the one the bolt leaves from
//! is drawn on — a bolt threading the crown's own words read as noise — and
//! the crown's mark flares as it fires. Nothing is drawn in mono, where the
//! glyph tier has no braille, or when the person turned it off
//! (`Prefs::crown_lightning`).
//!
//! It is status, never a demand: the crown's tint and the bright ink of the
//! cursor's ramp, never `attn`.

use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::{LandKind, Theme, CROWN_FLASH_MS, CROWN_LIT_MS, LAND_FADE_MS, LAND_SWEEP_MS};

/// The leader's run from the crown to the card. The touch lands at its end.
pub(crate) const LEADER_MS: u64 = 240;
/// The leader advances in this many jumps.
const LEADER_STEPS: f32 = 6.0;
/// The return stroke: the whole channel at its hottest.
const STROKE_MS: u64 = 90;
/// Then one flicker: dark for this long, and hot again for as long.
const FLICKER_MS: u64 = 60;
/// The channel has cooled into the ground by now, counted from the strike.
pub(crate) const BOLT_MS: u64 = 800;
/// A burnt card leaves its column this long after its title is gone.
const BURN_TAIL_MS: u64 = 120;
/// How far the arc bows from the straight line, against its length.
const BEND: f32 = 0.22;
/// How far each fractal step may push its midpoint, against its length.
const ROUGH: f32 = 0.28;

/// One crown touch as this board animates it.
#[derive(Debug, Clone)]
pub(crate) struct Strike {
    pub target: ulid::Ulid,
    /// Whose card the bolt leaves from: the touch's own `from`, which is
    /// the crown — or the worker, for the `woke` a worker's news gives it.
    pub from: Option<ulid::Ulid>,
    pub action: String,
    /// The daemon's stamp: with `target`, the touch's identity.
    pub at_ms: u64,
    /// When this board first saw it, in Unix ms: every phase counts here.
    pub seen: u64,
}

impl Strike {
    pub(crate) fn new(touch: &mesimon_core::command::CrownTouch, seen: u64) -> Self {
        Strike {
            target: touch.ticket,
            from: touch.from,
            action: touch.action.clone(),
            at_ms: touch.at_ms,
            seen,
        }
    }

    pub(crate) fn kind(&self) -> LandKind {
        LandKind::of(&self.action)
    }

    /// Ms since the bolt landed at `now`; negative while it is on its way.
    pub(crate) fn landed(&self, now: u64) -> i64 {
        now as i64 - (self.seen + LEADER_MS) as i64
    }

    /// The landing's front: the crowning's own sweep where the bolt lands
    /// on the crown (`woke`), the landing's elsewhere.
    fn sweep_ms(&self) -> u64 {
        if self.action == "woke" {
            CROWN_FLASH_MS
        } else {
            LAND_SWEEP_MS
        }
    }

    /// Is any of it moving at `now` — the bolt, the landing's front, or
    /// the lit title easing back? The frame loop runs fast while it is.
    pub(crate) fn moving(&self, now: u64) -> bool {
        let landed = self.landed(now);
        now.saturating_sub(self.seen) < BOLT_MS
            || (0..self.sweep_ms() as i64).contains(&landed)
            || ((CROWN_LIT_MS - LAND_FADE_MS) as i64..CROWN_LIT_MS as i64).contains(&landed)
    }

    /// Nothing of it is drawn any more.
    pub(crate) fn over(&self, now: u64) -> bool {
        self.landed(now) >= CROWN_LIT_MS as i64
    }

    /// An archived card still burning in its column at `now`.
    pub(crate) fn burning(&self, now: u64) -> bool {
        self.kind() == LandKind::Burn && self.landed(now) < (LAND_SWEEP_MS + BURN_TAIL_MS) as i64
    }

    /// The bolt's shape: one per touch, the same on every frame.
    fn seed(&self) -> u64 {
        let id = u128::from(self.target);
        (id as u64) ^ ((id >> 64) as u64) ^ self.at_ms.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }
}

/// Where the last draw put each card and column, for the bolt to join:
/// a fact of the frame, like `App::cursor_card`. `ui::draw` clears it.
#[derive(Debug, Default)]
pub(crate) struct Spots {
    pub cards: Vec<CardSpot>,
    /// A cell on each drawn column's header row, for a bolt whose card is
    /// scrolled out of its column.
    pub heads: Vec<(String, u16, u16)>,
}

/// One card's title row on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CardSpot {
    pub id: ulid::Ulid,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    /// The crown's mark on the holder's card, its column.
    pub mark: Option<u16>,
}

impl Spots {
    fn card(&self, id: ulid::Ulid) -> Option<&CardSpot> {
        self.cards.iter().find(|c| c.id == id)
    }

    fn head(&self, column: &str) -> Option<(u16, u16)> {
        self.heads.iter().find(|(n, ..)| n == column).map(|&(_, x, y)| (x, y))
    }
}

/// One braille dot of a bolt: where (two to a cell across, four down), how
/// far along the strike it is (0 at the crown, 1 at the card), and whether
/// it is on a fork rather than the channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Dot {
    pub x: i32,
    pub y: i32,
    pub s: f32,
    pub fork: bool,
}

/// A tiny deterministic generator (SplitMix64): the bolt's shape is the
/// touch's, so it holds still from frame to frame.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in -1..1.
    fn signed(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

type Pt = (f32, f32);

fn dist(a: Pt, b: Pt) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Midpoint displacement from `a` to `b`: each half's middle pushed off
/// its line by up to `rough` of its length, `depth` times. Pushes the
/// points after `a`.
fn jag(out: &mut Vec<Pt>, a: Pt, b: Pt, rough: f32, depth: u32, rng: &mut Rng) {
    let len = dist(a, b);
    if depth == 0 || len < 3.0 {
        out.push(b);
        return;
    }
    let (nx, ny) = ((a.1 - b.1) / len, (b.0 - a.0) / len);
    let off = rng.signed() * len * rough;
    let m = ((a.0 + b.0) / 2.0 + nx * off, (a.1 + b.1) / 2.0 + ny * off);
    jag(out, a, m, rough, depth - 1, rng);
    jag(out, m, b, rough, depth - 1, rng);
}

/// The dots of a polyline, joined cell to cell (8-connected), in order.
fn trace(points: &[Pt]) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for w in points.windows(2) {
        let (mut x, mut y) = (w[0].0.round() as i32, w[0].1.round() as i32);
        let (x1, y1) = (w[1].0.round() as i32, w[1].1.round() as i32);
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = ((x1 - x).signum(), (y1 - y).signum());
        let mut err = dx + dy;
        loop {
            if out.last() != Some(&(x, y)) {
                out.push((x, y));
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }
    out
}

/// A bolt from cell `from` to cell `to`, as dots. It bows away from the
/// straight line — upward, or leftward when the line is vertical — never
/// above dot row `top * 4`, and every step of it is pushed off its line
/// by the fractal; one to three forks branch off its middle.
pub(crate) fn bolt(seed: u64, from: (u16, u16), to: (u16, u16), top: u16) -> Vec<Dot> {
    let mut rng = Rng(seed);
    let at = |(x, y): (u16, u16)| (f32::from(x) * 2.0 + 0.5, f32::from(y) * 4.0 + 1.5);
    let (a, b) = (at(from), at(to));
    let len = dist(a, b).max(1.0);
    // The arc's control point: the middle, pushed along the normal that
    // points up (or left, for a vertical line), held under the top row.
    let (mut nx, mut ny) = ((b.1 - a.1) / len, (a.0 - b.0) / len);
    if ny > 0.0 || (ny == 0.0 && nx > 0.0) {
        (nx, ny) = (-nx, -ny);
    }
    let bend = len * BEND;
    let ceiling = f32::from(top) * 4.0;
    let c = ((a.0 + b.0) / 2.0 + nx * bend, ((a.1 + b.1) / 2.0 + ny * bend).max(ceiling));
    // The fractal may push a point past the top row; it rides along it.
    let under = |pts: &mut Vec<Pt>| pts.iter_mut().for_each(|p| p.1 = p.1.max(ceiling));
    let curve = |t: f32| {
        let u = 1.0 - t;
        (
            u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
            u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
        )
    };
    let mut points = vec![a];
    for i in 1..=4 {
        let prev = *points.last().unwrap_or(&a);
        jag(&mut points, prev, curve(i as f32 / 4.0), ROUGH, 4, &mut rng);
    }
    under(&mut points);
    let main = trace(&points);
    let n = main.len().max(2) - 1;
    let mut dots: Vec<Dot> = main
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| Dot { x, y, s: i as f32 / n as f32, fork: false })
        .collect();
    // Forks: from the middle of the channel, angled off its direction there
    // and leaning down, the way a leader's branches fall.
    let forks = 1 + usize::from(len > 60.0) + usize::from(len > 120.0);
    for _ in 0..forks {
        let i = ((0.45 + 0.25 * rng.signed()) * n as f32) as usize;
        let (Some(&(x0, y0)), Some(&(x1, y1))) = (main.get(i), main.get((i + 4).min(n))) else {
            continue;
        };
        let (dx, dy) = ((x1 - x0) as f32, (y1 - y0) as f32);
        let dl = dx.hypot(dy).max(1.0);
        let turn = (0.5 + 0.35 * rng.signed().abs()) * rng.signed().signum();
        let (cs, sn) = (turn.cos(), turn.sin());
        let dir = ((dx * cs - dy * sn) / dl, ((dx * sn + dy * cs) / dl + 0.35).min(1.0));
        let flen = (len * (0.16 + 0.06 * rng.signed())).max(6.0);
        let start = (x0 as f32, y0 as f32);
        let end = (start.0 + dir.0 * flen, start.1 + dir.1 * flen);
        let mut pts = vec![start];
        jag(&mut pts, start, end, ROUGH, 3, &mut rng);
        under(&mut pts);
        let path = trace(&pts);
        let s0 = i as f32 / n as f32;
        let m = path.len().max(2) - 1;
        dots.extend(path.iter().enumerate().skip(1).map(|(j, &(x, y))| Dot {
            x,
            y,
            s: s0 + (j as f32 / m as f32) * flen / len,
            fork: true,
        }));
    }
    dots
}

/// How hot one dot is `t` ms into the strike: 2 the hottest, 1 the
/// channel's tint, 0 not there.
pub(crate) fn heat(dot: &Dot, t: u64) -> f32 {
    if t < LEADER_MS {
        // The stepped leader: the channel so far, its newest jump hot. The
        // first frame draws nothing, and the last jump is the return
        // stroke's, so the bolt touches the card as it lands.
        let reach = ((t as f32 / LEADER_MS as f32) * LEADER_STEPS).floor() / LEADER_STEPS;
        return if dot.s > reach || reach <= 0.0 {
            0.0
        } else if dot.s > reach - 1.0 / LEADER_STEPS {
            2.0
        } else if dot.fork {
            0.8
        } else {
            1.0
        };
    }
    let r = t - LEADER_MS;
    if dot.fork {
        // The forks go out with the return stroke: only the channel carries it.
        return if r < STROKE_MS { 1.0 - r as f32 / STROKE_MS as f32 } else { 0.0 };
    }
    let cool = STROKE_MS + 2 * FLICKER_MS;
    if r < STROKE_MS {
        2.0
    } else if r < STROKE_MS + FLICKER_MS {
        0.55
    } else if r < cool {
        1.8
    } else {
        let span = BOLT_MS.saturating_sub(LEADER_MS + cool).max(1);
        (1.0 - (r - cool) as f32 / span as f32).max(0.0)
    }
}

/// U+2800's bit for a dot at (column, row) inside its cell.
const BRAILLE: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Paint `bolts` (each a dot list and its age in ms) into `buf` inside
/// `area`, every bolt's dots merged cell by cell first so two crossing
/// bolts share their cells. Only a blank cell takes a dot — a letter, a
/// glyph, half of a wide character and the gap between two words stay —
/// but every cell the channel crosses takes its glow, a faint ground of
/// the crown's tint behind whatever is written there, so the bolt runs
/// unbroken behind the titles it crosses. `skip` names the cells it takes
/// neither on, and a painted cell — a tag bar, a needs-you row, a chord's
/// flash: any ground but the page's and the cursor's surface — is never
/// touched, because what is painted there is the board's structure.
pub(crate) fn paint(
    buf: &mut Buffer,
    area: Rect,
    theme: &Theme,
    bolts: &[(Vec<Dot>, u64)],
    skip: &dyn Fn(u16, u16) -> bool,
) {
    let mut cells: HashMap<(i32, i32), (u8, f32)> = HashMap::new();
    for (dots, t) in bolts {
        for d in dots {
            let h = heat(d, *t);
            if h <= 0.0 {
                continue;
            }
            // The hottest strokes are drawn two dots wide, both columns of
            // the cell, so the channel thickens as it burns.
            let cols: &[i32] = if h >= 1.8 && !d.fork { &[0, 1] } else { &[d.x.rem_euclid(2)] };
            let cell = cells.entry((d.x.div_euclid(2), d.y.div_euclid(4))).or_insert((0, 0.0));
            for &c in cols {
                cell.0 |= BRAILLE[d.y.rem_euclid(4) as usize][c as usize];
            }
            cell.1 = cell.1.max(h);
        }
    }
    for ((cx, cy), (bits, h)) in cells {
        let (Ok(x), Ok(y)) = (u16::try_from(cx), u16::try_from(cy)) else { continue };
        if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() || skip(x, y) {
            continue;
        }
        let bg = buf[(x, y)].bg;
        if bg != Color::Reset && Some(bg) != theme.bg && Some(bg) != theme.selected_bg {
            continue;
        }
        let ground = match buf[(x, y)].bg {
            c @ Color::Rgb(..) => Some(c),
            _ => theme.bg,
        };
        let glow = theme.bolt_glow(h, ground);
        // Blank, and not a word's gap — a blank between two letters is
        // part of the words it parts — nor the cell under a wide
        // character's second half.
        let blank = |x: u16| buf[(x, y)].symbol() == " ";
        let left = (x > area.x).then(|| x - 1);
        let right = (x + 1 < area.right()).then_some(x + 1);
        if !blank(x)
            || left.is_some_and(|l| buf[(l, y)].symbol().width() > 1)
            || (left.is_some_and(|l| !blank(l)) && right.is_some_and(|r| !blank(r)))
        {
            if let Some(g) = glow {
                buf[(x, y)].set_bg(g);
            }
            continue;
        }
        let Some(ink) = theme.bolt_ink(h, ground) else { continue };
        let Some(ch) = char::from_u32(0x2800 + u32::from(bits)) else { continue };
        let style = Style::default().fg(ink).remove_modifier(Modifier::all());
        buf[(x, y)].set_char(ch).set_style(match glow {
            Some(g) => style.bg(g),
            None => style,
        });
    }
}

/// The board's bolts, over the columns `area` this frame drew: after the
/// columns and before any dialog, so a dialog covers them.
pub(crate) fn draw(f: &mut Frame, app: &App, area: Rect) {
    if !app.motion() {
        return;
    }
    let now = mesimon_core::clock::now_ms();
    let spots = app.spots.borrow();
    let theme = &app.theme;
    let mut bolts: Vec<(Vec<Dot>, u64)> = Vec::new();
    // The title rows no bolt draws on — the struck card's and the one it
    // leaves from — as (y, x, width), and the crown marks that flare.
    let mut rows: Vec<(u16, u16, u16)> = Vec::new();
    let mut marks: Vec<((u16, u16), u64)> = Vec::new();
    for s in &app.strikes {
        let t = now.saturating_sub(s.seen);
        if t >= BOLT_MS {
            continue;
        }
        let Some(to) = spots.card(s.target) else { continue };
        let Some(from_id) = s.from.or(app.board.crown) else { continue };
        let from = match spots.card(from_id) {
            Some(c) => {
                let x = c.mark.unwrap_or(c.x + crate::tags::BAR_WIDTH as u16 + 1);
                if c.mark.is_some() {
                    marks.push(((x, c.y), t));
                }
                rows.push((c.y, c.x, c.width));
                (x, c.y)
            }
            // A card scrolled out of its column fires from the header.
            None => match app.board.ticket(from_id).and_then(|t| spots.head(&t.column)) {
                Some(head) => head,
                None => continue,
            },
        };
        let hit = (to.x + crate::tags::BAR_WIDTH as u16 + 1, to.y);
        if from == hit {
            continue;
        }
        rows.push((to.y, to.x, to.width));
        bolts.push((bolt(s.seed(), from, hit, area.y), t));
    }
    if bolts.is_empty() {
        return;
    }
    let skip = |x: u16, y: u16| rows.iter().any(|&(ry, rx, rw)| y == ry && x >= rx && x < rx + rw);
    paint(f.buffer_mut(), area, theme, &bolts, &skip);
    // The crown flares as it fires, through the return stroke.
    for ((x, y), t) in marks {
        if t < LEADER_MS + STROKE_MS + 2 * FLICKER_MS {
            if let Some(ink) = theme.bolt_ink(2.0, None) {
                f.buffer_mut()[(x, y)].set_fg(ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adjacent(a: &Dot, b: &Dot) -> bool {
        (a.x - b.x).abs() <= 1 && (a.y - b.y).abs() <= 1
    }

    /// The channel runs from the crown's cell to the card's, unbroken and
    /// in order, and the same touch draws the same bolt every frame.
    #[test]
    fn the_bolt_joins_the_crown_to_the_card() {
        for (from, to) in [
            ((5, 3), (70, 12)),
            ((70, 12), (5, 3)),
            ((40, 4), (90, 4)),
            ((40, 20), (41, 2)),
            ((10, 2), (10, 25)),
            ((3, 2), (4, 3)),
        ] {
            for seed in 0..40u64 {
                let dots = bolt(seed, from, to, 0);
                let main: Vec<&Dot> = dots.iter().filter(|d| !d.fork).collect();
                let (first, last) = (main[0], main[main.len() - 1]);
                assert_eq!((first.x / 2, first.y / 4), (from.0 as i32, from.1 as i32));
                assert_eq!((last.x / 2, last.y / 4), (to.0 as i32, to.1 as i32));
                assert!(main.windows(2).all(|w| adjacent(w[0], w[1])), "{from:?} {to:?} {seed}");
                assert!(main.windows(2).all(|w| w[0].s <= w[1].s), "in order");
                assert!(dots.iter().all(|d| d.y >= 0), "never above the top row");
                assert_eq!(bolt(seed, from, to, 0), dots, "deterministic");
            }
        }
        // A crown and a card on one row are joined over the row, not along it.
        let dots = bolt(7, (40, 6), (90, 6), 0);
        let above = dots.iter().filter(|d| !d.fork && d.y / 4 < 6).count();
        let along = dots.iter().filter(|d| !d.fork && d.y / 4 == 6).count();
        assert!(above > along * 3, "the arc rides over the row: {above} above, {along} on it");
    }

    /// The leader reaches further at every jump and only the newest jump
    /// is hot; the return stroke lights the whole channel; then it cools
    /// and is gone at `BOLT_MS`.
    #[test]
    fn the_leader_strikes_then_the_channel_cools_away() {
        let dots = bolt(3, (5, 3), (70, 12), 0);
        let lit = |t: u64| dots.iter().filter(|d| !d.fork && heat(d, t) > 0.0).count();
        let hot = |t: u64| dots.iter().filter(|d| !d.fork && heat(d, t) >= 2.0).count();
        let main = dots.iter().filter(|d| !d.fork).count();
        assert_eq!(lit(0), 0, "the first frame draws nothing");
        let mut reach = 0;
        for t in (0..LEADER_MS).step_by(10) {
            let now = lit(t);
            assert!(now >= reach, "the leader never pulls back: {reach} then {now} at {t}");
            assert!(now < main, "the leader has not landed at {t}");
            let first = t < 2 * LEADER_MS / LEADER_STEPS as u64;
            assert!(hot(t) < now || first, "only the newest jump is hot at {t}");
            reach = now;
        }
        assert_eq!(lit(LEADER_MS), main, "the return stroke lights the whole channel");
        assert_eq!(hot(LEADER_MS), main);
        assert!(lit(LEADER_MS + STROKE_MS + 10) == main && hot(LEADER_MS + STROKE_MS + 10) == 0);
        let cooling: Vec<f32> = (LEADER_MS + STROKE_MS + 2 * FLICKER_MS..BOLT_MS)
            .step_by(20)
            .map(|t| heat(dots.iter().find(|d| !d.fork).expect("a dot"), t))
            .collect();
        assert!(cooling.windows(2).all(|w| w[0] >= w[1]), "it only cools: {cooling:?}");
        assert_eq!(lit(BOLT_MS), 0, "gone");
        assert!(dots.iter().filter(|d| d.fork).all(|d| heat(d, LEADER_MS + STROKE_MS) == 0.0));
    }

    /// Painted, the bolt is braille in the bolt's inks and nothing else: only
    /// blank cells, none it may not take, no wide character cut, never
    /// `attn`.
    #[test]
    fn the_bolt_paints_braille_and_leaves_what_it_must() {
        use crate::theme::{Flavor, Profile};
        for flavor in Flavor::ALL {
            for profile in [Profile::TrueColor, Profile::Ansi256, Profile::Mono] {
                let theme = Theme::new(flavor, profile);
                let area = Rect::new(0, 0, 100, 30);
                let mut buf = Buffer::empty(area);
                buf.set_string(30, 9, "漢字 title", Style::default());
                // A row of words across the bolt's whole path.
                buf.set_string(0, 12, "xx ".repeat(33), Style::default());
                let dots = bolt(11, (5, 3), (70, 20), 0);
                let skip = |x: u16, y: u16| y == 20 || (x, y) == (5, 3);
                paint(&mut buf, area, &theme, &[(dots, LEADER_MS + 10)], &skip);
                let mut drawn = 0;
                for y in 0..30 {
                    for x in 0..100 {
                        let c = &buf[(x, y)];
                        let Some(ch) = c.symbol().chars().next() else { continue };
                        if !(0x2800..=0x28FF).contains(&(ch as u32)) {
                            continue;
                        }
                        drawn += 1;
                        assert!(y != 20 && (x, y) != (5, 3), "{flavor:?}: a skipped cell");
                        assert_ne!(c.fg, theme.attn, "{flavor:?}/{profile:?}: attn");
                        assert!(c.modifier.is_empty(), "no SGR on the bolt");
                    }
                }
                assert_eq!(buf[(30, 9)].symbol(), "漢", "a wide character is never cut");
                let words = (0..99).map(|x| buf[(x, 12)].symbol()).collect::<String>();
                assert_eq!(words, "xx ".repeat(33), "letters and the gaps between them stay");
                if profile == Profile::Mono {
                    assert_eq!(drawn, 0, "mono draws no bolt");
                } else {
                    assert!(drawn > 20, "{flavor:?}/{profile:?}: the bolt is there ({drawn})");
                }
            }
        }
    }
}
