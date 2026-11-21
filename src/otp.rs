//! One-time codes (TOTP, RFC 6238) for saved logins. The key a site gives (a base32 text or an otpauth://totp/ address)
//! is kept next to the login's password in the system's store (passwords.rs); only the 6 to 8 digit code leaves Rust.
use sha2::{Digest, Sha256, Sha512};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }
}

/// A one-time-code key. Debug never prints the key, and it's wiped from memory when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret {
    key: Vec<u8>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({}, {} digits, {}s)", self.algorithm.name(), self.digits, self.period)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for b in self.key.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

const MOST_INPUT: usize = 2048;
const FEWEST_KEY_BYTES: usize = 10;
const MOST_KEY_BYTES: usize = 128;

impl Secret {
    /// A key as a person pastes it: base32 (any case, spaces, dashes, with or without `=` padding) or an
    /// otpauth://totp/… address. Errors are plain sentences and never repeat what was pasted.
    pub fn parse(input: &str) -> Result<Secret, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("Paste the key the site showed you (a line of letters and digits) or its otpauth:// address".into());
        }
        if input.len() > MOST_INPUT {
            return Err("That's far too long to be a one-time code key".into());
        }
        let lower = input.to_ascii_lowercase();
        if lower.starts_with("otpauth-migration:") {
            return Err("That's an export from an authenticator app, which holds several keys: add each site's own key instead".into());
        }
        if lower.starts_with("otpauth:") {
            return Self::parse_address(input);
        }
        Self::from_base32(input, Algorithm::Sha1, 6, 30)
    }

    fn parse_address(input: &str) -> Result<Secret, String> {
        let u = url::Url::parse(input).map_err(|_| "That otpauth:// address isn't complete".to_string())?;
        match u.host_str().map(str::to_ascii_lowercase).as_deref() {
            Some("totp") => {}
            Some("hotp") => return Err("That's a counter-based (HOTP) key; Valerie makes time-based (TOTP) codes only".into()),
            _ => return Err("That otpauth:// address should start with otpauth://totp/".into()),
        }
        let (mut secret, mut algorithm, mut digits, mut period) = (None, Algorithm::Sha1, 6, 30);
        for (k, v) in u.query_pairs() {
            match k.to_ascii_lowercase().as_str() {
                "secret" if secret.is_none() => secret = Some(v.into_owned()),
                "algorithm" => {
                    algorithm = match v.to_ascii_uppercase().replace('-', "").as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => return Err("That address asks for a hash Valerie doesn't know (it knows SHA1, SHA256 and SHA512)".into()),
                    }
                }
                "digits" => digits = v.trim().parse().map_err(|_| "The address's number of digits isn't a number".to_string())?,
                "period" => period = v.trim().parse().map_err(|_| "The address's period isn't a number of seconds".to_string())?,
                _ => {}
            }
        }
        let Some(secret) = secret.filter(|s| !s.trim().is_empty()) else { return Err("That otpauth:// address has no secret in it".into()) };
        Self::from_base32(&secret, algorithm, digits, period)
    }

    fn from_base32(text: &str, algorithm: Algorithm, digits: u32, period: u64) -> Result<Secret, String> {
        if !(6..=8).contains(&digits) {
            return Err("One-time codes have 6, 7 or 8 digits".into());
        }
        if !(5..=600).contains(&period) {
            return Err("A code's period should be between 5 and 600 seconds (it's usually 30)".into());
        }
        let key = base32_decode(text)?;
        if key.len() < FEWEST_KEY_BYTES {
            return Err("That key is too short: a site's key is usually 16 or 32 letters and digits".into());
        }
        if key.len() > MOST_KEY_BYTES {
            return Err("That key is too long to be a one-time code key".into());
        }
        Ok(Secret { key, algorithm, digits, period })
    }

    /// How it's kept in the store: an otpauth address with no label, read back with `parse`.
    pub fn to_stored(&self) -> String {
        format!("otpauth://totp/?secret={}&algorithm={}&digits={}&period={}", base32_encode(&self.key), self.algorithm.name(), self.digits, self.period)
    }

    /// The code for the Unix time `at` (seconds).
    pub fn code_at(&self, at: u64) -> String {
        let counter = (at / self.period).to_be_bytes();
        let mac = hmac(self.algorithm, &self.key, &counter);
        let offset = (mac[mac.len() - 1] & 0x0f) as usize;
        let bin = u32::from_be_bytes([mac[offset], mac[offset + 1], mac[offset + 2], mac[offset + 3]]) & 0x7fff_ffff;
        format!("{:0width$}", bin % 10u32.pow(self.digits), width = self.digits as usize)
    }

    /// The code now, and how many seconds it has left.
    pub fn code_now(&self) -> (String, u64) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        (self.code_at(now), self.period - now % self.period)
    }
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, forgiving about case, spaces, dashes and padding.
fn base32_decode(text: &str) -> Result<Vec<u8>, String> {
    let chars: Vec<u8> = text.bytes().filter(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'-')).collect();
    let body = {
        let end = chars.iter().rposition(|&b| b != b'=').map_or(0, |i| i + 1);
        &chars[..end]
    };
    if body.is_empty() {
        return Err("That key is empty".into());
    }
    if body.len() % 8 == 1 || body.len() % 8 == 3 || body.len() % 8 == 6 {
        return Err("That key looks cut short: check that it was copied whole".into());
    }
    let (mut out, mut buf, mut bits) = (Vec::with_capacity(body.len() * 5 / 8), 0u64, 0u32);
    for &c in body {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return Err("A key uses only the letters A to Z and the digits 2 to 7".into()),
        };
        buf = (buf << 5) | u64::from(v);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

fn base32_encode(bytes: &[u8]) -> String {
    let (mut out, mut buf, mut bits) = (String::new(), 0u64, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u64::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
        buf &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn hash(algorithm: Algorithm, data: &[u8]) -> Vec<u8> {
    match algorithm {
        Algorithm::Sha1 => sha1(data).to_vec(),
        Algorithm::Sha256 => Sha256::digest(data).to_vec(),
        Algorithm::Sha512 => Sha512::digest(data).to_vec(),
    }
}

/// HMAC (RFC 2104) over the hashes above.
fn hmac(algorithm: Algorithm, key: &[u8], message: &[u8]) -> Vec<u8> {
    let block = if algorithm == Algorithm::Sha512 { 128 } else { 64 };
    let mut k = if key.len() > block { hash(algorithm, key) } else { key.to_vec() };
    k.resize(block, 0);
    let inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).chain(message.iter().copied()).collect();
    let outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).chain(hash(algorithm, &inner)).collect();
    hash(algorithm, &outer)
}

/// SHA-1 (FIPS 180-4), only for TOTP's HMAC-SHA1: the sha2 crate doesn't have it.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, t);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_matches_known_digests() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
        assert_eq!(hex(&sha1(&vec![b'a'; 1_000_000])), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    #[test]
    fn hmac_matches_rfc_2202_and_4231() {
        assert_eq!(hex(&hmac(Algorithm::Sha1, &[0x0b; 20], b"Hi There")), "b617318655057264e28bc0b6fb378c8ef146be00");
        // A key longer than a block is hashed first.
        assert_eq!(hex(&hmac(Algorithm::Sha1, &[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First")), "aa4ae5e15272d00e95705637ce8a3b55ed402112");
        assert_eq!(hex(&hmac(Algorithm::Sha256, &[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(
            hex(&hmac(Algorithm::Sha512, &[0x0b; 20], b"Hi There")),
            "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
        );
    }

    /// RFC 6238 appendix B: every time, all three hashes, 8 digits.
    #[test]
    fn rfc_6238_test_vectors() {
        let key = |algorithm, seed: &[u8]| Secret { key: seed.to_vec(), algorithm, digits: 8, period: 30 };
        let sha1 = key(Algorithm::Sha1, b"12345678901234567890");
        let sha256 = key(Algorithm::Sha256, b"12345678901234567890123456789012");
        let sha512 = key(Algorithm::Sha512, b"1234567890123456789012345678901234567890123456789012345678901234");
        let table: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1111111111, "14050471", "67062674", "99943326"),
            (1234567890, "89005924", "91819424", "93441116"),
            (2000000000, "69279037", "90698825", "38618901"),
            (20000000000, "65353130", "77737706", "47863826"),
        ];
        for (t, a, b, c) in table {
            assert_eq!(sha1.code_at(t), a, "SHA1 at {t}");
            assert_eq!(sha256.code_at(t), b, "SHA256 at {t}");
            assert_eq!(sha512.code_at(t), c, "SHA512 at {t}");
        }
        // Six digits are the last six of the eight.
        let mut short = sha1.clone();
        short.digits = 6;
        assert_eq!(short.code_at(59), "287082");
        short.digits = 7;
        assert_eq!(short.code_at(1111111109), "7081804");
    }

    #[test]
    fn base32_is_forgiving_about_how_it_was_typed() {
        // "12345678901234567890" in base32.
        let want = b"12345678901234567890".to_vec();
        for input in [
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
            "gezdgnbvgy3tqojqgezdgnbvgy3tqojq",
            "GEZD GNBV GY3T QOJQ GEZD GNBV GY3T QOJQ",
            " gezd-gnbv-gy3t-qojq-gezd-gnbv-gy3t-qojq\n",
        ] {
            assert_eq!(base32_decode(input).unwrap(), want, "{input}");
        }
        // Padding is optional.
        assert_eq!(base32_decode("MZXW6YQ=").unwrap(), b"foob");
        assert_eq!(base32_decode("MZXW6YQ").unwrap(), b"foob");
        assert_eq!(base32_decode("MZXW6===").unwrap(), b"foo");
        assert_eq!(base32_decode("MY======").unwrap(), b"f");
        assert_eq!(base32_decode("MZXW6YTBOI======").unwrap(), b"foobar");
        // Not base32, or cut off where no whole byte could end.
        assert!(base32_decode("GEZDGNB1").is_err());
        assert!(base32_decode("GEZDGNB!").is_err());
        assert!(base32_decode("GEZDGNBVG").is_err());
        assert!(base32_decode("======").is_err());
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0xff; 37]] {
            assert_eq!(base32_decode(&base32_encode(bytes)).unwrap_or_default(), bytes);
        }
    }

    #[test]
    fn plain_keys_use_the_usual_settings() {
        let s = Secret::parse("gezd gnbv gy3t qojq gezd gnbv gy3t qojq").unwrap();
        assert_eq!((s.algorithm, s.digits, s.period), (Algorithm::Sha1, 6, 30));
        assert_eq!(s.code_at(59), "287082");
        assert!(Secret::parse("").is_err());
        assert!(Secret::parse("123456").is_err(), "a code isn't a key");
        assert!(Secret::parse("JBSWY3DP").is_err(), "five bytes is too short");
        assert!(Secret::parse(&"A".repeat(3000)).is_err());
    }

    #[test]
    fn otpauth_addresses() {
        let s = Secret::parse("otpauth://totp/Example:alice@google.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example").unwrap();
        assert_eq!((s.algorithm, s.digits, s.period), (Algorithm::Sha1, 6, 30));
        assert_eq!(s.code_at(59), "287082");
        let s = Secret::parse(
            "OTPAUTH://TOTP/ACME%20Co:john?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQGEZA&algorithm=sha256&digits=8&period=30",
        )
        .unwrap();
        assert_eq!((s.algorithm, s.digits), (Algorithm::Sha256, 8));
        assert_eq!(s.code_at(1111111109), "68084774");
        let s = Secret::parse("otpauth://totp/x?algorithm=SHA512&digits=7&period=60&secret=gezdgnbvgy3tqojq").unwrap();
        assert_eq!((s.algorithm, s.digits, s.period), (Algorithm::Sha512, 7, 60));
        // Kept and read back the same.
        assert_eq!(Secret::parse(&s.to_stored()).unwrap(), s);
        for bad in [
            "otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ&counter=1",
            "otpauth://totp/x?issuer=nobody",
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&algorithm=MD5",
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=5",
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=9",
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=six",
            "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&period=0",
            "otpauth://totp/x?secret=GEZD1NBV",
            "otpauth-migration://offline?data=abc",
            "otpauth:",
        ] {
            assert!(Secret::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nothing_secret_shows_in_debug_or_errors() {
        let s = Secret::parse("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").unwrap();
        assert!(!format!("{s:?}").contains("GEZD") && !format!("{s:?}").contains("1234"));
        let err = Secret::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=12").unwrap_err();
        assert!(!err.contains("GEZD"));
        let (code, left) = s.code_now();
        assert_eq!(code.len(), 6);
        assert!((1..=30).contains(&left));
    }
}
