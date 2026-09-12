//! Fixed-size byte arrays as lowercase hex in JSON. IDs, keys, nonces and
//! signatures all cross the wire this way; a byte array would serialize as a
//! JSON list of numbers, three times the size and unreadable in a journal.
use serde::{Deserialize, Deserializer, Serializer};

pub fn encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

pub fn decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(text.chunks_exact(2)) {
        *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

pub fn serialize<S: Serializer, const N: usize>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&encode(bytes))
}

pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(d: D) -> Result<[u8; N], D::Error> {
    let text = String::deserialize(d)?;
    decode::<N>(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("expected {} hex chars", N * 2)))
}

/// `Vec<u8>` fields (ciphertext) use the same spelling.
pub mod vec {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd hex length"));
        }
        text.as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .and_then(|p| u8::from_str_radix(p, 16).ok())
                    .ok_or_else(|| serde::de::Error::custom("invalid hex"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trips_and_rejects_bad_lengths() {
        let bytes = [0u8, 15, 255, 128];
        let text = super::encode(&bytes);
        assert_eq!(text, "000fff80");
        assert_eq!(super::decode::<4>(&text), Some(bytes));
        assert_eq!(super::decode::<3>(&text), None);
        assert_eq!(super::decode::<4>("000fff8g"), None);
    }
}
