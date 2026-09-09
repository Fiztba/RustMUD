use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn connection_burst_respects_limit_before_dns_promotion() {
    let root = std::env::temp_dir().join(format!("rustmud-admission-{}", std::process::id()));
    let lib = root.join("lib");
    copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib"), &lib);
    for slow in [0, 1] {
        std::fs::write(lib.join("etc/config"), format!(
            "max_playing = 2\nnameserver_is_slow = {slow}\ndflt_ip = 127.0.0.1\n"
        )).unwrap();
        let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = reserved.local_addr().unwrap();
        drop(reserved);
        let mut server = Server(Command::new(env!("CARGO_BIN_EXE_circle"))
            .args(["-q", "-d"]).arg(&lib).arg(addr.port().to_string())
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
        // Connect during boot: all sockets are waiting when the first
        // accept phase runs, before any descriptor has been promoted.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut clients = Vec::new();
        while clients.len() < 8 {
            assert!(Instant::now() < deadline, "server did not accept connections");
            assert!(server.0.try_wait().unwrap().is_none(), "server exited during boot");
            match TcpStream::connect_timeout(&addr, Duration::from_millis(100)) {
                Ok(client) => clients.push(client),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        let mut rejected = 0;
        for client in &mut clients {
            client.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
            let mut first = [0; 5];
            client.read_exact(&mut first).unwrap();
            if &first == b"Sorry" { rejected += 1; }
        }
        assert_eq!(rejected, 6, "nameserver_is_slow={slow}");
        drop(clients);
        drop(server);
    }
    std::fs::remove_dir_all(root).unwrap();
}
