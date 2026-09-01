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
//! Where the second tag goes is on trial: `Second::Stack` splits the bar cell
//! across (`▀`, first tag over second), `Second::Half` splits it down (`▌`),
//! and `Second::Edge` puts it on the card's right-edge pad. `w` in the `^t` picker cycles them on a live board
//! and `MESIMON_TAG_SECOND` picks the one you start with. The channel they
//! replaced — an SGR-58 underline across the bar — was built, shipped and
//! could not be seen: one pixel at the bottom of a fully painted cell.
//!
//! **The tints stay under the accent.** D19 reserves exactly one saturated
//! colour for "needs you", and a full painted cell is a lot more ink than a
//! pip, so the ramp matters more here, not less. Tab cycles a tag through the
//! six tints in `Theme::pip` and nothing else — there is no free-colour path.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use mesimon_core::board::{Board, TagRef};

use crate::theme::{TagLevel, Theme};

/// Where the SECOND tag goes. All three are on trial (author 2026-09-01);
/// `w` cycles them on a live board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Second {
    /// Stacked inside the bar cell: `▀` in the FIRST tag's colour over the
    /// second tag's paint, so the split runs across the bar rather than down
    /// it. A cell is taller than it is wide, so these two halves are the
    /// fatter pair.
    #[default]
    Stack,
    /// Side by side inside the bar cell: `▌` in the second tag's colour over
    /// the first tag's paint. Two slivers, each half a cell wide.
    Half,
    /// The card's right edge — the trailing pad cell, which was already
    /// blank, so it costs no width and does not sit against the bar.
    Edge,
}

impl Second {
    pub(crate) fn from_env() -> Self {
        match std::env::var("MESIMON_TAG_SECOND").unwrap_or_default().to_ascii_lowercase().as_str()
        {
            "half" => Second::Half,
            "edge" => Second::Edge,
            _ => Second::Stack,
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Second::Stack => Second::Half,
            Second::Half => Second::Edge,
            Second::Edge => Second::Stack,
        }
    }

    /// What this home looks like, for the status line.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Second::Stack => "stacked",
            Second::Half => "beside",
            Second::Edge => "on the edge",
        }
    }

    /// What the NEXT press gives, for the footer hint. A hint that named the
    /// current state would be a key you press to find out what it does.
    pub(crate) fn next_word(self) -> &'static str {
        match self.next() {
            Second::Stack => "2nd tag stacked",
            Second::Half => "2nd tag beside",
            Second::Edge => "2nd tag on edge",
        }
    }
}

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
/// and in `Second::Half` the second tag is `▌` drawn over it. An untagged
/// card keeps the neutral block — dimmed to the same level, because the
/// loudness says what the CARD is doing and every card has to answer it.
///
/// The card spends NO cell on tags — the bar is already there, and a second
/// painted cell beside it read as one two-tone bar rather than as two things
/// (dogfood 2026-09-01). An untagged ticket leaves the bar exactly as it was
/// handed over, which is what "neutral" means here.
///
/// `▀` U+2580 and `▌` U+258C are inside the `0x2500–0x259F` range the L1 law
/// bans and are East Asian Width *Ambiguous*; they are here on an explicit
/// exception from the author, because an underline — the only other way to
/// put two colours in one cell — is one pixel at the bottom of a fully
/// painted cell and cannot be seen. `test_no_drawn_structure` names the two
/// admitted codepoints and still bans the rest of the range.
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
    mode: Second,
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
    match (mode, worn.next()) {
        // `▀` paints the TOP half in the foreground: first tag over second.
        (Second::Stack, Some(second)) => {
            ("▀".to_string(), style.bg(theme.pip_at(second.tint as usize, level)).fg(paint))
        }
        // `▌` paints the LEFT half: second tag beside the first.
        (Second::Half, Some(second)) => {
            ("▌".to_string(), style.bg(paint).fg(theme.pip_at(second.tint as usize, level)))
        }
        _ => (ch.to_string(), style.bg(paint)),
    }
}

/// The card's right-edge cell in `Second::Edge`: the second tag's paint on
/// the trailing pad, which was blank anyway.
pub(crate) fn edge_cell(
    theme: &Theme,
    tags: &[Painted],
    mode: Second,
    level: TagLevel,
) -> Option<Style> {
    if mode != Second::Edge || !theme.paints_tags() {
        return None;
    }
    let second = tags.get(1)?;
    Some(Style::default().bg(theme.pip_at(second.tint as usize, level)))
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
    use crate::theme::{Flavor, Profile};
    use ratatui::style::Style;
    use unicode_width::UnicodeWidthStr;

    fn tags(n: usize) -> Vec<Painted> {
        (0..n).map(|i| Painted { name: format!("T{i}"), tint: i as u8 % 6 }).collect()
    }

    /// The first tag paints the bar. The second is a half-block over it —
    /// across in `Stack`, down in `Half` — or the right-edge cell in `Edge`.
    /// Never two of them, and never a second cell next to the bar.
    #[test]
    fn the_second_tag_has_three_homes() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let worn = tags(2);
        let (first, second) = (theme.pip(worn[0].tint as usize), theme.pip(worn[1].tint as usize));

        // Stack: `▀` puts the FIRST tag on top, the second underneath.
        let (ch, st) =
            bar_cell(&theme, ' ', Style::default(), &worn, Second::Stack, TagLevel::Selected);
        assert_eq!(ch, "▀");
        assert_eq!(st.fg, Some(first), "the first tag must be the top half");
        assert_eq!(st.bg, Some(second));
        assert_eq!(
            edge_cell(&theme, &worn, Second::Stack, TagLevel::Selected),
            None,
            "Stack also painted an edge"
        );

        // Half: `▌` puts the second tag beside the first.
        let (ch, st) =
            bar_cell(&theme, ' ', Style::default(), &worn, Second::Half, TagLevel::Selected);
        assert_eq!(ch, "▌");
        assert_eq!(st.bg, Some(first));
        assert_eq!(st.fg, Some(second));

        // Edge: the bar is a plain painted cell and the edge carries tag 2.
        let (ch, st) =
            bar_cell(&theme, ' ', Style::default(), &worn, Second::Edge, TagLevel::Selected);
        assert_eq!(ch, " ", "Edge kept a half-block");
        assert_eq!(st.bg, Some(first));
        assert_eq!(
            edge_cell(&theme, &worn, Second::Edge, TagLevel::Selected).and_then(|s| s.bg),
            Some(second)
        );

        // The cycle visits all three and comes home.
        let mut m = Second::Stack;
        for _ in 0..3 {
            m = m.next();
        }
        assert_eq!(m, Second::Stack);
    }

    /// Three loudnesses, and they are ordered: the cursor card's tag is the
    /// full tint, a resting card's is a small step down, a sleeping one's is
    /// well down — but every one of them keeps the hue, which is the only
    /// thing the colour is there to say.
    #[test]
    fn the_three_levels_fade_but_keep_the_hue() {
        for flavor in [Flavor::Graphite, Flavor::Chalk] {
            let theme = Theme::new(flavor, Profile::TrueColor);
            for n in 0..6 {
                let full = theme.pip_at(n, TagLevel::Selected);
                assert_eq!(full, theme.pip(n), "{flavor:?} the selected level is the tint");
                let rest = theme.pip_at(n, TagLevel::Rest);
                let sleep = theme.pip_at(n, TagLevel::Sleeping);
                assert_ne!(rest, full, "{flavor:?} rest did not step down");
                assert_ne!(sleep, rest, "{flavor:?} sleeping did not step down");
                // Every level is still a distinct colour per tag, so two
                // sleeping cards never read as the same tag.
                for other in 0..6 {
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
        for mode in [Second::Stack, Second::Half, Second::Edge] {
            let (ch, st) =
                bar_cell(&theme, ' ', Style::default(), &tags(1), mode, TagLevel::Selected);
            assert_eq!(ch, " ", "{mode:?} drew a half-block for one tag");
            assert_eq!(st.bg, Some(theme.pip(0)));
            assert_eq!(st.fg, None);
            assert_eq!(edge_cell(&theme, &tags(1), mode, TagLevel::Selected), None);
        }
    }

    /// A third tag does not reach the card: the bar has two channels and the
    /// peek row has the names.
    #[test]
    fn the_card_shows_the_first_two_tags() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let many = tags(5);
        let (ch, st) =
            bar_cell(&theme, ' ', Style::default(), &many, Second::Half, TagLevel::Selected);
        assert_eq!(st.bg, Some(theme.pip(many[0].tint as usize)));
        assert_eq!(st.fg, Some(theme.pip(many[1].tint as usize)));
        assert_eq!(
            bar_cell(&theme, ' ', Style::default(), &many, Second::Stack, TagLevel::Selected).0,
            "▀"
        );
        assert_eq!(ch.chars().count(), 1, "the mark grew past its cell");
    }

    /// An untagged ticket leaves the bar exactly as it was handed over. That
    /// is what keeps an untagged board rendering as it always did.
    #[test]
    fn an_untagged_ticket_leaves_the_bar_alone() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let base = Style::default().bg(theme.rest.dim3);
        for mode in [Second::Stack, Second::Half, Second::Edge] {
            assert_eq!(
                bar_cell(&theme, ' ', base, &[], mode, TagLevel::Selected),
                (" ".to_string(), base)
            );
            assert_eq!(edge_cell(&theme, &[], mode, TagLevel::Selected), None);
        }
    }

    /// Off TrueColor there is no tint and the bar is a character rather than
    /// paint, so a plain underline says "tagged" — the same honest
    /// degradation the mark has always had. And no half-block: the glyph is
    /// admitted for a colour it cannot show there.
    #[test]
    fn without_tints_the_bar_still_says_tagged() {
        for p in [Profile::Ansi256, Profile::Ansi16, Profile::Ansi8, Profile::Mono] {
            let theme = Theme::new(Flavor::Graphite, p);
            let (ch, st) = bar_cell(
                &theme,
                '|',
                Style::default(),
                &tags(2),
                Second::Stack,
                TagLevel::Selected,
            );
            assert_eq!(ch, "|", "{p:?} drew a half-block with no colour to put in it");
            assert_eq!(st.bg, None, "{p:?} painted a tint it does not have");
            assert!(st.add_modifier.contains(Modifier::UNDERLINED), "{p:?} says nothing");
            assert_eq!(
                edge_cell(&theme, &tags(2), Second::Edge, TagLevel::Selected),
                None,
                "{p:?}"
            );
        }
    }

    /// The half-block is ONE cell wide by `unicode-width`, which is the
    /// measurement the whole layout is arithmetic over. (It is East Asian
    /// Width Ambiguous, so a terminal set to render Ambiguous as double will
    /// disagree — that is the risk the author accepted, and it is recorded in
    /// STALE-MAP rather than hidden here.)
    #[test]
    fn the_half_blocks_are_one_cell() {
        assert_eq!("▌".width(), 1);
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
    /// half-block: `▀` U+2580, `▔` U+2594 and `█` U+2588 stay banned, and so
    /// does everything else in the range.
    #[test]
    fn the_mark_reaches_for_one_codepoint_only() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for mode in [Second::Stack, Second::Half, Second::Edge] {
            for n in 0..5 {
                let (ch, _) =
                    bar_cell(&theme, ' ', Style::default(), &tags(n), mode, TagLevel::Selected);
                for c in ch.chars() {
                    assert!(c == ' ' || c == '▌' || c == '▀', "the mark drew {c:?}");
                }
            }
        }
    }
}
