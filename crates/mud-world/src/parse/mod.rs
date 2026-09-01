//! Format parsers, one submodule per world-file format — including the
//! parse tolerances and error-recovery behaviours the shipped world files
//! rely on.

pub mod mob;
pub mod obj;
pub mod qst;
pub mod shp;
pub mod trg;
pub mod wld;
pub mod zon;

/// Whitespace-separated integers, up to `want` of them. Returns fewer than
/// `want` when the line runs dry or a token is not numeric; leading junk
/// stops the scan where it stands.
pub fn scan_ints(line: &[u8], want: usize) -> Vec<i64> {
    let mut out = Vec::new();
    let mut it = line.split(|&b| b == b' ' || b == b'\t').filter(|t| !t.is_empty());
    while out.len() < want {
        match it.next() {
            Some(tok) => {
                let ok = tok[0].is_ascii_digit()
                    || (tok.len() > 1 && (tok[0] == b'-' || tok[0] == b'+'));
                if !ok {
                    break;
                }
                out.push(crate::lex::atol(tok));
            }
            None => break,
        }
    }
    out
}

/// Whitespace-separated raw tokens (for flag fields that may be letters).
pub fn tokens(line: &[u8]) -> Vec<&[u8]> {
    line.split(|&b| b == b' ' || b == b'\t').filter(|t| !t.is_empty()).collect()
}
