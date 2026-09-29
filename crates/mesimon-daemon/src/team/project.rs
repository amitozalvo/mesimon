//! The board as shared objects, and the digests that say what changed.
//!
//! Pure: a `Board` in, a map of `ObjectId → RecordBody` out. Note bodies are
//! not here (they are files, read by the writer only when a note's `rev`
//! moved); everything else a member may see is. Sessions, worktrees,
//! execution policy, column settings, tags and paths are absent by
//! construction: there is no field for them in `SharedObject`.
use mesimon_core::board::{Board, Ticket};
use mesimon_core::team::{
    RecordBody, SharedNote, SharedObject, SharedTicket, BOARD_OBJECT, COLUMNS_OBJECT,
};
use mesimon_team::crypto::ObjectId;
use mesimon_team::hex;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn columns_id() -> ObjectId {
    ObjectId(COLUMNS_OBJECT)
}
pub fn board_id() -> ObjectId {
    ObjectId(BOARD_OBJECT)
}

/// `notes` off is the owner's "titles only" share: the ticket goes out with
/// no note list, so a member's board never names a body it will not get.
///
/// `me` is this device's display name. The author word goes out as
/// `member:<name>` on every machine — a ticket this machine made (`local`,
/// or an agent's) as `member:<me>`, one that arrived from a teammate as the
/// word it arrived with — so two copies of one ticket project the same
/// bytes and neither echoes the other's edit back as a change of its own
/// (T-335: the echo was clearing the teammate's name off the card).
pub fn ticket_body(t: &Ticket, notes: bool, me: &str) -> RecordBody {
    let created_by = if t.created_by.starts_with("member:") {
        t.created_by.clone()
    } else {
        format!("member:{me}")
    };
    RecordBody::new(SharedObject::Ticket(SharedTicket {
        title: t.title.clone(),
        column: t.column.clone(),
        order: t.order.clone(),
        created_at: t.created_at.clone(),
        created_by,
        archived: t.is_archived(),
        notes: if notes { t.notes.iter().map(|n| n.id).collect() } else { Vec::new() },
        deleted: false,
    }))
}

pub fn ticket_tombstone(t: &SharedTicket) -> RecordBody {
    RecordBody::new(SharedObject::Ticket(SharedTicket { deleted: true, ..t.clone() }))
}

pub fn note_body(ticket: ulid::Ulid, body: String) -> RecordBody {
    RecordBody::new(SharedObject::Note(SharedNote { ticket, body, deleted: false }))
}

pub fn note_tombstone(ticket: ulid::Ulid) -> RecordBody {
    RecordBody::new(SharedObject::Note(SharedNote { ticket, body: String::new(), deleted: true }))
}

/// The digest the writer compares to know whether an object changed.
pub fn digest(body: &RecordBody) -> String {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    hex::encode(&Sha256::digest(bytes))
}

/// Every ticket on the board as a shared object, plus the column list and
/// the board's name when this daemon owns the board. A joined board never
/// publishes the two singletons: the owner's daemon is their author.
pub fn project(
    board: &Board,
    owner: bool,
    title: &str,
    notes: bool,
    me: &str,
) -> BTreeMap<ObjectId, RecordBody> {
    let mut out = BTreeMap::new();
    for t in &board.tickets {
        out.insert(ObjectId::from(t.id), ticket_body(t, notes, me));
    }
    if owner {
        out.insert(
            columns_id(),
            RecordBody::new(SharedObject::Columns {
                names: board.columns.iter().map(|c| c.name.clone()).collect(),
            }),
        );
        out.insert(board_id(), RecordBody::new(SharedObject::Board { title: title.to_owned() }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::board::{Archived, NoteMeta};

    fn ticket(title: &str) -> Ticket {
        Ticket {
            id: ulid::Ulid::new(),
            short_key: "T-1".into(),
            title: title.into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@1".into(),
            created_by: "local".into(),
            created_from: None,
            entered_at: None,
            woke_at: None,
            manual_merge: true,
            execution_policy: Default::default(),
            tier: None,
            import_origin: None,
            envelope: None,
            raised: None,
            previous_column: None,
            workspace: Some(mesimon_core::board::WorkspaceStrategy::Worktree),
            tags: Vec::new(),
            notes: vec![NoteMeta {
                id: ulid::Ulid::new(),
                name: "desc".into(),
                rev: 1,
                created_at: "@1".into(),
                created_by: "local".into(),
                edited_at: "@1".into(),
                edited_by: "local".into(),
            }],
            archived: None,
        }
    }

    #[test]
    fn a_projection_carries_scalars_and_nothing_local() {
        let mut board = Board::with_default_columns();
        let mut t = ticket("Share it");
        t.archived =
            Some(Archived { at: "@2".into(), by: "local".into(), until: None, needs_you: false });
        let note = t.notes[0].id;
        let id = t.id;
        board.tickets.push(t);
        let objects = project(&board, true, "mesimon", true, "Amit");
        assert_eq!(objects.len(), 3);
        let text = serde_json::to_string(&objects[&ObjectId::from(id)]).unwrap();
        assert!(
            text.contains("Share it")
                && text.contains(&note.to_string())
                && text.contains("\"archived\":true")
        );
        for local in ["worktree", "manual_merge", "execution", "session", "T-1"] {
            assert!(!text.contains(local), "{local} leaked into the projection");
        }
        assert!(
            project(&board, false, "mesimon", true, "Amit").len() == 1,
            "a joined board publishes no singletons"
        );
        let withheld = project(&board, true, "mesimon", false, "Amit");
        let text = serde_json::to_string(&withheld[&ObjectId::from(id)]).unwrap();
        assert!(text.contains("Share it") && !text.contains(&note.to_string()));
    }

    #[test]
    fn digests_move_with_content_and_tombstones_differ() {
        let t = ticket("One");
        let a = digest(&ticket_body(&t, true, "Amit"));
        let mut renamed = t.clone();
        renamed.title = "Two".into();
        assert_ne!(a, digest(&ticket_body(&renamed, true, "Amit")));
        assert_eq!(a, digest(&ticket_body(&t, true, "Amit")));
        // Two copies project the same bytes: the maker's `local` and the
        // teammate's `member:Amit` are one word on the wire.
        let mut theirs = t.clone();
        theirs.created_by = "member:Amit".into();
        assert_eq!(a, digest(&ticket_body(&theirs, true, "Dana")));
        if let SharedObject::Ticket(shared) = &ticket_body(&t, true, "Amit").object {
            assert_ne!(a, digest(&ticket_tombstone(shared)));
        }
    }
}
