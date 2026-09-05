//! Branch/directory naming for ticket worktrees — doc 12 §12.5.2's fixed slugger
//! (D28: changing it produces a lying board — a binding persists its names, so a
//! slugger change only touches worktrees provisioned after it). Branch = `msmn/<KEY>-<slug>`,
//! worktree dir = `<KEY>-<slug>`, SAME slugger for both. Lowercasing is mandatory:
//! APFS is case-insensitive, so `Foo`/`foo` collide — folding makes the collision
//! deterministic instead of filesystem-dependent.
//!
//! Callers must pass every produced name as its own argv element, never inside a
//! shell string — a ticket titled `fix; rm -rf ~` is command injection otherwise.

/// The ref namespace mesimon owns; `refs/heads/msmn/` is globbable.
pub const BRANCH_NS: &str = "msmn";

/// Doc 12 said 60, and 60 bytes of a sentence-shaped title cut mid-word
/// (`T-164-mcp-tool-to-modify-tags-think-how-to-best-design-that-tool-t`,
/// dogfood 2026-09-03). The key already makes the name unique; the slug only
/// has to say which ticket, so it is a handful of words, cut at a word.
const SLUG_MAX_BYTES: usize = 32;

/// The word-boundary cut gives up and cuts mid-word when it would leave fewer
/// bytes than this — a title that opens with one long token still gets a slug.
const SLUG_MIN_WORD_CUT: usize = 8;

/// Doc 12 §12.5.2: NFC → ASCII-fold → lowercase → `[^a-z0-9._-]` → `-` →
/// collapse runs → strip leading/trailing `-` and `.` → 32-byte truncate at the
/// last `-` before the cap (mid-word only when no word fits) → reject-list →
/// fallback `"t"`.
///
/// ASCII-fold here is the pragmatic subset: strip combining marks after NFD-style
/// decomposition is out of scope without a unicode table dep, so non-ASCII chars
/// (including Hebrew titles) fold to `-` via the character-class step; a fully
/// non-ASCII title therefore hits the fallback, which is the documented behavior
/// for a rejected slug.
pub fn slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true; // suppress a leading '-'
    for c in title.chars() {
        let folded = match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '.' | '_' => c,
            // Cheap ASCII folding for the Latin-1 range the author actually hits.
            'à'..='å' | 'À'..='Å' => 'a',
            'è'..='ë' | 'È'..='Ë' => 'e',
            'ì'..='ï' | 'Ì'..='Ï' => 'i',
            'ò'..='ö' | 'Ò'..='Ö' => 'o',
            'ù'..='ü' | 'Ù'..='Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' | 'Ñ' => 'n',
            _ => '-',
        };
        if folded == '-' {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(folded);
            last_dash = false;
        }
    }
    // Strip leading/trailing '-' and '.'.
    let trimmed: &str = out.trim_matches(['-', '.']);
    // Truncate to the byte cap: at the last word boundary that keeps enough of
    // the name, else on a char boundary (slug is ASCII here, but stay safe).
    let mut s: String = trimmed.into();
    if s.len() > SLUG_MAX_BYTES {
        let mut cut = SLUG_MAX_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        // `s[..=cut]` holds one byte past the cap, so a '-' AT the cap counts:
        // the word before it fits whole.
        if let Some(dash) = s[..=cut].rfind('-').filter(|&d| d >= SLUG_MIN_WORD_CUT) {
            cut = dash;
        }
        s.truncate(cut);
        let retrimmed = s.trim_end_matches(['-', '.']).to_string();
        s = retrimmed;
    }
    if s.is_empty() || s == "@" || s.starts_with('-') || s.ends_with('.') || s.ends_with(".lock") {
        return "t".into();
    }
    s
}

/// `<KEY>-<slug>` — the worktree directory basename. The key stays verbatim
/// (doc 12: `AUTH-3-fix-login`); keys are board-unique so the case-collision
/// argument applies only to the slug half.
pub fn dir_name(short_key: &str, title: &str) -> String {
    format!("{}-{}", short_key, slug(title))
}

/// `msmn/<KEY>-<slug>` — the branch mesimon creates for a worktree ticket.
pub fn branch_name(short_key: &str, title: &str) -> String {
    format!("{}/{}", BRANCH_NS, dir_name(short_key, title))
}

// ---------------------------------------------------------------------------
// A board root that holds repositories (T-225).
//
// The author's `simbly` is a directory of twenty independent git repositories
// under a three-file "meta" repo whose `.gitignore` says `*/`. Every git
// answer mesimon gave there was about the meta — the header's branch, the
// checkout diff, a worktree of nothing. The census below is how the daemon
// learns the shape: one `readdir` of the root and a probe of `<child>/.git`,
// never recursive, classified here so the rule is testable without a disk.
// ---------------------------------------------------------------------------

/// The most nested repositories one board follows. A root with more child
/// repos than this is a code dump, not a workspace; the census stops there.
pub const MAX_WORKSPACE_REPOS: usize = 64;

/// What `<child>/.git` was, for one immediate child directory of the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitMark {
    /// No `.git` at all: a plain directory.
    None,
    /// A `.git` DIRECTORY: a repository of its own.
    Dir,
    /// A `.git` FILE (`gitdir: …`): a worktree of some repository, or a
    /// submodule checkout — either way its owner is elsewhere, so it is
    /// never a workspace repo itself.
    File,
}

/// The names of the root's immediate children that are independent
/// repositories: a `.git` directory of their own and not a submodule the root
/// declares. Sorted, capped at [`MAX_WORKSPACE_REPOS`]. Names starting with a
/// dot are admitted (the author keeps worktrees under `.wt/`, and those
/// resolve to their owners by carrying a gitfile); `.git` itself and the
/// board's own `.mesimon` never appear because neither holds a `.git`.
pub fn nested_repos<'a>(
    children: impl IntoIterator<Item = (&'a str, GitMark)>,
    submodule_paths: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = children
        .into_iter()
        .filter(|(_, mark)| *mark == GitMark::Dir)
        .map(|(name, _)| name)
        .filter(|name| !submodule_paths.iter().any(|p| p.trim_end_matches('/') == *name))
        .map(str::to_string)
        .collect();
    out.sort();
    out.truncate(MAX_WORKSPACE_REPOS);
    out
}

/// `19 repos` — what the header and the diff screen call a workspace where
/// they would name a branch. One word for both, so the two screens agree.
pub fn repos_word(n: usize) -> String {
    crate::text::plural(n, "repo")
}

/// The `path = …` values of a `.gitmodules` file. A submodule's checkout can
/// carry a real `.git` directory (pre-1.7.8 layout, or `git submodule
/// absorbgitdirs` never run), and a superproject's worktrees are the case the
/// corpus already refuses (12 §12.6.9) — so a declared submodule is never a
/// workspace repo, whatever its `.git` looks like.
pub fn submodule_paths(gitmodules: &str) -> Vec<String> {
    gitmodules
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "path").then(|| v.trim().to_string())
        })
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basics() {
        assert_eq!(slug("Fix auth redirect loop"), "fix-auth-redirect-loop");
        assert_eq!(slug("  --Weird__name.rs--  "), "weird__name.rs");
        assert_eq!(slug("CaSe FoLd"), "case-fold");
    }

    #[test]
    fn slug_collapses_runs_and_strips_edges() {
        assert_eq!(slug("a———b"), "a-b");
        assert_eq!(slug("...dots..."), "dots");
        assert_eq!(slug("-lead and trail-"), "lead-and-trail");
    }

    #[test]
    fn slug_injection_titles_are_inert() {
        assert_eq!(slug("fix; rm -rf ~"), "fix-rm-rf");
        assert_eq!(slug("$(evil) `cmd` |x&&y"), "evil-cmd-x-y");
    }

    #[test]
    fn slug_rejects_fall_back_to_t() {
        assert_eq!(slug(""), "t");
        assert_eq!(slug("@"), "t");
        assert_eq!(slug("!!!"), "t");
        assert_eq!(slug("שלום עולם"), "t"); // fully non-ASCII → fallback
        assert_eq!(slug("x.lock"), "t");
    }

    #[test]
    fn slug_truncates_at_32_bytes() {
        let long = "a".repeat(200);
        let s = slug(&long);
        assert_eq!(s.len(), SLUG_MAX_BYTES);
    }

    #[test]
    fn truncation_cuts_at_a_word() {
        // The dogfood title that motivated the cap: a sentence.
        let s = slug("mcp tool to modify tags, think how to best design that tool to be useful");
        assert_eq!(s, "mcp-tool-to-modify-tags-think");
        assert!(s.len() <= SLUG_MAX_BYTES);
        // A word ending exactly at the cap is kept whole.
        let s = slug(&format!("{}-{}", "a".repeat(SLUG_MAX_BYTES), "tail"));
        assert_eq!(s.len(), SLUG_MAX_BYTES);
        // Nothing to cut: a title under the cap is untouched.
        assert_eq!(slug("short and sweet"), "short-and-sweet");
    }

    #[test]
    fn truncation_falls_back_mid_word_when_the_first_word_is_long() {
        // Only a tiny word before the cap: the mid-word cut is the better name.
        let s = slug(&format!("ab-{}", "c".repeat(100)));
        assert_eq!(s.len(), SLUG_MAX_BYTES);
        assert!(s.starts_with("ab-ccc"), "{s:?}");
    }

    #[test]
    fn truncation_does_not_leave_trailing_dash_or_dot() {
        // A '-' run right at the cap.
        let title = format!("{}-{}", "a".repeat(SLUG_MAX_BYTES - 1), "b".repeat(20));
        let s = slug(&title);
        assert!(!s.ends_with('-') && !s.ends_with('.'), "{s:?}");
        // And a '.' at the word cut.
        let s = slug(&format!("{}.-{}", "a".repeat(20), "b".repeat(20)));
        assert!(!s.ends_with('-') && !s.ends_with('.'), "{s:?}");
    }

    #[test]
    fn names_compose() {
        assert_eq!(dir_name("T-7", "Fix login"), "T-7-fix-login");
        assert_eq!(branch_name("T-7", "Fix login"), "msmn/T-7-fix-login");
    }

    #[test]
    fn hebrew_title_still_yields_usable_names() {
        // D33f: author's board carries Hebrew titles; key prefix keeps them unique.
        assert_eq!(dir_name("T-12", "תקן את הבאג"), "T-12-t");
        assert_eq!(branch_name("T-12", "תקן את הבאג"), "msmn/T-12-t");
    }

    /// The census rule (T-225): a `.git` DIRECTORY is a repo of its own, a
    /// gitfile belongs to someone else, a declared submodule is never a
    /// workspace repo whatever its `.git` looks like, and the answer is
    /// sorted so the header's count and the diff's order never depend on
    /// `readdir`.
    #[test]
    fn nested_repos_admits_own_git_dirs_and_nothing_else() {
        let children = [
            ("web", GitMark::Dir),
            ("api", GitMark::Dir),
            (".wt", GitMark::None),
            ("fe-feedback", GitMark::File),
            ("node_modules", GitMark::None),
            ("vendor", GitMark::Dir),
        ];
        assert_eq!(nested_repos(children, &[]), vec!["api", "vendor", "web"]);
        assert_eq!(nested_repos(children, &["vendor/".to_string()]), vec!["api", "web"]);
        assert!(nested_repos([("x", GitMark::File), ("y", GitMark::None)], &[]).is_empty());
    }

    #[test]
    fn nested_repos_is_capped() {
        let names: Vec<String> = (0..100).map(|i| format!("r{i:03}")).collect();
        let out = nested_repos(names.iter().map(|n| (n.as_str(), GitMark::Dir)), &[]);
        assert_eq!(out.len(), MAX_WORKSPACE_REPOS);
        assert_eq!(out[0], "r000");
    }

    #[test]
    fn gitmodules_paths_are_read() {
        let text =
            "[submodule \"lib\"]\n\tpath = vendor/lib\n\turl = x\n[submodule \"b\"]\n  path=b\n";
        assert_eq!(submodule_paths(text), vec!["vendor/lib", "b"]);
        assert!(submodule_paths("").is_empty());
    }

    #[test]
    fn repos_word_counts() {
        assert_eq!(repos_word(1), "1 repo");
        assert_eq!(repos_word(19), "19 repos");
    }
}
