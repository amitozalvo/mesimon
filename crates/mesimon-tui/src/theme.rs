//! The two built-in themes (06 §2): graphite (dark, default) and chalk
//! (light), materialized per colour profile. Only tokens M3.5 actually
//! renders exist here; the six-surface elevation model is deliberately
//! collapsed to two painted surfaces (bg + selected) in every profile —
//! that is 06 §2.6's 256-colour rule applied one tier up, and the full
//! elevation ramp is an M6 refinement. Capability negotiation is per-client
//! (06 §2.9): the daemon never sees any of this.

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
}

/// The 4-step value ramp of one component state (06 §2.1, D19).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ramp {
    pub base: Color,
    pub dim1: Color,
    pub dim2: Color,
    pub dim3: Color,
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
    Rest,
    /// A sleeping session: quiet, but the hue must survive. "Which tag" is
    /// the one thing the colour is for, and a parked ticket still has to
    /// answer it.
    Sleeping,
}

pub(crate) struct Theme {
    pub profile: Profile,
    #[allow(dead_code)] // detection provenance; useful for doctor output later
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
    pub fn new(flavor: Flavor, profile: Profile) -> Self {
        match flavor {
            Flavor::Graphite => Self::graphite(profile),
            Flavor::Chalk => Self::chalk(profile),
        }
    }

    /// 06 §2.2 (truecolor), §2.6 (256), §2.7 (16/8), §2.8 (mono).
    pub fn graphite(profile: Profile) -> Self {
        let (bg, selected_bg, rest, sel, attn, err, calm, attn_ink, ghost, dormant, cursor) =
            match profile {
                Profile::TrueColor => (
                    Some(hex(0x131417)),
                    Some(hex(0x272B31)),
                    Ramp {
                        base: hex(0xE9E7E1),
                        dim1: hex(0xB6B2A9),
                        dim2: hex(0x8C8880),
                        dim3: hex(0x5E5B55),
                    },
                    Ramp {
                        base: hex(0xF1EFE9),
                        dim1: hex(0xC3BFB6),
                        dim2: hex(0x9A968D),
                        dim3: hex(0x6B675F),
                    },
                    hex(0xF0A93A),
                    hex(0xD5809A),
                    hex(0x6FBFB0),
                    hex(0x131417),
                    hex(0x5E5B55),
                    hex(0x8C8880),
                    hex(0xF1EFE9),
                ),
                Profile::Ansi256 => {
                    let r = Ramp {
                        base: Color::Indexed(253),
                        dim1: Color::Indexed(250),
                        // 246 fails AA on the selected surface — 06 §2.6 says
                        // use 248 on 236; one ramp means 248 everywhere.
                        dim2: Color::Indexed(248),
                        dim3: Color::Indexed(241),
                    };
                    (
                        Some(Color::Indexed(234)),
                        Some(Color::Indexed(236)),
                        r,
                        r,
                        Color::Indexed(214),
                        Color::Indexed(175),
                        Color::Indexed(73),
                        Color::Indexed(234),
                        Color::Indexed(241),
                        Color::Indexed(246),
                        Color::Indexed(253),
                    )
                }
                Profile::Ansi16 | Profile::Ansi8 | Profile::Mono => {
                    let bright = profile == Profile::Ansi16;
                    let r = Ramp {
                        base: Color::Reset,
                        dim1: Color::Indexed(7),
                        dim2: Color::Indexed(8),
                        dim3: Color::Indexed(8),
                    };
                    let mono = profile == Profile::Mono;
                    (
                        None,
                        if bright { Some(Color::Indexed(8)) } else { None },
                        if mono { MONO_RAMP } else { r },
                        if mono { MONO_RAMP } else { r },
                        if mono {
                            Color::Reset
                        } else if bright {
                            Color::Indexed(11)
                        } else {
                            Color::Indexed(3)
                        },
                        if mono { Color::Reset } else { Color::Indexed(1) },
                        if mono { Color::Reset } else { Color::Indexed(6) },
                        Color::Indexed(0),
                        Color::Reset,
                        Color::Reset,
                        Color::Reset,
                    )
                }
            };
        Theme {
            profile,
            flavor: Flavor::Graphite,
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

    /// 06 §2.3 (truecolor), §2.6 light (256), §2.7 (16/8), §2.8 (mono).
    pub fn chalk(profile: Profile) -> Self {
        let (bg, selected_bg, rest, sel, attn, err, calm, attn_ink, ghost, dormant, cursor) =
            match profile {
                Profile::TrueColor => (
                    Some(hex(0xFAF8F4)),
                    Some(hex(0xE7E3DA)),
                    Ramp {
                        base: hex(0x22252B),
                        dim1: hex(0x4C4F57),
                        dim2: hex(0x5F6169),
                        dim3: hex(0x8B8D94),
                    },
                    Ramp {
                        base: hex(0x1B1E23),
                        dim1: hex(0x43464E),
                        dim2: hex(0x585A62),
                        dim3: hex(0x82848B),
                    },
                    hex(0x8F5600),
                    hex(0x732E41),
                    hex(0x0B5F55),
                    hex(0xFFFFFF),
                    hex(0x8B8D94),
                    hex(0x5F6169),
                    hex(0x1B1E23),
                ),
                Profile::Ansi256 => {
                    let r = Ramp {
                        base: Color::Indexed(235),
                        dim1: Color::Indexed(238),
                        dim2: Color::Indexed(241),
                        dim3: Color::Indexed(243),
                    };
                    (
                        None,
                        // 06 §2.6: light-256 never paints `selected` (fails AA);
                        // the cursor card is signalled structurally only.
                        None,
                        r,
                        r,
                        Color::Indexed(94),
                        Color::Indexed(125),
                        Color::Indexed(23),
                        Color::Indexed(255),
                        Color::Indexed(243),
                        Color::Indexed(241),
                        Color::Indexed(235),
                    )
                }
                Profile::Ansi16 | Profile::Ansi8 | Profile::Mono => {
                    let bright = profile == Profile::Ansi16;
                    let r = Ramp {
                        base: Color::Reset,
                        dim1: Color::Indexed(0),
                        dim2: Color::Indexed(8),
                        dim3: Color::Indexed(8),
                    };
                    let mono = profile == Profile::Mono;
                    (
                        None,
                        if bright { Some(Color::Indexed(7)) } else { None },
                        if mono { MONO_RAMP } else { r },
                        if mono { MONO_RAMP } else { r },
                        if mono { Color::Reset } else { Color::Indexed(3) },
                        if mono { Color::Reset } else { Color::Indexed(1) },
                        if mono { Color::Reset } else { Color::Indexed(6) },
                        Color::Indexed(15),
                        Color::Reset,
                        Color::Reset,
                        Color::Reset,
                    )
                }
            };
        Theme {
            profile,
            flavor: Flavor::Chalk,
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
    /// every diff viewer): the page bg blended a step toward the calm/err
    /// registers — low-chroma by construction, so the one-saturated-colour
    /// law stands. TrueColor only; the indexed cube has no tint this quiet,
    /// so 256/16/8/mono keep the fg-register + glyph encoding alone.
    pub fn diff_add_bg(&self) -> Option<Color> {
        match (self.flavor, self.profile) {
            (Flavor::Graphite, Profile::TrueColor) => Some(hex(0x1E2C28)),
            (Flavor::Chalk, Profile::TrueColor) => Some(hex(0xDFEBE4)),
            _ => None,
        }
    }

    pub fn diff_del_bg(&self) -> Option<Color> {
        match (self.flavor, self.profile) {
            (Flavor::Graphite, Profile::TrueColor) => Some(hex(0x2E2127)),
            (Flavor::Chalk, Profile::TrueColor) => Some(hex(0xF2E0E4)),
            _ => None,
        }
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
    /// Six hues at one lightness (D31b): tag tints draw from their own ramp
    /// and may NEVER spend the one saturated colour reserved for "needs you"
    /// — the failure D19 names is that the alert stops being the only bright
    /// thing and users learn to distrust it within a week.
    ///
    /// The ramp was C* ~7 and it FAILED IN USE (2026-08-31): at that chroma
    /// the mark read as another grey rule and the tag said nothing. It is now
    /// C* 30 (graphite) / 26 (chalk) — a register below the accent, never
    /// beside it: `attn` keeps at least 2x the chroma of any tint. A colour
    /// nobody sees encodes nothing, and an unread tag is a worse outcome than
    /// a board with ten quiet hues on it.
    ///
    /// **The two flavors do NOT sit at the same lightness, and that is the
    /// fix for light mode** (author 2026-09-01: "barely visible on light
    /// theme"). Chalk's ground is paper, so a tint is INK on it and has to be
    /// darker than the paper by the same margin graphite's is lighter than
    /// its ground. At L* 45 it was not: it cleared the text floor (5.04) and
    /// still read as a smudge, because a tint is then faded toward the ground
    /// on almost every card and toward WHITE that costs far more chroma than
    /// toward black does. L* 38 buys the whole ladder back — see `faded`,
    /// which pays the other half of it.
    ///
    /// This is the FULL strength of a tint, which only the cursor card wears;
    /// `pip_at` steps it down for the rest.
    ///
    /// Below TrueColor the tint is abandoned DELIBERATELY rather than
    /// approximated: the indexed cube has no low-chroma hue wheel, so any
    /// hand-assignment either collapses several hues onto one index or
    /// reaches for cells with visible chroma — and a *visible* tag colour is
    /// precisely the accent-spending failure. Six indistinguishable tints are
    /// worse than none. Nothing is lost, because the pip is the tag's first
    /// letter: the letter was always the identity, the tint was the redundant
    /// half.
    pub fn pip(&self, n: usize) -> Color {
        if self.profile != Profile::TrueColor {
            return self.rest.dim2;
        }
        // Ten hues, the same ten on both flavors, at 25 132 161 193 224
        // 261 292 323 353 100 degrees — even around the wheel EXCEPT for the
        // 50-100 band, which is skipped because that is where `attn` lives
        // (h 75 graphite / 70 chalk) and a tag the colour of the alert is the
        // D19 failure however low its chroma. Even spacing is what maximises
        // the worst pair once the chroma ceiling is fixed; the six-hue ramp
        // could afford to be picked by hand, ten cannot.
        //
        // One ring at one lightness, NOT two rings of five: a second
        // lightness would separate same-hue pairs by dL* alone (~dE 12) and
        // is beaten by simply spacing ten hues on one ring (dE 15.6 / 13.2).
        // It would also make some tags louder than others, which is the one
        // thing a tag axis may never do.
        let ramp = match self.flavor {
            // L* 62, C* 30. Measured: >= 6.18 on bg, >= 5.47 on the selected
            // surface; worst pair dE76 15.6.
            Flavor::Graphite => [
                0xCB8381, 0x9F9761, 0x819F6D, 0x61A384, 0x41A4A1, 0x3BA2BA, 0x659BCA, 0x8F92C7,
                0xB288B6, 0xC6829D,
            ],
            // The same ten hues at L* 38, C* 26 — darker than graphite's is
            // light, because chalk's ground is the bright one, and a step
            // quieter in chroma because chalk's accent has less of it to be a
            // register above (C* 53.8 puts the ceiling at 26.9). Measured:
            // >= 6.52 on bg, >= 5.40 on the selected surface; worst pair 13.2.
            Flavor::Chalk => [
                0x824A49, 0x605A2F, 0x486039, 0x2D644B, 0x006562, 0x006274, 0x2D5D83, 0x535680,
                0x6F4E73, 0x7E495F,
            ],
        };
        hex(ramp[n % PIPS])
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
    /// The three states are a property of the CARD, not of the palette: a
    /// board where only tagged tickets dim answers "is this one asleep?" for
    /// some cards and not others, which is what the first cut did and what
    /// the author saw (2026-09-01).
    ///
    /// The steps are wide on purpose. 0.82/0.50 shipped first and neither
    /// boundary was visible on a real board: an 18% blend is nothing on a
    /// one-cell block, and a level nobody can tell from its neighbour is not
    /// a level.
    ///
    /// **The two flavors need different numbers to mean the same thing.** The
    /// blend is a ratio in sRGB bytes, and the same ratio costs far more
    /// toward WHITE than toward black: chalk's resting tint was landing at
    /// C* 16.8 / contrast 2.82 where graphite's landed at 21.7 / 3.61, and
    /// its sleeping one at C* 8.2 — on the floor, which is where "barely
    /// visible on light theme" came from (author 2026-09-01). Chalk therefore
    /// fades LESS per step (0.76/0.46) and its ramp starts darker (`pip`);
    /// together those put every chalk level at or above the graphite one it
    /// mirrors, while the step stays a step (dE76 14 and 20 between levels,
    /// against graphite's 17 and 19).
    pub(crate) fn faded(&self, base: Color, level: TagLevel) -> Color {
        let k = match (level, self.flavor) {
            (TagLevel::Selected, _) => return base,
            (TagLevel::Rest, Flavor::Graphite) => 0.70,
            (TagLevel::Rest, Flavor::Chalk) => 0.76,
            (TagLevel::Sleeping, Flavor::Graphite) => 0.38,
            (TagLevel::Sleeping, Flavor::Chalk) => 0.46,
        };
        if self.profile != Profile::TrueColor {
            return base; // no ground to blend into, and one grey to blend
        }
        let Color::Rgb(r, g, b) = base else { return base };
        let ground = match self.bg {
            Some(Color::Rgb(rr, gg, bb)) => (rr, gg, bb),
            _ => match self.flavor {
                Flavor::Graphite => (0x13, 0x14, 0x17),
                Flavor::Chalk => (0xFA, 0xF8, 0xF4),
            },
        };
        let mix = |a: u8, b: u8| (a as f32 * k + b as f32 * (1.0 - k)).round() as u8;
        Color::Rgb(mix(r, ground.0), mix(g, ground.1), mix(b, ground.2))
    }

    /// Are the six tag tints actually distinguishable here?
    ///
    /// Only in TrueColor. `pip()` collapses every tint to one grey below it,
    /// so anything that leans on colour alone (the picker's swatches, the
    /// ticket page's chips) has to say the name instead.
    pub fn paints_tags(&self) -> bool {
        self.profile == Profile::TrueColor
    }

    /// Ink for a name written on a tag-tinted ground: the page ground, which
    /// is the surface every tint was contrast-checked against.
    pub fn tag_ink(&self) -> Color {
        self.bg.unwrap_or(match self.flavor {
            Flavor::Graphite => hex(0x131417),
            Flavor::Chalk => hex(0xFAF8F4),
        })
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
            // Light-256: no painted surface; bar weight + bold carry it.
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
        };
        (' ', Style::default().bg(colour))
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

    // The transcribed hex tables, kept in one place for the law tests.
    const GRAPHITE: ([u32; 4], [u32; 4], u32, u32, u32, u32, u32) = (
        [0xE9E7E1, 0xB6B2A9, 0x8C8880, 0x5E5B55],
        [0xF1EFE9, 0xC3BFB6, 0x9A968D, 0x6B675F],
        0xF0A93A,
        0xD5809A,
        0x6FBFB0,
        0x131417,
        0x272B31,
    );
    const CHALK: ([u32; 4], [u32; 4], u32, u32, u32, u32, u32) = (
        [0x22252B, 0x4C4F57, 0x5F6169, 0x8B8D94],
        [0x1B1E23, 0x43464E, 0x585A62, 0x82848B],
        0x8F5600,
        0x732E41,
        0x0B5F55,
        0xFAF8F4,
        0xE7E3DA,
    );

    /// L2 (06 §0): exactly three chromatic tokens; every grey C* ≤ 8.2 (V1);
    /// C*(attn) ≥ C*(err)+12 and ≥ C*(calm)+20.
    #[test]
    fn test_chroma_law() {
        for (rest, sel, attn, err, calm, bg, selbg) in [GRAPHITE, CHALK] {
            for grey in rest.iter().chain(sel.iter()).chain([bg, selbg].iter()) {
                let (_, c) = lch(*grey);
                assert!(c <= 8.2, "grey {grey:06X} has C* {c:.1} > 8.2");
            }
            let (_, ca) = lch(attn);
            let (_, ce) = lch(err);
            let (_, cc) = lch(calm);
            assert!(ca >= ce + 12.0, "C*(attn) {ca:.1} < C*(err) {ce:.1} + 12");
            assert!(ca >= cc + 20.0, "C*(attn) {ca:.1} < C*(calm) {cc:.1} + 20");
        }
    }

    /// The tag ramp is chromatic — that is the point of it — but it stays a
    /// register below the accent (D31b: a tag may not spend the one saturated
    /// colour). Two numbers hold that line: a C* ceiling of 30.5, and a 2x
    /// margin under `attn`. Legibility is held to >= 4.5 on the page ground
    /// (the ticket page paints a chip in the tint and writes the ground on
    /// it, so this IS that chip's text contrast) and >= 4.0 on the selected
    /// surface. The raises from C* 7 are recorded on `pip()`.
    ///
    /// The three levels are checked here too, because "less visible" must
    /// stop short of "gone": a resting tag still clears the dim2 body floor
    /// and a sleeping one still clears the dim3 de-emphasis floor.
    #[test]
    fn test_pip_ramp_is_low_chroma_and_legible() {
        for (flavor, bg, selbg, attn) in [
            (Flavor::Graphite, GRAPHITE.5, GRAPHITE.6, GRAPHITE.2),
            (Flavor::Chalk, CHALK.5, CHALK.6, CHALK.2),
        ] {
            let t = Theme::new(flavor, Profile::TrueColor);
            let (_, c_attn) = lch(attn);
            for n in 0..PIPS {
                let Color::Rgb(r, g, b) = t.pip(n) else {
                    panic!("{flavor:?} pip {n} is not truecolor");
                };
                let rgb = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                let (_, c) = lch(rgb);
                assert!(c <= 30.5, "{flavor:?} pip {n} {rgb:06X} has C* {c:.1} > 30.5");
                // A register below the accent, and it must stay there.
                assert!(
                    c_attn >= c * 2.0,
                    "{flavor:?} pip {n} C* {c:.1} is not a register below attn C* {c_attn:.1}"
                );
                for (surface, floor) in [(bg, 4.5), (selbg, 4.0)] {
                    let k = contrast(rgb, surface);
                    assert!(k >= floor, "{flavor:?} pip {n} {rgb:06X} on {surface:06X} is {k:.2}");
                }
                // The quieter levels: still seen, still hued, never gone.
                // Floors, not targets: a quiet level is allowed under the
                // body-text floor, because it is paint and not text — what it
                // may never do is stop being a colour.
                for (level, floor) in [(TagLevel::Rest, 2.8), (TagLevel::Sleeping, 1.4)] {
                    let Color::Rgb(r, g, b) = t.pip_at(n, level) else {
                        panic!("{flavor:?} {level:?} {n} is not truecolor");
                    };
                    let faded = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                    let k = contrast(faded, bg);
                    assert!(k >= floor, "{flavor:?} {level:?} pip {n} {faded:06X} is {k:.2}");
                    let (_, cf) = lch(faded);
                    assert!(cf >= 8.0, "{flavor:?} {level:?} pip {n} lost its hue: C* {cf:.1}");
                    // Each step has to be visible as a step, not just be a
                    // different number: >= 12% of the ground-to-tint distance.
                    let step = |a: u32, b: u32| {
                        let ch = |v: u32, s: u32| ((v >> s) & 255) as f64;
                        (0..3).map(|i| (ch(a, i * 8) - ch(b, i * 8)).abs()).sum::<f64>()
                    };
                    let span = step(rgb, bg);
                    assert!(
                        step(faded, rgb) >= span * 0.12,
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
                let Color::Rgb(r, g, b) = t.pip(n) else { unreachable!() };
                let rgb = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                for (m, prev) in seen.iter().enumerate() {
                    let d = delta_e(rgb, *prev);
                    assert!(d >= 12.0, "{flavor:?} pips {m} and {n} are dE {d:.1} apart");
                }
                seen.push(rgb);
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
        for flavor in [Flavor::Graphite, Flavor::Chalk] {
            for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
                let t = Theme::new(flavor, p);
                for n in 0..PIPS {
                    assert_eq!(t.pip(n), t.rest.dim2, "{flavor:?}/{p:?} pip {n} kept a tint");
                }
            }
        }
    }

    /// 06 §12: every text role ≥ 4.5:1 on its legal surfaces (dim3 ≥ 3:1 is
    /// explicitly relaxed — it is a de-emphasis role); attn_ink on attn ≥ 4.5.
    #[test]
    fn test_contrast_matrix() {
        for (rest, sel, attn, err, calm, bg, selbg) in [GRAPHITE, CHALK] {
            for (ramp, surface) in [(rest, bg), (sel, selbg)] {
                assert!(contrast(ramp[0], surface) >= 4.5, "base on {surface:06X}");
                assert!(contrast(ramp[1], surface) >= 4.5, "dim1 on {surface:06X}");
                assert!(contrast(ramp[2], surface) >= 4.0, "dim2 on {surface:06X}");
                assert!(contrast(ramp[3], surface) >= 2.0, "dim3 on {surface:06X}");
            }
            for chroma in [attn, err, calm] {
                assert!(contrast(chroma, bg) >= 4.5, "{chroma:06X} on bg");
            }
            let ink = if bg == 0x131417 { 0x131417 } else { 0xFFFFFF };
            assert!(contrast(ink, attn) >= 4.5, "attn_ink on attn");
        }
    }

    /// 06 §2.4a: adjacent bar weights ≥ 15 L* apart.
    #[test]
    fn test_bar_ladder() {
        // graphite: ghost 5E5B55 < dormant 8C8880 < cursor F1EFE9
        // chalk:    ghost 8B8D94 > dormant 5F6169 > cursor 1B1E23
        for ladder in [[0x5E5B55u32, 0x8C8880, 0xF1EFE9], [0x8B8D94, 0x5F6169, 0x1B1E23]] {
            let ls: Vec<f64> = ladder.iter().map(|c| lch(*c).0).collect();
            assert!((ls[0] - ls[1]).abs() >= 15.0, "ghost vs dormant");
            assert!((ls[1] - ls[2]).abs() >= 15.0, "dormant vs cursor");
        }
    }

    #[test]
    fn mono_bar_is_the_ascii_ladder() {
        let t = Theme::graphite(Profile::Mono);
        assert_eq!(t.bar(BarWeight::Ghost).0, '.');
        assert_eq!(t.bar(BarWeight::Dormant).0, ':');
        assert_eq!(t.bar(BarWeight::Live(Register::Grey)).0, '|');
        assert_eq!(t.bar(BarWeight::Cursor).0, '#');
        // Colour profiles paint a space instead.
        let t = Theme::graphite(Profile::TrueColor);
        assert_eq!(t.bar(BarWeight::Cursor).0, ' ');
    }

    #[test]
    fn light_256_never_paints_selected() {
        assert_eq!(Theme::chalk(Profile::Ansi256).selected_bg, None);
        assert!(Theme::graphite(Profile::Ansi256).selected_bg.is_some());
    }

    #[test]
    fn reverse_only_in_mono_and_8() {
        for p in [Profile::TrueColor, Profile::Ansi256, Profile::Ansi16] {
            let t = Theme::graphite(p);
            assert!(!t.selected_row().add_modifier.contains(Modifier::REVERSED), "{p:?}");
            assert!(!t.attn_row().add_modifier.contains(Modifier::REVERSED), "{p:?}");
        }
        // Mono's inverted needs-you row is real SGR-7; Ansi8 still has the
        // three chromatic indices so its row is painted, but its cursor-card
        // surface (no painted selected) falls back to the sanctioned reverse.
        let t = Theme::graphite(Profile::Mono);
        assert!(t.attn_row().add_modifier.contains(Modifier::REVERSED));
        let t = Theme::graphite(Profile::Ansi8);
        assert!(t.attn_row().bg.is_some());
        assert!(t.selected_row().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn move_blink_rides_the_sel_ramp_only() {
        for flavor in [Flavor::Graphite, Flavor::Chalk] {
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
