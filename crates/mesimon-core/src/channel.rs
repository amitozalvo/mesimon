//! Release channels (T-946): `stable` and `unstable`, which one a version
//! belongs to, and the tag each is published under.
//!
//! A build's channel is read off its version and nowhere else, so a version
//! and its channel cannot disagree: an unstable version carries the
//! pre-release identifier `unstable` — `0.1.0-beta.13.unstable.N` after the
//! stable `0.1.0-beta.13`, and from 0.1.0 final the next minor's
//! `0.2.0-unstable.N` — and anything without it is stable. SemVer's
//! precedence then puts every unstable build above the stable it grew from
//! and below the stable that promotes it, and `0.1.0-beta.13.1` (a hotfix)
//! below the whole train, since a numeric identifier ranks under `unstable`.
//!
//! Both channels publish to one repository, `mesimon-releases`, and the tag
//! is what tells them apart. Every updater since T-877 (beta.12 and on)
//! takes the highest tag `semver` parses once a leading `v` is trimmed, and
//! the `install.sh` served since T-877 (2026-10-22) the highest tag matching
//! `^v?[0-9]+\.[0-9]+\.[0-9]+(-…)?$`; neither reads the pre-release flag,
//! and every release carries it. So a stable release is tagged `v<version>`
//! as ever, and an unstable one `unstable-<version>`, which both of those
//! pickers skip. A copy of `install.sh` saved before 2026-10-22 picks the
//! newest release by listing order and would install an unstable build; the
//! served script does not.
//! [`tag`] and [`parse_tag`] are the one map between versions and tags.

/// The word an unstable tag starts with, before its version.
pub const UNSTABLE_TAG_PREFIX: &str = "unstable-";

/// The release tag of `version`: `v0.1.0-beta.13` for a stable one,
/// `unstable-0.1.0-beta.13.unstable.3` for an unstable one.
pub fn tag(version: &str) -> String {
    match Channel::of(version) {
        Channel::Stable => format!("v{version}"),
        Channel::Unstable => format!("{UNSTABLE_TAG_PREFIX}{version}"),
    }
}

/// A release tag's channel and version, `None` for a tag this project does
/// not mint: an unstable version under `v` (which every shipped picker would
/// take) and a stable one under `unstable-` are both refused, so the map is
/// one-to-one. A stable tag's `v` may be left off, as the shipped pickers
/// allowed.
pub fn parse_tag(tag: &str) -> Option<(Channel, &str)> {
    let tag = tag.trim();
    let (channel, version) = match tag.strip_prefix(UNSTABLE_TAG_PREFIX) {
        Some(v) => (Channel::Unstable, v),
        None => (Channel::Stable, tag.strip_prefix('v').unwrap_or(tag)),
    };
    (version.starts_with(|c: char| c.is_ascii_digit()) && Channel::of(version) == channel)
        .then_some((channel, version))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    #[default]
    Stable,
    Unstable,
}

impl Channel {
    /// The channel of `version` (`0.1.0-beta.13.unstable.2`, a leading `v`
    /// allowed): unstable when one of its pre-release identifiers is exactly
    /// `unstable`. Build metadata (`+…`) is not a pre-release and says
    /// nothing. A tag goes through [`parse_tag`], never here.
    pub fn of(version: &str) -> Channel {
        let v = version.trim().trim_start_matches('v');
        let v = v.split_once('+').map_or(v, |(v, _)| v);
        match v.split_once('-') {
            Some((_, pre)) if pre.split('.').any(|id| id == "unstable") => Channel::Unstable,
            _ => Channel::Stable,
        }
    }

    /// The channel of the running build.
    pub fn running() -> Channel {
        Self::of(env!("CARGO_PKG_VERSION"))
    }

    /// `stable` or `unstable`: the preference's word and the one a person reads.
    pub fn word(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Unstable => "unstable",
        }
    }

    /// The channel a word names, the preference's back-reading of `word`.
    pub fn from_word(s: &str) -> Option<Channel> {
        match s {
            "stable" => Some(Channel::Stable),
            "unstable" => Some(Channel::Unstable),
            _ => None,
        }
    }

    /// The other channel: what the Settings row cycles to.
    pub fn next(self) -> Channel {
        match self {
            Channel::Stable => Channel::Unstable,
            Channel::Unstable => Channel::Stable,
        }
    }

    /// May a machine following this channel take a release of `release`'s?
    /// Stable takes stable alone; unstable takes both and the higher wins, so
    /// a promotion reaches it (electron-updater's beta-gets-latest rule).
    pub fn takes(self, release: Channel) -> bool {
        self == Channel::Unstable || release == Channel::Stable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_names_its_channel() {
        for (v, want) in [
            ("0.1.0-beta.13", Channel::Stable),
            ("v0.1.0-beta.13", Channel::Stable),
            ("0.1.0-beta.13.unstable.1", Channel::Unstable),
            ("v0.1.0-beta.13.unstable.12", Channel::Unstable),
            ("0.1.0-beta.13.1", Channel::Stable),
            ("0.1.0", Channel::Stable),
            ("0.2.0-unstable.1", Channel::Unstable),
            ("0.1.1", Channel::Stable),
            // An identifier that merely contains the word is not the word.
            ("0.1.0-unstable-ish.1", Channel::Stable),
            ("0.1.0-beta.14-unstable.3", Channel::Stable),
            // Build metadata is not a pre-release.
            ("0.1.0+unstable", Channel::Stable),
            ("0.1.0-beta.13.unstable.2+sha", Channel::Unstable),
            ("nightly", Channel::Stable),
        ] {
            assert_eq!(Channel::of(v), want, "{v}");
        }
    }

    /// The tag table: what each version is published as, and what each tag
    /// reads back as. The pairs are one map; the refusals are tags this
    /// project never mints.
    #[test]
    fn a_tag_and_its_version_are_one_map() {
        for (version, t) in [
            ("0.1.0-beta.13", "v0.1.0-beta.13"),
            ("0.1.0-beta.13.1", "v0.1.0-beta.13.1"),
            ("0.1.0-beta.13.unstable.1", "unstable-0.1.0-beta.13.unstable.1"),
            ("0.1.0-beta.13.unstable.12", "unstable-0.1.0-beta.13.unstable.12"),
            ("0.1.0", "v0.1.0"),
            ("0.2.0-unstable.1", "unstable-0.2.0-unstable.1"),
        ] {
            assert_eq!(tag(version), t);
            assert_eq!(parse_tag(t), Some((Channel::of(version), version)), "{t}");
        }
        assert_eq!(parse_tag("0.1.0-beta.13"), Some((Channel::Stable, "0.1.0-beta.13")));
        for refused in [
            "v0.1.0-beta.13.unstable.1",
            "unstable-0.1.0-beta.13",
            "unstable-v0.1.0-beta.13.unstable.1",
            "nightly",
            "v",
            "",
        ] {
            assert_eq!(parse_tag(refused), None, "{refused}");
        }
    }

    #[test]
    fn unstable_takes_both_channels_and_stable_its_own() {
        assert!(Channel::Stable.takes(Channel::Stable));
        assert!(!Channel::Stable.takes(Channel::Unstable));
        assert!(Channel::Unstable.takes(Channel::Stable));
        assert!(Channel::Unstable.takes(Channel::Unstable));
        assert_eq!(Channel::running(), Channel::of(env!("CARGO_PKG_VERSION")));
        for c in [Channel::Stable, Channel::Unstable] {
            assert_eq!(Channel::from_word(c.word()), Some(c));
            assert_eq!(c.next().next(), c);
        }
        assert_eq!(Channel::from_word("nightly"), None);
    }
}
