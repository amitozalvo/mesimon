//! The invite code (T-334 reads it on screen, T-335 types it in).
//!
//! `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`: 20 bytes in Crockford base32.
//! Twelve bytes are a one-time secret the relay knows only by hash; eight
//! bytes commit to the owner's device id. The code travels out of band (a
//! chat message, a call), which is exactly the channel a fingerprint ceremony
//! would need — so the ceremony is free. The joiner checks the owner's key
//! against the hint; the owner checks the joiner's key against a proof keyed
//! by the secret. A relay in the middle can do neither.
use crate::crypto::{DevicePublic, Transcript};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The relay computes this over a redeemed secret to find the invite.
pub fn hash_secret(secret: &[u8; 12]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"mesimon-team invite v1\0");
    h.update(secret);
    h.finalize().into()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("an invite code is 32 letters and digits, in groups of four")]
    Malformed,
}

#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct InviteCode {
    secret: [u8; 12],
    #[zeroize(skip)]
    owner_hint: [u8; 8],
}

impl InviteCode {
    /// Mint a code for a board whose owner key is `owner`.
    pub fn mint(owner: &DevicePublic) -> Self {
        use rand::RngCore;
        let mut secret = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        Self { secret, owner_hint: owner.hint() }
    }

    /// What the relay stores: it can recognise the secret without holding it.
    pub fn secret_hash(&self) -> [u8; 32] {
        hash_secret(&self.secret)
    }

    /// The secret itself, sent once to the relay when the code is redeemed.
    pub fn secret(&self) -> &[u8; 12] {
        &self.secret
    }

    /// Does the key the relay handed the joiner match what the owner typed
    /// into the code?
    pub fn names_owner(&self, owner: &DevicePublic) -> bool {
        self.owner_hint == owner.hint()
    }

    /// The joiner's proof that whoever redeemed the secret holds `joiner`.
    /// The owner, who minted the secret, recomputes it before wrapping a key.
    pub fn proof(&self, joiner: &DevicePublic) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).unwrap_or_else(|_| {
            // HMAC accepts any key length; the error branch is unreachable.
            Hmac::<Sha256>::new_from_slice(&[0u8; 12]).expect("hmac key")
        });
        let mut t = Transcript::new(b"mesimon-team join v1");
        t.bytes(&joiner.sign);
        t.bytes(&joiner.kex);
        mac.update(&t.finish());
        mac.finalize().into_bytes().into()
    }

    /// Check the endpoint proof without a timing-dependent byte comparison.
    pub fn verifies_proof(&self, joiner: &DevicePublic, proof: &[u8]) -> bool {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&self.secret) else { return false };
        let mut transcript = Transcript::new(b"mesimon-team join v1");
        transcript.bytes(&joiner.sign);
        transcript.bytes(&joiner.kex);
        mac.update(&transcript.finish());
        mac.verify_slice(proof).is_ok()
    }

    pub fn encode(&self) -> String {
        let mut bytes = [0u8; 20];
        bytes[..12].copy_from_slice(&self.secret);
        bytes[12..].copy_from_slice(&self.owner_hint);
        let mut out = String::with_capacity(39);
        let mut acc: u32 = 0;
        let mut bits = 0;
        let mut written = 0;
        for byte in bytes {
            acc = (acc << 8) | u32::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                if written > 0 && written % 4 == 0 {
                    out.push('-');
                }
                out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
                written += 1;
            }
        }
        out
    }

    /// Lenient on what people type: case, dashes, spaces, and the Crockford
    /// look-alikes (`O`→`0`, `I`/`L`→`1`) are all accepted.
    pub fn parse(text: &str) -> Result<Self, InviteError> {
        let mut acc: u32 = 0;
        let mut bits = 0;
        let mut bytes = Vec::with_capacity(20);
        let mut count = 0;
        for c in text.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let value =
                ALPHABET.iter().position(|a| *a as char == c).ok_or(InviteError::Malformed)?;
            acc = (acc << 5) | value as u32;
            bits += 5;
            count += 1;
            if bits >= 8 {
                bits -= 8;
                bytes.push(((acc >> bits) & 0xff) as u8);
            }
        }
        if count != 32 || bytes.len() != 20 {
            return Err(InviteError::Malformed);
        }
        let mut secret = [0u8; 12];
        let mut owner_hint = [0u8; 8];
        secret.copy_from_slice(&bytes[..12]);
        owner_hint.copy_from_slice(&bytes[12..]);
        Ok(Self { secret, owner_hint })
    }
}

impl std::fmt::Debug for InviteCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InviteCode([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::DeviceKeys;

    #[test]
    fn a_code_round_trips_through_what_a_person_types() {
        let owner = DeviceKeys::generate().public();
        let code = InviteCode::mint(&owner);
        let text = code.encode();
        assert_eq!(text.len(), 39);
        assert_eq!(text.split('-').count(), 8);
        assert!(text.chars().all(|c| c == '-' || ALPHABET.contains(&(c as u8))));
        assert_eq!(InviteCode::parse(&text).unwrap(), code);
        let sloppy = format!("  {} ", text.to_lowercase().replace('-', " "));
        assert_eq!(InviteCode::parse(&sloppy).unwrap(), code);
        assert_eq!(InviteCode::parse(&text[..38]), Err(InviteError::Malformed));
        assert_eq!(InviteCode::parse("not a code at all!"), Err(InviteError::Malformed));
    }

    #[test]
    fn the_code_names_its_owner_and_the_proof_names_the_joiner() {
        let owner = DeviceKeys::generate().public();
        let impostor = DeviceKeys::generate().public();
        let joiner = DeviceKeys::generate().public();
        let code = InviteCode::mint(&owner);
        assert!(code.names_owner(&owner));
        assert!(!code.names_owner(&impostor));
        let proof = code.proof(&joiner);
        assert_eq!(InviteCode::parse(&code.encode()).unwrap().proof(&joiner), proof);
        assert_ne!(code.proof(&impostor), proof);
        assert!(code.verifies_proof(&joiner, &proof));
        assert!(!code.verifies_proof(&impostor, &proof));
        assert!(!code.verifies_proof(&joiner, &[0; 32]));
        assert_ne!(InviteCode::mint(&owner).proof(&joiner), proof);
        assert_ne!(code.secret_hash(), InviteCode::mint(&owner).secret_hash());
        assert_eq!(format!("{code:?}"), "InviteCode([REDACTED])");
    }
}
