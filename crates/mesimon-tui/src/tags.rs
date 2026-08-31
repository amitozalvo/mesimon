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

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

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

/// Paint the tag colours onto a card line as a segmented UNDERLINE.
///
/// The line is the card block's bottom row, so the colour hugs the card
/// instead of costing it one. With several tags the row divides left to
/// right, one equal segment each — the proportions make the count readable
/// without anyone counting.
///
/// An underline rather than a glyph because every glyph that would draw a
/// rule — `▀` U+2580, `▔` U+2594, `█` U+2588 — is inside the `0x2500–0x259F`
/// range the L1 law bans AND is East Asian Width *Ambiguous*, the class that
/// already cost this project a render bug: the terminal spends two cells, the
/// width crate counts one, and every later cell shifts right leaving paint
/// the diff never repaints. SGR 58 has no width at all, so it cannot.
///
/// Degradation is honest rather than silent: a terminal without SGR 58 still
/// draws the underline, just in the row's own foreground — you can still see
/// that the ticket is tagged, only not with which.
pub(crate) fn underline(
    theme: &Theme,
    line: Line<'static>,
    tags: &[Painted],
    width: usize,
) -> Line<'static> {
    if tags.is_empty() || width == 0 {
        return line;
    }
    // Which tag owns column `x`.
    let owner = |x: usize| -> usize { (x * tags.len() / width).min(tags.len() - 1) };

    let mut out: Vec<Span<'static>> = Vec::new();
    let mut x = 0usize;
    for span in line.spans {
        // A span can straddle a boundary, so cut it where the owner changes.
        let mut chunk = String::new();
        let mut chunk_owner = owner(x);
        for ch in span.content.chars() {
            let o = owner(x);
            if o != chunk_owner && !chunk.is_empty() {
                out.push(paint(theme, &span, std::mem::take(&mut chunk), tags[chunk_owner].tint));
                chunk_owner = o;
            }
            chunk.push(ch);
            x += ch.width().unwrap_or(0);
        }
        if !chunk.is_empty() {
            out.push(paint(theme, &span, chunk, tags[chunk_owner].tint));
        }
    }
    Line::from(out).style(line.style)
}

fn paint(theme: &Theme, span: &Span<'static>, text: String, tint: u8) -> Span<'static> {
    let style =
        span.style.add_modifier(Modifier::UNDERLINED).underline_color(theme.pip(tint as usize));
    Span::styled(text, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};
    use ratatui::style::Style;
    use unicode_width::UnicodeWidthStr;

    fn tags(n: usize) -> Vec<Painted> {
        (0..n).map(|i| Painted { name: format!("T{i}"), tint: i as u8 % 6 }).collect()
    }

    fn line(text: &str) -> Line<'static> {
        Line::from(vec![Span::styled(text.to_string(), Style::default())])
    }

    /// The row keeps its exact width and text: the tags ride the underline,
    /// so they cost the card no cell and no row.
    #[test]
    fn underlining_changes_no_text() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for n in 0..5usize {
            let before = line("Fix OSC-11 detection   >1y");
            let want: String = before.spans.iter().map(|s| s.content.to_string()).collect();
            let after = underline(&theme, before, &tags(n), 26);
            let got: String = after.spans.iter().map(|s| s.content.to_string()).collect();
            assert_eq!(got, want, "{n} tags changed the text");
            assert_eq!(got.width(), 26);
        }
    }

    /// An untagged ticket is untouched — no underline, no restyle at all.
    /// This is what keeps a board with no tags rendering as it always did.
    #[test]
    fn untagged_rows_are_left_alone() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let out = underline(&theme, line("plain row"), &[], 26);
        for s in &out.spans {
            assert!(!s.style.add_modifier.contains(Modifier::UNDERLINED));
            assert_eq!(s.style.underline_color, None);
        }
    }

    /// Every cell is underlined, and the colour changes across the row — one
    /// equal segment per tag, so the proportions read the count back.
    #[test]
    fn the_row_splits_into_one_segment_per_tag() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for n in 1..=4usize {
            let out = underline(&theme, line(&"x".repeat(24)), &tags(n), 24);
            let mut seen: Vec<ratatui::style::Color> = Vec::new();
            let mut cells = 0usize;
            for s in &out.spans {
                assert!(
                    s.style.add_modifier.contains(Modifier::UNDERLINED),
                    "{n}: a cell missed the underline"
                );
                let c = s.style.underline_color.expect("tinted");
                if seen.last() != Some(&c) {
                    seen.push(c);
                }
                cells += s.content.width();
            }
            assert_eq!(cells, 24, "{n}: width changed");
            assert_eq!(seen.len(), n, "{n}: wrong number of segments");
            // Distinct tints, so the segments actually read apart.
            let mut uniq = seen.clone();
            uniq.dedup();
            assert_eq!(uniq.len(), n);
        }
    }

    /// A span straddling a segment boundary is cut, not rounded — otherwise a
    /// long title would swallow the whole row into one colour.
    #[test]
    fn a_long_span_is_cut_at_the_boundary() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        // One span covering the whole row, two tags: it must become two.
        let out = underline(&theme, line(&"y".repeat(20)), &tags(2), 20);
        assert!(out.spans.len() >= 2, "the span was not cut: {:?}", out.spans.len());
        let first = out.spans[0].content.width();
        assert_eq!(first, 10, "the cut landed off-centre");
    }

    /// No codepoint anywhere: the tags are an SGR attribute, which is the
    /// whole reason this is not `▀`/`▔`/`█` — those are inside the range the
    /// L1 law bans AND East Asian Width Ambiguous, the class that already
    /// cost a render bug here.
    #[test]
    fn tags_add_no_glyph_at_all() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let out = underline(&theme, line("Fix OSC-11 detection"), &tags(3), 20);
        for s in &out.spans {
            for ch in s.content.chars() {
                let cp = ch as u32;
                assert!(!(0x2500..=0x259F).contains(&cp), "drawn structure {ch:?}");
            }
        }
    }
}
