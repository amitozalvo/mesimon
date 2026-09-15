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
}
