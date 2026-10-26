//! Which crowned sessions have the crown's rules (T-930): told once a reply
//! carrying them was written to the session's connection, under the reset
//! count it was sent with, and forgotten at every reset since. In memory on
//! purpose: a restart tells every crown again, and the rules are the
//! binary's own text, so a new build is a restart and needs no version.
//! Nothing is promised from this record: a written line is not a read one,
//! and the crown's own `get_ticket` with `full: true` carries the rules on request.

use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct CrownRules {
    told: HashSet<uuid::Uuid>,
    /// How many resets there have been: an acknowledgement sent under an
    /// older count tells nobody.
    generation: u64,
    /// Each Codex seat's thread and compaction count as its snapshot last
    /// said them: a change is a context the rules may not be in.
    codex: HashMap<uuid::Uuid, (Option<String>, u64)>,
}

impl CrownRules {
    /// The reset count a reply to `session` would carry the rules under, or
    /// `None` when a reply carrying them already reached it since.
    pub(super) fn owed(&self, session: uuid::Uuid) -> Option<u64> {
        (!self.told.contains(&session)).then_some(self.generation)
    }

    /// A reply carrying the rules was written to `session`'s connection,
    /// sent under `generation`.
    pub(super) fn acked(&mut self, session: uuid::Uuid, generation: u64) {
        if generation == self.generation {
            self.told.insert(session);
        }
    }

    /// A context the rules may not be in: every crown's, at a crowning, or
    /// one session's, at its compaction or a new conversation.
    pub(super) fn forget(&mut self, session: Option<uuid::Uuid>) {
        self.generation = self.generation.wrapping_add(1);
        match session {
            Some(id) => {
                self.told.remove(&id);
            }
            None => self.told.clear(),
        }
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
        if self.codex.insert(session, now.clone()).is_some_and(|was| was != now) {
            self.forget(Some(session));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told(r: &mut CrownRules, s: uuid::Uuid) {
        let generation = r.owed(s).expect("owed");
        r.acked(s, generation);
    }

    /// T-930's review (b): a reply sent before a reset tells nobody when
    /// its acknowledgement comes after, and the next reply carries them.
    #[test]
    fn an_acknowledgement_under_an_old_count_tells_nobody() {
        let mut r = CrownRules::default();
        let s = uuid::Uuid::new_v4();
        let sent = r.owed(s).expect("a new crown is owed the rules");
        r.forget(Some(s));
        r.acked(s, sent);
        assert!(r.owed(s).is_some(), "the old reply's acknowledgement told nobody");
        let other = uuid::Uuid::new_v4();
        let sent = r.owed(other).unwrap();
        r.forget(Some(s));
        r.acked(other, sent);
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
        r.acked(s, sent);
        assert!(r.owed(s).is_some(), "a reply in flight across a compaction tells nobody");
    }
}
