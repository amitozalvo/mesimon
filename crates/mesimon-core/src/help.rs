//! The `help` tool's answer (T-1105): mesimon's own guide and this build's
//! key and settings tables, searched by a person's question.
//!
//! **Nothing here reaches a model unless it asks.** The tool's description is
//! the only standing cost; the passages travel in a call's result, and only
//! the ones that match. A person asking an agent "how do I make new tickets
//! use a worktree" is the case it exists for; an agent that never asks pays
//! one tool definition and nothing else.
//!
//! **Two sources, and the tables cannot drift.** The prose is the user-facing
//! docs compiled in (`README.md`, `docs/USING.md`, `docs/PROMISES.md`,
//! `docs/REMOTE-CONTROL.md`), cut into passages under their headings. The
//! keys, the Settings rows, the column settings rows and the Esc menu are
//! read off `keymap` itself — the same tables the `?` overlay and the
//! Settings pages draw — so a row renamed or removed is renamed or removed
//! here in the same build. The prose can lag; the tables cannot.
//!
//! **Ranking is BM25 over words**, with a crude stem and a short synonym
//! table, because a question is words ("hotkey", "dark mode", "phone") and
//! the passages are words. A passage's heading counts its words twice. Pure:
//! no I/O, so the shim answers it without asking the daemon, and the answer
//! carries no ticket, no session and no board state.

use std::collections::HashMap;

use serde_json::json;

use crate::keymap::{self, Ctx, Group, MenuItem, Scope, SettingsSection, Value};

/// The docs compiled in, each with the name a passage cites.
const GUIDES: [(&str, &str); 4] = [
    ("README", include_str!("../../../README.md")),
    ("guide", include_str!("../../../docs/USING.md")),
    ("promises", include_str!("../../../docs/PROMISES.md")),
    ("Remote Control", include_str!("../../../docs/REMOTE-CONTROL.md")),
];

/// A prose passage is cut at a paragraph boundary once it passes this.
const PASSAGE_BYTES: usize = 1_400;
/// At most this many passages answer one query.
pub const MAX_PASSAGES: usize = 6;
/// And at most this many bytes of passage text, so one answer stays a few
/// thousand tokens however long the guide grows.
pub const MAX_ANSWER_BYTES: usize = 9_000;
/// A passage scoring under this share of the best one is left out.
const KEEP_SHARE: f64 = 0.3;

/// One searchable passage: where it is from, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// `guide › On the board`, `Settings › Behaviour`, `Keys › a ticket's page`.
    pub from: String,
    pub text: String,
}

/// Every passage this build knows, prose first, then the tables.
pub fn corpus() -> Vec<Passage> {
    let mut out = Vec::new();
    for (name, src) in GUIDES {
        out.extend(guide_passages(name, src));
    }
    out.extend(key_passages());
    out.extend(settings_passages());
    out.extend(column_passages());
    out.push(menu_passage());
    out
}

/// The headings a person can ask about, for an empty query or one that
/// matched nothing.
pub fn topics(corpus: &[Passage]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for p in corpus {
        if !seen.contains(&p.from) {
            seen.push(p.from.clone());
        }
    }
    seen
}

/// The tool's result: the passages matching `query`, best first, or the
/// topics when nothing does.
pub fn answer(query: &str) -> serde_json::Value {
    let corpus = corpus();
    let about = "Passages of mesimon's own docs and of this build's key, Settings, column \
                 settings and Esc menu tables, ranked by the query's words. Keys are spelled \
                 as the board spells them: ^ is ctrl. Settings opens with , on the board or \
                 from the Esc menu; a column's settings open with enter on its heading.";
    let hits = search(&corpus, query);
    if hits.is_empty() {
        return json!({
            "version": env!("CARGO_PKG_VERSION"),
            "matched": false,
            "about": about,
            "topics": topics(&corpus),
        });
    }
    let mut passages = Vec::new();
    let mut bytes = 0;
    for i in hits {
        let p = &corpus[i];
        if passages.len() >= MAX_PASSAGES || bytes + p.text.len() > MAX_ANSWER_BYTES {
            break;
        }
        bytes += p.text.len();
        passages.push(json!({ "from": p.from, "text": p.text }));
    }
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "matched": true,
        "about": about,
        "passages": passages,
    })
}

/// Indices into `corpus`, best first; empty when no word of `query` is in
/// any passage.
pub fn search(corpus: &[Passage], query: &str) -> Vec<usize> {
    let mut terms = terms(query);
    for t in terms.clone() {
        for s in synonyms(&t) {
            let s = stem(s);
            if !terms.contains(&s) {
                terms.push(s);
            }
        }
    }
    if terms.is_empty() {
        return Vec::new();
    }
    // Each passage as a bag of stems: its heading twice, its text once.
    let docs: Vec<HashMap<String, f64>> = corpus
        .iter()
        .map(|p| {
            let mut bag: HashMap<String, f64> = HashMap::new();
            for t in terms_of(&p.from) {
                *bag.entry(t).or_default() += 2.0;
            }
            for t in terms_of(&p.text) {
                *bag.entry(t).or_default() += 1.0;
            }
            bag
        })
        .collect();
    let lens: Vec<f64> = docs.iter().map(|d| d.values().sum()).collect();
    let avg = lens.iter().sum::<f64>() / lens.len().max(1) as f64;
    let n = docs.len() as f64;
    let (k1, b) = (1.2, 0.75);
    let mut scored: Vec<(usize, f64)> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut score = 0.0;
            for t in &terms {
                let Some(tf) = d.get(t) else { continue };
                let df = docs.iter().filter(|d| d.contains_key(t)).count() as f64;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                score += idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * lens[i] / avg));
            }
            (i, score)
        })
        .filter(|(_, s)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let best = scored.first().map(|s| s.1).unwrap_or(0.0);
    scored.into_iter().filter(|(_, s)| *s >= best * KEEP_SHARE).map(|(i, _)| i).collect()
}

// ------------------------------------------------------------------ words

/// Words a question carries no meaning in.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "could", "do", "does", "for", "from",
    "get", "how", "i", "if", "in", "into", "is", "it", "its", "me", "mesimon", "my", "of", "on",
    "or", "so", "that", "the", "there", "this", "to", "way", "what", "when", "where", "which",
    "who", "why", "will", "with", "would", "you", "your",
];

/// A query's stems, stopwords out, each once.
fn terms(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms_of(s) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Every stem in `s`, in order, repeats kept.
fn terms_of(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

/// A crude stem: enough that "worktrees", "changed" and "notifications"
/// meet "worktree", "change" and "notification".
fn stem(w: &str) -> String {
    let mut w = w.to_string();
    if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w.pop();
    }
    for suffix in ["ing", "ed"] {
        if w.len() > suffix.len() + 3 && w.ends_with(suffix) {
            w.truncate(w.len() - suffix.len());
            break;
        }
    }
    if w.len() > 3 && w.ends_with('e') {
        w.pop();
    }
    w
}

/// The words a person uses for what the docs name otherwise.
fn synonyms(stem_: &str) -> &'static [&'static str] {
    match stem_ {
        "hotkey" | "shortcut" | "keybind" | "bind" | "keyboard" | "press" => &["key", "keys"],
        "dark" | "light" | "colour" | "color" | "colours" => &["theme"],
        "alert" | "notify" | "banner" | "sound" => &["notification", "notifications"],
        "phone" | "mobile" | "browser" | "remot" => &["remote", "control"],
        "upgrad" | "updat" => &["update", "updating"],
        "kill" | "quit" | "shut" | "exit" => &["stop", "stopping"],
        "delet" | "remov" => &["delete", "archive"],
        "branch" => &["worktree", "merge"],
        "checkout" | "folder" | "workspac" => &["workspace", "worktree", "checkout"],
        "permission" | "approv" => &["mode", "permission"],
        "idl" | "park" => &["sleep"],
        "cost" | "quota" | "limit" | "spend" => &["usage", "quota"],
        "pr" | "prs" => &["pull", "request"],
        _ => &[],
    }
}

// ----------------------------------------------------------------- prose

/// A guide cut into passages: each `#`-heading's text, split at paragraph
/// boundaries past `PASSAGE_BYTES`, pictures and captions dropped. Code
/// blocks stay whole with the paragraph they sit in.
fn guide_passages(name: &str, src: &str) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut heading: Vec<String> = Vec::new();
    let mut body = String::new();
    let mut fenced = false;
    let flush = |heading: &[String], body: &mut String, out: &mut Vec<Passage>| {
        let text = body.trim();
        if !text.is_empty() {
            let mut from = name.to_string();
            for h in heading {
                from.push_str(" › ");
                from.push_str(h);
            }
            out.push(Passage { from, text: text.to_string() });
        }
        body.clear();
    };
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced {
            let level = line.chars().take_while(|c| *c == '#').count();
            if level > 0 && line[level..].starts_with(' ') {
                flush(&heading, &mut body, &mut out);
                let title = line[level..].trim().to_string();
                // The document's own title is the passage's name already.
                heading.truncate(level.saturating_sub(2));
                if level >= 2 {
                    heading.push(title);
                }
                continue;
            }
            let t = line.trim_start();
            // Pictures, captions and the README's HTML header.
            if t.starts_with("![") || t.starts_with('<') {
                continue;
            }
            // Past the size, a paragraph's end cuts; a list item's start
            // cuts a long list.
            if body.len() > PASSAGE_BYTES {
                if line.trim().is_empty() {
                    flush(&heading, &mut body, &mut out);
                    continue;
                }
                if t.starts_with("- ") || t.starts_with("* ") || body.len() > 2 * PASSAGE_BYTES {
                    flush(&heading, &mut body, &mut out);
                }
            }
        }
        body.push_str(line);
        body.push('\n');
    }
    flush(&heading, &mut body, &mut out);
    out
}

// ---------------------------------------------------------------- tables

/// Where a screen scope is, in words a question uses. A scope without one
/// is named by its footer word alone.
fn scope_place(scope: Scope) -> &'static str {
    match scope {
        Scope::Global => "every screen",
        Scope::Board => "the board",
        Scope::Ticket => "a ticket's page",
        Scope::Diff | Scope::DiffView => "the diff viewer",
        Scope::Menu => "the Esc menu",
        Scope::Settings | Scope::Sections => "Settings",
        Scope::ColumnSettings => "a column's settings",
        Scope::Search => "search",
        Scope::Move => "moving a card",
        Scope::Archived => "archived tickets",
        Scope::Drawer => "external sessions",
        _ => "",
    }
}

/// The contexts a binding's hint is read in: a hint is a function of the
/// screen's state, and the table is for every state a person meets.
fn hint_contexts() -> Vec<(&'static str, Ctx)> {
    vec![
        ("", Ctx::default()),
        ("", Ctx { has_ticket: true, ..Default::default() }),
        ("", Ctx { has_ticket: true, ticket_hot: true, ..Default::default() }),
        ("on a column heading", Ctx { col_header: true, ..Default::default() }),
    ]
}

/// One passage per screen and key group: each key as the overlay spells it
/// and every hint it wears. A binding with no hint in any state (an
/// undisclosed one, as `y` before its first press) is left out, as the
/// overlay leaves it out.
fn key_passages() -> Vec<Passage> {
    let contexts = hint_contexts();
    let mut out = Vec::new();
    for scope in Scope::ALL {
        if scope == Scope::Arcade {
            continue;
        }
        for group in Group::ALL {
            let mut lines: Vec<String> = Vec::new();
            for b in keymap::bindings(scope).iter().filter(|b| b.group == group) {
                let mut hints: Vec<String> = Vec::new();
                for (when, ctx) in &contexts {
                    let hint = (b.hint)(ctx);
                    if hint.is_empty() {
                        continue;
                    }
                    let hint =
                        if when.is_empty() { hint.to_string() } else { format!("{hint} ({when})") };
                    if !hints.contains(&hint) {
                        hints.push(hint);
                    }
                }
                if !hints.is_empty() {
                    let line = format!("{}: {}", b.show, hints.join("; "));
                    if !lines.contains(&line) {
                        lines.push(line);
                    }
                }
            }
            if lines.is_empty() {
                continue;
            }
            let place = scope_place(scope);
            let from = if place.is_empty() {
                format!("Keys › {} › {}", scope.word().to_lowercase(), group.title().to_lowercase())
            } else {
                format!("Keys › {place} › {}", group.title().to_lowercase())
            };
            out.push(Passage { from, text: lines.join("\n") });
        }
    }
    out
}

/// A row as a line: its name, the choices it steps through, and its detail
/// when it has one. A row's current value is this board's state and is left
/// out: the table is the same on every board.
fn row_line(m: &MenuItem, ctx: &Ctx) -> String {
    let mut label = (m.label)(ctx);
    match keymap::value(m.verb, ctx) {
        Some(Value::Choice { options, .. }) => {
            label = format!("{label} [{}]", options.join(" | "));
        }
        Some(Value::Switch(_)) => label.push_str(" [on | off]"),
        _ => {}
    }
    let detail = (m.detail)(ctx);
    match (detail.is_empty(), m.key.is_empty()) {
        (true, true) => label,
        (false, true) => format!("{label}: {detail}"),
        (true, false) => format!("{label} (key {})", m.key),
        (false, false) => format!("{label} (key {}): {detail}", m.key),
    }
}

/// One passage per Settings section, each row by name with its detail, under
/// the heading the page draws over it.
fn settings_passages() -> Vec<Passage> {
    let ctx = Ctx::default();
    let mut out = Vec::new();
    for section in SettingsSection::ALL {
        let rows = keymap::section_rows(section);
        let text = if section == SettingsSection::Theme {
            "a picker of the board's colour themes, with a small board drawn in the one under \
             the cursor; enter keeps it"
                .to_string()
        } else if rows.is_empty() {
            continue;
        } else {
            let mut lines = Vec::new();
            for m in rows {
                if let Some(h) = keymap::settings_heading(m.verb) {
                    lines.push(format!("{}:", h.to_lowercase()));
                }
                lines.push(row_line(m, &ctx));
            }
            lines.join("\n")
        };
        out.push(Passage {
            from: format!("Settings › {}", section.name()),
            text: format!(
                "Settings (, on the board, or the Esc menu's Settings row) › {}:\n{text}",
                section.name()
            ),
        });
    }
    out
}

/// The column settings dialog's rows, one passage per heading the dialog
/// draws, so a question about one group is not diluted by the rest.
fn column_passages() -> Vec<Passage> {
    let ctx = Ctx::default();
    let lead = "A column's settings open with enter on the column's heading (k from its top \
                card); each row applies to that column alone.";
    let mut out: Vec<Passage> = Vec::new();
    let mut group: Option<Option<&str>> = None;
    for m in keymap::column_rows() {
        let g = keymap::column_group(m.verb);
        if group != Some(g) {
            let name = match g {
                None => "Column settings".to_string(),
                Some("") => "Column settings › agent behaviour".to_string(),
                Some(g) => format!("Column settings › {}", g.to_lowercase()),
            };
            out.push(Passage { from: name, text: lead.to_string() });
            group = Some(g);
        }
        let p = out.last_mut().expect("pushed above");
        p.text.push('\n');
        p.text.push_str(&row_line(m, &ctx));
    }
    out
}

/// The Esc menu's rows: what acts on the board as a whole.
fn menu_passage() -> Passage {
    let ctx = Ctx::default();
    let mut lines = vec![
        "The Esc menu (esc on the board) holds what acts on the board as a whole:".to_string(),
    ];
    for m in keymap::menu_rows() {
        let line = row_line(m, &ctx);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    Passage { from: "Esc menu".into(), text: lines.join("\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top_text(query: &str, n: usize) -> String {
        let corpus = corpus();
        search(&corpus, query)
            .into_iter()
            .take(n)
            .map(|i| format!("{}\n{}", corpus[i].from, corpus[i].text))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The ticket's own example: the answer names the column's Workspace
    /// row, which is where new tickets' workspace is chosen.
    #[test]
    fn a_default_workspace_question_reaches_the_column_workspace_row() {
        let text = top_text("how can I change default workspace to worktree", 3);
        assert!(
            text.contains("Workspace [worktree | shared checkout]: new tickets filed here"),
            "{text}"
        );
    }

    #[test]
    fn questions_in_a_persons_words_find_their_passage() {
        let cases = [
            ("what is the hotkey to open settings", ","),
            ("how do I turn off notifications sound", "Settings › Notifications"),
            ("dark mode", "Settings › Theme"),
            ("how do I update mesimon", "guide › Updating"),
            ("control the board from my phone", "Remote Control"),
            ("stop everything and kill the daemon", "guide › Stopping everything"),
            ("keep my mac awake while agents work", "Keep this machine awake"),
            ("what can my agents see", "guide › What your agents can see"),
            ("search tickets", "Keys › search"),
            ("merge a branch", "merge"),
        ];
        for (q, want) in cases {
            let text = top_text(q, 3);
            assert!(text.contains(want), "{q:?} wanted {want:?} in:\n{text}");
        }
    }

    /// The tables are the keymap's: a key the overlay shows is here, and a
    /// row's detail is the page's detail, word for word.
    #[test]
    fn the_tables_are_read_off_the_keymap() {
        let corpus = corpus();
        let board = corpus.iter().find(|p| p.from == "Keys › the board › app").unwrap();
        assert!(board.text.contains(",: settings"), "{}", board.text);
        let nav = corpus.iter().find(|p| p.from == "Keys › the board › move around").unwrap();
        assert!(nav.text.contains("column settings (on a column heading)"), "{}", nav.text);
        let behaviour = corpus.iter().find(|p| p.from == "Settings › Behaviour").unwrap();
        assert!(behaviour.text.contains("Keep this machine awake"), "{}", behaviour.text);
        // An undisclosed binding stays undisclosed.
        assert!(!corpus
            .iter()
            .filter(|p| p.from.starts_with("Keys › the board"))
            .any(|p| p.text.lines().any(|l| l.starts_with("y: "))));
        // Every Settings section with rows is a passage.
        for s in SettingsSection::ALL {
            assert!(corpus.iter().any(|p| p.from == format!("Settings › {}", s.name())), "{s:?}");
        }
    }

    #[test]
    fn guides_are_cut_under_their_headings_without_pictures() {
        let corpus = corpus();
        let board = corpus.iter().find(|p| p.from == "guide › On the board").unwrap();
        assert!(board.text.contains("`o` adds a ticket"), "{}", board.text);
        assert!(!corpus.iter().any(|p| p.text.contains("![")), "a picture leaked");
        // A long section is cut, each piece keeping the heading.
        assert!(
            corpus
                .iter()
                .filter(|p| p.from == "guide › The crown: one agent runs the board")
                .count()
                > 1
        );
        for p in &corpus {
            assert!(p.text.len() < 6_000, "{} is {} bytes", p.from, p.text.len());
        }
    }

    #[test]
    fn an_answer_is_bounded_and_says_when_nothing_matched() {
        let a = answer("worktree workspace merge crown settings keys notes");
        assert_eq!(a["matched"], true);
        let passages = a["passages"].as_array().unwrap();
        assert!(!passages.is_empty() && passages.len() <= MAX_PASSAGES);
        let bytes: usize = passages.iter().map(|p| p["text"].as_str().unwrap().len()).sum();
        assert!(bytes <= MAX_ANSWER_BYTES);

        for q in ["", "   ", "how do I", "zzqxj"] {
            let a = answer(q);
            assert_eq!(a["matched"], false, "{q:?}");
            let topics = a["topics"].as_array().unwrap();
            assert!(topics.iter().any(|t| t == "Column settings › new tickets"), "{q:?}");
        }
    }

    #[test]
    fn stems_meet() {
        for (a, b) in [
            ("worktrees", "worktree"),
            ("changed", "change"),
            ("changes", "changing"),
            ("notifications", "notification"),
            ("settings", "setting"),
        ] {
            assert_eq!(stem(a), stem(b), "{a} {b}");
        }
        assert_eq!(stem("pass"), "pass");
    }
}
