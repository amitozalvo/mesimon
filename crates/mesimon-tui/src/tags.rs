//! Fixed-width tag bars and the named chips shown in an open card.
//!
//! Every bar reserves two cells. One tag fills both; two tags use `▉▉`,
//! each glyph leaving a one-eighth-cell gap on the row's background.
//! The shape and tag order stay the same when a card opens. Tag colors do
//! not fade with selection; the neutral bar retains its selection ladder.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
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
    let mut out: Vec<Painted> =
        tags.iter().map(|t| Painted { name: t.name.clone(), tint: board.tint_of(t) }).collect();
    out.sort_by_key(|_| 0); // keep input order; tags are already group-sorted
    out
}

/// The same footprint for zero, one, or two tags, in every color profile.
pub(crate) const BAR_WIDTH: usize = 2;
pub(crate) const TAGS_ON_CARD: usize = 2;

/// Paint the bar on this row's surface. Explicitly replace the neutral bar's
/// background for two tags: leaving it behind the glyph would fill the gap.
/// `▉` is an intentional exception to the block-glyph restriction. Like the
/// former half-block it is width-ambiguous; ASCII profiles never use it.
pub(crate) fn bar_spans(
    theme: &Theme,
    ch: char,
    style: Style,
    tags: &[Painted],
    level: TagLevel,
    surface: Option<Color>,
) -> Vec<Span<'static>> {
    let Some(first) = tags.first() else {
        let faded = style.bg.map(|c| theme.faded(c, level));
        let style = faded.map(|c| style.bg(c)).unwrap_or(style);
        return vec![Span::styled(ch.to_string().repeat(BAR_WIDTH), style)];
    };
    if !theme.paints_tags() {
        // Keep the existing underline fallback and its ASCII state ladder.
        return vec![Span::styled(
            ch.to_string().repeat(BAR_WIDTH),
            style.add_modifier(Modifier::UNDERLINED),
        )];
    }
    if tags.len() == 1 {
        return vec![Span::styled("  ", style.bg(theme.pip(first.tint as usize)))];
    }
    tags.iter()
        .take(TAGS_ON_CARD)
        .map(|tag| {
            Span::styled(
                "▉",
                style.bg(surface.unwrap_or(Color::Reset)).fg(theme.pip(tag.tint as usize)),
            )
        })
        .collect()
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

    #[test]
    fn bars_keep_their_width_and_colors_across_selection() {
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let base = Style::default().bg(theme.rest.dim3);
            for n in 0..=5 {
                let worn = tags(n);
                for level in [TagLevel::Selected, TagLevel::Rest] {
                    let spans = bar_spans(&theme, ' ', base, &worn, level, theme.bg);
                    assert_eq!(spans.iter().map(Span::width).sum::<usize>(), BAR_WIDTH);
                    match n {
                        0 => {
                            assert_eq!(spans[0].style.bg, Some(theme.faded(theme.rest.dim3, level)))
                        }
                        1 => {
                            assert_eq!(spans[0].content, "  ");
                            assert_eq!(spans[0].style.bg, Some(theme.pip(0)));
                        }
                        _ => {
                            assert_eq!(spans.len(), 2);
                            for (i, span) in spans.iter().enumerate() {
                                assert_eq!(span.content, "▉");
                                assert_eq!(span.style.fg, Some(theme.pip(i)));
                                assert_eq!(span.style.bg, theme.bg);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn two_equal_tints_still_have_a_gap_on_each_surface() {
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            let worn = vec![Painted { name: "BUG".into(), tint: 0 }; 2];
            for surface in [theme.bg, theme.selected_bg, Some(theme.attn), theme.delete_row().bg] {
                let spans = bar_spans(
                    &theme,
                    ' ',
                    Style::default().bg(theme.rest.dim3),
                    &worn,
                    TagLevel::Selected,
                    surface,
                );
                for span in spans {
                    assert_eq!(span.content, "▉");
                    assert_eq!(span.style.bg, Some(surface.unwrap_or(Color::Reset)));
                    assert_eq!(span.style.fg, Some(theme.pip(0)));
                }
            }
        }
    }

    #[test]
    fn without_tints_the_bar_keeps_its_ascii_fallback() {
        for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
            let theme = Theme::new(Flavor::Graphite, p);
            for n in 0..=3 {
                let spans =
                    bar_spans(&theme, '|', Style::default(), &tags(n), TagLevel::Selected, None);
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].content, "||");
                assert_eq!(spans[0].style.bg, None);
                assert_eq!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED), n > 0);
            }
        }
    }

    #[test]
    fn the_seven_eighths_block_is_one_cell() {
        assert_eq!("▉".width(), 1);
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
