//! The summary a note carries (T-696).
//!
//! A ticket's notes are markdown, and a person reading the board wants one
//! thing from them without opening the ticket: how far along it is, and
//! what is left. A note section headed `Summary` is that answer, written on
//! purpose — an agent's working checklist under any other heading stays the
//! note's own, and only what it puts under `Summary` reaches the card. Inside
//! the section a GFM task item (`- [ ] …`, `- [x] …`) is a box the card
//! counts and the SUMMARY dialog ticks, and every other line is the note's
//! alone: the summary is its boxes (T-829, the author: "remove non checklist
//! items from summary, i never liked it anyway").
//!
//! `extract` reads a body and lists the rows in document order; `toggle`
//! flips one box in the body's text and returns the body to write back.
//! Pure: no filesystem, no board, no wire. Derived on every read and never
//! persisted, for `links.rs`'s reason: the recogniser will grow, and derived
//! data on disk drifts from its deriver. The drawing parser in
//! `tui/src/rich.rs` reads the same boxes to paint a note; the two agree on
//! `[ ] ` / `[x] ` at the head of a list item and share nothing else.

/// One row of a note's summary section, in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The body's line the row came from, zero-based. What `toggle` takes
    /// and what the ticket page scrolls to.
    pub line: usize,
    /// The row's words: the item past its box, trimmed.
    pub text: String,
    /// Is the box ticked?
    pub done: bool,
}

/// How many boxes a list of rows holds, and how many are ticked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Count {
    pub done: usize,
    pub total: usize,
}

impl Count {
    pub fn of(rows: &[Row]) -> Count {
        Count::of_marks(rows.iter().map(|r| r.done))
    }

    /// The count of boxes by whether each is ticked.
    pub fn of_marks(marks: impl IntoIterator<Item = bool>) -> Count {
        marks.into_iter().fold(Count::default(), |c, d| Count {
            done: c.done + usize::from(d),
            total: c.total + 1,
        })
    }

    /// The boxes not ticked yet.
    pub fn open(self) -> usize {
        self.total - self.done
    }

    /// Nothing, some or every box ticked — the fold row's own box.
    pub fn state(self) -> Option<bool> {
        if self.total == 0 {
            None
        } else if self.done == 0 {
            Some(false)
        } else if self.done == self.total {
            Some(true)
        } else {
            None
        }
    }
}

/// A markdown heading: its level and its text, `#`s and trailing `#`s off.
/// Up to three leading blanks, as CommonMark allows.
fn heading(line: &str) -> Option<(usize, &str)> {
    let s = line
        .strip_prefix("   ")
        .or_else(|| line.strip_prefix("  "))
        .or_else(|| line.strip_prefix(' '))
        .unwrap_or(line);
    let level = s.bytes().take_while(|b| *b == b'#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &s[level..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((level, text))
}

/// Does this heading open a summary section?
fn opens(text: &str) -> bool {
    text.eq_ignore_ascii_case("summary")
}

/// A fenced code block's fence — three or more backticks or tildes.
fn fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// A list item's marker — `-`, `*`, `+`, or `1.` / `1)` — and the text after it.
fn item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix(['-', '*', '+']) {
        return rest.strip_prefix(' ');
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = &t[digits..];
    rest.strip_prefix(['.', ')'])?.strip_prefix(' ')
}

/// `[ ] rest` / `[x] rest` at the head of an item → (ticked?, rest).
/// The same three spellings `rich.rs::task_box` reads, so a box the card
/// counts is a box the note draws.
fn task_box(text: &str) -> Option<(bool, &str)> {
    for (mark, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
        if let Some(rest) = text.strip_prefix(mark) {
            return Some((done, rest));
        }
    }
    // A box with nothing after it is still a box.
    for (mark, done) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if text == mark {
            return Some((done, ""));
        }
    }
    None
}

/// Every box of every `Summary` section in `body`, document order. A
/// section runs from its heading to the next heading of the same or a
/// higher level, or the end; a fenced block inside it is skipped whole, so
/// a box in sample code is not progress and a `#` in a shell snippet does
/// not close the section.
pub fn extract(body: &str) -> Vec<Row> {
    section_lines(body)
        .into_iter()
        .filter_map(|(line_no, line)| {
            let (done, text) = item(line).and_then(task_box)?;
            Some(Row { line: line_no, text: text.trim().to_string(), done })
        })
        .collect()
}

/// Every pull request a `Summary` section names (T-859): the PR URLs on
/// its lines, in document order, each once. A line there is the ticket's
/// own word on its work, so an address there is a fact the forge reads,
/// where a URL elsewhere in a note is a mention. A box or a plain line
/// alike; the box, if it is one, stays what the card counts.
pub fn pr_urls(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, line) in section_lines(body) {
        for url in crate::forge::pr_urls(line) {
            if !out.iter().any(|u| crate::forge::same_url(u, &url)) {
                out.push(url);
            }
        }
    }
    out
}

/// The lines inside every `Summary` section, numbered from the body's
/// first line: headings and fenced blocks left out.
fn section_lines(body: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut open: Option<usize> = None;
    let mut fenced = false;
    for (line_no, line) in body.lines().enumerate() {
        if fence(line) {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some((level, text)) = heading(line) {
            match open {
                Some(l) if level <= l => open = None,
                Some(_) => continue,
                None => {}
            }
            if open.is_none() && opens(text) {
                open = Some(level);
            }
            continue;
        }
        if open.is_some() {
            out.push((line_no, line));
        }
    }
    out
}

/// `body` with the box on `line` flipped — `[ ]` to `[x]`, `[x]` or `[X]`
/// to `[ ]`. `None` when that line holds no box at the head of an item: the
/// caller's row is older than the body, and nothing is written. Every other
/// byte of the body is kept as it was, line endings included.
pub fn toggle(body: &str, line: usize) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut flipped = false;
    for (n, piece) in body.split_inclusive('\n').enumerate() {
        if n != line {
            out.push_str(piece);
            continue;
        }
        let text = piece.trim_end_matches(['\n', '\r']);
        let ending = &piece[text.len()..];
        let rest = item(text)?;
        let head = text.len() - rest.len();
        let (done, _) = task_box(rest)?;
        out.push_str(&text[..head]);
        out.push_str(if done { "[ ]" } else { "[x]" });
        out.push_str(&rest[3..]);
        out.push_str(ending);
        flipped = true;
    }
    flipped.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-859: a pull request's address under `Summary` is the ticket's
    /// word, on a plain line or in a box, once; elsewhere it is a mention.
    #[test]
    fn the_summary_names_its_pull_requests_and_the_rest_of_the_note_does_not() {
        let body = "# Review the auth MR\n\n\
            Prerequisite: https://gitlab.com/g/p/-/merge_requests/3\n\n\
            ## Summary\n\
            - [x] reviewed https://gitlab.com/g/p/-/merge_requests/7\n\
            - MR: https://gitlab.com/g/p/-/merge_requests/7/\n\
            - https://github.com/o/r/pull/42#issuecomment-1\n\
            ```\n\
            https://github.com/o/r/pull/99\n\
            ```\n\n\
            ## Notes\n\
            see https://github.com/o/r/pull/100\n";
        assert_eq!(
            pr_urls(body),
            ["https://gitlab.com/g/p/-/merge_requests/7", "https://github.com/o/r/pull/42"]
        );
        assert_eq!(extract(body).len(), 1, "the box stays what the card counts");
        assert!(pr_urls("no summary here https://github.com/o/r/pull/1").is_empty());
    }

    const BODY: &str = "# Fix the login redirect\n\n\
        Return path is dropped after SSO.\n\n\
        ## Summary\n\
        redirect fixed; e2e next\n\
        - [x] rewrite the redirect\n\
        - [ ] add the e2e\n\
        * [X] unit test\n\
        1. [ ] release note\n\
        - a bullet with no box\n\n\
        ## Notes for later\n\
        - [ ] not progress\n";

    #[test]
    fn reads_the_summary_section_only() {
        let rows = extract(BODY);
        let texts: Vec<(&str, bool)> = rows.iter().map(|r| (r.text.as_str(), r.done)).collect();
        // A plain line and a bullet with no box are the note's, not the
        // summary's (T-829).
        assert_eq!(
            texts,
            vec![
                ("rewrite the redirect", true),
                ("add the e2e", false),
                ("unit test", true),
                ("release note", false),
            ]
        );
        assert_eq!(rows[0].line, 6);
        assert_eq!(Count::of(&rows), Count { done: 2, total: 4 });
        assert_eq!(Count::of(&rows).open(), 2);
    }

    #[test]
    fn the_heading_is_the_gate() {
        assert!(extract("- [ ] a box with no heading\n").is_empty());
        assert!(extract("## Summaries\n- [ ] close but no\n").is_empty());
        // Any level, any case, trailing hashes, a little indent.
        assert_eq!(extract("# SUMMARY #\n- [ ] a\n").len(), 1);
        assert!(extract("  ### summary\n- [x] a\n")[0].done);
        // `#hashtag` is not a heading.
        assert!(extract("#summary\n- [ ] a\n").is_empty());
    }

    #[test]
    fn a_deeper_heading_stays_inside_and_a_level_closes() {
        let body = "## Summary\n- [ ] a\n### detail\n- [ ] b\n## Next\n- [ ] c\n# Top\n- [ ] d\n";
        let rows = extract(body);
        let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b"]);
        // Two sections both count.
        let twice = "## Summary\n- [ ] a\n## Other\n- [ ] x\n## Summary\n- [ ] b\n";
        assert_eq!(extract(twice).len(), 2);
    }

    #[test]
    fn a_fence_is_skipped_whole() {
        let body = "## Summary\n- [ ] a\n```sh\n- [ ] not a box\n# not a heading\n```\n- [ ] b\n";
        let texts: Vec<String> = extract(body).into_iter().map(|r| r.text).collect();
        assert_eq!(texts, vec!["a", "b"]);
    }

    #[test]
    fn an_empty_box_is_a_box() {
        let rows = extract("## Summary\n- [ ]\n- [x]\n");
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].done);
        assert!(rows[1].done);
        assert_eq!(rows[0].text, "");
    }

    #[test]
    fn count_state_is_the_fold_rows_box() {
        let none = Count { done: 0, total: 3 };
        let some = Count { done: 1, total: 3 };
        let all = Count { done: 3, total: 3 };
        let empty = Count { done: 0, total: 0 };
        assert_eq!(none.state(), Some(false));
        assert_eq!(some.state(), None);
        assert_eq!(all.state(), Some(true));
        assert_eq!(empty.state(), None);
    }

    #[test]
    fn toggle_flips_one_box_and_nothing_else() {
        let rows = extract(BODY);
        let flipped = toggle(BODY, rows[1].line).expect("a box");
        assert!(flipped.contains("- [x] add the e2e\n"));
        assert_eq!(flipped.len(), BODY.len());
        let back = toggle(&flipped, rows[1].line).expect("a box");
        assert_eq!(back, BODY);
        // `[X]` unticks to `[ ]`; the marker and indent stay.
        let un = toggle(BODY, rows[2].line).expect("a box");
        assert!(un.contains("* [ ] unit test\n"));
        // CRLF endings survive.
        let crlf = "## Summary\r\n- [ ] a\r\n- [ ] b\r\n";
        assert_eq!(toggle(crlf, 1).as_deref(), Some("## Summary\r\n- [x] a\r\n- [ ] b\r\n"));
        // The last line with no newline.
        assert_eq!(toggle("## Summary\n- [ ] a", 1).as_deref(), Some("## Summary\n- [x] a"));
    }

    #[test]
    fn toggle_refuses_a_line_with_no_box() {
        assert_eq!(toggle(BODY, 0), None);
        assert_eq!(toggle(BODY, 5), None, "a plain row");
        assert_eq!(toggle(BODY, 400), None, "past the end");
        assert_eq!(toggle("- [ ]x\n", 0), None, "no blank after the box");
    }
}
