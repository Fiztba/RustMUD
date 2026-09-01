//! Reverse DNS for an incoming connection.
//!
//! The hostname is what `isbanned` is tested against, so it has to be known
//! before a new connection is greeted. Resolving inline on the game loop's
//! thread would stop the world for the resolver's whole budget — measured at
//! 10s on stock glibc settings, which took an established client's command
//! round-trip from 0.050s to 13.050s.
//!
//! We keep the lookup and its result, and move only the *waiting* off the
//! accept path — see `mud-server`. This module is just the primitive.
//!
//! `std` has no reverse resolver, so this is `getnameinfo(3)`: POSIX on Unix
//! and the same call in ws2_32 on Windows. The only difference between the
//! platforms is the shape of `sockaddr_in`, which BSD-derived systems (and
//! macOS) start with a one-byte length instead of a two-byte family.

use std::net::IpAddr;

/// `getnameinfo` flags: fail rather than return a numeric string, so a
/// missing PTR record is distinguishable from a successful lookup.
const NI_NAMEREQD: i32 = if cfg!(windows) { 0x04 } else { 8 };
const NI_MAXHOST: usize = 1025;

#[cfg(unix)]
mod imp {
    // SAFETY (declaration): the POSIX signature of getnameinfo(3). The
    // sockaddr pointer is read-only for `salen` bytes and the host buffer is
    // written for at most `hostlen` bytes, both of which the caller sizes.
    #[allow(unsafe_code)]
    unsafe extern "C" {
        pub fn getnameinfo(
            sa: *const u8,
            salen: u32,
            host: *mut u8,
            hostlen: u32,
            serv: *mut u8,
            servlen: u32,
            flags: i32,
        ) -> i32;
    }

    pub const AF_INET: u16 = 2;
    #[cfg(target_os = "linux")]
    pub const AF_INET6: u16 = 10;
    #[cfg(not(target_os = "linux"))]
    pub const AF_INET6: u16 = 30;
}

#[cfg(windows)]
mod imp {
    // SAFETY (declaration): ws2_32's getnameinfo has the same signature as
    // the POSIX one. It needs Winsock started, which `crate::init_sockets`
    // guarantees at the call site rather than assuming a socket exists.
    #[allow(unsafe_code)]
    #[link(name = "ws2_32")]
    unsafe extern "system" {
        pub fn getnameinfo(
            sa: *const u8,
            salen: i32,
            host: *mut u8,
            hostlen: u32,
            serv: *mut u8,
            servlen: u32,
            flags: i32,
        ) -> i32;
    }

    pub const AF_INET: u16 = 2;
    pub const AF_INET6: u16 = 23;
}

/// Lay out a `sockaddr_in`/`sockaddr_in6` for `ip`, port zero.
///
/// Returns the bytes and the length the platform expects. BSD-derived
/// systems put a one-byte `sin_len` where everyone else has the low half of
/// a two-byte `sin_family`; on a little-endian host the family byte lands in
/// the same place either way, so only the length byte differs.
fn sockaddr_for(ip: IpAddr) -> (Vec<u8>, usize) {
    let bsd = cfg!(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ));
    match ip {
        IpAddr::V4(v4) => {
            let mut sa = vec![0u8; 16];
            if bsd {
                sa[0] = 16;
                sa[1] = imp::AF_INET as u8;
            } else {
                sa[0..2].copy_from_slice(&imp::AF_INET.to_ne_bytes());
            }
            // sin_port stays zero; sin_addr is network order by definition.
            sa[4..8].copy_from_slice(&v4.octets());
            (sa, 16)
        }
        IpAddr::V6(v6) => {
            let mut sa = vec![0u8; 28];
            if bsd {
                sa[0] = 28;
                sa[1] = imp::AF_INET6 as u8;
            } else {
                sa[0..2].copy_from_slice(&imp::AF_INET6.to_ne_bytes());
            }
            // sin6_port and sin6_flowinfo stay zero; sin6_addr at offset 8.
            sa[8..24].copy_from_slice(&v6.octets());
            (sa, 28)
        }
    }
}

/// The hostname for `ip`, or `None` when it has no PTR record.
///
/// This blocks for as long as the resolver takes. Call it off the game
/// loop's thread.
pub fn reverse_lookup(ip: IpAddr) -> Option<String> {
    // Windows routes this through ws2_32, which wants its library started.
    crate::init_sockets().ok()?;
    let (sa, salen) = sockaddr_for(ip);
    let mut host = vec![0u8; NI_MAXHOST];

    // SAFETY: `sa` is a correctly laid out sockaddr of exactly `salen`
    // bytes for the family in its first field, and `host` has NI_MAXHOST
    // bytes for getnameinfo to write into. The service arguments are null
    // with zero length, which the call accepts as "no service wanted".
    #[allow(unsafe_code)]
    let rc = unsafe {
        imp::getnameinfo(
            sa.as_ptr(),
            salen as _,
            host.as_mut_ptr(),
            host.len() as u32,
            std::ptr::null_mut(),
            0,
            NI_NAMEREQD,
        )
    };
    if rc != 0 {
        return None;
    }
    let end = host.iter().position(|&b| b == 0).unwrap_or(host.len());
    String::from_utf8(host[..end].to_vec()).ok().filter(|s| !s.is_empty())
}
