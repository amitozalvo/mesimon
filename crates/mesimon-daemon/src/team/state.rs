//! `<state>/team.json`: this board's sharing state. Keys per epoch, the sync
//! cursor, what has been published (so the writer knows what changed), the
//! outbox of edits the relay has not accepted yet, the invites minted here
//! (their secrets, until the joiner is keyed), and the member list as last
//! seen. 0600 because of the keys.
use anyhow::{Context, Result};
use mesimon_core::team::{RecordBody, SharedMember};
use mesimon_team::crypto::{BoardId, BoardKey, DevicePublic, ObjectId, OperationId};
use mesimon_team::hex;
use mesimon_team::wire::{Member, Role, StoredRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TEAM_SCHEMA: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub revision: u64,
    pub digest: String,
    /// `ticket` | `note` | `columns` | `board`, so a published object that
    /// vanished from the board can be tombstoned as what it was.
    #[serde(default)]
    pub kind: String,
    /// The display name of the member whose record this is, when it was
    /// applied from the relay rather than sent from here (T-335). Cleared
    /// the moment this machine queues a change to the object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// Encrypted input retained across paging and restart. A publication marks
/// a verified content intent whose floor is saved before content is written.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRecord {
    pub record: StoredRecord,
    #[serde(default)]
    pub publication: Option<Published>,
    #[serde(default)]
    pub note_rev: Option<u64>,
    #[serde(default)]
    pub local_before: Option<LocalSnapshot>,
    #[serde(default)]
    pub local_after: Option<String>,
    #[serde(default)]
    pub local_intermediate: Vec<String>,
    #[serde(default)]
    pub applied_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSnapshot {
    pub digest: String,
    pub stamp: String,
}

impl PendingRecord {
    pub fn matches(&self, record: &StoredRecord) -> bool {
        self.record.object == record.object
            && self.record.revision == record.revision
            && self.record.record == record.record
    }
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

/// Identity and authority verified locally or by the invite-pinned owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedMember {
    pub public: DevicePublic,
    pub display_name: String,
    pub role: Role,
    pub active: bool,
}

impl VerifiedMember {
    pub fn shared(&self) -> SharedMember {
        SharedMember {
            device: self.public.id().to_hex(),
            sign: hex::encode(&self.public.sign),
            kex: hex::encode(&self.public.kex),
            display_name: self.display_name.clone(),
            role: self.role.word().into(),
            active: self.active,
        }
    }

    pub fn from_shared(m: &SharedMember) -> Option<Self> {
        let public = DevicePublic { sign: hex::decode(&m.sign)?, kex: hex::decode(&m.kex)? };
        let role = Role::parse(&m.role)?;
        (public.id().to_hex() == m.device).then(|| Self {
            public,
            role,
            display_name: m.display_name.clone(),
            active: m.active,
        })
    }
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
    /// Highest accepted revision per object, retained after its local data
    /// and published digest are removed. Older files use `published` as
    /// their initial floor.
    #[serde(default)]
    pub revision_floors: BTreeMap<String, u64>,
    #[serde(default)]
    pub pending_records: Vec<PendingRecord>,
    #[serde(default)]
    pub outbox: Vec<Outbound>,
    #[serde(default)]
    pub invites: Vec<PendingInvite>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub verified_members: BTreeMap<String, VerifiedMember>,
    /// Invite-pinned owner on joined boards; our own identity on owned boards.
    #[serde(default)]
    pub verified_owner: Option<String>,
    #[serde(default)]
    pub membership_revision: u64,
    /// Legacy joined boards have no independently persisted owner identity.
    #[serde(default)]
    pub needs_rejoin: bool,
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
            revision_floors: BTreeMap::new(),
            pending_records: Vec::new(),
            outbox: Vec::new(),
            invites: Vec::new(),
            members: Vec::new(),
            verified_members: BTreeMap::new(),
            verified_owner: None,
            membership_revision: 0,
            needs_rejoin: false,
        }
    }

    pub fn pin_owner(&mut self, public: DevicePublic, display_name: String) {
        let id = public.id().to_hex();
        self.verified_owner = Some(id.clone());
        self.verified_members
            .insert(id, VerifiedMember { public, display_name, role: Role::Owner, active: true });
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

    pub fn revision(&self, object: &str) -> Option<u64> {
        self.revision_floors
            .get(object)
            .copied()
            .max(self.published.get(object).map(|p| p.revision))
    }

    pub fn remember_revision(&mut self, object: &str, revision: u64) {
        let floor = self.revision(object).unwrap_or(0).max(revision);
        self.revision_floors.insert(object.to_owned(), floor);
    }

    pub fn forget_published(&mut self, object: &str) {
        if let Some(published) = self.published.remove(object) {
            self.remember_revision(object, published.revision);
        }
    }

    /// Queue an edit, one entry per object. An entry already in flight is
    /// marked dirty instead of replaced, so its receipt still matches what
    /// was sent.
    pub fn enqueue(&mut self, object: ObjectId, body: RecordBody, in_flight: Option<&str>) {
        let key = object.to_hex();
        let expected = self.revision(&key);
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
        let mut state: Self =
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if state.schema_version > TEAM_SCHEMA {
            anyhow::bail!("{} was written by a newer mesimon", path.display());
        }
        if state.board_id().is_none() {
            anyhow::bail!("{} names no board", path.display());
        }
        if state.schema_version < 2 {
            state.verified_members.clear();
            state.verified_owner = None;
            state.needs_rejoin = !state.is_owner();
            state.schema_version = TEAM_SCHEMA;
        }
        // A state written by a content-floors-only build may already have
        // schema 2 without identity pins. Version alone is not proof of trust.
        if !state.is_owner()
            && !state
                .verified_owner
                .as_ref()
                .and_then(|id| state.verified_members.get(id))
                .is_some_and(|p| p.active && p.role == Role::Owner)
        {
            state.needs_rejoin = true;
        }
        state.schema_version = TEAM_SCHEMA;
        if state.verified_members.iter().any(|(id, m)| m.public.id().to_hex() != *id) {
            anyhow::bail!("{} holds an inconsistent member identity", path.display());
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
            Published { revision: 4, digest: "d".into(), kind: "board".into(), by: None },
        );
        state.outbox.clear();
        state.enqueue(object, body("four"), None);
        assert_eq!(state.outbox[0].expected, Some(4));
    }

    #[test]
    fn legacy_rosters_never_seed_verified_authority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        let owner = mesimon_team::crypto::DeviceKeys::generate();
        for role in [Role::Owner, Role::Contributor] {
            let mut state =
                TeamState::new(BoardId::random(), role, role != Role::Owner, "Owner".into());
            state.pin_owner(owner.public(), "Owner".into());
            state.members.push(Member {
                device: owner.id(),
                public: owner.public(),
                display_name: "Owner".into(),
                role: Role::Owner,
                status: mesimon_team::wire::MemberStatus::Active,
                proof: None,
                epochs: vec![0],
            });
            state.add_key(0, &BoardKey::generate());
            state.schema_version = 1;
            state.save(&path).unwrap();
            let migrated = TeamState::load(&path).unwrap().unwrap();
            assert_eq!(migrated.schema_version, TEAM_SCHEMA);
            assert!(migrated.verified_members.is_empty());
            assert!(migrated.verified_owner.is_none());
            assert_eq!(migrated.needs_rejoin, role != Role::Owner);
            assert_eq!(migrated.keys, state.keys);
            migrated.save(&path).unwrap();
            assert_eq!(TeamState::load(&path).unwrap().unwrap(), migrated);
        }
    }

    #[test]
    fn missing_owner_trust_is_closed_at_every_schema_and_newer_state_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        for version in [1, 2, TEAM_SCHEMA] {
            let mut state =
                TeamState::new(BoardId::random(), Role::Contributor, true, "Owner".into());
            state.schema_version = version;
            state.membership_revision = 7;
            state.save(&path).unwrap();
            let loaded = TeamState::load(&path).unwrap().unwrap();
            assert!(loaded.needs_rejoin);
            assert_eq!(loaded.membership_revision, 7);
        }
        let mut state = TeamState::new(BoardId::random(), Role::Owner, false, "Owner".into());
        state.schema_version = TEAM_SCHEMA + 1;
        state.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(TeamState::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn legacy_published_revisions_survive_forgetting_and_restart() {
        let mut old = TeamState::new(BoardId::random(), Role::Contributor, true, "Amit".into());
        let object = ObjectId::random();
        old.published.insert(
            object.to_hex(),
            Published { revision: 3, digest: "old".into(), kind: "ticket".into(), by: None },
        );
        let mut json = serde_json::to_value(old).unwrap();
        json.as_object_mut().unwrap().remove("revision_floors");
        json["schema_version"] = serde_json::json!(1);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let mut state = TeamState::load(&path).unwrap().unwrap();
        assert_eq!(state.schema_version, TEAM_SCHEMA);
        assert_eq!(state.revision(&object.to_hex()), Some(3));
        state.forget_published(&object.to_hex());
        state.remember_revision(&object.to_hex(), 2);
        state.save(&path).unwrap();
        let mut restored = TeamState::load(&path).unwrap().unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["schema_version"], TEAM_SCHEMA);
        // Schema-1 binaries must refuse to write migrated floors.
        const _: () = assert!(TEAM_SCHEMA > 1);
        assert!(restored.published.is_empty());
        assert_eq!(restored.revision(&object.to_hex()), Some(3));
        restored.enqueue(object, body("restored"), None);
        assert_eq!(restored.outbox[0].expected, Some(3));
    }

    #[test]
    fn newer_sharing_state_is_refused_without_rewriting_it() {
        let mut state = TeamState::new(BoardId::random(), Role::Owner, false, "Amit".into());
        state.schema_version = TEAM_SCHEMA + 1;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        state.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(TeamState::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn state_round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("team.json");
        assert!(TeamState::load(&path).unwrap().is_none());
        let mut state = TeamState::new(BoardId::random(), Role::Contributor, true, "Amit".into());
        state.pin_owner(mesimon_team::crypto::DeviceKeys::generate().public(), "Amit".into());
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
