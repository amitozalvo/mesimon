//! One-cell tag bars and the named chips shown in an open card.
//!
//! Every bar on the board is one cell (`BAR`), however many tags a ticket
//! wears (T-1159). The bar is the first tag's colour down every row of the
//! card. A second tag ends it: the stripe's last row leaves its bottom
//! eighth on the row's surface, and the row under the card carries the
//! second tag as a half block in its first cell (`StripeEnd`). Tag colors do
//! not fade with selection; the neutral bar retains its selection ladder.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use mesimon_core::board::{Board, TagRef};

use crate::theme::{TagLevel, Theme};

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
    tags.iter().map(|t| Painted { name: t.name.clone(), tint: board.tint_of(t) }).collect()
}

/// Every tag bar's width: one cell, so a second tag on one ticket moves no
/// other card's title (T-1159; T-821 widened the whole board to two).
pub(crate) const BAR: usize = 1;

/// Paint the bar, one cell, on this row: the first tag whole, untagged the
/// neutral ladder, below TrueColor the underline. A second tag is not drawn
/// here; the card's last row and the row under it carry it (`StripeEnd`).
pub(crate) fn bar_spans(
    theme: &Theme,
    ch: char,
    style: Style,
    tags: &[Painted],
    level: TagLevel,
) -> Vec<Span<'static>> {
    let Some(first) = tags.first() else {
        let faded = style.bg.map(|c| theme.faded(c, level));
        let style = faded.map(|c| style.bg(c)).unwrap_or(style);
        return vec![Span::styled(ch.to_string().repeat(BAR), style)];
    };
    if !theme.paints_tags() {
        // Keep the existing underline fallback and its ASCII state ladder.
        return vec![Span::styled(
            ch.to_string().repeat(BAR),
            style.add_modifier(Modifier::UNDERLINED),
        )];
    }
    vec![Span::styled(" ".repeat(BAR), style.bg(theme.pip(first.tint as usize)))]
}

/// Where a two-tag stripe ends (T-1159, the author: "the 2nd tag color should
/// be a block below the first tag, only half and keep a small space between
/// them"). A cell holds two colours, and the first tag, the space and the
/// second tag are three, so the second tag takes the row under the stripe.
///
/// - `foot`: the stripe's last row, the first tag over its top seven eighths
///   and `▁` in the row's surface under it: the space.
/// - `hang`: the row under that, the second tag as `▀` on whatever that row
///   is, the blank row between cards or a field hung under the card.
///
/// `▁` and `▀` are deliberate block-glyph exceptions, one cell wide in every
/// font; ASCII profiles never draw them.
#[derive(Debug, Clone)]
pub(crate) struct StripeEnd {
    foot: Span<'static>,
    hang: Span<'static>,
}

impl StripeEnd {
    /// The end of this stripe, or `None` while it wears fewer than two tags
    /// or the profile has no tints to tell them apart. `surface` is the
    /// background the card's row gives its bar (`theme.bg` at rest).
    pub(crate) fn new(theme: &Theme, tags: &[Painted], surface: Option<Color>) -> Option<Self> {
        let [first, second, ..] = tags else { return None };
        if !theme.paints_tags() {
            return None;
        }
        let foot =
            Style::default().bg(theme.pip(first.tint as usize)).fg(surface.unwrap_or(Color::Reset));
        Some(StripeEnd {
            foot: Span::styled("▁", foot),
            hang: Span::styled("▀", Style::default().fg(theme.pip(second.tint as usize))),
        })
    }

    /// Close the stripe on this row, the card's last.
    pub(crate) fn foot(&self, line: &mut Line<'static>) {
        first_cell(line, self.foot.clone());
    }

    /// Hang the second tag in this row's first cell, the row under the card.
    pub(crate) fn hang(&self, line: &mut Line<'static>) {
        first_cell(line, self.hang.clone());
    }
}

/// Put `cell` in the line's first cell, splitting a wider first span; the
/// rest of the row stays where it was. A row that opens on a wide glyph is
/// left alone rather than widened or cut.
fn first_cell(line: &mut Line<'static>, cell: Span<'static>) {
    line.spans.retain(|s| !s.content.is_empty());
    let Some(head) = line.spans.first_mut() else {
        line.spans.push(cell);
        return;
    };
    let mut chars = head.content.chars();
    let Some(c) = chars.next() else { return };
    if c.to_string().width() != 1 {
        return;
    }
    let rest = chars.as_str().to_string();
    if rest.is_empty() {
        line.spans[0] = cell;
    } else {
        head.content = rest.into();
        line.spans.insert(0, cell);
    }
}

/// The tags spelled out as painted chips, for the one row the peek gives
/// them under the title.
///
/// The colour under a card says "this ticket is tagged, and with how many";
/// it cannot say *which* without the reader holding a legend in their head.
/// So the moment a card opens far enough to show a sentence, it can afford
/// to show the words. Several tags share `budget` equally rather than the
/// first ones eating it all — the row's job is to name every axis this
/// ticket sits on, and one name in full with the rest cut off would be the
/// wrong half of that.
pub(crate) fn chips(theme: &Theme, tags: &[Painted], budget: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    // How many chips still fit: as many as possible, and the rest drop off
    // the end rather than every name going illegible together. The mark
    // under the card is still carrying the count.
    let mut widths = Vec::new();
    for n in (1..=tags.len()).rev() {
        if let Some(w) = fit(&tags[..n], budget) {
            widths = w;
            break;
        }
    }
    for (t, w) in tags.iter().zip(widths) {
        let text = format!(" {} ", crate::text::truncate(&t.name, w));
        let style = if theme.paints_tags() {
            Style::default().bg(theme.pip(t.tint as usize)).fg(theme.tag_ink())
        } else {
            // No tint to tell them apart, so the name does it alone.
            Style::default().fg(theme.rest.dim1)
        };
        out.push(Span::styled(text, style));
        out.push(Span::raw(" "));
    }
    out.pop(); // the gap after the last chip
    out
}

/// Name widths for these tags inside `budget`, or `None` if they will not go.
///
/// A chip is " name " plus a cell of gap after it: everything but the name is
/// fixed, so the names are what has to give — and the LONGEST gives first,
/// one cell at a time. An equal split would cut "BUG" down to make room for a
/// "STAGING" that then gets cut anyway, and three tags would all arrive as
/// two letters and a tilde. Nothing shrinks below three cells, which is the
/// narrowest a truncated name still says anything ("STA~").
fn fit(tags: &[Painted], budget: usize) -> Option<Vec<usize>> {
    const CHROME: usize = 3; // " " + " " + the gap after
    const FLOOR: usize = 3;
    if tags.is_empty() || budget == 0 {
        return None;
    }
    let floor = |t: &Painted| t.name.width().min(FLOOR);
    let mut widths: Vec<usize> = tags.iter().map(|t| t.name.width().max(1)).collect();
    let spent = |w: &[usize]| w.iter().map(|n| n + CHROME).sum::<usize>().saturating_sub(1);
    while spent(&widths) > budget {
        let (i, _) = widths
            .iter()
            .enumerate()
            .filter(|(i, w)| **w > floor(&tags[*i]))
            .max_by_key(|(i, w)| (**w, usize::MAX - i))?;
        widths[i] -= 1;
    }
    Some(widths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::TagLevel;
    use crate::theme::{Flavor, Profile, PIPS};
    use ratatui::style::Style;
    use unicode_width::UnicodeWidthStr;

    fn tags(n: usize) -> Vec<Painted> {
        (0..n).map(|i| Painted { name: format!("T{i}"), tint: (i % PIPS) as u8 }).collect()
    }

    /// One cell, whatever the ticket wears: the first tag whole, untagged
    /// the faded neutral bar (T-1159).
    #[test]
    fn bars_keep_one_cell_and_their_colors_across_selection() {
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let base = Style::default().bg(theme.rest.dim3);
            for n in 0..=5 {
                let worn = tags(n);
                for level in [TagLevel::Selected, TagLevel::Rest] {
                    let spans = bar_spans(&theme, ' ', base, &worn, level);
                    assert_eq!(spans.len(), 1);
                    assert_eq!(spans[0].width(), BAR);
                    let want =
                        if n == 0 { theme.faded(theme.rest.dim3, level) } else { theme.pip(0) };
                    assert_eq!(spans[0].style.bg, Some(want), "{flavor:?} with {n} tags");
                }
            }
        }
    }

    /// The second tag ends the stripe: the last row keeps its bottom eighth
    /// on the row's own surface, and the row under it carries the second
    /// tag's top half. Two equal tints are still two blocks.
    #[test]
    fn a_second_tag_hangs_under_the_stripe_with_a_space() {
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            assert!(StripeEnd::new(&theme, &tags(1), theme.bg).is_none(), "one tag has no end");
            for worn in [tags(2), tags(4), vec![Painted { name: "BUG".into(), tint: 0 }; 2]] {
                for surface in [theme.bg, theme.selected_bg] {
                    let end = StripeEnd::new(&theme, &worn, surface).expect("two tags end");
                    let mut row = Line::from(vec![
                        bar_spans(&theme, ' ', Style::default(), &worn, TagLevel::Rest).remove(0),
                        Span::raw(" Fix the parser"),
                    ]);
                    end.foot(&mut row);
                    assert_eq!(row.width(), 1 + " Fix the parser".width());
                    assert_eq!(row.spans[0].content, "▁");
                    assert_eq!(row.spans[0].style.bg, Some(theme.pip(worn[0].tint as usize)));
                    assert_eq!(row.spans[0].style.fg, Some(surface.unwrap_or(Color::Reset)));
                    let mut gap = Line::default();
                    end.hang(&mut gap);
                    assert_eq!(gap.width(), 1);
                    assert_eq!(gap.spans[0].content, "▀");
                    assert_eq!(gap.spans[0].style.fg, Some(theme.pip(worn[1].tint as usize)));
                    assert_eq!(gap.spans[0].style.bg, None, "the row under keeps its own ground");
                }
            }
        }
    }

    /// A field hung under the card opens on a run of spaces: the half block
    /// takes the first and the row keeps its width.
    #[test]
    fn the_hang_takes_one_cell_of_a_wider_span() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let end = StripeEnd::new(&theme, &tags(2), theme.bg).expect("two tags");
        let mut row = Line::from(vec![Span::raw(""), Span::raw("  "), Span::raw("ask")]);
        end.hang(&mut row);
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "▀ ask");
        let mut wide = Line::from("界 x");
        end.hang(&mut wide);
        assert_eq!(wide.spans.len(), 1, "a wide first glyph is left alone");
    }

    #[test]
    fn without_tints_the_bar_keeps_its_ascii_fallback() {
        for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
            let theme = Theme::new(Flavor::Graphite, p);
            for n in 0..=3 {
                let worn = tags(n);
                let spans = bar_spans(&theme, '|', Style::default(), &worn, TagLevel::Selected);
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].content, "|");
                assert_eq!(spans[0].style.bg, None);
                assert_eq!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED), n > 0);
                assert!(StripeEnd::new(&theme, &worn, None).is_none(), "no tints, no end");
            }
        }
    }

    #[test]
    fn the_stripe_ends_glyphs_are_one_cell() {
        assert_eq!("▁".width(), 1);
        assert_eq!("▀".width(), 1);
    }

    /// Every tag gets named, even when the row is tight: the names share the
    /// budget instead of the first one eating it.
    #[test]
    fn chips_share_the_row_between_them() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let long = vec![
            Painted { name: "PRODUCTION".into(), tint: 0 },
            Painted { name: "REGRESSION".into(), tint: 1 },
            Painted { name: "auth".into(), tint: 2 },
        ];
        for budget in [40usize, 24, 18] {
            let spans = chips(&theme, &long, budget);
            let width: usize = spans.iter().map(|s| s.content.width()).sum();
            assert!(width <= budget, "budget {budget}: row is {width} wide");
            let named = spans.iter().filter(|s| s.style.bg.is_some()).count();
            assert_eq!(named, 3, "budget {budget}: a tag went unnamed");
        }
        // Roomy: the names come through whole.
        let text: String = chips(&theme, &long, 60).iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("PRODUCTION") && text.contains("auth"), "{text:?}");
        // Too tight for even one chip: nothing rather than a chip of nothing.
        assert!(chips(&theme, &long, 3).is_empty());
        // Tight enough to lose some: the ones that stay are still readable.
        let squeezed: String =
            chips(&theme, &long, 9).iter().map(|s| s.content.to_string()).collect();
        assert!(squeezed.width() <= 9 && squeezed.contains("PRODU"), "{squeezed:?}");
        assert!(chips(&theme, &[], 40).is_empty());
    }
}
