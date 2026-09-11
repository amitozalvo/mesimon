//! The sentences mesimon itself types into an agent's box, and the user's
//! right to rewrite them (T-353).
//!
//! Three of mesimon's own sentences reach a live agent: the rebase ask when
//! the base branch moved past a ticket's branch, the notice after that branch
//! was merged, and the nudge when a note on the ticket changed. Every one of
//! them was a `format!` in the daemon until now, which meant the words that
//! start somebody's turn were the binary's and not theirs — wrong in a tool
//! whose third README promise is that mesimon adds no token of its own to a
//! conversation. The three exceptions stay exceptions; what changes here is
//! WHOSE words they are.
//!
//! The shape is deliberately the smallest one that works:
//!
//! - a template is ONE LINE, because [`crate::command::sanitize_prompt`] is
//!   what the text crosses on the way to a tty and it removes every newline.
//!   A field, not a document;
//! - a placeholder is `{name}` from a FIXED per-prompt list ([`AgentPrompt::fields`]),
//!   substituted by literal replacement. An unknown `{word}` is left exactly
//!   as written — a template is the user's text, and guessing at it would be
//!   mesimon adding a token again;
//! - `None` means mesimon's own words. There is no copy of the default on
//!   disk, so a default that improves in a later build reaches every board
//!   that never overrode it, and "put it back" is one empty field.

use serde::{Deserialize, Serialize};

/// One of the three sentences mesimon writes for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPrompt {
    /// The default branch moved past this ticket's branch and the merge
    /// cannot fast-forward: `MergeToAgent { request: Rebase }`, sent by `m`
    /// and by the merge train.
    Rebase,
    /// The branch was merged: `MergeToAgent { request: MergedNotice }`, sent
    /// by `m` and, while the notice switch is on, by the train.
    Merged,
    /// A note on the ticket changed under a working agent: `NoteToAgent`.
    NoteUpdated,
}

impl AgentPrompt {
    /// The ring, in the order the Settings list shows them: the two halves of
    /// the merge flow in the order they happen, then the note nudge.
    pub const ALL: [AgentPrompt; 3] =
        [AgentPrompt::Rebase, AgentPrompt::Merged, AgentPrompt::NoteUpdated];

    /// The row's name — what the setting IS, in the user's words.
    pub fn label(self) -> &'static str {
        match self {
            AgentPrompt::Rebase => "Rebase ask",
            AgentPrompt::Merged => "Merged notice",
            AgentPrompt::NoteUpdated => "Note nudge",
        }
    }

    /// When mesimon sends it — the row's detail while the field is closed.
    pub fn when(self) -> &'static str {
        match self {
            AgentPrompt::Rebase => "when the base branch moved past this one",
            AgentPrompt::Merged => "after this ticket's branch was merged",
            AgentPrompt::NoteUpdated => "when a note on the ticket changed",
        }
    }

    /// mesimon's own words, with every placeholder in place.
    pub fn default_text(self) -> &'static str {
        match self {
            AgentPrompt::Rebase => {
                "Rebase your current branch {branch} onto {base}, resolve any conflicts, \
                 then run the tests and fix any failures before we merge."
            }
            AgentPrompt::Merged => {
                "Your branch {branch} has been merged into {base}. The main checkout now \
                 contains this work."
            }
            AgentPrompt::NoteUpdated => {
                "Note \"{note}\" on this ticket was just updated; read_note with id {id} \
                 returns the new text."
            }
        }
    }

    /// The placeholders this prompt fills, in the order a person meets them.
    /// Nothing else is substituted — see the module note.
    pub fn fields(self) -> &'static [&'static str] {
        match self {
            AgentPrompt::Rebase | AgentPrompt::Merged => &["branch", "base"],
            AgentPrompt::NoteUpdated => &["note", "id"],
        }
    }

    /// `{branch} {base}` — the placeholder list as the editor prints it.
    pub fn fields_hint(self) -> String {
        self.fields().iter().map(|f| format!("{{{f}}}")).collect::<Vec<_>>().join(" ")
    }

    /// `columns.toml`'s and the wire's spelling, for `doctor` and for tests.
    pub fn key(self) -> &'static str {
        match self {
            AgentPrompt::Rebase => "rebase",
            AgentPrompt::Merged => "merged",
            AgentPrompt::NoteUpdated => "note_updated",
        }
    }
}

/// The board's three templates: `None` is mesimon's own words.
///
/// Three `Option<String>`s and not a map, for one reason that is about disk:
/// these ride `columns.toml` as SCALARS beside `system_prompt` and
/// `default_column`, and a map would serialize as a TOML table, which may not
/// be followed by any scalar. The enum above is what keeps the three honest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_updated: Option<String>,
}

impl PromptSet {
    /// The user's template, or `None` where mesimon's words still stand.
    pub fn custom(&self, which: AgentPrompt) -> Option<&str> {
        match which {
            AgentPrompt::Rebase => self.rebase.as_deref(),
            AgentPrompt::Merged => self.merged.as_deref(),
            AgentPrompt::NoteUpdated => self.note_updated.as_deref(),
        }
    }

    /// The template that will actually be sent — theirs if they wrote one.
    pub fn text(&self, which: AgentPrompt) -> &str {
        self.custom(which).unwrap_or_else(|| which.default_text())
    }

    pub fn is_custom(&self, which: AgentPrompt) -> bool {
        self.custom(which).is_some()
    }

    /// How many of the three are the user's — the door row's label.
    pub fn custom_count(&self) -> usize {
        AgentPrompt::ALL.iter().filter(|w| self.is_custom(**w)).count()
    }

    /// Write one, or `None` to put mesimon's words back.
    pub fn set(&mut self, which: AgentPrompt, text: Option<String>) {
        let slot = match which {
            AgentPrompt::Rebase => &mut self.rebase,
            AgentPrompt::Merged => &mut self.merged,
            AgentPrompt::NoteUpdated => &mut self.note_updated,
        };
        *slot = text;
    }

    /// The sentence itself: the template with its placeholders filled.
    ///
    /// `vars` is `(name, value)` in [`AgentPrompt::fields`] order; a name the
    /// prompt does not declare is not substituted, so a template can never
    /// reach for a value the caller has no business supplying.
    pub fn render(&self, which: AgentPrompt, vars: &[(&str, &str)]) -> String {
        let mut out = self.text(which).to_string();
        for field in which.fields() {
            let Some((_, value)) = vars.iter().find(|(n, _)| n == field) else { continue };
            out = out.replace(&format!("{{{field}}}"), value);
        }
        out
    }

    pub fn is_default(&self) -> bool {
        *self == PromptSet::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::sanitize_prompt;

    /// The defaults are what the daemon sent before this module existed,
    /// character for character — a board nobody has touched must read exactly
    /// the same to every agent it has already started.
    #[test]
    fn the_defaults_are_the_words_the_daemon_used_to_format() {
        let p = PromptSet::default();
        assert_eq!(
            p.render(AgentPrompt::Rebase, &[("branch", "msmn/T-1-x"), ("base", "main")]),
            "Rebase your current branch msmn/T-1-x onto main, resolve any conflicts, then run \
             the tests and fix any failures before we merge."
        );
        assert_eq!(
            p.render(AgentPrompt::Merged, &[("branch", "msmn/T-1-x"), ("base", "main")]),
            "Your branch msmn/T-1-x has been merged into main. The main checkout now contains \
             this work."
        );
        assert_eq!(
            p.render(AgentPrompt::NoteUpdated, &[("note", "Design"), ("id", "01ABC")]),
            "Note \"Design\" on this ticket was just updated; read_note with id 01ABC returns \
             the new text."
        );
    }

    /// Every default names every placeholder it declares, or the declared
    /// list is a lie the editor prints under the field.
    #[test]
    fn every_default_uses_every_field_it_declares() {
        for which in AgentPrompt::ALL {
            for field in which.fields() {
                assert!(
                    which.default_text().contains(&format!("{{{field}}}")),
                    "{}'s default never uses {{{field}}}",
                    which.key()
                );
            }
        }
    }

    /// A template is one line with no control characters: whatever a person
    /// types, `sanitize_prompt` is the boundary it crosses, and a default
    /// that did not already survive it would be delivered as something else.
    #[test]
    fn every_default_survives_the_prompt_sanitizer_unchanged() {
        for which in AgentPrompt::ALL {
            let text = which.default_text();
            assert_eq!(
                sanitize_prompt(text).as_deref(),
                Some(text),
                "{}'s default is not what the tty would receive",
                which.key()
            );
        }
    }

    /// The user's words win, and only the fields the prompt declares are
    /// substituted — a `{whoami}` in somebody's template is their text.
    #[test]
    fn a_custom_template_is_substituted_but_never_second_guessed() {
        let mut p = PromptSet::default();
        p.set(AgentPrompt::Rebase, Some("catch {branch} up to {base} please {whoami}".into()));
        assert!(p.is_custom(AgentPrompt::Rebase));
        assert_eq!(p.custom_count(), 1);
        assert_eq!(
            p.render(AgentPrompt::Rebase, &[("branch", "b"), ("base", "main"), ("whoami", "x")]),
            "catch b up to main please {whoami}"
        );
        // And a template that names nothing is sent as written.
        p.set(AgentPrompt::Merged, Some("done".into()));
        assert_eq!(p.render(AgentPrompt::Merged, &[("branch", "b"), ("base", "main")]), "done");
    }

    /// Putting it back is emptying the field, not copying the default in:
    /// the default a board falls back to is the BINARY's, so one that
    /// improves later reaches every board that never overrode it.
    #[test]
    fn clearing_one_restores_mesimons_words_and_writes_nothing() {
        let mut p = PromptSet::default();
        p.set(AgentPrompt::NoteUpdated, Some("a note changed".into()));
        assert!(!p.is_default());
        p.set(AgentPrompt::NoteUpdated, None);
        assert!(p.is_default(), "cleared is indistinguishable from never set");
        assert_eq!(p.text(AgentPrompt::NoteUpdated), AgentPrompt::NoteUpdated.default_text());
        assert_eq!(serde_json::to_string(&p).unwrap(), "{}", "defaults are absent, not written");
    }

    /// The keys are the disk's and the wire's: distinct, and stable.
    #[test]
    fn the_keys_are_distinct_and_serde_spells_them() {
        let mut seen = std::collections::BTreeSet::new();
        for which in AgentPrompt::ALL {
            assert!(seen.insert(which.key()), "{} twice", which.key());
            assert_eq!(
                serde_json::to_string(&which).unwrap(),
                format!("\"{}\"", which.key()),
                "serde and key() disagree about {which:?}"
            );
        }
        assert_eq!(AgentPrompt::Rebase.fields_hint(), "{branch} {base}");
    }
}
