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
enum Eyes {
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
    /// Squeezed shut, `>` `<`: the effort of a hard keystroke. The right eye
    /// is the left one mirrored.
    Squeeze,
}

impl Eyes {
    fn mirrored(self) -> bool {
        self == Eyes::Squeeze
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
        (Size::Small, Eyes::Squeeze) => &["o.", ".o"],
        (Size::Medium, Eyes::Open) => &["*oo", "ooo", "oo*"],
        (Size::Medium, Eyes::Blink) => &["...", "ooo", "..."],
        (Size::Medium, Eyes::Closed) => &["...", "o.o", ".o."],
        (Size::Medium, Eyes::Happy) => &[".o.", "o.o", "..."],
        (Size::Medium, Eyes::Wide) => &["*oo", "ooo", "ooo", "oo*"],
        (Size::Medium, Eyes::Lid) => &["...", "*oo", "ooo"],
        (Size::Medium, Eyes::X) => &["o.o", ".o.", "o.o"],
        (Size::Medium, Eyes::UpRight) => &["*oo", "*oo", "***"],
        (Size::Medium, Eyes::UpLeft) => &["oo*", "oo*", "***"],
        (Size::Medium, Eyes::Squeeze) => &["oo.", "..o", "oo."],
        (Size::Large, Eyes::Open) => &["*ooo", "oooo", "oooo", "oo*o"],
        (Size::Large, Eyes::Blink) => &["....", "....", "oooo", "...."],
        (Size::Large, Eyes::Closed) => &["....", "o..o", ".oo.", "...."],
        (Size::Large, Eyes::Happy) => &["....", ".oo.", "o..o", "...."],
        (Size::Large, Eyes::Wide) => &["*ooo", "oooo", "oooo", "oooo", "oo*o"],
        (Size::Large, Eyes::Lid) => &["....", "*ooo", "oooo", "oo*o"],
        (Size::Large, Eyes::X) => &["o..o", ".oo.", ".oo.", "o..o"],
        (Size::Large, Eyes::UpRight) => &["**oo", "**oo", "****", "****"],
        (Size::Large, Eyes::UpLeft) => &["oo**", "oo**", "****", "****"],
        (Size::Large, Eyes::Squeeze) => &["oo..", "..oo", "..oo", "oo.."],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mouth {
    Smile,
    Flat,
    O,
    Grin,
    Wavy,
    /// Set straight across: resolve.
    Firm,
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
        (_, Mouth::Firm) => &["....", "oooo"],
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
    /// The laptop at step `step` of its diff, the hand pressing a key or
    /// raised, and the cursor shown or not.
    Laptop { step: u8, press: bool, cursor: bool },
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

/// At the laptop, eyes on the screen, `step` of the diff showing.
const fn typing(ms: u32, step: u8, press: bool, cursor: bool) -> Frame {
    f(ms).look(1, 0).mouth(Mouth::Flat).scene(Scene::Laptop { step, press, cursor })
}
/// A keystroke: the hand comes down and the head strokes nod at the screen,
/// then the hand comes up.
const fn key(step: u8) -> [Frame; 2] {
    [typing(200, step, true, true).tilt(1), typing(100, step, false, true)]
}
/// Typing at the laptop, the screen a diff (`Laptop::script`): new lines
/// type out a keystroke a character; a line turns red, the mouth sets, and
/// one hard keystroke with the eyes squeezed shut deletes it; a save turns
/// everything plain, with a grin. Every size's script has these 13 steps.
const WORKING: &[Frame] = &[
    key(0)[0],
    key(0)[1],
    key(1)[0],
    key(1)[1],
    key(2)[0],
    key(2)[1],
    key(3)[0],
    key(3)[1],
    key(4)[0],
    key(4)[1],
    key(5)[0],
    key(5)[1],
    key(6)[0],
    key(6)[1],
    typing(700, 7, false, false).mouth(Mouth::Firm),
    typing(300, 8, true, true).eyes(Eyes::Squeeze).mouth(Mouth::Firm).tilt(1),
    typing(200, 8, false, true).eyes(Eyes::Squeeze).mouth(Mouth::Firm),
    key(9)[0],
    key(9)[1],
    key(10)[0],
    key(10)[1],
    key(11)[0],
    key(11)[1],
    typing(800, 12, false, false).eyes(Eyes::Happy).mouth(Mouth::Grin),
    typing(100, 12, false, false).eyes(Eyes::Blink),
];
const TYPING_STEPS: usize = 13;
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
}

impl Anim {
    #[cfg(test)]
    pub(crate) const ALL: [Anim; 14] = [
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
    fn timeline(self) -> (&'static [Frame], &'static [Frame]) {
        match self {
            Anim::Seat => (&[], SEAT),
            Anim::Spawning => (SPAWNING, IDLE),
            Anim::Idle => (&[], IDLE),
            Anim::Thinking => (&[], THINKING),
            Anim::Working => (&[], WORKING),
            Anim::NeedsYou => (&[], NEEDS_YOU),
            Anim::Done => (DONE, CONTENT),
            Anim::Monitoring => (&[], MONITORING),
            Anim::Interrupted => (INTERRUPTED, LOOKING),
            Anim::Sleeping => (&[], SLEEPING),
            Anim::Throttled => (&[], THROTTLED),
            Anim::Failed => (&[], FAILED),
            Anim::Exited => (&[], EXITED),
            Anim::Unknown => (&[], UNKNOWN),
        }
    }
}

/// The pose `ms` into `anim`.
pub(crate) fn frame_at(anim: Anim, ms: u64) -> &'static Frame {
    let (intro, cycle) = anim.timeline();
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
    Del,
}

impl Role {
    fn ink(self, ink: &CreatureInk) -> Color {
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
            Role::Del => ink.del,
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
    w: i16,
    h: i16,
    px: Vec<Option<Role>>,
    lift: bool,
    marks: Vec<Mark>,
}

impl Picture {
    /// The role at screen pixel (x, y), the lift applied.
    fn at(&self, x: i16, y: i16) -> Option<Role> {
        let y = y + i16::from(self.lift);
        (x >= 0 && y >= 0 && x < self.w && y < self.h)
            .then(|| self.px[(y * self.w + x) as usize])
            .flatten()
    }
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

    let stamp = |grid: &mut Vec<Option<Role>>, pat: &[&str], ax: i16, ay: i16, mirror: bool| {
        for (dy, line) in pat.iter().enumerate() {
            let width = line.len();
            for (dx, ch) in line.bytes().enumerate() {
                let role = match ch {
                    b'o' => Role::Eye,
                    b'*' => Role::Glint,
                    _ => continue,
                };
                let dx = if mirror { width - 1 - dx } else { dx };
                if let Some(i) = idx(ax + dx as i16, ay + dy as i16) {
                    grid[i] = Some(role);
                }
            }
        }
    };
    for (n, &(x, y)) in b.eyes.iter().enumerate() {
        let mirror = n == 1 && fr.eyes.mirrored();
        stamp(&mut grid, eye(size, fr.eyes), x + fr.look.0, y + fr.look.1, mirror);
    }
    stamp(&mut grid, mouth(size, fr.mouth), b.mouth.0, b.mouth.1, false);
    if fr.blush {
        for &(x, y) in &b.blush {
            for i in (0..size.blush_w()).filter_map(|dx| idx(x + dx, y)) {
                if grid[i] == Some(Role::Body) {
                    grid[i] = Some(Role::Blush);
                }
            }
        }
    }

    // Rim light: top and left edges catch it, the right edge falls away, and
    // the body's underside sits in the deepest shade.
    let solid = |x: i16, y: i16| idx(x, y).is_some_and(|i| grid[i].is_some());
    let mut px = grid.clone();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if grid[i] != Some(Role::Body) {
                continue;
            }
            px[i] = Some(if !solid(x, y + 1) {
                if y >= b.top {
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
    // The scene last, over everything: the hand rests in front of the body.
    let mut put = |x: i16, y: i16, role: Role| {
        if let Some(i) = idx(x, y) {
            px[i] = Some(role);
        }
    };
    match fr.scene {
        Some(Scene::Laptop { step, press, cursor }) => {
            if let Some(l) = laptop(size) {
                l.draw(step as usize, press, cursor, &mut put);
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

/// A line of code on the laptop's screen: its indent, its length, and whether
/// it is old, just added or about to go.
type CodeLine = Option<(i16, i16, Diff)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Diff {
    Code,
    Add,
    Del,
}

/// The laptop beside a body, open and facing the reader: its screen's bezel,
/// the deck in front of it, the hand's two poses, the key a press lights,
/// the columns it adds to the stage, and the diff its screen plays, one
/// entry per step of `WORKING` (lines on every other row of the screen).
struct Laptop {
    screen: (i16, i16, i16, i16),
    deck: &'static [(i16, i16, i16, Role)],
    up: &'static [(i16, i16, Role)],
    down: &'static [(i16, i16, Role)],
    key: (i16, i16),
    pad: i16,
    script: [[CodeLine; 3]; TYPING_STEPS],
}

const fn c(indent: i16, len: i16) -> CodeLine {
    Some((indent, len, Diff::Code))
}
const fn a(indent: i16, len: i16) -> CodeLine {
    Some((indent, len, Diff::Add))
}
const fn d(indent: i16, len: i16) -> CodeLine {
    Some((indent, len, Diff::Del))
}

const SMALL_LAPTOP: Laptop = Laptop {
    screen: (13, 2, 18, 7),
    deck: &[(12, 19, 8, Role::Bezel), (11, 20, 9, Role::Deep)],
    up: &[(12, 6, Role::Hi), (13, 6, Role::Hi), (12, 7, Role::Body), (13, 7, Role::Shade)],
    down: &[(12, 7, Role::Hi), (13, 7, Role::Hi), (12, 8, Role::Body), (13, 8, Role::Shade)],
    key: (14, 8),
    pad: 9,
    script: [
        [c(0, 2), a(0, 1), None],
        [c(0, 2), a(0, 2), None],
        [c(0, 2), a(0, 3), None],
        [a(0, 3), a(0, 1), None],
        [a(0, 3), a(0, 2), None],
        [a(0, 3), a(0, 3), None],
        [a(0, 3), a(1, 1), None],
        [d(0, 3), a(1, 1), None],
        [a(1, 1), None, None],
        [a(1, 1), a(0, 1), None],
        [a(1, 1), a(0, 2), None],
        [a(1, 1), a(0, 3), None],
        [c(1, 1), c(0, 3), None],
    ],
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
    script: [
        [c(0, 4), c(1, 3), a(1, 1)],
        [c(0, 4), c(1, 3), a(1, 2)],
        [c(0, 4), c(1, 3), a(1, 3)],
        [c(0, 4), c(1, 3), a(1, 4)],
        [c(1, 3), a(1, 4), a(1, 1)],
        [c(1, 3), a(1, 4), a(1, 2)],
        [c(1, 3), a(1, 4), a(1, 3)],
        [d(1, 3), a(1, 4), a(1, 3)],
        [a(1, 4), a(1, 3), None],
        [a(1, 4), a(1, 3), a(0, 1)],
        [a(1, 4), a(1, 3), a(0, 2)],
        [a(1, 4), a(1, 3), a(0, 3)],
        [c(1, 4), c(1, 3), c(0, 3)],
    ],
};

fn laptop(size: Size) -> Option<&'static Laptop> {
    match size {
        Size::Small => Some(&SMALL_LAPTOP),
        Size::Medium => Some(&MEDIUM_LAPTOP),
        Size::Large => None,
    }
}

impl Laptop {
    fn draw(&self, step: usize, press: bool, cursor: bool, put: &mut impl FnMut(i16, i16, Role)) {
        let (x0, y0, x1, y1) = self.screen;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let edge = y == y0 || y == y1 || x == x0 || x == x1;
                put(x, y, if edge { Role::Bezel } else { Role::Screen });
            }
        }
        let mut end = None;
        for (i, line) in self.script[step.min(TYPING_STEPS - 1)].iter().enumerate() {
            let Some((indent, len, diff)) = *line else { continue };
            let y = y0 + 1 + 2 * i as i16;
            let role = match diff {
                Diff::Code => Role::Code,
                Diff::Add => Role::Add,
                Diff::Del => Role::Del,
            };
            for x in 0..len {
                put(x0 + 1 + indent + x, y, role);
            }
            end = Some((x0 + 1 + indent + len, y));
        }
        if let Some((x, y)) = end.filter(|_| cursor) {
            put(x, y, Role::Glint);
        }
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
    let (intro, cycle) = anim.timeline();
    intro
        .iter()
        .chain(cycle)
        .filter_map(|fr| match fr.scene? {
            Scene::Laptop { .. } => laptop(size).map(|l| l.pad),
            Scene::Cloud(_) => cloud(size).map(|c| c.pad),
        })
        .max()
        .unwrap_or(0)
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
    cells: Vec<Option<(char, Style)>>,
}

/// The stage for `anim`, `ms` into it, in `theme`'s inks. `None` where the
/// theme draws no picture (mono). Every frame of one animation has the same
/// stage, so a reply beside it wraps once per state, not once per frame.
pub(crate) fn stage_for(size: Size, anim: Anim, ms: u64, theme: &Theme) -> Option<Stage> {
    let frame = frame_at(anim, ms);
    let ink = theme.creature_ink(frame.faded)?;
    let mut st = stage(&compose(size, frame, anim_pad(size, anim)), &ink);
    st.body = Body::get(size).w as u16;
    Some(st)
}

fn stage(pic: &Picture, ink: &CreatureInk) -> Stage {
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
            cells[(r * cols + c) as usize] = match (top, bottom) {
                (Some(t), Some(b)) if t == b => Some(('█', Style::default().fg(t))),
                (Some(t), Some(b)) => Some(('▀', Style::default().fg(t).bg(b))),
                (Some(t), None) => Some(('▀', Style::default().fg(t))),
                (None, Some(b)) => Some(('▄', Style::default().fg(b))),
                (None, None) => None,
            };
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
    Stage { cols, rows, body: pic.w as u16, cells }
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

    fn frames(anim: Anim) -> Vec<&'static Frame> {
        let (intro, cycle) = anim.timeline();
        intro.iter().chain(cycle).collect()
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
                for fr in frames(anim) {
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
                    let st = stage(&pic, &ink);
                    let want = (b.w as u16 + pad as u16 + 4, b.h as u16 / 2 + ABOVE);
                    assert_eq!((st.cols, st.rows), want, "{size:?} {anim:?}");
                }
            }
        }
    }

    /// The redraw clock steps every 100 ms, so a frame of any other length
    /// would lie about its length; and every cycle has something to repeat.
    #[test]
    fn every_timeline_steps_on_the_redraw_clock() {
        for anim in Anim::ALL {
            let (intro, cycle) = anim.timeline();
            assert!(!cycle.is_empty(), "{anim:?}");
            for fr in intro.iter().chain(cycle) {
                assert!(fr.ms > 0 && fr.ms % 100 == 0, "{anim:?}: {} ms", fr.ms);
            }
            assert!(std::ptr::eq(frame_at(anim, 0), intro.first().unwrap_or(&cycle[0])));
        }
        // An intro plays once and the cycle takes over for good.
        let intro: u64 = DONE.iter().map(|f| u64::from(f.ms)).sum();
        assert_eq!(frame_at(Anim::Done, intro).eyes, Eyes::Open);
        assert_eq!(frame_at(Anim::Done, 0).eyes, Eyes::Happy);
    }

    /// The one saturated colour keeps its one job on the creature too: only
    /// the needs-you pose wears `attn`, and it always does.
    #[test]
    fn only_needs_you_wears_attn() {
        for size in [Size::Small, Size::Medium] {
            for anim in Anim::ALL {
                let lit = frames(anim).into_iter().any(|fr| {
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
            stage(&compose(Size::Medium, fr, 0), &ink).cells
        };
        let firsts: Vec<_> = Anim::ALL.iter().map(|&a| glyphs(frame_at(a, 0))).collect();
        for (i, a) in Anim::ALL.iter().enumerate() {
            for (j, b) in Anim::ALL.iter().enumerate().skip(i + 1) {
                if [*a, *b] != [Anim::Seat, Anim::Idle] {
                    assert_ne!(firsts[i], firsts[j], "{a:?} and {b:?} open on the same face");
                }
            }
        }
        for anim in Anim::ALL.into_iter().filter(|a| *a != Anim::Exited) {
            let moves = frames(anim).windows(2).any(|w| glyphs(w[0]) != glyphs(w[1]));
            assert!(moves, "{anim:?} never moves");
        }
        assert_eq!(frames(Anim::Exited).len(), 1, "the exited body is still");
    }

    /// Working is typing at a laptop whose screen is a diff: every size's
    /// script has a step for every step of the timeline, the hand is down
    /// on a keystroke and up between them, lines are added and one is
    /// deleted, and the save turns the screen back to plain code.
    #[test]
    fn working_types_a_diff_at_the_laptop() {
        let roles = |size: Size, fr: &Frame| compose(size, fr, anim_pad(size, Anim::Working)).px;
        for size in [Size::Small, Size::Medium] {
            let l = laptop(size).expect("an animated size has a laptop");
            let (x0, y0, x1, y1) = l.screen;
            let lines = ((y1 - y0 - 1) as usize).div_ceil(2);
            assert!(
                l.script.iter().all(|step| step[lines..].iter().all(Option::is_none)),
                "{size:?}"
            );
            for step in &l.script {
                for (indent, len, _) in step.iter().flatten() {
                    // A line and the cursor after it stay inside the screen.
                    assert!(x0 + 1 + indent + len < x1, "{size:?}: a line runs off the screen");
                }
            }
            let has = |fr: &Frame, role: Role| roles(size, fr).contains(&Some(role));
            assert!(WORKING.iter().any(|fr| has(fr, Role::Add)), "{size:?}: lines are added");
            assert!(WORKING.iter().any(|fr| has(fr, Role::Del)), "{size:?}: a line goes");
            let saved = WORKING.iter().rev().find(|fr| fr.eyes == Eyes::Happy).expect("a save");
            assert!(has(saved, Role::Code) && !has(saved, Role::Add) && !has(saved, Role::Del));
            let press = WORKING
                .iter()
                .find(|fr| matches!(fr.scene, Some(Scene::Laptop { press: true, .. })));
            let raised = WORKING
                .iter()
                .find(|fr| matches!(fr.scene, Some(Scene::Laptop { press: false, .. })));
            let (press, raised) = (press.expect("a press"), raised.expect("a raise"));
            let &(hx, hy, _) = l.down.last().expect("a hand");
            let at = |fr: &Frame| roles(size, fr)[(hy * (Body::get(size).w + l.pad) + hx) as usize];
            assert_ne!(at(press), at(raised), "{size:?}: the hand moves");
        }
        // The step each frame names is the script's, and every step is shown.
        let steps: std::collections::BTreeSet<u8> = WORKING
            .iter()
            .filter_map(|fr| match fr.scene {
                Some(Scene::Laptop { step, .. }) => Some(step),
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), TYPING_STEPS);
        assert!(WORKING.iter().any(|fr| fr.eyes == Eyes::Squeeze), "the delete is an effort");
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
        let pic = compose(Size::Large, frame_at(Anim::Seat, 0), 0);
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
    /// tab gives an icon the "!" is a fraction of a pixel, so that icon is
    /// the pose as a dark silhouette on the attention-colour tile — the
    /// board's one saturated colour, readable from across the tab strip.
    fn icon_png_on(needs_you: bool, tab: bool) -> Vec<u8> {
        const SIZE: u32 = 256;
        const SCALE: i32 = 10;
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let ink = theme.creature_ink(false).expect("truecolor draws");
        let anim = if needs_you { Anim::NeedsYou } else { Anim::Seat };
        let fr = frame_at(anim, 0);
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
        let mut ground = rgb(theme.bg.expect("graphite paints its ground"));
        let mut silhouette = None;
        if tab {
            ground = rgb(theme.attn);
            silhouette = Some(rgb(theme.attn_ink));
        }
        let (ox, oy) =
            ((SIZE as i32 - pic.w as i32 * SCALE) / 2, (SIZE as i32 - pic.h as i32 * SCALE) / 2);
        // The tile's rounded corners, antialiased; everything inside it is
        // square pixels on whole image pixels, so it needs none.
        let radius = 54.0f32;
        let coverage = |x: u32, y: u32| {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (px, py) =
                        (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                    let cx = px.clamp(radius, SIZE as f32 - radius);
                    let cy = py.clamp(radius, SIZE as f32 - radius);
                    if (px - cx).powi(2) + (py - cy).powi(2) <= radius * radius {
                        hits += 1;
                    }
                }
            }
            (hits * 255 / 16) as u8
        };
        let img = image::RgbaImage::from_fn(SIZE, SIZE, |x, y| {
            let (sx, sy) = ((x as i32 - ox).div_euclid(SCALE), (y as i32 - oy).div_euclid(SCALE));
            let role = pic.at(sx as i16, sy as i16);
            let [r, g, b] = role.map_or(ground, |r| silhouette.unwrap_or_else(|| rgb(r.ink(&ink))));
            image::Rgba([r, g, b, coverage(x, y)])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).expect("encode");
        out.into_inner()
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
