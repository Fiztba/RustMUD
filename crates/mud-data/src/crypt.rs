//! Classic DES `crypt(3)` — the traditional 13-character Unix password hash.
//!
//! Reference implementation: glibc 2.x / libxcrypt descrypt. Player files
//! store passwords as `salt[2] + hash[11]` strings in the `./0-9A-Za-z`
//! alphabet, so this module reimplements that exact function to keep
//! existing player files verifying.
//!
//! Algorithm (FIPS 46-3 DES plus the two crypt twists):
//! * the 56-bit key is the low 7 bits of the first 8 password bytes
//! (each byte shifted left once; DES discards the parity bit that lands
//! in the low position, and a NUL byte ends the key early, C-string style);
//! * the 12 salt bits perturb the E expansion — salt bit `i` swaps expanded
//! bit `i` with bit `i + 24` (0-based, MSB-first), per FreeSec/libxcrypt;
//! * the all-zeros block is DES-encrypted 25 times, and the 64-bit result
//! plus two zero pad bits is written as 11 chars of the ascii64 alphabet.
//!
//! Salt chars map through the classic table ('.'=0, '/'=1, '0'-'9'=2-11,
//! 'A'-'Z'=12-37, 'a'-'z'=38-63). Out-of-alphabet salt bytes use the same
//! arithmetic glibc and libxcrypt share internally (signed-char tiers, six
//! low bits kept). Modern libxcrypt refuses such salts outright, and salts
//! here are only ever letters of a player name, so the case does not arise.
//!
//! Verified against glibc 2.43 / libxcrypt 4.5.1.

/// Output alphabet, value 0-63: `./0-9A-Za-z` (NOT standard base64).
const ASCII64: &[u8; 64] =
    b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

// FIPS 46-3 tables. Entries are 1-based bit positions counted from the MSB
// of the input value, exactly as the standard prints them.

/// Initial permutation IP (64 -> 64).
#[rustfmt::skip]
const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10,  2,
    60, 52, 44, 36, 28, 20, 12,  4,
    62, 54, 46, 38, 30, 22, 14,  6,
    64, 56, 48, 40, 32, 24, 16,  8,
    57, 49, 41, 33, 25, 17,  9,  1,
    59, 51, 43, 35, 27, 19, 11,  3,
    61, 53, 45, 37, 29, 21, 13,  5,
    63, 55, 47, 39, 31, 23, 15,  7,
];

/// Final permutation IP⁻¹ (64 -> 64).
#[rustfmt::skip]
const FP: [u8; 64] = [
    40,  8, 48, 16, 56, 24, 64, 32,
    39,  7, 47, 15, 55, 23, 63, 31,
    38,  6, 46, 14, 54, 22, 62, 30,
    37,  5, 45, 13, 53, 21, 61, 29,
    36,  4, 44, 12, 52, 20, 60, 28,
    35,  3, 43, 11, 51, 19, 59, 27,
    34,  2, 42, 10, 50, 18, 58, 26,
    33,  1, 41,  9, 49, 17, 57, 25,
];

/// E expansion (32 -> 48); crypt's salt swaps operate on its output.
#[rustfmt::skip]
const E: [u8; 48] = [
    32,  1,  2,  3,  4,  5,
     4,  5,  6,  7,  8,  9,
     8,  9, 10, 11, 12, 13,
    12, 13, 14, 15, 16, 17,
    16, 17, 18, 19, 20, 21,
    20, 21, 22, 23, 24, 25,
    24, 25, 26, 27, 28, 29,
    28, 29, 30, 31, 32,  1,
];

/// P permutation after the S-boxes (32 -> 32).
#[rustfmt::skip]
const P: [u8; 32] = [
    16,  7, 20, 21, 29, 12, 28, 17,
     1, 15, 23, 26,  5, 18, 31, 10,
     2,  8, 24, 14, 32, 27,  3,  9,
    19, 13, 30,  6, 22, 11,  4, 25,
];

/// Permuted choice 1 (64-bit key -> 56 bits, parity dropped).
#[rustfmt::skip]
const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17,  9,
     1, 58, 50, 42, 34, 26, 18,
    10,  2, 59, 51, 43, 35, 27,
    19, 11,  3, 60, 52, 44, 36,
    63, 55, 47, 39, 31, 23, 15,
     7, 62, 54, 46, 38, 30, 22,
    14,  6, 61, 53, 45, 37, 29,
    21, 13,  5, 28, 20, 12,  4,
];

/// Permuted choice 2 (56-bit C‖D -> 48-bit round key).
#[rustfmt::skip]
const PC2: [u8; 48] = [
    14, 17, 11, 24,  1,  5,
     3, 28, 15,  6, 21, 10,
    23, 19, 12,  4, 26,  8,
    16,  7, 27, 20, 13,  2,
    41, 52, 31, 37, 47, 55,
    30, 40, 51, 45, 33, 48,
    44, 49, 39, 56, 34, 53,
    46, 42, 50, 36, 29, 32,
];

/// Per-round left-rotation amounts for the C and D key halves.
const SHIFTS: [u32; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

/// The eight S-boxes. Row = outer two bits (b1 b6), column = inner four
/// (b2 b3 b4 b5) of each 6-bit group, per FIPS 46-3.
#[rustfmt::skip]
const SBOXES: [[u8; 64]; 8] = [
    [ // S1
        14,  4, 13,  1,  2, 15, 11,  8,  3, 10,  6, 12,  5,  9,  0,  7,
         0, 15,  7,  4, 14,  2, 13,  1, 10,  6, 12, 11,  9,  5,  3,  8,
         4,  1, 14,  8, 13,  6,  2, 11, 15, 12,  9,  7,  3, 10,  5,  0,
        15, 12,  8,  2,  4,  9,  1,  7,  5, 11,  3, 14, 10,  0,  6, 13,
    ],
    [ // S2
        15,  1,  8, 14,  6, 11,  3,  4,  9,  7,  2, 13, 12,  0,  5, 10,
         3, 13,  4,  7, 15,  2,  8, 14, 12,  0,  1, 10,  6,  9, 11,  5,
         0, 14,  7, 11, 10,  4, 13,  1,  5,  8, 12,  6,  9,  3,  2, 15,
        13,  8, 10,  1,  3, 15,  4,  2, 11,  6,  7, 12,  0,  5, 14,  9,
    ],
    [ // S3
        10,  0,  9, 14,  6,  3, 15,  5,  1, 13, 12,  7, 11,  4,  2,  8,
        13,  7,  0,  9,  3,  4,  6, 10,  2,  8,  5, 14, 12, 11, 15,  1,
        13,  6,  4,  9,  8, 15,  3,  0, 11,  1,  2, 12,  5, 10, 14,  7,
         1, 10, 13,  0,  6,  9,  8,  7,  4, 15, 14,  3, 11,  5,  2, 12,
    ],
    [ // S4
         7, 13, 14,  3,  0,  6,  9, 10,  1,  2,  8,  5, 11, 12,  4, 15,
        13,  8, 11,  5,  6, 15,  0,  3,  4,  7,  2, 12,  1, 10, 14,  9,
        10,  6,  9,  0, 12, 11,  7, 13, 15,  1,  3, 14,  5,  2,  8,  4,
         3, 15,  0,  6, 10,  1, 13,  8,  9,  4,  5, 11, 12,  7,  2, 14,
    ],
    [ // S5
         2, 12,  4,  1,  7, 10, 11,  6,  8,  5,  3, 15, 13,  0, 14,  9,
        14, 11,  2, 12,  4,  7, 13,  1,  5,  0, 15, 10,  3,  9,  8,  6,
         4,  2,  1, 11, 10, 13,  7,  8, 15,  9, 12,  5,  6,  3,  0, 14,
        11,  8, 12,  7,  1, 14,  2, 13,  6, 15,  0,  9, 10,  4,  5,  3,
    ],
    [ // S6
        12,  1, 10, 15,  9,  2,  6,  8,  0, 13,  3,  4, 14,  7,  5, 11,
        10, 15,  4,  2,  7, 12,  9,  5,  6,  1, 13, 14,  0, 11,  3,  8,
         9, 14, 15,  5,  2,  8, 12,  3,  7,  0,  4, 10,  1, 13, 11,  6,
         4,  3,  2, 12,  9,  5, 15, 10, 11, 14,  1,  7,  6,  0,  8, 13,
    ],
    [ // S7
         4, 11,  2, 14, 15,  0,  8, 13,  3, 12,  9,  7,  5, 10,  6,  1,
        13,  0, 11,  7,  4,  9,  1, 10, 14,  3,  5, 12,  2, 15,  8,  6,
         1,  4, 11, 13, 12,  3,  7, 14, 10, 15,  6,  8,  0,  5,  9,  2,
         6, 11, 13,  8,  1,  4, 10,  7,  9,  5,  0, 15, 14,  2,  3, 12,
    ],
    [ // S8
        13,  2,  8,  4,  6, 15, 11,  1, 10,  9,  3, 14,  5,  0, 12,  7,
         1, 15, 13,  8, 10,  3,  7,  4, 12,  5,  6, 11,  0, 14,  9,  2,
         7, 11,  4,  1,  9, 12, 14,  2,  0,  6, 10, 13, 15,  3,  5,  8,
         2,  1, 14,  7,  4, 10,  8, 13, 15, 12,  9,  0,  3,  5,  6, 11,
    ],
];

/// Apply a FIPS-style permutation table: `input` holds `in_width`
/// significant bits (MSB-first); each table entry names the 1-based
/// source bit for the next output bit.
fn permute(input: u64, in_width: u32, table: &[u8]) -> u64 {
    let mut out = 0u64;
    for &src in table {
        out = (out << 1) | ((input >> (in_width - u32::from(src))) & 1);
    }
    out
}

/// Rotate a 28-bit key half left by `n` (n is 1 or 2).
fn rotl28(v: u32, n: u32) -> u32 {
    ((v << n) | (v >> (28 - n))) & 0x0FFF_FFFF
}

/// PC1/PC2 key schedule: sixteen 48-bit round keys from the 64-bit key.
fn subkeys(key64: u64) -> [u64; 16] {
    let cd = permute(key64, 64, &PC1);
    let mut c = (cd >> 28) as u32;
    let mut d = (cd as u32) & 0x0FFF_FFFF;
    let mut ks = [0u64; 16];
    for (k, &shift) in ks.iter_mut().zip(SHIFTS.iter()) {
        c = rotl28(c, shift);
        d = rotl28(d, shift);
        *k = permute((u64::from(c) << 28) | u64::from(d), 56, &PC2);
    }
    ks
}

/// The 12 salt bits as a 24-bit swap mask: salt bit `i` selects expanded
/// bit `i` of each 24-bit half of the E output (0-based, MSB-first) for
/// swapping — the FreeSec `0x800000 >> i` formulation.
fn salt_swap_mask(salt0: u8, salt1: u8) -> u64 {
    let bits = ascii_to_bin(salt0) | (ascii_to_bin(salt1) << 6);
    let mut mask = 0u64;
    for i in 0..12 {
        if (bits >> i) & 1 == 1 {
            mask |= 0x0080_0000 >> i;
        }
    }
    mask
}

/// Classic crypt salt-char value, total over all 256 bytes: the tiered
/// signed-char arithmetic glibc's `ascii_to_bin` macro and libxcrypt's
/// function both use, keeping the six low bits. In-alphabet bytes give
/// '.'=0, '/'=1, '0'-'9'=2-11, 'A'-'Z'=12-37, 'a'-'z'=38-63.
fn ascii_to_bin(b: u8) -> u32 {
    let c = i32::from(b as i8);
    let v = if c >= i32::from(b'a') {
        c - 59
    } else if c >= i32::from(b'A') {
        c - 53
    } else {
        c - i32::from(b'.')
    };
    (v & 0x3F) as u32
}

/// The Feistel function with crypt's salted E expansion.
fn feistel(r: u32, round_key: u64, salt_swap: u64) -> u32 {
    let mut e = permute(u64::from(r), 32, &E);
    // Swap salt-selected bits between the two 24-bit halves of E's output.
    let swap = ((e >> 24) ^ e) & salt_swap;
    e ^= swap | (swap << 24);
    e ^= round_key;
    let mut s_out = 0u32;
    for (box_no, sbox) in SBOXES.iter().enumerate() {
        let six = ((e >> (42 - 6 * box_no)) & 0x3F) as usize;
        let row = ((six >> 4) & 0b10) | (six & 1);
        let col = (six >> 1) & 0xF;
        s_out = (s_out << 4) | u32::from(sbox[row * 16 + col]);
    }
    permute(u64::from(s_out), 32, &P) as u32
}

/// One DES encryption of `block` (with crypt's salt perturbation).
fn des_encrypt(block: u64, ks: &[u64; 16], salt_swap: u64) -> u64 {
    let ip = permute(block, 64, &IP);
    let mut l = (ip >> 32) as u32;
    let mut r = ip as u32;
    for &k in ks {
        let next_r = l ^ feistel(r, k, salt_swap);
        l = r;
        r = next_r;
    }
    // Preoutput swaps the halves (R16 ‖ L16) before the final permutation.
    permute((u64::from(r) << 32) | u64::from(l), 64, &FP)
}

/// Classic DES `crypt(3)`.
/// `key` = password bytes (only the first 8 matter, low 7 bits of each);
/// `salt` = at least 2 bytes (only the first 2 matter).
/// Returns the 13-byte hash (2 salt chars + 11 hash chars), or None if
/// salt is shorter than 2 bytes.
pub fn crypt(key: &[u8], salt: &[u8]) -> Option<[u8; 13]> {
    if salt.len() < 2 {
        return None;
    }

    // 64-bit DES key: first 8 key bytes, each shifted left once (high bit
    // out, zero parity bit in). A NUL ends the key early — the password is
    // read as a NUL-terminated string — but bytes whose LOW seven bits
    // are zero (e.g. 0x80) do not terminate; they contribute zero bits.
    let mut key64 = 0u64;
    for (i, &b) in key.iter().take(8).enumerate() {
        if b == 0 {
            break;
        }
        key64 |= u64::from(b << 1) << (56 - 8 * i);
    }

    let ks = subkeys(key64);
    let salt_swap = salt_swap_mask(salt[0], salt[1]);

    // 25 iterations of DES over the all-zeros block.
    let mut block = 0u64;
    for _ in 0..25 {
        block = des_encrypt(block, &ks, salt_swap);
    }

    // Output: the salt bytes echoed as given, then the 64 result bits plus
    // two zero pad bits as 11 ascii64 chars, 6 bits each, MSB-first.
    let mut out = [0u8; 13];
    out[0] = salt[0];
    out[1] = salt[1];
    let ext = u128::from(block) << 2;
    for (i, ch) in out[2..].iter_mut().enumerate() {
        *ch = ASCII64[((ext >> (60 - 6 * i)) & 0x3F) as usize];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Undo the generator's escaping: `\xNN` hex bytes and `\\`.
    fn unescape(field: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut bytes = field.bytes();
        while let Some(b) = bytes.next() {
            if b != b'\\' {
                out.push(b);
                continue;
            }
            match bytes.next() {
                Some(b'\\') => out.push(b'\\'),
                Some(b'x') => {
                    let hi = bytes.next().expect("truncated \\x escape");
                    let lo = bytes.next().expect("truncated \\x escape");
                    let hex = [hi, lo];
                    let hex = std::str::from_utf8(&hex).unwrap();
                    out.push(u8::from_str_radix(hex, 16).expect("bad \\x escape"));
                }
                other => panic!("unknown escape {other:?} in vector file"),
            }
        }
        out
    }

    /// Inline safety net: a handful of vectors copied directly from
    /// crypt-vectors.txt so a lost golden file cannot silently pass.
    #[test]
    fn known_answers() {
        let cases: [(&[u8], &[u8], &[u8; 13]); 4] = [
            (b"password", b"Bo", b"BodDv430F5Nhs"),
            (b"hello", b"Fi", b"FidZ9Le.Gq51M"),
            (b"", b"ab", b"abmF1QH4PEr.E"),
            (b"test", b"..", b"..9sjyf8zL76k"),
        ];
        for (key, salt, want) in cases {
            assert_eq!(crypt(key, salt).as_ref(), Some(want));
        }
    }

    /// The classic FIPS worked example pins the DES core (tables, key
    /// schedule, rounds) independently of the crypt wrapper.
    #[test]
    fn des_core_known_answer() {
        let ks = subkeys(0x1334_5779_9BBC_DFF1);
        let ct = des_encrypt(0x0123_4567_89AB_CDEF, &ks, 0);
        assert_eq!(ct, 0x85E8_1354_0F0A_B405);
    }

    /// Only the first 8 key bytes participate.
    #[test]
    fn key_truncates_at_eight_bytes() {
        let want = crypt(b"12345678", b"XY");
        assert_eq!(crypt(b"123456789", b"XY"), want);
        assert_eq!(crypt(b"12345678zzzz", b"XY"), want);
        assert_eq!(want.as_ref().map(|h| h.as_slice()), Some(&b"XYUxf1kbUCJG."[..]));
    }

    /// Only the first 2 salt bytes participate: hashing against a player
    /// name ("Fizban") equals hashing against its first two letters.
    #[test]
    fn salt_truncates_at_two_bytes() {
        assert_eq!(crypt(b"swordfish", b"Fizban"), crypt(b"swordfish", b"Fi"));
    }

    /// The MUD's verify pattern: CRYPT(typed, GET_PASSWD(ch)) passes the
    /// whole stored hash as the salt; its first two bytes are the salt, so
    /// the result must round-trip to the stored hash.
    #[test]
    fn verify_roundtrip_against_stored_hash() {
        let stored = crypt(b"swordfish", b"Fizban").unwrap();
        assert_eq!(crypt(b"swordfish", &stored), Some(stored));
    }

    /// High key-byte bits are stripped (low 7 bits used): 0xE1 acts as
    /// 'a', and 0x80 contributes zero bits without terminating the key.
    #[test]
    fn key_bytes_use_low_seven_bits() {
        assert_eq!(crypt(b"p\xE1ss", b"AB"), crypt(b"pass", b"AB"));
        assert_eq!(crypt(b"\x80", b"ab"), crypt(b"", b"ab"));
    }

    /// A NUL byte ends the key early — the key is read as a NUL-terminated
    /// string.
    #[test]
    fn nul_terminates_key() {
        assert_eq!(crypt(b"pass\0word", b"AB"), crypt(b"pass", b"AB"));
    }

    /// Salts shorter than two bytes cannot hash.
    #[test]
    fn short_salt_is_none() {
        assert_eq!(crypt(b"password", b""), None);
        assert_eq!(crypt(b"password", b"a"), None);
    }
}
