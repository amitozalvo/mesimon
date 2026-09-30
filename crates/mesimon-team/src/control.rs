//! Mesophon transport and connection encryption. Routing carries only opaque ids.
use crate::crypto::{
    self, BoardId, BoardKey, DeviceId, DeviceKeys, DevicePublic, ObjectId, RecordScope,
    SealedRecord, WrappedKey,
};
use crate::wire::Credential;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 256 * 1024;
pub const QUEUE: usize = 32;
/// The refusal word for mail a host's lapsed grant does not admit: a
/// `Refused` code to the browser, and (T-522) an `Error` code to the
/// collecting host, which renews on it. A host from before drops any
/// `Error` frame unread, so the relay may send it to every host.
pub const LAPSED: &str = "lapsed";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    pub credential: Option<Credential>,
    pub register: Option<Registration>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    pub name: String,
    pub public: DevicePublic,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Wire {
    Authenticated {
        device: DeviceId,
        credential: Option<Credential>,
    },
    Host {
        board: BoardId,
        invites: Vec<String>,
        devices: Vec<DeviceId>,
    },
    Pair {
        hash: String,
        public: DevicePublic,
        proof: String,
        challenge: ObjectId,
    },
    Connect {
        challenge: ObjectId,
        host: DeviceId,
        board: BoardId,
        public: DevicePublic,
    },
    Peer {
        peer: String,
        device: DeviceId,
        name: String,
        public: DevicePublic,
        proof: Option<String>,
        challenge: ObjectId,
    },
    Welcome {
        peer: String,
        welcome: Box<Welcome>,
    },
    Packet {
        peer: String,
        record: SealedRecord,
    },
    Close {
        peer: String,
    },
    Gone {
        peer: String,
    },
    Error {
        code: String,
    },
    Published,
    // ---- the mailbox (T-497): tickets for a host that may be away --------
    /// Browser → relay: keep this envelope for the board's host.
    Deposit {
        board: BoardId,
        envelope: Box<Envelope>,
    },
    /// Relay → browser: the envelope is kept. The one tick.
    Deposited {
        id: ObjectId,
    },
    /// Relay → browser: the envelope was not kept, and why.
    Refused {
        id: ObjectId,
        code: String,
    },
    /// Browser → relay: take back an envelope the host has not been sent.
    Withdraw {
        board: BoardId,
        id: ObjectId,
    },
    /// Relay → browser: whether it was taken back; never once the host has it.
    Withdrawn {
        id: ObjectId,
        removed: bool,
    },
    /// Browser → relay: what became of these envelopes. It also subscribes
    /// the connection to this board's receipts.
    Sync {
        board: BoardId,
        ids: Vec<ObjectId>,
    },
    /// Relay → browser: the answer to `Sync`, one state per id asked about.
    Mailbox {
        board: BoardId,
        items: Vec<MailState>,
    },
    /// Relay → browser: a receipt, as it arrives.
    Receipt {
        board: BoardId,
        receipt: Box<Envelope>,
    },
    /// Host → relay: this host files its board's envelopes, and wants the
    /// next ones. Sent only to a relay whose `ControlMail` answered, so an
    /// older relay never reads it.
    Collect {
        board: BoardId,
    },
    /// Relay → host: envelopes to file, oldest first; empty when none wait.
    Mail {
        items: Vec<MailItem>,
    },
    /// Host → relay: one envelope answered by its sealed receipt, which
    /// carries the envelope's id.
    Collected {
        device: DeviceId,
        receipt: Box<Envelope>,
    },
    /// Host → relay: drop one envelope unanswered: its sender holds no grant,
    /// or it does not open.
    Discard {
        device: DeviceId,
        id: ObjectId,
    },
}

/// The longest envelope the relay keeps, serialized. A ticket's title and a
/// 32 KiB description fit with room; a larger one waits for a live host.
pub const MAIL_BYTES: usize = 128 * 1024;

/// A letter the relay keeps while its reader is away (T-497): a fresh key
/// wrapped to the recipient, and one record sealed under it, both signed by
/// the sender. A browser's is a ticket for the host; the host's is the
/// receipt that answers it. The relay sees the id and the two device ids.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub id: ObjectId,
    pub wrapped: WrappedKey,
    pub record: SealedRecord,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailItem {
    pub device: DeviceId,
    pub envelope: Envelope,
}
/// Where one envelope is. `Sent` can no longer be taken back: the host has
/// seen it and will file it, at most once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailStage {
    Waiting,
    Sent,
    Answered,
    Gone,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailState {
    pub id: ObjectId,
    pub stage: MailStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Envelope>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Letter {
    domain: String,
    version: u32,
    board: BoardId,
    grant: BoardId,
    id: ObjectId,
    direction: String,
    body: Value,
}
/// Each way has its own record revision, so a ticket never opens as a
/// receipt, nor a receipt as a ticket.
#[derive(Clone, Copy)]
enum Way {
    ToHost,
    ToBrowser,
}
impl Way {
    fn word(self) -> &'static str {
        match self {
            Way::ToHost => "browser",
            Way::ToBrowser => "host",
        }
    }
    fn revision(self) -> u64 {
        match self {
            Way::ToHost => 1,
            Way::ToBrowser => 2,
        }
    }
}
fn seal_letter(
    way: Way,
    (board, grant, id): (BoardId, BoardId, ObjectId),
    sender: &DeviceKeys,
    recipient: &DevicePublic,
    body: Value,
) -> Result<Envelope, crypto::CryptoError> {
    let key = BoardKey::generate();
    let wrapped = crypto::wrap(&key, VERSION, grant, sender, recipient);
    let letter = Letter {
        domain: "mesophon-mail".into(),
        version: VERSION,
        board,
        grant,
        id,
        direction: way.word().into(),
        body,
    };
    let bytes = serde_json::to_vec(&letter).map_err(|_| crypto::CryptoError::BadCiphertext)?;
    let scope = RecordScope { board: grant, object: id, revision: way.revision() };
    let record = crypto::seal(&key, VERSION, scope, sender, &bytes)?;
    Ok(Envelope { id, wrapped, record })
}
fn open_letter(
    way: Way,
    (board, grant): (BoardId, BoardId),
    envelope: &Envelope,
    recipient: &DeviceKeys,
    sender: &DevicePublic,
) -> Result<Value, crypto::CryptoError> {
    if envelope.wrapped.epoch != VERSION || envelope.record.epoch != VERSION {
        return Err(crypto::CryptoError::BadCiphertext);
    }
    let key = crypto::unwrap(&envelope.wrapped, grant, recipient, sender)?;
    let scope = RecordScope { board: grant, object: envelope.id, revision: way.revision() };
    let bytes = crypto::open(&key, scope, &envelope.record, sender)?;
    let letter: Letter =
        serde_json::from_slice(&bytes).map_err(|_| crypto::CryptoError::BadCiphertext)?;
    if letter.domain != "mesophon-mail"
        || letter.version != VERSION
        || letter.board != board
        || letter.grant != grant
        || letter.id != envelope.id
        || letter.direction != way.word()
    {
        return Err(crypto::CryptoError::BadCiphertext);
    }
    Ok(letter.body)
}
/// A browser seals a ticket for its host, under a fresh id.
pub fn seal_mail(
    board: BoardId,
    grant: BoardId,
    browser: &DeviceKeys,
    host: &DevicePublic,
    body: Value,
) -> Result<Envelope, crypto::CryptoError> {
    seal_letter(Way::ToHost, (board, grant, ObjectId::random()), browser, host, body)
}
/// The host opens a browser's ticket; `browser` is the grant's device key.
pub fn open_mail(
    board: BoardId,
    grant: BoardId,
    envelope: &Envelope,
    host: &DeviceKeys,
    browser: &DevicePublic,
) -> Result<Value, crypto::CryptoError> {
    open_letter(Way::ToHost, (board, grant), envelope, host, browser)
}
/// The host answers one envelope; only the browser that sent it can read it.
pub fn seal_receipt(
    board: BoardId,
    grant: BoardId,
    id: ObjectId,
    host: &DeviceKeys,
    browser: &DevicePublic,
    body: Value,
) -> Result<Envelope, crypto::CryptoError> {
    seal_letter(Way::ToBrowser, (board, grant, id), host, browser, body)
}
/// A browser opens its host's receipt; `host` is the key it pinned at pairing.
pub fn open_receipt(
    board: BoardId,
    grant: BoardId,
    receipt: &Envelope,
    browser: &DeviceKeys,
    host: &DevicePublic,
) -> Result<Value, crypto::CryptoError> {
    open_letter(Way::ToBrowser, (board, grant), receipt, browser, host)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Welcome {
    pub host: DevicePublic,
    pub board: BoardId,
    pub grant: BoardId,
    pub connection: ObjectId,
    pub incarnation: ObjectId,
    pub wrapped: WrappedKey,
    pub hello: SealedRecord,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plain {
    domain: String,
    version: u32,
    board: BoardId,
    incarnation: ObjectId,
    direction: String,
    body: Value,
}

pub struct Channel {
    key: BoardKey,
    board: BoardId,
    grant: BoardId,
    connection: ObjectId,
    incarnation: ObjectId,
    remote: DevicePublic,
    host: bool,
    sent: u64,
    received: u64,
}
impl Channel {
    pub fn host(
        board: BoardId,
        grant: BoardId,
        incarnation: ObjectId,
        remote: DevicePublic,
        keys: &DeviceKeys,
        ready: Value,
        connection: ObjectId,
    ) -> Result<(Self, Welcome), crypto::CryptoError> {
        let key = BoardKey::generate();
        let wrapped = crypto::wrap(&key, VERSION, grant, keys, &remote);
        let mut c = Self {
            key,
            board,
            grant,
            connection,
            incarnation,
            remote,
            host: true,
            sent: 0,
            received: 0,
        };
        let hello = c.seal(keys, ready)?;
        let welcome =
            Welcome { host: keys.public(), board, grant, connection, incarnation, wrapped, hello };
        Ok((c, welcome))
    }
    /// The caller must pin `welcome.host` against its pairing code or stored key.
    pub fn client(w: &Welcome, keys: &DeviceKeys) -> Result<(Self, Value), crypto::CryptoError> {
        let key = crypto::unwrap(&w.wrapped, w.grant, keys, &w.host)?;
        let mut c = Self {
            key,
            board: w.board,
            grant: w.grant,
            connection: w.connection,
            incarnation: w.incarnation,
            remote: w.host,
            host: false,
            sent: 0,
            received: 0,
        };
        let hello = c.open(&w.hello)?;
        Ok((c, hello))
    }
    pub fn seal(
        &mut self,
        keys: &DeviceKeys,
        body: Value,
    ) -> Result<SealedRecord, crypto::CryptoError> {
        let next = self.sent.checked_add(1).ok_or(crypto::CryptoError::BadCiphertext)?;
        let plain = Plain {
            domain: "mesophon-control".into(),
            version: VERSION,
            board: self.board,
            incarnation: self.incarnation,
            direction: if self.host { "host" } else { "browser" }.into(),
            body,
        };
        let bytes = serde_json::to_vec(&plain).map_err(|_| crypto::CryptoError::BadCiphertext)?;
        let record = crypto::seal(
            &self.key,
            VERSION,
            RecordScope { board: self.grant, object: self.connection, revision: next },
            keys,
            &bytes,
        )?;
        self.sent = next;
        Ok(record)
    }
    pub fn open(&mut self, record: &SealedRecord) -> Result<Value, crypto::CryptoError> {
        let next = self.received.checked_add(1).ok_or(crypto::CryptoError::BadCiphertext)?;
        if record.epoch != VERSION {
            return Err(crypto::CryptoError::BadCiphertext);
        }
        let bytes = crypto::open(
            &self.key,
            RecordScope { board: self.grant, object: self.connection, revision: next },
            record,
            &self.remote,
        )?;
        let p: Plain =
            serde_json::from_slice(&bytes).map_err(|_| crypto::CryptoError::BadCiphertext)?;
        if p.domain != "mesophon-control"
            || p.version != VERSION
            || p.board != self.board
            || p.incarnation != self.incarnation
            || p.direction != if self.host { "browser" } else { "host" }
        {
            return Err(crypto::CryptoError::BadCiphertext);
        }
        self.received = next;
        Ok(p.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The host hears its lapse (T-522) on a frame every host already
    /// parses: an `Error`, which a host from before drops unread.
    #[test]
    fn the_lapsed_word_rides_an_error_frame() {
        let text = serde_json::to_string(&Wire::Error { code: LAPSED.into() }).unwrap();
        assert_eq!(text, r#"{"kind":"error","code":"lapsed"}"#);
        assert!(matches!(serde_json::from_str(&text), Ok(Wire::Error { code }) if code == LAPSED));
    }
    #[test]
    fn connection_binds_identity_scope_direction_and_sequence() {
        let host = DeviceKeys::generate();
        let browser = DeviceKeys::generate();
        let (mut a, w) = Channel::host(
            BoardId::random(),
            BoardId::random(),
            ObjectId::random(),
            browser.public(),
            &host,
            serde_json::json!({"ready":true}),
            ObjectId::random(),
        )
        .unwrap();
        let (mut b, ready) = Channel::client(&w, &browser).unwrap();
        assert_eq!(ready["ready"], true);
        let packet = b.seal(&browser, serde_json::json!({"prompt":"canary"})).unwrap();
        assert!(!serde_json::to_string(&packet).unwrap().contains("canary"));
        assert!(b.open(&packet).is_err());
        assert_eq!(a.open(&packet).unwrap()["prompt"], "canary");
        assert!(a.open(&packet).is_err());
        assert!(Channel::client(&w, &DeviceKeys::generate()).is_err());
        let mut wrong = w.clone();
        wrong.board = BoardId::random();
        assert!(Channel::client(&wrong, &browser).is_err());
        let mut wrong = w.clone();
        wrong.incarnation = ObjectId::random();
        assert!(Channel::client(&wrong, &browser).is_err());
        let (_, other) = Channel::host(
            w.board,
            w.grant,
            w.incarnation,
            browser.public(),
            &host,
            Value::Null,
            ObjectId::random(),
        )
        .unwrap();
        let (mut other, _) = Channel::client(&other, &browser).unwrap();
        assert!(other.open(&a.seal(&host, Value::Null).unwrap()).is_err());
    }

    /// The mailbox's letters (T-497): only the host opens a ticket, only the
    /// sending browser opens its receipt, each only for its board and grant,
    /// and neither opens as the other.
    #[test]
    fn letters_open_only_for_their_reader_board_grant_and_direction() {
        let host = DeviceKeys::generate();
        let browser = DeviceKeys::generate();
        let stranger = DeviceKeys::generate();
        let (board, grant) = (BoardId::random(), BoardId::random());
        let body = serde_json::json!({"title": "letter-canary"});
        let mail = seal_mail(board, grant, &browser, &host.public(), body.clone()).unwrap();
        assert!(!serde_json::to_string(&mail).unwrap().contains("letter-canary"));
        assert_eq!(open_mail(board, grant, &mail, &host, &browser.public()).unwrap(), body);
        assert!(open_mail(board, grant, &mail, &stranger, &browser.public()).is_err());
        assert!(open_mail(board, grant, &mail, &host, &stranger.public()).is_err());
        assert!(open_mail(BoardId::random(), grant, &mail, &host, &browser.public()).is_err());
        assert!(open_mail(board, BoardId::random(), &mail, &host, &browser.public()).is_err());
        let mut moved = mail.clone();
        moved.id = ObjectId::random();
        assert!(open_mail(board, grant, &moved, &host, &browser.public()).is_err());

        let answer = serde_json::json!({"result": "created", "key": "T-1"});
        let receipt =
            seal_receipt(board, grant, mail.id, &host, &browser.public(), answer.clone()).unwrap();
        assert_eq!(receipt.id, mail.id);
        assert_eq!(open_receipt(board, grant, &receipt, &browser, &host.public()).unwrap(), answer);
        assert!(open_receipt(board, grant, &receipt, &stranger, &host.public()).is_err());
        // A relay cannot answer for the host, nor turn one letter into the other.
        let forged =
            seal_receipt(board, grant, mail.id, &stranger, &browser.public(), answer).unwrap();
        assert!(open_receipt(board, grant, &forged, &browser, &host.public()).is_err());
        let back = seal_letter(
            Way::ToHost,
            (board, grant, mail.id),
            &host,
            &browser.public(),
            Value::Null,
        )
        .unwrap();
        assert!(open_receipt(board, grant, &back, &browser, &host.public()).is_err());
        assert!(serde_json::to_vec(&mail).unwrap().len() < MAIL_BYTES);
    }
}
