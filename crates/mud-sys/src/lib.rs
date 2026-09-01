//! Socket handoff across a process replacement — the copyover primitive.
//!
//! This is the only crate in the workspace that contains `unsafe`. Everything
//! here exists because the operating system has no safe interface for giving
//! a live socket to a successor process:
//!
//! * **Unix** inherits file descriptors across `exec`, so the socket survives
//! as long as `FD_CLOEXEC` is cleared — `fcntl(2)`, an `extern "C"` call.
//! Re-adopting the inherited descriptor on the other side is
//! `FromRawFd::from_raw_fd`, unsafe by definition.
//! * **Windows** has no such inheritance. Winsock instead duplicates a socket
//! *into a named process* with `WSADuplicateSocketW`, handing back an opaque
//! `WSAPROTOCOL_INFOW` blob that the successor turns back into a socket with
//! `WSASocketW(..., FROM_PROTOCOL_INFO)`. Both are raw FFI.
//!
//! Every function here is safe to call; the unsafety is confined to the
//! blocks below and each one documents the invariant it relies on.

#[cfg(unix)]
mod imp {
    use std::os::unix::io::{AsRawFd, FromRawFd};

    // SAFETY (declaration): the POSIX signature of fcntl(2). The variadic
    // form is used with an int third argument only, which is ABI-correct.
    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }

    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;

    /// Clear FD_CLOEXEC so the descriptor survives `exec`, and report it.
    pub fn keep_open<T: AsRawFd>(s: &T) -> std::io::Result<i64> {
        let fd = s.as_raw_fd();
        // SAFETY: `fd` is owned by `s` and valid for the call; both fcntl
        // commands used here take/return an int and never touch memory.
        #[allow(unsafe_code)]
        unsafe {
            let flags = fcntl(fd, F_GETFD);
            if flags < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if fcntl(fd, F_SETFD, flags & !FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(fd as i64)
    }

    /// Unix hands sockets over by inheritance, so there is no blob to carry.
    pub fn dup_for_child<T: AsRawFd>(s: &T, _pid: u32) -> std::io::Result<Vec<u8>> {
        keep_open(s).map(|_| Vec::new())
    }

    /// Sockets need no library-level startup here.
    pub fn init() -> std::io::Result<()> {
        Ok(())
    }

    fn check(fd: i64) -> std::io::Result<i32> {
        if fd < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no descriptor"));
        }
        Ok(fd as i32)
    }

    pub fn adopt_stream(fd: i64, _blob: &[u8]) -> std::io::Result<std::net::TcpStream> {
        let fd = check(fd)?;
        // SAFETY: the descriptor was inherited across exec and named in
        // copyover.dat by the process that owned it; ownership transfers here
        // exactly once, and nothing else in this process holds it.
        #[allow(unsafe_code)]
        Ok(unsafe { std::net::TcpStream::from_raw_fd(fd) })
    }

    pub fn adopt_listener(fd: i64, _blob: &[u8]) -> std::io::Result<std::net::TcpListener> {
        let fd = check(fd)?;
        // SAFETY: as above — the inherited listening descriptor, adopted once.
        #[allow(unsafe_code)]
        Ok(unsafe { std::net::TcpListener::from_raw_fd(fd) })
    }

    /// Replace this process image, keeping the cleared-CLOEXEC descriptors.
    pub fn exec(cmd: &mut std::process::Command) -> std::io::Error {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::{AsRawSocket, FromRawSocket};

    /// `sizeof(WSAPROTOCOL_INFOW)` is 632 bytes; the buffer is oversized so
    /// the call can never overflow it, and the trailing zeroes round-trip
    /// harmlessly through `WSASocketW`.
    pub const PROTOCOL_INFO_LEN: usize = 1024;

    type Socket = usize;
    const INVALID_SOCKET: Socket = usize::MAX;
    const WSA_FLAG_OVERLAPPED: u32 = 0x01;

    /// `WSASocketW` reads the address family, socket type and protocol out of
    /// the blob when each of the three is passed as this sentinel.
    const FROM_PROTOCOL_INFO: i32 = -1;

    /// `MAKEWORD(2, 2)`, the Winsock version every supported Windows offers.
    const WINSOCK_VERSION: u16 = 0x0202;

    // SAFETY (declaration): the documented ws2_32 signatures. lpProtocolInfo
    // and lpWSAData are opaque byte buffers here, each sized by the caller
    // above the struct the call writes into it.
    #[allow(unsafe_code)]
    #[link(name = "ws2_32")]
    unsafe extern "system" {
        fn WSAStartup(w_version_requested: u16, lp_wsa_data: *mut u8) -> i32;
        fn WSADuplicateSocketW(s: Socket, dw_process_id: u32, lp_protocol_info: *mut u8) -> i32;
        fn WSASocketW(
            af: i32,
            type_: i32,
            protocol: i32,
            lp_protocol_info: *mut u8,
            g: u32,
            dw_flags: u32,
        ) -> Socket;
        fn WSAGetLastError() -> i32;
    }

    /// Start Winsock, so a raw ws2_32 call below has something to talk to.
    ///
    /// `std` starts Winsock lazily, the first time it creates a socket of its
    /// own. A successor recovering a copyover never creates one: the mother
    /// connection and every player socket arrive as protocol-info blobs, so
    /// `WSASocketW` is the first ws2_32 call the process makes — and without
    /// this it fails with WSANOTINITIALISED, taking the whole handoff and
    /// every player on it. Winsock reference-counts its startups, so this
    /// costs nothing when `std` gets there first, and there is no matching
    /// cleanup to do: the library stays loaded for the life of the process
    /// either way.
    pub fn init() -> std::io::Result<()> {
        use std::sync::OnceLock;
        static RC: OnceLock<i32> = OnceLock::new();
        let rc = *RC.get_or_init(|| {
            // WSADATA is 408 bytes on x64; the buffer is oversized so the
            // call can never overflow it.
            let mut data = [0u8; 1024];
            // SAFETY: `data` is a writable allocation larger than WSADATA,
            // and the version word is one ws2_32 has always accepted.
            #[allow(unsafe_code)]
            unsafe {
                WSAStartup(WINSOCK_VERSION, data.as_mut_ptr())
            }
        });
        if rc != 0 {
            // WSAStartup reports through its return value; WSAGetLastError
            // is not meaningful until it has succeeded at least once.
            return Err(std::io::Error::from_raw_os_error(rc));
        }
        Ok(())
    }

    /// Windows cannot pre-authorise a handoff: the target pid is required, so
    /// there is nothing to do until the successor exists.
    pub fn keep_open<T: AsRawSocket>(_s: &T) -> std::io::Result<i64> {
        Ok(-1)
    }

    /// Duplicate `s` into process `pid`, returning the WSAPROTOCOL_INFOW blob
    /// that process needs to rebuild it.
    pub fn dup_for_child<T: AsRawSocket>(s: &T, pid: u32) -> std::io::Result<Vec<u8>> {
        init()?;
        let mut buf = vec![0u8; PROTOCOL_INFO_LEN];
        // SAFETY: `s` owns a valid socket for the duration of the call, and
        // `buf` is a writable allocation larger than WSAPROTOCOL_INFOW.
        #[allow(unsafe_code)]
        let rc = unsafe { WSADuplicateSocketW(s.as_raw_socket() as Socket, pid, buf.as_mut_ptr()) };
        if rc != 0 {
            return Err(last_wsa_error());
        }
        Ok(buf)
    }

    fn last_wsa_error() -> std::io::Error {
        // SAFETY: WSAGetLastError takes no arguments and only reads TLS.
        #[allow(unsafe_code)]
        let err = unsafe { WSAGetLastError() };
        std::io::Error::from_raw_os_error(err)
    }

    fn from_blob(blob: &[u8]) -> std::io::Result<Socket> {
        init()?;
        if blob.len() < PROTOCOL_INFO_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "short WSAPROTOCOL_INFOW blob",
            ));
        }
        let mut buf = blob.to_vec();
        // SAFETY: `buf` holds the blob WSADuplicateSocketW produced for this
        // process and stays valid for the call; the three FROM_PROTOCOL_INFO
        // arguments tell WSASocketW to take the address family, type and
        // protocol from that blob rather than from the argument list.
        #[allow(unsafe_code)]
        let s = unsafe {
            WSASocketW(
                FROM_PROTOCOL_INFO,
                FROM_PROTOCOL_INFO,
                FROM_PROTOCOL_INFO,
                buf.as_mut_ptr(),
                0,
                WSA_FLAG_OVERLAPPED,
            )
        };
        if s == INVALID_SOCKET {
            return Err(last_wsa_error());
        }
        Ok(s)
    }

    pub fn adopt_stream(_fd: i64, blob: &[u8]) -> std::io::Result<std::net::TcpStream> {
        let s = from_blob(blob)?;
        // SAFETY: `s` is a fresh socket this process now owns outright.
        #[allow(unsafe_code)]
        Ok(unsafe { std::net::TcpStream::from_raw_socket(s as u64) })
    }

    pub fn adopt_listener(_fd: i64, blob: &[u8]) -> std::io::Result<std::net::TcpListener> {
        let s = from_blob(blob)?;
        // SAFETY: as above.
        #[allow(unsafe_code)]
        Ok(unsafe { std::net::TcpListener::from_raw_socket(s as u64) })
    }

    /// Windows has no `exec`; the caller spawns and exits instead.
    pub fn exec(_cmd: &mut std::process::Command) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Unsupported, "exec is Unix-only")
    }
}

pub use imp::{adopt_listener, adopt_stream, dup_for_child, exec, keep_open};

/// Bring the platform's socket library up.
///
/// Windows wants `WSAStartup` before any ws2_32 call, and a process that
/// adopts all of its sockets from a copyover makes that call before it has
/// created a socket of its own. Everywhere else this is a no-op.
pub fn init_sockets() -> std::io::Result<()> {
    imp::init()
}

pub mod resolve;

/// True when this platform replaces the process image in place (Unix) rather
/// than spawning a successor and exiting (Windows).
pub const EXEC_IN_PLACE: bool = cfg!(unix);
