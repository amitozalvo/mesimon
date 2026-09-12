//! Board-key cryptography (T-332).
//!
//! One random 256-bit key per board. It is wrapped to each member's X25519
//! key by someone who holds it, and every wrap is signed by the wrapper's
//! Ed25519 key so the relay cannot inject a key of its own. Records are sealed
//! with XChaCha20-Poly1305 under the board key: the 24-byte nonce is random, so
//! nothing has to count, and the associated data binds the ciphertext to its
//! board, object and revision so the relay cannot move a record elsewhere. The
//! author signs the sealed record; readers verify against the member list.
//!
//! Removing a member mints a new key at the next epoch and re-wraps it to the
//! members that remain. Old epochs stay readable to everyone who held them:
//! a new member gets every epoch, a removed one gets nothing new. Forward
//! secrecy is deliberately absent — a board is a shared document, and a person
//! who joins it must read what is already there.
use crate::hex;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Ciphertext cap per record. A note body is 32 KiB at most and a ticket's
/// scalars are a few hundred bytes, so this is headroom, not a target.
pub const MAX_RECORD_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("signature did not verify")]
    BadSignature,
    #[error("ciphertext did not authenticate")]
    BadCiphertext,
    #[error("malformed key material")]
    BadKey,
    #[error("record exceeds {MAX_RECORD_BYTES} bytes")]
    TooLarge,
    #[error("wrapped key is for another recipient")]
    WrongRecipient,
}

macro_rules! id16 {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(#[serde(with = "hex")] pub [u8; 16]);
        impl $name {
            pub fn random() -> Self {
                use rand::RngCore;
                let mut bytes = [0u8; 16];
                rand::rngs::OsRng.fill_bytes(&mut bytes);
                Self(bytes)
            }
            pub fn to_hex(self) -> String {
                hex::encode(&self.0)
            }
            pub fn parse(text: &str) -> Option<Self> {
                hex::decode::<16>(text.trim()).map(Self)
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.to_hex())
            }
        }
    };
}

id16!(
    /// A device: the first 16 bytes of a hash over its signing key, so the
    /// relay cannot rename one device into another.
    DeviceId
);
id16!(
    /// A shared board. Minted by the relay; never derived from a repo path.
    BoardId
);
id16!(
    /// A shared object: a ticket, a note, the column list, a request thread.
    /// Tickets and notes reuse their ULID bytes, so the local id is the
    /// shared id and no mapping table exists.
    ObjectId
);
id16!(
    /// Idempotency key for one mutation. Retrying with the same id returns the
    /// original receipt; a different payload under the same id is refused.
    OperationId
);
id16!(
    /// A minted invite, by id. The secret that redeems it is never an id.
    InviteId
);

impl From<ulid::Ulid> for ObjectId {
    fn from(id: ulid::Ulid) -> Self {
        Self(id.to_bytes())
    }
}
impl From<ObjectId> for ulid::Ulid {
    fn from(id: ObjectId) -> Self {
        ulid::Ulid::from_bytes(id.0)
    }
}

/// A device's public half: what the relay stores and what members verify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DevicePublic {
    /// Ed25519 verifying key.
    #[serde(with = "hex")]
    pub sign: [u8; 32],
    /// X25519 public key.
    #[serde(with = "hex")]
    pub kex: [u8; 32],
}

impl DevicePublic {
    pub fn id(&self) -> DeviceId {
        let mut h = Sha256::new();
        h.update(b"mesimon-team device v1\0");
        h.update(self.sign);
        let digest = h.finalize();
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        DeviceId(id)
    }
    /// The bytes an invite code commits to. Eight bytes of the id: a relay
    /// that wants to substitute an owner key must find a second preimage of
    /// a 64-bit prefix before the code is redeemed.
    pub fn hint(&self) -> [u8; 8] {
        let mut hint = [0u8; 8];
        hint.copy_from_slice(&self.id().0[..8]);
        hint
    }
    fn verifying(&self) -> Result<VerifyingKey, CryptoError> {
        VerifyingKey::from_bytes(&self.sign).map_err(|_| CryptoError::BadKey)
    }
    fn verify(&self, transcript: &[u8], signature: &[u8; 64]) -> Result<(), CryptoError> {
        self.verifying()?
            .verify(transcript, &Signature::from_bytes(signature))
            .map_err(|_| CryptoError::BadSignature)
    }
}

impl std::fmt::Debug for DevicePublic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DevicePublic({})", self.id())
    }
}

/// A device's secret half. One 32-byte seed on disk (0600, in the state dir)
/// derives both keys, so there is one file to protect and one to lose.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct DeviceKeys {
    seed: [u8; 32],
    #[zeroize(skip)]
    sign: SigningKey,
    #[zeroize(skip)]
    kex: x25519_dalek::StaticSecret,
}

impl DeviceKeys {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut seed = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        Self::from_seed(seed)
    }
    pub fn from_seed(seed: [u8; 32]) -> Self {
        let hk = Hkdf::<Sha256>::new(Some(b"mesimon-team device v1"), &seed);
        let mut sign = [0u8; 32];
        let mut kex = [0u8; 32];
        // A 32-byte output can never exceed HKDF's limit; expand cannot fail.
        let _ = hk.expand(b"sign", &mut sign);
        let _ = hk.expand(b"kex", &mut kex);
        let keys = Self {
            seed,
            sign: SigningKey::from_bytes(&sign),
            kex: x25519_dalek::StaticSecret::from(kex),
        };
        sign.zeroize();
        kex.zeroize();
        keys
    }
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }
    pub fn public(&self) -> DevicePublic {
        DevicePublic {
            sign: self.sign.verifying_key().to_bytes(),
            kex: x25519_dalek::PublicKey::from(&self.kex).to_bytes(),
        }
    }
    pub fn id(&self) -> DeviceId {
        self.public().id()
    }
    fn sign(&self, transcript: &[u8]) -> [u8; 64] {
        self.sign.sign(transcript).to_bytes()
    }
}

impl std::fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DeviceKeys({})", self.id())
    }
}

/// The symmetric key every member of a board holds, one per epoch.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct BoardKey([u8; 32]);

impl BoardKey {
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self(key)
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for BoardKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoardKey([REDACTED])")
    }
}

/// A board key at one epoch, sealed to one member and signed by the member
/// who sealed it. Stored by the relay as an opaque row per (board, device,
/// epoch); the relay learns the epoch and the two device ids and nothing else.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    pub epoch: u32,
    pub recipient: DeviceId,
    pub sender: DeviceId,
    #[serde(with = "hex")]
    pub ephemeral: [u8; 32],
    #[serde(with = "hex")]
    pub nonce: [u8; 24],
    #[serde(with = "hex::vec")]
    pub ciphertext: Vec<u8>,
    #[serde(with = "hex")]
    pub signature: [u8; 64],
}

impl std::fmt::Debug for WrappedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WrappedKey")
            .field("epoch", &self.epoch)
            .field("recipient", &self.recipient)
            .field("sender", &self.sender)
            .finish_non_exhaustive()
    }
}

fn wrap_transcript(board: BoardId, w: &WrappedKey) -> Vec<u8> {
    let mut t = Transcript::new(b"mesimon-team wrap v1");
    t.bytes(&board.0);
    t.u32(w.epoch);
    t.bytes(&w.recipient.0);
    t.bytes(&w.sender.0);
    t.bytes(&w.ephemeral);
    t.bytes(&w.nonce);
    t.bytes(&w.ciphertext);
    t.finish()
}

fn wrap_key(shared: &[u8; 32], board: BoardId, epoch: u32) -> Key {
    let mut salt = Vec::with_capacity(20);
    salt.extend_from_slice(&board.0);
    salt.extend_from_slice(&epoch.to_be_bytes());
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut okm = [0u8; 32];
    let _ = hk.expand(b"mesimon-team wrap v1", &mut okm);
    let key = *Key::from_slice(&okm);
    okm.zeroize();
    key
}

/// Seal `key` for `recipient`. `sender` must hold the key; the relay stores
/// the result and any member can check who produced it.
pub fn wrap(
    key: &BoardKey,
    epoch: u32,
    board: BoardId,
    sender: &DeviceKeys,
    recipient: &DevicePublic,
) -> WrappedKey {
    use rand::RngCore;
    let ephemeral = x25519_dalek::EphemeralSecret::random_from_rng(rand::rngs::OsRng);
    let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral).to_bytes();
    let shared = ephemeral.diffie_hellman(&x25519_dalek::PublicKey::from(recipient.kex));
    let aead = XChaCha20Poly1305::new(&wrap_key(shared.as_bytes(), board, epoch));
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut w = WrappedKey {
        epoch,
        recipient: recipient.id(),
        sender: sender.id(),
        ephemeral: ephemeral_public,
        nonce,
        ciphertext: Vec::new(),
        signature: [0u8; 64],
    };
    // AAD is the transcript minus the ciphertext, i.e. everything the
    // recipient knows before opening.
    let aad = wrap_transcript(board, &w);
    w.ciphertext = aead
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: key.bytes(), aad: &aad })
        .unwrap_or_default();
    w.signature = sender.sign(&wrap_transcript(board, &w));
    w
}

/// Open a wrapped key. `sender` is the public half of whoever the relay says
/// wrapped it, and must be a member whose signature the recipient trusts.
pub fn unwrap(
    w: &WrappedKey,
    board: BoardId,
    recipient: &DeviceKeys,
    sender: &DevicePublic,
) -> Result<BoardKey, CryptoError> {
    if w.recipient != recipient.id() {
        return Err(CryptoError::WrongRecipient);
    }
    if w.sender != sender.id() {
        return Err(CryptoError::BadSignature);
    }
    sender.verify(&wrap_transcript(board, w), &w.signature)?;
    let shared = recipient.kex.diffie_hellman(&x25519_dalek::PublicKey::from(w.ephemeral));
    let aead = XChaCha20Poly1305::new(&wrap_key(shared.as_bytes(), board, w.epoch));
    let stripped = WrappedKey { ciphertext: Vec::new(), ..w.clone() };
    let aad = wrap_transcript(board, &stripped);
    let mut bytes = aead
        .decrypt(XNonce::from_slice(&w.nonce), Payload { msg: &w.ciphertext, aad: &aad })
        .map_err(|_| CryptoError::BadCiphertext)?;
    let key = <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| CryptoError::BadKey)?;
    bytes.zeroize();
    Ok(BoardKey(key))
}

/// What a sealed record is bound to. The relay enforces the same three fields
/// on its side, so a record accepted for one object at one revision can never
/// be served as another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordScope {
    pub board: BoardId,
    pub object: ObjectId,
    pub revision: u64,
}

/// A record as the relay stores it: opaque bytes plus the epoch that unlocks
/// them and the author who signed them.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedRecord {
    pub epoch: u32,
    pub author: DeviceId,
    #[serde(with = "hex")]
    pub nonce: [u8; 24],
    #[serde(with = "hex::vec")]
    pub ciphertext: Vec<u8>,
    #[serde(with = "hex")]
    pub signature: [u8; 64],
}

impl std::fmt::Debug for SealedRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealedRecord")
            .field("epoch", &self.epoch)
            .field("author", &self.author)
            .field("byte_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

fn record_transcript(scope: RecordScope, r: &SealedRecord) -> Vec<u8> {
    let mut t = Transcript::new(b"mesimon-team record v1");
    t.bytes(&scope.board.0);
    t.bytes(&scope.object.0);
    t.u64(scope.revision);
    t.u32(r.epoch);
    t.bytes(&r.author.0);
    t.bytes(&r.nonce);
    t.bytes(&r.ciphertext);
    t.finish()
}

/// Seal a plaintext record under the board key for `epoch`.
pub fn seal(
    key: &BoardKey,
    epoch: u32,
    scope: RecordScope,
    author: &DeviceKeys,
    plaintext: &[u8],
) -> Result<SealedRecord, CryptoError> {
    use rand::RngCore;
    if plaintext.len() + 16 > MAX_RECORD_BYTES {
        return Err(CryptoError::TooLarge);
    }
    let mut nonce = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut r = SealedRecord {
        epoch,
        author: author.id(),
        nonce,
        ciphertext: Vec::new(),
        signature: [0u8; 64],
    };
    let aad = record_transcript(scope, &r);
    let aead = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    r.ciphertext = aead
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
        .map_err(|_| CryptoError::TooLarge)?;
    r.signature = author.sign(&record_transcript(scope, &r));
    Ok(r)
}

/// Open a sealed record. `author` is the public half of the member the
/// record names; the caller looks it up in the member list it trusts.
pub fn open(
    key: &BoardKey,
    scope: RecordScope,
    r: &SealedRecord,
    author: &DevicePublic,
) -> Result<Vec<u8>, CryptoError> {
    if r.ciphertext.len() > MAX_RECORD_BYTES {
        return Err(CryptoError::TooLarge);
    }
    if r.author != author.id() {
        return Err(CryptoError::BadSignature);
    }
    author.verify(&record_transcript(scope, r), &r.signature)?;
    let stripped = SealedRecord { ciphertext: Vec::new(), ..r.clone() };
    let aad = record_transcript(scope, &stripped);
    let aead = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    aead.decrypt(XNonce::from_slice(&r.nonce), Payload { msg: &r.ciphertext, aad: &aad })
        .map_err(|_| CryptoError::BadCiphertext)
}

/// Length-prefixed, domain-separated byte encoding for everything that is
/// signed or used as associated data. Two transcripts are equal only when
/// every field is.
pub(crate) struct Transcript(Vec<u8>);

impl Transcript {
    pub fn new(domain: &[u8]) -> Self {
        let mut t = Self(Vec::with_capacity(256));
        t.bytes(domain);
        t
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(&(b.len() as u64).to_be_bytes());
        self.0.extend_from_slice(b);
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> RecordScope {
        RecordScope { board: BoardId::random(), object: ObjectId::random(), revision: 3 }
    }

    #[test]
    fn seed_derives_the_same_keys_every_time() {
        let a = DeviceKeys::generate();
        let b = DeviceKeys::from_seed(*a.seed());
        assert_eq!(a.public(), b.public());
        assert_eq!(a.id(), b.id());
        assert_ne!(a.id(), DeviceKeys::generate().id());
    }

    #[test]
    fn a_record_opens_only_in_its_scope_with_its_key_and_author() {
        let key = BoardKey::generate();
        let author = DeviceKeys::generate();
        let other = DeviceKeys::generate();
        let s = scope();
        let sealed = seal(&key, 0, s, &author, b"hello board").unwrap();
        assert_eq!(open(&key, s, &sealed, &author.public()).unwrap(), b"hello board");

        let moved = RecordScope { revision: 4, ..s };
        assert_eq!(open(&key, moved, &sealed, &author.public()), Err(CryptoError::BadSignature));
        assert_eq!(
            open(&BoardKey::generate(), s, &sealed, &author.public()),
            Err(CryptoError::BadCiphertext)
        );
        assert_eq!(open(&key, s, &sealed, &other.public()), Err(CryptoError::BadSignature));

        let mut tampered = sealed.clone();
        tampered.ciphertext[0] ^= 1;
        assert_eq!(open(&key, s, &tampered, &author.public()), Err(CryptoError::BadSignature));
        let mut resigned = tampered.clone();
        resigned.author = other.id();
        resigned.signature = other.sign(&record_transcript(s, &resigned));
        assert_eq!(open(&key, s, &resigned, &other.public()), Err(CryptoError::BadCiphertext));
    }

    #[test]
    fn a_wrapped_key_opens_for_its_recipient_and_no_one_else() {
        let key = BoardKey::generate();
        let board = BoardId::random();
        let owner = DeviceKeys::generate();
        let member = DeviceKeys::generate();
        let stranger = DeviceKeys::generate();
        let w = wrap(&key, 2, board, &owner, &member.public());
        assert_eq!(w.epoch, 2);
        assert_eq!(unwrap(&w, board, &member, &owner.public()).unwrap(), key);
        assert_eq!(unwrap(&w, board, &stranger, &owner.public()), Err(CryptoError::WrongRecipient));
        assert_eq!(
            unwrap(&w, BoardId::random(), &member, &owner.public()),
            Err(CryptoError::BadSignature)
        );
        // A relay that relabels the sender cannot make the member trust it.
        assert_eq!(unwrap(&w, board, &member, &stranger.public()), Err(CryptoError::BadSignature));
        let mut forged = wrap(&BoardKey::generate(), 2, board, &stranger, &member.public());
        forged.sender = owner.id();
        assert_eq!(
            unwrap(&forged, board, &member, &owner.public()),
            Err(CryptoError::BadSignature)
        );
    }

    #[test]
    fn a_removed_member_cannot_read_the_next_epoch() {
        let owner = DeviceKeys::generate();
        let epoch0 = BoardKey::generate();
        let epoch1 = BoardKey::generate();
        let s = scope();
        let old = seal(&epoch0, 0, s, &owner, b"before").unwrap();
        let new = seal(&epoch1, 1, s, &owner, b"after").unwrap();
        // The removed member keeps epoch 0 and can read what it already saw.
        assert_eq!(open(&epoch0, s, &old, &owner.public()).unwrap(), b"before");
        // With only epoch 0 in hand the new record is noise.
        assert_eq!(open(&epoch0, s, &new, &owner.public()), Err(CryptoError::BadCiphertext));
        assert_eq!(open(&epoch1, s, &new, &owner.public()).unwrap(), b"after");
    }

    #[test]
    fn sizes_are_bounded() {
        let key = BoardKey::generate();
        let author = DeviceKeys::generate();
        let big = vec![0u8; MAX_RECORD_BYTES];
        assert_eq!(seal(&key, 0, scope(), &author, &big), Err(CryptoError::TooLarge));
        let fits = vec![0u8; MAX_RECORD_BYTES - 16];
        assert!(seal(&key, 0, scope(), &author, &fits).is_ok());
    }

    #[test]
    fn ids_and_keys_serialize_as_hex_and_redact_secrets() {
        let id = ObjectId::from(ulid::Ulid::new());
        let text = serde_json::to_string(&id).unwrap();
        assert_eq!(text.len(), 34);
        assert_eq!(serde_json::from_str::<ObjectId>(&text).unwrap(), id);
        assert_eq!(format!("{:?}", BoardKey::generate()), "BoardKey([REDACTED])");
        let keys = DeviceKeys::generate();
        assert!(!format!("{keys:?}").contains(&hex::encode(keys.seed())));
    }
}
