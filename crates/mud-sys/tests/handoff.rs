//! The copyover socket handoff, end to end across a real process boundary.
//!
//! Windows only, because it is the only platform where the handoff is a thing
//! this crate *does* rather than something the kernel does for it: elsewhere
//! the sockets go over by descriptor inheritance across `exec`, which cannot
//! be exercised without replacing the test binary itself.
//!
//! The predecessor binds a listener, takes a connection on it, duplicates
//! both into a freshly spawned successor and drops its own copies. The
//! successor is a second run of this same test binary, so it reaches
//! `adopt_listener` having never created a socket of its own — the exact
//! state a recovering server is in, and the one that used to fail with
//! WSANOTINITIALISED and take every player on the MUD down with it.

#![cfg(windows)]

use std::io::{Read, Write};

/// Set on the re-invoked run, naming the directory the blobs arrive in.
const ROLE: &str = "RUSTMUD_HANDOFF_SUCCESSOR";
const TEST_NAME: &str = "sockets_survive_a_process_handoff";

const GREETING: &[u8] = b"Restoring from copyover...";
const SERVED: &[u8] = b"new connection served";

#[test]
fn sockets_survive_a_process_handoff() {
    match std::env::var(ROLE) {
        Ok(dir) => successor(&dir),
        Err(_) => predecessor(),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn unhex(h: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(h.len() / 2);
    let mut i = 0;
    while i + 1 < h.len() {
        let hi = (h[i] as char).to_digit(16).expect("hex digit") as u8;
        let lo = (h[i + 1] as char).to_digit(16).expect("hex digit") as u8;
        out.push(hi << 4 | lo);
        i += 2;
    }
    out
}

fn predecessor() {
    let dir = std::env::temp_dir().join(format!("rustmud-handoff-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");

    // A player, connected before the handoff, who must not notice it.
    let player = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(addr).expect("player connect");
        let mut buf = [0u8; 64];
        let n = s.read(&mut buf).expect("player read");
        buf[..n].to_vec()
    });
    let (stream, _) = listener.accept().expect("accept");

    // The successor: this binary again, running this test alone, in the role
    // the environment names.
    let exe = std::env::current_exe().expect("current exe");
    let mut succ = std::process::Command::new(&exe)
        .arg(TEST_NAME)
        .arg("--exact")
        .arg("--nocapture")
        .env(ROLE, dir.to_str().expect("utf-8 scratch path"))
        .spawn()
        .expect("spawn successor");

    let lblob = mud_sys::dup_for_child(&listener, succ.id()).expect("duplicate listener");
    let sblob = mud_sys::dup_for_child(&stream, succ.id()).expect("duplicate player socket");
    std::fs::write(dir.join("listener.hex"), hex(&lblob)).expect("write listener blob");
    std::fs::write(dir.join("stream.hex"), hex(&sblob)).expect("write stream blob");
    std::fs::write(dir.join("ready"), b"go").expect("write ready");

    // The predecessor is done with both sockets, as exiting would leave it.
    drop(stream);
    drop(listener);

    let saw = player.join().expect("player thread");
    assert_eq!(saw, GREETING, "the established connection did not survive the handoff");

    // And the adopted listener has to still be a listener.
    let mut fresh = std::net::TcpStream::connect(addr).expect("connect after handoff");
    let mut buf = [0u8; 64];
    let n = fresh.read(&mut buf).expect("read after handoff");
    assert_eq!(&buf[..n], SERVED, "the adopted listener stopped accepting");

    let status = succ.wait().expect("wait for successor");
    assert!(status.success(), "successor failed: {}", status);
    let _ = std::fs::remove_dir_all(&dir);
}

fn successor(dirs: &str) {
    let dir = std::path::PathBuf::from(dirs);
    for _ in 0..400 {
        if dir.join("ready").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let lblob = unhex(&std::fs::read(dir.join("listener.hex")).expect("listener blob"));
    let sblob = unhex(&std::fs::read(dir.join("stream.hex")).expect("stream blob"));

    // The first ws2_32 call this process makes.
    let listener = mud_sys::adopt_listener(-1, &lblob).expect("adopt listener");
    let mut player = mud_sys::adopt_stream(-1, &sblob).expect("adopt player socket");
    player.write_all(GREETING).expect("greet the recovered player");

    let (mut fresh, _) = listener.accept().expect("accept on the adopted listener");
    fresh.write_all(SERVED).expect("serve the new connection");
}
