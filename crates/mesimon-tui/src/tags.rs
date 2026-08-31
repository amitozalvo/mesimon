//! Tag pips: the minimal at-rest indication of a ticket's tags (D18/D31b).
//!
//! A pip is the tag's **first letter, lowercase**, tinted `pip.N` where
//! `N = stable_hash(name) % PIPS`. Two properties make that the right shape:
//!
//! - The colour is a pure function of the name, so `auth` is the same colour
//!   on every machine and in every screenshot — never the tag's position in a
//!   list, or two people looking at the same board see different colours.
//! - The letter, not the tint, carries the identity. That is what lets the
//!   tint be abandoned wholesale below TrueColor (`Theme::pip`) and what keeps
//!   D31b's colour-only exception honest at the bottom of the ladder.
//!
//! Lowercase is deliberate and not cosmetic: uppercase is reserved board-wide
//! for "a human is required", and a tag never means that.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use mesimon_core::board::Tag;

use crate::theme::{Theme, PIPS};

/// How many pips render before the run collapses to `+N` (D31b). Three is the
/// count at which a run still reads as a set rather than a bar code.
pub(crate) const PIP_CAP: usize = 3;

/// FNV-1a over the name's bytes. Any stable hash would do; what matters is
/// that it is stable ACROSS MACHINES, which rules out `DefaultHasher`
/// (randomly seeded per process, so the same tag would change colour on every
/// restart).
pub(crate) fn tint_index(name: &str) -> usize {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (h % PIPS as u64) as usize
}

/// The pip character for a tag: its first letter, lowercased, or `#` for a
/// name that starts with something unalphabetic.
///
/// Restricted to ASCII on purpose. A pip occupies exactly one cell, and the
/// only way to guarantee that is to refuse anything whose width the terminal
/// and `unicode-width` might disagree about — the disagreement strands a
/// `selected_bg` cell past the card edge that the diff never repaints.
pub(crate) fn pip_char(name: &str) -> char {
    name.chars().find(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).unwrap_or('#')
}

/// Cells the pip run will occupy, including its leading space. Zero when the
/// ticket has no tags — the zone collapses, which is what keeps an untagged
/// board byte-identical to the one before this feature.
pub(crate) fn pip_cells(tags: &[Tag]) -> usize {
    if tags.is_empty() {
        return 0;
    }
    let shown = tags.len().min(PIP_CAP);
    let extra = tags.len().saturating_sub(PIP_CAP);
    // sp + one cell per pip + "+N" when the run is capped.
    1 + shown + if extra > 0 { 1 + digits(extra) } else { 0 }
}

fn digits(n: usize) -> usize {
    if n >= 10 {
        2
    } else {
        1
    }
}

/// The pip run: `" b d p"` style, one span per pip so each keeps its tint.
///
/// `quiet` is the style a demoted row wants (a trail ghost, or the inverted
/// needs-you row): there, the pips give up their tint entirely rather than
/// fight the row for attention. Colour-only encoding is only defensible while
/// the tags are ambient, and a card that is shouting is not the moment.
pub(crate) fn pip_spans(theme: &Theme, tags: &[Tag], quiet: Option<Style>) -> Vec<Span<'static>> {
    if tags.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::raw(" ".to_string())];
    for tag in tags.iter().take(PIP_CAP) {
        let style = quiet.unwrap_or_else(|| Style::default().fg(theme.pip(tint_index(&tag.name))));
        spans.push(Span::styled(pip_char(&tag.name).to_string(), style));
    }
    let extra = tags.len().saturating_sub(PIP_CAP);
    if extra > 0 {
        let style = quiet.unwrap_or_else(|| theme.dim2());
        spans.push(Span::styled(format!("+{extra}"), style));
    }
    spans
}

/// The spelled-out form, one span per tag: `#BUG #STAGING`. This is what the
/// peek toggle reveals, and it is load-bearing rather than decorative —
/// D31b's grant of colour-only encoding to tags holds only while "the full
/// names appear on selection one keystroke away" stays true.
pub(crate) fn name_spans(theme: &Theme, tags: &[Tag], dim: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".to_string()));
        }
        spans.push(Span::styled("#".to_string(), Style::default().fg(dim)));
        spans.push(Span::styled(
            tag.name.clone(),
            Style::default().fg(theme.pip(tint_index(&tag.name))),
        ));
    }
    spans
}

/// Display width of [`name_spans`].
pub(crate) fn names_width(tags: &[Tag]) -> usize {
    use unicode_width::UnicodeWidthStr;
    let mut w = 0;
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            w += 1;
        }
        w += 1 + tag.name.width();
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Flavor, Profile};

    fn tag(name: &str, group: u8) -> Tag {
        Tag { name: name.into(), group }
    }

    /// The tint is a pure function of the NAME, so the same tag is the same
    /// colour everywhere. A per-process hash would repaint the board on every
    /// restart.
    #[test]
    fn tint_is_stable_and_name_derived() {
        assert_eq!(tint_index("BUG"), tint_index("BUG"));
        assert_ne!(tint_index("BUG"), tint_index("REGR"));
        // Pinned: if the hash changes, every board changes colour.
        assert!(tint_index("BUG") < PIPS);
        assert!(tint_index("") < PIPS);
    }

    /// A pip is exactly one cell, whatever the name. Anything wider strands a
    /// selected-surface cell past the card edge.
    #[test]
    fn a_pip_is_always_one_ascii_cell() {
        for name in ["BUG", "regr", "9lives", "  spaced", "→arrow", "日本語", "!", ""] {
            let c = pip_char(name);
            assert!(c.is_ascii(), "{name:?} gave a non-ascii pip {c:?}");
            assert!(!c.is_ascii_uppercase(), "{name:?} gave an uppercase pip");
        }
        assert_eq!(pip_char("BUG"), 'b');
        assert_eq!(pip_char("→arrow"), 'a');
        // Uppercase is reserved for "a human is required"; a tag is never that.
        assert_eq!(pip_char("日本語"), '#');
        assert_eq!(pip_char(""), '#');
    }

    /// The run caps at 3 then `+N`, and the advertised width matches what is
    /// actually rendered — the card's title budget is computed from it.
    #[test]
    fn pip_cells_match_what_renders() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for n in 0..8usize {
            let tags: Vec<Tag> = (0..n).map(|i| tag(&format!("t{i}"), i as u8 + 1)).collect();
            let spans = pip_spans(&theme, &tags, None);
            let rendered: usize =
                spans.iter().map(|s| unicode_width::UnicodeWidthStr::width(&*s.content)).sum();
            assert_eq!(rendered, pip_cells(&tags), "{n} tags");
        }
        // An untagged ticket costs nothing — the zone collapses.
        assert_eq!(pip_cells(&[]), 0);
        assert!(pip_spans(&theme, &[], None).is_empty());
        // Four tags: sp + 3 pips + "+1".
        let four: Vec<Tag> = (0..4).map(|i| tag(&format!("t{i}"), i as u8 + 1)).collect();
        assert_eq!(pip_cells(&four), 1 + 3 + 2);
    }

    /// A demoted row gives up the tint rather than fighting for attention.
    #[test]
    fn quiet_rows_drop_the_tint() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        let tags = [tag("BUG", 1)];
        let quiet = theme.dim3();
        for s in pip_spans(&theme, &tags, Some(quiet)) {
            if s.content.trim().is_empty() {
                continue;
            }
            assert_eq!(s.style, quiet);
        }
    }

    #[test]
    fn names_width_matches_the_spans() {
        let theme = Theme::new(Flavor::Graphite, Profile::TrueColor);
        for tags in [vec![], vec![tag("BUG", 1)], vec![tag("BUG", 1), tag("STAGING", 2)]] {
            let spans = name_spans(&theme, &tags, theme.rest.dim2);
            let rendered: usize =
                spans.iter().map(|s| unicode_width::UnicodeWidthStr::width(&*s.content)).sum();
            assert_eq!(rendered, names_width(&tags));
        }
    }
}
