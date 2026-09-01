//! Archive containers for `export`, written in-process.
//!
//! Shelling out to `rm`, `tar -cf` and `gzip` would tie `export` to
//! platforms where those exist ("tar and gzip are usually not available"
//! on Windows) and would hand a zone name to `system` unquoted. Emitting
//! the bytes here removes both problems: the command works identically on
//! every platform, and there is no shell for a builder-supplied zone name
//! to escape into.
//!
//! Members are compressed with [`super::deflate`], so an exported zone is
//! the size a builder expects to mail rather than four times it, and the
//! archives carry the real time of the export — a file dated 1970 reads as
//! a broken tool, and reproducible output is worth nothing to the person
//! receiving a zone.

/// CRC-32/ISO-HDLC, the one both gzip and zip use.
const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// One archive member.
pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
}

/// Write an octal field: zero-padded to width-1 digits, then NUL.
fn octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    let bytes = text.as_bytes();
    // Values here are file sizes and permissions; they cannot overflow the
    // field, but truncate from the left rather than panic if one ever does.
    let start = bytes.len().saturating_sub(digits);
    field[..digits].copy_from_slice(&bytes[start..]);
    field[digits] = 0;
}

/// USTAR archive. Each member gets a 512-byte header and its data padded
/// to a 512-byte boundary; the archive ends with two zero blocks.
/// `mtime` is a Unix timestamp, stamped on every member.
pub fn tar(members: &[Member], mtime: u64) -> Vec<u8> {
    let mut out = Vec::new();
    for m in members {
        let mut h = [0u8; 512];
        let name = m.name.as_bytes();
        let n = name.len().min(100);
        h[..n].copy_from_slice(&name[..n]);
        octal(&mut h[100..108], 0o644); // mode
        octal(&mut h[108..116], 0); // uid
        octal(&mut h[116..124], 0); // gid
        octal(&mut h[124..136], m.data.len() as u64);
        octal(&mut h[136..148], mtime);
        h[156] = b'0'; // typeflag: regular file
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");

        // Checksum is computed with the checksum field read as spaces.
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        let text = format!("{sum:06o}");
        h[148..154].copy_from_slice(text.as_bytes());
        h[154] = 0;
        h[155] = b' ';

        out.extend_from_slice(&h);
        out.extend_from_slice(&m.data);
        let pad = (512 - m.data.len() % 512) % 512;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out.extend(std::iter::repeat_n(0u8, 1024)); // two zero blocks
    out
}

/// gzip container around a deflate stream. `mtime` is a Unix timestamp;
/// gzip stores it in the header, and `gzip -l` reports it.
pub fn gzip(data: &[u8], mtime: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 64);
    // magic, CM=deflate, no flags, mtime, no extra flags, OS unknown.
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00]);
    out.extend_from_slice(&mtime.to_le_bytes());
    out.extend_from_slice(&[0x00, 0xff]);

    out.extend_from_slice(&super::deflate::deflate(data));

    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// ZIP archive — it opens with a double-click on any Windows back to XP,
/// where.tar.gz needs a tool. Members are deflated unless that would make
/// them bigger, in which case they go in stored. `dos_date`/`dos_time` are
/// the MS-DOS packed forms zip uses for timestamps.
pub fn zip(members: &[Member], dos_date: u16, dos_time: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count = 0u16;

    for m in members {
        let name = m.name.as_bytes();
        let crc = crc32(&m.data);
        let offset = out.len() as u32;
        let deflated = super::deflate::deflate(&m.data);
        // Method 8 carries a raw deflate stream; method 0 the bytes as-is.
        let (method, body): (u16, &[u8]) = if deflated.len() < m.data.len() {
            (8, &deflated)
        } else {
            (0, &m.data)
        };

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local header
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&dos_time.to_le_bytes());
        out.extend_from_slice(&dos_date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(m.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name);
        out.extend_from_slice(body);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // dir entry
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&dos_time.to_le_bytes());
        central.extend_from_slice(&dos_date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(m.data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
        count += 1;
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // end of directory
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with directory
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// Pack a local date and time into the MS-DOS fields zip stores.
pub fn dos_stamp(year: i32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> (u16, u16) {
    let year = year.clamp(1980, 2107);
    let date = (((year - 1980) as u16) << 9) | ((month as u16) << 5) | day as u16;
    let time = ((hour as u16) << 11) | ((min as u16) << 5) | (sec / 2) as u16;
    (date, time)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, data: &[u8]) -> Member {
        Member { name: name.to_string(), data: data.to_vec() }
    }

    #[test]
    fn crc32_matches_the_reference_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn tar_header_is_a_valid_ustar_record() {
        let out = tar(&[member("qq.wld", b"#3001\n$~\n")], 1_760_000_000);
        // Header, one data block, two zero blocks.
        assert_eq!(out.len(), 512 * 4);
        assert_eq!(&out[..6], b"qq.wld");
        assert_eq!(&out[257..263], b"ustar\0");
        assert_eq!(&out[263..265], b"00");
        assert_eq!(&out[100..108], b"0000644\0"); // mode
        assert_eq!(&out[124..136], b"00000000011\0"); // size: 9 octal
        assert_eq!(&out[136..148], b"15071674000\0"); // mtime, octal
        assert_eq!(out[156], b'0'); // regular file
        assert_eq!(&out[512..521], b"#3001\n$~\n");
        assert!(out[521..1024].iter().all(|&b| b == 0), "data block padded");
        assert!(out[1024..].iter().all(|&b| b == 0), "two zero blocks");

        // The checksum field must equal the sum of the header read with
        // that field blanked — this is what tar validates on open.
        let mut h = [0u8; 512];
        h.copy_from_slice(&out[..512]);
        let stored = std::str::from_utf8(&h[148..154]).unwrap().to_string();
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        assert_eq!(u32::from_str_radix(&stored, 8).unwrap(), sum);
    }

    #[test]
    fn gzip_header_and_trailer_describe_the_payload() {
        let body = b"the quick brown fox jumps over the lazy dog".repeat(40);
        let out = gzip(&body, 1_760_000_000);
        assert_eq!(&out[..4], &[0x1f, 0x8b, 0x08, 0x00], "magic, deflate, no flags");
        assert_eq!(&out[4..8], &1_760_000_000u32.to_le_bytes(), "mtime in the header");
        // Trailer: CRC of the ORIGINAL bytes, then their length.
        let tail = &out[out.len() - 8..];
        assert_eq!(&tail[..4], &crc32(&body).to_le_bytes());
        assert_eq!(&tail[4..], &(body.len() as u32).to_le_bytes());
        // And it is actually compressed — the deflate round-trip itself is
        // covered in olc::deflate.
        assert!(out.len() < body.len() / 2, "{} vs {}", out.len(), body.len());
    }

    /// The whole pipeline: tar eight members, gzip it, then take it back
    /// apart and check every header. Uses the independently written
    /// inflater from olc::deflate's tests.
    #[test]
    fn a_gzipped_tar_takes_apart_into_the_members_that_went_in() {
        let files: Vec<Member> = ["info", "wld", "zon", "mob", "obj", "shp", "qst", "trg"]
            .iter()
            .enumerate()
            .map(|(i, ext)| member(&format!("qq.{ext}"), &vec![b'a' + i as u8; 700 * (i + 1)]))
            .collect();
        let tarred = tar(&files, 1_760_000_000);
        let gz = gzip(&tarred, 1_760_000_000);
        assert!(gz.len() < tarred.len() / 4, "the tar should compress hard");

        let payload = &gz[10..gz.len() - 8];
        let back = crate::olc::deflate::tests::inflate(payload);
        assert_eq!(back, tarred, "gzip payload inflates to the tar");
        assert_eq!(&gz[gz.len() - 8..gz.len() - 4], &crc32(&tarred).to_le_bytes());

        let mut seen = Vec::new();
        let mut off = 0;
        while off + 512 <= back.len() && back[off] != 0 {
            let name =
                String::from_utf8_lossy(&back[off..off + 100]).trim_end_matches('\0').to_string();
            let size = usize::from_str_radix(
                String::from_utf8_lossy(&back[off + 124..off + 135]).trim_end_matches('\0'),
                8,
            )
            .unwrap();
            let body = &back[off + 512..off + 512 + size];
            let original = files.iter().find(|m| m.name == name).expect("member name");
            assert_eq!(body, &original.data[..], "{name} body survived the round trip");
            seen.push(name);
            off += 512 + size.div_ceil(512) * 512;
        }
        assert_eq!(seen.len(), 8, "every member is listed");
    }

    #[test]
    fn dos_stamps_pack_the_way_zip_reads_them() {
        // 2026-08-25 11:15:30 -> the fields PKZIP splits back out.
        let (date, time) = dos_stamp(2026, 8, 25, 11, 15, 30);
        assert_eq!(date >> 9, 46, "year - 1980");
        assert_eq!((date >> 5) & 0xF, 8);
        assert_eq!(date & 0x1F, 25);
        assert_eq!(time >> 11, 11);
        assert_eq!((time >> 5) & 0x3F, 15);
        assert_eq!((time & 0x1F) * 2, 30);
        // The format cannot represent anything before 1980.
        assert_eq!(dos_stamp(1970, 1, 1, 0, 0, 0).0 >> 9, 0);
    }

    #[test]
    fn zip_directory_points_at_each_local_header() {
        let out = zip(&[member("qq.info", b"info"), member("qq.wld", b"rooms")], 0x5519, 0x5A0F);
        assert_eq!(&out[..4], &0x0403_4b50u32.to_le_bytes());
        // End-of-directory record: 22 bytes, no comment.
        let eocd = &out[out.len() - 22..];
        assert_eq!(&eocd[..4], &0x0605_4b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([eocd[10], eocd[11]]), 2, "two members");
        let dir_size = u32::from_le_bytes([eocd[12], eocd[13], eocd[14], eocd[15]]) as usize;
        let dir_off = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]) as usize;
        assert_eq!(dir_off + dir_size, out.len() - 22);
        assert_eq!(&out[dir_off..dir_off + 4], &0x0201_4b50u32.to_le_bytes());
        // First member's local header sits at the offset the directory gives.
        let off = u32::from_le_bytes([
            out[dir_off + 42],
            out[dir_off + 43],
            out[dir_off + 44],
            out[dir_off + 45],
        ]) as usize;
        assert_eq!(&out[off..off + 4], &0x0403_4b50u32.to_le_bytes());
        assert_eq!(&out[off + 30..off + 37], b"qq.info");
    }
}
