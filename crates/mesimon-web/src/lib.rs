//! Small browser facade over the same Rust crypto the host uses.
use mesimon_team::{
    control::{self, Auth, Channel, Envelope, Registration, Welcome, Wire},
    crypto::{DeviceKeys, ObjectId},
    hex,
    invite::InviteCode,
    wire::Credential,
};
use wasm_bindgen::prelude::*;

fn error() -> JsValue {
    JsValue::from_str("identity, pairing, or encrypted message did not verify")
}
#[wasm_bindgen]
pub struct Browser {
    keys: DeviceKeys,
    channel: Option<Channel>,
    challenge: Option<ObjectId>,
}
#[wasm_bindgen]
impl Browser {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: Option<String>) -> Result<Browser, JsValue> {
        let keys = match seed {
            Some(s) => DeviceKeys::from_seed(hex::decode::<32>(&s).ok_or_else(error)?),
            None => DeviceKeys::generate(),
        };
        Ok(Self { keys, channel: None, challenge: None })
    }
    pub fn seed(&self) -> String {
        hex::encode(self.keys.seed())
    }
    pub fn auth(&self, credential: Option<String>, name: String) -> Result<String, JsValue> {
        let auth = match credential {
            Some(c) => {
                Auth { credential: Some(Credential::parse(&c).ok_or_else(error)?), register: None }
            }
            None => Auth {
                credential: None,
                register: Some(Registration { name, public: self.keys.public() }),
            },
        };
        serde_json::to_string(&auth).map_err(|_| error())
    }
    pub fn pair(&mut self, code: String) -> Result<String, JsValue> {
        let code = InviteCode::parse(&code).map_err(|_| error())?;
        let challenge = ObjectId::random();
        self.challenge = Some(challenge);
        serde_json::to_string(&Wire::Pair {
            challenge,
            hash: hex::encode(&code.secret_hash()),
            public: self.keys.public(),
            proof: hex::encode(&code.proof(&self.keys.public())),
        })
        .map_err(|_| error())
    }
    pub fn connect(&mut self, pin: String) -> Result<String, JsValue> {
        let w: Welcome = serde_json::from_str(&pin).map_err(|_| error())?;
        let challenge = ObjectId::random();
        self.challenge = Some(challenge);
        serde_json::to_string(&Wire::Connect {
            challenge,
            host: w.host.id(),
            board: w.board,
            public: self.keys.public(),
        })
        .map_err(|_| error())
    }
    pub fn accept(
        &mut self,
        welcome: String,
        code: Option<String>,
        pin: Option<String>,
    ) -> Result<String, JsValue> {
        let w: Welcome = serde_json::from_str(&welcome).map_err(|_| error())?;
        if self.challenge.take() != Some(w.connection) {
            return Err(error());
        }
        match (code, pin) {
            (Some(code), None) => {
                if !InviteCode::parse(&code).map_err(|_| error())?.names_owner(&w.host) {
                    return Err(error());
                }
            }
            (None, Some(pin)) => {
                let old: Welcome = serde_json::from_str(&pin).map_err(|_| error())?;
                if old.host != w.host || old.board != w.board || old.grant != w.grant {
                    return Err(error());
                }
            }
            _ => return Err(error()),
        }
        let (channel, ready) = Channel::client(&w, &self.keys).map_err(|_| error())?;
        self.channel = Some(channel);
        serde_json::to_string(&ready).map_err(|_| error())
    }
    pub fn packet(&mut self, command: String) -> Result<String, JsValue> {
        let body = serde_json::from_str(&command).map_err(|_| error())?;
        let record =
            self.channel.as_mut().ok_or_else(error)?.seal(&self.keys, body).map_err(|_| error())?;
        serde_json::to_string(&Wire::Packet { peer: String::new(), record }).map_err(|_| error())
    }
    /// Seal a ticket for the host's mailbox (T-497): a fresh key wrapped to
    /// the host this browser pinned at pairing, and the body under it.
    pub fn mail(&self, pin: String, body: String) -> Result<String, JsValue> {
        let w: Welcome = serde_json::from_str(&pin).map_err(|_| error())?;
        let body = serde_json::from_str(&body).map_err(|_| error())?;
        let envelope =
            control::seal_mail(w.board, w.grant, &self.keys, &w.host, body).map_err(|_| error())?;
        serde_json::to_string(&envelope).map_err(|_| error())
    }
    /// Open the host's receipt for one of this browser's envelopes. The pin
    /// is what makes it the host's word rather than the relay's.
    pub fn receipt(&self, pin: String, receipt: String) -> Result<String, JsValue> {
        let w: Welcome = serde_json::from_str(&pin).map_err(|_| error())?;
        let receipt: Envelope = serde_json::from_str(&receipt).map_err(|_| error())?;
        let body = control::open_receipt(w.board, w.grant, &receipt, &self.keys, &w.host)
            .map_err(|_| error())?;
        serde_json::to_string(&body).map_err(|_| error())
    }
    pub fn open(&mut self, packet: String) -> Result<String, JsValue> {
        let Wire::Packet { record, .. } = serde_json::from_str(&packet).map_err(|_| error())?
        else {
            return Err(error());
        };
        let body = self.channel.as_mut().ok_or_else(error)?.open(&record).map_err(|_| error())?;
        serde_json::to_string(&body).map_err(|_| error())
    }
}
