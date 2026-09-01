//! Player command implementations (the act.*.c family).

pub mod comm;
pub mod informative;
pub mod item;
pub mod movement;
pub mod offensive;
pub mod other;
pub mod social;
pub mod wizard;
pub mod wizset;
pub mod wizshow;
pub mod wizstat;
pub mod write;

pub type BStr = Vec<u8>;

/// Byte-string right-pad to `width` (`%-Ns`). Longer strings pass through.
pub fn pad_right(b: &[u8], width: usize) -> BStr {
    let mut out = b.to_vec();
    while out.len() < width {
        out.push(b' ');
    }
    out
}

/// Truncating %-N.Ns.
pub fn pad_right_trunc(b: &[u8], width: usize) -> BStr {
    let mut out = b[..b.len().min(width)].to_vec();
    while out.len() < width {
        out.push(b' ');
    }
    out
}

/// Truncating %N.Ns: right-justified in `width`, cut to `trunc` first.
pub fn pad_left_trunc(b: &[u8], width: usize, trunc: usize) -> BStr {
    pad_left(&b[..b.len().min(trunc)], width)
}

/// %Nd-style left pad for already-rendered text.
pub fn pad_left(b: &[u8], width: usize) -> BStr {
    let mut out = Vec::new();
    while out.len() + b.len() < width {
        out.push(b' ');
    }
    out.extend_from_slice(b);
    out
}
