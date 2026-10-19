//! Boards that reach each other on one machine (T-733), judged without I/O.
//!
//! The author ran several boards at once — the core's, the relay's — and
//! carried asks from one crown to the other by hand, pasting across terminal
//! tabs. A link is a person's say-so that two boards' crowns may talk: the
//! crown of either calls `ask_board` naming the other, the words land on that
//! board's crown's card and go in as its next prompt, and the reply is the
//! same tool back.
//!
//! One file holds every pair, `links.toml` beside every board's state dir
//! (`paths::machine_links_file`), the `tiers.toml` shape: every board's
//! daemon reads it when it decides, so a board that was closed when the
//! person linked it is linked the moment it opens, and either board unlinks
//! for both. A pair names two repositories by their canonical paths; a
//! board's NAME, what the header and the tool say, is its directory's.
//! Nothing here reads or writes a file: the daemon's `peer` module does.

use serde::{Deserialize, Serialize};

/// Two boards that reach each other. Unordered: `a`/`b` is how the file
/// spells it, never which side linked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub a: String,
    pub b: String,
}

impl Pair {
    fn holds(&self, path: &str) -> bool {
        self.a == path || self.b == path
    }

    fn other(&self, path: &str) -> Option<&str> {
        if self.a == path {
            Some(&self.b)
        } else if self.b == path {
            Some(&self.a)
        } else {
            None
        }
    }
}

/// Every pair on the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Links {
    #[serde(default)]
    pub pairs: Vec<Pair>,
}

/// What the board calls a repository: its directory's name, as the
/// breadcrumb does (`App::board_name`). A root with no name (`/`) is its
/// path.
pub fn board_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string())
}

impl Links {
    /// The boards `me` reaches, by path, in the file's order, each once.
    pub fn partners(&self, me: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.pairs {
            if let Some(other) = p.other(me) {
                if other != me && !out.iter().any(|o| o == other) {
                    out.push(other.to_string());
                }
            }
        }
        out
    }

    pub fn linked(&self, me: &str, other: &str) -> bool {
        me != other && self.pairs.iter().any(|p| p.holds(me) && p.holds(other))
    }

    /// Link two boards. False when they were linked already, or are one board.
    pub fn link(&mut self, me: &str, other: &str) -> bool {
        if me == other || self.linked(me, other) {
            return false;
        }
        self.pairs.push(Pair { a: me.to_string(), b: other.to_string() });
        true
    }

    /// Unlink two boards. False when they were not linked.
    pub fn unlink(&mut self, me: &str, other: &str) -> bool {
        let before = self.pairs.len();
        self.pairs.retain(|p| !(p.holds(me) && p.holds(other)));
        self.pairs.len() != before
    }

    /// The partner `word` names — a board's name (case aside) or its path —
    /// or why none: not linked, with the names that are, or a name two
    /// partners share, which the path settles.
    pub fn find(&self, me: &str, word: &str) -> Result<String, String> {
        let word = word.trim();
        let partners = self.partners(me);
        if let Some(p) = partners.iter().find(|p| p.as_str() == word) {
            return Ok(p.clone());
        }
        let by_name: Vec<&String> =
            partners.iter().filter(|p| board_name(p).eq_ignore_ascii_case(word)).collect();
        match by_name.as_slice() {
            [one] => Ok((*one).clone()),
            [] if partners.is_empty() => Err(format!(
                "this board reaches no other board; a person links one in Settings → Crown → \
                 Reaches boards (asked for {word})"
            )),
            [] => Err(format!(
                "{word} is not a board this one reaches; it reaches {}",
                partners.iter().map(|p| board_name(p)).collect::<Vec<_>>().join(", ")
            )),
            many => Err(format!(
                "{word} names {} linked boards; name one by its path: {}",
                many.len(),
                many.iter().map(|p| p.as_str()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "/home/me/code/mesimon";
    const RELAY: &str = "/home/me/code/mesimon-relay";
    const SITE: &str = "/home/me/code/site";

    #[test]
    fn a_board_is_named_by_its_directory() {
        assert_eq!(board_name(CORE), "mesimon");
        assert_eq!(board_name("/home/me/code/mesimon/"), "mesimon");
        assert_eq!(board_name("/"), "/");
    }

    #[test]
    fn a_link_is_one_pair_and_reaches_both_ways() {
        let mut l = Links::default();
        assert!(l.link(CORE, RELAY));
        assert!(!l.link(RELAY, CORE), "the same pair, from the other side");
        assert!(!l.link(CORE, CORE), "a board does not reach itself");
        assert!(l.linked(CORE, RELAY) && l.linked(RELAY, CORE));
        assert_eq!(l.partners(CORE), vec![RELAY.to_string()]);
        assert_eq!(l.partners(RELAY), vec![CORE.to_string()]);
        assert!(l.partners(SITE).is_empty());
        assert!(l.unlink(RELAY, CORE));
        assert!(!l.unlink(RELAY, CORE));
        assert!(l.pairs.is_empty());
    }

    #[test]
    fn a_partner_is_found_by_name_or_path_and_a_miss_names_the_rest() {
        let mut l = Links::default();
        assert!(l.find(CORE, "mesimon-relay").unwrap_err().contains("reaches no other board"));
        l.link(CORE, RELAY);
        l.link(CORE, SITE);
        assert_eq!(l.find(CORE, "mesimon-relay").unwrap(), RELAY);
        assert_eq!(l.find(CORE, "Mesimon-Relay").unwrap(), RELAY);
        assert_eq!(l.find(CORE, SITE).unwrap(), SITE);
        let err = l.find(CORE, "docs").unwrap_err();
        assert!(err.contains("not a board this one reaches"), "{err}");
        assert!(err.contains("mesimon-relay, site"), "{err}");
        // Two partners of one name: the path settles it.
        l.link(CORE, "/elsewhere/site");
        let err = l.find(CORE, "site").unwrap_err();
        assert!(err.contains("names 2 linked boards"), "{err}");
        assert_eq!(l.find(CORE, "/elsewhere/site").unwrap(), "/elsewhere/site");
    }

    /// The wire shape (the file's is the daemon's store's, on this): a
    /// `pairs` list of `a`/`b`, and nothing is no links.
    #[test]
    fn the_shape_round_trips_and_nothing_is_no_links() {
        let mut l = Links::default();
        l.link(CORE, RELAY);
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["pairs"][0]["a"], CORE);
        assert_eq!(v["pairs"][0]["b"], RELAY);
        assert_eq!(serde_json::from_value::<Links>(v).unwrap(), l);
        assert_eq!(serde_json::from_str::<Links>("{}").unwrap(), Links::default());
    }
}
