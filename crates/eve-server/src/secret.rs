//! Passwords at rest: PBKDF2-HMAC-SHA256 with a random salt, stored as
//! `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`. Nothing stores or
//! compares a password in the clear. Shared by the game server (accounts) and
//! the dashboard program (its sign-in), with no outside crates.

use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// The algorithm name kept beside a stored hash (account."passwordAlgo").
pub const ALGO: &str = "pbkdf2-sha256";
/// OWASP's 2023 figure for PBKDF2-HMAC-SHA256.
pub const ITERATIONS: u32 = 600_000;

/// A new stored form of `password`, with a fresh random salt.
pub fn hash_password(password: &str) -> String {
    let salt = random_hex(16);
    let hash = pbkdf2_sha256(password.as_bytes(), salt.as_bytes(), ITERATIONS);
    format!("{ALGO}${ITERATIONS}${salt}${}", hex(&hash))
}

/// Whether `password` matches a stored `pbkdf2-sha256$...` form. Compares
/// without stopping at the first difference.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    let (Some(algo), Some(iterations), Some(salt), Some(hash), None) = (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let Ok(iterations) = iterations.parse::<u32>() else { return false };
    if algo != ALGO || iterations == 0 || iterations > 10_000_000 || salt.is_empty() {
        return false;
    }
    let got = hex(&pbkdf2_sha256(password.as_bytes(), salt.as_bytes(), iterations));
    same(got.as_bytes(), hash.as_bytes())
}

/// Whether a string is a stored hash this module can check.
pub fn is_hash(stored: &str) -> bool {
    stored.starts_with(&format!("{ALGO}$")) && stored.split('$').count() == 4
}

/// Compares two byte strings in time that does not depend on where they differ.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

/// Hex of `n` unpredictable bytes: SipHash keyed from the operating system's
/// random source (std's RandomState) over a counter and the clock.
pub fn random_hex(n: usize) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut out = String::with_capacity(n * 2 + 16);
    let mut i = 0u64;
    while out.len() < n * 2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(nanos);
        h.write_usize(&out as *const String as usize);
        out.push_str(&format!("{:016x}", h.finish()));
        i += 1;
    }
    out.truncate(n * 2);
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// PBKDF2 with HMAC-SHA256, one 32-byte block (RFC 8018).
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let key = Hmac::new(password);
    let mut first = salt.to_vec();
    first.extend_from_slice(&1u32.to_be_bytes());
    let mut u = key.sign(&first);
    let mut out = u;
    for _ in 1..iterations {
        u = key.sign(&u);
        for (o, x) in out.iter_mut().zip(u.iter()) {
            *o ^= x;
        }
    }
    out
}

/// HMAC-SHA256 with the key's inner and outer blocks computed once.
struct Hmac {
    inner: [u8; 64],
    outer: [u8; 64],
}

impl Hmac {
    fn new(key: &[u8]) -> Hmac {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&sha256(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = [0u8; 64];
        let mut outer = [0u8; 64];
        for i in 0..64 {
            inner[i] = block[i] ^ 0x36;
            outer[i] = block[i] ^ 0x5c;
        }
        Hmac { inner, outer }
    }

    fn sign(&self, message: &[u8]) -> [u8; 32] {
        let mut first = Vec::with_capacity(64 + message.len());
        first.extend_from_slice(&self.inner);
        first.extend_from_slice(message);
        let inner = sha256(&first);
        let mut second = [0u8; 96];
        second[..64].copy_from_slice(&self.outer);
        second[64..].copy_from_slice(&inner);
        sha256(&second)
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let bits = (data.len() as u64).wrapping_mul(8);
    let mut tail = [0u8; 128];
    let full = data.len() / 64 * 64;
    let rest = &data[full..];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let tail_len = if rest.len() < 56 { 64 } else { 128 };
    tail[tail_len - 8..tail_len].copy_from_slice(&bits.to_be_bytes());
    let mut w = [0u32; 64];
    for block in data[..full].chunks(64).chain(tail[..tail_len].chunks(64)) {
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha256(&million)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn pbkdf2_matches_the_rfc_7914_vector() {
        // RFC 7914 section 11: PBKDF2-HMAC-SHA256("passwd", "salt", 1).
        assert_eq!(hex(&pbkdf2_sha256(b"passwd", b"salt", 1)), "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc");
        // RFC 7914: ("Password", "NaCl", 80000), first 32 bytes.
        assert_eq!(hex(&pbkdf2_sha256(b"Password", b"NaCl", 80000)), "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56");
    }

    #[test]
    fn a_stored_password_checks_only_against_itself() {
        let stored = hash_password("correct horse");
        assert!(is_hash(&stored));
        assert!(verify_password("correct horse", &stored));
        assert!(!verify_password("correct horsE", &stored));
        assert!(!verify_password("correct horse", "correct horse"));
        assert_ne!(stored, hash_password("correct horse"), "every hash has its own salt");
    }
}
