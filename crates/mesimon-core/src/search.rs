//! The board's fuzzy search (T-349) — ranking, nothing else. The picker's
//! keys are `keymap::Scope::Search` and its pixels are `tui/src/ui/search.rs`;
//! what lives here is the pure question "which tickets does this query mean,
//! and in what order".
//!
//! **The matcher is `nucleo-matcher`, not ours.** It is the engine behind
//! Helix's picker: fzf's v2 scoring (Smith-Waterman with the word-boundary,
//! camelCase and path-separator bonuses), smart case, unicode normalisation,
//! and — the part that is hard to write and easy to get subtly wrong — the
//! match *indices*, so the picker can underline the characters the query
//! actually landed on. `Pattern::parse` also gives fzf's query grammar for
//! free, which is the one grammar a reader who reaches for `/` already knows:
//! space-separated words are ANDed, `'foo` is a literal substring, `^foo` and
//! `foo$` anchor, and `!foo` excludes.
//!
//! **The row IS the haystack.** A hit is scored against the exact text the
//! picker draws — key, title, column, tags — so a highlighted character is
//! always one the reader can see, and there is no hidden field that explains
//! why a card matched. That is the whole reason [`Hit`] carries the three
//! fields split out rather than one string: matching wants them joined,
//! drawing wants them apart, and splitting the indices at the join is cheaper
//! and more honest than matching three times.
//!
//! **Archived tickets rank below live ones, never among them** (user, T-349).
//! A hard tier rather than a score penalty: a penalty makes "where did that
//! ticket go" a question about weights, and the answer has to be "keep
//! scrolling", which a tier says once.
//!
//! Everything here runs over the snapshot the board already holds, on the
//! keystroke. There is no index to warm, no wire to wait for and no debounce:
//! a board is tens to hundreds of cards of a few dozen characters each, and
//! `how_long_a_big_board_takes` measures a DEBUG build ranking 500 of them at
//! 0.8 ms for an empty query, 1.0 ms for one word and 3.1 ms for three — a
//! release build being several times under that again. A real board is an
//! order of magnitude smaller than the fixture, so the whole pass costs less
//! than the frame it is drawn into.
//!
//! That budget is what buys the two behaviours above it: the list re-ranks on
//! every keystroke rather than filtering the last result (a filter cannot
//! recover a hit the previous keystroke dropped), and it re-ranks again on
//! every snapshot, so a card an agent moved or archived under an open picker
//! cannot leave a row that sends Enter somewhere that is no longer there.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::board::{Board, Ticket};

/// A query that matches a ticket's short key outright wins its tier. Typing
/// `T-3` is a jump, not a search, and `T-3` fuzzy-matches `T-349` too.
const EXACT_KEY: u32 = 1_000_000;

/// One field of a row: the text, and which of ITS characters matched.
/// Indices are character offsets into `text`, sorted and deduplicated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    pub matched: Vec<u32>,
}

impl Field {
    /// Is the character at `i` (a char offset) one the query matched?
    pub fn is_match(&self, i: usize) -> bool {
        self.matched.binary_search(&(i as u32)).is_ok()
    }
}

/// One row of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: ulid::Ulid,
    /// The ticket's short key (`T-12`).
    pub key: Field,
    pub title: Field,
    /// The column, then the tags, then `archived` or `snoozed` — the words
    /// that say where a card is without leaving the picker, and that a query
    /// can name: `review feature` narrows to exactly that.
    pub trail: Field,
    pub archived: bool,
    pub score: u32,
}

/// A reusable matcher. `Matcher::new` eagerly allocates ~135 KB of scoring
/// matrix, so one is kept for the life of the picker rather than minted per
/// keystroke — which is also what the crate's own documentation asks for.
pub struct Searcher {
    matcher: Matcher,
    pattern: Pattern,
    /// `Utf32Str`'s scratch, reused across every haystack in a pass.
    buf: Vec<char>,
    indices: Vec<u32>,
}

impl Default for Searcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Searcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            pattern: Pattern::default(),
            buf: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// The board's tickets that `query` means, best first.
    ///
    /// An empty query matches everything at score 0, so the picker opens as
    /// the board's own list — live cards in board order, archived after them,
    /// newest archive first. That is not a special case in here; it is what
    /// `Pattern` with no atoms does, and leaning on it is what keeps "open
    /// the picker" and "clear the query" the same screen.
    pub fn rank(&mut self, board: &Board, query: &str, archived: bool) -> Vec<Hit> {
        self.pattern.reparse(query, CaseMatching::Smart, Normalization::Smart);
        let trimmed = query.trim();
        let mut out: Vec<(bool, u32, usize, Hit)> = Vec::new();

        // Board order is the tie-break, so it is also the enumeration order:
        // live cards column by column, then the archived list as the archived
        // dialog would show it.
        let columns: Vec<String> = board.sorted_columns().iter().map(|c| c.name.clone()).collect();
        let live: Vec<&Ticket> = columns.iter().flat_map(|c| board.column_tickets(c)).collect();
        let rest: Vec<&Ticket> = if archived { board.archived_tickets() } else { Vec::new() };
        for (rank, t) in live.into_iter().chain(rest).enumerate() {
            if let Some(hit) = self.score(t, trimmed) {
                out.push((hit.archived, hit.score, rank, hit));
            }
        }
        // Archived below live; inside a tier, score first and board order
        // second — so an empty query is the board, and equal scores never
        // shuffle between frames.
        out.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        out.into_iter().map(|(_, _, _, hit)| hit).collect()
    }

    fn score(&mut self, ticket: &Ticket, query: &str) -> Option<Hit> {
        let key = ticket.short_key.clone();
        // The title is drawn in cells, so it crosses `scrub_cells` on the
        // way in — the one boundary function, never a second sanitizer.
        let title = crate::text::scrub_cells(&ticket.title, false);
        let trail = trail_of(ticket);
        // One haystack, joined by the single space the picker draws between
        // the fields, so a query may cross a field boundary the way the eye
        // does: `t-3 redirect` is one atom per word and both land.
        let haystack = format!("{key} {title} {trail}");

        self.indices.clear();
        let score = self.pattern.indices(
            Utf32Str::new(&haystack, &mut self.buf),
            &mut self.matcher,
            &mut self.indices,
        )?;
        self.indices.sort_unstable();
        self.indices.dedup();

        // Split the indices back over the three fields. Every offset is a
        // CHARACTER offset — nucleo counts characters, and so does the draw.
        let key_len = key.chars().count();
        let title_len = title.chars().count();
        let title_at = key_len + 1;
        let trail_at = title_at + title_len + 1;
        let cut = |lo: usize, hi: usize| -> Vec<u32> {
            self.indices
                .iter()
                .filter(|i| (**i as usize) >= lo && (**i as usize) < hi)
                .map(|i| *i - lo as u32)
                .collect()
        };
        let hit = Hit {
            id: ticket.id,
            key: Field { matched: cut(0, key_len), text: key },
            title: Field { matched: cut(title_at, title_at + title_len), text: title },
            trail: Field { matched: cut(trail_at, trail_at + trail.chars().count()), text: trail },
            archived: ticket.is_archived(),
            score: score + exact_key_bonus(ticket, query),
        };
        Some(hit)
    }
}

/// The words under the title: where the card is, what it wears, and whether
/// it is off the board. All of them searchable, which is what makes `review`,
/// `feature` and `archived` filters without a grammar to learn.
fn trail_of(ticket: &Ticket) -> String {
    // The column in the register the board draws it in, so the word in the
    // row is the word on the screen behind it. Smart case keeps a lowercase
    // query matching it (`todo` finds `TODO`).
    let mut parts: Vec<String> = vec![ticket.column.to_uppercase()];
    parts.extend(ticket.tags.iter().map(|t| t.name.clone()));
    if ticket.is_archived() {
        parts
            .push(if ticket.snooze_until_secs().is_some() { "snoozed" } else { "archived" }.into());
    }
    parts.join(" ")
}

fn exact_key_bonus(ticket: &Ticket, query: &str) -> u32 {
    if !query.is_empty() && query.eq_ignore_ascii_case(&ticket.short_key) {
        EXACT_KEY
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Archived, Board, Column, TagRef, Ticket};

    fn ulid_n(n: u128) -> ulid::Ulid {
        ulid::Ulid::from(n)
    }

    fn ticket(n: u128, key: &str, title: &str, column: &str) -> Ticket {
        Ticket {
            id: ulid_n(n),
            short_key: key.into(),
            title: title.into(),
            column: column.into(),
            order: format!("{n:04}"),
            created_at: "2026-09-01T00:00:00Z".into(),
            created_by: String::new(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: false,
            execution_policy: Default::default(),
            workspace: None,
            import_origin: None,
            raised: None,
            previous_column: None,
            tags: Vec::new(),
            notes: Vec::new(),
            archived: None,
        }
    }

    fn board() -> Board {
        Board {
            columns: vec![Column::new("TODO", "a"), Column::new("REVIEW", "b")],
            tickets: vec![
                ticket(1, "T-1", "Fix auth redirect", "TODO"),
                ticket(2, "T-2", "Login page copy", "REVIEW"),
                ticket(3, "T-3", "Auth middleware", "REVIEW"),
            ],
            ..Board::default()
        }
    }

    fn archived(at: &str, until: Option<&str>) -> Archived {
        Archived {
            at: at.into(),
            by: "local".into(),
            until: until.map(Into::into),
            needs_you: false,
        }
    }

    fn keys(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.key.text.as_str()).collect()
    }

    #[test]
    fn an_empty_query_is_the_board_in_board_order() {
        let mut s = Searcher::new();
        let hits = s.rank(&board(), "", true);
        assert_eq!(keys(&hits), ["T-1", "T-2", "T-3"]);
        assert!(hits.iter().all(|h| h.score == 0));
        assert!(hits.iter().all(|h| h.key.matched.is_empty() && h.title.matched.is_empty()));
    }

    #[test]
    fn a_query_matches_across_the_fields_it_draws() {
        let mut s = Searcher::new();
        let hits = s.rank(&board(), "auth", true);
        assert_eq!(keys(&hits), ["T-1", "T-3"]);
        // The highlight lands on the title, at the characters the eye sees.
        let t3 = hits.iter().find(|h| h.key.text == "T-3").expect("T-3");
        assert_eq!(t3.title.matched, vec![0, 1, 2, 3]);
        assert!(t3.title.is_match(0) && !t3.title.is_match(4));
    }

    #[test]
    fn the_column_and_the_tags_are_searchable() {
        let mut b = board();
        b.tickets[0].tags.push(TagRef { name: "FEATURE".into(), group: 1 });
        let mut s = Searcher::new();
        assert_eq!(keys(&s.rank(&b, "review", true)), ["T-2", "T-3"]);
        // The row says the column the way the board does.
        assert!(s.rank(&b, "review", true)[0].trail.text.starts_with("REVIEW"));
        assert_eq!(keys(&s.rank(&b, "feature", true)), ["T-1"]);
        // Two words are ANDed, fzf's rule and telescope's.
        assert_eq!(keys(&s.rank(&b, "auth review", true)), ["T-3"]);
    }

    #[test]
    fn an_exact_short_key_wins_its_tier() {
        let mut b = board();
        b.tickets.push(ticket(4, "T-34", "Auth retries", "TODO"));
        let mut s = Searcher::new();
        // `T-3` fuzzy-matches `T-34` too, and it is the earlier card; the
        // ticket the reader NAMED is still first.
        assert_eq!(s.rank(&b, "T-3", true)[0].key.text, "T-3");
        assert_eq!(s.rank(&b, "t-3", true)[0].key.text, "T-3");
    }

    #[test]
    fn archived_tickets_rank_below_every_live_one() {
        let mut b = board();
        b.tickets[2].archived = Some(archived("@1757030400", None));
        let mut s = Searcher::new();
        // T-3's title is the better match for `auth`, and it still comes
        // second: the tier outranks the score.
        let hits = s.rank(&b, "auth", true);
        assert_eq!(keys(&hits), ["T-1", "T-3"]);
        assert!(hits[1].archived);
        assert!(hits[1].trail.text.ends_with("archived"));
        // And the word is searchable, which is the filter with no grammar.
        assert_eq!(keys(&s.rank(&b, "archived", true)), ["T-3"]);
        // Excluded, it is not there at all.
        assert_eq!(keys(&s.rank(&b, "auth", false)), ["T-1"]);
        assert!(s.rank(&b, "archived", false).is_empty());
    }

    #[test]
    fn a_snoozed_ticket_says_snoozed() {
        let mut b = board();
        b.tickets[1].archived = Some(archived("@1757030400", Some("@1758326400")));
        let mut s = Searcher::new();
        assert_eq!(keys(&s.rank(&b, "snoozed", true)), ["T-2"]);
        assert!(s.rank(&b, "archived", true).is_empty());
    }

    #[test]
    fn fzf_syntax_reaches_the_query() {
        let mut b = board();
        b.tickets.push(ticket(4, "T-4", "Authorise the webhook", "TODO"));
        let mut s = Searcher::new();
        // A literal substring: `auth` fuzzy-matches "Authorise" anyway, so
        // the anchor is what tells the two apart.
        assert_eq!(keys(&s.rank(&b, "'middleware", true)), ["T-3"]);
        // And the exclusion.
        let hits = s.rank(&b, "auth !middleware", true);
        assert_eq!(keys(&hits), ["T-1", "T-4"]);
    }

    #[test]
    fn a_query_nothing_matches_is_empty_and_not_everything() {
        let mut s = Searcher::new();
        assert!(s.rank(&board(), "zzzzzz", true).is_empty());
    }

    #[test]
    fn a_multi_line_title_is_flattened_before_it_is_matched() {
        let mut b = board();
        b.tickets[0].title = "Fix auth\nredirect".into();
        let mut s = Searcher::new();
        let hits = s.rank(&b, "auth", true);
        assert!(!hits[0].title.text.contains('\n'), "{:?}", hits[0].title.text);
    }

    /// Not an assertion about wall clock — a printed number, so the figures
    /// in the module header are ones someone can check. A timing assertion
    /// here would be a flake on a loaded machine and would measure the
    /// machine rather than the code.
    #[test]
    #[ignore = "prints a timing; run with --ignored"]
    fn how_long_a_big_board_takes() {
        let mut b = board();
        b.tickets.clear();
        for n in 0..500u128 {
            let mut t = ticket(n + 10, &format!("T-{n}"), "Fix the auth redirect loop", "TODO");
            t.tags.push(TagRef { name: "FEATURE".into(), group: 1 });
            b.tickets.push(t);
        }
        let mut s = Searcher::new();
        for q in ["", "auth", "fix auth red", "zzzz"] {
            let t0 = std::time::Instant::now();
            let mut hits = 0;
            for _ in 0..100 {
                hits = s.rank(&b, q, true).len();
            }
            println!(
                "{:>14} {:>5} hits {:>8.1}µs",
                format!("{q:?}"),
                hits,
                t0.elapsed().as_micros() as f64 / 100.0
            );
        }
    }
}
