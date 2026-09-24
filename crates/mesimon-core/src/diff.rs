//! Read-only diff model + parsers (M4b, docs/08 §1.2–1.3).
//!
//! Pure: bytes in, structs out. Every parser takes the raw `-z` output and
//! splits on NUL *before* decoding — git octal-escapes non-ASCII paths unless
//! `-z` is given, and a lossy decode-then-split would corrupt the record
//! stream on paths containing what lossy decoding mangles.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// One row of the file list (`DiffList`). Stable entries come from
/// `diff --raw -z`; untracked-only files appear as extra rows with an empty
/// status (display only — `git diff` cannot see a file the agent never added,
/// docs/08 §2, and that is exactly the change the reviewer least wants to miss).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    /// Some(..) iff status == "R" (the pre-rename path).
    #[serde(default)]
    pub old_path: Option<String>,
    /// "A" "M" "D" "R" "T" — or "" for an in-flight-only (untracked) row,
    /// which is display-only on a branch diff because `git diff` cannot see a
    /// file the agent never added. The checkout diff (T-221) stamps those rows
    /// "A" instead and serves them from `--no-index`, so an empty status still
    /// means exactly "there is no patch behind this row".
    pub status: String,
    #[serde(default)]
    pub old_mode: String,
    #[serde(default)]
    pub new_mode: String,
    /// 40-hex (`--abbrev=40` is load-bearing: raw output abbreviates by default).
    #[serde(default)]
    pub old_blob: String,
    #[serde(default)]
    pub new_blob: String,
    /// None = binary (numstat `-\t-`) or numstat absent.
    #[serde(default)]
    pub adds: Option<u32>,
    #[serde(default)]
    pub dels: Option<u32>,
    /// Uncommitted edits in the worktree touch this path (porcelain v2 1/2/u).
    #[serde(default)]
    pub dirty: bool,
    #[serde(default)]
    pub untracked: bool,
}

/// The per-file payload (`DiffFile`): the hunk model the renderer reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    #[serde(default)]
    pub old_path: Option<String>,
    pub render: Render,
    /// Non-empty for `Text` (and `Symlink`, whose hunks carry the targets).
    #[serde(default)]
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// The text after the second `@@`.
    pub header: String,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HunkLine {
    pub sign: Sign,
    #[serde(default)]
    pub old_ln: Option<u32>,
    #[serde(default)]
    pub new_ln: Option<u32>,
    /// Raw line body — tabs and a lone `\r` preserved; expanding tabs and
    /// rendering `^M` is the renderer's job.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sign {
    Ctx,
    Add,
    Del,
}

/// Exhaustive — the list of things a hunk browser meets on day one; a skipped
/// case is a day-one panic (docs/08 §1.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Render {
    Text,
    Binary,
    ModeOnly {
        old_mode: String,
        new_mode: String,
    },
    Symlink,
    Submodule {
        old_oid: String,
        new_oid: String,
    },
    TooLarge {
        bytes: u64,
    },
    /// The per-file git call exited non-zero: stderr verbatim.
    Unresolvable {
        message: String,
    },
}

/// Split `-z` output into NUL-terminated tokens (the trailing empty token from
/// a terminating NUL is dropped).
fn nul_tokens(out: &[u8]) -> Vec<&[u8]> {
    let mut toks: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    while toks.last().is_some_and(|t| t.is_empty()) {
        toks.pop();
    }
    toks
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `diff --raw -z --abbrev=40 --find-renames --no-ext-diff BASE...BRANCH`.
///
/// Record shape: `:<old_mode> <new_mode> <old_oid> <new_oid> <status>` NUL
/// `<path>` NUL — and for R/C a second path token (source first, dest second).
pub fn parse_raw_z(out: &[u8]) -> Vec<FileEntry> {
    let toks = nul_tokens(out);
    let mut files = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let meta = toks[i];
        i += 1;
        if meta.first() != Some(&b':') {
            continue; // desynced or unexpected token — skip, never panic
        }
        let meta = decode(&meta[1..]);
        let mut f = meta.split_ascii_whitespace();
        let (Some(old_mode), Some(new_mode), Some(old_blob), Some(new_blob), Some(st)) =
            (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        // Status letter with optional score: "M", "R100", "C75", "T"…
        let status: String = st.chars().take(1).collect();
        let Some(path1) = toks.get(i) else { break };
        i += 1;
        let (path, old_path) = if matches!(status.as_str(), "R" | "C") {
            // Source first, destination second.
            let Some(path2) = toks.get(i) else { break };
            i += 1;
            (decode(path2), Some(decode(path1)))
        } else {
            (decode(path1), None)
        };
        files.push(FileEntry {
            path,
            old_path,
            status,
            old_mode: old_mode.to_string(),
            new_mode: new_mode.to_string(),
            old_blob: old_blob.to_string(),
            new_blob: new_blob.to_string(),
            adds: None,
            dels: None,
            dirty: false,
            untracked: false,
        });
    }
    files
}

/// `diff --numstat -z`: `(path, Some((adds, dels)) | None-for-binary)`.
/// Rename records are `adds TAB dels TAB` NUL `old` NUL `new` NUL — keyed by
/// the destination path.
pub fn parse_numstat_z(out: &[u8]) -> Vec<(String, Option<(u32, u32)>)> {
    let toks = nul_tokens(out);
    let mut rows = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i];
        i += 1;
        let mut parts = tok.splitn(3, |b| *b == b'\t');
        let (Some(a), Some(d), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let counts = match (decode(a).parse::<u32>(), decode(d).parse::<u32>()) {
            (Ok(a), Ok(d)) => Some((a, d)),
            _ => None, // "-\t-" = binary
        };
        let path = if rest.is_empty() {
            // Rename: consume the two path tokens, keep the destination.
            let _old = toks.get(i);
            let new = toks.get(i + 1);
            i += 2;
            match new {
                Some(p) => decode(p),
                None => break,
            }
        } else {
            decode(rest)
        };
        rows.push((path, counts));
    }
    rows
}

#[derive(Debug, Default, Clone)]
pub struct StatusFlags {
    pub dirty: std::collections::HashSet<String>,
    pub untracked: Vec<String>,
}

/// `status --porcelain=v2 -unormal -z`. Only display flags: which tracked
/// paths carry uncommitted edits (`1`/`2`/`u` records) and which files exist
/// untracked (`?`). A `2` (rename) record carries a second NUL-separated
/// path — it must be consumed or every following record desyncs.
pub fn parse_status_v2_z(out: &[u8]) -> StatusFlags {
    let toks = nul_tokens(out);
    let mut flags = StatusFlags::default();
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i];
        i += 1;
        let (kind, n_fields) = match tok.first() {
            Some(b'1') => ('1', 8),
            Some(b'2') => ('2', 9),
            Some(b'u') => ('u', 10),
            Some(b'?') => ('?', 1),
            _ => continue, // '#' headers, '!' ignored, unknown
        };
        // The path is everything after the fixed field count's spaces.
        let mut rest: &[u8] = tok;
        for _ in 0..n_fields {
            match rest.iter().position(|b| *b == b' ') {
                Some(p) => rest = &rest[p + 1..],
                None => {
                    rest = b"";
                    break;
                }
            }
        }
        if rest.is_empty() {
            continue;
        }
        let path = decode(rest);
        match kind {
            '?' => flags.untracked.push(path),
            '2' => {
                flags.dirty.insert(path);
                i += 1; // the origPath token
            }
            _ => {
                flags.dirty.insert(path);
            }
        }
    }
    flags
}

/// Parse a per-file unified diff into hunks. Returns the hunks and whether a
/// binary marker (`Binary files … differ` / `GIT binary patch`) was seen.
pub fn parse_patch(out: &[u8]) -> (Vec<Hunk>, bool) {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut binary = false;
    let mut old_ln = 0u32;
    let mut new_ln = 0u32;
    for raw in out.split(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(raw);
        if let Some(h) = parse_hunk_header(&line) {
            old_ln = h.old_start;
            new_ln = h.new_start;
            hunks.push(h);
            continue;
        }
        let Some(cur) = hunks.last_mut() else {
            // Header region.
            if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                binary = true;
            }
            continue;
        };
        let mut chars = line.chars();
        match chars.next() {
            Some(' ') => {
                cur.lines.push(HunkLine {
                    sign: Sign::Ctx,
                    old_ln: Some(old_ln),
                    new_ln: Some(new_ln),
                    text: chars.as_str().to_string(),
                });
                old_ln += 1;
                new_ln += 1;
            }
            Some('+') => {
                cur.lines.push(HunkLine {
                    sign: Sign::Add,
                    old_ln: None,
                    new_ln: Some(new_ln),
                    text: chars.as_str().to_string(),
                });
                new_ln += 1;
            }
            Some('-') => {
                cur.lines.push(HunkLine {
                    sign: Sign::Del,
                    old_ln: Some(old_ln),
                    new_ln: None,
                    text: chars.as_str().to_string(),
                });
                old_ln += 1;
            }
            // "\ No newline at end of file" — a marker, not a content line.
            Some('\\') => {}
            // Anything else ("diff --git", "index …") ends the hunk region.
            _ => {}
        }
    }
    (hunks, binary)
}

/// `@@ -18,7 +18,9 @@ header text` (lengths default to 1 when omitted).
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let close = rest.find(" @@")?;
    let (ranges, tail) = rest.split_at(close);
    let header = tail[3..].strip_prefix(' ').unwrap_or(&tail[3..]).to_string();
    let (old_part, new_part) = ranges.split_once(" +")?;
    let parse_range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = parse_range(old_part)?;
    let (new_start, new_len) = parse_range(new_part)?;
    Some(Hunk { old_start, old_len, new_start, new_len, header, lines: Vec::new() })
}

/// Classify + assemble one file's diff from its list entry and patch bytes.
pub fn build_file_diff(entry: &FileEntry, patch: &[u8], max_bytes: u64) -> FileDiff {
    let base = FileDiff {
        path: entry.path.clone(),
        old_path: entry.old_path.clone(),
        render: Render::Text,
        hunks: Vec::new(),
    };
    if patch.len() as u64 > max_bytes {
        return FileDiff { render: Render::TooLarge { bytes: patch.len() as u64 }, ..base };
    }
    if entry.old_mode == "160000" || entry.new_mode == "160000" {
        return FileDiff {
            render: Render::Submodule {
                old_oid: entry.old_blob.clone(),
                new_oid: entry.new_blob.clone(),
            },
            ..base
        };
    }
    let (hunks, binary_marker) = parse_patch(patch);
    if entry.old_mode == "120000" || entry.new_mode == "120000" {
        // The hunks carry the link targets, one line each side.
        return FileDiff { render: Render::Symlink, hunks, ..base };
    }
    // The marker is sufficient: a non-text file always prints
    // `Binary files … differ` under --no-color without --binary. A numstat
    // `-\t-` alone is NOT usable here — a pure rename also has no counts and
    // an empty patch, and must stay Text (zero hunks).
    if binary_marker {
        return FileDiff { render: Render::Binary, ..base };
    }
    // A mode change and nothing else. 08 §1.3 spelled this as blob equality,
    // which only holds on a commit-to-commit range: `git diff --raw HEAD`
    // writes forty zeros for the destination blob whenever the worktree file's
    // stat differs from the index, so a worktree-only `chmod +x` could never
    // match and rendered as an empty Text diff (T-221). The patch says it
    // directly instead — no content, not binary, the modes differ — and both
    // modes must be real file modes: without that clause an empty new file
    // (`000000` → `100644`, no hunks), an empty deleted file, and an untracked
    // row would every one of them read as a mode change.
    let file_mode = |m: &String| !m.is_empty() && m != "000000";
    if entry.old_mode != entry.new_mode
        && file_mode(&entry.old_mode)
        && file_mode(&entry.new_mode)
        && hunks.is_empty()
    {
        return FileDiff {
            render: Render::ModeOnly {
                old_mode: entry.old_mode.clone(),
                new_mode: entry.new_mode.clone(),
            },
            ..base
        };
    }
    FileDiff { hunks, ..base }
}

/// The changed byte ranges of one line's `text`, in order and disjoint.
pub type Marks = Vec<Range<usize>>;

/// Past this many bytes on either side a pair keeps the whole-line treatment:
/// a line that long is minified or generated, and nobody reads it by word.
const INTRALINE_MAX_BYTES: usize = 2048;
/// How much of a pair's content may differ and still read as one line
/// edited, rather than a line deleted and an unrelated one written in its
/// place — where marking every word would be noise. delta's own default
/// (`max-line-distance`); at 0.7, `let base8 = d.base_oid;` →
/// `let against = "uncommitted";` paired and lit every word but `let`.
const INTRALINE_MAX_CHANGE: f64 = 0.6;
/// How many adds past the last paired one a delete looks at for its partner.
/// Bounds the work on a block rewritten wholesale, where nothing pairs and
/// every delete would otherwise try every add.
const INTRALINE_WINDOW: usize = 4;

/// The changed words of every paired line in one hunk (T-454), inferred
/// the way delta infers them: inside a run of deletes followed by a run of
/// adds, each delete takes the add most like it among the next
/// `INTRALINE_WINDOW` after the last paired one, if that add differs from it
/// by at most `INTRALINE_MAX_CHANGE` — so a line written above the edited
/// one is stepped over rather than paired by position. One entry
/// per line of `lines`: `None` keeps the whole-line treatment (context, an
/// unpaired line, a pair past the caps), `Some` holds byte ranges of `text`
/// that changed — empty on the side of a pure insertion or deletion, which
/// is still a paired line.
pub fn intraline(lines: &[HunkLine]) -> Vec<Option<Marks>> {
    let mut out = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].sign != Sign::Del {
            i += 1;
            continue;
        }
        let dels = i;
        while i < lines.len() && lines[i].sign == Sign::Del {
            i += 1;
        }
        let adds = i;
        while i < lines.len() && lines[i].sign == Sign::Add {
            i += 1;
        }
        // Each line is cut into words once; a delete weighs up to
        // `INTRALINE_WINDOW` adds and an add is weighed by as many deletes.
        let side: Vec<Option<Words>> = lines[dels..i].iter().map(|l| Words::of(&l.text)).collect();
        let mut next = adds;
        for d in dels..adds {
            let Some(old) = &side[d - dels] else { continue };
            let best = (next..i.min(next + INTRALINE_WINDOW))
                .filter_map(|a| {
                    let new = side[a - dels].as_ref()?;
                    word_diff(old, new).map(|p| (a, p))
                })
                .min_by(|(_, x), (_, y)| x.0.total_cmp(&y.0));
            if let Some((a, (_, old, new))) = best {
                out[d] = Some(old);
                out[a] = Some(new);
                next = a + 1;
            }
        }
    }
    out
}

/// One side of a pair, cut into words.
struct Words<'a> {
    text: &'a str,
    /// Byte ranges of `text`, in order.
    spans: Vec<Range<usize>>,
    words: Vec<&'a str>,
    /// `words` sorted, for the bound in `word_diff`.
    sorted: Vec<&'a str>,
    /// Bytes of leading whitespace. Indentation makes no two lines alike, so
    /// the ratio is over the content after it: two unrelated lines at one
    /// depth would otherwise pair.
    indent: usize,
}

impl<'a> Words<'a> {
    /// `None` past `INTRALINE_MAX_BYTES`.
    fn of(text: &'a str) -> Option<Self> {
        if text.len() > INTRALINE_MAX_BYTES {
            return None;
        }
        let spans = tokens(text);
        let words: Vec<&str> = spans.iter().map(|r| &text[r.clone()]).collect();
        let mut sorted = words.clone();
        sorted.sort_unstable();
        Some(Words { text, spans, words, sorted, indent: text.len() - text.trim_start().len() })
    }

    fn content(&self) -> usize {
        self.text.len() - self.indent
    }

    /// Bytes of `marks` past the indentation.
    fn changed(&self, marks: &[Range<usize>]) -> usize {
        marks.iter().map(|r| r.end.max(self.indent) - r.start.max(self.indent)).sum()
    }
}

/// One pair's share of changed content and its changed byte ranges, or
/// `None` past the ratio.
fn word_diff(old: &Words, new: &Words) -> Option<(f64, Marks, Marks)> {
    let content = old.content() + new.content();
    if content == 0 {
        return None;
    }
    // A bound before the diff: only a word both sides hold can come out
    // equal, so a pair sharing too few bytes of them cannot pass the ratio
    // whatever Myers finds. On a block rewritten wholesale — where nothing
    // pairs and each delete tries `INTRALINE_WINDOW` adds — this is most of
    // the work saved.
    let (mut x, mut y, mut common) = (0, 0, 0);
    while x < old.sorted.len() && y < new.sorted.len() {
        match old.sorted[x].cmp(new.sorted[y]) {
            std::cmp::Ordering::Less => x += 1,
            std::cmp::Ordering::Greater => y += 1,
            std::cmp::Ordering::Equal => {
                common += old.sorted[x].len();
                x += 1;
                y += 1;
            }
        }
    }
    if content.saturating_sub(2 * common) as f64 > INTRALINE_MAX_CHANGE * content as f64 {
        return None;
    }
    let mut om = Vec::new();
    let mut nm = Vec::new();
    for op in similar::capture_diff_slices(similar::Algorithm::Myers, &old.words, &new.words) {
        if let similar::DiffOp::Equal { .. } = op {
            continue;
        }
        let (o, n) = (op.old_range(), op.new_range());
        if !o.is_empty() {
            om.push(old.spans[o.start].start..old.spans[o.end - 1].end);
        }
        if !n.is_empty() {
            nm.push(new.spans[n.start].start..new.spans[n.end - 1].end);
        }
    }
    let (om, nm) = (merge_gaps(old.text, om), merge_gaps(new.text, nm));
    let change = (old.changed(&om) + new.changed(&nm)) as f64 / content as f64;
    (change <= INTRALINE_MAX_CHANGE).then_some((change, om, nm))
}

/// A line cut where a reader's eye cuts it: a run of word characters, a run
/// of whitespace, or one character of anything else — so `get(code)` →
/// `get(code, v)` marks `, v` and not the whole call.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            2
        } else {
            0
        }
    };
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut prev = None;
    for (at, c) in s.char_indices() {
        let k = class(c);
        match out.last_mut() {
            Some(r) if k != 0 && prev == Some(k) => r.end = at + c.len_utf8(),
            _ => out.push(at..at + c.len_utf8()),
        }
        prev = Some(k);
    }
    out
}

/// Two marks with only whitespace between them read as one change, so they
/// are drawn as one: `a b` → `c d` marks `c d`, not `c`, a gap, and `d`.
fn merge_gaps(s: &str, marks: Marks) -> Marks {
    let mut out: Marks = Vec::with_capacity(marks.len());
    for m in marks {
        match out.last_mut() {
            Some(last) if s[last.end..m.start].chars().all(char::is_whitespace) => last.end = m.end,
            _ => out.push(m),
        }
    }
    out
}

/// Fold numstat counts into the raw-z entries (keyed by destination path).
pub fn merge_numstat(files: &mut [FileEntry], numstat: &[(String, Option<(u32, u32)>)]) {
    for f in files.iter_mut() {
        if let Some((_, Some((a, d)))) = numstat.iter().find(|(p, _)| *p == f.path) {
            f.adds = Some(*a);
            f.dels = Some(*d);
        }
    }
}

/// Set dirty flags on stable entries and append untracked-only rows.
///
/// The stable rows are indexed once rather than scanned per untracked path:
/// a worktree listed with `-unormal` has a handful of those, but the board's
/// own checkout is listed with `-uall` (T-221) and a repository missing a
/// `.gitignore` rule has tens of thousands — against a `Vec` that grows as
/// this pushes.
pub fn apply_status_flags(files: &mut Vec<FileEntry>, flags: &StatusFlags) {
    let mut at: std::collections::HashMap<String, usize> =
        files.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
    for f in files.iter_mut() {
        if flags.dirty.contains(&f.path) {
            f.dirty = true;
        }
    }
    for path in &flags.untracked {
        if let Some(i) = at.get(path) {
            files[*i].untracked = true;
            continue;
        }
        at.insert(path.clone(), files.len());
        files.push(FileEntry {
            path: path.clone(),
            old_path: None,
            status: String::new(),
            old_mode: String::new(),
            new_mode: String::new(),
            old_blob: String::new(),
            new_blob: String::new(),
            adds: None,
            dels: None,
            dirty: false,
            untracked: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OID_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn entry(status: &str, old_mode: &str, new_mode: &str) -> FileEntry {
        FileEntry {
            path: "src/x.rs".into(),
            old_path: None,
            status: status.into(),
            old_mode: old_mode.into(),
            new_mode: new_mode.into(),
            old_blob: OID_A.into(),
            new_blob: OID_B.into(),
            adds: Some(1),
            dels: Some(1),
            dirty: false,
            untracked: false,
        }
    }

    #[test]
    fn raw_z_plain_modify() {
        let rec = format!(":100644 100644 {OID_A} {OID_B} M\0src/x.rs\0");
        let files = parse_raw_z(rec.as_bytes());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "M");
        assert_eq!(files[0].path, "src/x.rs");
        assert_eq!(files[0].old_blob.len(), 40, "--abbrev=40 is load-bearing");
        assert_eq!(files[0].old_path, None);
    }

    #[test]
    fn raw_z_rename_two_paths() {
        let rec = format!(":100644 100644 {OID_A} {OID_B} R100\0old/name.rs\0new/name.rs\0");
        let files = parse_raw_z(rec.as_bytes());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].path, "new/name.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn raw_z_typechange_and_following_record_stays_synced() {
        let rec = format!(
            ":100644 120000 {OID_A} {OID_B} T\0link.txt\0:100644 100644 {OID_A} {OID_B} M\0b.rs\0"
        );
        let files = parse_raw_z(rec.as_bytes());
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "T");
        assert_eq!(files[1].path, "b.rs");
    }

    #[test]
    fn raw_z_non_ascii_path_survives_verbatim() {
        // -z output carries the raw UTF-8 bytes, no octal quoting.
        let mut rec = format!(":000000 100644 {OID_A} {OID_B} A\0").into_bytes();
        rec.extend_from_slice("מסמך.txt".as_bytes());
        rec.push(0);
        let files = parse_raw_z(&rec);
        assert_eq!(files[0].path, "מסמך.txt");
        // Without -z git would emit "\327\236..." — which is NOT what we parse:
        let quoted = format!(":000000 100644 {OID_A} {OID_B} A\0\"\\327\\236\\327\\241\"\0");
        let files = parse_raw_z(quoted.as_bytes());
        assert_ne!(files[0].path, "מסמך.txt");
    }

    #[test]
    fn numstat_counts_and_binary() {
        let out = b"3\t1\tsrc/x.rs\0-\t-\timg/logo.png\0";
        let rows = parse_numstat_z(out);
        assert_eq!(rows[0], ("src/x.rs".into(), Some((3, 1))));
        assert_eq!(rows[1], ("img/logo.png".into(), None));
    }

    #[test]
    fn numstat_rename_keyed_by_destination() {
        let out = b"2\t0\t\0old/name.rs\0new/name.rs\x004\t4\tafter.rs\0";
        let rows = parse_numstat_z(out);
        assert_eq!(rows[0], ("new/name.rs".into(), Some((2, 0))));
        assert_eq!(rows[1], ("after.rs".into(), Some((4, 4))), "record after rename stays synced");
    }

    #[test]
    fn status_v2_dirty_untracked_and_rename_second_path() {
        let out = format!(
            "1 .M N... 100644 100644 100644 {OID_A} {OID_A} src/x.rs\0\
             2 R. N... 100644 100644 100644 {OID_A} {OID_A} R100 new.rs\0orig.rs\0\
             ? scratch.txt\0\
             u UU N... 100644 100644 100644 100644 {OID_A} {OID_A} {OID_A} conflicted.rs\0"
        );
        let flags = parse_status_v2_z(out.as_bytes());
        assert!(flags.dirty.contains("src/x.rs"));
        assert!(flags.dirty.contains("new.rs"));
        assert!(flags.dirty.contains("conflicted.rs"), "u record after rename stays synced");
        assert!(
            !flags.dirty.contains("orig.rs"),
            "origPath token consumed, not parsed as a record"
        );
        assert_eq!(flags.untracked, vec!["scratch.txt".to_string()]);
    }

    #[test]
    fn patch_multi_hunk_numbering() {
        let patch = b"diff --git a/x b/x\nindex aaa..bbb 100644\n--- a/x\n+++ b/x\n\
@@ -18,3 +18,4 @@ fn handle() {\n line18\n-old19\n+new19\n+new20\n line20\n\
@@ -41,2 +43,2 @@ fn persist() {\n line41\n+metrics\n";
        let (hunks, binary) = parse_patch(patch);
        assert!(!binary);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header, "fn handle() {");
        let l = &hunks[0].lines;
        assert_eq!((l[0].sign, l[0].old_ln, l[0].new_ln), (Sign::Ctx, Some(18), Some(18)));
        assert_eq!((l[1].sign, l[1].old_ln, l[1].new_ln), (Sign::Del, Some(19), None));
        assert_eq!((l[2].sign, l[2].old_ln, l[2].new_ln), (Sign::Add, None, Some(19)));
        assert_eq!((l[3].sign, l[3].old_ln, l[3].new_ln), (Sign::Add, None, Some(20)));
        assert_eq!((l[4].sign, l[4].old_ln, l[4].new_ln), (Sign::Ctx, Some(20), Some(21)));
        assert_eq!(hunks[1].lines[1].new_ln, Some(44));
    }

    #[test]
    fn patch_no_newline_marker_skipped_and_cr_preserved() {
        let patch =
            b"--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-old\r\n+new\n\\ No newline at end of file\n";
        let (hunks, _) = parse_patch(patch);
        assert_eq!(hunks[0].lines.len(), 2, "the backslash marker is not a content line");
        assert_eq!(hunks[0].lines[0].text, "old\r", "lone \\r preserved for the renderer's ^M");
    }

    #[test]
    fn patch_binary_marker() {
        let (hunks, binary) = parse_patch(b"diff --git a/l b/l\nBinary files a/l and b/l differ\n");
        assert!(binary);
        assert!(hunks.is_empty());
    }

    #[test]
    fn hunk_header_len_defaults_to_one() {
        let h = parse_hunk_header("@@ -5 +7 @@").expect("bare-count hunk header parses");
        assert_eq!((h.old_start, h.old_len, h.new_start, h.new_len), (5, 1, 7, 1));
        assert_eq!(h.header, "");
    }

    #[test]
    fn classify_too_large() {
        let fd = build_file_diff(&entry("M", "100644", "100644"), &[b'x'; 32], 16);
        assert_eq!(fd.render, Render::TooLarge { bytes: 32 });
    }

    #[test]
    fn classify_submodule() {
        let fd = build_file_diff(&entry("M", "160000", "160000"), b"", 1024);
        assert_eq!(fd.render, Render::Submodule { old_oid: OID_A.into(), new_oid: OID_B.into() });
    }

    #[test]
    fn classify_symlink_keeps_target_hunks() {
        let patch = b"--- a/l\n+++ b/l\n@@ -1,1 +1,1 @@\n-/old/target\n+/new/target\n";
        let fd = build_file_diff(&entry("M", "120000", "120000"), patch, 1024);
        assert_eq!(fd.render, Render::Symlink);
        assert_eq!(fd.hunks[0].lines[1].text, "/new/target");
    }

    #[test]
    fn classify_binary_by_marker_but_pure_rename_stays_text() {
        let fd = build_file_diff(
            &entry("M", "100644", "100644"),
            b"Binary files a/x and b/x differ\n",
            1024,
        );
        assert_eq!(fd.render, Render::Binary);
        // A pure rename also has no numstat counts and an empty patch — it
        // must classify Text with zero hunks, never Binary.
        let mut e = entry("R", "100644", "100644");
        e.adds = None;
        e.dels = None;
        e.old_path = Some("old.rs".into());
        let fd = build_file_diff(&e, b"", 1024);
        assert_eq!(fd.render, Render::Text);
        assert!(fd.hunks.is_empty());
    }

    #[test]
    fn classify_mode_only() {
        let mut e = entry("M", "100644", "100755");
        e.new_blob = OID_A.into(); // same content, chmod only
        let fd = build_file_diff(&e, b"old mode 100644\nnew mode 100755\n", 1024);
        assert_eq!(
            fd.render,
            Render::ModeOnly { old_mode: "100644".into(), new_mode: "100755".into() }
        );
    }

    /// The T-221 bug: `git diff --raw HEAD` writes forty zeros for the
    /// destination blob whenever the worktree file's stat differs from the
    /// index, so blob equality could never answer here and a worktree-only
    /// chmod rendered as an empty Text diff.
    #[test]
    fn classify_mode_only_when_the_new_blob_is_zeroed() {
        let mut e = entry("M", "100644", "100755");
        e.old_blob = OID_A.into();
        e.new_blob = "0".repeat(40);
        let fd = build_file_diff(&e, b"old mode 100644\nnew mode 100755\n", 1024);
        assert_eq!(
            fd.render,
            Render::ModeOnly { old_mode: "100644".into(), new_mode: "100755".into() }
        );
    }

    /// The `000000` guard, which is what keeps the rule above from swallowing
    /// three other shapes that also have differing modes and no hunks.
    #[test]
    fn an_empty_new_file_is_not_a_mode_change() {
        let mut e = entry("A", "000000", "100644");
        e.new_blob = OID_A.into();
        let fd = build_file_diff(&e, b"new file mode 100644\nindex 0000000..e69de29\n", 1024);
        assert_eq!(fd.render, Render::Text, "an empty add is not a chmod");

        let mut e = entry("D", "100644", "000000");
        e.old_blob = OID_A.into();
        let fd = build_file_diff(&e, b"deleted file mode 100644\nindex e69de29..0000000\n", 1024);
        assert_eq!(fd.render, Render::Text, "an empty delete is not a chmod");
    }

    #[test]
    fn an_untracked_row_is_not_a_mode_change() {
        // The row `apply_status_flags` appends: every mode and blob empty.
        let e = entry("", "", "");
        assert_eq!(build_file_diff(&e, b"", 1024).render, Render::Text);
    }

    #[test]
    fn status_flags_fold_in_linear_time() {
        // 5k stable rows and 5k untracked paths, half of them already listed.
        let mut files: Vec<FileEntry> = (0..5000)
            .map(|i| FileEntry { path: format!("src/f{i}.rs"), ..entry("M", "100644", "100644") })
            .collect();
        let flags = StatusFlags {
            dirty: (0..5000).map(|i| format!("src/f{i}.rs")).collect(),
            untracked: (2500..7500).map(|i| format!("src/f{i}.rs")).collect(),
        };
        apply_status_flags(&mut files, &flags);
        assert_eq!(files.len(), 7500, "only the unlisted paths are appended");
        assert!(files[0].dirty && !files[0].untracked);
        assert!(files[2500].dirty && files[2500].untracked, "a listed path is flagged, not pushed");
        assert_eq!(files[7499].status, "", "the appended rows keep the display-only shape");
        assert!(files[7499].untracked);
    }

    #[test]
    fn classify_text() {
        let patch = b"--- a/x\n+++ b/x\n@@ -1,1 +1,2 @@\n line\n+added\n";
        let fd = build_file_diff(&entry("M", "100644", "100644"), patch, 1024);
        assert_eq!(fd.render, Render::Text);
        assert_eq!(fd.hunks.len(), 1);
    }

    fn hunk(lines: &[(Sign, &str)]) -> Vec<HunkLine> {
        lines
            .iter()
            .map(|(sign, text)| HunkLine {
                sign: *sign,
                old_ln: None,
                new_ln: None,
                text: text.to_string(),
            })
            .collect()
    }

    /// The marked text of every line, for assertions a reader can check.
    fn marked(lines: &[HunkLine]) -> Vec<Option<Vec<String>>> {
        intraline(lines)
            .into_iter()
            .zip(lines)
            .map(|(m, l)| m.map(|m| m.into_iter().map(|r| l.text[r].to_string()).collect()))
            .collect()
    }

    #[test]
    fn intraline_marks_the_changed_words_of_a_pair() {
        let lines = hunk(&[
            (Sign::Del, "  const t = await exchange(code)"),
            (Sign::Add, "  const t = await exchange(code, verifier)"),
        ]);
        assert_eq!(marked(&lines), vec![Some(vec![]), Some(vec![", verifier".to_string()])]);
        let lines =
            hunk(&[(Sign::Del, "let n = d.files.len();"), (Sign::Add, "let n = d.rows.len();")]);
        assert_eq!(marked(&lines), vec![Some(vec!["files".into()]), Some(vec!["rows".into()])]);
    }

    /// delta's pairing, not a positional one: a line written above the edited
    /// one is skipped over and stays unpaired.
    #[test]
    fn intraline_pairs_across_an_inserted_line() {
        let lines = hunk(&[
            (Sign::Ctx, "  let code = get();"),
            (Sign::Del, "  let t = exchange(code);"),
            (Sign::Add, "  let verifier = store.take(state);"),
            (Sign::Add, "  let t = exchange(code, verifier);"),
            (Sign::Ctx, "  persist(t)"),
            (Sign::Add, "  metrics.ok();"),
        ]);
        let m = marked(&lines);
        assert_eq!(m[0], None, "context is never marked");
        assert_eq!(m[1], Some(vec![]));
        assert_eq!(m[2], None, "the inserted line keeps the whole-line treatment");
        assert_eq!(m[3], Some(vec![", verifier".to_string()]));
        assert_eq!(m[5], None, "an add with no delete before it is unpaired");
    }

    #[test]
    fn intraline_leaves_a_rewritten_line_whole() {
        let lines = hunk(&[(Sign::Del, "    let a = foo();"), (Sign::Add, "    return None;")]);
        assert_eq!(marked(&lines), vec![None, None]);
        // Shared indentation is not likeness: this pair differs in all of
        // its content, and would pass the ratio if the indent counted.
        let lines = hunk(&[(Sign::Del, "            }"), (Sign::Add, "            return x;")]);
        assert_eq!(marked(&lines), vec![None, None]);
    }

    #[test]
    fn intraline_skips_a_line_past_the_byte_cap() {
        let long = format!("let x = \"{}\";", "a".repeat(INTRALINE_MAX_BYTES));
        let edited = long.replace("let x", "let y");
        let lines = hunk(&[(Sign::Del, &long), (Sign::Add, &edited)]);
        assert_eq!(marked(&lines), vec![None, None]);
    }

    #[test]
    fn intraline_joins_marks_across_whitespace_and_keeps_char_boundaries() {
        let lines =
            hunk(&[(Sign::Del, "call(alpha beta, x)"), (Sign::Add, "call(gamma delta, x)")]);
        assert_eq!(
            marked(&lines),
            vec![Some(vec!["alpha beta".into()]), Some(vec!["gamma delta".into()])]
        );
        let lines = hunk(&[(Sign::Del, "let שם = \"א\";"), (Sign::Add, "let שם = \"ב\";")]);
        assert_eq!(marked(&lines), vec![Some(vec!["א".into()]), Some(vec!["ב".into()])]);
    }

    /// A CRLF → LF rewrite is exactly the edit the eye cannot find.
    #[test]
    fn intraline_marks_a_carriage_return() {
        let lines = hunk(&[(Sign::Del, "fn main() {}\r"), (Sign::Add, "fn main() {}")]);
        assert_eq!(marked(&lines), vec![Some(vec!["\r".into()]), Some(vec![])]);
    }

    #[test]
    fn merge_and_status_fold() {
        let raw = format!(":100644 100644 {OID_A} {OID_B} M\0src/x.rs\0");
        let mut files = parse_raw_z(raw.as_bytes());
        merge_numstat(&mut files, &[("src/x.rs".into(), Some((3, 1)))]);
        assert_eq!((files[0].adds, files[0].dels), (Some(3), Some(1)));
        let mut flags = StatusFlags::default();
        flags.dirty.insert("src/x.rs".into());
        flags.untracked.push(".env.local".into());
        apply_status_flags(&mut files, &flags);
        assert!(files[0].dirty);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, ".env.local");
        assert!(files[1].untracked);
        assert_eq!(files[1].status, "");
    }
}
