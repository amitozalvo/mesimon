//! A card's move, played (T-817). A move this board did not make — the
//! board's automove on a turn's edge, an agent's `move_ticket`, the merge
//! train, a phone, a teammate — used to be one frame: the card gone from
//! its column and already in the next, easy to miss. Now it plays out in
//! the direction it travels:
//!
//! - **out**: the card fades from where it was, along the move (left to
//!   right for a move right, right to left for a move left), down to a
//!   ghost that holds its old place;
//! - **trail**: a thin grey trail runs from there to where it went in
//!   straight lines through the gaps alone — out of the card's end into the
//!   gutter beside its column, along the gutter, along the empty row under
//!   the column headers when a column lies between, down the gutter beside
//!   the one it went to and into the card — never over a card;
//! - **in**: the card fades in where it went, along the move, and the ghost
//!   goes with that;
//! - then its row **shimmers**, the pipeline's crest, until the cursor
//!   rests on it.
//!
//! This board's own moves never play — the cursor rides those — and the
//! crown's keep their bolt (`strike.rs`). Status, never a demand: grey on
//! grey, never `attn`. `Prefs::card_moves` turns it off, and the trail is
//! braille, so a glyph tier without it plays the rest.

use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::strike::CardSpot;
use crate::theme::Theme;

/// The card fades out where it was in this long.
pub(crate) const OUT_MS: u64 = 450;
/// Then the trail runs from there to where it went in this long.
pub(crate) const TRAIL_MS: u64 = 600;
/// Then the card fades in where it went in this long, and the ghost goes.
pub(crate) const IN_MS: u64 = 450;
/// The whole move, start to landing.
pub(crate) const GLIDE_MS: u64 = OUT_MS + TRAIL_MS + IN_MS;
/// Several moves in one snapshot (the train merging three) start this far
/// apart, so each is seen to go.
pub(crate) const STAGGER_MS: u64 = 150;
/// At most this many play from one snapshot; the rest are simply where
/// they went.
pub(crate) const MAX_PLAYED: usize = 4;
/// A move this board asked for is its own for this long: the snapshot that
/// carries it does not play it.
pub(crate) const OWN_MOVE_MS: u64 = 10_000;
/// Cells over which a fade's front softens.
const EDGE: f32 = 6.0;
/// How much of its ink a faded-out card keeps while it waits as a ghost.
const GHOST: f32 = 0.3;
/// The trail's lit length, against its whole path: it travels.
const TAIL: f32 = 0.4;

/// One move this board is playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Glide {
    pub ticket: ulid::Ulid,
    /// The column it left and its order there: where the ghost stands.
    pub was: (String, String),
    /// When it starts, in Unix ms (later than the snapshot for a staggered
    /// one).
    pub start: u64,
    /// It went to a column on the right.
    pub right: bool,
}

impl Glide {
    /// The ghost still holds the old place at `now`: from the snapshot to
    /// the landing.
    pub(crate) fn ghost(&self, now: u64) -> bool {
        now < self.landed_at()
    }

    /// It has landed and nothing of the move is drawn.
    pub(crate) fn over(&self, now: u64) -> bool {
        !self.ghost(now)
    }

    pub(crate) fn landed_at(&self) -> u64 {
        self.start + GLIDE_MS
    }

    /// How far a phase that starts `from` ms in and lasts `len` has come:
    /// below 0 before it, above 1 after.
    fn phase(&self, now: u64, from: u64, len: u64) -> f32 {
        // In whole ms first: a Unix-ms stamp is past what an `f32` holds.
        (now as i64 - (self.start + from) as i64) as f32 / len as f32
    }
}

/// How far a fade's front has come over cell `i` of `cells`, counted along
/// the move, at progress `p`: 0 not reached, 1 passed, a soft edge between.
pub(crate) fn reached(p: f32, i: usize, cells: usize) -> f32 {
    let front = ease(p.clamp(0.0, 1.0)) * (cells as f32 + EDGE);
    ((front - i as f32) / EDGE).clamp(0.0, 1.0)
}

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// The trail's corners, in braille dots (two across a cell, four down):
/// from the end of `from` (the ghost's title row) to `to` (the landed
/// card's), through the gaps alone. `corridor` is the empty row under the
/// column headers, taken when a column lies between the two. Every run is
/// straight, along a card's title row only in the gap beside its column.
pub(crate) fn route(from: &CardSpot, to: &CardSpot, right: bool, corridor: u16) -> Vec<(i32, i32)> {
    let row = |y: u16| i32::from(y) * 4 + 1;
    let col = |x: i32| x * 2;
    let (fx, fw, tx, tw) =
        (i32::from(from.x), i32::from(from.width), i32::from(to.x), i32::from(to.width));
    let (exit, gs, gt, entry) =
        if right { (fx + fw - 1, fx + fw, tx - 1, tx) } else { (fx, fx - 1, tx + tw, tx + tw - 1) };
    let (ys, yt) = (row(from.y), row(to.y));
    let mut pts = vec![(col(exit), ys), (col(gs), ys)];
    if gs == gt {
        pts.push((col(gs), yt));
    } else {
        let yc = row(corridor);
        pts.extend([(col(gs), yc), (col(gt), yc), (col(gt), yt)]);
    }
    pts.push((col(entry), yt));
    pts
}

/// Every dot along `corners`, in order, one per step.
pub(crate) fn walk(corners: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for w in corners.windows(2) {
        let ((mut x, mut y), (x1, y1)) = (w[0], w[1]);
        loop {
            if out.last() != Some(&(x, y)) {
                out.push((x, y));
            }
            if (x, y) == (x1, y1) {
                break;
            }
            x += (x1 - x).signum();
            y += (y1 - y).signum();
        }
    }
    out
}

/// The dots of the trail lit at progress `p`: a stretch of the path that
/// enters at its start, travels, and leaves into the card. Indexes into
/// `walk`'s list, empty before it starts and after it ends.
pub(crate) fn lit(p: f32, dots: usize) -> std::ops::Range<usize> {
    if !(0.0..1.0).contains(&p) || dots == 0 {
        return 0..0;
    }
    let tail = (dots as f32 * TAIL).max(8.0);
    let head = ease(p) * (dots as f32 + tail);
    let lo = (head - tail).max(0.0) as usize;
    let hi = (head as usize).min(dots);
    lo.min(hi)..hi
}

/// U+2800's bit for a dot at (column, row) inside its cell.
const BRAILLE: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Is the cell at (`x`, `y`) on the page's own ground (or the cursor's
/// surface) rather than a painted one — a tag bar, a band?
fn on_ground(buf: &Buffer, theme: &Theme, x: u16, y: u16) -> bool {
    let bg = buf[(x, y)].bg;
    bg == Color::Reset || Some(bg) == theme.bg || Some(bg) == theme.selected_bg
}

/// Paint `dots` into `buf` inside `area`, merged cell by cell, in the
/// ramp's quiet grey: a blank cell on the ground alone, never a letter,
/// never a painted cell, never the half of a wide character.
fn paint_trail(buf: &mut Buffer, area: Rect, theme: &Theme, dots: &[(i32, i32)]) {
    let mut cells: HashMap<(i32, i32), u8> = HashMap::new();
    for &(x, y) in dots {
        *cells.entry((x.div_euclid(2), y.div_euclid(4))).or_default() |=
            BRAILLE[y.rem_euclid(4) as usize][x.rem_euclid(2) as usize];
    }
    for ((cx, cy), bits) in cells {
        let (Ok(x), Ok(y)) = (u16::try_from(cx), u16::try_from(cy)) else { continue };
        if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
            continue;
        }
        if buf[(x, y)].symbol() != " " || !on_ground(buf, theme, x, y) {
            continue;
        }
        if x > area.x && buf[(x - 1, y)].symbol().width() > 1 {
            continue;
        }
        let Some(ch) = char::from_u32(0x2800 + u32::from(bits)) else { continue };
        let style = Style::default().fg(theme.rest.dim2).remove_modifier(Modifier::all());
        buf[(x, y)].set_char(ch).set_style(style);
    }
}

/// Fade the cells of `rect` along the move at progress `p`. Going `out`, a
/// cell the front has passed is taken down to the ghost's ink; coming in,
/// a cell it has not reached is not there yet and one it has comes in
/// from the ground.
fn fade(buf: &mut Buffer, rect: Rect, theme: &Theme, p: f32, right: bool, out: bool) {
    let w = usize::from(rect.width);
    for y in rect.y..rect.bottom() {
        for i in 0..w {
            let x = if right { rect.x + i as u16 } else { rect.right() - 1 - i as u16 };
            let k = reached(p, i, w);
            let keep = if out { 1.0 - k * (1.0 - GHOST) } else { k };
            if keep >= 1.0 {
                continue;
            }
            let painted = !on_ground(buf, theme, x, y);
            let cell = &mut buf[(x, y)];
            if keep <= 0.0 {
                cell.set_char(' ');
                if painted {
                    cell.bg = Color::Reset;
                }
                continue;
            }
            cell.fg = theme.toward_ground(cell.fg, keep);
            if painted {
                cell.bg = theme.recede_ground_by(cell.bg, keep);
            }
        }
    }
}

impl CardSpot {
    /// The card's rows on screen, from its title row.
    pub(crate) fn rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, width: self.width, height: self.rows.max(1) }
    }
}

/// The board's moves, over the columns `area` this frame drew: after the
/// columns and the bolts, before any dialog. `quiet` names a card whose row
/// already moves (a running pipeline's words): it does not shimmer too.
pub(crate) fn draw(f: &mut Frame, app: &App, area: Rect, quiet: &dyn Fn(ulid::Ulid) -> bool) {
    if !app.prefs.card_moves {
        return;
    }
    let now = mesimon_core::clock::now_ms();
    let spots = app.spots.borrow();
    let theme = &app.theme;
    let braille = theme.glyph_tier() == crate::glyphs::Tier::Unicode;
    let buf = f.buffer_mut();
    for g in app.glides.iter().filter(|g| !g.over(now)) {
        let from = spots.trail(g.ticket);
        let to = spots.card(g.ticket);
        if let Some(from) = from {
            fade(
                buf,
                from.rect().intersection(area),
                theme,
                g.phase(now, 0, OUT_MS),
                g.right,
                true,
            );
        }
        if let Some(to) = to {
            let p = g.phase(now, OUT_MS + TRAIL_MS, IN_MS);
            fade(buf, to.rect().intersection(area), theme, p, g.right, false);
        }
        if let (Some(from), Some(to), true) = (from, to, braille) {
            let dots = walk(&route(from, to, g.right, area.y + 1));
            let on = lit(g.phase(now, OUT_MS, TRAIL_MS), dots.len());
            paint_trail(buf, area, theme, &dots[on]);
        }
    }
    for (&id, &landed) in &app.unseen_moves {
        if now < landed || quiet(id) {
            continue;
        }
        let Some(spot) = spots.card(id) else { continue };
        let row = Rect { height: 1, ..spot.rect() }.intersection(area);
        let lead = crate::tags::BAR_WIDTH as u16 + 1;
        let (x0, cells) = (row.x + lead, usize::from(row.width.saturating_sub(lead)));
        for i in 0..cells {
            let x = x0 + i as u16;
            if buf[(x, row.y)].symbol() == " " || !on_ground(buf, theme, x, row.y) {
                continue;
            }
            let ink = buf[(x, row.y)].fg;
            if let Some(style) = theme.move_shimmer(now - landed, cells, i, ink) {
                buf[(x, row.y)].set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: u16, y: u16) -> CardSpot {
        CardSpot { id: ulid::Ulid::nil(), x, y, width: 29, rows: 1, mark: None }
    }

    /// The fade's front crosses every cell along the move, softly, and has
    /// passed them all when its phase ends.
    #[test]
    fn the_front_crosses_the_card_along_the_move() {
        assert_eq!(reached(0.0, 0, 30), 0.0, "nothing before it starts");
        assert_eq!(reached(-1.0, 5, 30), 0.0);
        for i in 0..30 {
            assert_eq!(reached(1.0, i, 30), 1.0, "cell {i} is passed at the end");
        }
        for p in [0.2, 0.4, 0.6, 0.8] {
            let ks: Vec<f32> = (0..30).map(|i| reached(p, i, 30)).collect();
            assert!(ks.windows(2).all(|w| w[0] >= w[1]), "the near end leads at {p}: {ks:?}");
            assert!(ks.iter().any(|k| *k > 0.0 && *k < 1.0), "a soft edge at {p}");
        }
    }

    /// Next door, the trail goes down the gutter between the two columns
    /// and nowhere else; two columns over, it takes the row under the
    /// headers. Every run is straight.
    #[test]
    fn the_trail_runs_straight_through_the_gaps() {
        // IN PROGRESS's second card to REVIEW's first: the gutter at 60.
        let corners = route(&spot(31, 6), &spot(61, 4), true, 3);
        assert_eq!(corners, vec![(118, 25), (120, 25), (120, 17), (122, 17)]);
        // TODO's first to REVIEW's first: over IN PROGRESS, on row 3.
        let corners = route(&spot(1, 4), &spot(61, 4), true, 3);
        assert_eq!(corners, vec![(58, 17), (60, 17), (60, 13), (120, 13), (120, 17), (122, 17)]);
        // And back, leftward, from the near end of the card.
        let corners = route(&spot(61, 6), &spot(31, 4), false, 3);
        assert_eq!(corners, vec![(122, 25), (120, 25), (120, 17), (118, 17)]);
        let dots = walk(&corners);
        assert!(dots.windows(2).all(|w| {
            let (a, b) = (w[0], w[1]);
            (a.0 - b.0).abs() + (a.1 - b.1).abs() == 1
        }));
        assert_eq!(dots.first(), Some(&(122, 25)));
        assert_eq!(dots.last(), Some(&(118, 17)));
    }

    /// The lit stretch enters at the start, only moves forward, reaches the
    /// card, and is gone outside its phase.
    #[test]
    fn the_trail_travels_and_leaves() {
        assert!(lit(-0.1, 100).is_empty());
        assert!(lit(1.0, 100).is_empty());
        let mut last = 0..0;
        for step in 1..100 {
            let r = lit(step as f32 / 100.0, 100);
            assert!(r.start >= last.start && r.end >= last.end, "{last:?} then {r:?}");
            assert!(r.len() <= 40, "a stretch, never the whole path: {r:?}");
            last = r;
        }
        assert_eq!(last.end, 100, "it reaches the card");
        assert!(lit(0.02, 100).start == 0, "it enters from the start");
    }

    /// The trail paints blank cells on the ground and nothing else: a
    /// letter, a painted cell and a wide character's half all stay.
    #[test]
    fn the_trail_never_covers_a_card() {
        use crate::theme::{Flavor, Profile};
        let theme = Theme::new(Flavor::ALL[0], Profile::TrueColor);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 2, "x".repeat(40), Style::default());
        buf.set_string(10, 4, "漢", Style::default());
        buf[(20, 6)].set_bg(Color::Rgb(200, 0, 0));
        let dots: Vec<(i32, i32)> =
            (0..80).flat_map(|x| [(x, 9), (x, 17), (x, 25)]).collect::<Vec<_>>();
        paint_trail(&mut buf, area, &theme, &dots);
        assert_eq!((0..40).map(|x| buf[(x, 2)].symbol()).collect::<String>(), "x".repeat(40));
        assert_eq!(buf[(10, 4)].symbol(), "漢");
        assert!(
            !buf[(11, 4)].symbol().chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
            "the wide character's half stays"
        );
        assert_eq!(buf[(20, 6)].symbol(), " ", "a painted cell stays");
        assert_eq!(buf[(21, 6)].symbol(), "⠒", "a blank one takes the dots");
        assert_ne!(buf[(21, 6)].fg, theme.attn, "never attn");
    }

    /// A glide's ghost stands from the snapshot to the landing, a staggered
    /// one too.
    #[test]
    fn the_ghost_stands_until_the_landing() {
        let g = Glide {
            ticket: ulid::Ulid::nil(),
            was: ("IN PROGRESS".into(), "a".into()),
            start: 1_000,
            right: true,
        };
        assert!(g.ghost(900), "waiting its turn");
        assert!(g.ghost(1_000 + GLIDE_MS - 1));
        assert!(g.over(1_000 + GLIDE_MS));
        assert!(g.phase(1_000 + OUT_MS, OUT_MS, TRAIL_MS) == 0.0);
    }
}
