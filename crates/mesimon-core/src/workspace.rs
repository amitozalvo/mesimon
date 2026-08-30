//! Branch/directory naming for ticket worktrees — doc 12 §12.5.2's fixed slugger
//! (D28: changing it produces a lying board). Branch = `msmn/<KEY>-<slug>`,
//! worktree dir = `<KEY>-<slug>`, SAME slugger for both. Lowercasing is mandatory:
//! APFS is case-insensitive, so `Foo`/`foo` collide — folding makes the collision
//! deterministic instead of filesystem-dependent.
//!
//! Callers must pass every produced name as its own argv element, never inside a
//! shell string — a ticket titled `fix; rm -rf ~` is command injection otherwise.

/// The ref namespace mesimon owns; `refs/heads/msmn/` is globbable.
pub const BRANCH_NS: &str = "msmn";

const SLUG_MAX_BYTES: usize = 60;

/// Doc 12 §12.5.2: NFC → ASCII-fold → lowercase → `[^a-z0-9._-]` → `-` →
/// collapse runs → strip leading/trailing `-` and `.` → 60-byte truncate on a
/// char boundary → reject-list → fallback `"t"`.
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
    // Truncate to the byte cap on a char boundary (slug is ASCII here, but stay safe).
    let mut s: String = trimmed.into();
    if s.len() > SLUG_MAX_BYTES {
        let mut cut = SLUG_MAX_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
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
    fn slug_truncates_at_60_bytes() {
        let long = "a".repeat(200);
        let s = slug(&long);
        assert_eq!(s.len(), 60);
    }

    #[test]
    fn truncation_does_not_leave_trailing_dash_or_dot() {
        // 59 chars then a '-' run right at the cap.
        let title = format!("{}-{}", "a".repeat(59), "b".repeat(20));
        let s = slug(&title);
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
}
