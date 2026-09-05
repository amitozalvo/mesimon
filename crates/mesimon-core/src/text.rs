//! Scrubbing user and agent text at the boundaries.
//!
//! Every string that reaches a cell or leaves for another process passes
//! through one of two functions here, and the hazard lists live nowhere else.
//! Before this there were seven sanitizers with four different lists, and
//! only one of them stripped the bidi overrides that reverse what a card
//! shows.

/// `1 repo` / `19 repos`: a count with its noun, the English plural by `s`.
pub fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Format characters that draw nothing and reorder or hide what is around
/// them: zero-width spaces, joiners and marks (U+200B-200F), the bidi
/// overrides (U+202A-202E), the word joiner, invisible operators and bidi
/// isolates (U+2060-206F), variation selectors (U+FE00-FE0F), the BOM and
/// the combining keycap.
pub fn is_format_hazard(c: char) -> bool {
    matches!(
        c as u32,
        0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF | 0x20E3
    )
}

/// Box drawing and block elements (U+2500-259F): terminals disagree with
/// `unicode-width` on them, and a one-column disagreement strands a painted
/// cell past the card edge that the diff never repaints.
pub fn is_cell_hazard(c: char) -> bool {
    (0x2500..=0x259F).contains(&(c as u32))
}

/// Text that will be drawn in cells. Controls go (`\t` becomes a space;
/// `\n` survives only when `newlines`, for a zone that reads block
/// structure), and so do format and cell hazards.
pub fn scrub_cells(raw: &str, newlines: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\n' if newlines => out.push('\n'),
            '\t' => out.push(' '),
            c if c.is_control() || is_format_hazard(c) || is_cell_hazard(c) => {}
            c => out.push(c),
        }
    }
    out
}

/// Text leaving mesimon for another process (a prompt into an agent's box).
/// Controls and format hazards go; what a shape draws is the reader's
/// business, so box drawing survives. Only ever REMOVES: the result is a
/// subsequence of the input.
pub fn scrub_text(raw: &str) -> String {
    raw.chars().filter(|&c| !c.is_control() && !is_format_hazard(c)).collect()
}

/// The longest prefix of `s` within `max` bytes, never split mid-character.
pub fn cap_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Trimmed, or `None` when nothing is left — there is no such thing as a
/// blank tag or an empty prompt.
pub fn nonblank(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hazards_are_scrubbed_from_cells() {
        let dirty = "a\u{202e}b\u{200b}c\u{2588}d\te\x07f\ng";
        assert_eq!(scrub_cells(dirty, false), "abcd efg");
        assert_eq!(scrub_cells(dirty, true), "abcd ef\ng");
    }

    #[test]
    fn text_for_a_process_keeps_shapes_and_is_a_subsequence() {
        let dirty = "see \u{2502} this\u{202e}\r\n";
        let out = scrub_text(dirty);
        assert_eq!(out, "see \u{2502} this");
        let mut it = dirty.chars();
        assert!(out.chars().all(|c| it.any(|d| d == c)), "not a subsequence");
    }

    #[test]
    fn caps_land_on_char_boundaries() {
        assert_eq!(cap_bytes("héllo", 2), "h");
        assert_eq!(cap_bytes("héllo", 3), "hé");
        assert_eq!(cap_bytes("hi", 10), "hi");
        assert_eq!(nonblank("  "), None);
        assert_eq!(nonblank(" x "), Some("x".into()));
    }
}
