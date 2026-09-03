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

/// How loud a tag's colour is on a card. There is no alpha in a terminal, so
/// the levels are blends toward the page ground (`Theme::pip_at`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagLevel {
    /// The cursor card: full strength, so the card you are on carries the
    /// loudest tags on the board.
    Selected,
    /// A card at rest: one step down. This is the level almost every tag on
    /// the board is read at, and the step has to be big enough to SEE — the
    /// first cut used 0.82 and the boundary was invisible.
    ///
    /// Two levels, not three (author, 2026-09-02): a third, quieter one for
    /// a parked ticket shipped for a day and read as "too muted" — the glyph
    /// already says asleep, and the block's job is "which tag", loud enough
    /// to read. The block answers selected-or-not and nothing else.
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
    /// What `faded` blends toward. The page ground on paper; a NEUTRAL at the
    /// ground's lightness wherever the ground has a hue, because a tint
    /// blended into a coloured ground takes on the ground's hue and ten tags
    /// become one (measured: 169° of drift on navy, 47° on the green-black).
    pub shadow: u32,
}

pub(crate) struct Tints {
    pub ring: [u32; PIPS],
    /// The C* ceiling the law holds the ring under. 30.5 on paper; a ring
    /// that sits UNDER a C* 83 ground may be a step louder.
    #[cfg_attr(not(test), allow(dead_code))]
    pub ceiling: f64,
    /// The blend factor for `TagLevel::Rest`.
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
            // L* 62, C* 30. Measured: >= 6.18 on bg, >= 5.47 on the selected
            // surface; worst pair dE76 15.6.
            ring: [
                0xCB8381, 0x9F9761, 0x819F6D, 0x61A384, 0x41A4A1, 0x3BA2BA, 0x659BCA, 0x8F92C7,
                0xB288B6, 0xC6829D,
            ],
            ceiling: 30.5,
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
            // The same ten hues as graphite at L* 38, C* 26 — darker than
            // graphite's is light, because chalk's ground is the bright one,
            // and a step quieter in chroma because chalk's accent has less of
            // it to be a register above (C* 53.8 puts the ceiling at 26.9).
            // Measured: >= 6.52 on bg, >= 5.40 on the selected surface;
            // worst pair 13.2.
            ring: [
                0x824A49, 0x605A2F, 0x486039, 0x2D644B, 0x006562, 0x006274, 0x2D5D83, 0x535680,
                0x6F4E73, 0x7E495F,
            ],
            ceiling: 30.5,
            // Chalk fades LESS per step than graphite: the same sRGB ratio
            // costs far more toward white than toward black.
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
            // L* 70, C* 34: hues skip 60-120 (the gold) AND 276-336 (the
            // navy) — a tag the colour of the ground reads as ground. A step
            // louder than paper's ring because it sits under a C* 83 ground;
            // gold at C* 87 leaves the 2x margin whole. Measured: worst pair
            // dE76 15.1, >= 6.7 on bg, >= 4.5 on the selected surface.
            ring: [
                0x9BB477, 0x76B98F, 0x51BCAE, 0x3CBBCD, 0x57B5E2, 0x84ADE8, 0xD997C3, 0xE693A8,
                0xE7968E, 0xDC9D79,
            ],
            ceiling: 35.0,
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
            // Graphite's L* 62 / C* 30, on hues 127.5 + 29k: the 70° band
            // around the phosphor (77°) is skipped, since the ladder and
            // the accent are on that hue. Measured: worst pair dE 14.8,
            // >= 6.2 on bg, >= 5.4 on the selected surface, >= 49° from the
            // phosphor.
            ring: [
                0x849E6B, 0x66A380, 0x48A49A, 0x39A3B3, 0x4E9FC5, 0x7598CB, 0x9B8EC3, 0xB886B0,
                0xC88297, 0xCA847E,
            ],
            ceiling: 30.5,
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
            // The same ring on hues 192.5 + 29k, skipping 108-178 around the
            // phosphor (144°). Measured: worst pair dE 14.3, >= 6.2 on bg,
            // >= 5.3 on the selected surface, >= 49° from the phosphor.
            ring: [
                0x42A4A0, 0x3BA2B8, 0x579DC7, 0x7F95CA, 0xA38CBF, 0xBD85AB, 0xCA8291, 0xC88578,
                0xBB8C67, 0xA59560,
            ],
            ceiling: 30.5,
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
            ring: [
                0x824A49, 0x605A2F, 0x486039, 0x2D644B, 0x006562, 0x006274, 0x2D5D83, 0x535680,
                0x6F4E73, 0x7E495F,
            ],
            ceiling: 30.5,
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
    /// A tag pip's tint, `n = stable_hash(name) % PIPS`.
    ///
    /// Ten hues at one lightness (D31b): tag tints draw from their own ramp
    /// and may NEVER spend the one saturated colour reserved for "needs you"
    /// — the failure D19 names is that the alert stops being the only bright
    /// thing and users learn to distrust it within a week.
    ///
    /// The ramp was C* ~7 and it FAILED IN USE (2026-08-31): at that chroma
    /// the mark read as another grey rule and the tag said nothing. It is now
    /// C* 30 (graphite) / 26 (chalk) / 34 (blue) — a register below the
    /// accent, never beside it: `attn` keeps at least 2x the chroma of any
    /// tint. A colour nobody sees encodes nothing, and an unread tag is a
    /// worse outcome than a board with ten quiet hues on it.
    ///
    /// **The flavors do NOT share a ring.** Chalk's ground is paper, so a tint
    /// is INK on it and has to be darker than the paper by the same margin
    /// graphite's is lighter than its ground (author 2026-09-01: "barely
    /// visible on light theme"); blue's ring skips the navy's hue band as
    /// well as the gold's. The hues are even around the wheel except the
    /// bands skipped, because even spacing is what maximises the worst pair
    /// once the chroma ceiling is fixed. One ring at one lightness, NOT two
    /// rings of five: a second lightness would separate same-hue pairs by dL*
    /// alone (~dE 12) and is beaten by simply spacing ten hues on one ring; it
    /// would also make some tags louder than others, which is the one thing
    /// a tag axis may never do.
    ///
    /// This is the FULL strength of a tint, which only the cursor card wears;
    /// `pip_at` steps it down for the rest.
    ///
    /// Below TrueColor the tint is abandoned DELIBERATELY rather than
    /// approximated: the indexed cube has no low-chroma hue wheel, so any
    /// hand-assignment either collapses several hues onto one index or
    /// reaches for cells with visible chroma — and a *visible* tag colour is
    /// precisely the accent-spending failure. Ten indistinguishable tints are
    /// worse than none. Nothing is lost, because the pip is the tag's first
    /// letter: the letter was always the identity, the tint was the redundant
    /// half.
    ///
    /// A phosphor keeps its ring too. It shipped without one (ten foreign
    /// hues on a one-hue screen seemed the fiction broken) and the author
    /// asked for it back within the hour: a tag's colour is what the tag is
    /// FOR, and a theme does not get to take it away. The register-below-
    /// the-accent rule is restated in lightness there (`test_pip_ramp_*`),
    /// because a white-hot accent has no chroma for a tint to sit under.
    pub fn pip(&self, n: usize) -> Color {
        match self.tints() {
            Some(t) => hex(t.ring[n % PIPS]),
            None => self.rest.dim2,
        }
    }

    /// The same tint at the loudness the card has earned.
    ///
    /// There is no alpha in a terminal, so "less visible" is a blend toward
    /// the page ground — which is what the eye reads as a colour receding
    /// anyway, on either flavor: a tint fades DOWN into graphite and UP into
    /// chalk. The hue survives every level, because which tag it is remains
    /// the only thing the colour is there to say.
    pub(crate) fn pip_at(&self, n: usize, level: TagLevel) -> Color {
        self.faded(self.pip(n), level)
    }

    /// Any block colour at the loudness the card has earned — the tag tints
    /// go through here, and so does the NEUTRAL block of an untagged ticket.
    ///
    /// The levels are a property of the CARD, not of the palette: a board
    /// where only tagged tickets dim answers "is this the cursor card?" for
    /// some cards and not others, which is what the first cut did and what
    /// the author saw (2026-09-01).
    ///
    /// There are two: the cursor card at the full tint, every other card one
    /// step down. A third, quieter level for a parked ticket (0.38 graphite /
    /// 0.46 chalk) shipped 2026-09-01 and was cut the next day as too muted —
    /// the glyph says asleep, the block says which tag.
    ///
    /// The step is wide on purpose. 0.82 shipped first and the boundary was
    /// not visible on a real board: an 18% blend is nothing on a one-cell
    /// block, and a level nobody can tell from its neighbour is not a level.
    ///
    /// **The flavors need different numbers to mean the same thing.** The
    /// blend is a ratio in sRGB bytes, and the same ratio costs far more
    /// toward WHITE than toward black: chalk's resting tint was landing at
    /// C* 16.8 / contrast 2.82 where graphite's landed at 21.7 / 3.61, which
    /// is where "barely visible on light theme" came from (author
    /// 2026-09-01). Chalk therefore fades LESS (0.76) and its ramp starts
    /// darker (`pip`); together those put the chalk level at or above the
    /// graphite one it mirrors, while the step stays a step.
    ///
    /// **And the blend target is the palette's `shadow`, not always its
    /// ground.** On a navy ground a tint blended 62% into the ground is
    /// navy-hued whatever it started as (169° of drift, ten tags one colour);
    /// blue fades toward a neutral at the ground's lightness instead, which
    /// recedes exactly as far and keeps the hue.
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
        let Color::Rgb(r, g, b) = base else { return base };
        let Color::Rgb(gr, gg, gb) = hex(self.flavor.palette().truecolor.shadow) else {
            return base;
        };
        let mix = |a: u8, b: u8| (a as f32 * k + b as f32 * (1.0 - k)).round() as u8;
        Color::Rgb(mix(r, gr), mix(g, gg), mix(b, gb))
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

    /// The tag ring is chromatic — that is the point of it — but it stays a
    /// register below the accent (D31b: a tag may not spend the one saturated
    /// colour). Two numbers hold that line: the palette's own C* ceiling, and
    /// a 2x margin under `attn`. Legibility is held to >= 4.5 on the page
    /// ground (the ticket page paints a chip in the tint and writes the
    /// ground on it, so this IS that chip's text contrast) and >= 4.0 on the
    /// selected surface. The raises from C* 7 are recorded on `pip()`.
    ///
    /// The three levels are checked here too, because "less visible" must
    /// stop short of "gone": a resting tag still clears the dim2 body floor
    /// and a sleeping one still clears the dim3 de-emphasis floor — and it
    /// keeps its HUE: the navy ground proved a level can hold its chroma and
    /// still have become a different colour.
    ///
    /// A flavor with no ring is held to the opposite promise: every pip is
    /// the one grey, at every level. On a phosphor the accent is the beam,
    /// so the chroma clause holds as on paper — and the ring must also keep
    /// clear of the phosphor's own hue, which the ground and the accent wear.
    #[test]
    fn test_pip_ramp_is_low_chroma_and_legible() {
        for flavor in Flavor::ALL {
            let t = Theme::new(flavor, Profile::TrueColor);
            let tab = tc(flavor);
            let Some(tints) = &tab.tints else {
                for n in 0..PIPS {
                    assert_eq!(t.pip(n), t.rest.dim2, "{flavor:?} pip {n} has a tint");
                }
                assert!(!t.paints_tags(), "{flavor:?} claims to paint tags without a ring");
                continue;
            };
            let (bg, selbg) = (tab.bg, tab.selected);
            let (_, c_attn) = lch(tab.attn);
            for n in 0..PIPS {
                let full = rgb(t.pip(n));
                let (_, c) = lch(full);
                assert!(
                    c <= tints.ceiling,
                    "{flavor:?} pip {n} {full:06X} has C* {c:.1} > {:.1}",
                    tints.ceiling
                );
                // A register below the accent, and it must stay there.
                assert!(
                    c_attn >= c * 2.0,
                    "{flavor:?} pip {n} C* {c:.1} is not a register below attn C* {c_attn:.1}"
                );
                match flavor.kind() {
                    Kind::Paper | Kind::ChromaticGround | Kind::TintedPaper => {}
                    Kind::Phosphor | Kind::Ladder => {
                        let gap = hue_gap(full, tab.attn);
                        assert!(gap >= 35.0, "{flavor:?} pip {n} is {gap:.1}° from the phosphor");
                    }
                }
                for (surface, floor) in [(bg, 4.5), (selbg, 4.0)] {
                    let k = contrast(full, surface);
                    assert!(k >= floor, "{flavor:?} pip {n} {full:06X} on {surface:06X} is {k:.2}");
                }
                // The quieter level: still seen, still hued, never gone.
                // A floor, not a target: the resting level is allowed under
                // the body-text floor, because it is paint and not text —
                // what it may never do is stop being a colour.
                {
                    let (level, floor) = (TagLevel::Rest, 2.8);
                    let faded = rgb(t.pip_at(n, level));
                    let k = contrast(faded, bg);
                    assert!(k >= floor, "{flavor:?} {level:?} pip {n} {faded:06X} is {k:.2}");
                    let (_, cf) = lch(faded);
                    assert!(cf >= 8.0, "{flavor:?} {level:?} pip {n} lost its hue: C* {cf:.1}");
                    let drift = hue_gap(faded, full);
                    assert!(drift <= 20.0, "{flavor:?} {level:?} pip {n} drifted {drift:.1}°");
                    // Each step has to be visible as a step, not just be a
                    // different number: >= 12% of the ground-to-tint distance.
                    let step = |a: u32, b: u32| {
                        let ch = |v: u32, s: u32| ((v >> s) & 255) as f64;
                        (0..3).map(|i| (ch(a, i * 8) - ch(b, i * 8)).abs()).sum::<f64>()
                    };
                    let span = step(full, bg);
                    assert!(
                        step(faded, full) >= span * 0.12,
                        "{flavor:?} {level:?} pip {n} is not far enough from the tint"
                    );
                }
            }
            // DISTINCT hues, or the ramp encodes nothing — and distinct by
            // enough to tell apart in a one-cell block, which is the only
            // place most of them are ever seen. dE76 13 is the worst pair the
            // chalk ceiling allows at ten hues; anything under 12 means the
            // ramp has been stretched past what it can hold.
            let mut seen: Vec<u32> = Vec::new();
            for n in 0..PIPS {
                let c = rgb(t.pip(n));
                for (m, prev) in seen.iter().enumerate() {
                    let d = delta_e(c, *prev);
                    assert!(d >= 12.0, "{flavor:?} pips {m} and {n} are dE {d:.1} apart");
                }
                seen.push(c);
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
}
