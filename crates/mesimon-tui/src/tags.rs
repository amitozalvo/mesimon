//! Tag rendering: the colour bands stacked under a card, and the tint ramp
//! they draw from.
//!
//! A tag is a **painted band across the bottom of the card block**, one row
//! per tag. Not a glyph and not text: at rest the colour alone says which
//! tags a ticket wears, and `p` writes the names onto the bands.
//!
//! Two constraints shape the band and are not negotiable:
//!
//! - **It is painted, never drawn.** The obvious ways to draw a rule —
//!   `─` U+2500, `▁` U+2581, `█` U+2588 — all sit in the `0x2500–0x259F`
//!   range the L1 no-drawn-structure law bans board-wide
//!   (`test_no_drawn_structure`). A band is spaces with a background colour,
//!   the same trick the accent bar uses.
//! - **The tints stay low-chroma.** D19 reserves exactly one saturated colour
//!   for "needs you", and a band is a lot more ink than a pip, so the ramp
//!   matters more here, not less. Tab cycles a tag through the six tints in
//!   `Theme::pip` and nothing else — there is no free-colour path.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use mesimon_core::board::{Board, TagRef};

use crate::theme::Theme;

/// A tag resolved for rendering: its name and the tint index the registry
/// says to paint it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Painted {
    pub name: String,
    pub tint: u8,
}

/// Resolve a ticket's tag references against the registry, in group order so
/// the bands never reshuffle under the reader.
pub(crate) fn painted(board: &Board, tags: &[TagRef]) -> Vec<Painted> {
    let mut out: Vec<Painted> =
        tags.iter().map(|t| Painted { name: t.name.clone(), tint: board.tint_of(t) }).collect();
    out.sort_by_key(|_| 0); // keep input order; tags are already group-sorted
    out
}

/// One band: `width` cells painted in the tag's tint.
///
/// With `name`, the name is written across it in the page ground, which is
/// the peek reveal. Without, the band is solid — the minimal indication.
pub(crate) fn band(theme: &Theme, tag: &Painted, width: usize, named: bool) -> Vec<Span<'static>> {
    // Below TrueColor the tint collapses to one grey, so six identical bands
    // would say less than six names do. The row keeps its place in the layout
    // and carries the name instead — the same call the ramp makes everywhere.
    if !theme.paints_bands() {
        let label = crate::text::truncate(&format!("#{}", tag.name), width);
        let pad = width.saturating_sub(label.chars().count());
        return vec![
            Span::styled(label, theme.dim2()),
            Span::styled(" ".repeat(pad), Style::default()),
        ];
    }
    let tint = theme.pip(tag.tint as usize);
    let painted = Style::default().bg(tint);
    if !named {
        return vec![Span::styled(" ".repeat(width), painted)];
    }
    // Ink that reads on the band: the page ground, which every tint was
    // contrast-checked against.
    let ink = Style::default().bg(tint).fg(theme.band_ink()).add_modifier(Modifier::BOLD);
    let label = crate::text::truncate(&tag.name, width.saturating_sub(2));
    let pad = width.saturating_sub(label.chars().count() + 1);
    vec![
        Span::styled(" ".to_string(), painted),
        Span::styled(label, ink),
        Span::styled(" ".repeat(pad), painted),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};
    use unicode_width::UnicodeWidthStr;

    fn tags(n: usize) -> Vec<Painted> {
        (0..n).map(|i| Painted { name: format!("T{i}"), tint: i as u8 % 6 }).collect()
    }

    /// A band fills its width exactly, named or not. One cell over and the
    /// card's right edge strands a painted cell the diff never repaints.
    #[test]
    fn a_band_is_exactly_its_width() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for width in [4usize, 10, 26, 40] {
            for named in [false, true] {
                for t in tags(3) {
                    let w: usize =
                        band(&theme, &t, width, named).iter().map(|s| s.content.width()).sum();
                    assert_eq!(w, width, "width {width}, named {named}, tag {}", t.name);
                }
            }
        }
    }

    /// A name too long for the band truncates rather than overflowing.
    #[test]
    fn a_long_name_truncates_into_the_band() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let t = Painted { name: "a-very-long-tag-name-indeed".into(), tint: 0 };
        let spans = band(&theme, &t, 12, true);
        let w: usize = spans.iter().map(|s| s.content.width()).sum();
        assert_eq!(w, 12);
    }

    /// The band is PAINTED, never drawn: no codepoint may land in the
    /// structure range the L1 law bans, whatever the tier.
    #[test]
    fn bands_never_use_drawn_structure() {
        for (flavor, profile) in [
            (Flavor::Graphite, Profile::TrueColor),
            (Flavor::Chalk, Profile::Ansi256),
            (Flavor::Graphite, Profile::Mono),
        ] {
            let theme = Theme::new(flavor, profile);
            for t in tags(6) {
                for named in [false, true] {
                    for s in band(&theme, &t, 20, named) {
                        for ch in s.content.chars() {
                            let cp = ch as u32;
                            assert!(
                                !(0x2500..=0x259F).contains(&cp),
                                "{flavor:?}/{profile:?} band used drawn structure {ch:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
