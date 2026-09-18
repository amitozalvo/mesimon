//! The links a note carries (T-256).
//!
//! A ticket's notes are markdown, and what a person wants to reach next is
//! usually in them: the Jira issue the ticket mirrors, the parent ticket, the
//! file the agent named in its plan. `extract` reads a body and says what in
//! it can be followed — a URL, a ticket key, a path — so the TUI can list them
//! (`^k`) and open one. Pure: no filesystem, no board. A path is only a
//! CANDIDATE here (it may not exist, it may be `and/or`); the caller checks
//! the disk, and a ticket key is resolved against the live board at the
//! same time. Derived on every ask and never persisted: the recogniser will
//! grow, and derived data on disk drifts from its deriver.
//!
//! This is the second markdown-link parser in the tree — `tui/src/rich.rs`
//! has one for DRAWING a body, and its walk returns painted spans. The two
//! agree on `[label](target)` and nothing else needs to be shared.

use crate::board::KEY_PREFIX;

/// What a link points at, as written. `Path` is a candidate until the caller
/// finds the file; `Ticket` is a key until the caller finds the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Attachment(ulid::Ulid),
    /// `http://` or `https://`, as written (trailing prose punctuation off).
    Url(String),
    /// `T-12`: a ticket's short key.
    Ticket(String),
    /// A path as written, with a `:LINE` (or `:LINE:COL`) suffix split off.
    Path {
        path: String,
        line: Option<u32>,
    },
}

/// One link in document order: the markdown label when there was one, and
/// the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub label: Option<String>,
    pub target: Found,
}

/// Git's rule for "is this text": no NUL in the first 8 KiB. An empty file
/// is text.
pub fn looks_text(head: &[u8]) -> bool {
    !head.iter().take(8192).any(|b| *b == 0)
}

/// Every link in `body`, document order, deduplicated by target (the first
/// occurrence keeps its label). One pass: a `[label](target)` is one link,
/// never a label and a bare target, and every other whitespace-delimited word
/// is read on its own.
pub fn extract(body: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let mut push = |link: Link| {
        if !out.iter().any(|l| l.target == link.target) {
            out.push(link);
        }
    };
    let chars: Vec<char> = body.chars().collect();
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' || (c == '!' && chars.get(i + 1) == Some(&'[')) {
            let open = if c == '!' { i + 1 } else { i };
            if let Some((link, end)) = markdown_link(&chars, open) {
                if let Some(target) = classify(&word) {
                    push(Link { label: None, target });
                }
                word.clear();
                if let Some(link) = link {
                    push(link);
                }
                i = end;
                continue;
            }
        }
        if c.is_whitespace() {
            if let Some(target) = classify(&word) {
                push(Link { label: None, target });
            }
            word.clear();
        } else {
            word.push(c);
        }
        i += 1;
    }
    if let Some(target) = classify(&word) {
        push(Link { label: None, target });
    }
    out
}

/// `[label](target)` with `chars[open] == '['`: the link, if the target
/// classifies, and the index just past the closing `)`. A `(target "title")`
/// keeps the target only; an `<angle>` target loses its brackets. No nesting
/// and no escapes — rich.rs's rule, and enough for a note.
fn markdown_link(chars: &[char], open: usize) -> Option<(Option<Link>, usize)> {
    let close = (open + 1..chars.len()).find(|&j| chars[j] == ']' || chars[j] == '\n')?;
    if chars[close] != ']' || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = (close + 2..chars.len()).find(|&j| chars[j] == ')' || chars[j] == '\n')?;
    if chars[end] != ')' {
        return None;
    }
    let label: String = chars[open + 1..close].iter().collect();
    let raw: String = chars[close + 2..end].iter().collect();
    let target = raw.split_whitespace().next().unwrap_or("");
    let target = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(target);
    let label = label.trim();
    let link = classify(target)
        .map(|target| Link { label: (!label.is_empty()).then(|| label.to_string()), target });
    Some((link, end + 1))
}

const LEAD: &[char] = &['(', '[', '<', '{', '"', '\'', '`', '*', '_'];
const TRAIL: &[char] =
    &[')', ']', '>', '}', '"', '\'', '`', '*', '_', ',', '.', ';', ':', '!', '?'];

/// One whitespace-delimited word of prose, or one markdown target: what it
/// points at, if anything.
fn classify(word: &str) -> Option<Found> {
    if let Some(id) = crate::attachment::parse_target(word) {
        return Some(Found::Attachment(id));
    }
    let word = word.trim_start_matches(LEAD);
    if word.is_empty() {
        return None;
    }
    let lower = word.to_ascii_lowercase();
    // A URL anywhere in the word: `[x](https://…` with its `)` on the next
    // line, `url=https://…`. The scheme is unambiguous, so what precedes it
    // is prose.
    if let Some(at) = lower.find("http://").or_else(|| lower.find("https://")) {
        let url = trim_url(&word[at..]);
        return (url.len() > "https://".len()).then(|| Found::Url(url.to_string()));
    }
    let word = word.trim_end_matches(TRAIL);
    if let Some(rest) = lower.strip_prefix("file://") {
        let path = &word[word.len() - rest.len()..];
        return path_candidate(path.trim_end_matches(TRAIL));
    }
    if let Some(n) = word.strip_prefix(KEY_PREFIX) {
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
            return Some(Found::Ticket(word.to_string()));
        }
    }
    path_candidate(word)
}

/// A URL keeps its own punctuation and loses the sentence's: a trailing
/// `.,;:!?` or quote is prose, a closing bracket is prose unless the URL
/// opened one (`https://x/a_(b)` is whole; `(https://x/a)` is not).
fn trim_url(word: &str) -> &str {
    let mut s = word;
    loop {
        let Some(last) = s.chars().last() else { return s };
        let cut = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '`' | '*' | '_' | '>' | ']' | '}' => {
                true
            }
            ')' => s.matches('(').count() < s.matches(')').count(),
            _ => false,
        };
        if !cut {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// A path-shaped word: rooted (`/`, `./`, `../`, `~/`) or carrying a `/`
/// somewhere, never a scheme. `:LINE` and `:LINE:COL` come off the end.
fn path_candidate(word: &str) -> Option<Found> {
    if word.contains("://") || word.len() < 2 {
        return None;
    }
    let rooted = word.starts_with('/')
        || word.starts_with("./")
        || word.starts_with("../")
        || word.starts_with("~/");
    if !rooted && !word.contains('/') {
        return None;
    }
    let (path, line) = split_line(word);
    if path.trim_matches(|c| c == '/' || c == '.').is_empty() {
        return None;
    }
    Some(Found::Path { path: path.to_string(), line })
}

/// `a/b.rs:42` → (`a/b.rs`, 42); `a/b.rs:42:7` → (`a/b.rs`, 42); a suffix
/// that is not digits stays on the path.
fn split_line(word: &str) -> (&str, Option<u32>) {
    let mut parts = word.rsplitn(3, ':');
    let last = parts.next().unwrap_or("");
    let mid = parts.next();
    let head = parts.next();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match (head, mid) {
        // `path:LINE:COL`
        (Some(h), Some(m)) if digits(last) && digits(m) => (h, m.parse().ok()),
        // `a:b:LINE` — the head still holds a colon; treat as `path:LINE`.
        (Some(_), Some(_)) if digits(last) => {
            let cut = word.len() - last.len() - 1;
            (&word[..cut], last.parse().ok())
        }
        (None, Some(m)) if digits(last) => (m, last.parse().ok()),
        _ => (word, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Found {
        Found::Url(s.into())
    }
    fn path(p: &str, line: Option<u32>) -> Found {
        Found::Path { path: p.into(), line }
    }
    fn targets(body: &str) -> Vec<Found> {
        extract(body).into_iter().map(|l| l.target).collect()
    }

    #[test]
    fn a_markdown_link_keeps_its_label() {
        let got = extract("see [the Jira ticket](https://jira.example.com/browse/ABC-1) first");
        assert_eq!(
            got,
            vec![Link {
                label: Some("the Jira ticket".into()),
                target: url("https://jira.example.com/browse/ABC-1")
            }]
        );
    }

    #[test]
    fn an_image_an_angle_target_and_a_title_are_the_target_alone() {
        assert_eq!(targets("![shot](./shot.png)"), vec![path("./shot.png", None)]);
        assert_eq!(targets("[x](<https://a.test/b c>)"), vec![url("https://a.test/b")]);
        assert_eq!(targets("[x](https://a.test/b \"a title\")"), vec![url("https://a.test/b")]);
        // The label is not also read as prose, and the target is not read twice.
        assert_eq!(targets("[docs/a.md](docs/a.md)").len(), 1);
    }

    #[test]
    fn a_bare_url_loses_the_sentence_punctuation_and_keeps_its_own() {
        assert_eq!(
            targets("go to https://a.test/x?y=1&z=2."),
            vec![url("https://a.test/x?y=1&z=2")]
        );
        assert_eq!(targets("(see https://a.test/p)"), vec![url("https://a.test/p")]);
        assert_eq!(targets("https://a.test/w_(x)"), vec![url("https://a.test/w_(x)")]);
        assert_eq!(targets("`https://a.test/c`, then"), vec![url("https://a.test/c")]);
        assert_eq!(targets("<https://a.test/auto>"), vec![url("https://a.test/auto")]);
        assert_eq!(targets("HTTP://A.test/up"), vec![url("HTTP://A.test/up")]);
        assert_eq!(targets("https://"), vec![]);
    }

    #[test]
    fn a_ticket_key_is_word_bounded() {
        assert_eq!(
            targets("blocks T-12, after (T-7)."),
            vec![Found::Ticket("T-12".into()), Found::Ticket("T-7".into())]
        );
        assert_eq!(targets("T- T-x T-1a NOT-12 aT-3"), vec![]);
    }

    #[test]
    fn a_path_is_rooted_or_has_a_slash_and_sheds_its_line() {
        assert_eq!(
            targets("edit `crates/core/src/board.rs:42` now"),
            vec![path("crates/core/src/board.rs", Some(42))]
        );
        assert_eq!(targets("at src/x.rs:10:5"), vec![path("src/x.rs", Some(10))]);
        assert_eq!(
            targets("/etc/hosts, ~/notes.md and ../up.txt"),
            vec![path("/etc/hosts", None), path("~/notes.md", None), path("../up.txt", None)]
        );
        assert_eq!(targets("file:///tmp/a.txt"), vec![path("/tmp/a.txt", None)]);
        assert_eq!(targets("./"), vec![]);
        assert_eq!(targets("/"), vec![]);
        // Not a path: no slash, an email, a scheme.
        assert_eq!(targets("board.rs a@b.test ssh://h/p"), vec![]);
        // `and/or` IS a candidate — existence is the caller's check.
        assert_eq!(targets("and/or"), vec![path("and/or", None)]);
    }

    #[test]
    fn duplicates_collapse_to_the_first_and_order_is_the_documents() {
        let body =
            "https://a.test/1 then T-3 and [again](https://a.test/1)\n\n- T-3 x\n- docs/a.md";
        assert_eq!(
            targets(body),
            vec![url("https://a.test/1"), Found::Ticket("T-3".into()), path("docs/a.md", None)]
        );
        assert_eq!(extract(body)[0].label, None);
    }

    #[test]
    fn a_broken_markdown_link_is_read_as_prose() {
        // No `(` after `]`, or the `)` is on another line: the words stay words.
        assert_eq!(targets("[x] https://a.test/y"), vec![url("https://a.test/y")]);
        assert_eq!(targets("[x](https://a.test/z\n)"), vec![url("https://a.test/z")]);
    }

    #[test]
    fn text_is_the_absence_of_nul() {
        assert!(looks_text(b""));
        assert!(looks_text("fn main() {}\n".as_bytes()));
        assert!(!looks_text(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
    }
}
