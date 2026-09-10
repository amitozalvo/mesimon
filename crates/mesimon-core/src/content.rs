//! Allowlisted content transfer and inert local materialization. These pure
//! models deliberately supply no remote identity verification, network endpoint,
//! persistence, sessions, or execution. A trusted state writer owns those seams.

use serde::{Deserialize, Serialize};

use crate::board::{
    note_name, sanitize_note, sanitize_title, ExecutionPolicy, NoteMeta, Ticket, WorkspaceStrategy,
};
use crate::{authorize, Action, Decision, Principal, Resource};

/// Opaque correlation supplied by an import adapter. It is a claim about where
/// content came from, NOT a verified principal or permission. Never use these
/// identifiers as paths or infer an author's authority from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportOrigin {
    pub source: ulid::Ulid,
    pub item: ulid::Ulid,
}

/// A bounded content projection, not a serialized local Ticket. Notes are absent
/// by default and included only through explicit selection. In particular this
/// carries no local IDs, authors, paths, tags, columns, provider settings, session
/// records, executable policy, or approval/grant fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TicketContent {
    pub title: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Bound the complete transfer as well as each individual note. These are local
/// import limits, not a Teams wire protocol or a claim about remote service limits.
pub const CONTENT_MAX_NOTES: usize = 128;
pub const CONTENT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("content import was not authorized")]
    Unauthorized,
    #[error("content exceeds the import limits or contains unsupported controls")]
    InvalidContent,
    #[error("note selection contains an unknown or duplicate note")]
    InvalidSelection,
    #[error("local import placement or allocated identities are invalid")]
    InvalidPlacement,
}

impl TicketContent {
    /// The caller reads only the explicitly selected note bodies. Ordinary local
    /// notes (including agent drafts) are not discovered or included here.
    pub fn project(
        ticket: &Ticket,
        selected: &[(ulid::Ulid, String)],
    ) -> Result<Self, ContentError> {
        let mut ids = std::collections::BTreeSet::new();
        for (id, _) in selected {
            if !ticket.notes.iter().any(|note| note.id == *id) || !ids.insert(*id) {
                return Err(ContentError::InvalidSelection);
            }
        }
        let content = Self {
            title: ticket.title.clone(),
            notes: selected.iter().map(|(_, body)| body.clone()).collect(),
        };
        content.validate()?;
        Ok(content)
    }

    /// Fail rather than silently truncate or change approved content. Transport
    /// adapters must also bound encoded input before attempting deserialization.
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.title.trim().is_empty()
            || sanitize_title(&self.title) != self.title
            || self.notes.len() > CONTENT_MAX_NOTES
            || self.notes.iter().any(|body| sanitize_note(body) != *body)
        {
            return Err(ContentError::InvalidContent);
        }
        let bytes =
            self.notes.iter().try_fold(self.title.len(), |sum, body| sum.checked_add(body.len()));
        if !bytes.is_some_and(|bytes| bytes <= CONTENT_MAX_BYTES) {
            return Err(ContentError::InvalidContent);
        }
        Ok(())
    }
}

/// Local mint facts supplied by the sole state writer, never copied from the
/// content payload. The writer must validate the destination column, reserve the
/// display key durably, and ensure the minted identities are not already in use.
/// This structure intentionally has no Deserialize implementation.
#[derive(Debug, Clone)]
pub struct ImportPlacement {
    pub id: ulid::Ulid,
    pub short_key: String,
    pub column: String,
    pub order: String,
    pub created_at: String,
}

/// Prepared data only: constructing this value does not publish, persist, spawn,
/// merge, or mutate a board. The daemon's single writer must save all note bodies
/// and ticket metadata before adding it to a live board or acknowledging intake.
#[derive(Debug)]
pub struct PreparedImport {
    pub ticket: Ticket,
    pub note_bodies: Vec<(ulid::Ulid, String)>,
}

impl PreparedImport {
    /// Owner-authorized local import seam. There is deliberately no conversion
    /// from an asserted remote identity to Local and no transport command calling
    /// this function. A future broker must establish its own verified authority.
    pub fn prepare(
        by: &Principal,
        content: TicketContent,
        origin: ImportOrigin,
        placement: ImportPlacement,
        mut next_note_id: impl FnMut() -> ulid::Ulid,
    ) -> Result<Self, ContentError> {
        if authorize(
            by,
            &Action::ImportContent,
            &Resource::Column { name: placement.column.clone() },
        ) != Decision::Allow
        {
            return Err(ContentError::Unauthorized);
        }
        content.validate()?;
        // The display key can become a directory name at persistence. Do not
        // accept a path even from an accidentally miswired local adapter.
        if placement.id == ulid::Ulid::nil()
            || !placement
                .short_key
                .strip_prefix(crate::board::KEY_PREFIX)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            || placement.column.trim().is_empty()
            || placement.order.is_empty()
            || crate::board::stamp_secs(&placement.created_at).is_none()
        {
            return Err(ContentError::InvalidPlacement);
        }
        let mut allocated = std::collections::BTreeSet::from([placement.id]);
        let mut notes = Vec::with_capacity(content.notes.len());
        let mut note_bodies = Vec::with_capacity(content.notes.len());
        for body in content.notes {
            let id = next_note_id();
            if id == ulid::Ulid::nil() || !allocated.insert(id) {
                return Err(ContentError::InvalidPlacement);
            }
            notes.push(NoteMeta {
                id,
                name: note_name(&body),
                rev: 1,
                created_at: placement.created_at.clone(),
                created_by: "imported".into(),
                edited_at: placement.created_at.clone(),
                edited_by: "imported".into(),
            });
            note_bodies.push((id, body));
        }
        Ok(Self {
            ticket: Ticket {
                id: placement.id,
                short_key: placement.short_key,
                title: content.title,
                column: placement.column,
                order: placement.order,
                entered_at: Some(placement.created_at.clone()),
                created_at: placement.created_at,
                created_by: "imported".into(),
                created_from: None,
                woke_at: None,
                manual_merge: false,
                previous_column: None,
                execution_policy: ExecutionPolicy::OwnerOnly,
                workspace: Some(WorkspaceStrategy::Worktree),
                import_origin: Some(origin),
                raised: None,
                tags: Vec::new(),
                notes,
                archived: None,
            },
            note_bodies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement() -> ImportPlacement {
        ImportPlacement {
            id: ulid::Ulid::from(1),
            short_key: "T-1".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@0".into(),
        }
    }

    fn origin() -> ImportOrigin {
        ImportOrigin { source: ulid::Ulid::from(100), item: ulid::Ulid::from(101) }
    }

    fn content() -> TicketContent {
        TicketContent {
            title: "Incoming question".into(),
            notes: vec!["# Context\nDetails".into()],
        }
    }

    fn prepared() -> PreparedImport {
        PreparedImport::prepare(&Principal::Local, content(), origin(), placement(), || {
            ulid::Ulid::from(2)
        })
        .unwrap()
    }

    #[test]
    fn projection_excludes_notes_until_explicitly_selected() {
        let import = prepared();
        let projected = TicketContent::project(&import.ticket, &[]).unwrap();
        assert!(projected.notes.is_empty());
        let encoded = serde_json::to_value(projected).unwrap();
        assert_eq!(
            encoded.as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            ["notes", "title"]
        );
        let selected = TicketContent::project(&import.ticket, &import.note_bodies).unwrap();
        assert_eq!(selected, content());
        let duplicate = vec![import.note_bodies[0].clone(), import.note_bodies[0].clone()];
        assert_eq!(
            TicketContent::project(&import.ticket, &duplicate),
            Err(ContentError::InvalidSelection)
        );
        assert_eq!(
            TicketContent::project(&import.ticket, &[(ulid::Ulid::from(99), "private".into())]),
            Err(ContentError::InvalidSelection)
        );
    }

    #[test]
    fn control_fields_cannot_enter_the_content_schema() {
        for field in [
            "workspace",
            "execution_policy",
            "sessions",
            "provider",
            "cwd",
            "created_by",
            "import_origin",
            "manual_merge",
            "grants",
        ] {
            let mut payload = serde_json::to_value(content()).unwrap();
            payload[field] = serde_json::json!("local");
            assert!(serde_json::from_value::<TicketContent>(payload).is_err(), "{field}");
        }
    }

    #[test]
    fn import_is_inert_and_never_asserts_remote_authority() {
        let import = prepared();
        let ticket = import.ticket;
        assert_eq!(ticket.execution_policy, ExecutionPolicy::OwnerOnly);
        assert_eq!(ticket.workspace_strategy(), WorkspaceStrategy::Worktree);
        assert_eq!(ticket.import_origin, Some(origin()));
        assert_eq!(ticket.created_by, "imported");
        assert!(ticket.created_from.is_none());
        assert!(!ticket.manual_merge);
        assert_eq!(ticket.notes[0].created_by, "imported");
        assert_eq!(ticket.notes[0].id, ulid::Ulid::from(2));
        assert_eq!(import.note_bodies[0].1, content().notes[0]);
        assert_eq!(ticket.id, placement().id);
    }

    #[test]
    fn content_write_authority_does_not_authorize_import() {
        for by in [
            Principal::Agent { session: uuid::Uuid::nil() },
            Principal::Automation { rule: "intake".into() },
        ] {
            let result = PreparedImport::prepare(&by, content(), origin(), placement(), || {
                panic!("unauthorized allocation")
            });
            assert_eq!(result.unwrap_err(), ContentError::Unauthorized);
        }
        // There is no caller-forgeable verified-remote principal or local import command.
        assert!(serde_json::from_str::<Principal>(r#"{"kind":"verified_remote","device":"1"}"#)
            .is_err());
        assert!(
            serde_json::from_str::<crate::command::Command>(r#"{"cmd":"import_content"}"#).is_err()
        );
    }

    #[test]
    fn malformed_or_oversized_content_is_rejected_without_truncation() {
        for invalid in [
            TicketContent { title: " ".into(), notes: vec![] },
            TicketContent { title: "x\u{1b}[2J".into(), notes: vec![] },
            TicketContent { title: "x".repeat(crate::board::TITLE_MAX_BYTES + 1), notes: vec![] },
            TicketContent {
                title: "x".into(),
                notes: vec!["x".repeat(crate::board::NOTE_MAX_BYTES + 1)],
            },
            TicketContent { title: "x".into(), notes: vec!["x".into(); CONTENT_MAX_NOTES + 1] },
            TicketContent {
                title: "x".into(),
                notes: vec!["x".repeat(crate::board::NOTE_MAX_BYTES); 33],
            },
        ] {
            assert_eq!(invalid.validate(), Err(ContentError::InvalidContent));
        }
    }

    #[test]
    fn local_paths_and_reused_note_identities_are_rejected() {
        let mut path = placement();
        path.short_key = "T-1/../../secrets".into();
        assert_eq!(
            PreparedImport::prepare(&Principal::Local, content(), origin(), path, || {
                ulid::Ulid::from(2)
            })
            .unwrap_err(),
            ContentError::InvalidPlacement
        );
        let mut repeated = content();
        repeated.notes.push("another".into());
        assert_eq!(
            PreparedImport::prepare(&Principal::Local, repeated, origin(), placement(), || {
                ulid::Ulid::from(2)
            })
            .unwrap_err(),
            ContentError::InvalidPlacement
        );
    }
}
