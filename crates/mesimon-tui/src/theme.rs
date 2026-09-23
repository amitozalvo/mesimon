//! The built-in themes (06 §2), each a `Palette` TABLE materialized per colour
//! profile by one builder. Graphite (dark, default) and chalk (light) are the
//! paper pair the terminal's light/dark answer picks between; blue, amber and
//! green are chosen by hand (2026-09-02, user request — the author's Neovim
//! `blue` scheme, and two phosphor monitors to keep it company). Only tokens
//! M3.5 actually renders exist here; the six-surface elevation model is
//! deliberately collapsed to two painted surfaces (bg + selected) in every
//! profile — that is 06 §2.6's 256-colour rule applied one tier up, and the
//! full elevation ramp is an M6 refinement. Capability negotiation is
//! per-client (06 §2.9): the daemon never sees any of this.
//!
//! **A theme is data, and `Flavor::palette` is the gate.** Every per-flavor
//! decision — the hexes, the hand-authored 256/16/8 tables, which fade target
//! `faded` blends toward, whether there is a tag ring at all — sits on the
//! table, and the colour-law tests read the table rather than a transcription
//! of it. A sixth flavor does not compile until `palette`, `name`, `blurb`
//! and `from_name` have all classified it.

use ratatui::style::{Color, Modifier, Style};

use crate::glyphs::Tier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    TrueColor,
    Ansi256,
    Ansi16,
    /// `TERM=linux` class: 8 colours, no value ramp; bars fall back to the
    /// mono ASCII ladder (06 §2.7).
    Ansi8,
    /// `NO_COLOR` / `--color=never`: structure carries everything (06 §2.8).
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    Graphite,
    Chalk,
    /// Navy and gold — the Borland / Norton Commander look, from the
    /// author's Neovim `blue` scheme.
    Blue,
    /// Amber on black, white base text.
    Amber,
    /// White ink on green-black, the green spent on the accent.
    Green,
    /// Solarized light: cream paper, blue-grey ink.
    Solarized,
}

/// Which of the terminal's two answers a theme sits on. The OSC 11 query
/// only ever says light or dark; a preference maps each answer to a flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ground {
    Dark,
    Light,
}

impl Ground {
    /// The slot's name — in `prefs.json`, the picker and the Settings row.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Ground::Dark => "dark",
            Ground::Light => "light",
        }
    }
}

/// What the colour law asks of a palette. Four kinds because four shapes of
/// screen exist: ink on paper (greys plus three registers), neutral ink on a
/// COLOURED paper (the ground is a hue, the ink is not), a phosphor glow
/// (the ground and the accent share one hue, the ink is white, and `err` is
/// a red off that hue), and a phosphor LADDER (the glow plus the three dim
/// steps and the bars on the hue too, under the beam — only the base step
/// is white). The fourth was argued 2026-09-03: amber's author wanted the
/// original monitor back with white titles, and green's author wanted the
/// glow kept, and one clause cannot say "the dims are grey" and "the dims are
/// amber" at once. The fifth is TINTED paper (Solarized light): the paper
/// is warm and the ink is cool, both carrying a chroma Paper forbids
/// (C* 10 on the cream, 9 on the blue-grey) and nowhere near a chromatic
/// ground's 40 — so the clause holds both under 16 and ≥ 90° apart, which
/// is what keeps the ink from reading as a tint of the paper. The law tests
/// match on this exhaustively, so a sixth shape needs a sixth clause, argued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Paper,
    ChromaticGround,
    Phosphor,
    Ladder,
    TintedPaper,
}

impl Flavor {
    /// Every flavor, in picker order. The paper pair first because they are
    /// the defaults; the rest in the order they were built.
    pub const ALL: [Flavor; 6] = [
        Flavor::Graphite,
        Flavor::Chalk,
        Flavor::Blue,
        Flavor::Amber,
        Flavor::Green,
        Flavor::Solarized,
    ];

    /// The stable id: what `prefs.json` stores and `MESIMON_THEME` accepts.
    pub fn name(self) -> &'static str {
        match self {
            Flavor::Graphite => "graphite",
            Flavor::Chalk => "chalk",
            Flavor::Blue => "blue",
            Flavor::Amber => "amber",
            Flavor::Green => "green",
            Flavor::Solarized => "solarized",
        }
    }

    /// The inverse of `name`, plus the two aliases `MESIMON_THEME` has
    /// accepted since M3.5: `dark` is graphite and `light` is chalk.
    pub fn from_name(s: &str) -> Option<Flavor> {
        match s {
            "dark" => return Some(Flavor::Graphite),
            "light" => return Some(Flavor::Chalk),
            _ => {}
        }
        Flavor::ALL.into_iter().find(|f| f.name() == s)
    }

    /// One line for the picker row.
    pub fn blurb(self) -> &'static str {
        match self {
            Flavor::Graphite => "dark, the default",
            Flavor::Chalk => "light, paper",
            Flavor::Blue => "navy and gold, the Borland look",
            Flavor::Amber => "amber on black",
            Flavor::Green => "green-black, white ink, a green glow",
            Flavor::Solarized => "solarized light, cream and blue-grey",
        }
    }

    pub fn ground(self) -> Ground {
        self.palette().ground
    }

    /// Read by the law tests, which match on it exhaustively.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kind(self) -> Kind {
        self.palette().kind
    }

    /// THE exhaustive gate: six arms, no `_`.
    pub(crate) fn palette(self) -> &'static Palette {
        match self {
            Flavor::Graphite => &GRAPHITE,
            Flavor::Chalk => &CHALK,
            Flavor::Blue => &BLUE,
            Flavor::Amber => &AMBER,
            Flavor::Green => &GREEN,
            Flavor::Solarized => &SOLARIZED,
        }
    }
}

/// The 4-step value ramp of one component state (06 §2.1, D19).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ramp {
    pub base: Color,
    pub dim1: Color,
    pub dim2: Color,
    pub dim3: Color,
}

impl Ramp {
    const fn of(c: [Color; 4]) -> Self {
        Ramp { base: c[0], dim1: c[1], dim2: c[2], dim3: c[3] }
    }
}

/// Accent-bar weight (06 §2.4a). The bar is a background-painted space in
/// colour profiles and an ASCII ladder character in mono/8-colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarWeight {
    /// Board behind an overlay / MOVE origin slot — no producer until the M6
    /// decay/overlay pass, but the ladder (and its tests) keep the weight.
    #[allow(dead_code)]
    Ghost,
    Dormant,
    /// Full-value state hue: the register decides which.
    Live(crate::glyphs::Register),
    Cursor,
}

/// Selection levels for the neutral bar. Tagged bars always keep their color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagLevel {
    Selected,
    Rest,
}

// -- the tables ---------------------------------------------------------------

/// One theme, every profile. The truecolor half is the design; the indexed
/// halves are hand-authored, never nearest-matched (06 §2.6).
pub(crate) struct Palette {
    pub ground: Ground,
    /// Which clause of the law this palette is held to (`test_chroma_law`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub kind: Kind,
    pub truecolor: TrueColor,
    pub ansi256: Ansi256,
    pub ansi16: Ansi16,
    /// Shared: the dark themes all degrade to the same eight colours, because
    /// eight colours cannot hold a navy ground or a phosphor — and saying so
    /// is the 06 §2.7 rule.
    pub ansi8: &'static Ansi8,
}

pub(crate) struct TrueColor {
    pub bg: u32,
    pub selected: u32,
    pub rest: [u32; 4],
    pub sel: [u32; 4],
    pub attn: u32,
    pub err: u32,
    pub calm: u32,
    pub attn_ink: u32,
    pub ghost: u32,
    pub dormant: u32,
    pub cursor: u32,
    /// The diff pane's (add, del) line grounds; `None` where there is no
    /// second hue to blend toward.
    pub diff: Option<(u32, u32)>,
    /// The tag ring. Every shipped palette has one — the phosphors were built
    /// without and the author asked for it back the same day ("it's simply
    /// amber / green"); the `Option` stays because a palette with no second
    /// hue at all is a legitimate thing to declare.
    pub tints: Option<Tints>,
    /// Neutral target for fading an untagged bar. Colored grounds keep a
    /// neutral at similar lightness so the neutral bar does not pick up hue.
    pub shadow: u32,
}

pub(crate) struct Tints {
    pub ring: [u32; PIPS],
    /// Blend factor for an untagged bar off the cursor.
    pub fade: f32,
}

pub(crate) struct Ansi256 {
    pub bg: Option<u8>,
    pub selected: Option<u8>,
    pub ramp: [u8; 4],
    pub attn: u8,
    pub err: u8,
    pub calm: u8,
    pub attn_ink: u8,
    pub ghost: u8,
    pub dormant: u8,
    pub cursor: u8,
}

pub(crate) struct Ansi16 {
    pub bg: Option<u8>,
    pub selected: Option<u8>,
    pub ramp: [Color; 4],
    pub attn: u8,
    pub err: u8,
    pub calm: u8,
    pub attn_ink: u8,
}

pub(crate) struct Ansi8 {
    pub ramp: [Color; 4],
    pub attn: u8,
    pub err: u8,
    pub calm: u8,
    pub attn_ink: u8,
}

const I7: Color = Color::Indexed(7);
const I8: Color = Color::Indexed(8);
const I0: Color = Color::Indexed(0);

/// Graphite's eight-colour form, shared by every dark theme.
static DARK_ANSI8: Ansi8 =
    Ansi8 { ramp: [Color::Reset, I7, I8, I8], attn: 3, err: 1, calm: 6, attn_ink: 0 };
static LIGHT_ANSI8: Ansi8 =
    Ansi8 { ramp: [Color::Reset, I0, I8, I8], attn: 3, err: 1, calm: 6, attn_ink: 15 };

/// 06 §2.2 (truecolor), §2.6 (256), §2.7 (16/8), §2.8 (mono).
static GRAPHITE: Palette = Palette {
    ground: Ground::Dark,
    kind: Kind::Paper,
    truecolor: TrueColor {
        bg: 0x131417,
        selected: 0x272B31,
        rest: [0xE9E7E1, 0xB6B2A9, 0x8C8880, 0x5E5B55],
        sel: [0xF1EFE9, 0xC3BFB6, 0x9A968D, 0x6B675F],
        attn: 0xF0A93A,
        err: 0xD5809A,
        calm: 0x6FBFB0,
        attn_ink: 0x131417,
        ghost: 0x5E5B55,
        dormant: 0x8C8880,
        cursor: 0xF1EFE9,
        diff: Some((0x1E2C28, 0x2E2127)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xF47A7A, 0xBAA601, 0x7EB850, 0x20C188, 0x00BCB9, 0x00B8D8, 0x46ACFC, 0x9697FF,
                0xD182DA, 0xEB79AA,
            ],
            fade: 0.70,
        }),
        shadow: 0x131417,
    },
    ansi256: Ansi256 {
        bg: Some(234),
        selected: Some(236),
        // 246 fails AA on the selected surface — 06 §2.6 says use 248 on
        // 236; one ramp means 248 everywhere.
        ramp: [253, 250, 248, 241],
        attn: 214,
        err: 175,
        calm: 73,
        attn_ink: 234,
        ghost: 241,
        dormant: 246,
        cursor: 253,
    },
    ansi16: Ansi16 {
        bg: None,
        selected: Some(8),
        ramp: [Color::Reset, I7, I8, I8],
        attn: 11,
        err: 1,
        calm: 6,
        attn_ink: 0,
    },
    ansi8: &DARK_ANSI8,
};

/// 06 §2.3 (truecolor), §2.6 light (256), §2.7 (16/8), §2.8 (mono).
static CHALK: Palette = Palette {
    ground: Ground::Light,
    kind: Kind::Paper,
    truecolor: TrueColor {
        bg: 0xFAF8F4,
        selected: 0xE7E3DA,
        rest: [0x22252B, 0x4C4F57, 0x5F6169, 0x8B8D94],
        sel: [0x1B1E23, 0x43464E, 0x585A62, 0x82848B],
        attn: 0x8F5600,
        err: 0x732E41,
        calm: 0x0B5F55,
        attn_ink: 0xFFFFFF,
        ghost: 0x8B8D94,
        dormant: 0x5F6169,
        cursor: 0x1B1E23,
        diff: Some((0xDFEBE4, 0xF2E0E4)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xA12E36, 0x6A5E00, 0x3C6D00, 0x006F4C, 0x006C6A, 0x00697C, 0x00629E, 0x534DAE,
                0x84398D, 0x9A2F63,
            ],
            fade: 0.76,
        }),
        shadow: 0xFAF8F4,
    },
    ansi256: Ansi256 {
        bg: None,
        // 06 §2.6: light-256 never paints `selected` (fails AA); the cursor
        // card is signalled structurally only.
        selected: None,
        ramp: [235, 238, 241, 243],
        attn: 94,
        err: 125,
        calm: 23,
        attn_ink: 255,
        ghost: 243,
        dormant: 241,
        cursor: 235,
    },
    ansi16: Ansi16 {
        bg: None,
        selected: Some(7),
        ramp: [Color::Reset, I0, I8, I8],
        attn: 3,
        err: 1,
        calm: 6,
        attn_ink: 15,
    },
    ansi8: &LIGHT_ANSI8,
};

/// Neovim's `blue.vim` as a mesimon theme: the navy is verbatim, the gold is
/// spent on needs-you alone (navy ink on a gold title row — the Borland menu
/// bar), and the body is cream. Two things did NOT survive the numbers: the
/// scheme's `#005faf` cursor line is L* 40 and a mid-ramp grey measures 2.7:1
/// on it, so the cursor card is one step up on the navy's own hue; and a tag
/// tint faded INTO the navy takes on its hue, so the fade target is a neutral
/// at the ground's lightness (`shadow`).
static BLUE: Palette = Palette {
    ground: Ground::Dark,
    kind: Kind::ChromaticGround,
    truecolor: TrueColor {
        bg: 0x000087,
        selected: 0x2C3590,
        rest: [0xF6F0E7, 0xD1CBC3, 0xB0AAA2, 0x7B766F],
        sel: [0xFCF6ED, 0xD6D1C8, 0xB5B0A7, 0x868179],
        attn: 0xFFD700,
        err: 0xFF7F50,
        calm: 0x59D3D3,
        attn_ink: 0x000087,
        ghost: 0x7B766F,
        dormant: 0xB0AAA2,
        cursor: 0xFCF6ED,
        diff: Some((0x102695, 0x2E177D)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xFF8887, 0xBFAC15, 0x80BB52, 0x21C289, 0x00BFBB, 0x00BBDC, 0x52B2FF, 0xA0A2FF,
                0xDE8EE6, 0xF986B7,
            ],
            fade: 0.70,
        }),
        shadow: 0x242424,
    },
    ansi256: Ansi256 {
        bg: Some(18),
        // 19, not 25: the same hue one step lighter. 25 (`#005faf`) puts
        // dim2 at 3.0 on it.
        selected: Some(19),
        ramp: [255, 252, 249, 245],
        attn: 220,
        err: 209,
        calm: 80,
        attn_ink: 18,
        ghost: 245,
        dormant: 249,
        cursor: 255,
    },
    ansi16: Ansi16 {
        bg: Some(4),
        // Not 6 (Norton's black-on-cyan): `code_bg()` is this surface and
        // rest-ramp text on cyan is 2.8:1.
        selected: Some(12),
        ramp: [Color::Indexed(15), I7, I7, I8],
        attn: 11,
        // Bright red and bright cyan: the dark pair is 1.7:1 on navy.
        err: 9,
        calm: 14,
        attn_ink: 4,
    },
    ansi8: &DARK_ANSI8,
};

/// Amber on black with white base text (2026-09-03). The ground, the cursor
/// surface, the three dim steps, the bars and `calm` are amber, a step less
/// bright than the first cut; the base step of each ramp is cream-white;
/// `attn` is the amber at full beam, above every other amber token by ≥ 8
/// L*; `err` is red so the armed-delete flash is red; the diff has tints.
/// It shipped first with every token amber (body text included) and then
/// for an hour as a glow with grey dims; the author asked for this shape.
static AMBER: Palette = Palette {
    ground: Ground::Dark,
    kind: Kind::Ladder,
    truecolor: TrueColor {
        // The original ground and one step up on it (L* 6 / 14.6).
        bg: 0x1B1201,
        selected: 0x322205,
        // Base cream-white (C* 7.6); the dims are the original ladder's
        // rungs a step less bright — L* 61 / 52 / 34, contrast 6.1 / 4.5 /
        // 2.25 on the ground.
        rest: [0xF3ECDE, 0xC08B1E, 0xA0751C, 0x664A14],
        sel: [0xF9F3E7, 0xC9931F, 0xAA7D1F, 0x70521A],
        // The beam: L* 77.5, C* 82.7, hue 77°.
        attn: 0xFFB000,
        // Red, 57° off the phosphor: L* 62.7, C* 55.5, 6.4 on the ground.
        err: 0xF26D78,
        // The original pale rung: C* 45, dE ≥ 20 from every ramp step.
        calm: 0xDEAE65,
        attn_ink: 0x1B1201,
        ghost: 0x664A14,
        dormant: 0xA0751C,
        // L* 68: a rung under the beam, above dim1.
        cursor: 0xD59B2C,
        // The ground one step toward calm and toward err; the del tint is
        // 49° off the phosphor and dE 22 from the cursor surface, which is
        // what makes the delete flash a RED flash here.
        diff: Some((0x33280A, 0x4E1717)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xF47A7A, 0xBAA601, 0x7EB850, 0x20C188, 0x00BCB9, 0x00B8D8, 0x46ACFC, 0x9697FF,
                0xD182DA, 0xEB79AA,
            ],
            fade: 0.70,
        }),
        shadow: 0x161616,
    },
    ansi256: Ansi256 {
        // The cube has no dark amber, so the ground is the grey ramp's; the
        // base is 230 (`#ffffd7`), the cube's cream, over the original
        // 172 / 136 / 94 rungs.
        bg: Some(232),
        selected: Some(234),
        ramp: [230, 172, 136, 94],
        attn: 214,
        err: 203,
        calm: 179,
        attn_ink: 232,
        ghost: 94,
        dormant: 136,
        cursor: 178,
    },
    ansi16: Ansi16 {
        bg: None,
        // With the dims on 8 nothing survives a surface of 8, and 7 is lighter
        // than 3: the cursor card is structural, the chalk-256 road.
        selected: None,
        ramp: [Color::Reset, Color::Indexed(3), I8, I8],
        attn: 11,
        // Bright red: the dark one is what the flash has to be seen against.
        err: 9,
        calm: 6,
        attn_ink: 0,
    },
    ansi8: &DARK_ANSI8,
};

/// A phosphor GLOW (author 2026-09-03, "make it more beautiful and white
/// text"): the ground and the accent share the hue and the INK is white —
/// both ramps a cool white leaned onto the phosphor's hue (C* ≤ 8.2, like
/// paper's), `attn` the phosphor at full beam, `calm` mint, `err` a red 124°
/// off the hue so the armed-delete flash is red, and the diff tinted. The
/// first green painted every token green (a P1 monitor) and was a screen
/// you squint at; the author kept this shape ("green leave as is, it was
/// good") when amber went back to its ladder.
static GREEN: Palette = Palette {
    ground: Ground::Dark,
    kind: Kind::Phosphor,
    truecolor: TrueColor {
        // L* 7.4, hue 149°.
        bg: 0x081A0C,
        selected: 0x18301E,
        rest: [0xE8F1E6, 0xB9C3B7, 0x8D968B, 0x5F675E],
        sel: [0xF0F8EE, 0xC1CBBF, 0x949D92, 0x6A736A],
        // L* 81.2, C* 81.7, hue 144°.
        attn: 0x45E66B,
        err: 0xF26D78,
        // Mint: C* 28.
        calm: 0xA3D6AE,
        attn_ink: 0x081A0C,
        ghost: 0x5F675E,
        dormant: 0x8D968B,
        // L* 84, C* 35.
        cursor: 0x9FE0AF,
        diff: Some((0x123A20, 0x4A171A)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xF47A7A, 0xBAA601, 0x7EB850, 0x20C188, 0x00BCB9, 0x00B8D8, 0x46ACFC, 0x9697FF,
                0xD182DA, 0xEB79AA,
            ],
            fade: 0.70,
        }),
        shadow: 0x161616,
    },
    ansi256: Ansi256 {
        bg: Some(233),
        selected: Some(236),
        ramp: [255, 250, 248, 241],
        attn: 41,
        err: 203,
        calm: 114,
        attn_ink: 233,
        ghost: 241,
        dormant: 246,
        cursor: 157,
    },
    ansi16: Ansi16 {
        bg: None,
        selected: Some(8),
        ramp: [Color::Reset, I7, I8, I8],
        attn: 10,
        err: 9,
        calm: 2,
        attn_ink: 0,
    },
    ansi8: &DARK_ANSI8,
};

/// Solarized light (Ethan Schoonover), as far as the laws let it be: the
/// ground, the cursor surface and the four grey rungs are the canonical
/// values (base3, base2, base02 / base01 / base00 / base1), and that is the
/// look — warm cream paper, cool blue-grey ink. Two things did not survive
/// the numbers. Solarized's accents all sit at L* 49–60, so none of them
/// clears 4.5 on the cream (yellow is 2.98, red 4.29, cyan 2.93): the three
/// registers keep Solarized's hues and are darkened until they do. And
/// base01 / base00 fall to 4.39 / 3.64 on base2, so the `sel` dims are a
/// step darker than the canonical greys, the way chalk's are. The ring is
/// chalk's (measured on this cream: ≥ 6.4 on bg, ≥ 5.3 on base2) and it
/// fades into the cream itself — measured drift ≤ 20°, so no neutral
/// shadow is needed.
static SOLARIZED: Palette = Palette {
    ground: Ground::Light,
    kind: Kind::TintedPaper,
    truecolor: TrueColor {
        bg: 0xFDF6E3,
        selected: 0xEEE8D5,
        rest: [0x073642, 0x586E75, 0x657B83, 0x93A1A1],
        sel: [0x073642, 0x4E646B, 0x5C717A, 0x8A9898],
        // Solarized yellow (hue 84°) at L* 45: 4.9 on the cream, white ink
        // 5.3 on it.
        attn: 0x8A6600,
        // Solarized red darkened and quieted to C* 40, a register under attn.
        err: 0x9A4247,
        // Solarized cyan at L* 42, C* 25.
        calm: 0x1E6F6A,
        attn_ink: 0xFFFFFF,
        // base1 / base01 / base02: L* 65 / 45 / 20 (base00 sat exactly 15
        // L* under base1, on the bar ladder's line).
        ghost: 0x93A1A1,
        dormant: 0x586E75,
        cursor: 0x073642,
        // The cream a step toward green and toward red.
        diff: Some((0xE9EBCB, 0xF6DDD3)),
        tints: Some(Tints {
            // Shared Graphite hue identities in OKLCH; lightness/chroma adapted
            // to this ground, gamut-mapped at fixed hue. No selection fade.
            ring: [
                0xA12E36, 0x6A5E00, 0x3C6D00, 0x006F4C, 0x006C6A, 0x00697C, 0x00629E, 0x534DAE,
                0x84398D, 0x9A2F63,
            ],
            fade: 0.76,
        }),
        shadow: 0xFDF6E3,
    },
    ansi256: Ansi256 {
        // Solarized's own cube mapping: base3 230, base02 235, base01 240,
        // base00 241, base1 245. Light-256 never paints `selected` (06 §2.6).
        bg: Some(230),
        selected: None,
        ramp: [235, 240, 241, 245],
        // The canonical accents (136 / 160 / 37) fail on 230 the same way
        // the truecolor ones do; these are chalk's darker indices.
        attn: 94,
        err: 124,
        calm: 30,
        attn_ink: 255,
        ghost: 245,
        dormant: 241,
        cursor: 235,
    },
    ansi16: Ansi16 {
        bg: None,
        selected: Some(7),
        ramp: [Color::Reset, I0, I8, I8],
        attn: 3,
        err: 1,
        calm: 6,
        attn_ink: 15,
    },
    ansi8: &LIGHT_ANSI8,
};

pub(crate) struct Theme {
    pub profile: Profile,
    pub flavor: Flavor,
    /// Painted page background; None rides the terminal default.
    pub bg: Option<Color>,
    /// Cursor-card surface; None where the profile cannot paint it
    /// (mono, and light-256 per 06 §2.6 — signalled structurally instead).
    pub selected_bg: Option<Color>,
    pub rest: Ramp,
    pub sel: Ramp,
    pub attn: Color,
    pub err: Color,
    pub calm: Color,
    pub attn_ink: Color,
    bar_ghost: Color,
    bar_dormant: Color,
    bar_cursor: Color,
}

/// How many tag tints exist. A tag's index is `stable_hash(name) % PIPS`, so
/// the same tag is the same colour on every machine and in every screenshot —
/// never its position in a list, or two people see different boards.
///
/// Ten, because that is `MAX_TAGS_PER_GROUP`: one axis can now be entirely
/// colour-distinct, which is the only count that makes the tint mean anything
/// within a group. Six shipped first and ran out in use (author 2026-09-01) —
/// a board with two axes was collapsing four names onto the same tint. The
/// two constants are pinned together by `tag_tints_agree`.
pub const PIPS: usize = 10;

const fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `a` weighted `k` against `b`, channel by channel. Only two RGB colours
/// have a halfway; anything else is `a` from the half up and `b` below it.
fn mix(a: Color, b: Color, k: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return if k >= 0.5 { a } else { b };
    };
    let k = k.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 * k + y as f32 * (1.0 - k)).round() as u8;
    Color::Rgb(ch(ar, br), ch(ag, bg), ch(ab, bb))
}

/// Sine ease-in-out over `0..=1`: starts and lands gently.
fn ease(t: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::PI * t.clamp(0.0, 1.0)).cos()
}

/// How long the crowning runs on a newly crowned card and page (T-411).
pub(crate) const CROWN_FLASH_MS: u64 = 2_000;
/// The crowning's wavefront crosses the title in this long (T-442), whatever
/// its length — a card's twenty cells and a page's hundred finish together.
const CROWN_SWEEP_MS: u64 = 1_300;
/// The filled title holds the crown's tint until here, then eases back to
/// its resting look by `CROWN_FLASH_MS`.
const CROWN_HOLD_MS: u64 = 1_700;
/// Cells of glow the wavefront trails, at most (half the title, at least
/// two, on a short one).
const CROWN_GLOW: f32 = 6.0;
/// Glow strength from which a lit cell's letter is written in ground ink;
/// below it the letter is bright ink cooling to the crown's tint.
const CROWN_INK_K: f32 = 0.6;

impl Theme {
    /// Materialize one flavor at one profile from its table. Mono is derived
    /// rather than tabled: every ramp step is `Reset`, every register is
    /// `Reset`, and only the ink survives (for the profiles that paint it).
    pub fn new(flavor: Flavor, profile: Profile) -> Self {
        let p = flavor.palette();
        let idx = Color::Indexed;
        let (bg, selected_bg, rest, sel, attn, err, calm, attn_ink, ghost, dormant, cursor) =
            match profile {
                Profile::TrueColor => {
                    let t = &p.truecolor;
                    (
                        Some(hex(t.bg)),
                        Some(hex(t.selected)),
                        Ramp::of(t.rest.map(hex)),
                        Ramp::of(t.sel.map(hex)),
                        hex(t.attn),
                        hex(t.err),
                        hex(t.calm),
                        hex(t.attn_ink),
                        hex(t.ghost),
                        hex(t.dormant),
                        hex(t.cursor),
                    )
                }
                Profile::Ansi256 => {
                    let a = &p.ansi256;
                    let r = Ramp::of(a.ramp.map(idx));
                    (
                        a.bg.map(idx),
                        a.selected.map(idx),
                        r,
                        r,
                        idx(a.attn),
                        idx(a.err),
                        idx(a.calm),
                        idx(a.attn_ink),
                        idx(a.ghost),
                        idx(a.dormant),
                        idx(a.cursor),
                    )
                }
                Profile::Ansi16 => {
                    let a = &p.ansi16;
                    let r = Ramp::of(a.ramp);
                    (
                        a.bg.map(idx),
                        a.selected.map(idx),
                        r,
                        r,
                        idx(a.attn),
                        idx(a.err),
                        idx(a.calm),
                        idx(a.attn_ink),
                        Color::Reset,
                        Color::Reset,
                        Color::Reset,
                    )
                }
                Profile::Ansi8 => {
                    let a = p.ansi8;
                    let r = Ramp::of(a.ramp);
                    (
                        None,
                        None,
                        r,
                        r,
                        idx(a.attn),
                        idx(a.err),
                        idx(a.calm),
                        idx(a.attn_ink),
                        Color::Reset,
                        Color::Reset,
                        Color::Reset,
                    )
                }
                Profile::Mono => (
                    None,
                    None,
                    MONO_RAMP,
                    MONO_RAMP,
                    Color::Reset,
                    Color::Reset,
                    Color::Reset,
                    idx(p.ansi16.attn_ink),
                    Color::Reset,
                    Color::Reset,
                    Color::Reset,
                ),
            };
        Theme {
            profile,
            flavor,
            bg,
            selected_bg,
            rest,
            sel,
            attn,
            err,
            calm,
            attn_ink,
            bar_ghost: ghost,
            bar_dormant: dormant,
            bar_cursor: cursor,
        }
    }

    /// Diff-pane line tints (M4b dogfood: full-line green/red grounds like
    /// every diff viewer): the page bg one step toward the calm/err
    /// registers — quiet by construction, so the one-saturated-colour law
    /// stands. TrueColor only; the indexed cube has no tint this quiet, so
    /// 256/16/8/mono keep the fg-register + glyph encoding alone. Every
    /// shipped flavor tints (the phosphors since 2026-09-03: a red `err`
    /// gave them a second hue); `None` stays for a palette that declares
    /// no `diff`.
    pub fn diff_add_bg(&self) -> Option<Color> {
        self.diff_tints().map(|(add, _)| add)
    }

    pub fn diff_del_bg(&self) -> Option<Color> {
        self.diff_tints().map(|(_, del)| del)
    }

    fn diff_tints(&self) -> Option<(Color, Color)> {
        if self.profile != Profile::TrueColor {
            return None;
        }
        self.flavor.palette().truecolor.diff.map(|(a, d)| (hex(a), hex(d)))
    }

    /// The surface a transcript's code sits on. There is exactly ONE
    /// elevated surface in this design (the module header's two-painted-
    /// surfaces collapse), and the cursor card is only its first tenant —
    /// a code slab is the second, on a screen that has no cursor card to
    /// confuse it with. `None` where the profile paints no elevation
    /// (chalk-256, mono): there the code span keeps its backticks instead,
    /// the same call `pip()` makes for tag tints.
    pub fn code_bg(&self) -> Option<Color> {
        self.selected_bg
    }

    pub fn glyph_tier(&self) -> Tier {
        if self.profile == Profile::Mono {
            Tier::Ascii
        } else {
            Tier::Unicode
        }
    }

    fn has_colour(&self) -> bool {
        self.profile != Profile::Mono
    }

    /// SGR-7 is legal only in Mono/Ansi8 (06 §5.1).
    fn reverse_allowed(&self) -> bool {
        matches!(self.profile, Profile::Mono | Profile::Ansi8)
    }

    /// The tag ring, where this profile of this flavor has one.
    fn tints(&self) -> Option<&'static Tints> {
        if self.profile != Profile::TrueColor {
            return None;
        }
        self.flavor.palette().truecolor.tints.as_ref()
    }

    // -- text styles ---------------------------------------------------------

    pub fn base(&self) -> Style {
        Style::default().fg(self.rest.base)
    }
    pub fn dim1(&self) -> Style {
        Style::default().fg(self.rest.dim1)
    }
    pub fn dim2(&self) -> Style {
        Style::default().fg(self.rest.dim2)
    }
    pub fn dim3(&self) -> Style {
        Style::default().fg(self.rest.dim3)
    }
    pub fn attn_text(&self) -> Style {
        if self.has_colour() {
            Style::default().fg(self.attn)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        }
    }
    /// The crown's tint (T-411): the last ring colour, low-chroma by the tag
    /// law, so the one saturated colour stays needs-you's. No bold — the
    /// mark is the glyph, and one card in a hundred wearing it is what makes
    /// it read. Below the tinted profiles it is the quiet ramp, and the
    /// glyph carries it alone.
    pub fn crown_text(&self) -> Style {
        Style::default().fg(self.pip(5))
    }
    /// Ten stable hue identities shared by all themes. Only lightness and
    /// chroma adapt to the surfaces; tags keep their color off the cursor.
    /// Below TrueColor the named chips and underline carry the information.
    pub fn pip(&self, n: usize) -> Color {
        match self.tints() {
            Some(t) => hex(t.ring[n % PIPS]),
            None => self.rest.dim2,
        }
    }

    /// Fade only an untagged bar toward its neutral shadow. Tag colors no
    /// longer share the selection ladder: dimming them hid their identity.
    pub(crate) fn faded(&self, base: Color, level: TagLevel) -> Color {
        // A tintless palette still fades its neutral block: the ladder is the
        // card's, not the ring's.
        let k = match level {
            TagLevel::Selected => return base,
            TagLevel::Rest => self.tints().map_or(0.70, |t| t.fade),
        };
        if self.profile != Profile::TrueColor {
            return base; // no ground to blend into, and one grey to blend
        }
        mix(base, hex(self.flavor.palette().truecolor.shadow), k)
    }

    /// Are the ten tag tints actually distinguishable here?
    ///
    /// Only in TrueColor, and only on a flavor with a ring. `pip()` collapses
    /// every tint to one grey otherwise, so anything that leans on colour
    /// alone (the picker's swatches, the ticket page's chips) has to say the
    /// name instead.
    pub fn paints_tags(&self) -> bool {
        self.tints().is_some()
    }

    /// Ink for a name written on a tag-tinted ground: the page ground, which
    /// is the surface every tint was contrast-checked against.
    pub fn tag_ink(&self) -> Color {
        self.bg.unwrap_or(hex(self.flavor.palette().truecolor.bg))
    }

    pub fn err_text(&self) -> Style {
        Style::default().fg(self.err)
    }
    pub fn calm_text(&self) -> Style {
        Style::default().fg(self.calm)
    }

    /// The MOVE ghost's blink (author 2026-08-30, "I press `<`, I expect the
    /// ticket to blink in place"): the grabbed card's title fg square-waves
    /// down the sel ramp — `sel.base` 400 ms, `sel.dim3` 400 ms — until it is
    /// dropped or the grab is cancelled. Grey ramp only, so the
    /// one-saturated-colour law (`test_attn_provenance*`) is untouched, and
    /// this is fg repainting on the redraw clock — real SGR blink stays
    /// banned for everything else (06 §8; the held card is the one sanctioned
    /// exception, superseding D19's blanket ban — STALE-MAP). BOLD rides both
    /// phases: it is the cursor-title treatment, not the blink. Mono's ramp
    /// is all Reset — no luminance to blink — so the ghost holds the steady
    /// cursor treatment there; its reversed surface already marks the grab.
    pub fn move_blink(&self, frame: usize) -> Style {
        const PHASE_FRAMES: usize = 4; // 4 × 100 ms redraw-clock frames
        let dark = self.has_colour() && (frame / PHASE_FRAMES) % 2 == 1;
        let fg = if dark { self.sel.dim3 } else { self.sel.base };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    }

    /// The pending-delete card (author 2026-09-03, "flash with a red tint
    /// until either delete or cancel"): from the first `d` to the second, or
    /// the stray key that cancels, the card square-waves between the diff's
    /// deleted-line treatment — `diff_del_bg` ground, `err` title — and its
    /// ordinary cursor surface, on the MOVE ghost's cadence (400 ms a phase).
    /// Same clause as that blink: fg/bg repainting on the redraw clock, never
    /// SGR 5, for a card awaiting a second gesture. Both colours are ones the
    /// diff already spends, so the one-saturated-colour law is untouched.
    /// `delete_lit` is the phase; `delete_row` is the lit ground, which falls
    /// back to the cursor surface where the profile has no tint (256 and
    /// below) — there the `err` title carries the flash alone. Every flavor
    /// tints in truecolor, the phosphors since they took a white ramp and a
    /// red `err` (2026-09-03): on the first amber the flash was amber.
    pub fn delete_lit(&self, frame: usize) -> bool {
        const PHASE_FRAMES: usize = 4; // 4 × 100 ms redraw-clock frames
        !self.has_colour() || (frame / PHASE_FRAMES) % 2 == 0
    }

    pub fn delete_row(&self) -> Style {
        match self.diff_del_bg() {
            Some(bg) => Style::default().bg(bg),
            None => self.selected_row(),
        }
    }

    /// The crowning (T-442, replacing T-411's square wave, which the cursor
    /// card's own title treatment hid): a lit wavefront flows out of the
    /// crown and across the title one letter at a time, and leaves every
    /// letter in the crown's tint. `cell` is this letter's column in a swept
    /// run of `cells`, `elapsed` the time since the crowning, `resting` the
    /// style the run wears without it and `surface` the row's ground, which
    /// the glow cools into.
    ///
    /// The front eases across in `CROWN_SWEEP_MS`. Its head is a solid
    /// crown-tint cell written in ground ink, the tag chip's pairing; the
    /// cell ahead fades in with the front's fraction, so the motion glides
    /// rather than steps; behind it the glow falls off over `CROWN_GLOW`
    /// cells, its letter bright ink easing to the tint. Filled, the run holds
    /// the tint to `CROWN_HOLD_MS` and eases back to `resting` by
    /// `CROWN_FLASH_MS` — a no-op off the cursor, where the holder's title
    /// rests in the tint anyway.
    ///
    /// Only TrueColor has a tint and a halfway. Below it the head alone walks
    /// the run, the resting ink and its ground swapped — painted colours,
    /// never SGR 7, so only where the profile paints that ground — and
    /// nothing fills, because the ring's grey there would read as dimming.
    /// Mono holds `resting` and the glyph carries it, the move blink's rule.
    /// Fg/bg repainted on the frame clock, never SGR 5, and never `attn`.
    pub fn crown_sweep(
        &self,
        cell: usize,
        cells: usize,
        elapsed: u64,
        resting: Style,
        surface: Option<Color>,
    ) -> Style {
        if !self.has_colour() || cells == 0 || elapsed >= CROWN_FLASH_MS {
            return resting;
        }
        let rich = self.paints_tags();
        let tint = self.pip(5); // `crown_text`'s ink
        if elapsed >= CROWN_HOLD_MS {
            let u = (elapsed - CROWN_HOLD_MS) as f32 / (CROWN_FLASH_MS - CROWN_HOLD_MS) as f32;
            return match resting.fg {
                Some(fg) if rich => resting.fg(mix(fg, tint, ease(u))),
                _ => resting,
            };
        }
        let glow = if rich { CROWN_GLOW.min(cells as f32 / 2.0).max(2.0) } else { 0.0 };
        let t = elapsed as f32 / CROWN_SWEEP_MS as f32;
        // From one cell short of the run to past its end by the glow, so the
        // first frame lights nothing and the last leaves nothing lit.
        let front = -1.0 + (cells as f32 + glow + 1.0) * ease(t);
        let d = front - cell as f32; // how far behind the front this cell is
        if !rich {
            return match (resting.fg, surface) {
                (Some(ink), Some(ground)) if (0.0..1.0).contains(&d) => {
                    resting.fg(ground).bg(ink).add_modifier(Modifier::BOLD)
                }
                _ => resting,
            };
        }
        let k = if d <= -1.0 {
            0.0
        } else if d < 0.0 {
            d + 1.0
        } else if d < 1.0 {
            1.0
        } else {
            (1.0 - (d - 1.0) / glow).max(0.0).powi(2)
        };
        if k <= 0.0 {
            return if d > 0.0 { resting.fg(tint) } else { resting };
        }
        let lit = match surface {
            Some(ground) => resting.bg(mix(tint, ground, k)),
            None if k >= CROWN_INK_K => resting.bg(tint),
            None => resting,
        };
        let lit = if k >= CROWN_INK_K {
            lit.fg(self.tag_ink())
        } else if d < 0.0 {
            lit // the leading edge: its letter keeps the resting ink
        } else {
            lit.fg(mix(self.sel.base, tint, k / CROWN_INK_K))
        };
        if k >= 1.0 {
            lit.add_modifier(Modifier::BOLD)
        } else {
            lit
        }
    }

    /// The inverted needs-you title row (06 §2.4b): `attn` ground, `attn_ink`
    /// text. In mono this is one of the three sanctioned SGR-7 uses.
    pub fn attn_row(&self) -> Style {
        if self.has_colour() {
            Style::default().bg(self.attn).fg(self.attn_ink)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        }
    }

    /// The cursor card's surface + title treatment pieces (06 §7): callers
    /// combine `selected_row` (surface) with `sel` ramp text and BOLD.
    pub fn selected_row(&self) -> Style {
        match self.selected_bg {
            Some(bg) => Style::default().bg(bg),
            None if self.reverse_allowed() => Style::default().add_modifier(Modifier::REVERSED),
            // Light-256 (and a phosphor at 16): no painted surface; bar
            // weight + bold carry it.
            None => Style::default(),
        }
    }

    /// The accent bar cell: painted space in colour profiles, ASCII ladder in
    /// mono/8-colour (06 §2.4a / §2.8: `.` ghost, `:` dormant, `|` live,
    /// `#` cursor).
    pub fn bar(&self, weight: BarWeight) -> (char, Style) {
        use crate::glyphs::Register;
        let ladder = self.profile == Profile::Mono || self.profile == Profile::Ansi8;
        if ladder {
            let ch = match weight {
                BarWeight::Ghost => '.',
                BarWeight::Dormant => ':',
                BarWeight::Live(_) => '|',
                BarWeight::Cursor => '#',
            };
            let style = match weight {
                BarWeight::Live(Register::Attn) => self.attn_text(),
                BarWeight::Live(Register::Err) => self.err_text(),
                BarWeight::Live(Register::Calm) => self.calm_text(),
                _ => Style::default(),
            };
            return (ch, style);
        }
        let colour = match weight {
            BarWeight::Ghost => self.bar_ghost,
            BarWeight::Dormant => self.bar_dormant,
            BarWeight::Cursor => self.bar_cursor,
            BarWeight::Live(Register::Attn) => self.attn,
            BarWeight::Live(Register::Err) => self.err,
            BarWeight::Live(Register::Calm) => self.calm,
            BarWeight::Live(Register::Grey) => self.rest.dim1,
            // A parked mark never asks for a live bar (`card.rs` routes the
            // register to `Dormant` before it gets here); the arm exists so
            // the enum stays exhaustive and says what it would mean.
            BarWeight::Live(Register::Dormant) => self.bar_dormant,
        };
        (' ', Style::default().bg(colour))
    }

    /// The ticket page's description bar: the card's neutral cursor bar at a
    /// quarter of the width (author 2026-09-03, "reduce thickness"). A
    /// painted cell has one width, so thinner means a glyph — `▎` U+258E in
    /// the bar's colour, drawn over whatever surface the row is on. The
    /// ladder tiers draw `|`, their thin stroke.
    pub fn desc_bar(&self) -> (char, Style) {
        if self.profile == Profile::Mono || self.profile == Profile::Ansi8 {
            return ('|', Style::default());
        }
        ('▎', Style::default().fg(self.bar_cursor))
    }
}

const MONO_RAMP: Ramp =
    Ramp { base: Color::Reset, dim1: Color::Reset, dim2: Color::Reset, dim3: Color::Reset };

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::Register;

    /// sRGB → CIE L*a*b*, for the colour-law tests (06 §12). Test-only.
    fn lab(rgb: u32) -> (f64, f64, f64) {
        let srgb = |v: u32| {
            let c = (v & 0xFF) as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (srgb(rgb >> 16), srgb(rgb >> 8), srgb(rgb));
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
        let f = |t: f64| {
            if t > 0.008856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
    }

    /// Lightness and chroma — what most of the laws are stated in.
    fn lch(rgb: u32) -> (f64, f64) {
        let (l, a, b) = lab(rgb);
        (l, (a * a + b * b).sqrt())
    }

    /// Hue angle in degrees, 0..360.
    fn hue(rgb: u32) -> f64 {
        let (_, a, b) = lab(rgb);
        b.atan2(a).to_degrees().rem_euclid(360.0)
    }

    /// The shorter way round the wheel between two hues.
    fn hue_gap(a: u32, b: u32) -> f64 {
        let d = (hue(a) - hue(b)).abs() % 360.0;
        d.min(360.0 - d)
    }

    /// CIE76 distance. Coarse next to CIEDE2000, but the tag ramp is one
    /// lightness and one chroma, so the only thing separating two tints is
    /// the hue angle — exactly where CIE76 is at its most honest.
    fn delta_e(a: u32, b: u32) -> f64 {
        let (l1, a1, b1) = lab(a);
        let (l2, a2, b2) = lab(b);
        ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let lum = |rgb: u32| {
            let srgb = |v: u32| {
                let c = (v & 0xFF) as f64 / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * srgb(rgb >> 16) + 0.7152 * srgb(rgb >> 8) + 0.0722 * srgb(rgb)
        };
        let (la, lb) = (lum(a), lum(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn tc(f: Flavor) -> &'static TrueColor {
        &f.palette().truecolor
    }

    /// A `Color::Rgb` back to the hex the laws are stated in.
    fn rgb(c: Color) -> u32 {
        let Color::Rgb(r, g, b) = c else { panic!("{c:?} is not truecolor") };
        ((r as u32) << 16) | ((g as u32) << 8) | b as u32
    }

    /// The chroma budget shared by every kind that has three registers:
    /// C*(attn) ≥ C*(err)+12 and ≥ C*(calm)+20.
    fn assert_register_budget(f: Flavor) {
        let t = tc(f);
        let (_, ca) = lch(t.attn);
        let (_, ce) = lch(t.err);
        let (_, cc) = lch(t.calm);
        assert!(ca >= ce + 12.0, "{f:?}: C*(attn) {ca:.1} < C*(err) {ce:.1} + 12");
        assert!(ca >= cc + 20.0, "{f:?}: C*(attn) {ca:.1} < C*(calm) {cc:.1} + 20");
    }

    /// L2 (06 §0), stated per kind. Paper: exactly three chromatic tokens;
    /// every grey C* ≤ 8.2 (V1); the register budget. A chromatic ground: the
    /// GROUND is a colour and the ink is not — both ramps stay grey, the two
    /// surfaces share a hue, every register sits ≥ 60° of hue away from the
    /// ground so nothing chromatic can be mistaken for it, and the fade
    /// target is a neutral at the ground's lightness. A phosphor glow: the
    /// ground, its cursor surface, `calm` and the cursor bar all sit within
    /// 15° of `attn`, which IS the phosphor at full beam (C* ≥ 60, and the
    /// most chromatic token); the ink is white — both ramps neutral, base
    /// L* ≥ 90; `err` is ≥ 45° of hue off the phosphor so it reads as its
    /// own colour (the armed-delete flash was invisible on the first amber,
    /// whose `err` was amber); `calm` is the hue gone pale (C* ≤ 35, dE ≥ 20
    /// from every ramp step); the diff tints exist, and the del tint is red
    /// (≥ 45° off the phosphor, a step up from the ground); the fade target
    /// is a neutral at the ground's lightness. A ladder: the glow's clauses
    /// with the dims moved onto the hue — the base step of each ramp is
    /// white, every other amber token (dims, bars, surfaces) sits within 15°
    /// of `attn` AND under it by ≥ 8 L* and in chroma, so the beam is the
    /// top of the ladder; `calm` is held by distance (dE ≥ 20 from every
    /// ramp step) and the register budget, not by a chroma cap. Tinted paper:
    /// the ground and its cursor surface carry a chroma between 5 and 16 on
    /// one hue (the surface a step DOWN, it is a light ground), every ink
    /// rung is under C* 16 and ≥ 90° of hue from the paper, the register
    /// budget holds, the diff tints are a step down from the paper, and the
    /// fade target is the paper itself (measured: the ring drifts ≤ 20°).
    #[test]
    fn test_chroma_law() {
        for f in Flavor::ALL {
            let t = tc(f);
            let greys = t.rest.iter().chain(t.sel.iter());
            match f.kind() {
                Kind::Paper => {
                    for grey in greys.chain([t.bg, t.selected].iter()) {
                        let (_, c) = lch(*grey);
                        assert!(c <= 8.2, "{f:?}: grey {grey:06X} has C* {c:.1} > 8.2");
                    }
                    assert_register_budget(f);
                    assert_eq!(t.shadow, t.bg, "{f:?}: paper fades into its own ground");
                }
                Kind::ChromaticGround => {
                    let (lb, cb) = lch(t.bg);
                    assert!(cb >= 40.0, "{f:?}: the ground is a colour, C* {cb:.1}");
                    let gap = hue_gap(t.bg, t.selected);
                    assert!(gap <= 15.0, "{f:?}: selected is {gap:.1}° off the ground's hue");
                    let (ls, _) = lch(t.selected);
                    assert!(
                        ls - lb >= 8.0,
                        "{f:?}: selected is not a step up ({ls:.1} vs {lb:.1})"
                    );
                    for grey in greys {
                        let (_, c) = lch(*grey);
                        assert!(c <= 8.2, "{f:?}: ink {grey:06X} has C* {c:.1} > 8.2");
                    }
                    for (name, reg) in [("attn", t.attn), ("err", t.err), ("calm", t.calm)] {
                        let gap = hue_gap(reg, t.bg);
                        assert!(gap >= 60.0, "{f:?}: {name} is only {gap:.1}° from the ground");
                    }
                    assert_register_budget(f);
                    let (lsh, csh) = lch(t.shadow);
                    assert!(csh <= 8.2, "{f:?}: the shadow is not neutral, C* {csh:.1}");
                    assert!((lsh - lb).abs() <= 3.0, "{f:?}: shadow L* {lsh:.1} vs ground {lb:.1}");
                }
                Kind::Phosphor => {
                    let (la, ca) = lch(t.attn);
                    assert!(ca >= 60.0, "{f:?}: attn is not the beam, C* {ca:.1}");
                    let phosphor = hue(t.attn);
                    for (name, c) in [
                        ("bg", t.bg),
                        ("selected", t.selected),
                        ("calm", t.calm),
                        ("cursor", t.cursor),
                    ] {
                        let gap = hue_gap(c, t.attn);
                        assert!(
                            gap <= 15.0,
                            "{f:?}: {name} {c:06X} is {gap:.1}° off {phosphor:.0}°"
                        );
                    }
                    let (lb, cb) = lch(t.bg);
                    assert!(cb >= 5.0, "{f:?}: the ground is untinted, C* {cb:.1}");
                    let (ls, _) = lch(t.selected);
                    assert!(
                        ls - lb >= 8.0,
                        "{f:?}: selected is not a step up ({ls:.1} vs {lb:.1})"
                    );
                    for ink in greys {
                        let (_, c) = lch(*ink);
                        assert!(c <= 8.2, "{f:?}: ink {ink:06X} has C* {c:.1} > 8.2");
                    }
                    let (lr, _) = lch(t.rest[0]);
                    assert!(lr >= 90.0, "{f:?}: the ink is not white, L* {lr:.1}");
                    let gap = hue_gap(t.err, t.attn);
                    assert!(gap >= 45.0, "{f:?}: err is only {gap:.1}° off the phosphor");
                    let (_, cc) = lch(t.calm);
                    assert!(cc <= 35.0, "{f:?}: calm is not pale, C* {cc:.1}");
                    for step in t.rest.iter().chain(t.sel.iter()) {
                        let d = delta_e(t.calm, *step);
                        assert!(d >= 20.0, "{f:?}: calm is dE {d:.1} from ramp step {step:06X}");
                    }
                    assert_register_budget(f);
                    // The tokens the accent must out-shout: every other
                    // chromatic thing on the board sits under it in chroma.
                    for (name, c) in [("cursor", t.cursor), ("calm", t.calm), ("err", t.err)] {
                        let (_, cx) = lch(c);
                        assert!(cx < ca, "{f:?}: {name} C* {cx:.1} rivals attn C* {ca:.1}");
                    }
                    let _ = la;
                    assert_eq!(t.attn_ink, t.bg, "{f:?}: the ink on the beam is the ground");
                    let (add, del) = t.diff.expect("a glow has two tints to blend toward");
                    let (ld, _) = lch(del);
                    assert!(ld - lb >= 6.0, "{f:?}: the del tint is not a step up from the ground");
                    let gap = hue_gap(del, t.attn);
                    assert!(gap >= 45.0, "{f:?}: the del tint is only {gap:.1}° off the phosphor");
                    let (ladd, _) = lch(add);
                    assert!(
                        ladd - lb >= 6.0,
                        "{f:?}: the add tint is not a step up from the ground"
                    );
                    // The ground is tinted, so a fade into it would drain a
                    // tint of the opposite hue through grey (47° of drift,
                    // C* 4.7 at the sleeping level): the shadow is neutral.
                    let (lsh, csh) = lch(t.shadow);
                    assert!(csh <= 8.2, "{f:?}: the shadow is not neutral, C* {csh:.1}");
                    assert!((lsh - lb).abs() <= 3.0, "{f:?}: shadow L* {lsh:.1} vs ground {lb:.1}");
                }
                Kind::Ladder => {
                    let (la, ca) = lch(t.attn);
                    assert!(ca >= 60.0, "{f:?}: attn is not the beam, C* {ca:.1}");
                    let phosphor = hue(t.attn);
                    let (lb, cb) = lch(t.bg);
                    assert!(cb >= 5.0, "{f:?}: the ground is untinted, C* {cb:.1}");
                    let (ls, _) = lch(t.selected);
                    assert!(
                        ls - lb >= 8.0,
                        "{f:?}: selected is not a step up ({ls:.1} vs {lb:.1})"
                    );
                    for base in [t.rest[0], t.sel[0]] {
                        let (l, c) = lch(base);
                        assert!(c <= 8.2, "{f:?}: base {base:06X} has C* {c:.1} > 8.2");
                        assert!(l >= 90.0, "{f:?}: base {base:06X} is not white, L* {l:.1}");
                    }
                    // Every rung is on the hue and under the beam.
                    let rungs = [
                        ("bg", t.bg),
                        ("selected", t.selected),
                        ("ghost", t.ghost),
                        ("dormant", t.dormant),
                        ("cursor", t.cursor),
                        ("calm", t.calm),
                        ("dim1", t.rest[1]),
                        ("dim2", t.rest[2]),
                        ("dim3", t.rest[3]),
                        ("sel dim1", t.sel[1]),
                        ("sel dim2", t.sel[2]),
                        ("sel dim3", t.sel[3]),
                    ];
                    for (name, c) in rungs {
                        let gap = hue_gap(c, t.attn);
                        assert!(
                            gap <= 15.0,
                            "{f:?}: {name} {c:06X} is {gap:.1}° off {phosphor:.0}°"
                        );
                        let (l, cx) = lch(c);
                        assert!(cx < ca, "{f:?}: {name} C* {cx:.1} rivals attn C* {ca:.1}");
                        if name != "calm" {
                            assert!(
                                l <= la - 8.0,
                                "{f:?}: {name} L* {l:.1} is not under the beam L* {la:.1}"
                            );
                        }
                    }
                    for step in t.rest.iter().chain(t.sel.iter()) {
                        let d = delta_e(t.calm, *step);
                        assert!(d >= 20.0, "{f:?}: calm is dE {d:.1} from ramp step {step:06X}");
                    }
                    assert_register_budget(f);
                    let gap = hue_gap(t.err, t.attn);
                    assert!(gap >= 45.0, "{f:?}: err is only {gap:.1}° off the phosphor");
                    assert_eq!(t.attn_ink, t.bg, "{f:?}: the ink on the beam is the ground");
                    let (add, del) = t.diff.expect("a ladder has two tints to blend toward");
                    let (ld, _) = lch(del);
                    assert!(ld - lb >= 6.0, "{f:?}: the del tint is not a step up from the ground");
                    let gap = hue_gap(del, t.attn);
                    assert!(gap >= 45.0, "{f:?}: the del tint is only {gap:.1}° off the phosphor");
                    let (ladd, _) = lch(add);
                    assert!(
                        ladd - lb >= 6.0,
                        "{f:?}: the add tint is not a step up from the ground"
                    );
                    let (lsh, csh) = lch(t.shadow);
                    assert!(csh <= 8.2, "{f:?}: the shadow is not neutral, C* {csh:.1}");
                    assert!((lsh - lb).abs() <= 3.0, "{f:?}: shadow L* {lsh:.1} vs ground {lb:.1}");
                }
                Kind::TintedPaper => {
                    let (lb, cb) = lch(t.bg);
                    assert!((5.0..=16.0).contains(&cb), "{f:?}: paper C* {cb:.1} is not a tint");
                    let (ls, cs) = lch(t.selected);
                    assert!((5.0..=16.0).contains(&cs), "{f:?}: surface C* {cs:.1} is not a tint");
                    let gap = hue_gap(t.bg, t.selected);
                    assert!(gap <= 15.0, "{f:?}: selected is {gap:.1}° off the paper's hue");
                    assert!(
                        lb - ls >= 4.0,
                        "{f:?}: selected is not a step down ({ls:.1} vs {lb:.1})"
                    );
                    for ink in greys {
                        let (_, c) = lch(*ink);
                        assert!(c <= 16.0, "{f:?}: ink {ink:06X} has C* {c:.1} > 16");
                        let gap = hue_gap(*ink, t.bg);
                        assert!(
                            gap >= 90.0,
                            "{f:?}: ink {ink:06X} is only {gap:.1}° off the paper"
                        );
                    }
                    assert_register_budget(f);
                    let (add, del) = t.diff.expect("tinted paper has two tints");
                    for (name, tint) in [("add", add), ("del", del)] {
                        let (l, _) = lch(tint);
                        assert!(
                            (2.0..=10.0).contains(&(lb - l)),
                            "{f:?}: the {name} tint is not a step down from the paper ({l:.1})"
                        );
                    }
                    assert_eq!(t.shadow, t.bg, "{f:?}: tinted paper fades into its own ground");
                }
            }
        }
    }

    /// Tag identity survives theme changes, with useful contrast and pair
    /// separation. Attention retains its own token and full title band;
    /// tags no longer share its old chroma ceiling or forbidden hue bands.
    #[test]
    fn tag_colors_are_legible_distinct_and_keep_their_hue_across_themes() {
        // OKLab, independent of the CIE Lab model used for the older palette
        // laws. Compare hue after 8-bit RGB rounding, with one degree tolerance.
        let oklab = |rgb: u32| {
            let linear = |v: u32| {
                let c = (v & 255) as f64 / 255.0;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            let (r, g, b) = (linear(rgb >> 16), linear(rgb >> 8), linear(rgb));
            let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
            let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
            let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
            (
                1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
                0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
            )
        };
        let reference = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let tab = tc(flavor);
            for n in 0..PIPS {
                let color = rgb(theme.pip(n));
                for surface in [tab.bg, tab.selected] {
                    assert!(
                        contrast(color, surface) >= 4.5,
                        "{flavor:?} tint {n} on {surface:06X}"
                    );
                }
                assert_ne!(color, tab.attn, "tags cannot use the attention token");
                let (a, b) = oklab(color);
                assert!(a.hypot(b) >= 0.08, "{flavor:?} tint {n} lost its chroma");
                let (ra, rb) = oklab(rgb(reference.pip(n)));
                let gap =
                    ((b.atan2(a) - rb.atan2(ra)).to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
                assert!(gap.abs() <= 1.0, "{flavor:?} tint {n} drifted {gap:.2} degrees");
                for other in 0..n {
                    let d = delta_e(color, rgb(theme.pip(other)));
                    assert!(d >= 12.0, "{flavor:?} tints {other}/{n} only dE {d:.1} apart");
                }
            }
        }
    }

    /// The tint index is stored in `columns.toml` by `mesimon-core`, which
    /// cannot see the theme, and read back here — so the two moduli have to
    /// be the same number or a saved colour lands on a different hue than the
    /// one that was picked. The doc comment on `TAG_TINTS` promises this test
    /// exists; it now does.
    #[test]
    fn tag_tints_agree() {
        assert_eq!(PIPS, mesimon_core::board::TAG_TINTS as usize);
        assert_eq!(PIPS, mesimon_core::board::MAX_TAGS_PER_GROUP, "an axis cannot be all-distinct");
    }

    /// Below TrueColor the tint is abandoned, not approximated: every pip is
    /// the same grey and the name carries the tag.
    #[test]
    fn test_pips_lose_the_tint_below_truecolor() {
        for flavor in Flavor::ALL {
            for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
                let t = Theme::new(flavor, p);
                assert!(!t.paints_tags(), "{flavor:?}/{p:?} claims to paint tags");
                for n in 0..PIPS {
                    assert_eq!(t.pip(n), t.rest.dim2, "{flavor:?}/{p:?} pip {n} kept a tint");
                }
            }
        }
    }

    /// Every shipped theme paints its tags in truecolor — the phosphors
    /// included, since 2026-09-02's second cut.
    #[test]
    fn every_flavor_paints_tags_in_truecolor() {
        for f in Flavor::ALL {
            assert!(Theme::new(f, Profile::TrueColor).paints_tags(), "{f:?}");
        }
    }

    /// 06 §12: every text role ≥ 4.5:1 on its legal surfaces (dim3 ≥ 3:1 is
    /// explicitly relaxed — it is a de-emphasis role); attn_ink on attn ≥ 4.5.
    #[test]
    fn test_contrast_matrix() {
        for f in Flavor::ALL {
            let t = tc(f);
            for (ramp, surface) in [(t.rest, t.bg), (t.sel, t.selected)] {
                assert!(contrast(ramp[0], surface) >= 4.5, "{f:?}: base on {surface:06X}");
                assert!(contrast(ramp[1], surface) >= 4.5, "{f:?}: dim1 on {surface:06X}");
                assert!(contrast(ramp[2], surface) >= 4.0, "{f:?}: dim2 on {surface:06X}");
                assert!(contrast(ramp[3], surface) >= 2.0, "{f:?}: dim3 on {surface:06X}");
            }
            for chroma in [t.attn, t.err, t.calm] {
                assert!(contrast(chroma, t.bg) >= 4.5, "{f:?}: {chroma:06X} on bg");
            }
            assert!(contrast(t.attn_ink, t.attn) >= 4.5, "{f:?}: attn_ink on attn");
        }
    }

    /// 06 §2.4a: adjacent bar weights ≥ 15 L* apart.
    #[test]
    fn test_bar_ladder() {
        for f in Flavor::ALL {
            let t = tc(f);
            let ls: Vec<f64> = [t.ghost, t.dormant, t.cursor].iter().map(|c| lch(*c).0).collect();
            assert!((ls[0] - ls[1]).abs() >= 15.0, "{f:?}: ghost vs dormant");
            assert!((ls[1] - ls[2]).abs() >= 15.0, "{f:?}: dormant vs cursor");
        }
    }

    /// The one saturated colour is its OWN colour at every profile that has
    /// colour: never a ramp step, never another register, never a surface, a
    /// bar or a tint. On paper this is redundant with the chroma law; on a
    /// phosphor, where the ground, the bar and `calm` share the accent's hue,
    /// it is what keeps `test_attn_provenance*` meaningful — and it is what
    /// forbids the tempting `{3,3,3,3}` eight-colour ramp, whose base IS the
    /// accent.
    #[test]
    fn attn_is_its_own_colour() {
        for f in Flavor::ALL {
            for p in [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8] {
                let t = Theme::new(f, p);
                let mut others = vec![
                    t.rest.base,
                    t.rest.dim1,
                    t.rest.dim2,
                    t.rest.dim3,
                    t.sel.base,
                    t.sel.dim1,
                    t.sel.dim2,
                    t.sel.dim3,
                    t.err,
                    t.calm,
                    t.bar_ghost,
                    t.bar_dormant,
                    t.bar_cursor,
                ];
                others.extend(t.bg);
                others.extend(t.selected_bg);
                others.extend(t.diff_add_bg());
                others.extend(t.diff_del_bg());
                others.extend((0..PIPS).map(|n| t.pip(n)));
                assert!(!others.contains(&t.attn), "{f:?}/{p:?}: attn {:?} is spent twice", t.attn);
            }
        }
    }

    #[test]
    fn every_flavor_round_trips_its_name() {
        for f in Flavor::ALL {
            assert_eq!(Flavor::from_name(f.name()), Some(f));
            assert!(!f.blurb().is_empty());
        }
        assert_eq!(Flavor::from_name("dark"), Some(Flavor::Graphite));
        assert_eq!(Flavor::from_name("light"), Some(Flavor::Chalk));
        assert_eq!(Flavor::from_name("sepia"), None);
        let names: std::collections::HashSet<_> = Flavor::ALL.iter().map(|f| f.name()).collect();
        assert_eq!(names.len(), Flavor::ALL.len(), "two flavors share a name");
    }

    #[test]
    fn mono_bar_is_the_ascii_ladder() {
        let t = Theme::new(Flavor::Graphite, Profile::Mono);
        assert_eq!(t.bar(BarWeight::Ghost).0, '.');
        assert_eq!(t.bar(BarWeight::Dormant).0, ':');
        assert_eq!(t.bar(BarWeight::Live(Register::Grey)).0, '|');
        assert_eq!(t.bar(BarWeight::Cursor).0, '#');
        // Colour profiles paint a space instead.
        let t = Theme::new(Flavor::Graphite, Profile::TrueColor);
        assert_eq!(t.bar(BarWeight::Cursor).0, ' ');
    }

    /// 06 §2.6: light-256 never paints `selected` (fails AA); every dark
    /// ground does.
    #[test]
    fn selected_is_painted_by_ground_at_256() {
        for f in Flavor::ALL {
            let painted = Theme::new(f, Profile::Ansi256).selected_bg.is_some();
            assert_eq!(painted, f.ground() == Ground::Dark, "{f:?}");
        }
    }

    /// At sixteen colours a ladder has nothing to paint its cursor card
    /// with (the dims sit on 8, and 7 is lighter than the phosphor's own
    /// index), so the card is structural — and everything else paints one,
    /// the glow included since its ramp went white.
    #[test]
    fn ladder_16_never_paints_selected() {
        for f in Flavor::ALL {
            let painted = Theme::new(f, Profile::Ansi16).selected_bg.is_some();
            assert_eq!(painted, f.kind() != Kind::Ladder, "{f:?}");
        }
    }

    #[test]
    fn reverse_only_in_mono_and_8() {
        for f in Flavor::ALL {
            for p in [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16] {
                let t = Theme::new(f, p);
                assert!(!t.selected_row().add_modifier.contains(Modifier::REVERSED), "{f:?}/{p:?}");
                assert!(!t.attn_row().add_modifier.contains(Modifier::REVERSED), "{f:?}/{p:?}");
            }
            // Mono's inverted needs-you row is real SGR-7; Ansi8 still has the
            // three chromatic indices so its row is painted, but its
            // cursor-card surface (no painted selected) falls back to the
            // sanctioned reverse.
            let t = Theme::new(f, Profile::Mono);
            assert!(t.attn_row().add_modifier.contains(Modifier::REVERSED));
            let t = Theme::new(f, Profile::Ansi8);
            assert!(t.attn_row().bg.is_some());
            assert!(t.selected_row().add_modifier.contains(Modifier::REVERSED));
        }
    }

    #[test]
    fn move_blink_rides_the_sel_ramp_only() {
        for flavor in Flavor::ALL {
            for p in [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16, Profile::Ansi8] {
                let t = Theme::new(flavor, p);
                let chromatic = [t.attn, t.err, t.calm];
                let ramp = [t.sel.base, t.sel.dim3];
                let mut seen = std::collections::HashSet::new();
                for f in 0..16 {
                    let s = t.move_blink(f);
                    let fg = s.fg.expect("blink always paints a fg");
                    // Mono's "chromatic" tokens are Reset like the ramp —
                    // the collision check only means something with colour.
                    if t.has_colour() {
                        assert!(
                            !chromatic.contains(&fg),
                            "{flavor:?}/{p:?} frame {f}: chromatic blink"
                        );
                    }
                    assert!(ramp.contains(&fg), "{flavor:?}/{p:?} frame {f}: off-ramp blink");
                    // Luminance repainting only — never real SGR blink.
                    assert!(!s.add_modifier.contains(Modifier::SLOW_BLINK));
                    seen.insert(format!("{fg:?}"));
                }
                // It actually blinks: both phases appear across a cycle.
                assert_eq!(seen.len(), 2, "{flavor:?}/{p:?}: blink is flat");
            }
            // Mono has no ramp to blink: steady cursor treatment.
            let t = Theme::new(flavor, Profile::Mono);
            assert_eq!(t.move_blink(0), t.move_blink(5));
        }
    }

    /// The crowning (T-442): one solid head walks the run left to right and
    /// never back, the glow behind it cools toward the ground, every cell
    /// ends in the crown's tint, and the whole thing lands on the resting
    /// look — never in `attn`, never SGR 5.
    #[test]
    fn crown_sweep_fills_the_run_with_the_crowns_tint() {
        const N: usize = 24;
        let dist = |a: Option<Color>, b: Color| match (a, b) {
            (Some(Color::Rgb(r, g, b1)), Color::Rgb(x, y, z)) => {
                r.abs_diff(x) as u32 + g.abs_diff(y) as u32 + b1.abs_diff(z) as u32
            }
            _ => u32::MAX,
        };
        for flavor in Flavor::ALL {
            let t = Theme::new(flavor, Profile::TrueColor);
            let tint = t.pip(5);
            // Off the cursor the holder's title rests in the tint, unbolded,
            // so the head is the one bold cell.
            let resting = t.crown_text();
            let run =
                |ms| (0..N).map(|c| t.crown_sweep(c, N, ms, resting, t.bg)).collect::<Vec<_>>();
            assert_eq!(run(0), vec![resting; N], "{flavor:?}: lit on the first frame");
            let mut last = 0;
            for ms in (0..CROWN_SWEEP_MS).step_by(16) {
                let cells = run(ms);
                for s in &cells {
                    assert_ne!(s.fg, Some(t.attn), "{flavor:?} at {ms} ms");
                    assert_ne!(s.bg, Some(t.attn), "{flavor:?} at {ms} ms");
                    assert!(!s.add_modifier.contains(Modifier::SLOW_BLINK));
                }
                let heads: Vec<usize> =
                    (0..N).filter(|&c| cells[c].add_modifier.contains(Modifier::BOLD)).collect();
                assert!(heads.len() <= 1, "{flavor:?} at {ms} ms: heads {heads:?}");
                let Some(&h) = heads.first() else { continue };
                assert!(h >= last, "{flavor:?} at {ms} ms: the head went back to {h}");
                last = h;
                assert_eq!(cells[h].bg, Some(tint), "{flavor:?}: the head is the tint");
                assert_eq!(cells[h].fg, Some(t.tag_ink()), "{flavor:?}: in ground ink");
                // Behind the head the glow only cools.
                let lit: Vec<u32> = (0..h)
                    .rev()
                    .take_while(|&c| cells[c].bg.is_some())
                    .map(|c| dist(cells[c].bg, tint))
                    .collect();
                assert!(lit.windows(2).all(|w| w[0] <= w[1]), "{flavor:?} at {ms} ms: {lit:?}");
            }
            assert_eq!(last, N - 1, "{flavor:?}: the head never reached the end");
            // Filled: every letter in the tint, on its own ground.
            assert!(run(1_500).iter().all(|s| s.fg == Some(tint) && s.bg.is_none()), "{flavor:?}");
            assert_eq!(run(CROWN_FLASH_MS), vec![resting; N], "{flavor:?}: never settled");

            // Under the cursor the filled title eases back to the cursor's ink.
            let cursor = Style::default().fg(t.sel.base).add_modifier(Modifier::BOLD);
            let at = |ms| t.crown_sweep(3, N, ms, cursor, t.selected_bg);
            assert_eq!(at(1_500).fg, Some(tint), "{flavor:?}: filled under the cursor");
            let mid = at((CROWN_HOLD_MS + CROWN_FLASH_MS) / 2).fg;
            assert!(mid != Some(tint) && mid != Some(t.sel.base), "{flavor:?}: no ease back");
            assert_eq!(at(CROWN_FLASH_MS), cursor);
        }
    }

    /// Below TrueColor the head alone walks the run — the resting ink and its
    /// ground swapped, painted — and nothing fills; mono holds still.
    #[test]
    fn crown_sweep_below_truecolor_is_a_walking_head() {
        const N: usize = 12;
        for flavor in Flavor::ALL {
            for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8] {
                let t = Theme::new(flavor, p);
                let resting = t.crown_text();
                for ms in (0..CROWN_FLASH_MS).step_by(16) {
                    let changed: Vec<Style> = (0..N)
                        .map(|c| t.crown_sweep(c, N, ms, resting, t.bg))
                        .filter(|s| *s != resting)
                        .collect();
                    assert!(changed.len() <= 1, "{flavor:?}/{p:?} at {ms} ms: {changed:?}");
                    for s in changed {
                        assert_eq!((s.fg, s.bg), (t.bg, resting.fg), "{flavor:?}/{p:?}");
                        assert!(!s.add_modifier.contains(Modifier::REVERSED), "painted, not SGR 7");
                    }
                }
            }
            let t = Theme::new(flavor, Profile::Mono);
            let resting = t.crown_text();
            for ms in (0..CROWN_FLASH_MS).step_by(50) {
                assert!((0..N).all(|c| t.crown_sweep(c, N, ms, resting, t.bg) == resting));
            }
        }
    }
}
