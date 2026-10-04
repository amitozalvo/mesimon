//! Board sharing, the pure half (T-215 v1).
//!
//! Two things live here. [`TeamInfo`] is what the snapshot carries about
//! sharing: who this device is, whether this board is shared, how the sync
//! stands, and the boards this device may open. [`SharedObject`] is the
//! plaintext inside a sealed record: the schema every member's daemon writes
//! and reads. The relay never sees it; `mesimon-team` seals it.
//!
//! Every id crosses as lowercase hex so this crate needs no crypto types.
use serde::{Deserialize, Serialize};

/// The plaintext schema version inside a sealed record. A record from a
/// newer build is kept as bytes and applied when the daemon is upgraded.
pub const RECORD_SCHEMA: u32 = 1;

/// The hosted relay (T-514): what the sign-in's relay field holds before a
/// person types. No pin: it presents a public certificate, verified against
/// the Mozilla roots the client carries. A self-hoster replaces it with their
/// own address and the pin their relay printed. This is the one place the
/// address is written; `docs/REMOTE-CONTROL.md` repeats it in prose.
///
/// It names port 443 (T-519) because an office network commonly lets out
/// only 80 and 443: the hosted relay serves Teams and the browser on that
/// one port, split by the name each connection asks for. The endpoint's
/// default port stays 8443, a self-hoster's, and the hosted relay still
/// answers there for a Mac that signed in before.
pub const HOSTED_RELAY: &str = "relay.mesimon.dev:443";

/// Where a license key for the hosted relay is bought (T-647): the sharing
/// dialog's `Get a license key` opens it.
pub const CHECKOUT_URL: &str = "https://mesimon.dev/relay";

/// Whether `relay` (as typed: `host[:port]`, maybe a space and a pin) is the
/// hosted relay, on either of its ports. A self-hosted relay mints its own
/// codes and sells nothing, so the dialog offers the checkout for this one
/// alone.
pub fn is_hosted(relay: &str) -> bool {
    let (host, _) = HOSTED_RELAY.split_once(':').unwrap_or((HOSTED_RELAY, ""));
    let typed = relay.split_whitespace().next().unwrap_or("");
    let typed = typed.split_once(':').map_or(typed, |(h, _)| h);
    typed.eq_ignore_ascii_case(host)
}

/// Whether `text` has the shape of a license key a relay accepts (T-647):
/// a relay-minted `MSMN-XXXX-XXXX-XXXX-XXXX`, or the merchant's
/// `<PREFIX>-<UUID>`. Case, dashes and spaces in a relay-minted one are
/// the reader's. Only ever a reassurance under the field: the relay is
/// the judge, so a key this does not recognise is still sent.
pub fn looks_like_license_key(text: &str) -> bool {
    let text = text.trim();
    let minted: String = text
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect();
    let crockford = |c: char| c.is_ascii_digit() || (c.is_ascii_uppercase() && !"ILOU".contains(c));
    if minted.len() == 20 && minted.starts_with("MSMN") && minted[4..].chars().all(crockford) {
        return true;
    }
    let uuid = |s: &str| {
        s.len() == 36
            && s.bytes().enumerate().all(|(i, b)| match i {
                8 | 13 | 18 | 23 => b == b'-',
                _ => b.is_ascii_hexdigit(),
            })
    };
    text.len() > 37 && text.is_char_boundary(text.len() - 37) && {
        let (prefix, tail) = text.split_at(text.len() - 37);
        !prefix.is_empty() && tail.starts_with('-') && uuid(&tail[1..])
    }
}

/// What is inside a sealed record. One object per ticket (its scalars), one
/// per note (its body), one for the column list, one for the board's own
/// name. The object id of a ticket or a note is its ULID, so nothing maps
/// local ids to shared ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharedObject {
    Ticket(SharedTicket),
    Note(SharedNote),
    Columns { names: Vec<String> },
    Board { title: String },
}

/// A ticket's shared scalars. Nothing here is a session, a worktree, an
/// execution policy or a path: those stay on the machine they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedTicket {
    pub title: String,
    pub column: String,
    /// The fractional index, shared as-is so every member orders the column
    /// the same way.
    pub order: String,
    pub created_at: String,
    /// `member:<name>` or the owner's own author word; display only.
    pub created_by: String,
    #[serde(default)]
    pub archived: bool,
    /// Note ids in display order; each body is its own object.
    #[serde(default)]
    pub notes: Vec<ulid::Ulid>,
    /// A ticket that was removed for good. Kept as a tombstone so a member
    /// that was offline does not resurrect it.
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedNote {
    pub ticket: ulid::Ulid,
    pub body: String,
    #[serde(default)]
    pub deleted: bool,
}

/// The fixed object ids of the two singletons. Ticket and note ids are
/// ULIDs, which never start with a zero byte in this century, so neither can
/// collide with a real object.
pub const COLUMNS_OBJECT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
pub const BOARD_OBJECT: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];

/// A sealed record's plaintext, versioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordBody {
    pub schema: u32,
    pub object: SharedObject,
}

impl RecordBody {
    pub fn new(object: SharedObject) -> Self {
        Self { schema: RECORD_SCHEMA, object }
    }
}

// ---- what the snapshot says ------------------------------------------------

/// Who this machine is on the relay, or nothing when not signed in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDevice {
    pub display_name: String,
    /// `host` or `host:port`, as typed.
    pub relay: String,
    /// The device id, hex.
    pub device: String,
    /// The relay accepted the registration and handed back a credential.
    /// False is an identity minted here that the relay has not admitted —
    /// a sign-in that failed or never finished — which is not signed in.
    #[serde(default)]
    pub registered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMember {
    pub device: String,
    pub display_name: String,
    /// `owner` | `contributor` | `viewer`.
    pub role: String,
    /// `active` | `revoked` | `left`.
    pub status: String,
    /// Joined, verified, and still waiting for the owner's daemon to hand
    /// them a key — or joined with a proof the owner cannot verify.
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub unverified: bool,
    /// This machine.
    #[serde(default)]
    pub me: bool,
}

/// How the sync stands. Words, not an enum, for the same reason
/// `Notice::kind` is: a newer daemon must not blank an older client.
/// `synced` | `syncing` | `offline` | `frozen` (rotation pending) | `gone`
/// (the owner unshared) | `error`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub state: String,
    /// Edits made here and not yet accepted by the relay.
    #[serde(default)]
    pub drafts: usize,
    /// Epoch ms of the last successful exchange, if any.
    #[serde(default)]
    pub synced_at_ms: Option<u64>,
    /// Why, when `state` is `error`. Mesimon's own words, never relay text.
    #[serde(default)]
    pub detail: Option<String>,
}

/// This board's sharing, when it is shared or joined.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamBoard {
    /// The board id, hex.
    pub board: String,
    /// This device's role: `owner` | `contributor` | `viewer`.
    pub role: String,
    /// True when this daemon holds the repository: a joined board on a
    /// synthetic root has no checkout, no sessions and no execution.
    #[serde(default)]
    pub repository: bool,
    #[serde(default)]
    pub members: Vec<TeamMember>,
    #[serde(default)]
    pub sync: SyncState,
    /// The most recently minted invite code, until the next one.
    #[serde(default)]
    pub invite: Option<String>,
    /// The owner's display name.
    #[serde(default)]
    pub owner_name: String,
    /// The owner shared titles, columns and order and kept the notes on
    /// their machine (`ShareBoard { notes: false }`). Absent means shared.
    #[serde(default)]
    pub notes_withheld: bool,
    /// Tickets and notes whose last accepted change came from another
    /// member, by ULID, with that member's display name (T-335): what a
    /// card's initials and a note editor's `changed elsewhere` read. An
    /// entry leaves when this machine changes the object again.
    #[serde(default)]
    pub edited_elsewhere: std::collections::BTreeMap<String, String>,
}

/// A board this device belongs to, as the relay lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamBoardSummary {
    pub board: String,
    pub role: String,
    pub owner_name: String,
    /// The local root to open it at, once joined on this machine.
    #[serde(default)]
    pub root: Option<std::path::PathBuf>,
    /// The board's own title, when this daemon has decrypted it.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamInfo {
    #[serde(default)]
    pub device: Option<TeamDevice>,
    #[serde(default)]
    pub board: Option<TeamBoard>,
    #[serde(default)]
    pub boards: Vec<TeamBoardSummary>,
    /// What the relay thread is doing for a person right now: `signing in`,
    /// `sharing`, `joining`, `inviting`, `revoking`, … Empty when idle.
    #[serde(default)]
    pub busy: Option<String>,
    /// The last thing that failed, in mesimon's words. Cleared by the next
    /// success of the same kind.
    #[serde(default)]
    pub error: Option<String>,
    /// The relay refused the sign-in for want of an access code (T-515):
    /// the dialog's code field is what answers it.
    #[serde(default)]
    pub code_required: bool,
    /// The relay refused a write because this machine's access ran out:
    /// shared boards are read-only until a code is redeemed.
    #[serde(default)]
    pub lapsed: bool,
    /// The last code entered was accepted; cleared by the next sign-in or
    /// code.
    #[serde(default)]
    pub granted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosted_relay_is_known_by_its_host() {
        assert!(is_hosted(HOSTED_RELAY));
        assert!(is_hosted("relay.mesimon.dev"));
        assert!(is_hosted("Relay.Mesimon.dev:8443"));
        assert!(!is_hosted("relay.example:8443 abcd"));
        assert!(!is_hosted("relay.mesimon.dev.example"));
        assert!(!is_hosted(""));
    }

    #[test]
    fn a_license_key_is_known_by_its_shape() {
        assert!(looks_like_license_key("MSMN-7A3K-M9Q2-XB4D-H8FN"));
        assert!(looks_like_license_key(" msmn7a3km9q2xb4dh8fn "));
        assert!(looks_like_license_key("MSMN-POLAR-6F0E0C4A-2B7D-4E3A-9C1F-0A1B2C3D4E5F"));
        assert!(looks_like_license_key("SHOP-6F0E0C4A-2B7D-4E3A-9C1F-0A1B2C3D4E5F"));
        assert!(!looks_like_license_key("MSMN-7A3K-M9Q2-XB4D"), "short");
        assert!(!looks_like_license_key("MSMN-7A3K-M9Q2-XB4D-H8FI"), "I is not Crockford");
        assert!(!looks_like_license_key("6F0E0C4A-2B7D-4E3A-9C1F-0A1B2C3D4E5F"), "no prefix");
        assert!(!looks_like_license_key("ééé-6F0E0C4A-2B7D-4E3A-9C1F-0A1B2C3D4E5"));
        assert!(!looks_like_license_key(""));
    }

    #[test]
    fn an_old_snapshot_without_team_parses_as_signed_out() {
        let info: TeamInfo = serde_json::from_str("{}").unwrap();
        assert_eq!(info, TeamInfo::default());
        assert!(info.device.is_none() && info.board.is_none());
    }

    #[test]
    fn records_carry_their_schema_and_round_trip() {
        let body = RecordBody::new(SharedObject::Ticket(SharedTicket {
            title: "Share the board".into(),
            column: "TODO".into(),
            order: "a0".into(),
            created_at: "@1".into(),
            created_by: "local".into(),
            archived: false,
            notes: vec![ulid::Ulid::nil()],
            deleted: false,
        }));
        let text = serde_json::to_string(&body).unwrap();
        assert!(text.contains("\"schema\":1"));
        assert_eq!(serde_json::from_str::<RecordBody>(&text).unwrap(), body);
        let columns =
            serde_json::to_string(&SharedObject::Columns { names: vec!["TODO".into()] }).unwrap();
        assert!(columns.starts_with("{\"kind\":\"columns\""));
    }

    #[test]
    fn the_singleton_ids_cannot_be_ulids() {
        // A ULID's first six bytes are a millisecond timestamp; the year 2000
        // already has a non-zero first byte.
        for id in [COLUMNS_OBJECT, BOARD_OBJECT] {
            assert_eq!(id[0], 0);
            assert!(ulid::Ulid::from_bytes(id).timestamp_ms() < 1_000_000_000_000);
        }
    }
}
