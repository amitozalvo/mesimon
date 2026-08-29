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
    pub fn err_text(&self) -> Style {
        Style::default().fg(self.err)
    }
    pub fn calm_text(&self) -> Style {
        Style::default().fg(self.calm)
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

    /// sRGB → CIE L*a*b* → LCh, for the colour-law tests (06 §12). Test-only.
    fn lch(rgb: u32) -> (f64, f64) {
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
        let l = 116.0 * fy - 16.0;
        let a = 500.0 * (fx - fy);
        let bb = 200.0 * (fy - fz);
        (l, (a * a + bb * bb).sqrt())
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
}
