//! Tag rendering: the stripe down the left of a card, the names inside its
//! peek, and the tint ramp both draw from.
//!
//! A tag is **the card's own accent bar, painted**: neutral while nothing is
//! tagged, the tag's colour once something is. Not a glyph and not text, and
//! not a cell of its own — a second painted cell beside the bar read as one
//! two-tone bar rather than as two things.
//!
//! **One cell holds two tags**, because a cell has more than one colour
//! channel: the bar's *paint* is the first tag and the *underline stroke*
//! across it is the second. That is why the card spends no cell at all for
//! tags — and why there is no glyph here either. The obvious multi-colour
//! glyph, `▌` U+258C with a foreground and a background, is banned twice
//! over: inside the `0x2500–0x259F` range the L1 no-drawn-structure law bans
//! (`test_no_drawn_structure`), and East Asian Width *Ambiguous*, where the
//! terminal spends two cells and `unicode-width` counts one. A painted space
//! has neither problem, and it is the same trick the accent bar already uses.
//!
//! Three earlier marks are recorded in STALE-MAP because each failed in use:
//! a per-tag row of bands (cost a row per tag), a full-width underline (read
//! as a border, and at C* 7 read as nothing), and a painted row under the
//! card (read as an extra line below the ticket).
//!
//! **The second tag stacks**: `▀` U+2580 in the FIRST tag's colour over the
//! second tag's paint, so the split runs across the bar rather than down it —
//! a cell is taller than it is wide, so those are the fatter halves. Two
//! rivals were built and cut (author 2026-09-01): a `▌` split down the cell,
//! and the card's right-edge pad. So was the channel all three replaced — an
//! SGR-58 underline across the bar, which shipped and could not be seen: one
//! pixel at the bottom of a fully painted cell.
//!
//! **An open card has no need of the half-block.** One cell is all a resting
//! card can give, but a card with its peek out is five or six cells tall, and
//! at that height the two tags are FULL painted blocks instead: the first
//! takes the top ~70% of the stripe, the second the ~30% under it, in the
//! same order the half-block drew them (`stack_full`).
//!
//! **The tints stay under the accent.** D19 reserves exactly one saturated
//! colour for "needs you", and a full painted cell is a lot more ink than a
//! pip, so the ramp matters more here, not less. Tab cycles a tag through the
//! ten tints in `Theme::pip` and nothing else — there is no free-colour path.

use ratatui::style::{Modifier, Style};
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
    let mut out: Vec<Painted> =
        tags.iter().map(|t| Painted { name: t.name.clone(), tint: board.tint_of(t) }).collect();
    out.sort_by_key(|_| 0); // keep input order; tags are already group-sorted
    out
}

/// How many tags the card's bar can show: the cell's paint and the stroke
/// across it, and there is no third channel a colour could use without
/// spending a character. The rest are named in the peek row and on the
/// ticket page.
pub(crate) const TAGS_ON_CARD: usize = 2;

/// The card's accent bar, painted with the tags: the first tag is the cell,
/// and the second is the lower half of it (`▀` over the second's paint). An
/// untagged card keeps the neutral block — dimmed to the same level, because
/// the loudness says what the CARD is doing and every card has to answer it.
///
/// The card spends NO cell on tags — the bar is already there, and a second
/// painted cell beside it read as one two-tone bar rather than as two things
/// (dogfood 2026-09-01). An untagged ticket leaves the bar exactly as it was
/// handed over, which is what "neutral" means here.
///
/// `▀` U+2580 is inside the `0x2500–0x259F` range the L1 law bans and is East
/// Asian Width *Ambiguous*; it is here on an explicit exception from the
/// author, because an underline — the only other way to put two colours in
/// one cell — is one pixel at the bottom of a fully painted cell and cannot
/// be seen. `test_no_drawn_structure` names the ONE admitted codepoint and
/// still bans the rest of the range; a card tall enough to run the two tags
/// as full blocks does not reach for it at all (`stack_full`).
///
/// `level` is how loud the colour is allowed to be — the cursor card gets it
/// at full strength, a sleeping one gets it faded but still legible as a hue
/// (`Theme::pip_at`).
///
/// Off TrueColor there is no tint and the bar is a character rather than
/// paint, so a plain underline says "tagged" without saying which.
pub(crate) fn bar_cell(
    theme: &Theme,
    ch: char,
    style: Style,
    tags: &[Painted],
    level: TagLevel,
) -> (String, Style) {
    let mut worn = tags.iter().take(TAGS_ON_CARD);
    let Some(first) = worn.next() else {
        // Untagged, and still a block: the three loudnesses belong to the
        // CARD, so an untagged sleeping ticket dims exactly like a tagged one
        // and the cursor card's block is the brightest either way.
        let faded = style.bg.map(|c| theme.faded(c, level));
        return (ch.to_string(), faded.map(|c| style.bg(c)).unwrap_or(style));
    };
    if !theme.paints_tags() {
        return (ch.to_string(), style.add_modifier(Modifier::UNDERLINED));
    }
    let paint = theme.pip_at(first.tint as usize, level);
    match worn.next() {
        // `▀` paints the TOP half in the foreground: first tag over second.
        Some(second) => {
            ("▀".to_string(), style.bg(theme.pip_at(second.tint as usize, level)).fg(paint))
        }
        None => (ch.to_string(), style.bg(paint)),
    }
}

/// The fewest rows a stripe can be cut into two runs and still read as ~70/30
/// rather than as halves. Under it the half-block is the honest mark.
const SPLIT_MIN_ROWS: usize = 3;

/// How many rows at the BOTTOM of a stripe belong to the second tag: ~30% of
/// it, rounded, never fewer than one and never more than half. The first tag
/// is the ticket's first axis and has to stay the run the eye lands on.
/// `None` where the stripe is too short to say that at all.
pub(crate) fn second_rows(rows: usize) -> Option<usize> {
    (rows >= SPLIT_MIN_ROWS).then(|| ((rows * 3 + 5) / 10).clamp(1, rows / 2))
}

/// Repaint an open card's stripe: the two tags as full painted blocks, one
/// run over the other, instead of the two halves of a single cell.
///
/// `▀` puts both tags in one cell because a resting card has exactly one cell
/// to spend. A card with its peek out has five or six, and at that height the
/// half-block is a glyph doing what plain paint does better — and it is a
/// glyph held on an explicit exception to the L1 no-drawn-structure law, so
/// not reaching for it is worth something by itself. The order is the one the
/// half-block drew: first tag on top.
///
/// Below TrueColor there is no tint to run, and one tag has nothing to split
/// with; both leave the stripe exactly as `bar_cell` left it. Only the first
/// span of each line is touched — the bar is span 0 on every card row — so
/// this moves no text and changes no width.
pub(crate) fn stack_full(
    theme: &Theme,
    lines: &mut [Line<'static>],
    ch: char,
    base: Style,
    tags: &[Painted],
    level: TagLevel,
) {
    if !theme.paints_tags() {
        return;
    }
    let (Some(first), Some(second)) = (tags.first(), tags.get(1)) else {
        return;
    };
    let rows = lines.len();
    let Some(low) = second_rows(rows) else {
        return;
    };
    for (i, line) in lines.iter_mut().enumerate() {
        let tag = if i + low < rows { first } else { second };
        let Some(cell) = line.spans.first_mut() else {
            continue;
        };
        *cell = Span::styled(ch.to_string(), base.bg(theme.pip_at(tag.tint as usize, level)));
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

    /// The first tag paints the bar; the second is the lower half of that one
    /// cell. Never two cells, and never a second block beside the bar — both
    /// were built and both read as two things rather than one two-tone bar.
    #[test]
    fn the_second_tag_is_the_lower_half_of_the_cell() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let worn = tags(2);
        let (first, second) = (theme.pip(worn[0].tint as usize), theme.pip(worn[1].tint as usize));
        let (ch, st) = bar_cell(&theme, ' ', Style::default(), &worn, TagLevel::Selected);
        assert_eq!(ch, "▀");
        assert_eq!(st.fg, Some(first), "the first tag must be the top half");
        assert_eq!(st.bg, Some(second));
    }

    /// The stripe of an OPEN card is tall enough to give each tag its own run
    /// of full blocks, so the half-block is not reached for at all: ~70% of
    /// the rows to the first tag from the top, ~30% to the second under it.
    #[test]
    fn an_open_card_runs_the_two_tags_as_full_blocks() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let worn = tags(2);
        let (first, second) = (theme.pip(worn[0].tint as usize), theme.pip(worn[1].tint as usize));
        let base = Style::default().bg(theme.rest.dim3);
        let card = |rows: usize| -> Vec<Line<'static>> {
            (0..rows)
                .map(|_| {
                    Line::from(vec![
                        Span::styled("▀".to_string(), base.bg(second).fg(first)),
                        Span::raw("  body".to_string()),
                    ])
                })
                .collect()
        };

        let mut lines = card(6);
        stack_full(&theme, &mut lines, ' ', base, &worn, TagLevel::Selected);
        let stripe: Vec<(String, Option<_>)> =
            lines.iter().map(|l| (l.spans[0].content.to_string(), l.spans[0].style.bg)).collect();
        assert!(stripe.iter().all(|(c, _)| c == " "), "the split still drew a glyph: {stripe:?}");
        let tops = stripe.iter().filter(|(_, bg)| *bg == Some(first)).count();
        assert_eq!((tops, stripe.len() - tops), (4, 2), "six rows should run 4/2");
        assert_eq!(stripe[0].1, Some(first), "the first tag is the top run");
        assert_eq!(stripe[5].1, Some(second), "the second tag is the bottom run");
        // The rest of every row is untouched: this repaints a cell, it does
        // not re-lay a card.
        assert!(lines.iter().all(|l| l.spans[1].content == "  body"));

        // The ratio across the heights an open card actually reaches. The
        // first tag is never the smaller run, and the second is never absent.
        for rows in 3..=12 {
            let low = second_rows(rows).expect("tall enough");
            assert!(low >= 1 && low <= rows / 2, "{rows} rows split {low}");
        }
        assert_eq!(second_rows(2), None, "two rows are halves, not a 70/30 split");
        assert_eq!(second_rows(1), None);
        assert_eq!(second_rows(10), Some(3));

        // A resting card keeps the half-block: one row has nothing to run.
        let mut one = card(1);
        stack_full(&theme, &mut one, ' ', base, &worn, TagLevel::Selected);
        assert_eq!(one[0].spans[0].content, "▀", "a one-row stripe was split");
    }

    /// Nothing to split with, nothing to split into: one tag, no tags, and
    /// every profile below TrueColor leave an open card's stripe exactly as
    /// `bar_cell` left it.
    #[test]
    fn a_stripe_with_one_tag_or_no_paint_is_left_alone() {
        let base = Style::default();
        let card = || -> Vec<Line<'static>> {
            (0..6).map(|_| Line::from(vec![Span::styled("|".to_string(), base)])).collect()
        };
        let truecolor = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for worn in [tags(0), tags(1)] {
            let mut lines = card();
            stack_full(&truecolor, &mut lines, ' ', base, &worn, TagLevel::Selected);
            assert!(lines.iter().all(|l| l.spans[0].content == "|"), "{} tags", worn.len());
        }
        for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
            let theme = Theme::new(Flavor::Graphite, p);
            let mut lines = card();
            stack_full(&theme, &mut lines, ':', base, &tags(2), TagLevel::Selected);
            assert!(lines.iter().all(|l| l.spans[0].content == "|"), "{p:?} painted a tint");
        }
    }

    /// Three loudnesses, and they are ordered: the cursor card's tag is the
    /// full tint, a resting card's is a small step down, a sleeping one's is
    /// well down — but every one of them keeps the hue, which is the only
    /// thing the colour is there to say.
    #[test]
    fn the_three_levels_fade_but_keep_the_hue() {
        for flavor in Flavor::ALL {
            let theme = Theme::new(flavor, Profile::TrueColor);
            for n in 0..PIPS {
                let full = theme.pip_at(n, TagLevel::Selected);
                assert_eq!(full, theme.pip(n), "{flavor:?} the selected level is the tint");
                let rest = theme.pip_at(n, TagLevel::Rest);
                let sleep = theme.pip_at(n, TagLevel::Sleeping);
                assert_ne!(rest, full, "{flavor:?} rest did not step down");
                assert_ne!(sleep, rest, "{flavor:?} sleeping did not step down");
                // Every level is still a distinct colour per tag, so two
                // sleeping cards never read as the same tag.
                for other in 0..PIPS {
                    if other != n {
                        assert_ne!(sleep, theme.pip_at(other, TagLevel::Sleeping));
                    }
                }
            }
        }
        // Below TrueColor there is no ground to fade into, so the levels
        // collapse rather than inventing greys the palette does not have.
        let flat = Theme::new(Flavor::Graphite, Profile::Ansi256);
        assert_eq!(flat.pip_at(0, TagLevel::Sleeping), flat.pip(0));
    }

    /// One tag paints the bar and asks for nothing else — no half-block, no
    /// edge: both would read as a second tag that is not there.
    #[test]
    fn one_tag_is_paint_and_nothing_else() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let (ch, st) = bar_cell(&theme, ' ', Style::default(), &tags(1), TagLevel::Selected);
        assert_eq!(ch, " ", "one tag drew a half-block");
        assert_eq!(st.bg, Some(theme.pip(0)));
        assert_eq!(st.fg, None);
    }

    /// A third tag does not reach the card: the bar has two channels and the
    /// peek row has the names.
    #[test]
    fn the_card_shows_the_first_two_tags() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let many = tags(5);
        let (ch, st) = bar_cell(&theme, ' ', Style::default(), &many, TagLevel::Selected);
        assert_eq!(ch, "▀");
        assert_eq!(st.fg, Some(theme.pip(many[0].tint as usize)));
        assert_eq!(st.bg, Some(theme.pip(many[1].tint as usize)));
        assert_eq!(ch.chars().count(), 1, "the mark grew past its cell");
        // And the open card's runs are the same two, not five.
        let base = Style::default();
        let mut lines: Vec<Line<'static>> =
            (0..6).map(|_| Line::from(vec![Span::styled(" ".to_string(), base)])).collect();
        stack_full(&theme, &mut lines, ' ', base, &many, TagLevel::Selected);
        let hues: std::collections::BTreeSet<String> =
            lines.iter().map(|l| format!("{:?}", l.spans[0].style.bg)).collect();
        assert_eq!(hues.len(), 2, "the stripe ran more than two tags: {hues:?}");
    }

    /// An untagged ticket leaves the bar exactly as it was handed over. That
    /// is what keeps an untagged board rendering as it always did.
    #[test]
    fn an_untagged_ticket_leaves_the_bar_alone() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let base = Style::default().bg(theme.rest.dim3);
        assert_eq!(bar_cell(&theme, ' ', base, &[], TagLevel::Selected), (" ".to_string(), base));
    }

    /// Off TrueColor there is no tint and the bar is a character rather than
    /// paint, so a plain underline says "tagged" — the same honest
    /// degradation the mark has always had. And no half-block: the glyph is
    /// admitted for a colour it cannot show there.
    #[test]
    fn without_tints_the_bar_still_says_tagged() {
        for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
            let theme = Theme::new(Flavor::Graphite, p);
            let (ch, st) = bar_cell(&theme, '|', Style::default(), &tags(2), TagLevel::Selected);
            assert_eq!(ch, "|", "{p:?} drew a half-block with no colour to put in it");
            assert_eq!(st.bg, None, "{p:?} painted a tint it does not have");
            assert!(st.add_modifier.contains(Modifier::UNDERLINED), "{p:?} says nothing");
        }
    }

    /// The half-block is ONE cell wide by `unicode-width`, which is the
    /// measurement the whole layout is arithmetic over. (It is East Asian
    /// Width Ambiguous, so a terminal set to render Ambiguous as double will
    /// disagree — that is the risk the author accepted, and it is recorded in
    /// STALE-MAP rather than hidden here. An open card sidesteps it entirely:
    /// `stack_full` paints spaces.)
    #[test]
    fn the_half_block_is_one_cell() {
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

    /// The only codepoint the mark may ever reach for is the admitted
    /// half-block `▀` U+2580. `▌` U+258C went with the home that used it —
    /// an exception nothing spends is a ban — and `▔` U+2594, `█` U+2588 and
    /// the rest of the range were never admitted at all.
    #[test]
    fn the_mark_reaches_for_one_codepoint_only() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let base = Style::default();
        for n in 0..5 {
            let (ch, _) = bar_cell(&theme, ' ', base, &tags(n), TagLevel::Selected);
            for c in ch.chars() {
                assert!(c == ' ' || c == '▀', "the mark drew {c:?}");
            }
            // And the open card's stripe reaches for no glyph at all.
            let mut lines: Vec<Line<'static>> =
                (0..6).map(|_| Line::from(vec![Span::styled(" ".to_string(), base)])).collect();
            stack_full(&theme, &mut lines, ' ', base, &tags(n), TagLevel::Selected);
            for l in &lines {
                assert!(l.spans[0].content.chars().all(|c| c == ' '), "the split drew a glyph");
            }
        }
    }
}
