//! The shin creature (T-451): the letter ש as a creature, acting out what its
//! session is doing. One source of pixels (`assets/mascot/shin.txt`), a face
//! stamped over its anchors, props around it, and a timeline per animation.
//! The ticket page draws it; the notification icons and the installer's
//! welcome are rendered from it by the goldens at the bottom of this file.
//!
//! A cell holds two square pixels: `▀` inks the upper one, `▄` the lower,
//! `█` both, and a `▀` with a background paints two colours. The L1 law
//! admits `▄` and `█` only at the cells a draw records (`Drawn`).

use std::ops::RangeInclusive;
use std::sync::OnceLock;

use mesimon_core::board::{ExitReason, SessionState, StopReason};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{CreatureInk, Theme};

const SOURCE: &str = include_str!("../../../assets/mascot/shin.txt");

/// Which drawing: the companion beside a transcript, the preview's own, and
/// the installer's (never animated, so only the goldens draw it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Size {
    Small,
    Medium,
    #[cfg_attr(not(test), allow(dead_code))]
    Large,
}

impl Size {
    fn blush_w(self) -> i16 {
        match self {
            Size::Small => 1,
            Size::Medium => 2,
            Size::Large => 3,
        }
    }

    /// How many of the stems' top rows the crown tints (T-1186): two of the
    /// small one's three, three of the medium's five, as a crown sits on a
    /// head and leaves the neck.
    fn crown_rows(self) -> i16 {
        match self {
            Size::Small => 2,
            Size::Medium => 3,
            Size::Large => 4,
        }
    }
}

// -- the crown ------------------------------------------------------------------

/// One row's step as the crown fills the stems or drains off them, and one
/// beat of its flash (T-1186): two steps of the redraw clock.
pub(crate) const CROWN_STEP_MS: u64 = 200;
/// How long the crown takes to drain off every row it can tint: the
/// longest `Crown::Going` a shin draws.
pub(crate) const CROWN_OFF_MS: u64 = CROWN_STEP_MS * 4;
/// How long the crown flashes when its agent acts on another ticket.
pub(crate) const CROWN_ACT_MS: u64 = CROWN_STEP_MS * 3;

/// The crown on the shin (T-1186): while its ticket wears the crown, the
/// top of its stems is in the crown's tint. It fills in from the top as the
/// crown comes on, with a smile and a sparkle; drains back up as it comes
/// off; and flashes as the crown's lightning leaves the shin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Crown {
    #[default]
    Off,
    On,
    /// Crowned `ms` ago.
    Coming(u64),
    /// Uncrowned `ms` ago.
    Going(u64),
    /// The crown acted on another ticket `ms` ago.
    Acting(u64),
}

/// What a crown does to one frame: rows tinted from the top, whether they
/// flash, whether the shin cheers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Gild {
    rows: i16,
    flash: bool,
    cheer: bool,
}

impl Crown {
    fn gild(self, size: Size) -> Gild {
        let full = size.crown_rows();
        let step = |ms: u64| i16::try_from(ms / CROWN_STEP_MS).unwrap_or(i16::MAX);
        match self {
            Crown::Off => Gild::default(),
            Crown::On => Gild { rows: full, ..Gild::default() },
            Crown::Coming(ms) => {
                let rows = (step(ms) + 1).min(full);
                Gild { rows, cheer: step(ms) >= full, ..Gild::default() }
            }
            Crown::Going(ms) => Gild { rows: (full - 1 - step(ms)).max(0), ..Gild::default() },
            Crown::Acting(ms) => Gild {
                rows: full,
                flash: ms < CROWN_ACT_MS && step(ms) % 2 == 0,
                ..Gild::default()
            },
        }
    }
}

// -- the body -----------------------------------------------------------------

/// One drawing from the source, parsed once.
struct Body {
    w: i16,
    /// Even: a half-block cell holds two rows.
    h: i16,
    solid: Vec<bool>,
    eyes: Vec<(i16, i16)>,
    mouth: (i16, i16),
    blush: Vec<(i16, i16)>,
    light: Option<(i16, i16)>,
    drops: Vec<(i16, i16)>,
    /// The right arm's first column.
    split: i16,
    /// The body's first row: everything above it is arms.
    top: i16,
}

impl Body {
    fn get(size: Size) -> &'static Body {
        static BODIES: OnceLock<[Body; 3]> = OnceLock::new();
        let all = BODIES.get_or_init(|| [parse("small"), parse("medium"), parse("large")]);
        &all[size as usize]
    }

    fn solid(&self, x: i16, y: i16) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h && self.solid[(y * self.w + x) as usize]
    }

    /// Row `y`'s solid runs, as (start, length).
    fn runs(&self, y: i16) -> Vec<(i16, i16)> {
        let mut runs: Vec<(i16, i16)> = Vec::new();
        for x in 0..self.w {
            if !self.solid(x, y) {
                continue;
            }
            match runs.last_mut() {
                Some((start, len)) if *start + *len == x => *len += 1,
                _ => runs.push((x, 1)),
            }
        }
        runs
    }
}

fn parse(name: &str) -> Body {
    let header = format!("[{name}]");
    let rows: Vec<&str> =
        SOURCE.lines().skip_while(|l| *l != header).skip(1).take_while(|l| !l.is_empty()).collect();
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let h = rows.len() + rows.len() % 2;
    let mut b = Body {
        w: w as i16,
        h: h as i16,
        solid: vec![false; w * h],
        eyes: Vec::new(),
        mouth: (0, 0),
        blush: Vec::new(),
        light: None,
        drops: Vec::new(),
        split: 0,
        top: 0,
    };
    for (y, row) in rows.iter().enumerate() {
        for (x, ch) in row.bytes().enumerate() {
            let at = (x as i16, y as i16);
            match ch {
                b'#' => {}
                b'E' => b.eyes.push(at),
                b'M' => b.mouth = at,
                b'B' => b.blush.push(at),
                b'L' => b.light = Some(at),
                b'D' => {
                    b.drops.push(at);
                    continue;
                }
                _ => continue,
            }
            b.solid[y * w + x] = true;
        }
    }
    b.split = b.runs(0).last().map_or(0, |r| r.0);
    b.top = (0..b.h).find(|&y| b.runs(y).iter().any(|r| r.1 * 2 >= b.w)).unwrap_or(0);
    b
}

// -- the face and the props -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Eyes {
    Open,
    Blink,
    Closed,
    Happy,
    Wide,
    Lid,
    X,
    /// White eyes, the pupils rolled up to the right or the left: thinking.
    /// A solid eye cannot look anywhere, so this is the one face with whites.
    UpRight,
    UpLeft,
}

impl Eyes {
    /// Whether a click on the shin shuts these eyes for its blink (T-1177):
    /// eyes open, wide, heavy or rolled up. Eyes already shut, or the happy
    /// arcs and the X of a failure, stay as the frame draws them.
    fn shut_by_a_click(self) -> bool {
        matches!(self, Eyes::Open | Eyes::Wide | Eyes::Lid | Eyes::UpRight | Eyes::UpLeft)
    }
}

/// An eye, from its top-left anchor: `o` is the eye, `*` its glint (or the
/// white of an eye that has one), `.` leaves the body showing.
fn eye(size: Size, e: Eyes) -> &'static [&'static str] {
    match (size, e) {
        (Size::Small, Eyes::Open) => &["*o", "oo"],
        (Size::Small, Eyes::Blink | Eyes::Closed) => &["..", "oo"],
        (Size::Small, Eyes::Happy) => &["oo", ".."],
        (Size::Small, Eyes::Wide) => &["*o", "oo", "oo"],
        (Size::Small, Eyes::Lid) => &["..", "*o"],
        (Size::Small, Eyes::X) => &["o.", ".o"],
        (Size::Small, Eyes::UpRight) => &["*o", "**"],
        (Size::Small, Eyes::UpLeft) => &["o*", "**"],
        (Size::Medium, Eyes::Open) => &["*oo", "ooo", "oo*"],
        (Size::Medium, Eyes::Blink) => &["...", "ooo", "..."],
        (Size::Medium, Eyes::Closed) => &["...", "o.o", ".o."],
        (Size::Medium, Eyes::Happy) => &[".o.", "o.o", "..."],
        (Size::Medium, Eyes::Wide) => &["*oo", "ooo", "ooo", "oo*"],
        (Size::Medium, Eyes::Lid) => &["...", "*oo", "ooo"],
        (Size::Medium, Eyes::X) => &["o.o", ".o.", "o.o"],
        (Size::Medium, Eyes::UpRight) => &["*oo", "*oo", "***"],
        (Size::Medium, Eyes::UpLeft) => &["oo*", "oo*", "***"],
        (Size::Large, Eyes::Open) => &["*ooo", "oooo", "oooo", "oo*o"],
        (Size::Large, Eyes::Blink) => &["....", "....", "oooo", "...."],
        (Size::Large, Eyes::Closed) => &["....", "o..o", ".oo.", "...."],
        (Size::Large, Eyes::Happy) => &["....", ".oo.", "o..o", "...."],
        (Size::Large, Eyes::Wide) => &["*ooo", "oooo", "oooo", "oooo", "oo*o"],
        (Size::Large, Eyes::Lid) => &["....", "*ooo", "oooo", "oo*o"],
        (Size::Large, Eyes::X) => &["o..o", ".oo.", ".oo.", "o..o"],
        (Size::Large, Eyes::UpRight) => &["**oo", "**oo", "****", "****"],
        (Size::Large, Eyes::UpLeft) => &["oo**", "oo**", "****", "****"],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mouth {
    Smile,
    Flat,
    O,
    Grin,
    Wavy,
    /// A short line pushed to one side: "hmm".
    Hmm,
    None,
}

fn mouth(size: Size, m: Mouth) -> &'static [&'static str] {
    match (size, m) {
        (_, Mouth::None) => &[],
        (Size::Small, Mouth::O) => &["oo", "oo"],
        (Size::Small, Mouth::Hmm) => &[".o"],
        (Size::Small, _) => &["oo"],
        (_, Mouth::Smile) => &["o..o", ".oo."],
        (_, Mouth::Flat) => &["....", ".oo."],
        (_, Mouth::O) => &[".oo.", ".oo."],
        (_, Mouth::Grin) => &["oooo", ".oo."],
        (_, Mouth::Wavy) => &["o.o.", ".o.o"],
        (_, Mouth::Hmm) => &["....", "..oo"],
    }
}

/// What stands around the body. A mark is a text glyph in a cell no pixel
/// touches; `Light` and `Drop` are pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prop {
    /// "?" over the middle.
    Ask,
    /// The needs-you "!" beside the right arm: the creature's one `attn`.
    Bang,
    /// The sleeper's rising z, small or big, by rung.
    Z(usize),
    BigZ(usize),
    Sparkle(usize),
    Star(usize),
    /// The right arm's head, lit like a status light.
    Light,
    /// The sweat drop, by rung down its path.
    Drop(usize),
}

/// What stands beside the body and widens the stage for it: the laptop the
/// creature types at, or the cloud it thinks in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scene {
    /// The laptop with line `line` of its code being typed, `chars` of it
    /// so far, and the hand pressing a key or raised.
    Laptop { line: u8, chars: u8, press: bool },
    /// The thought cloud with its first n dots filled in.
    Cloud(u8),
}

// -- frames and timelines ---------------------------------------------------------

/// One pose: a face, where the arms are, what stands around, and for how
/// long. Every `ms` is a multiple of 100, the redraw clock's step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    eyes: Eyes,
    mouth: Mouth,
    /// Up one pixel: half a cell, which a half block can draw.
    lift: bool,
    look: (i16, i16),
    /// The right arm's head, leaned out a pixel.
    wave: i16,
    /// The heads, spread (1) or drawn in (-1).
    wig: i16,
    /// The arms, leaned together.
    tilt: i16,
    faded: bool,
    blush: bool,
    props: &'static [Prop],
    scene: Option<Scene>,
    ms: u32,
}

const fn f(ms: u32) -> Frame {
    Frame {
        eyes: Eyes::Open,
        mouth: Mouth::Smile,
        lift: false,
        look: (0, 0),
        wave: 0,
        wig: 0,
        tilt: 0,
        faded: false,
        blush: true,
        props: &[],
        scene: None,
        ms,
    }
}

impl Frame {
    const fn eyes(self, eyes: Eyes) -> Self {
        Frame { eyes, ..self }
    }
    const fn mouth(self, mouth: Mouth) -> Self {
        Frame { mouth, ..self }
    }
    const fn lift(self) -> Self {
        Frame { lift: true, ..self }
    }
    const fn look(self, x: i16, y: i16) -> Self {
        Frame { look: (x, y), ..self }
    }
    const fn wave(self) -> Self {
        Frame { wave: 1, ..self }
    }
    const fn wig(self, wig: i16) -> Self {
        Frame { wig, ..self }
    }
    const fn tilt(self, tilt: i16) -> Self {
        Frame { tilt, ..self }
    }
    const fn props(self, props: &'static [Prop]) -> Self {
        Frame { props, ..self }
    }
    const fn gone(self) -> Self {
        Frame { faded: true, blush: false, ..self }
    }
    const fn scene(self, scene: Scene) -> Self {
        Frame { scene: Some(scene), ..self }
    }
}
/// A pose for the intro (T-869), which plays its own clock: a face, where
/// it looks, and whether the right arm waves and the cheeks show.
pub(crate) const fn pose(eyes: Eyes, mouth: Mouth, look: i16, wave: bool, blush: bool) -> Frame {
    Frame { eyes, mouth, look: (look, 0), wave: wave as i16, blush, ..f(100) }
}

/// A drawing's silhouette: its size, its solid pixels and the row its body
/// starts at, for a picture that grows into it (the intro's morph).
pub(crate) fn silhouette(size: Size) -> (i16, i16, &'static [bool], i16) {
    let b = Body::get(size);
    (b.w, b.h, &b.solid, b.top)
}

const fn alarmed(ms: u32) -> Frame {
    f(ms).eyes(Eyes::Wide).mouth(Mouth::O)
}
const fn asleep(ms: u32) -> Frame {
    f(ms).eyes(Eyes::Closed).mouth(Mouth::None)
}
const fn tired(ms: u32) -> Frame {
    f(ms).eyes(Eyes::Lid).mouth(Mouth::Wavy)
}
const fn dizzy(ms: u32) -> Frame {
    f(ms).eyes(Eyes::X).mouth(Mouth::Wavy).wig(1)
}
const fn plain(ms: u32) -> Frame {
    f(ms).mouth(Mouth::Flat)
}

const BLINK: Frame = f(100).eyes(Eyes::Blink);
const BANG: &[Prop] = &[Prop::Bang];
const ASK: &[Prop] = &[Prop::Ask];
const LIGHT: &[Prop] = &[Prop::Light];

/// The invitation: looks at you, blinks, waves now and then.
const SEAT: &[Frame] = &[
    f(1500),
    BLINK,
    f(1100),
    f(300).mouth(Mouth::Grin).wave(),
    f(200).mouth(Mouth::Grin),
    f(300).mouth(Mouth::Grin).wave(),
    f(900).mouth(Mouth::Grin),
    BLINK,
    f(700),
];
/// Waiting for a first prompt: glances left and right.
const IDLE: &[Frame] =
    &[f(1200), f(900).look(-1, 0), f(300), f(900).look(1, 0), f(500), BLINK, f(900)];
/// Waking: shut, heavy, shut again, then open with a stretch.
const SPAWNING: &[Frame] = &[
    f(900).eyes(Eyes::Closed).mouth(Mouth::Flat),
    f(500).eyes(Eyes::Lid).mouth(Mouth::Flat),
    f(300).eyes(Eyes::Closed).mouth(Mouth::Flat),
    alarmed(500).lift(),
    BLINK,
    f(400),
    BLINK,
];
const fn ponder(ms: u32, eyes: Eyes, dots: u8) -> Frame {
    f(ms).eyes(eyes).mouth(Mouth::Hmm).scene(Scene::Cloud(dots))
}
/// Eyes rolled up, a "hmm", the head tilted toward a thought cloud whose
/// dots fill in one by one; now and then a glance the other way.
const THINKING: &[Frame] = &[
    ponder(400, Eyes::UpRight, 0).tilt(1),
    ponder(400, Eyes::UpRight, 1).tilt(1),
    ponder(400, Eyes::UpRight, 2).tilt(1),
    ponder(600, Eyes::UpRight, 3).tilt(1),
    ponder(100, Eyes::Blink, 3).tilt(1),
    ponder(300, Eyes::UpRight, 0).tilt(1),
    ponder(400, Eyes::UpLeft, 1),
    ponder(400, Eyes::UpLeft, 2),
    ponder(600, Eyes::UpLeft, 3),
    ponder(300, Eyes::UpLeft, 0),
];

/// At the laptop, eyes on the screen: line `line` of the code being typed,
/// `chars` of it so far.
const fn typing(ms: u32, line: u8, chars: u8, press: bool) -> Frame {
    f(ms).look(1, 0).mouth(Mouth::Flat).scene(Scene::Laptop { line, chars, press })
}
/// Typing at the laptop, a loop with no seam (T-1150): each line of the
/// size's `Laptop::code` types out a keystroke a character, then an Enter,
/// with a nod, scrolls the screen up a line, so the last frame leads into
/// the first. One face the whole time, eyes on the screen, and one blink.
fn working(size: Size) -> &'static [Frame] {
    fn typed(code: &[(i16, i16)]) -> Vec<Frame> {
        let mut frames = Vec::new();
        for (line, &(_, len)) in code.iter().enumerate() {
            let (line, len) = (line as u8, len as u8);
            frames.push(typing(300, line, 0, true).tilt(1));
            frames.push(typing(200, line, 0, false));
            for chars in 1..=len {
                frames.push(typing(200, line, chars, true));
                frames.push(typing(100, line, chars, false));
            }
            if line == 1 {
                frames.push(typing(300, line, len, false));
                frames.push(typing(100, line, len, false).eyes(Eyes::Blink));
            } else {
                frames.push(typing(400, line, len, false));
            }
        }
        frames
    }
    static LOOPS: OnceLock<[Vec<Frame>; 2]> = OnceLock::new();
    let loops = LOOPS.get_or_init(|| [typed(SMALL_LAPTOP.code), typed(MEDIUM_LAPTOP.code)]);
    &loops[usize::from(size != Size::Small)]
}
/// Wide eyes on you, the right arm waving beside an amber "!".
const NEEDS_YOU: &[Frame] = &[
    alarmed(300).wave().props(BANG),
    alarmed(300).props(BANG),
    alarmed(300).wave().props(BANG),
    alarmed(300),
    alarmed(900).props(BANG),
    f(100).eyes(Eyes::Blink).mouth(Mouth::O).props(BANG),
    alarmed(500).props(BANG),
];
/// Two hops with a sparkle, then a quiet smile.
const DONE: &[Frame] = &[
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).lift().props(&[Prop::Sparkle(0)]),
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).props(&[Prop::Sparkle(1)]),
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).lift().props(&[Prop::Sparkle(0)]),
    f(900).eyes(Eyes::Happy).mouth(Mouth::Grin).props(&[Prop::Sparkle(1)]),
    f(1300).eyes(Eyes::Happy),
];
const CONTENT: &[Frame] = &[f(2000), BLINK, f(1500), f(600).eyes(Eyes::Happy)];
/// Keeping watch: eyes sweep, the right arm's head blinks like a light.
const MONITORING: &[Frame] = &[
    plain(700).look(-1, 0).props(LIGHT),
    plain(700).look(-1, 0),
    plain(500).props(LIGHT),
    plain(700).look(1, 0),
    plain(700).look(1, 0).props(LIGHT),
    plain(500),
];
/// A startled hop and a "?", then it looks around.
const INTERRUPTED: &[Frame] = &[
    alarmed(300).lift().props(ASK),
    alarmed(800).props(ASK),
    plain(100).eyes(Eyes::Blink).props(ASK),
];
const LOOKING: &[Frame] = &[
    plain(900).look(-1, 0).props(ASK),
    plain(900).look(1, 0),
    plain(1200),
    plain(100).eyes(Eyes::Blink),
];
/// Shut eyes, slow breath, a snore, z Z drifting away.
const SLEEPING: &[Frame] = &[
    asleep(700).props(&[Prop::Z(0)]),
    asleep(700).props(&[Prop::Z(0), Prop::Z(1)]),
    asleep(700).mouth(Mouth::O).lift().props(&[Prop::Z(1), Prop::BigZ(2)]),
    asleep(700).mouth(Mouth::O).lift().props(&[Prop::BigZ(2)]),
    asleep(700),
];
/// Tired, a wobbly mouth and a sweat drop sliding down.
const THROTTLED: &[Frame] = &[
    tired(400).props(&[Prop::Drop(0)]),
    tired(400).props(&[Prop::Drop(1)]),
    tired(400).props(&[Prop::Drop(2)]),
    f(100).eyes(Eyes::Blink).mouth(Mouth::Wavy),
    tired(900),
];
/// X eyes, splayed heads, a star circling.
const FAILED: &[Frame] = &[
    dizzy(300).props(&[Prop::Star(0)]),
    dizzy(300).props(&[Prop::Star(1)]),
    dizzy(300).props(&[Prop::Star(2)]),
    dizzy(300).props(&[Prop::Star(3)]),
    dizzy(300).props(&[Prop::Star(2)]),
    dizzy(300).props(&[Prop::Star(1)]),
];
/// Sunk into the ground, eyes shut, still.
const EXITED: &[Frame] = &[f(1000).eyes(Eyes::Blink).mouth(Mouth::None).gone()];
/// A head tilt one way, then the other, with a "?".
const UNKNOWN: &[Frame] = &[
    plain(900).tilt(1).look(1, 0).props(ASK),
    plain(100).tilt(1).look(1, 0).eyes(Eyes::Blink).props(ASK),
    plain(700).tilt(1).look(1, 0).props(ASK),
    plain(400),
    plain(900).tilt(-1).look(-1, 0).props(ASK),
    plain(500),
];

/// Half asleep, the cheeks not yet showing: the waking letter's first poses.
const fn drowsy(ms: u32, eyes: Eyes, mouth: Mouth) -> Frame {
    Frame { blush: false, ..f(ms).eyes(eyes).mouth(mouth) }
}
/// The shin the ש has just become (T-1156), as the intro's wakes: asleep,
/// heavy, shut again, awake at once, then a look at each neighbour.
const WAKING: &[Frame] = &[
    drowsy(800, Eyes::Closed, Mouth::None),
    drowsy(500, Eyes::Lid, Mouth::Flat),
    drowsy(300, Eyes::Closed, Mouth::Flat),
    Frame { blush: false, ..alarmed(500).lift() },
    BLINK,
    plain(600).look(-1, 0),
    plain(200),
    plain(600).look(1, 0),
    BLINK,
];
/// The launch over and the agent at its box (T-1156): the intro's smile,
/// wave and happy hop with its sparkle.
const READY: &[Frame] = &[
    f(300).mouth(Mouth::Grin).wave(),
    f(200).mouth(Mouth::Grin),
    f(200).mouth(Mouth::Grin),
    f(300).mouth(Mouth::Grin).wave(),
    f(400).mouth(Mouth::Grin),
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).lift().props(&[Prop::Sparkle(0)]),
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).props(&[Prop::Sparkle(1)]),
    f(200).eyes(Eyes::Happy).mouth(Mouth::Grin).lift().props(&[Prop::Sparkle(0)]),
    f(700).eyes(Eyes::Happy).mouth(Mouth::Grin).props(&[Prop::Sparkle(1)]),
    f(500).eyes(Eyes::Happy),
];

/// The letter's beats (T-1156): how long, then the morph's `t` (`None` for
/// the letter itself), the letter's shift right and whether it is up a
/// pixel. Every beat a multiple of the redraw clock's 100 ms.
type LetterBeat = (u32, Option<f32>, i16, bool);
/// A breath, a long hold, a shiver, a hold: the letter while it waits on
/// the worktree.
const STIR: &[LetterBeat] = &[
    (1200, None, 0, false),
    (200, None, 0, true),
    (200, None, 0, false),
    (900, None, 0, false),
    (100, None, 1, false),
    (100, None, -1, false),
    (100, None, 1, false),
    (100, None, -1, false),
    (100, None, 0, false),
    (600, None, 0, false),
];
/// The intro's morph, its beats at the redraw clock: a held letter, then
/// the stems swelling into the body from the bottom up, a breath held.
const MORPH: &[LetterBeat] = &[
    (300, None, 0, false),
    (100, Some(0.2), 0, false),
    (100, Some(0.3), 0, false),
    (100, Some(0.4), 0, false),
    (100, Some(0.5), 0, false),
    (100, Some(0.6), 0, false),
    (100, Some(0.7), 0, false),
    (100, Some(0.82), 0, false),
    (100, Some(1.0), 0, false),
    (100, Some(1.1), 0, false),
    (100, Some(1.1), 0, false),
    (100, Some(1.0), 0, false),
];
fn beats_ms(beats: &[LetterBeat]) -> u64 {
    beats.iter().map(|b| u64::from(b.0)).sum()
}
/// The letter's beat `ms` into `anim`, or `None` where the shin's own
/// poses play: past the morph, and in every other animation.
fn letter_beat(anim: Anim, ms: u64) -> Option<LetterBeat> {
    let (beats, mut t) = match anim {
        Anim::Letter => return Some((0, None, 0, false)),
        Anim::Stir => (STIR, ms % beats_ms(STIR)),
        Anim::Waking if ms < beats_ms(MORPH) => (MORPH, ms),
        _ => return None,
    };
    for b in beats {
        if t < u64::from(b.0) {
            return Some(*b);
        }
        t -= u64::from(b.0);
    }
    None
}
/// How long the launch's last act plays before the shin settles (T-1156).
pub(crate) fn ready_ms() -> u64 {
    READY.iter().map(|f| u64::from(f.ms)).sum()
}

/// What the creature acts out: the empty seat's invitation, or a session's
/// state as the rail names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Anim {
    Seat,
    Spawning,
    Idle,
    Thinking,
    Working,
    NeedsYou,
    Done,
    Monitoring,
    Interrupted,
    Sleeping,
    Throttled,
    Failed,
    Exited,
    Unknown,
    /// The rail's ש on a ticket with no agent yet (T-1156): the letter of
    /// the intro, still.
    Letter,
    /// The ש stirring while the agent's worktree is made and its init
    /// script runs: a breath, a hold, now and then a shiver.
    Stir,
    /// The ש waking into the shin as the agent starts: the intro's morph,
    /// then asleep, half awake, awake, a look round; then `IDLE`.
    Waking,
    /// The agent ready, the launch over: a smile, a wave, the happy hop;
    /// then `IDLE`.
    Ready,
}

impl Anim {
    #[cfg(test)]
    pub(crate) const ALL: [Anim; 18] = [
        Anim::Seat,
        Anim::Spawning,
        Anim::Idle,
        Anim::Thinking,
        Anim::Working,
        Anim::NeedsYou,
        Anim::Done,
        Anim::Monitoring,
        Anim::Interrupted,
        Anim::Sleeping,
        Anim::Throttled,
        Anim::Failed,
        Anim::Exited,
        Anim::Unknown,
        Anim::Letter,
        Anim::Stir,
        Anim::Waking,
        Anim::Ready,
    ];

    /// A session's animation. Exhaustive: a new state does not compile until
    /// someone decides what the creature does in it. `thinking` is the peek's
    /// word for a turn with no tool in flight.
    pub(crate) fn of(state: &SessionState, thinking: bool) -> Anim {
        match state {
            SessionState::Spawning => Anim::Spawning,
            SessionState::Running if thinking => Anim::Thinking,
            SessionState::Running => Anim::Working,
            SessionState::RequiresAction { .. } => Anim::NeedsYou,
            SessionState::Idle { stop_reason: StopReason::EndTurn } => Anim::Done,
            SessionState::Idle { stop_reason: StopReason::Background | StopReason::Monitoring } => {
                Anim::Monitoring
            }
            SessionState::Idle { stop_reason: StopReason::Interrupted } => Anim::Interrupted,
            SessionState::Idle { stop_reason: StopReason::Unknown } => Anim::Idle,
            SessionState::Sleeping => Anim::Sleeping,
            SessionState::Throttled => Anim::Throttled,
            SessionState::Failed { .. } | SessionState::Exited { reason: ExitReason::Crashed } => {
                Anim::Failed
            }
            SessionState::Exited { .. } => Anim::Exited,
            SessionState::Unknown { .. } => Anim::Unknown,
        }
    }

    /// Where the old static mark stood: the invitation, and a session not
    /// yet spoken to. Mono's wordmark keeps exactly that reach.
    pub(crate) fn unspoken(self) -> bool {
        matches!(self, Anim::Seat | Anim::Spawning | Anim::Idle)
    }

    /// Frames played once from the moment the animation starts, then a
    /// cycle that repeats for as long as it lasts.
    fn timeline(self, size: Size) -> (&'static [Frame], &'static [Frame]) {
        match self {
            Anim::Seat => (&[], SEAT),
            Anim::Spawning => (SPAWNING, IDLE),
            Anim::Idle => (&[], IDLE),
            Anim::Thinking => (&[], THINKING),
            Anim::Working => (&[], working(size)),
            Anim::NeedsYou => (&[], NEEDS_YOU),
            Anim::Done => (DONE, CONTENT),
            Anim::Monitoring => (&[], MONITORING),
            Anim::Interrupted => (INTERRUPTED, LOOKING),
            Anim::Sleeping => (&[], SLEEPING),
            Anim::Throttled => (&[], THROTTLED),
            Anim::Failed => (&[], FAILED),
            Anim::Exited => (&[], EXITED),
            Anim::Unknown => (&[], UNKNOWN),
            // The letter's beats are pixels, not poses (`letter_beat`); the
            // cycle here is what a shin after them would do.
            Anim::Letter | Anim::Stir => (&[], IDLE),
            Anim::Waking => (WAKING, IDLE),
            Anim::Ready => (READY, IDLE),
        }
    }
}

/// The pose `ms` into `anim` at `size`.
pub(crate) fn frame_at(size: Size, anim: Anim, ms: u64) -> &'static Frame {
    let (intro, cycle) = anim.timeline(size);
    let total = |frames: &[Frame]| frames.iter().map(|f| u64::from(f.ms)).sum::<u64>();
    let intro_ms = total(intro);
    let (frames, mut t) =
        if ms < intro_ms { (intro, ms) } else { (cycle, (ms - intro_ms) % total(cycle).max(1)) };
    for fr in frames {
        if t < u64::from(fr.ms) {
            return fr;
        }
        t -= u64::from(fr.ms);
    }
    &frames[0]
}

// -- composing ----------------------------------------------------------------

/// What a pixel or a mark is, before a theme says what colour that is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Hi,
    Body,
    Shade,
    Deep,
    Eye,
    Glint,
    Blush,
    Dim,
    Drop,
    Calm,
    Attn,
    Err,
    /// The laptop: its frame, its dark screen, and the diff on it.
    Bezel,
    Screen,
    Code,
    Add,
    /// The crown's tint on the stems (T-1186), lit as the body is: the
    /// lit edge, the face, the shaded edge.
    CrownHi,
    Crown,
    CrownDeep,
    /// Every stem pixel in the crown, as its lightning leaves the shin.
    CrownFlash,
}

impl Role {
    /// The body's own four values, which `light` gives a solid pixel.
    fn lit_body(self) -> bool {
        matches!(self, Role::Hi | Role::Body | Role::Shade | Role::Deep)
    }

    pub(crate) fn ink(self, ink: &CreatureInk) -> Color {
        match self {
            Role::Hi => ink.hi,
            Role::Body => ink.body,
            Role::Shade => ink.shade,
            Role::Deep => ink.deep,
            Role::Eye => ink.eye,
            Role::Glint => ink.glint,
            Role::Blush => ink.blush,
            Role::Dim => ink.dim,
            Role::Drop => ink.drop,
            Role::Calm => ink.calm,
            Role::Attn => ink.attn,
            Role::Err => ink.err,
            Role::Bezel => ink.bezel,
            Role::Screen => ink.screen,
            Role::Code => ink.code,
            Role::Add => ink.add,
            Role::CrownHi => ink.crown.map_or(ink.hi, |c| c.lit),
            Role::Crown => ink.crown.map_or(ink.body, |c| c.tint),
            Role::CrownDeep => ink.crown.map_or(ink.shade, |c| c.shade),
            Role::CrownFlash => ink.crown.map_or(ink.glint, |c| c.flash),
        }
    }
}

/// A text glyph beside the body, at a cell relative to the body's top-left
/// cell (negative is above or left of it).
#[derive(Debug, Clone, Copy)]
struct Mark {
    col: i16,
    row: i16,
    ch: char,
    role: Role,
    bold: bool,
}

/// One frame, composed: every pixel's role, and the marks.
pub(crate) struct Picture {
    pub(crate) w: i16,
    pub(crate) h: i16,
    px: Vec<Option<Role>>,
    lift: bool,
    marks: Vec<Mark>,
}

impl Picture {
    /// The role at screen pixel (x, y), the lift applied.
    pub(crate) fn at(&self, x: i16, y: i16) -> Option<Role> {
        let y = y + i16::from(self.lift);
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }
}

/// The room a frame's prop asks for beside the body, when it has one that
/// fits this size.
fn scene_pad(size: Size, fr: &Frame) -> Option<i16> {
    match fr.scene? {
        Scene::Laptop { .. } => laptop(size).map(|l| l.pad),
        Scene::Cloud(_) => cloud(size).map(|c| c.pad),
    }
}

/// Rim light over a grid's `Body` pixels: top and left edges catch it, the
/// right edge falls away, and an underside below row `top` (the arms are
/// above it) sits in the deepest shade.
pub(crate) fn light(grid: &[Option<Role>], w: i16, h: i16, top: i16) -> Vec<Option<Role>> {
    let solid =
        |x: i16, y: i16| x >= 0 && y >= 0 && x < w && y < h && grid[(y * w + x) as usize].is_some();
    let mut px = grid.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if grid[i] != Some(Role::Body) {
                continue;
            }
            px[i] = Some(if !solid(x, y + 1) {
                if y >= top {
                    Role::Deep
                } else {
                    Role::Shade
                }
            } else if !solid(x, y - 1) || !solid(x - 1, y) {
                Role::Hi
            } else if !solid(x + 1, y) {
                Role::Shade
            } else {
                Role::Body
            });
        }
    }
    px
}

/// Move the pixels of `rows` whose column passes `cols` by `dx`.
fn shift(
    grid: &mut [Option<Role>],
    w: i16,
    rows: RangeInclusive<i16>,
    cols: impl Fn(i16) -> bool,
    dx: i16,
) {
    for y in rows {
        let row = &mut grid[(y * w) as usize..((y + 1) * w) as usize];
        let old = row.to_vec();
        for x in (0..w).filter(|&x| cols(x)) {
            row[x as usize] = None;
        }
        for x in (0..w).filter(|&x| cols(x) && old[x as usize].is_some()) {
            if (0..w).contains(&(x + dx)) {
                row[(x + dx) as usize] = old[x as usize];
            }
        }
    }
}

/// One frame, composed `pad` pixels wider than the body so a scene fits
/// beside it (`anim_pad`: every frame of an animation gets its widest).
pub(crate) fn compose(size: Size, fr: &Frame, pad: i16) -> Picture {
    compose_crowned(size, fr, pad, Gild::default())
}

/// `compose` with the crown's tint over the stems' top `gild.rows` rows
/// (T-1186), before the props and the scene: the status light still lights
/// over it, and a cloud or a laptop beside the shin keeps its own inks.
fn compose_crowned(size: Size, fr: &Frame, pad: i16, gild: Gild) -> Picture {
    let b = Body::get(size);
    let (bw, h) = (b.w, b.h);
    let w = bw + pad;
    let idx = |x: i16, y: i16| (x >= 0 && y >= 0 && x < w && y < h).then(|| (y * w + x) as usize);
    let mut grid: Vec<Option<Role>> = vec![None; (w * h) as usize];
    for y in 0..h {
        for x in (0..bw).filter(|&x| b.solid(x, y)) {
            grid[(y * w + x) as usize] = Some(Role::Body);
        }
    }

    // The arms first, so the face is stamped on a body that stands still.
    let split = b.split;
    if fr.wave != 0 {
        shift(&mut grid, w, 0..=b.top / 2, |x| x >= split, fr.wave);
    }
    if fr.wig != 0 {
        shift(&mut grid, w, 0..=b.top / 4, |x| x < split, -fr.wig);
        shift(&mut grid, w, 0..=b.top / 4, |x| x >= split, fr.wig);
    }
    if fr.tilt != 0 {
        shift(&mut grid, w, 0..=b.top / 2, |_| true, fr.tilt);
    }

    let stamp = |grid: &mut Vec<Option<Role>>, pat: &[&str], ax: i16, ay: i16| {
        for (dy, line) in pat.iter().enumerate() {
            for (dx, ch) in line.bytes().enumerate() {
                let role = match ch {
                    b'o' => Role::Eye,
                    b'*' => Role::Glint,
                    _ => continue,
                };
                if let Some(i) = idx(ax + dx as i16, ay + dy as i16) {
                    grid[i] = Some(role);
                }
            }
        }
    };
    for &(x, y) in &b.eyes {
        stamp(&mut grid, eye(size, fr.eyes), x + fr.look.0, y + fr.look.1);
    }
    stamp(&mut grid, mouth(size, fr.mouth), b.mouth.0, b.mouth.1);
    if fr.blush {
        for &(x, y) in &b.blush {
            for i in (0..size.blush_w()).filter_map(|dx| idx(x + dx, y)) {
                if grid[i] == Some(Role::Body) {
                    grid[i] = Some(Role::Blush);
                }
            }
        }
    }

    let mut px = light(&grid, w, h, b.top);
    for cell in px.iter_mut().take((gild.rows.min(b.top) * w) as usize) {
        *cell = match *cell {
            Some(r) if gild.flash && r.lit_body() => Some(Role::CrownFlash),
            Some(Role::Hi) => Some(Role::CrownHi),
            Some(Role::Body) => Some(Role::Crown),
            Some(Role::Shade | Role::Deep) => Some(Role::CrownDeep),
            other => other,
        };
    }

    let mut marks = Vec::new();
    let mut mark = |(col, row): (i16, i16), ch: char, role: Role| {
        marks.push(Mark { col, row, ch, role, bold: role == Role::Attn });
    };
    // Marks stand around the body itself, not around its scene.
    let zz = [(bw - 1, 1), (bw, 0), (bw + 1, -1)];
    let sparkle = [[(-1, 3), (bw, 1)], [(-1, 1), (bw, 3)]];
    let stars = [(bw / 5, -1), (2 * bw / 5, -1), (3 * bw / 5, -1), (4 * bw / 5, -1)];
    for &p in fr.props {
        match p {
            Prop::Ask => mark((bw / 2, -1), '?', Role::Dim),
            Prop::Bang => mark((bw - 1, 0), '!', Role::Attn),
            Prop::Z(i) => mark(zz[i.min(2)], 'z', Role::Dim),
            Prop::BigZ(i) => mark(zz[i.min(2)], 'Z', Role::Dim),
            Prop::Sparkle(i) => {
                for at in sparkle[i.min(1)] {
                    mark(at, if i == 0 { '✦' } else { '✧' }, Role::Calm);
                }
            }
            Prop::Star(i) => mark(stars[i.min(3)], '✶', Role::Err),
            Prop::Light => {
                if let Some(i) = b.light.and_then(|(x, y)| idx(x, y)) {
                    px[i] = Some(Role::Calm);
                }
            }
            Prop::Drop(i) => {
                let at = b.drops.get(i).or(b.drops.last()).and_then(|&(x, y)| idx(x, y));
                if let Some(i) = at.filter(|&i| px[i].is_none()) {
                    px[i] = Some(Role::Drop);
                }
            }
        }
    }
    // The crown landing: a sparkle either side of its head, in its tint.
    if gild.cheer {
        mark((-1, -1), '✦', Role::CrownHi);
        mark((bw, -1), '✦', Role::CrownHi);
    }
    // The scene last, over everything: the hand rests in front of the body.
    let mut put = |x: i16, y: i16, role: Role| {
        if let Some(i) = idx(x, y) {
            px[i] = Some(role);
        }
    };
    match fr.scene {
        Some(Scene::Laptop { line, chars, press }) => {
            if let Some(l) = laptop(size) {
                l.draw(line as usize, i16::from(chars), press, &mut put);
            }
        }
        Some(Scene::Cloud(dots)) => {
            if let Some(c) = cloud(size) {
                c.draw(dots as usize, &mut put);
            }
        }
        None => {}
    }
    Picture { w, h, px, lift: fr.lift, marks }
}

// -- the scenes -----------------------------------------------------------------

/// The laptop beside a body, open and facing the reader: its screen's bezel,
/// the deck in front of it, the hand's two poses, the key a press lights,
/// the columns it adds to the stage, and the code its screen shows: lines
/// as (indent, length), typed in turn and scrolled up `rows` at a time on
/// every other row of the screen, the last one wrapping to the first.
struct Laptop {
    screen: (i16, i16, i16, i16),
    deck: &'static [(i16, i16, i16, Role)],
    up: &'static [(i16, i16, Role)],
    down: &'static [(i16, i16, Role)],
    key: (i16, i16),
    pad: i16,
    rows: usize,
    code: &'static [(i16, i16)],
}

const SMALL_LAPTOP: Laptop = Laptop {
    screen: (13, 2, 18, 7),
    deck: &[(12, 19, 8, Role::Bezel), (11, 20, 9, Role::Deep)],
    up: &[(12, 6, Role::Hi), (13, 6, Role::Hi), (12, 7, Role::Body), (13, 7, Role::Shade)],
    down: &[(12, 7, Role::Hi), (13, 7, Role::Hi), (12, 8, Role::Body), (13, 8, Role::Shade)],
    key: (14, 8),
    pad: 9,
    rows: 2,
    code: &[(0, 3), (1, 2), (1, 1)],
};

const MEDIUM_LAPTOP: Laptop = Laptop {
    screen: (22, 5, 30, 12),
    deck: &[(21, 31, 13, Role::Bezel), (20, 32, 14, Role::Deep)],
    up: &[
        (20, 11, Role::Hi),
        (21, 11, Role::Hi),
        (22, 11, Role::Hi),
        (20, 12, Role::Body),
        (21, 12, Role::Body),
        (22, 12, Role::Shade),
    ],
    down: &[
        (20, 12, Role::Hi),
        (21, 12, Role::Hi),
        (22, 12, Role::Hi),
        (20, 13, Role::Body),
        (21, 13, Role::Body),
        (22, 13, Role::Shade),
    ],
    key: (23, 13),
    pad: 13,
    rows: 3,
    code: &[(0, 4), (1, 3), (1, 2)],
};

fn laptop(size: Size) -> Option<&'static Laptop> {
    match size {
        Size::Small => Some(&SMALL_LAPTOP),
        Size::Medium => Some(&MEDIUM_LAPTOP),
        Size::Large => None,
    }
}

impl Laptop {
    /// The screen with line `line` being typed, `chars` of it so far: the
    /// lines before it quiet above, it in `calm`, the cursor after it.
    fn draw(&self, line: usize, chars: i16, press: bool, put: &mut impl FnMut(i16, i16, Role)) {
        let (x0, y0, x1, y1) = self.screen;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let edge = y == y0 || y == y1 || x == x0 || x == x1;
                put(x, y, if edge { Role::Bezel } else { Role::Screen });
            }
        }
        let n = self.code.len();
        let mut end = (x0 + 1, y0 + 1);
        for row in 0..self.rows {
            let back = self.rows - 1 - row;
            let (indent, len) = self.code[(line + n * self.rows - back) % n];
            let (len, role) =
                if back == 0 { (chars.min(len), Role::Add) } else { (len, Role::Code) };
            let y = y0 + 1 + 2 * row as i16;
            for x in 0..len {
                put(x0 + 1 + indent + x, y, role);
            }
            end = (x0 + 1 + indent + len, y);
        }
        put(end.0, end.1, Role::Glint);
        for &(from, to, y, role) in self.deck {
            for x in from..=to {
                put(x, y, role);
            }
        }
        for &(x, y, role) in if press { self.down } else { self.up } {
            put(x, y, role);
        }
        if press {
            put(self.key.0, self.key.1, Role::Glint);
        }
    }
}

/// The thought cloud beside a body: the trailing bubbles (x, y, side), the
/// cloud's own pixels from its top-left, its dots and their size, and the
/// columns it adds to the stage.
struct Cloud {
    trail: &'static [(i16, i16, i16)],
    rows: &'static [&'static str],
    at: (i16, i16),
    dots: [(i16, i16); 3],
    dot: i16,
    pad: i16,
}

const SMALL_CLOUD: Cloud = Cloud {
    trail: &[(12, 3, 1)],
    rows: &[".##.###.", "########", "########", "########", ".###.##."],
    at: (14, 0),
    dots: [(2, 2), (4, 2), (6, 2)],
    dot: 1,
    pad: 10,
};

const MEDIUM_CLOUD: Cloud = Cloud {
    trail: &[(18, 5, 1), (20, 3, 2)],
    rows: &[
        "..###..###..",
        ".##########.",
        "############",
        "############",
        "############",
        ".##########.",
        "...###.##...",
    ],
    at: (23, 0),
    dots: [(2, 3), (5, 3), (8, 3)],
    dot: 2,
    pad: 15,
};

fn cloud(size: Size) -> Option<&'static Cloud> {
    match size {
        Size::Small => Some(&SMALL_CLOUD),
        Size::Medium => Some(&MEDIUM_CLOUD),
        Size::Large => None,
    }
}

impl Cloud {
    fn draw(&self, dots: usize, put: &mut impl FnMut(i16, i16, Role)) {
        for &(x, y, side) in self.trail {
            for dy in 0..side {
                for dx in 0..side {
                    put(x + dx, y + dy, Role::Hi);
                }
            }
        }
        let (ax, ay) = self.at;
        for (y, row) in self.rows.iter().enumerate() {
            for (x, ch) in row.bytes().enumerate() {
                if ch == b'#' {
                    put(ax + x as i16, ay + y as i16, Role::Hi);
                }
            }
        }
        for &(x, y) in self.dots.iter().take(dots) {
            for dy in 0..self.dot {
                for dx in 0..self.dot {
                    put(ax + x + dx, ay + y + dy, Role::Eye);
                }
            }
        }
    }
}

/// How much wider than the body an animation draws: its widest scene.
fn anim_pad(size: Size, anim: Anim) -> i16 {
    let (intro, cycle) = anim.timeline(size);
    intro.iter().chain(cycle).filter_map(|fr| scene_pad(size, fr)).max().unwrap_or(0)
}

// -- the stage ------------------------------------------------------------------

/// Room around the body for its props: a column left, three right, and a
/// row above, which is as high as any mark stands.
pub(crate) const LEFT: u16 = 1;
const RIGHT: u16 = 3;
const ABOVE: u16 = 1;

/// A picture in terminal cells, inked: `None` is a cell the creature leaves
/// alone. `body` is the body's own columns, which a layout centres; the rest
/// is margin and scene.
pub(crate) struct Stage {
    pub cols: u16,
    pub rows: u16,
    pub body: u16,
    /// Columns the stage is drawn left of a body stage's place, so its
    /// body stands where the shin's does: the letter's margin (T-1156).
    pub shift: u16,
    cells: Vec<Option<(char, Style)>>,
}

/// The stage for `anim`, `ms` into it, in `theme`'s inks. `None` where the
/// theme draws no picture (mono). Every frame of one animation has the same
/// stage, so a reply beside it wraps once per state, not once per frame.
/// `blink`: a click's blink (T-1177) shuts the frame's eyes where they are
/// open, and nothing else of the frame or its clock changes; the letter has
/// no face to blink. `crown`: the crown on the stems (T-1186), in the
/// crown's tint where the theme paints tags and nowhere else; the letter
/// wears none.
pub(crate) fn stage_for(
    size: Size,
    anim: Anim,
    ms: u64,
    blink: bool,
    crown: Crown,
    theme: &Theme,
) -> Option<Stage> {
    // The ש (T-1156): the intro's letter or its morph, on the body's own
    // pixels, so its stage is the shin's to the cell.
    // A size with no letter (Large) plays the poses instead.
    if let Some((_, t, dx, lift)) = letter_beat(anim, ms) {
        if let Some((pad, px)) = crate::intro::letter_pixels(size, t, dx) {
            let ink = theme.creature_ink(false)?;
            let b = Body::get(size);
            let pic = Picture { w: b.w + 2 * pad, h: b.h, px, lift, marks: Vec::new() };
            let mut st = stage(&pic, &ink, theme.block_gaps);
            st.body = b.w as u16;
            st.shift = pad as u16;
            return Some(st);
        }
    }
    // The waking shin's poses start where its morph ends.
    let ms = if anim == Anim::Waking { ms.saturating_sub(beats_ms(MORPH)) } else { ms };
    let mut frame = *frame_at(size, anim, ms);
    let ink = theme.creature_ink(frame.faded)?;
    let gild = if ink.crown.is_some() { crown.gild(size) } else { Gild::default() };
    // The crown landing: open eyes turn happy and the mouth grins; a shin
    // asleep, shut or failed keeps its face.
    if gild.cheer && frame.eyes.shut_by_a_click() {
        frame.eyes = Eyes::Happy;
        frame.mouth = Mouth::Grin;
    }
    if blink && frame.eyes.shut_by_a_click() {
        frame.eyes = Eyes::Blink;
    }
    let pic = compose_crowned(size, &frame, anim_pad(size, anim), gild);
    let mut st = stage(&pic, &ink, theme.block_gaps);
    st.body = Body::get(size).w as u16;
    Some(st)
}

/// Preserve both logical pixels while keeping dark facial details out of a
/// half-block's background on font-based renderers: that background otherwise
/// leaks above the glyph and duplicates eyes and mouths in the line leading.
pub(crate) fn pixel_cell(
    top: Option<Color>,
    bottom: Option<Color>,
    ink: &CreatureInk,
    block_gaps: bool,
) -> Option<(char, Style)> {
    match (top, bottom) {
        (Some(t), Some(b)) if t == b => Some(('█', Style::default().fg(t).bg(t))),
        (Some(t), Some(b)) if block_gaps && b == ink.eye => {
            Some(('▄', Style::default().fg(b).bg(t)))
        }
        (Some(t), Some(b)) => Some(('▀', Style::default().fg(t).bg(b))),
        (Some(t), None) => Some(('▀', Style::default().fg(t))),
        (None, Some(b)) => Some(('▄', Style::default().fg(b))),
        (None, None) => None,
    }
}

/// A scene's pixels, inked: one column and half a row each. The intro and
/// the loader paint into one and `blit` it as half blocks, the way `stage`
/// lays a picture.
pub(crate) struct Canvas {
    w: i16,
    h: i16,
    px: Vec<Option<Color>>,
}

impl Canvas {
    pub(crate) fn new(w: i16, h: i16) -> Canvas {
        Canvas { w, h, px: vec![None; w.max(0) as usize * h.max(0) as usize] }
    }

    pub(crate) fn put(&mut self, x: i16, y: i16, c: Color) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = Some(c);
        }
    }

    pub(crate) fn get(&self, x: i16, y: i16) -> Option<Color> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }

    /// Pixels to half blocks from `origin`, inside the buffer's area.
    pub(crate) fn blit(
        &self,
        buf: &mut Buffer,
        origin: Position,
        ink: &CreatureInk,
        block_gaps: bool,
    ) {
        for r in 0..self.h / 2 {
            for c in 0..self.w {
                let (t, b) = (self.get(c, 2 * r), self.get(c, 2 * r + 1));
                let Some((ch, style)) = pixel_cell(t, b, ink, block_gaps) else { continue };
                let at = Position::new(origin.x + c as u16, origin.y + r as u16);
                if buf.area.contains(at) {
                    buf[at].set_char(ch).set_style(style);
                }
            }
        }
    }
}

fn stage(pic: &Picture, ink: &CreatureInk, block_gaps: bool) -> Stage {
    let above = ABOVE;
    let cols = pic.w as u16 + LEFT + RIGHT;
    let rows = pic.h as u16 / 2 + above;
    let mut cells = vec![None; cols as usize * rows as usize];
    for r in 0..rows {
        for c in 0..cols {
            let x = c as i16 - LEFT as i16;
            let y = (r as i16 - above as i16) * 2;
            let paint = |y: i16| pic.at(x, y).map(|role| role.ink(ink));
            let (top, bottom) = (paint(y), paint(y + 1));
            cells[(r * cols + c) as usize] = pixel_cell(top, bottom, ink, block_gaps);
        }
    }
    for m in &pic.marks {
        let (c, r) = (m.col + LEFT as i16, m.row + above as i16);
        if c < 0 || r < 0 || c >= cols as i16 || r >= rows as i16 {
            continue;
        }
        let cell = &mut cells[(r as u16 * cols + c as u16) as usize];
        if cell.is_none() {
            let style = Style::default().fg(m.role.ink(ink));
            *cell = Some((m.ch, if m.bold { style.add_modifier(Modifier::BOLD) } else { style }));
        }
    }
    Stage { cols, rows, body: pic.w as u16, shift: 0, cells }
}

impl Stage {
    fn row(&self, r: u16) -> &[Option<(char, Style)>] {
        &self.cells[(r * self.cols) as usize..((r + 1) * self.cols) as usize]
    }

    /// Row `r` as a line, `pad` columns in. Trailing empty cells are left
    /// off, so the line is no wider than what it draws.
    pub(crate) fn line(&self, r: u16, pad: usize) -> Line<'static> {
        let row = self.row(r);
        let Some(last) = row.iter().rposition(Option::is_some) else {
            return Line::default();
        };
        let mut spans = vec![Span::raw(" ".repeat(pad))];
        let mut gap = 0;
        for cell in &row[..=last] {
            match cell {
                None => gap += 1,
                Some((ch, style)) => {
                    if gap > 0 {
                        spans.push(Span::raw(" ".repeat(gap)));
                        gap = 0;
                    }
                    spans.push(Span::styled(ch.to_string(), *style));
                }
            }
        }
        Line::from(spans)
    }

    /// Paint the stage with its top-left cell at (x, y), inside `clip`. A
    /// pixel cell always lands; a mark only on a blank cell, so a prop never
    /// writes over a word.
    pub(crate) fn paint(&self, buf: &mut Buffer, x: u16, y: u16, clip: Rect) {
        for r in 0..self.rows {
            for (c, cell) in self.row(r).iter().enumerate() {
                let Some((ch, style)) = cell else { continue };
                let at = Position::new(x + c as u16, y + r);
                if !clip.contains(at) || !buf.area.contains(at) {
                    continue;
                }
                let target = &mut buf[at];
                if !matches!(ch, '▀' | '▄' | '█') && target.symbol() != " " {
                    continue;
                }
                target.set_char(*ch).set_style(*style);
            }
        }
    }

    /// The record the L1 law reads: what this stage puts where, when its
    /// top-left cell is at (x, y).
    pub(crate) fn drawn(&self, x: u16, y: u16) -> Drawn {
        Drawn {
            rect: Rect::new(x, y, self.cols, self.rows),
            glyphs: self.cells.iter().map(|c| c.map_or(' ', |(ch, _)| ch)).collect(),
        }
    }
}

/// Where the creature stood in the last frame, and what it drew there. The
/// L1 law admits `▄` and `█` at exactly these cells and nowhere else.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Drawn {
    pub rect: Rect,
    glyphs: Vec<char>,
}

impl Drawn {
    /// The glyph this draw put at (x, y), or `None` off the stage.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn glyph(&self, x: u16, y: u16) -> Option<char> {
        let r = self.rect;
        (x >= r.x && y >= r.y && x < r.right() && y < r.bottom())
            .then(|| self.glyphs[((y - r.y) * r.width + (x - r.x)) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};
    use mesimon_core::board::{FailReason, Reason, UnknownReason};

    const SIZES: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    fn frames(size: Size, anim: Anim) -> Vec<&'static Frame> {
        let (intro, cycle) = anim.timeline(size);
        intro.iter().chain(cycle).collect()
    }

    /// The crown (T-1186): worn, the stems' top rows take the crown's inks,
    /// lit as the body is, and nothing else of any frame changes: the face,
    /// the props, a scene beside the shin and every row under the crown.
    #[test]
    fn the_crown_tints_the_stems_top_and_nothing_else() {
        for size in SIZES {
            let (b, rows) = (Body::get(size), size.crown_rows());
            for anim in Anim::ALL {
                let pad = anim_pad(size, anim);
                for fr in frames(size, anim) {
                    let off = compose(size, fr, pad);
                    let on = compose_crowned(size, fr, pad, Crown::On.gild(size));
                    let mut tinted = 0;
                    for (i, (was, is)) in off.px.iter().zip(&on.px).enumerate() {
                        let (x, y) = (i as i16 % on.w, i as i16 / on.w);
                        if was != is {
                            assert!(y < rows, "{size:?} {anim:?}: ({x},{y}) under the crown");
                            let want = match was {
                                Some(Role::Hi) => Role::CrownHi,
                                Some(Role::Body) => Role::Crown,
                                _ => Role::CrownDeep,
                            };
                            assert_eq!(*is, Some(want), "{size:?} {anim:?}: ({x},{y})");
                            tinted += 1;
                        }
                        // Every stem pixel up there wears it; a scene's own
                        // pixels stand right of the arms' furthest lean.
                        if y < rows && x < b.w + 2 {
                            assert!(!is.is_some_and(Role::lit_body), "{size:?} {anim:?} ({x},{y})");
                        }
                    }
                    assert!(tinted > 0, "{size:?} {anim:?}: no crown");
                    let marks = |p: &Picture| {
                        p.marks.iter().map(|m| (m.col, m.row, m.ch)).collect::<Vec<_>>()
                    };
                    assert_eq!(marks(&off), marks(&on), "{size:?} {anim:?}: the props moved");
                }
            }
        }
    }

    /// The crown's beats (T-1186): it fills the stems from the top and the
    /// shin cheers once it is whole; it drains back up off them before
    /// `CROWN_OFF_MS`; it flashes on the redraw clock's steps and settles
    /// worn by `CROWN_ACT_MS`.
    #[test]
    fn the_crown_fills_down_drains_up_and_flashes() {
        for size in SIZES {
            let full = size.crown_rows();
            assert_eq!(Crown::Off.gild(size), Gild::default());
            assert_eq!(Crown::On.gild(size), Gild { rows: full, ..Gild::default() });
            let coming: Vec<Gild> =
                (0..2_000).step_by(100).map(|ms| Crown::Coming(ms).gild(size)).collect();
            assert_eq!(coming[0].rows, 1, "{size:?}: the top row first");
            assert!(coming.windows(2).all(|w| w[0].rows <= w[1].rows), "{size:?}: it fills");
            assert!(
                coming.iter().all(|g| !g.cheer || g.rows == full),
                "{size:?}: cheer when whole"
            );
            assert!(coming.iter().any(|g| g.cheer), "{size:?}: and it cheers");
            let going: Vec<i16> =
                (0..CROWN_OFF_MS).step_by(100).map(|ms| Crown::Going(ms).gild(size).rows).collect();
            assert!(going[0] < full, "{size:?}: it leaves at once");
            assert!(going.windows(2).all(|w| w[0] >= w[1]), "{size:?}: it drains");
            assert_eq!(going.last(), Some(&0), "{size:?}: off before CROWN_OFF_MS");
            let flash = |ms| Crown::Acting(ms).gild(size);
            assert!(flash(0).flash && !flash(CROWN_STEP_MS).flash, "{size:?}: it flashes");
            assert!(flash(2 * CROWN_STEP_MS).flash, "{size:?}: twice");
            assert_eq!(flash(CROWN_ACT_MS), Crown::On.gild(size), "{size:?}: then is worn");
        }
    }

    /// The crown in a theme (T-1186): the crown's own tint where the theme
    /// paints tags, nothing where it does not, and none on the letter.
    #[test]
    fn the_crown_is_the_crown_s_tint_and_only_where_tags_are_painted() {
        let cells = |theme: &Theme, anim, crown| {
            stage_for(Size::Small, anim, 0, false, crown, theme).map(|s| s.cells)
        };
        let tc = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = tc.creature_ink(false).and_then(|i| i.crown).expect("tinted");
        let (lit, tint, shade) = (ink.lit, ink.tint, ink.shade);
        assert_eq!(tc.crown_text().fg, Some(tint), "the crown's own tint");
        let worn = cells(&tc, Anim::Idle, Crown::On).expect("truecolor draws it");
        assert_ne!(Some(&worn), cells(&tc, Anim::Idle, Crown::Off).as_ref());
        let crowned = |c: Option<Color>| c.is_some_and(|c| [lit, tint, shade].contains(&c));
        assert!(
            worn.iter().flatten().any(|(_, st)| crowned(st.fg) || crowned(st.bg)),
            "the crown's inks on the stems"
        );
        let landing = stage_for(Size::Small, Anim::Idle, 1_000, false, Crown::Coming(1_000), &tc)
            .expect("drawn");
        assert!(landing.cells.iter().flatten().any(|(ch, _)| *ch == '✦'), "a sparkle as it lands");
        // The flash stands off the ground on every flavor: a light ground's
        // own colour would erase the stems instead of lighting them.
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let Some(c) = theme.creature_ink(false).and_then(|i| i.crown) else { continue };
            assert_ne!(theme.bg, Some(c.flash), "{flavor:?}: the flash is the ground");
            assert_ne!(c.flash, c.lit, "{flavor:?}: the flash is the crown's lit edge");
        }
        let plain = Theme::new(Flavor::Graphite, Profile::Ansi256);
        assert_eq!(cells(&plain, Anim::Idle, Crown::On), cells(&plain, Anim::Idle, Crown::Off));
        assert_eq!(cells(&tc, Anim::Letter, Crown::On), cells(&tc, Anim::Letter, Crown::Off));
    }

    /// A click's blink (T-1177): every frame whose eyes are open draws them
    /// shut and changes nothing else; a frame with its eyes already shut,
    /// happy or X'd, and the letter, draws as it always does.
    #[test]
    fn a_click_blinks_open_eyes_and_leaves_the_rest_of_the_frame() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for size in SIZES {
            for anim in Anim::ALL {
                let (intro, cycle) = anim.timeline(size);
                let span: u64 = intro.iter().chain(cycle).map(|f| u64::from(f.ms)).sum::<u64>()
                    + if anim == Anim::Waking { beats_ms(MORPH) } else { 0 };
                for ms in (0..span).step_by(100) {
                    let cells = |blink| {
                        stage_for(size, anim, ms, blink, Crown::Off, &theme).map(|s| s.cells)
                    };
                    let (open, blinked) = (cells(false), cells(true));
                    let pose_ms =
                        if anim == Anim::Waking { ms.saturating_sub(beats_ms(MORPH)) } else { ms };
                    let fr = frame_at(size, anim, pose_ms);
                    // The letter has no face; Large has no letter.
                    let face = letter_beat(anim, ms).is_none() || size == Size::Large;
                    if face && fr.eyes.shut_by_a_click() {
                        assert_ne!(open, blinked, "{size:?} {anim:?} at {ms}: the eyes shut");
                        let as_blink = fr.eyes(Eyes::Blink);
                        let ink = theme.creature_ink(fr.faded).expect("truecolor");
                        let pic = compose(size, &as_blink, anim_pad(size, anim));
                        assert_eq!(
                            blinked,
                            Some(stage(&pic, &ink, theme.block_gaps).cells),
                            "{size:?} {anim:?} at {ms}: the same frame, its eyes shut"
                        );
                    } else {
                        assert_eq!(open, blinked, "{size:?} {anim:?} at {ms}: nothing to blink");
                    }
                }
            }
        }
    }

    #[test]
    fn the_source_parses_into_three_shins_with_their_anchors() {
        for size in SIZES {
            let b = Body::get(size);
            assert!(b.w >= 12 && b.h >= 10 && b.h % 2 == 0, "{size:?}: {}x{}", b.w, b.h);
            assert_eq!(b.eyes.len(), 2, "{size:?}: two eyes");
            assert_eq!(b.blush.len(), 2, "{size:?}: two cheeks");
            assert!(b.solid(b.mouth.0, b.mouth.1), "{size:?}: the mouth sits on the body");
            // The letter: the right arm stands apart, and the left run of row
            // 0 holds two heads (the middle arm joins the left arm below).
            let heads = b.runs(0);
            assert!(heads.len() >= 2, "{size:?}: arms in row 0: {heads:?}");
            assert!(b.split > b.w / 2 && b.top > 0 && b.top < b.h / 2, "{size:?}");
            for &(x, y) in &b.drops {
                assert!(!b.solid(x, y), "{size:?}: the drop falls through air at {x},{y}");
            }
            if let Some((x, y)) = b.light {
                assert!(b.solid(x, y) && x >= b.split, "{size:?}: the light is on the right arm");
            }
        }
        for size in [Size::Small, Size::Medium] {
            assert!(Body::get(size).light.is_some() && !Body::get(size).drops.is_empty());
        }
    }

    /// Every pose at every animated size: the face stays on the body, every
    /// mark lands inside the stage and off the body, and the arms never
    /// leave the drawing.
    #[test]
    fn every_pose_composes_inside_its_stage() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for size in [Size::Small, Size::Medium] {
            let b = Body::get(size);
            for anim in Anim::ALL {
                let pad = anim_pad(size, anim);
                for fr in frames(size, anim) {
                    let pic = compose(size, fr, pad);
                    let solid = pic.px.iter().filter(|p| p.is_some()).count();
                    let body = b.solid.iter().filter(|s| **s).count();
                    assert!(solid + 4 >= body, "{size:?} {anim:?}: the arms lost pixels");
                    for m in &pic.marks {
                        assert!(
                            m.col >= -(LEFT as i16)
                                && m.col < b.w + RIGHT as i16
                                && m.row >= -(ABOVE as i16),
                            "{size:?} {anim:?}: {:?} off the stage at {},{}",
                            m.ch,
                            m.col,
                            m.row
                        );
                    }
                    let ink = theme.creature_ink(fr.faded).expect("truecolor draws");
                    let st = stage(&pic, &ink, false);
                    for (ch, style) in st.cells.iter().flatten() {
                        if *ch == '█' {
                            assert_eq!(
                                style.bg, style.fg,
                                "{size:?} {anim:?}: solid pixels fill the cell"
                            );
                            assert!(style.bg.is_some());
                        }
                    }
                    let want = (b.w as u16 + pad as u16 + 4, b.h as u16 / 2 + ABOVE);
                    assert_eq!((st.cols, st.rows), want, "{size:?} {anim:?}");
                }
            }
        }
    }

    #[test]
    fn font_gap_fallback_keeps_every_pixel_and_never_backgrounds_a_half_eye() {
        let mut changed = 0;
        for flavor in Flavor::ALL {
            for profile in [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8] {
                let theme = Theme::new(flavor, profile);
                for size in SIZES {
                    for anim in Anim::ALL {
                        for frame in frames(size, anim) {
                            let ink = theme.creature_ink(frame.faded).unwrap();
                            let pic = compose(size, frame, anim_pad(size, anim));
                            let before = stage(&pic, &ink, false);
                            let after = stage(&pic, &ink, true);
                            assert_eq!((before.cols, before.rows), (after.cols, after.rows));
                            for (a, b) in before.cells.iter().zip(&after.cells) {
                                if a == b {
                                    continue;
                                }
                                changed += 1;
                                let (ac, ast) = a.unwrap();
                                let (bc, bst) = b.unwrap();
                                assert_eq!((ac, bc), ('▀', '▄'));
                                assert_eq!((ast.fg, ast.bg), (bst.bg, bst.fg));
                                assert_eq!(bst.fg, Some(ink.eye));
                                assert_ne!(bst.bg, Some(ink.eye));
                            }
                            for (ch, style) in after.cells.iter().flatten() {
                                if matches!(ch, '▀' | '▄') {
                                    assert_ne!(
                                        style.bg,
                                        Some(ink.eye),
                                        "{flavor:?}/{size:?}/{anim:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(changed > 0);
    }

    /// The redraw clock steps every 100 ms, so a frame of any other length
    /// would lie about its length; and every cycle has something to repeat.
    #[test]
    fn every_timeline_steps_on_the_redraw_clock() {
        for size in SIZES {
            for anim in Anim::ALL {
                let (intro, cycle) = anim.timeline(size);
                assert!(!cycle.is_empty(), "{anim:?}");
                for fr in intro.iter().chain(cycle) {
                    assert!(fr.ms > 0 && fr.ms % 100 == 0, "{anim:?}: {} ms", fr.ms);
                }
                let first = intro.first().unwrap_or(&cycle[0]);
                assert!(std::ptr::eq(frame_at(size, anim, 0), first));
            }
        }
        // An intro plays once and the cycle takes over for good.
        let intro: u64 = DONE.iter().map(|f| u64::from(f.ms)).sum();
        assert_eq!(frame_at(Size::Medium, Anim::Done, intro).eyes, Eyes::Open);
        assert_eq!(frame_at(Size::Medium, Anim::Done, 0).eyes, Eyes::Happy);
    }

    /// The one saturated colour keeps its one job on the creature too: only
    /// the needs-you pose wears `attn`, and it always does.
    #[test]
    fn only_needs_you_wears_attn() {
        for size in [Size::Small, Size::Medium] {
            for anim in Anim::ALL {
                let lit = frames(size, anim).into_iter().any(|fr| {
                    let pic = compose(size, fr, anim_pad(size, anim));
                    pic.marks.iter().any(|m| m.role == Role::Attn)
                        || pic.px.contains(&Some(Role::Attn))
                });
                assert_eq!(lit, anim == Anim::NeedsYou, "{size:?} {anim:?}");
            }
        }
    }

    #[test]
    fn every_state_has_its_animation() {
        use SessionState as S;
        let idle = |r| S::Idle { stop_reason: r };
        let cases = [
            (S::Spawning, false, Anim::Spawning),
            (S::Running, false, Anim::Working),
            (S::Running, true, Anim::Thinking),
            (S::RequiresAction { reason: Reason::Permission }, false, Anim::NeedsYou),
            (idle(StopReason::EndTurn), false, Anim::Done),
            (idle(StopReason::Background), false, Anim::Monitoring),
            (idle(StopReason::Monitoring), false, Anim::Monitoring),
            (idle(StopReason::Interrupted), false, Anim::Interrupted),
            (idle(StopReason::Unknown), false, Anim::Idle),
            (S::Sleeping, false, Anim::Sleeping),
            (S::Throttled, false, Anim::Throttled),
            (S::Failed { reason: FailReason::Server }, false, Anim::Failed),
            (S::Exited { reason: ExitReason::Crashed }, false, Anim::Failed),
            (S::Exited { reason: ExitReason::Killed }, false, Anim::Exited),
            (S::Unknown { reason: UnknownReason::NoSignal }, false, Anim::Unknown),
        ];
        for (state, thinking, want) in cases {
            assert_eq!(Anim::of(&state, thinking), want, "{state:?}");
        }
    }

    /// No two animations open on the same face but the two waits (the
    /// invitation and a session not yet prompted, which are one wait seen
    /// from either side of the press), and every one but the exited body
    /// moves within its first cycle.
    #[test]
    fn the_states_are_told_apart() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let glyphs = |fr: &Frame| {
            let ink = theme.creature_ink(fr.faded).expect("ink");
            stage(&compose(Size::Medium, fr, 0), &ink, false).cells
        };
        // The ש's two (T-1156) are pixels, not poses: `letter_beat`.
        let posed: Vec<Anim> =
            Anim::ALL.into_iter().filter(|a| !matches!(a, Anim::Letter | Anim::Stir)).collect();
        let firsts: Vec<_> = posed.iter().map(|&a| glyphs(frame_at(Size::Medium, a, 0))).collect();
        for (i, a) in posed.iter().enumerate() {
            for (j, b) in posed.iter().enumerate().skip(i + 1) {
                if [*a, *b] != [Anim::Seat, Anim::Idle] {
                    assert_ne!(firsts[i], firsts[j], "{a:?} and {b:?} open on the same face");
                }
            }
        }
        for anim in posed.into_iter().filter(|a| *a != Anim::Exited) {
            let moves = frames(Size::Medium, anim).windows(2).any(|w| glyphs(w[0]) != glyphs(w[1]));
            assert!(moves, "{anim:?} never moves");
        }
        assert_eq!(frames(Size::Medium, Anim::Exited).len(), 1, "the exited body is still");
    }

    /// Working is typing at a laptop in a loop with no seam (T-1150): the
    /// code fits the screen, each frame either types one more character or
    /// scrolls to the next line on an Enter (the last frame into the first
    /// too), the scrolled screen is the one before it moved up a row, the
    /// face never changes but for one blink, the head nods only on an Enter,
    /// and the hand comes down on every key.
    #[test]
    fn working_types_line_after_line_with_no_seam() {
        for size in [Size::Small, Size::Medium] {
            let l = laptop(size).expect("an animated size has a laptop");
            let (x0, y0, x1, y1) = l.screen;
            assert!(y0 + 1 + 2 * (l.rows as i16 - 1) < y1, "{size:?}: the rows fit the screen");
            for &(indent, len) in l.code {
                // A line and the cursor after it stay inside the screen.
                assert!(x0 + 1 + indent + len < x1, "{size:?}: a line runs off the screen");
            }
            let cycle = working(size);
            let at = |fr: &Frame| match fr.scene {
                Some(Scene::Laptop { line, chars, press }) => (line as usize, chars as i16, press),
                _ => panic!("{size:?}: every working frame is at the laptop"),
            };
            let pad = anim_pad(size, Anim::Working);
            let w = Body::get(size).w + pad;
            // Each screen row's code, as the columns it covers.
            let rows = |fr: &Frame| -> Vec<Vec<i16>> {
                let px = compose(size, fr, pad).px;
                (0..l.rows as i16)
                    .map(|r| {
                        let y = y0 + 1 + 2 * r;
                        (x0 + 1..x1)
                            .filter(|&x| {
                                matches!(px[(y * w + x) as usize], Some(Role::Code | Role::Add))
                            })
                            .collect()
                    })
                    .collect()
            };
            let n = l.code.len();
            for i in 0..cycle.len() {
                let (a, b) = (&cycle[i], &cycle[(i + 1) % cycle.len()]);
                let ((la, ca, _), (lb, cb, _)) = (at(a), at(b));
                if la == lb {
                    assert!(cb == ca || cb == ca + 1, "{size:?}: frame {i} types one at a time");
                } else {
                    assert_eq!(lb, (la + 1) % n, "{size:?}: frame {i} scrolls to the next line");
                    assert_eq!((ca, cb), (l.code[la].1, 0), "{size:?}: a full line, then Enter");
                    assert_eq!(rows(b)[..l.rows - 1], rows(a)[1..], "{size:?}: frame {i} scrolls");
                    assert!(b.tilt == 1, "{size:?}: the Enter nods");
                }
                if la == lb {
                    assert_eq!(b.tilt, 0, "{size:?}: frame {}: only an Enter nods", i + 1);
                }
                assert!(a.mouth == Mouth::Flat && a.look == (1, 0), "{size:?}: one face");
            }
            let blinks = cycle.iter().filter(|fr| fr.eyes == Eyes::Blink).count();
            assert!(cycle.iter().all(|fr| matches!(fr.eyes, Eyes::Open | Eyes::Blink)));
            assert_eq!(blinks, 1, "{size:?}: one blink a loop");
            let lines: std::collections::BTreeSet<usize> =
                cycle.iter().map(|fr| at(fr).0).collect();
            assert_eq!(lines.len(), n, "{size:?}: every line is typed");
            let press = cycle.iter().find(|fr| at(fr).2).expect("a press");
            let raised = cycle.iter().find(|fr| !at(fr).2).expect("a raise");
            let &(hx, hy, _) = l.down.last().expect("a hand");
            let hand = |fr: &Frame| compose(size, fr, pad).px[(hy * w + hx) as usize];
            assert_ne!(hand(press), hand(raised), "{size:?}: the hand moves");
        }
    }

    /// Thinking is white eyes rolled up and a cloud beside the head whose
    /// dots fill in; the cloud stays inside the stage it widens.
    #[test]
    fn thinking_rolls_its_eyes_up_at_a_cloud() {
        for size in [Size::Small, Size::Medium] {
            let pad = anim_pad(size, Anim::Thinking);
            let bw = Body::get(size).w;
            assert!(pad > 0, "{size:?}: the cloud widens the stage");
            let beside = |fr: &Frame, role: Role| {
                let pic = compose(size, fr, pad);
                (0..pic.h)
                    .any(|y| (bw..pic.w).any(|x| pic.px[(y * pic.w + x) as usize] == Some(role)))
            };
            assert!(THINKING.iter().all(|fr| beside(fr, Role::Hi)), "{size:?}: the cloud stays");
            assert!(THINKING.iter().any(|fr| beside(fr, Role::Eye)), "{size:?}: its dots fill in");
            assert!(THINKING.iter().any(|fr| !beside(fr, Role::Eye)), "{size:?}: and clear");
            let whites = compose(size, &THINKING[0], pad)
                .px
                .iter()
                .filter(|p| **p == Some(Role::Glint))
                .count();
            assert!(whites >= 4, "{size:?}: the eyes have whites to roll in");
        }
    }

    // -- the assets: goldens of this engine --------------------------------------

    fn golden(path: &std::path::Path, bytes: &[u8]) {
        if std::env::var_os("MESIMON_UPDATE_GOLDEN").is_some() {
            std::fs::write(path, bytes).expect("write asset");
            return;
        }
        let want = std::fs::read(path).unwrap_or_default();
        assert!(
            want == bytes,
            "{} drifted from the shin's pixels; review + MESIMON_UPDATE_GOLDEN=1",
            path.display()
        );
    }

    fn asset(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/mascot").join(name)
    }

    /// The installer's welcome: the large shin in two tones, because the
    /// installer writes no escapes. Every body pixel is ink and the face is
    /// the terminal's own ground showing through.
    fn installer_art() -> String {
        let pic = compose(Size::Large, frame_at(Size::Large, Anim::Seat, 0), 0);
        let ink = |x: i16, y: i16| pic.at(x, y).is_some_and(|r| r != Role::Eye);
        let mut out = String::new();
        for r in 0..pic.h / 2 {
            let row: String = (0..pic.w)
                .map(|x| match (ink(x, 2 * r), ink(x, 2 * r + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                })
                .collect();
            out.push_str("  ");
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn creature_the_installer_welcomes_with_the_large_shin() {
        let art = installer_art();
        assert_eq!(art.lines().count(), 12);
        assert!(art.lines().all(|l| l.chars().count() <= 26));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
        let original = std::fs::read_to_string(&path).expect("install.sh");
        let (start, end) = ("# mascot:start\n", "# mascot:end");
        let (before, tail) = original.split_once(start).expect("mascot:start");
        let (_, after) = tail.split_once(end).expect("mascot:end");
        let updated =
            format!("{before}{start}  cat <<'MESIMON_SHIN'\n{art}MESIMON_SHIN\n{end}{after}");
        golden(&path, updated.as_bytes());
    }

    /// A notification icon: the medium shin as pixel art on the graphite
    /// tile, ten image pixels to a shin pixel. Needs-you is the pose the
    /// ticket page opens it on, its "!" drawn in pixels beside the arm.
    fn icon_png(needs_you: bool) -> Vec<u8> {
        icon_png_on(needs_you, false)
    }

    /// `tab`: the TAB's needs-you icon (T-492). At the sixteen pixels a
    /// tab gives an icon the "!" is a fraction of a pixel, so that icon
    /// wears an attention-colour badge in its top-right corner — the
    /// board's one saturated colour, a quarter of the tile, readable from
    /// across the tab strip — and the shin keeps its own inks (a dark
    /// silhouette on the amber tile read as a black blob, dogfood
    /// 2026-09-29).
    fn icon_png_on(needs_you: bool, tab: bool) -> Vec<u8> {
        tile_png(256, 10, Some(54.0), needs_you, tab)
    }

    /// The shin on the graphite tile, `size` image pixels a side and `scale`
    /// to a shin pixel. `radius` rounds the tile's corners; `None` is a full
    /// bleed square, for a platform that cuts its own shape (T-497: a
    /// maskable icon, and the apple touch icon, whose transparency iOS
    /// paints black).
    fn tile_png(size: u32, scale: i32, radius: Option<f32>, needs_you: bool, tab: bool) -> Vec<u8> {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = theme.creature_ink(false).expect("truecolor draws");
        let anim = if needs_you { Anim::NeedsYou } else { Anim::Seat };
        let fr = frame_at(Size::Medium, anim, 0);
        let mut pic = compose(Size::Medium, fr, 0);
        if fr.props.contains(&Prop::Bang) {
            let x = pic.w - 1;
            for y in [0, 1, 2, 4] {
                pic.px[(y * pic.w + x) as usize] = Some(Role::Attn);
            }
        }
        let rgb = |c: Color| match c {
            Color::Rgb(r, g, b) => [r, g, b],
            other => panic!("truecolor ink {other:?}"),
        };
        let ground = rgb(theme.bg.expect("graphite paints its ground"));
        let badge = tab.then(|| rgb(theme.attn));
        // The badge: a disc in the top-right corner, its centre inside the
        // tile's rounded corner so it never spills past the tile's edge.
        let (bx, by, br) = (size as f32 - 62.0, 62.0, 46.0f32);
        let (ox, oy) =
            ((size as i32 - pic.w as i32 * scale) / 2, (size as i32 - pic.h as i32 * scale) / 2);
        // The tile's rounded corners, antialiased; everything inside it is
        // square pixels on whole image pixels, so it needs none.
        let coverage = |x: u32, y: u32| {
            let Some(radius) = radius else { return 255 };
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (px, py) =
                        (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                    let cx = px.clamp(radius, size as f32 - radius);
                    let cy = py.clamp(radius, size as f32 - radius);
                    if (px - cx).powi(2) + (py - cy).powi(2) <= radius * radius {
                        hits += 1;
                    }
                }
            }
            (hits * 255 / 16) as u8
        };
        let img = image::RgbaImage::from_fn(size, size, |x, y| {
            let (sx, sy) = ((x as i32 - ox).div_euclid(scale), (y as i32 - oy).div_euclid(scale));
            let role = pic.at(sx as i16, sy as i16);
            let mut px = role.map_or(ground, |r| rgb(r.ink(&ink)));
            if let Some(amber) = badge {
                // Antialiased on the same 4x4 grid as the corners.
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (fx, fy) = (
                            x as f32 + (sx as f32 + 0.5) / 4.0,
                            y as f32 + (sy as f32 + 0.5) / 4.0,
                        );
                        if (fx - bx).powi(2) + (fy - by).powi(2) <= br * br {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    let k = hits as f32 / 16.0;
                    for (c, a) in px.iter_mut().zip(amber) {
                        *c = (*c as f32 * (1.0 - k) + a as f32 * k).round() as u8;
                    }
                }
            }
            let [r, g, b] = px;
            image::Rgba([r, g, b, coverage(x, y)])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).expect("encode");
        out.into_inner()
    }

    fn web_icon(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/mesophon/icons").join(name)
    }

    /// Remote Control's home-screen icons (T-497): the resting shin on the
    /// graphite tile, at the notification icon's proportions. The maskable
    /// one bleeds to the edge and keeps the shin inside the central circle
    /// a platform may cut to (radius 40% of the side).
    #[test]
    fn creature_the_home_screen_icons_are_the_shin_s_own_pixels() {
        let pic = compose(Size::Medium, frame_at(Size::Medium, Anim::Seat, 0), 0);
        let half_diagonal = |scale: i32| {
            (((pic.w as i32 * scale) as f32 / 2.0).powi(2)
                + ((pic.h as i32 * scale) as f32 / 2.0).powi(2))
            .sqrt()
        };
        assert!(half_diagonal(15) <= 512.0 * 0.4, "the maskable shin leaves the safe zone");
        golden(&web_icon("icon-192.png"), &tile_png(192, 7, Some(40.5), false, false));
        golden(&web_icon("icon-512.png"), &tile_png(512, 20, Some(108.0), false, false));
        golden(&web_icon("maskable-512.png"), &tile_png(512, 15, None, false, false));
        golden(&web_icon("apple-touch-icon.png"), &tile_png(180, 7, None, false, false));
    }

    #[test]
    fn creature_the_notification_icons_are_the_shin_s_own_pixels() {
        let (resting, waiting) = (icon_png(false), icon_png(true));
        assert_ne!(resting, waiting);
        golden(&asset("resting.png"), &resting);
        golden(&asset("needs-you.png"), &waiting);
        golden(&asset("tab-needs-you.png"), &icon_png_on(true, true));
    }
}
