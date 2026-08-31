//! Read-only diff model + parsers (M4b, docs/08 §1.2–1.3).
//!
//! Pure: bytes in, structs out. Every parser takes the raw `-z` output and
//! splits on NUL *before* decoding — git octal-escapes non-ASCII paths unless
//! `-z` is given, and a lossy decode-then-split would corrupt the record
//! stream on paths containing what lossy decoding mangles.

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
    /// "A" "M" "D" "R" "T" — or "" for an in-flight-only (untracked) row.
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
    if entry.old_blob == entry.new_blob && entry.old_mode != entry.new_mode {
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
pub fn apply_status_flags(files: &mut Vec<FileEntry>, flags: &StatusFlags) {
    for f in files.iter_mut() {
        if flags.dirty.contains(&f.path) {
            f.dirty = true;
        }
    }
    for path in &flags.untracked {
        if let Some(f) = files.iter_mut().find(|f| f.path == *path) {
            f.untracked = true;
            continue;
        }
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

    #[test]
    fn classify_text() {
        let patch = b"--- a/x\n+++ b/x\n@@ -1,1 +1,2 @@\n line\n+added\n";
        let fd = build_file_diff(&entry("M", "100644", "100644"), patch, 1024);
        assert_eq!(fd.render, Render::Text);
        assert_eq!(fd.hunks.len(), 1);
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
