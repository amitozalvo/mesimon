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
//! agree on `[label](target)`, and share the fence rule (`fence_open`,
//! `fence_closes`) and the shell fence's commands (`commands`, T-1141), so
//! the command a list names is the rows the draw marks.

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
    /// A command from a shell fence (T-1141), its prompt off and its
    /// continuation lines kept, joined by `\n`: what pasted into a shell
    /// runs as written. Copied, never run.
    Command(String),
}

impl Found {
    /// The target as a body spells it: the address itself, a path with its
    /// `:LINE` back on, an attachment's `attachment:` word.
    pub fn spelling(&self) -> String {
        match self {
            Found::Url(s) | Found::Ticket(s) | Found::Command(s) => s.clone(),
            Found::Path { path, line } => {
                line.map_or_else(|| path.clone(), |n| format!("{path}:{n}"))
            }
            Found::Attachment(id) => crate::attachment::target(*id),
        }
    }
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
/// occurrence keeps its label). A shell fence's lines are its commands
/// (`commands`) and nothing else; an untagged fence's command lines are
/// commands and its other lines prose; every other line is prose, a fence of
/// another language included. In prose a `[label](target)` is one link,
/// never a label and a bare target, an inline code span that is a command
/// (`inline_command`) is that command, and every other whitespace-delimited
/// word is read on its own.
pub fn extract(body: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let mut push = |link: Link| {
        if !out.iter().any(|l| l.target == link.target) {
            out.push(link);
        }
    };
    let lines: Vec<&str> = body.lines().collect();
    let mut prose = String::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        at += 1;
        prose.push_str(line);
        prose.push('\n');
        let Some((ch, len, info)) = fence_open(line) else { continue };
        let start = at;
        while at < lines.len() && !fence_closes(lines[at], ch, len) {
            at += 1;
        }
        let rows = &lines[start..at];
        if holds_commands(info) {
            // An untagged fence's other rows are prose; a shell fence's are
            // its comments and output.
            let loose = info.is_empty();
            let mut row = 0;
            for (command, span) in commands(info, rows) {
                if loose {
                    for r in &rows[row..span.start] {
                        prose.push_str(r);
                        prose.push('\n');
                    }
                }
                scan(&prose, true, &mut push);
                prose.clear();
                push(Link { label: None, target: Found::Command(command) });
                row = span.end;
            }
            if loose {
                for r in &rows[row..] {
                    prose.push_str(r);
                    prose.push('\n');
                }
            }
        } else {
            for row in rows {
                prose.push_str(row);
                prose.push('\n');
            }
        }
        at += 1; // the closing line, if there is one
    }
    scan(&prose, true, &mut push);
    out
}

/// Every target `body` names in its words, every fence's rows and code span
/// read as prose too: `extract` before T-1141. What a picture's reference check
/// reads, so a picture named anywhere in the words stays named.
pub fn words(body: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    scan(body, false, &mut |link: Link| {
        if !out.iter().any(|l| l.target == link.target) {
            out.push(link);
        }
    });
    out
}

/// The prose walk: markdown links and bare words, in order, and with
/// `spans` a code span that is a command (`inline_command`) as that command.
fn scan(body: &str, spans: bool, push: &mut impl FnMut(Link)) {
    let chars: Vec<char> = body.chars().collect();
    let mut word = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if spans && c == '`' {
            let ticks = chars[i..].iter().take_while(|x| **x == '`').count();
            let command = span_close(&chars, i + ticks, ticks).and_then(|close| {
                let text: String = chars[i + ticks..close].iter().collect();
                inline_command(&text).map(|command| (command, close + ticks))
            });
            if let Some((command, end)) = command {
                if let Some(target) = classify(&word) {
                    push(Link { label: None, target });
                }
                word.clear();
                push(Link { label: None, target: Found::Command(command) });
                i = end;
            } else {
                word.extend(&chars[i..i + ticks]);
                i += ticks;
            }
            continue;
        }
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
}

/// Where a code span opened by `len` backticks closes: the next run of
/// exactly `len` on the same line (rich.rs's rule, held to the line).
fn span_close(chars: &[char], from: usize, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() && chars[i] != '\n' {
        let n = chars[i..].iter().take_while(|x| **x == '`').count();
        if n == len {
            return Some(i);
        }
        i += n.max(1);
    }
    None
}

/// A fence's opening line: three or more backticks or tildes, indented or
/// not, and the first word of its info string (`bash` of ```` ```bash ````,
/// empty when it names none).
pub fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start();
    for ch in ['`', '~'] {
        let n = t.chars().take_while(|c| *c == ch).count();
        if n >= 3 {
            let info = t[n..].trim_start();
            let end = info.find(|c: char| c.is_whitespace() || c == '{' || c == ',');
            return Some((ch, n, &info[..end.unwrap_or(info.len())]));
        }
    }
    None
}

/// Does `line` close a fence opened by `len` of `ch`: a run at least as
/// long, and nothing else on the line?
pub fn fence_closes(line: &str, ch: char, len: usize) -> bool {
    let t = line.trim();
    t.chars().count() >= len && t.chars().all(|c| c == ch)
}

/// The info words that say a fence holds shell (T-1141). An untagged fence
/// is not one: it holds output and trees as often as commands, so only its
/// command lines are commands (`commands`).
pub fn is_shell(info: &str) -> bool {
    ["sh", "bash", "zsh", "shell", "console", "shell-session", "shellsession", "fish", "ksh"]
        .iter()
        .any(|w| w.eq_ignore_ascii_case(info))
}

/// Does a fence tagged `info` hold commands: a shell fence, or an untagged
/// one (its command lines only)?
pub fn holds_commands(info: &str) -> bool {
    info.is_empty() || is_shell(info)
}

/// A fence's commands, each with the rows it spans. One command per line; a
/// blank line and a `#` comment are none; a leading `$ ` or `% ` prompt
/// comes off. A line ending in `\`, `|`, `&&` or `||` runs on into the
/// next, and a heredoc (`<<EOF`) runs to its terminator, so the command
/// keeps every row it needs, joined by `\n`. In a `console` fence only a
/// prompted line starts a command: the rest is the session's output. In an
/// untagged fence (`info` empty) a line starts one when it is prompted, the
/// agent's `! ` included, or led by a known program (`led_by_program`).
pub fn commands(info: &str, rows: &[&str]) -> Vec<(String, std::ops::Range<usize>)> {
    let session =
        ["console", "shell-session", "shellsession"].iter().any(|w| w.eq_ignore_ascii_case(info));
    let loose = info.is_empty();
    let mut out = Vec::new();
    let mut at = 0;
    while at < rows.len() {
        let start = at;
        let line = rows[at].trim();
        at += 1;
        let line = match prompt_off(line, loose) {
            Some(rest) => rest,
            None if session => continue,
            None if loose && !led_by_program(line) => continue,
            None => line,
        };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut text = line.to_string();
        let mut last = line.to_string();
        loop {
            if let Some(word) = heredoc(&last) {
                while at < rows.len() {
                    let row = rows[at].trim_end();
                    at += 1;
                    text.push('\n');
                    text.push_str(row);
                    if row.trim() == word {
                        break;
                    }
                }
            }
            if !runs_on(&last) || at >= rows.len() {
                break;
            }
            let row = rows[at].trim();
            at += 1;
            // A session's continuation wears the shell's `> `.
            let row = if session { row.strip_prefix("> ").unwrap_or(row) } else { row };
            text.push('\n');
            text.push_str(row);
            last = row.to_string();
        }
        out.push((text, start..at));
    }
    out
}

/// `$ cmd` or `% cmd` → `cmd`, and with `bang` Claude Code's `! cmd` too
/// (in a shell fence `!` is the shell's own); a bare `$` is an empty
/// command.
pub fn prompt_off(line: &str, bang: bool) -> Option<&str> {
    if line == "$" || line == "%" {
        return Some("");
    }
    line.strip_prefix("$ ")
        .or_else(|| line.strip_prefix("% "))
        .or_else(|| line.strip_prefix("! ").filter(|_| bang))
        .map(str::trim_start)
}

/// The programs whose name leading a line or a code span outside a shell
/// fence makes it a command (T-1141).
const PROGRAMS: &[&str] = &[
    "apt",
    "apt-get",
    "aws",
    "bash",
    "brew",
    "bun",
    "cargo",
    "cat",
    "cd",
    "chmod",
    "chown",
    "claude",
    "cmake",
    "code",
    "codex",
    "cp",
    "curl",
    "deno",
    "df",
    "diff",
    "docker",
    "du",
    "echo",
    "env",
    "export",
    "fd",
    "find",
    "gcloud",
    "gh",
    "git",
    "go",
    "grep",
    "head",
    "helm",
    "jq",
    "just",
    "kill",
    "kubectl",
    "less",
    "ln",
    "ls",
    "make",
    "man",
    "mesimon",
    "mkdir",
    "mv",
    "node",
    "npm",
    "npx",
    "nvim",
    "open",
    "pip",
    "pip3",
    "pkill",
    "pnpm",
    "printf",
    "ps",
    "psql",
    "python",
    "python3",
    "rg",
    "rm",
    "rmdir",
    "rsync",
    "rustc",
    "rustup",
    "scp",
    "sed",
    "sh",
    "source",
    "sqlite3",
    "ssh",
    "sudo",
    "tail",
    "tar",
    "terraform",
    "tmux",
    "touch",
    "uv",
    "vim",
    "wget",
    "which",
    "xdg-open",
    "yarn",
    "zip",
    "unzip",
    "zsh",
    "awk",
];

/// Is `line` led by a known program, past any `NAME=value` assignments?
pub fn led_by_program(line: &str) -> bool {
    let assignment = |w: &str| {
        w.split_once('=').is_some_and(|(name, _)| {
            name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    line.split_whitespace().find(|w| !assignment(w)).is_some_and(|w| PROGRAMS.contains(&w))
}

/// The command an inline code span holds, if it holds one: a prompted span
/// (`$ `, `% `, `! `) past its prompt, or a span led by a known program and
/// given an argument (`cat ~/a.txt`, never a bare `git`).
pub fn inline_command(span: &str) -> Option<String> {
    let s = span.trim();
    if s.contains('\n') {
        return None;
    }
    let command = match prompt_off(s, true) {
        Some(rest) => rest,
        None if led_by_program(s) && s.split_whitespace().count() > 1 => s,
        None => return None,
    };
    (!command.is_empty()).then(|| command.to_string())
}

/// Does the shell read on past this line?
fn runs_on(line: &str) -> bool {
    let l = line.trim_end();
    (l.ends_with('\\') && !l.ends_with("\\\\")) || l.ends_with('|') || l.ends_with("&&")
}

/// The terminator of a heredoc this line opens: `<<EOF`, `<<-EOF`,
/// `<< 'EOF'`, `<<"EOF"`. A here-string (`<<<`) opens none, nor a `<<`
/// inside quotes or arithmetic (`$((1 << 3))`), nor one whose word does not
/// start as a name does (`<< 3`).
fn heredoc(line: &str) -> Option<String> {
    let mut quote = None;
    let mut arith = 0usize;
    let mut chars = line.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(') if line[at + 1..].starts_with('(') => {
                arith += 1;
                chars.next();
            }
            (None, ')') if arith > 0 && line[at + 1..].starts_with(')') => {
                arith -= 1;
                chars.next();
            }
            (None, '<') if arith == 0 && line[at + 1..].starts_with('<') => {
                let rest = &line[at + 2..];
                if rest.starts_with('<') {
                    return None;
                }
                let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
                let rest = rest.trim_start_matches(['\'', '"']);
                if !rest.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                    return None;
                }
                let word: String =
                    rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                return Some(word);
            }
            _ => {}
        }
    }
    None
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
    let label = label.trim();
    let link = classify_target(&raw)
        .map(|target| Link { label: (!label.is_empty()).then(|| label.to_string()), target });
    Some((link, end + 1))
}

/// What a markdown link's `(target)` points at, if anything: the first
/// word of it, its `<angle>` brackets off, classified as a word of prose
/// is. A target broken across lines is no link, as `extract` reads it.
pub fn classify_target(raw: &str) -> Option<Found> {
    if raw.contains('\n') {
        return None;
    }
    let target = raw.split_whitespace().next().unwrap_or("");
    let target = target.strip_prefix('<').and_then(|t| t.strip_suffix('>')).unwrap_or(target);
    classify(target)
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

    fn command(s: &str) -> Found {
        Found::Command(s.into())
    }

    #[test]
    fn a_shell_fence_holds_one_command_a_line() {
        let body = "Run:\n\n```bash\n# build first\n$ cargo ut\n\ncargo build --workspace\n```\n\nthen `cargo ut` again";
        assert_eq!(targets(body), vec![command("cargo ut"), command("cargo build --workspace")]);
        // `console` is a session: only a prompted line is a command.
        let session = "```console\n$ ls src/\nmain.rs lib.rs\n$ echo hi\nhi\n```";
        assert_eq!(targets(session), vec![command("ls src/"), command("echo hi")]);
        // Indented in a list item, tagged with attributes, tildes.
        assert_eq!(targets("- x\n  ```sh {.copy}\n  make\n  ```"), vec![command("make")]);
        assert_eq!(targets("~~~zsh\nmake\n~~~"), vec![command("make")]);
    }

    #[test]
    fn a_command_keeps_the_rows_it_runs_on() {
        let body = "```sh\ncargo test -p x \\\n  --test hook_e2e\nls |\n  wc -l\ncat <<'EOF' > a.txt\none\n  two\nEOF\necho done\n```";
        assert_eq!(
            targets(body),
            vec![
                command("cargo test -p x \\\n--test hook_e2e"),
                command("ls |\nwc -l"),
                command("cat <<'EOF' > a.txt\none\n  two\nEOF"),
                command("echo done"),
            ]
        );
        let rows: Vec<&str> = body.lines().skip(1).take(9).collect();
        let spans: Vec<_> = commands("sh", &rows).into_iter().map(|(_, r)| r).collect();
        assert_eq!(spans, vec![0..2, 2..4, 4..8, 8..9]);
        // A here-string is one line.
        assert_eq!(targets("```bash\ngrep x <<< \"$v\"\nls\n```").len(), 2);
        // A shift in arithmetic or a `<<` in quotes opens no heredoc.
        for line in ["echo $((1<<3))", "echo $((1 << 3))", "echo \"a << b\"", "let x=1<<3"] {
            let fence = format!("```bash\n{line}\nmake\n```");
            assert_eq!(targets(&fence), vec![command(line), command("make")], "{line}");
        }
        assert_eq!(
            targets("```sh\ncat <<-\"END\"\nx\nEND\nls\n```"),
            vec![command("cat <<-\"END\"\nx\nEND"), command("ls")]
        );
    }

    #[test]
    fn a_shell_fence_holds_commands_and_another_language_none() {
        assert_eq!(targets("```rust\nlet x = 1;\ncat(a, b);\n```"), vec![]);
        // A shell fence's words are its commands, not paths; prose around
        // it keeps its links, in document order; an unterminated fence runs
        // to the end, as the draw shows it.
        assert_eq!(
            targets("see T-4\n```bash\npython3 ci/rig.py\n```\nand docs/a.md\n```sh\nmake"),
            vec![
                Found::Ticket("T-4".into()),
                command("python3 ci/rig.py"),
                path("docs/a.md", None),
                command("make"),
            ]
        );
        // A fence inside another (a longer run) is its row, not a fence.
        assert_eq!(targets("````md\n```bash\nmake\n```\n````"), vec![]);
        assert_eq!(fence_open("```bash title=x"), Some(('`', 3, "bash")));
        assert!(fence_closes("  ````  ", '`', 3) && !fence_closes("``", '`', 3));
    }

    #[test]
    fn an_untagged_fence_holds_its_prompted_and_program_led_lines() {
        // The agent's `! ` comes off; the line's path is the command's.
        let body = "Run:\n\n```\n! bash /tmp/s/reset.sh\n```\n";
        assert_eq!(targets(body), vec![command("bash /tmp/s/reset.sh")]);
        // Its other rows are prose, so a tree's path is a path; a program's
        // output line (`cat: x`) and code are not commands.
        let body = "```\nsrc/a.rs\nRUST_LOG=1 cargo ut \\\n  -q\ncat: x: No such file\n$ make\nfoo(bar)\n```";
        assert_eq!(
            targets(body),
            vec![path("src/a.rs", None), command("RUST_LOG=1 cargo ut \\\n-q"), command("make")]
        );
        // In a shell fence `!` is the shell's own.
        assert_eq!(targets("```sh\n! grep -q x a\n```"), vec![command("! grep -q x a")]);
    }

    #[test]
    fn a_code_span_is_a_command_when_prompted_or_led_by_a_program() {
        let body = "Then `cat ~/.cache/s/owner.pw`, type it in as `a@b.test`, run `$ make up`.";
        assert_eq!(targets(body), vec![command("cat ~/.cache/s/owner.pw"), command("make up")]);
        // A bare program, an identifier, a path: not commands; a path stays a path.
        assert_eq!(
            targets("use `git`, call `find(x)`, edit `src/a.rs:3` or ``ls `x` ``"),
            vec![path("src/a.rs", Some(3)), command("ls `x`")]
        );
        // Its order is the document's, and an unclosed tick is a word's.
        assert_eq!(
            targets("T-1 `git status` T-2 `oops docs/a.md"),
            vec![
                Found::Ticket("T-1".into()),
                command("git status"),
                Found::Ticket("T-2".into()),
                path("docs/a.md", None),
            ]
        );
        // `words` reads a span as words.
        assert_eq!(
            words("`cat ~/a.txt`").into_iter().map(|l| l.target).collect::<Vec<_>>(),
            vec![path("~/a.txt", None)]
        );
    }

    #[test]
    fn words_read_a_shell_fence_as_prose() {
        let body = "```bash\nopen docs/a.md\n```";
        assert_eq!(targets(body), vec![command("open docs/a.md")]);
        let words: Vec<Found> = words(body).into_iter().map(|l| l.target).collect();
        assert_eq!(words, vec![path("docs/a.md", None)]);
    }

    #[test]
    fn text_is_the_absence_of_nul() {
        assert!(looks_text(b""));
        assert!(looks_text("fn main() {}\n".as_bytes()));
        assert!(!looks_text(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
    }
}
