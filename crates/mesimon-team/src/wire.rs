//! The relay protocol (T-332).
//!
//! One JSON object per line, one request per connection, over TLS. A frame
//! carries the device's bearer credential and the request; the response is
//! one line back. Every id is opaque to the relay; every payload it stores is
//! a [`SealedRecord`] or a [`WrappedKey`]. There is no field in which a
//! title, a note or a name could travel — except `display_name`, which is the
//! one piece of plaintext the relay holds, so that an owner can see who
//! redeemed an invite before handing them a key.
//!
//! The framer here is the only one: the relay server and the daemon's client
//! both call [`read_frame`] and [`write_frame`].
use crate::crypto::{
    BoardId, DeviceId, DevicePublic, InviteId, ObjectId, OperationId, SealedRecord, WrappedKey,
};
use crate::hex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};

/// A line, including its newline. A sync page is capped well under this.
pub const MAX_FRAME_BYTES: usize = 6 * 1024 * 1024;
/// Records per sync page, and the ciphertext bytes a page may carry.
pub const MAX_SYNC_RECORDS: u32 = 256;
pub const MAX_SYNC_BYTES: usize = 4 * 1024 * 1024;
/// How long an invite code stays redeemable.
pub const INVITE_TTL_SECS: u64 = 7 * 24 * 3600;

/// 256 random bits, hex. The relay stores its SHA-256 and nothing else.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Credential(String);

impl Credential {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(hex::encode(&bytes))
    }
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(text.to_ascii_lowercase()))
    }
    /// The only place the secret is spelled out: writing it to a 0600 file.
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn hash(&self) -> [u8; 32] {
        use sha2::Digest;
        sha2::Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Contributor,
    Viewer,
}

impl Role {
    pub fn word(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Contributor => "contributor",
            Role::Viewer => "viewer",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "owner" => Some(Role::Owner),
            "contributor" => Some(Role::Contributor),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }
    pub fn may_write(self) -> bool {
        matches!(self, Role::Owner | Role::Contributor)
    }
    pub fn may_administer(self) -> bool {
        matches!(self, Role::Owner)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Active,
    Revoked,
    Left,
}

impl MemberStatus {
    pub fn word(self) -> &'static str {
        match self {
            MemberStatus::Active => "active",
            MemberStatus::Revoked => "revoked",
            MemberStatus::Left => "left",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(MemberStatus::Active),
            "revoked" => Some(MemberStatus::Revoked),
            "left" => Some(MemberStatus::Left),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub device: DeviceId,
    pub public: DevicePublic,
    pub display_name: String,
    pub role: Role,
    pub status: MemberStatus,
    /// Set while the member has redeemed an invite and holds no key yet: the
    /// owner recomputes it from the invite secret before wrapping one.
    #[serde(default, with = "opt_hex32")]
    pub proof: Option<[u8; 32]>,
    /// Epochs a key has been wrapped to this member for.
    #[serde(default)]
    pub epochs: Vec<u32>,
}

mod opt_hex32 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(bytes) => s.serialize_some(&crate::hex::encode(bytes)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
        let text: Option<String> = Option::deserialize(d)?;
        text.map(|t| {
            crate::hex::decode::<32>(&t)
                .ok_or_else(|| serde::de::Error::custom("expected 64 hex chars"))
        })
        .transpose()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardSummary {
    pub board: BoardId,
    pub role: Role,
    pub head: Head,
    pub owner_name: String,
}

/// Where a board stands: the sequence number of its newest record, the
/// current key epoch, and whether writes are frozen pending a rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub seq: u64,
    pub epoch: u32,
    pub rotation_required: bool,
}

/// A durable receipt for one accepted `Put`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub object: ObjectId,
    pub revision: u64,
    pub seq: u64,
}

/// The current version of one object, as a sync page returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRecord {
    pub object: ObjectId,
    pub revision: u64,
    pub seq: u64,
    pub record: SealedRecord,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The one request without a credential: mint a device. `code` is an
    /// access code (T-515): a relay serving `--registration code` refuses
    /// without one (`CodeRequired`); a valid one, on any relay, stamps the
    /// device's grant. Absent from the frame when `None`, so a relay from
    /// before, which refuses unknown fields, still admits a codeless sign-in.
    Register {
        display_name: String,
        public: DevicePublic,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    /// An access code on a device the relay already knows (T-515): a
    /// renewal, or a friend's code on a Mac that registered before the gate.
    /// Same stamp as at `register`.
    Redeem {
        code: String,
    },
    /// When the caller's grant ends (T-522), so a host renews before its
    /// phone is refused rather than after. A read, never gated. Answered
    /// `Response::Grant { until }` in unix seconds, `None` for a grant with
    /// no end (a friend's code, an open relay, a preview device) — a new
    /// answer sent only to a caller that asked, so no client from before
    /// reads it. A relay from before answers `InvalidRequest`, and the host
    /// then renews only on a refusal, as it did.
    Grant,
    ControlInfo,
    /// Whether the relay keeps Mesophon mail for an away host (T-497). A
    /// relay from before answers `InvalidRequest` and the host collects
    /// nothing; `ControlInfo` stays as it was, so older hosts are untouched.
    ControlMail,
    Whoami,
    Boards,
    CreateBoard,
    /// Owner only. The board disappears for everyone; nothing is returned.
    Unshare {
        board: BoardId,
    },
    /// Any member but the owner.
    Leave {
        board: BoardId,
    },
    /// Owner only. The relay keeps the hash; the code shows the secret.
    MintInvite {
        board: BoardId,
        role: Role,
        #[serde(with = "hex")]
        secret_hash: [u8; 32],
    },
    /// Redeem a code. The proof binds the caller's keys to the secret so the
    /// owner can check them before wrapping a key.
    Join {
        #[serde(with = "hex")]
        secret: [u8; 12],
        #[serde(with = "hex")]
        proof: [u8; 32],
    },
    Members {
        board: BoardId,
    },
    /// Owner only. `epoch` is the board's epoch after the call: equal to the
    /// current one when keys are being handed to new members, one more when
    /// the board is rotated — and then every active member needs a key.
    PutKeys {
        board: BoardId,
        epoch: u32,
        wrapped: Vec<WrappedKey>,
    },
    MyKeys {
        board: BoardId,
    },
    /// Owner only. Freezes writes until `PutKeys` rotates the epoch.
    Revoke {
        board: BoardId,
        device: DeviceId,
    },
    Head {
        board: BoardId,
    },
    /// Create (`expected: None`) or update (`expected: Some(current)`) one
    /// object. Idempotent on `operation`.
    Put {
        board: BoardId,
        operation: OperationId,
        object: ObjectId,
        expected: Option<u64>,
        record: SealedRecord,
    },
    /// Current versions of every object whose sequence number is past
    /// `after`, oldest first.
    Sync {
        board: BoardId,
        after: u64,
        limit: u32,
    },
}

impl Request {
    pub fn board(&self) -> Option<BoardId> {
        match self {
            Request::Register { .. }
            | Request::ControlInfo
            | Request::ControlMail
            | Request::Whoami
            | Request::Boards
            | Request::CreateBoard
            | Request::Redeem { .. }
            | Request::Grant
            | Request::Join { .. } => None,
            Request::Unshare { board }
            | Request::Leave { board }
            | Request::MintInvite { board, .. }
            | Request::Members { board }
            | Request::PutKeys { board, .. }
            | Request::MyKeys { board }
            | Request::Revoke { board, .. }
            | Request::Head { board }
            | Request::Put { board, .. }
            | Request::Sync { board, .. } => Some(*board),
        }
    }
    pub fn word(&self) -> &'static str {
        match self {
            Request::Register { .. } => "register",
            Request::ControlInfo => "control_info",
            Request::ControlMail => "control_mail",
            Request::Whoami => "whoami",
            Request::Boards => "boards",
            Request::CreateBoard => "create_board",
            Request::Redeem { .. } => "redeem",
            Request::Grant => "grant",
            Request::Unshare { .. } => "unshare",
            Request::Leave { .. } => "leave",
            Request::MintInvite { .. } => "mint_invite",
            Request::Join { .. } => "join",
            Request::Members { .. } => "members",
            Request::PutKeys { .. } => "put_keys",
            Request::MyKeys { .. } => "my_keys",
            Request::Revoke { .. } => "revoke",
            Request::Head { .. } => "head",
            Request::Put { .. } => "put",
            Request::Sync { .. } => "sync",
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("kind", &self.word())
            .field("board", &self.board())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    ControlInfo { version: u32, origin: Option<String> },
    ControlMail { version: u32 },
    Grant { until: Option<u64> },
    Registered { device: DeviceId, credential: Credential },
    Device { device: DeviceId, display_name: String },
    Boards { boards: Vec<BoardSummary> },
    BoardCreated { board: BoardId },
    Ok,
    InviteMinted { invite: InviteId, expires_at: u64 },
    Joined { board: BoardId, role: Role, owner: Member },
    Members { members: Vec<Member> },
    Keys { wrapped: Vec<WrappedKey> },
    Head { head: Head },
    Accepted { receipt: Receipt },
    Records { records: Vec<StoredRecord>, next: u64, more: bool },
    Error { code: ErrorCode },
}

/// Closed set. A relay never explains itself with a free-form string, so a
/// log line cannot carry anything a request carried.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    #[error("access denied")]
    Denied,
    #[error("invalid request")]
    InvalidRequest,
    #[error("the object changed; refresh and retry")]
    StaleRevision,
    #[error("the record was sealed under an old key epoch")]
    StaleEpoch,
    #[error("same operation id, different payload")]
    OperationMismatch,
    #[error("a member was removed; the board key must be rotated first")]
    RotationRequired,
    #[error("that invite code is unknown, used or expired")]
    InviteInvalid,
    #[error("no such board or member")]
    NotFound,
    #[error("too large")]
    Capacity,
    #[error("relay unavailable")]
    Unavailable,
    /// The relay admits new devices by access code and none was given
    /// (T-515). The sharing dialog opens its code field on it.
    #[error("this relay needs an access code")]
    CodeRequired,
    /// At `register` or `redeem`: the code is not one the relay minted, is
    /// used up, or the license behind it is not active.
    #[error("that access code is unknown, used up or expired")]
    CodeInvalid,
    /// A write from a device whose access has run out. Reads still answer,
    /// so the device is read-only, never locked out.
    #[error("this machine's access has lapsed; enter a code to keep editing")]
    GrantLapsed,
    /// A code this build does not know: a relay from after it. Serde's
    /// catch-all, so the next variant added here no longer reads as a
    /// broken frame on the clients already shipped (the three above did,
    /// on every client before alpha.32: they answered `InvalidRequest`).
    #[serde(other)]
    #[error("the relay answered with a code this build does not know")]
    Unknown,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<Credential>,
    pub request: Request,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame").field("request", &self.request).finish_non_exhaustive()
    }
}

/// One line, at most [`MAX_FRAME_BYTES`]. EOF before the newline is an error,
/// never a short frame.
pub fn read_frame<S: Read>(stream: &mut S) -> Result<Vec<u8>, ErrorCode> {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|_| ErrorCode::Unavailable)?;
        if available.is_empty() {
            return Err(ErrorCode::InvalidRequest);
        }
        let length = available.iter().position(|b| *b == b'\n').map_or(available.len(), |p| p + 1);
        if bytes.len() + length > MAX_FRAME_BYTES {
            return Err(ErrorCode::Capacity);
        }
        let ended = available[length - 1] == b'\n';
        bytes.extend_from_slice(&available[..length]);
        reader.consume(length);
        if ended {
            return Ok(bytes);
        }
    }
}

pub fn write_frame<S: Write>(stream: &mut S, value: &impl Serialize) -> Result<(), ErrorCode> {
    let bytes = serde_json::to_vec(value).map_err(|_| ErrorCode::InvalidRequest)?;
    if bytes.len() + 1 > MAX_FRAME_BYTES {
        return Err(ErrorCode::Capacity);
    }
    stream
        .write_all(&bytes)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|_| ErrorCode::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{BoardKey, DeviceKeys, RecordScope};

    #[test]
    fn frames_round_trip_and_hide_the_credential() {
        let keys = DeviceKeys::generate();
        let frame = Frame {
            credential: Some(Credential::generate()),
            request: Request::Register {
                display_name: "Dana".into(),
                public: keys.public(),
                code: None,
            },
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &frame).unwrap();
        assert_eq!(buffer.last(), Some(&b'\n'));
        let back: Frame =
            serde_json::from_slice(&read_frame(&mut buffer.as_slice()).unwrap()).unwrap();
        assert_eq!(back.request, frame.request);
        assert_eq!(back.credential, frame.credential);
        assert!(!format!("{frame:?}").contains(frame.credential.as_ref().unwrap().expose()));
    }

    #[test]
    fn a_frame_without_its_newline_is_not_a_frame() {
        assert_eq!(
            read_frame(&mut b"{\"kind\":\"whoami\"}".as_slice()),
            Err(ErrorCode::InvalidRequest)
        );
        let long = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert_eq!(read_frame(&mut long.as_slice()), Err(ErrorCode::Capacity));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let text = r#"{"request":{"kind":"whoami"},"extra":1}"#;
        assert!(serde_json::from_str::<Frame>(text).is_err());
        let text = format!(
            r#"{{"request":{{"kind":"head","board":"{}","extra":1}}}}"#,
            BoardId::random().to_hex()
        );
        assert!(serde_json::from_str::<Frame>(&text).is_err());
        let text = r#"{"request":{"kind":"whoami"},"credential":null}"#;
        assert!(serde_json::from_str::<Frame>(text).unwrap().credential.is_none());
    }

    #[test]
    fn a_put_serializes_only_ciphertext_and_ids() {
        let author = DeviceKeys::generate();
        let key = BoardKey::generate();
        let board = BoardId::random();
        let object = ObjectId::random();
        let record = crate::crypto::seal(
            &key,
            0,
            RecordScope { board, object, revision: 1 },
            &author,
            b"canary-title",
        )
        .unwrap();
        let text = serde_json::to_string(&Request::Put {
            board,
            operation: OperationId::random(),
            object,
            expected: None,
            record,
        })
        .unwrap();
        assert!(!text.contains("canary"));
        assert!(text.contains(&board.to_hex()));
    }

    /// A codeless register is the frame it always was, so a relay from
    /// before the gate (which refuses unknown fields) still admits it; a
    /// code rides only when given; and an error code from a relay newer
    /// than this build parses as `Unknown`, never as a broken frame.
    #[test]
    fn the_access_code_is_optional_on_the_wire_and_new_codes_parse() {
        let keys = DeviceKeys::generate();
        let plain =
            Request::Register { display_name: "Dana".into(), public: keys.public(), code: None };
        let text = serde_json::to_string(&plain).unwrap();
        assert!(!text.contains("code"), "{text}");
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), plain);
        let coded = Request::Register {
            display_name: "Dana".into(),
            public: keys.public(),
            code: Some("MSMN-1".into()),
        };
        let text = serde_json::to_string(&coded).unwrap();
        assert!(text.contains("\"code\":\"MSMN-1\""), "{text}");
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), coded);
        assert_eq!(Request::Redeem { code: "x".into() }.word(), "redeem");
        assert_eq!(Request::Redeem { code: "x".into() }.board(), None);
        assert_eq!(serde_json::to_string(&Request::Grant).unwrap(), r#"{"kind":"grant"}"#);
        assert_eq!((Request::Grant.word(), Request::Grant.board()), ("grant", None));
        for until in [Some(1_790_000_000), None] {
            let text = serde_json::to_string(&Response::Grant { until }).unwrap();
            assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), Response::Grant { until });
        }
        for (code, text) in [
            (ErrorCode::CodeRequired, "code_required"),
            (ErrorCode::CodeInvalid, "code_invalid"),
            (ErrorCode::GrantLapsed, "grant_lapsed"),
        ] {
            assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{text}\""));
            assert_eq!(serde_json::from_str::<ErrorCode>(&format!("\"{text}\"")).unwrap(), code);
        }
        let from_the_future = r#"{"kind":"error","code":"quota_exceeded"}"#;
        assert_eq!(
            serde_json::from_str::<Response>(from_the_future).unwrap(),
            Response::Error { code: ErrorCode::Unknown }
        );
    }

    #[test]
    fn roles_and_statuses_spell_themselves() {
        for role in [Role::Owner, Role::Contributor, Role::Viewer] {
            assert_eq!(Role::parse(role.word()), Some(role));
            assert_eq!(serde_json::to_string(&role).unwrap(), format!("\"{}\"", role.word()));
        }
        for status in [MemberStatus::Active, MemberStatus::Revoked, MemberStatus::Left] {
            assert_eq!(MemberStatus::parse(status.word()), Some(status));
        }
        assert!(Role::Owner.may_administer() && !Role::Contributor.may_administer());
        assert!(Role::Contributor.may_write() && !Role::Viewer.may_write());
    }
}
