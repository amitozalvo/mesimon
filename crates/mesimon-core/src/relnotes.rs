//! Release notes: the repo's `CHANGELOG.md`, compiled into the binary and
//! read as a list of releases.
//!
//! One document, three readers. The author writes it; `ci/release.sh` lifts
//! the tag's section out of it for the GitHub release body; and the TUI's
//! RELEASES screen (a menu row) renders the whole thing, newest first. The
//! file is the store — nothing is fetched, nothing is written, and a binary
//! carries exactly the notes of the versions that exist at its build, so a
//! board offline on a plane still answers "what changed". Each release is a
//! `## <tag> — <date>` heading and the markdown under it; the parser accepts
//! nothing looser, and `every_release_is_dated_and_in_order` runs it over the
//! real file, so a malformed heading fails `cargo ut` rather than rendering
//! as body text.

/// The changelog, at build time. `include_str!` makes it a rebuild
/// dependency, so an edit is in the next binary.
pub const SOURCE: &str = include_str!("../../../CHANGELOG.md");

/// One release: its tag (`v0.1.0-alpha.11`), its date (`2026-09-03`) and the
/// markdown under its heading, blank edges trimmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub date: String,
    pub body: String,
}

impl Release {
    /// `3 Sep 2026` — the date as a reader says it. An undated release (a
    /// heading with no ` — YYYY-MM-DD`) says nothing rather than `--`.
    pub fn date_words(&self) -> String {
        const MONTHS: [&str; 12] =
            ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let Some((y, m, d)) = ymd(&self.date) else {
            return String::new();
        };
        match MONTHS.get((m as usize).wrapping_sub(1)) {
            Some(mon) => format!("{d} {mon} {y}"),
            None => self.date.clone(),
        }
    }
}

/// `YYYY-MM-DD` → its three numbers, or `None` for anything else.
fn ymd(date: &str) -> Option<(u32, u32, u32)> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |s: &str| s.parse::<u32>().ok().filter(|_| s.bytes().all(|c| c.is_ascii_digit()));
    Some((num(&date[0..4])?, num(&date[5..7])?, num(&date[8..10])?))
}

/// Is `date` a well-formed calendar-ish `YYYY-MM-DD`? Shape and range only —
/// this is a document lint, not a calendar.
pub fn well_formed_date(date: &str) -> bool {
    matches!(ymd(date), Some((_, m, d)) if (1..=12).contains(&m) && (1..=31).contains(&d))
}

/// A `## ` heading, split into `(tag, date)` when it names a release.
/// `## v0.1.0-alpha.11 — 2026-09-03` (or ` - `, the hyphen spelling) is one;
/// an undated `## v0.1.0-alpha.11` is one with an empty date; any other
/// second-level heading (`## Unreleased`, say) is body text of whatever
/// release precedes it — which is what keeps a prose heading from minting
/// a version.
fn heading(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("## ")?.trim();
    let is_tag = |s: &str| {
        s.starts_with('v') && s.len() > 1 && s.as_bytes()[1].is_ascii_digit() && !s.contains(' ')
    };
    for sep in [" — ", " - "] {
        if let Some((tag, date)) = rest.split_once(sep) {
            let (tag, date) = (tag.trim(), date.trim());
            if is_tag(tag) {
                return Some((tag.to_string(), date.to_string()));
            }
        }
    }
    is_tag(rest).then(|| (rest.to_string(), String::new()))
}

/// Every release in `src`, in document order (the changelog is newest first,
/// and `every_release_is_dated_and_in_order` holds it to that). Text before
/// the first release heading — the file's title and preamble — is dropped.
pub fn parse(src: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut open: Option<(String, String)> = None;
    let mut fence = false;
    let close =
        |open: &mut Option<(String, String)>, body: &mut Vec<&str>, out: &mut Vec<Release>| {
            if let Some((tag, date)) = open.take() {
                let text = body.join("\n");
                out.push(Release { tag, date, body: text.trim_matches('\n').to_string() });
            }
            body.clear();
        };
    for line in src.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence {
            if let Some(h) = heading(line) {
                close(&mut open, &mut body, &mut out);
                open = Some(h);
                continue;
            }
        }
        if open.is_some() {
            body.push(line);
        }
    }
    close(&mut open, &mut body, &mut out);
    out
}

/// The index of `tag` in `releases`, if it has notes.
pub fn position(releases: &[Release], tag: &str) -> Option<usize> {
    releases.iter().position(|r| r.tag == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `v0.1.0-alpha.11` → a key that orders the way `semver` would for the
    /// shapes this project mints: `x.y.z`, optionally `-alpha.N` or
    /// `-beta.N`, where every beta outranks every alpha of its number and a
    /// final release outranks both.
    fn order_key(tag: &str) -> Option<(u64, u64, u64, (u64, u64))> {
        let v = tag.strip_prefix('v')?;
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let mut it = core.split('.').map(|s| s.parse::<u64>().ok());
        let (x, y, z) = (it.next()??, it.next()??, it.next()??);
        if it.next().is_some() {
            return None;
        }
        let n = match pre {
            None => (u64::MAX, 0),
            Some(p) => match p.strip_prefix("alpha.") {
                Some(n) => (0, n.parse().ok()?),
                None => (1, p.strip_prefix("beta.")?.parse().ok()?),
            },
        };
        Some((x, y, z, n))
    }

    #[test]
    fn order_key_crosses_alpha_into_beta() {
        let k = |t| order_key(t).unwrap_or_else(|| panic!("{t} has no key"));
        assert!(k("v0.1.0-alpha.10") > k("v0.1.0-alpha.9"), "numeric, not lexical");
        assert!(k("v0.1.0-beta.1") > k("v0.1.0-alpha.40"), "a beta outranks every alpha");
        assert!(k("v0.1.0-beta.2") > k("v0.1.0-beta.1"));
        assert!(k("v0.1.0") > k("v0.1.0-beta.9"), "a release outranks its prereleases");
        assert!(k("v0.2.0-alpha.1") > k("v0.1.0"));
        assert_eq!(order_key("v0.1.0-rc.1"), None, "a shape this project does not mint");
    }

    #[test]
    fn headings_are_tag_then_date() {
        assert_eq!(
            heading("## v0.1.0-alpha.11 — 2026-09-03"),
            Some(("v0.1.0-alpha.11".into(), "2026-09-03".into()))
        );
        assert_eq!(heading("## v1.0.0 - 2027-01-01"), Some(("v1.0.0".into(), "2027-01-01".into())));
        assert_eq!(heading("## v0.1.0-alpha.3"), Some(("v0.1.0-alpha.3".into(), String::new())));
        assert_eq!(heading("## Unreleased"), None);
        assert_eq!(heading("### v0.1.0-alpha.3"), None);
        assert_eq!(heading("## version 2 — 2026-01-01"), None);
    }

    #[test]
    fn parse_splits_on_release_headings_and_keeps_bodies() {
        let src = "# Changelog\n\npreamble\n\n## v0.2.0 — 2026-10-01\n\n- **A.** one\n\n## \
                   v0.1.0 — 2026-09-01\n\n- b\n\n```\n## v9.9.9 — not a heading\n```\n\n## \
                   Notes\n\nprose\n";
        let rs = parse(src);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].tag, "v0.2.0");
        assert_eq!(rs[0].date, "2026-10-01");
        assert_eq!(rs[0].body, "- **A.** one");
        assert_eq!(rs[1].tag, "v0.1.0");
        // A fenced line that looks like a heading stays in the body, and so
        // does a prose `##` heading — a version is minted by nothing else.
        assert!(rs[1].body.contains("## v9.9.9"));
        assert!(rs[1].body.ends_with("## Notes\n\nprose"));
        assert_eq!(position(&rs, "v0.1.0"), Some(1));
        assert_eq!(position(&rs, "v0.3.0"), None);
    }

    #[test]
    fn dates_read_as_words() {
        let r = |d: &str| Release { tag: "v1".into(), date: d.into(), body: String::new() };
        assert_eq!(r("2026-09-03").date_words(), "3 Sep 2026");
        assert_eq!(r("2026-12-25").date_words(), "25 Dec 2026");
        assert_eq!(r("").date_words(), "");
        assert!(well_formed_date("2026-09-03"));
        assert!(!well_formed_date("2026-13-03"));
        assert!(!well_formed_date("26-09-03"));
        assert!(!well_formed_date("2026-09-3"));
    }

    /// The real changelog: every heading is a dated release, no tag twice,
    /// newest first, and the version this workspace builds has its notes at
    /// the top. This is the gate `ci/release.sh` relies on — a release with
    /// no notes, or notes under a heading the screen cannot read, fails here
    /// before it fails on a user's board.
    #[test]
    fn every_release_is_dated_and_in_order() {
        let rs = parse(SOURCE);
        assert!(rs.len() >= 11, "the changelog parsed to {} releases", rs.len());
        for r in &rs {
            assert!(
                well_formed_date(&r.date),
                "{} has no `— YYYY-MM-DD` date: {:?}",
                r.tag,
                r.date
            );
            assert!(order_key(&r.tag).is_some(), "{} is not a version tag", r.tag);
            assert!(!r.body.is_empty(), "{} has no notes under it", r.tag);
        }
        for w in rs.windows(2) {
            assert!(
                order_key(&w[0].tag) > order_key(&w[1].tag),
                "{} is listed above {} — newest first",
                w[0].tag,
                w[1].tag
            );
            assert!(w[0].date >= w[1].date, "{} is dated before {}", w[0].tag, w[1].tag);
        }
        // Every second-level heading in the file IS a release: a `## ` line
        // that did not parse would render as prose under the release above.
        for line in SOURCE.lines().filter(|l| l.starts_with("## ")) {
            assert!(heading(line).is_some(), "not a release heading: {line:?}");
        }
        let build = concat!("v", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            rs.first().map(|r| r.tag.as_str()),
            Some(build),
            "the top entry of CHANGELOG.md must be this build's ({build}); add `## {build} — <date>`"
        );
    }
}
