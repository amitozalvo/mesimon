//! `<state>/team.json`: this board's sharing state. Keys per epoch, the sync
//! cursor, what has been published (so the writer knows what changed), the
//! outbox of edits the relay has not accepted yet, the invites minted here
//! (their secrets, until the joiner is keyed), and the member list as last
//! seen. 0600 because of the keys.
use anyhow::{Context, Result};
use mesimon_core::team::RecordBody;
use mesimon_team::crypto::{BoardId, BoardKey, ObjectId, OperationId};
use mesimon_team::hex;
use mesimon_team::wire::{Member, Role};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TEAM_SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub revision: u64,
    pub digest: String,
    /// `ticket` | `note` | `columns` | `board`, so a published object that
    /// vanished from the board can be tombstoned as what it was.
    #[serde(default)]
    pub kind: String,
}

/// One edit waiting for the relay. `body` is the plaintext as it stood when
/// the edit was made; `dirty` means the object changed again while this was
/// in flight, so it is re-projected and re-sent once this one lands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outbound {
    pub operation: String,
    pub object: String,
    pub expected: Option<u64>,
    pub body: RecordBody,
    #[serde(default)]
    pub dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingInvite {
    pub invite: String,
    pub secret: String,
    pub role: String,
    /// The code as shown, so the dialog can show it again.
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamState {
    pub schema_version: u32,
    pub board: String,
    pub role: String,
    /// A joined board on a synthetic root: no checkout, no sessions.
    #[serde(default)]
    pub content_only: bool,
    #[serde(default)]
    pub owner_name: String,
    #[serde(default)]
    pub title: Option<String>,
    /// The owner chose to keep the notes on their machine: tickets go out
    /// with an empty note list and no note body is ever sealed.
    #[serde(default)]
    pub notes_withheld: bool,
    /// epoch → key, hex.
    #[serde(default)]
    pub keys: BTreeMap<u32, String>,
    #[serde(default)]
    pub cursor: u64,
    #[serde(default)]
    pub published: BTreeMap<String, Published>,
    #[serde(default)]
    pub outbox: Vec<Outbound>,
    #[serde(default)]
    pub invites: Vec<PendingInvite>,
    #[serde(default)]
    pub members: Vec<Member>,
}

impl TeamState {
    pub fn new(board: BoardId, role: Role, content_only: bool, owner_name: String) -> Self {
        Self {
            schema_version: TEAM_SCHEMA,
            board: board.to_hex(),
            role: role.word().into(),
            content_only,
            owner_name,
            title: None,
            notes_withheld: false,
            keys: BTreeMap::new(),
            cursor: 0,
            published: BTreeMap::new(),
            outbox: Vec::new(),
            invites: Vec::new(),
            members: Vec::new(),
        }
    }

    pub fn board_id(&self) -> Option<BoardId> {
        BoardId::parse(&self.board)
    }
    pub fn role(&self) -> Role {
        Role::parse(&self.role).unwrap_or(Role::Viewer)
    }
    pub fn is_owner(&self) -> bool {
        self.role() == Role::Owner
    }
    pub fn current_epoch(&self) -> Option<u32> {
        self.keys.keys().next_back().copied()
    }
    pub fn key(&self, epoch: u32) -> Option<BoardKey> {
        self.keys.get(&epoch).and_then(|k| hex::decode::<32>(k)).map(BoardKey::from_bytes)
    }
    pub fn add_key(&mut self, epoch: u32, key: &BoardKey) {
        self.keys.insert(epoch, hex::encode(key.bytes()));
    }

    /// Queue an edit, one entry per object. An entry already in flight is
    /// marked dirty instead of replaced, so its receipt still matches what
    /// was sent.
    pub fn enqueue(&mut self, object: ObjectId, body: RecordBody, in_flight: Option<&str>) {
        let key = object.to_hex();
        let expected = self.published.get(&key).map(|p| p.revision);
        if let Some(entry) = self.outbox.iter_mut().find(|o| o.object == key) {
            if in_flight == Some(entry.operation.as_str()) {
                entry.dirty = true;
            } else {
                entry.body = body;
                entry.expected = expected;
                entry.dirty = false;
            }
            return;
        }
        self.outbox.push(Outbound {
            operation: OperationId::random().to_hex(),
            object: key,
            expected,
            body,
            dirty: false,
        });
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let state: Self =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if state.schema_version > TEAM_SCHEMA {
            anyhow::bail!("{} was written by a newer mesimon", path.display());
        }
        if state.board_id().is_none() {
            anyhow::bail!("{} names no board", path.display());
        }
        Ok(Some(state))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::store::write_atomic(path, &serde_json::to_string_pretty(self)?, 0o600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesimon_core::team::SharedObject;

    fn body(title: &str) -> RecordBody {
        RecordBody::new(SharedObject::Board { title: title.into() })
    }

    #[test]
    fn the_outbox_coalesces_per_object_and_respects_flight() {
        let mut state = TeamState::new(BoardId::random(), Role::Owner, false, "Amit".into());
        let object = ObjectId::random();
        state.enqueue(object, body("one"), None);
        state.enqueue(object, body("two"), None);
        assert_eq!(state.outbox.len(), 1);
        assert_eq!(state.outbox[0].body, body("two"));
        assert!(!state.outbox[0].dirty);
        let op = state.outbox[0].operation.clone();
        state.enqueue(object, body("three"), Some(&op));
        assert_eq!(state.outbox[0].body, body("two"), "an in-flight body is not rewritten");
        assert!(state.outbox[0].dirty);
        state.published.insert(
            object.to_hex(),
            Published { revision: 4, digest: "d".into(), kind: "board".into() },
        );
        state.outbox.clear();
        state.enqueue(object, body("four"), None);
        assert_eq!(state.outbox[0].expected, Some(4));
    }

    #[test]
    fn state_round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        assert!(TeamState::load(&path).unwrap().is_none());
        let mut state = TeamState::new(BoardId::random(), Role::Contributor, true, "Amit".into());
        state.add_key(0, &BoardKey::generate());
        state.add_key(1, &BoardKey::generate());
        state.save(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let back = TeamState::load(&path).unwrap().unwrap();
        assert_eq!(back, state);
        assert_eq!(back.current_epoch(), Some(1));
        assert_eq!(back.key(1), state.key(1));
        assert!(back.content_only && !back.is_owner());
    }
}
