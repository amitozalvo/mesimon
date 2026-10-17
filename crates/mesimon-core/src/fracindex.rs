//! Fractional indexing (LexoRank-style) over base-62 strings (D30).
//! Keys sort lexicographically; `between(a, b)` returns a key strictly between
//! `a` and `b`. Empty string means "no bound" on either side.

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn idx(c: u8) -> usize {
    // A byte off the alphabet reads as the lowest digit (T-838): an `order`
    // comes off disk unchecked, and a hand-edited `ticket.toml` or another
    // tool's key must not take the daemon's writer thread down on the next
    // filing beside it. The key minted is still a valid one; it may only
    // sort beside the foreign key rather than strictly between.
    DIGITS.iter().position(|&d| d == c).unwrap_or(0)
}

fn mid(a: usize, b: usize) -> Option<usize> {
    if b > a + 1 {
        Some((a + b) / 2)
    } else {
        None
    }
}

/// A key strictly between `a` and `b` (lexicographically). `""` = unbounded.
/// Precondition: `a < b` when both are non-empty.
pub fn between(a: &str, b: &str) -> String {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    loop {
        // Digit of a at i, or virtual minimum (one below '0').
        let da = if i < a.len() { idx(a[i]) } else { 0 };
        // Digit of b at i, or virtual maximum (one past 'z').
        let db = if i < b.len() { idx(b[i]) } else { DIGITS.len() };
        if let Some(m) = mid(da, db) {
            out.push(DIGITS[m]);
            return String::from_utf8(out).expect("ascii");
        }
        // No room at this position: copy the lower digit and continue. Generated
        // keys never end in digit 0 (mid() >= 1), so the trailing-'0' dead zone
        // of naive base-N midpointing is unreachable for keys we produced.
        out.push(DIGITS[da]);
        i += 1;
        // Safety valve: if a is a prefix of the output and both exhausted,
        // append the midpoint of the full range.
        if i > a.len().max(b.len()) + 32 {
            out.push(DIGITS[DIGITS.len() / 2]);
            return String::from_utf8(out).expect("ascii");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_unbounded() {
        let k = between("", "");
        assert!(!k.is_empty());
    }

    #[test]
    fn a_key_off_the_alphabet_mints_a_valid_key_and_never_panics() {
        // A hand-edited `order` (T-838): the daemon's writer thread files
        // beside it rather than dying on it.
        for foreign in ["a.0", "-", "é", " "] {
            for k in [between(foreign, ""), between("", foreign), between(foreign, "z")] {
                assert!(!k.is_empty());
                assert!(k.bytes().all(|b| DIGITS.contains(&b)), "{k:?}");
            }
        }
    }

    #[test]
    fn ordering_holds() {
        let a = between("", "");
        let b = between(&a, "");
        let c = between(&a, &b);
        assert!(a < c && c < b, "{a} < {c} < {b}");
    }

    #[test]
    fn append_many_after() {
        let mut prev = String::new();
        let mut keys = Vec::new();
        for _ in 0..200 {
            let k = between(&prev, "");
            assert!(k > prev, "{k} > {prev}");
            keys.push(k.clone());
            prev = k;
        }
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn insert_many_between() {
        let a = between("", "");
        let b = between(&a, "");
        let mut lo = a.clone();
        let hi = b.clone();
        for _ in 0..100 {
            let m = between(&lo, &hi);
            assert!(m > lo && m < hi, "{lo} < {m} < {hi}");
            lo = m;
        }
    }

    #[test]
    fn insert_before_first() {
        let a = between("", "");
        let z = between("", &a);
        assert!(z < a);
        let y = between("", &z);
        assert!(y < z);
    }
}
