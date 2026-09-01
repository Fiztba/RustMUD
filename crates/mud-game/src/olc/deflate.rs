//! DEFLATE (RFC 1951) with fixed Huffman codes, for `export`'s archives.
//!
//! Written rather than depended on: the workspace carries two third-party
//! crates (mio, chrono) and neither is load-bearing here, so a compressor
//! for one command is not worth a dependency tree. Fixed-Huffman coding
//! costs a few percent against gzip's dynamic tables and needs none of the
//! code that building and emitting those tables would.
//!
//! If a block comes out larger than the bytes that went in — already
//! compressed data, mostly — the stored form is emitted instead, so the
//! output never exceeds the input by more than the framing.

const WINDOW: usize = 32768;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
/// Longest hash chain walked per position. Higher finds longer matches at
/// a cost; 128 is around zlib's default level.
const MAX_CHAIN: usize = 128;
const NONE: u32 = u32::MAX;

/// RFC 1951 §3.2.5 length codes 257-285 and distance codes 0-29.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
    13, 13,
];

/// Bits go out least-significant first, but a Huffman code's own bits go
/// out most-significant first (RFC 1951 §3.1.1).
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        BitWriter { out: Vec::with_capacity(capacity), acc: 0, nbits: 0 }
    }

    fn bits(&mut self, value: u32, count: u32) {
        self.acc |= value << self.nbits;
        self.nbits += count;
        while self.nbits >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }

    fn code(&mut self, code: u32, len: u32) {
        for i in (0..len).rev() {
            self.bits((code >> i) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

/// The fixed literal/length alphabet (RFC 1951 §3.2.6).
fn literal_code(sym: u16) -> (u32, u32) {
    match sym {
        0..=143 => (0x30 + u32::from(sym), 8),
        144..=255 => (0x190 + u32::from(sym) - 144, 9),
        256..=279 => (u32::from(sym) - 256, 7),
        _ => (0xC0 + u32::from(sym) - 280, 8),
    }
}

fn length_code(len: usize) -> usize {
    // Highest base not exceeding len.
    LENGTH_BASE.iter().rposition(|&b| usize::from(b) <= len).unwrap()
}

fn distance_code(dist: usize) -> usize {
    DIST_BASE.iter().rposition(|&b| usize::from(b) <= dist).unwrap()
}

fn hash(data: &[u8], i: usize) -> usize {
    let h = (u32::from(data[i]) << 10) ^ (u32::from(data[i + 1]) << 5) ^ u32::from(data[i + 2]);
    (h.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize % HASH_SIZE
}

/// One stored block per 65535 bytes, for data that will not compress.
fn stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    let mut rest = data;
    loop {
        let take = rest.len().min(0xFFFF);
        let (chunk, remainder) = rest.split_at(take);
        let last = remainder.is_empty();
        out.push(u8::from(last));
        out.extend_from_slice(&(take as u16).to_le_bytes());
        out.extend_from_slice(&(!(take as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
        if last {
            break;
        }
        rest = remainder;
    }
    out
}

/// Compress to a raw DEFLATE stream.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    if data.len() < MIN_MATCH {
        return stored(data);
    }

    let mut w = BitWriter::new(data.len() / 2 + 64);
    w.bits(1, 1); // BFINAL
    w.bits(1, 2); // BTYPE = 01, fixed Huffman

    let mut head = vec![NONE; HASH_SIZE];
    let mut prev = vec![NONE; data.len()];
    let mut i = 0usize;

    while i < data.len() {
        let mut best_len = 0usize;
        let mut best_dist = 0usize;

        if i + MIN_MATCH <= data.len() {
            let h = hash(data, i);
            let mut candidate = head[h];
            let limit = i.saturating_sub(WINDOW);
            let mut chain = 0;
            while candidate != NONE && (candidate as usize) >= limit && chain < MAX_CHAIN {
                let c = candidate as usize;
                let max = MAX_MATCH.min(data.len() - i);
                // Cheap rejection before the full compare.
                if max > best_len && data[c + best_len] == data[i + best_len] {
                    let mut len = 0;
                    while len < max && data[c + len] == data[i + len] {
                        len += 1;
                    }
                    if len > best_len {
                        best_len = len;
                        best_dist = i - c;
                        if len == max {
                            break;
                        }
                    }
                }
                candidate = prev[c];
                chain += 1;
            }
        }

        if best_len >= MIN_MATCH {
            let lc = length_code(best_len);
            let (code, bits) = literal_code(257 + lc as u16);
            w.code(code, bits);
            let extra = LENGTH_EXTRA[lc];
            if extra > 0 {
                w.bits((best_len - usize::from(LENGTH_BASE[lc])) as u32, u32::from(extra));
            }
            let dc = distance_code(best_dist);
            w.code(dc as u32, 5);
            let extra = DIST_EXTRA[dc];
            if extra > 0 {
                w.bits((best_dist - usize::from(DIST_BASE[dc])) as u32, u32::from(extra));
            }
            // Every position the match covers still has to enter the hash
            // chains, or later matches will not find them.
            for (k, slot) in prev.iter_mut().enumerate().take(i + best_len).skip(i) {
                if k + MIN_MATCH <= data.len() {
                    let h = hash(data, k);
                    *slot = head[h];
                    head[h] = k as u32;
                }
            }
            i += best_len;
        } else {
            let (code, bits) = literal_code(u16::from(data[i]));
            w.code(code, bits);
            if i + MIN_MATCH <= data.len() {
                let h = hash(data, i);
                prev[i] = head[h];
                head[h] = i as u32;
            }
            i += 1;
        }
    }

    let (code, bits) = literal_code(256); // end of block
    w.code(code, bits);
    let compressed = w.finish();

    // Never do worse than not compressing at all.
    if compressed.len() >= data.len() + 5 {
        stored(data)
    } else {
        compressed
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal INFLATE for stored and fixed-Huffman blocks. Test-only,
    /// and deliberately written from the RFC rather than from the encoder
    /// above, so a shared misreading cannot make both agree.
    struct BitReader<'a> {
        data: &'a [u8],
        pos: usize,
        bit: u32,
    }

    impl<'a> BitReader<'a> {
        fn bits(&mut self, count: u32) -> u32 {
            let mut v = 0;
            for i in 0..count {
                let byte = self.data[self.pos];
                let b = (byte >> self.bit) & 1;
                v |= u32::from(b) << i;
                self.bit += 1;
                if self.bit == 8 {
                    self.bit = 0;
                    self.pos += 1;
                }
            }
            v
        }

        /// Huffman codes arrive most-significant bit first.
        fn code_bits(&mut self, count: u32) -> u32 {
            let mut v = 0;
            for _ in 0..count {
                v = (v << 1) | self.bits(1);
            }
            v
        }

        fn align(&mut self) {
            if self.bit != 0 {
                self.bit = 0;
                self.pos += 1;
            }
        }
    }

    pub(crate) fn inflate(data: &[u8]) -> Vec<u8> {
        let mut r = BitReader { data, pos: 0, bit: 0 };
        let mut out: Vec<u8> = Vec::new();
        loop {
            let last = r.bits(1);
            let btype = r.bits(2);
            match btype {
                0 => {
                    r.align();
                    let len = u16::from_le_bytes([r.data[r.pos], r.data[r.pos + 1]]) as usize;
                    let nlen = u16::from_le_bytes([r.data[r.pos + 2], r.data[r.pos + 3]]);
                    assert_eq!(!(len as u16), nlen, "stored block LEN/NLEN");
                    r.pos += 4;
                    out.extend_from_slice(&r.data[r.pos..r.pos + len]);
                    r.pos += len;
                }
                1 => loop {
                    // Decode one fixed-Huffman symbol by prefix length.
                    let first7 = r.code_bits(7);
                    let sym = if first7 <= 0b0010111 {
                        256 + first7 as u16
                    } else {
                        let v8 = (first7 << 1) | r.bits(1);
                        if (0x30..=0xBF).contains(&v8) {
                            (v8 - 0x30) as u16
                        } else if v8 >= 0xC8 {
                            let v9 = (v8 << 1) | r.bits(1);
                            (v9 - 0x190 + 144) as u16
                        } else {
                            (v8 - 0xC0 + 280) as u16
                        }
                    };
                    if sym == 256 {
                        break;
                    }
                    if sym < 256 {
                        out.push(sym as u8);
                        continue;
                    }
                    let lc = (sym - 257) as usize;
                    let len = usize::from(LENGTH_BASE[lc])
                        + r.bits(u32::from(LENGTH_EXTRA[lc])) as usize;
                    let dc = r.code_bits(5) as usize;
                    let dist =
                        usize::from(DIST_BASE[dc]) + r.bits(u32::from(DIST_EXTRA[dc])) as usize;
                    let start = out.len() - dist;
                    for k in 0..len {
                        let b = out[start + k];
                        out.push(b);
                    }
                },
                _ => panic!("unexpected block type {btype}"),
            }
            if last == 1 {
                break;
            }
        }
        out
    }

    fn round_trip(data: &[u8]) {
        let got = inflate(&deflate(data));
        assert_eq!(got.len(), data.len(), "length after round trip");
        assert!(got == data, "content differs after round trip");
    }

    #[test]
    fn round_trips_the_awkward_shapes() {
        round_trip(b"");
        round_trip(b"a");
        round_trip(b"ab");
        round_trip(b"abc");
        round_trip(&[0u8; 300]); // one long run
        round_trip(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        round_trip(b"the quick brown fox jumps over the lazy dog");
        // Every byte value, so all four fixed-code ranges are exercised.
        let all: Vec<u8> = (0..=255u8).collect();
        round_trip(&all);
        round_trip(&all.repeat(20));
    }

    #[test]
    fn round_trips_real_world_files_and_actually_compresses() {
        let wld = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../lib/world/wld/30.wld"),
        )
        .expect("30.wld");
        round_trip(&wld);
        let ratio = deflate(&wld).len() as f64 / wld.len() as f64;
        assert!(ratio < 0.45, "expected real compression, got ratio {ratio:.3}");
    }

    #[test]
    fn a_match_at_the_window_edge_still_round_trips() {
        // A repeat separated by just under the 32K window, so the match is
        // found at the furthest distance the encoder may emit.
        let mut data = b"tbaMUD zone export".to_vec();
        data.extend(std::iter::repeat_n(b'.', WINDOW - data.len() - 1));
        data.extend_from_slice(b"tbaMUD zone export");
        round_trip(&data);
    }

    #[test]
    fn incompressible_input_falls_back_to_stored() {
        // A counter-based pseudo-random stream: no repeats to find.
        let mut x = 0x12345678u32;
        let noise: Vec<u8> = (0..40_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (x >> 16) as u8
            })
            .collect();
        let out = deflate(&noise);
        assert!(out.len() <= noise.len() + 16, "must not inflate the input");
        round_trip(&noise);
    }
}
