//! The intro (T-869): the ש in משימון takes a breath, fills out into the
//! shin, wakes, looks at its neighbours and then at you, waves and hops.
//! A person sees it once per machine: under the installer's output when it
//! installs mesimon (`mesimon welcome`, inline), or else on the first board
//! this machine opens (fullscreen, before the board). A marker file under
//! the state root says it was seen; an update is not an installation and
//! asks nothing. A terminal too narrow or too short for the word, or one
//! with no colour, plays nothing and marks nothing, so a wider one plays it.
//!
//! The letters are drawn here in the shin's own weight and rim light; the
//! shin itself is `creature.rs`'s, face and inks included.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::Terminal;

use crate::creature::{self, Anim, Eyes, Frame, Mouth, Role, Size};
use crate::theme::{CreatureInk, Theme};

// -- the word -------------------------------------------------------------------

/// The ש as a letter: three stems on a base, in the shin's weight.
const SHIN: &[&str] = &[
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "####...####...####",
    "#####..####..#####",
    "##################",
    "##################",
    "##################",
    ".################.",
    "..##############..",
];
/// מ opens at its bottom left.
const MEM: &[&str] = &[
    "####............",
    "#####..########.",
    ".#####.#########",
    "..##############",
    "..##############",
    "..####......####",
    "..####......####",
    "..####......####",
    "..####......####",
    "..####......####",
    "..####......####",
    "..####......####",
    "........########",
    ".......#########",
    ".......#########",
    ".......########.",
];
const YOD: &[&str] = &["######.", "#######", "#######", "...####", "...###.", "..###..", "..##..."];
const VAV: &[&str] = &[
    "######.", "#######", "#######", "...####", "...####", "...####", "...####", "...####",
    "...####", "...####", "...####", "...####", "...####", "...####", "...####", "...####",
];
/// ן: a ו whose stem runs below the line.
const NUN: &[&str] = &[
    "######.", "#######", "#######", "...####", "...####", "...####", "...####", "...####",
    "...####", "...####", "...####", "...####", "...####", "...####", "...####", "...####",
    "...####", "...####", "...####", "...####", "...####", "...####",
];

/// משימון as it stands on screen, left to right: ן ו מ י ש מ.
const WORD: [&[&str]; 6] = [NUN, VAV, MEM, YOD, SHIN, MEM];
const HERO: usize = 4;
const GAP: i16 = 2;
/// The shin's stage: the letter is 18 pixels wide, the shin 20, one more
/// each side for its sparkles' columns.
const LETTER_W: i16 = 18;

/// A letter as pixels, lit as the shin is lit.
struct Glyph {
    w: i16,
    h: i16,
    px: Vec<Option<Role>>,
}

impl Glyph {
    fn of(rows: &[&str]) -> Glyph {
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0) as i16;
        let h = rows.len() as i16;
        let grid: Vec<Option<Role>> = rows
            .iter()
            .flat_map(|r| (0..w as usize).map(move |x| r.as_bytes().get(x) == Some(&b'#')))
            .map(|on| on.then_some(Role::Body))
            .collect();
        Glyph { w, h, px: creature::light(&grid, w, h, 0) }
    }

    fn at(&self, x: i16, y: i16) -> Option<Role> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }

    /// Squash and stretch: resampled to `w`×`h`, nearest pixel.
    fn resample(&self, w: i16, h: i16) -> Glyph {
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = ((2 * x + 1) * self.w / (2 * w)).min(self.w - 1);
                let sy = ((2 * y + 1) * self.h / (2 * h)).min(self.h - 1);
                px.push(self.px[(sy * self.w + sx) as usize]);
            }
        }
        Glyph { w, h, px }
    }

    fn of_picture(pic: &creature::Picture) -> Glyph {
        let px = (0..pic.h).flat_map(|y| (0..pic.w).map(move |x| pic.at(x, y))).collect();
        Glyph { w: pic.w, h: pic.h, px }
    }
}

fn shin_frame(fr: &Frame) -> Glyph {
    Glyph::of_picture(&creature::compose(Size::Medium, fr, 0))
}

// -- the morph ------------------------------------------------------------------

/// The stage margin each side of the morph, so its swell has room.
const MPAD: i16 = 2;

/// The letter and the shin as distance fields on one stage (positive
/// inside). A frame of the morph is their blend: the stems swell, the gaps
/// close, the arms slide to where the shin keeps them.
struct Morph {
    w: i16,
    h: i16,
    letter: Vec<f32>,
    shin: Vec<f32>,
    top: i16,
}

fn morph() -> &'static Morph {
    static MORPH: OnceLock<Morph> = OnceLock::new();
    MORPH.get_or_init(|| {
        let letter = Glyph::of(SHIN);
        let (sw, sh, solid, top) = creature::silhouette(Size::Medium);
        let (w, h) = (sw + 2 * MPAD, sh + MPAD);
        // The letter stands a pixel in from the shin's left edge.
        let in_letter = |x: i16, y: i16| letter.at(x - MPAD - 1, y - MPAD).is_some();
        let in_shin = |x: i16, y: i16| {
            let (sx, sy) = (x - MPAD, y - MPAD);
            sx >= 0 && sy >= 0 && sx < sw && sy < sh && solid[(sy * sw + sx) as usize]
        };
        let field = |inside: &dyn Fn(i16, i16) -> bool| -> Vec<f32> {
            let mut d = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let me = inside(x, y);
                    let mut best = f32::MAX;
                    for yy in -4..h + 4 {
                        for xx in -4..w + 4 {
                            if inside(xx, yy) != me {
                                let (dx, dy) = (f32::from(xx - x), f32::from(yy - y));
                                best = best.min((dx * dx + dy * dy).sqrt());
                            }
                        }
                    }
                    d.push(if me { best - 0.5 } else { 0.5 - best });
                }
            }
            d
        };
        Morph { w, h, letter: field(&in_letter), shin: field(&in_shin), top: top + MPAD }
    })
}

/// The morph at `t`: the bottom rows lead, so the letter fills from the
/// bottom up, and past 1 the body (never the arms, whose heads would run
/// together) swells a pixel, a breath held.
fn morph_at(t: f32) -> Glyph {
    let m = morph();
    let mut grid = Vec::with_capacity((m.w * m.h) as usize);
    for y in 0..m.h {
        let lag = 0.3 * (1.0 - f32::from(y) / f32::from(m.h - 1));
        let te = ((t - lag) / 0.7).clamp(0.0, 1.0);
        let swell = if t > 1.0 && y >= m.top { 0.5 } else { 0.0 };
        for x in 0..m.w {
            let i = (y * m.w + x) as usize;
            let v = m.letter[i] + (m.shin[i] - m.letter[i]) * te + swell;
            grid.push((v > 0.0).then_some(Role::Body));
        }
    }
    Glyph { w: m.w, h: m.h, px: creature::light(&grid, m.w, m.h, m.top) }
}

// -- the storyboard ---------------------------------------------------------------

/// What the waking letter is in a beat.
#[derive(Debug, Clone, Copy)]
enum Hero {
    /// The letter, shivered `dx` and resampled to a size while it hops.
    Letter {
        dx: i16,
        size: Option<(i16, i16)>,
    },
    Morph(f32),
    /// The shin, resampled: a landing's squash or a take-off's stretch.
    Squash(Frame, (i16, i16)),
    Shin(Frame),
}

/// One beat: how long it holds and what the screen shows.
#[derive(Debug, Clone, Copy)]
struct Beat {
    ms: u32,
    /// How far the word has come up out of the ground, 0 to 1.
    fade: f32,
    hero: Hero,
    /// The hero lifted, in pixels.
    dy: i16,
    /// The neighbours nudged aside, in pixels.
    push: i16,
    sparkle: Option<u8>,
    /// How many of the closing lines are up.
    tail: u8,
    /// While true, "any key skips" stands where the next words will come.
    skip: bool,
}

const fn face(eyes: Eyes, mouth: Mouth) -> Frame {
    creature::pose(eyes, mouth, 0, false, true)
}

fn storyboard() -> Vec<Beat> {
    let mut beats = Vec::new();
    let mut s = Beat {
        ms: 0,
        fade: 0.0,
        hero: Hero::Letter { dx: 0, size: None },
        dy: 0,
        push: 0,
        sparkle: None,
        tail: 0,
        skip: true,
    };
    let mut at = |ms: u32, change: &dyn Fn(&mut Beat)| {
        change(&mut s);
        s.ms = ms;
        beats.push(s);
    };
    let letter = |dx| Hero::Letter { dx, size: None };
    let asleep = |eyes, mouth| Hero::Shin(creature::pose(eyes, mouth, 0, false, false));

    // The word comes up out of the dark, right to left as it reads.
    for f in 1..=14u8 {
        at(70, &|b| b.fade = f32::from(f) / 14.0);
    }
    at(900, &|_| {});
    // The letter stirs, shivers, and takes a breath.
    at(200, &|b| b.dy = 1);
    at(160, &|b| b.dy = 0);
    at(700, &|_| {});
    for dx in [1, -1, 1, -1, 0] {
        at(60, &|b| b.hero = letter(dx));
    }
    at(250, &|_| {});
    for (i, t) in [0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.82, 1.0, 1.1, 1.1, 1.0].into_iter().enumerate() {
        at(if i < 7 { 80 } else { 100 }, &|b| {
            b.hero = Hero::Morph(t);
            b.push = if t < 0.4 {
                0
            } else if t > 1.0 {
                2
            } else {
                1
            };
        });
    }
    // A face on the finished body, eyes shut, and mesimon is ready with it.
    at(160, &|b| {
        b.hero = asleep(Eyes::Closed, Mouth::None);
        b.tail = 1;
    });
    at(160, &|b| b.tail = 3);
    // Asleep, half awake, asleep again, and then awake at once.
    at(700, &|_| {});
    at(500, &|b| b.hero = asleep(Eyes::Lid, Mouth::Flat));
    at(350, &|b| b.hero = asleep(Eyes::Closed, Mouth::Flat));
    at(550, &|b| {
        b.hero = asleep(Eyes::Wide, Mouth::O);
        b.dy = 1;
    });
    at(100, &|b| {
        b.hero = asleep(Eyes::Blink, Mouth::Flat);
        b.dy = 0;
    });
    // Who is this? One neighbour, the other; then you, and a blush.
    let looking = |look| Hero::Shin(creature::pose(Eyes::Open, Mouth::Flat, look, false, false));
    at(650, &|b| b.hero = looking(-1));
    at(200, &|b| b.hero = looking(0));
    at(650, &|b| b.hero = looking(1));
    at(100, &|b| b.hero = asleep(Eyes::Blink, Mouth::Flat));
    at(500, &|b| b.hero = Hero::Shin(face(Eyes::Open, Mouth::Smile)));
    let wave = Hero::Shin(creature::pose(Eyes::Open, Mouth::Grin, 0, true, true));
    let grin = Hero::Shin(face(Eyes::Open, Mouth::Grin));
    at(280, &|b| b.hero = wave);
    at(200, &|b| b.hero = grin);
    at(280, &|b| b.hero = wave);
    at(400, &|b| b.hero = grin);
    // A happy hop with a sparkle.
    let happy = face(Eyes::Happy, Mouth::Grin);
    at(90, &|b| b.hero = Hero::Squash(happy, (22, 14)));
    at(90, &|b| {
        b.hero = Hero::Squash(happy, (19, 18));
        b.dy = 3;
    });
    at(110, &|b| {
        b.hero = Hero::Shin(happy);
        b.dy = 7;
        b.sparkle = Some(0);
    });
    at(140, &|b| b.dy = 8);
    at(110, &|b| {
        b.dy = 6;
        b.sparkle = Some(1);
    });
    at(90, &|b| {
        b.hero = Hero::Squash(happy, (19, 18));
        b.dy = 2;
    });
    at(110, &|b| {
        b.hero = Hero::Squash(happy, (22, 14));
        b.dy = 0;
        b.sparkle = Some(0);
    });
    at(700, &|b| {
        b.hero = Hero::Shin(happy);
        b.sparkle = Some(1);
    });
    at(500, &|b| {
        b.hero = Hero::Shin(face(Eyes::Happy, Mouth::Smile));
        b.sparkle = None;
    });
    at(150, &|b| b.hero = Hero::Shin(face(Eyes::Blink, Mouth::Grin)));
    at(400, &|b| b.hero = Hero::Shin(face(Eyes::Open, Mouth::Smile)));
    at(300, &|b| b.skip = false);
    beats
}

fn total(beats: &[Beat]) -> u64 {
    beats.iter().map(|b| u64::from(b.ms)).sum()
}

/// The beat `ms` in, or `None` once the storyboard is over.
fn beat_at(beats: &[Beat], ms: u64) -> Option<Beat> {
    let mut t = ms;
    for b in beats {
        if t < u64::from(b.ms) {
            return Some(*b);
        }
        t -= u64::from(b.ms);
    }
    None
}

// -- one frame ------------------------------------------------------------------

/// Where the intro plays: under the installer's output, or before a board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Install,
    Board,
}

/// Five rows of headroom for the hop, eight for the word and three for the
/// ן's stem, a blank, and the closing lines under it.
const HEAD: u16 = 5;
const LINE_A: u16 = 17;
const LINE_B: u16 = 19;
/// The installer's hint, once its lines are up: where the prompt comes back.
const PROMPT: u16 = 22;
/// The word's width in pixels (one per column), the letter standing.
fn word_width() -> i16 {
    WORD.iter().map(|g| Glyph::of(g).w).sum::<i16>() + GAP * (WORD.len() as i16 - 1)
}
/// The narrowest terminal it plays in: the word, the shin's nudge and its
/// sparkles, and a margin.
const MIN_COLS: u16 = 88;

impl Place {
    fn rows(self) -> u16 {
        match self {
            Place::Install => PROMPT + 1,
            Place::Board => LINE_B + 1,
        }
    }

    /// Room for the whole of it: below this nothing plays and nothing is
    /// marked, so a bigger terminal plays it.
    pub(crate) fn fits(self, cols: u16, rows: u16) -> bool {
        cols >= MIN_COLS
            && match self {
                Place::Install => rows >= self.rows() + 3,
                Place::Board => rows >= self.rows() + 2,
            }
    }
}

/// The installer's closing lines, word for word what `install.sh` prints
/// when no intro plays (`install_sh_says_the_same_next_lines`).
const READY: &str = "mesimon is ready";
const NEXT: [&str; 2] =
    ["next:  mesimon doctor      check your environment", "       cd <a git repo> && mesimon"];
const TAGLINE: &str = "a terminal kanban board that runs your coding agents";
const OPENS: &str = "enter opens your board";
const SKIPS: &str = "any key skips";

/// A frame's pixels, inked: one column and half a row each.
struct Canvas {
    w: i16,
    h: i16,
    px: Vec<Option<Color>>,
}

impl Canvas {
    fn put(&mut self, x: i16, y: i16, c: Color) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = Some(c);
        }
    }

    fn get(&self, x: i16, y: i16) -> Option<Color> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }
}

/// Draw `beat` into `area` (for the board, the block centred in it).
fn render(
    buf: &mut Buffer,
    area: Rect,
    place: Place,
    beat: &Beat,
    theme: &Theme,
    ink: &CreatureInk,
) {
    let rows = place.rows();
    let top = match place {
        Place::Install => area.y + 1,
        Place::Board => area.y + area.height.saturating_sub(rows) / 2,
    };
    let mut cv = Canvas {
        w: area.width as i16,
        h: rows as i16 * 2,
        px: vec![None; area.width as usize * rows as usize * 2],
    };
    let mut put = |g: &Glyph, x0: i16, y0: i16, keep: f32| {
        for y in 0..g.h {
            for x in 0..g.w {
                if let Some(role) = g.at(x, y) {
                    cv.put(x0 + x, y0 + y, theme.toward_ground(role.ink(ink), keep));
                }
            }
        }
    };
    // The word's bottom pixel row.
    let base = (HEAD as i16 + 8) * 2 - 1;
    let n = WORD.len();
    let mut x = (area.width as i16 - word_width()) / 2;
    let mut marks: Vec<(i16, i16, char)> = Vec::new();
    for (i, rows) in WORD.iter().enumerate() {
        let g = Glyph::of(rows);
        let at = x;
        x += g.w + GAP;
        // Right to left, each letter a little after the one before it.
        let order = (n - 1 - i) as f32;
        let keep = (beat.fade * (1.0 + (n as f32 - 1.0) * 0.5) - order * 0.5).clamp(0.0, 1.0);
        if keep <= 0.0 {
            continue;
        }
        if i != HERO {
            let nudge = if i < HERO { -beat.push } else { beat.push };
            // Every letter hangs from the same top line: the י stays up there
            // and the ן's stem runs on below.
            put(&g, at + nudge, base + 1 - 16, keep);
            continue;
        }
        let (pic, ox) = match beat.hero {
            Hero::Letter { dx, size } => {
                let p = match size {
                    Some((w, h)) => g.resample(w, h),
                    None => g,
                };
                let ox = (LETTER_W - p.w) / 2 + dx;
                (p, ox)
            }
            Hero::Morph(t) => (morph_at(t), -1 - MPAD),
            Hero::Squash(fr, (w, h)) => {
                let p = shin_frame(&fr).resample(w, h);
                let ox = -1 + (20 - p.w) / 2;
                (p, ox)
            }
            Hero::Shin(fr) => (shin_frame(&fr), -1),
        };
        put(&pic, at + ox, base + 1 - pic.h - beat.dy, keep);
        if let Some(s) = beat.sparkle {
            let (bx, row) = (at - 1, (base + 1 - 16 - beat.dy).div_euclid(2));
            let spots = if s == 0 { [(-2, 3), (21, 1)] } else { [(-2, 1), (21, 3)] };
            for (c, r) in spots {
                marks.push((bx + c, row + r, if s == 0 { '✦' } else { '✧' }));
            }
        }
    }
    // Pixels to half blocks, as the shin's own stage does.
    for r in 0..rows as i16 {
        for c in 0..cv.w {
            let (t, b) = (cv.get(c, 2 * r), cv.get(c, 2 * r + 1));
            let (ch, style) = match (t, b) {
                (Some(t), Some(b)) if t == b => ('█', Style::default().fg(t)),
                (Some(t), Some(b)) => ('▀', Style::default().fg(t).bg(b)),
                (Some(t), None) => ('▀', Style::default().fg(t)),
                (None, Some(b)) => ('▄', Style::default().fg(b)),
                (None, None) => continue,
            };
            let at = Position::new(area.x + c as u16, top + r as u16);
            if buf.area.contains(at) {
                buf[at].set_char(ch).set_style(style);
            }
        }
    }
    for (c, r, ch) in marks {
        if c < 0 || r < 0 {
            continue;
        }
        let at = Position::new(area.x + c as u16, top + r as u16);
        if buf.area.contains(at) && buf[at].symbol() == " " {
            buf[at].set_char(ch).set_style(Style::default().fg(ink.calm));
        }
    }
    let text = |buf: &mut Buffer, col: u16, row: u16, s: &str, style: Style| {
        if row < rows && (col as usize + s.chars().count()) <= area.width as usize {
            buf.set_string(area.x + col, top + row, s, style);
        }
    };
    let centre = |s: &str| (area.width.saturating_sub(s.chars().count() as u16)) / 2;
    if beat.tail >= 1 {
        let first = if place == Place::Install { READY } else { TAGLINE };
        text(buf, centre(first), LINE_A, first, theme.base());
    }
    if beat.tail >= 3 {
        match place {
            Place::Install => {
                text(buf, 2, LINE_B, NEXT[0], theme.dim2());
                text(buf, 2, LINE_B + 1, NEXT[1], theme.dim2());
            }
            Place::Board => text(buf, centre(OPENS), LINE_B, OPENS, theme.dim3()),
        }
    }
    // The hint holds the place of what comes next: the words, then the
    // prompt. On the board, once its words are up, enter is the way on.
    if beat.skip && beat.fade > 0.0 {
        if beat.tail == 0 {
            text(buf, centre(SKIPS), LINE_A, SKIPS, theme.dim3());
        } else if place == Place::Install {
            text(buf, 0, PROMPT, SKIPS, theme.dim3());
        }
    }
}

// -- playing it -------------------------------------------------------------------

/// Play the storyboard on `term`. A key skips to its end; on the board the
/// shin then keeps the seat's invitation until a key opens the board, and
/// under the installer the last frame stays. Returns the area it drew in.
fn play<B: Backend<Error = io::Error>>(
    term: &mut Terminal<B>,
    place: Place,
    theme: &Theme,
    ink: &CreatureInk,
) -> io::Result<Rect> {
    let beats = storyboard();
    let end = total(&beats);
    let last = *beats.last().expect("a storyboard has beats");
    let start = Instant::now();
    let mut ahead = 0u64;
    // A late answer to the colour query is the one thing on stdin that is
    // not a person, and it must not skip anything.
    let mut swallow = crate::osc::ReplySwallow::default();
    let mut area = Rect::default();
    loop {
        let ms = start.elapsed().as_millis() as u64 + ahead;
        let beat = beat_at(&beats, ms).unwrap_or_else(|| match place {
            Place::Install => last,
            Place::Board => {
                Beat { hero: Hero::Shin(*creature::frame_at(Anim::Seat, ms - end)), ..last }
            }
        });
        term.draw(|f| {
            area = f.area();
            render(f.buffer_mut(), area, place, &beat, theme, ink);
        })?;
        if ms >= end && place == Place::Install {
            return Ok(area);
        }
        if !event::poll(Duration::from_millis(25))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let crate::osc::Feed::Pass(keys) = swallow.feed(key.code, key.modifiers) {
            if keys.is_empty() {
                continue;
            }
            if ms >= end {
                return Ok(area);
            }
            ahead = end.saturating_sub(start.elapsed().as_millis() as u64);
        }
    }
}

/// `~/.local/state/mesimon/welcomed`: at the state root beside `prefs.json`,
/// keyed off `HOME` as it is, because the intro is the machine's, not a
/// board's.
fn marker_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/mesimon/welcomed"))
}

/// Marked before it plays: a person who quits halfway has seen it. A marker
/// that cannot be written plays it again next time, which is the lesser
/// harm.
fn mark(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("mesimon {}\n", env!("CARGO_PKG_VERSION")));
}

/// Whether this machine is still to see it, and in this terminal.
fn due(place: Place, theme: &Theme) -> Option<(PathBuf, CreatureInk)> {
    let path = marker_path()?;
    if path.exists() || !io::stdout().is_terminal() {
        return None;
    }
    let ink = theme.creature_ink(false)?;
    let (cols, rows) = ratatui::crossterm::terminal::size().ok()?;
    place.fits(cols, rows).then_some((path, ink))
}

/// The board's turn, before its first frame: the intro fullscreen, once.
pub(crate) fn before_board<B: Backend<Error = io::Error>>(
    term: &mut Terminal<B>,
    theme: &Theme,
) -> io::Result<()> {
    let Some((path, ink)) = due(Place::Board, theme) else { return Ok(()) };
    mark(&path);
    term.hide_cursor()?;
    play(term, Place::Board, theme, &ink)?;
    term.clear()
}

/// `mesimon welcome`, run by the installer after a successful install: the
/// intro inline under its output. Exits 0 when it played (the installer
/// then prints neither its picture nor its `next:` lines, which the intro
/// carries), 1 when it did not.
pub fn welcome_command() -> i32 {
    use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
    let loaded = crate::prefs::load_home();
    let known = loaded.prefs.follows_os().then(crate::appearance::probe).flatten();
    let detected = crate::detect::detect(known);
    let flavor = detected.forced.unwrap_or(loaded.prefs.for_ground(detected.ground));
    let theme = Theme::new(flavor, detected.profile);
    let Some((path, ink)) = due(Place::Install, &theme) else { return 1 };
    mark(&path);
    let played = (|| -> io::Result<()> {
        enable_raw_mode()?;
        let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
        let opts = ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(Place::Install.rows() + 1),
        };
        let mut term = Terminal::with_options(backend, opts)?;
        term.hide_cursor()?;
        let area = play(&mut term, Place::Install, &theme, &ink);
        // The prompt comes back on the row the hint held.
        if let Ok(a) = &area {
            term.set_cursor_position(Position::new(0, a.bottom().saturating_sub(1)))?;
        }
        term.show_cursor()?;
        io::stdout().flush()?;
        area.map(drop)
    })();
    let _ = disable_raw_mode();
    if played.is_ok() {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};

    fn screen(place: Place, beat: &Beat, w: u16, h: u16) -> String {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = theme.creature_ink(false).expect("truecolor draws the shin");
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        render(&mut buf, area, place, beat, &theme, &ink);
        (0..h)
            .map(|y| {
                let row: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
                row.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn golden(name: &str, got: &str) {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden").join(name);
        if std::env::var_os("MESIMON_UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, got).expect("write golden");
            return;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(want == got, "{} drifted; review + MESIMON_UPDATE_GOLDEN=1\n{got}", path.display());
    }

    #[test]
    fn the_word_ends_with_the_shin_in_it_where_the_letter_stood() {
        let beats = storyboard();
        let last = *beats.last().expect("beats");
        golden("intro-board-end.txt", &screen(Place::Board, &last, 100, 24));
        golden("intro-install-end.txt", &screen(Place::Install, &last, 100, 24));
        // Mid-breath: the letter half way to the shin.
        let mid = beats.iter().find(|b| matches!(b.hero, Hero::Morph(t) if t == 0.5));
        golden("intro-morph.txt", &screen(Place::Board, mid.expect("a mid-morph beat"), 100, 24));
    }

    #[test]
    fn the_morph_starts_on_the_letter_and_ends_on_the_shin() {
        let m = morph();
        let solid = |g: &Glyph| g.px.iter().map(Option::is_some).collect::<Vec<_>>();
        let letter = Glyph::of(SHIN);
        let want: Vec<bool> = (0..m.h)
            .flat_map(|y| (0..m.w).map(move |x| (x, y)))
            .map(|(x, y)| letter.at(x - MPAD - 1, y - MPAD).is_some())
            .collect();
        assert_eq!(solid(&morph_at(0.0)), want, "t = 0 is the letter");
        let (sw, sh, body, _) = creature::silhouette(Size::Medium);
        let want: Vec<bool> = (0..m.h)
            .flat_map(|y| (0..m.w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (sx, sy) = (x - MPAD, y - MPAD);
                sx >= 0 && sy >= 0 && sx < sw && sy < sh && body[(sy * sw + sx) as usize]
            })
            .collect();
        assert_eq!(solid(&morph_at(1.0)), want, "t = 1 is the shin");
    }

    #[test]
    fn the_storyboard_is_short_and_says_ready_as_the_morph_ends() {
        let beats = storyboard();
        let ms = total(&beats);
        assert!((10_000..16_000).contains(&ms), "{ms} ms");
        let ready = beats.iter().position(|b| b.tail > 0).expect("its words come");
        assert!(matches!(beats[ready - 1].hero, Hero::Morph(t) if t == 1.0));
        assert!(matches!(beats[ready].hero, Hero::Shin(_)), "a face with the words");
        assert!(!beats.last().expect("beats").skip, "the hint is gone at the end");
    }

    #[test]
    fn it_plays_only_where_the_word_fits() {
        assert!(Place::Board.fits(MIN_COLS, 22));
        assert!(!Place::Board.fits(MIN_COLS - 1, 40));
        assert!(!Place::Board.fits(120, 21));
        assert!(Place::Install.fits(MIN_COLS, 26));
        assert!(!Place::Install.fits(MIN_COLS, 25));
        // Every frame keeps the word, its nudge and its sparkles on screen.
        for b in storyboard() {
            let s = screen(Place::Board, &b, MIN_COLS, 22);
            assert!(s.lines().all(|l| l.chars().count() <= MIN_COLS as usize));
        }
        assert!(word_width() + 6 <= MIN_COLS as i16);
    }

    #[test]
    fn install_sh_says_the_same_next_lines() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
        let sh = std::fs::read_to_string(path).expect("install.sh");
        for line in NEXT.iter().chain([&READY]) {
            assert!(sh.contains(line), "install.sh no longer prints {line:?}");
        }
        assert!(sh.contains("mesimon\" welcome"), "install.sh runs the intro");
    }

    #[test]
    fn the_marker_is_at_the_state_root() {
        let p = marker_path().expect("HOME is set under test");
        assert!(p.ends_with(".local/state/mesimon/welcomed"));
    }
}
