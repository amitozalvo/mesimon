//! Which crowned sessions have the crown's rules (T-930): told once a reply
//! carrying them was written to the session's connection, under the reset
//! count it was sent with, and forgotten at every reset since. Nothing is
//! promised from this record: a written line is not a read one, and the
//! crown's own `get_ticket` with `full: true` carries the rules on request.
//!
//! A restart keeps it (T-1279): `crown.json` holds each told session's
//! `Told`, the rules text's digest and when the reply went, and the start
//! takes it back only where nothing since says the context changed. A new
//! build whose rules read differently tells again, as does a Claude seat
//! whose transcript shows a compaction since (or cannot say), and a Codex
//! seat whose first snapshot names another thread or compaction count. On
//! the simbly board, half the rules' deliveries in a day (six of twelve,
//! 3,415 tokens each) followed a `U` that changed neither.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

#[derive(Default)]
pub(super) struct CrownRules {
    /// Each told session, with when its reply went (epoch ms), for the
    /// restart's look at its transcript.
    told: HashMap<uuid::Uuid, u64>,
    /// How many resets there have been: an acknowledgement sent under an
    /// older count tells nobody.
    generation: u64,
    /// Each Codex seat's thread and compaction count as its snapshot last
    /// said them: a change is a context the rules may not be in.
    codex: HashMap<uuid::Uuid, (Option<String>, u64)>,
    /// `told` moved since the ledger last wrote it.
    dirty: bool,
}

/// A told session as `crown.json` keeps it (T-1279). Every field defaults:
/// a ledger without it reads as no session told, which is the old start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Told {
    /// `digest(crown_view())` when the reply went.
    pub digest: String,
    pub told_ms: u64,
    /// A Codex seat's thread and compaction count as they stood.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<(Option<String>, u64)>,
}

/// A stable digest of the rules text (FNV-1a, 64 bits): the same bytes give
/// the same word in every build, so a rebuild that leaves the rules as they
/// were keeps the crowns told.
pub(super) fn digest(text: &str) -> String {
    format!("{:016x}", mesimon_core::text::fnv1a(text.bytes()))
}

/// `digest(crown_view())`, once: the rules text is this build's constants.
pub(super) fn rules_digest() -> &'static str {
    static DIGEST: LazyLock<String> = LazyLock::new(|| digest(&mesimon_core::mcp::crown_view()));
    &DIGEST
}

impl CrownRules {
    /// The reset count a reply to `session` would carry the rules under, or
    /// `None` when a reply carrying them already reached it since.
    pub(super) fn owed(&self, session: uuid::Uuid) -> Option<u64> {
        (!self.told.contains_key(&session)).then_some(self.generation)
    }

    /// A reply carrying the rules was written to `session`'s connection,
    /// sent under `generation`.
    pub(super) fn acked(&mut self, session: uuid::Uuid, generation: u64, now_ms: u64) {
        if generation == self.generation {
            self.told.insert(session, now_ms);
            self.dirty = true;
        }
    }

    /// A context the rules may not be in: every crown's, at a crowning, or
    /// one session's, at its compaction or a new conversation.
    pub(super) fn forget(&mut self, session: Option<uuid::Uuid>) {
        self.generation = self.generation.wrapping_add(1);
        match session {
            Some(id) => {
                self.dirty |= self.told.remove(&id).is_some();
            }
            None => {
                self.dirty |= !self.told.is_empty();
                self.told.clear();
            }
        }
    }

    /// What `crown.json` keeps: each told session, under the rules' `digest`.
    pub(super) fn kept(&self, digest: &str) -> BTreeMap<uuid::Uuid, Told> {
        self.told
            .iter()
            .map(|(id, told_ms)| {
                let told = Told {
                    digest: digest.to_owned(),
                    told_ms: *told_ms,
                    codex: self.codex.get(id).cloned(),
                };
                (*id, told)
            })
            .collect()
    }

    /// A restart's start: each kept session whose digest is `digest` and
    /// that `still` finds in the context it was told in is told again
    /// without a word sent. A Codex seat's thread and count come back too,
    /// so its first snapshot is compared with them.
    pub(super) fn restore(
        &mut self,
        kept: BTreeMap<uuid::Uuid, Told>,
        digest: &str,
        still: impl Fn(&uuid::Uuid, &Told) -> bool,
    ) {
        for (id, told) in kept {
            if told.digest != digest || !still(&id, &told) {
                continue;
            }
            if let Some(context) = told.codex.clone() {
                self.codex.insert(id, context);
            }
            self.told.insert(id, told.told_ms);
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(super) fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// A Codex seat's snapshot: a new thread or a moved compaction count
    /// (a `contextCompaction` started or completed) forgets its rules.
    pub(super) fn codex_context(
        &mut self,
        session: uuid::Uuid,
        thread: Option<String>,
        compactions: u64,
    ) {
        let now = (thread, compactions);
        let was = self.codex.insert(session, now.clone());
        if was.as_ref() != Some(&now) && self.told.contains_key(&session) {
            self.dirty = true;
        }
        if was.is_some_and(|was| was != now) {
            self.forget(Some(session));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told(r: &mut CrownRules, s: uuid::Uuid) {
        let generation = r.owed(s).expect("owed");
        r.acked(s, generation, 1);
    }

    /// T-930's review (b): a reply sent before a reset tells nobody when
    /// its acknowledgement comes after, and the next reply carries them.
    #[test]
    fn an_acknowledgement_under_an_old_count_tells_nobody() {
        let mut r = CrownRules::default();
        let s = uuid::Uuid::new_v4();
        let sent = r.owed(s).expect("a new crown is owed the rules");
        r.forget(Some(s));
        r.acked(s, sent, 1);
        assert!(r.owed(s).is_some(), "the old reply's acknowledgement told nobody");
        let other = uuid::Uuid::new_v4();
        let sent = r.owed(other).unwrap();
        r.forget(Some(s));
        r.acked(other, sent, 1);
        assert!(r.owed(other).is_some(), "any reset voids a reply in flight");
        told(&mut r, other);
        assert_eq!(r.owed(other), None, "an acknowledgement under the current count tells");
        r.forget(None);
        assert!(r.owed(other).is_some(), "a crowning forgets every crown's");
    }

    /// T-930's review (c): a Codex seat's thread change and its compaction
    /// count reset the record, a reply between a compaction's start and its
    /// completion included; a snapshot that repeats itself does not.
    #[test]
    fn a_codex_thread_or_compaction_forgets_what_the_seat_was_told() {
        let mut r = CrownRules::default();
        let s = uuid::Uuid::new_v4();
        r.codex_context(s, Some("t1".into()), 0);
        told(&mut r, s);
        r.codex_context(s, Some("t1".into()), 0);
        assert_eq!(r.owed(s), None, "the same context keeps them");
        r.codex_context(s, Some("t2".into()), 0);
        assert!(r.owed(s).is_some(), "a new thread forgets them");
        told(&mut r, s);
        r.codex_context(s, Some("t2".into()), 1);
        assert!(r.owed(s).is_some(), "a compaction's start forgets them");
        told(&mut r, s);
        r.codex_context(s, Some("t2".into()), 2);
        assert!(r.owed(s).is_some(), "what was said before its completion counts for nothing");
        let sent = r.owed(s).unwrap();
        r.codex_context(s, Some("t2".into()), 3);
        r.acked(s, sent, 1);
        assert!(r.owed(s).is_some(), "a reply in flight across a compaction tells nobody");
    }

    /// T-1279: a restart takes a told session back only under the same
    /// rules text and where the context check finds it unchanged; a Codex
    /// seat's kept thread and count are what its first snapshot meets.
    #[test]
    fn a_restart_keeps_told_only_under_the_same_rules_and_context() {
        let mut r = CrownRules::default();
        let (a, b, c) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        r.codex_context(c, Some("t1".into()), 2);
        for s in [a, b, c] {
            let generation = r.owed(s).unwrap();
            r.acked(s, generation, 500);
        }
        assert!(r.is_dirty());
        let kept = r.kept(&digest("rules v1"));
        assert_eq!(kept[&a].told_ms, 500);
        assert_eq!(kept[&c].codex, Some((Some("t1".into()), 2)));

        let mut same = CrownRules::default();
        same.restore(kept.clone(), &digest("rules v1"), |id, _| *id != b);
        assert_eq!(same.owed(a), None, "unchanged rules and context: told");
        assert!(same.owed(b).is_some(), "a context that moved is owed");
        assert_eq!(same.owed(c), None);
        same.codex_context(c, Some("t1".into()), 2);
        assert_eq!(same.owed(c), None, "the snapshot that matches keeps it");
        same.codex_context(c, Some("t1".into()), 3);
        assert!(same.owed(c).is_some(), "a compaction while no daemon listened forgets it");

        let mut rebuilt = CrownRules::default();
        rebuilt.restore(kept, &digest("rules v2"), |_, _| true);
        assert!(rebuilt.owed(a).is_some(), "new rules text tells every crown again");
    }

    #[test]
    fn the_digest_is_stable_and_tells_texts_apart() {
        assert_eq!(digest(""), "cbf29ce484222325");
        assert_eq!(digest("a"), "af63dc4c8601ec8c");
        assert_ne!(digest("rules"), digest("rules."));
    }
}
