//! The RustMUD server binary: the `circle` CLI flags
//! the select-equivalent mio loop with the exact pulse
//! timing semantics, and syslog output.

use std::io::Write;
use std::net::IpAddr;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mio::net::TcpListener;
use mio::{Events, Interest, Poll, Token};

use mud_data::types::*;
use mud_game::game::Game;
use mud_game::run::{boot_game, game_pulse, new_connection, save_mud_time, BootFlags};

const LISTENER: Token = Token(usize::MAX - 1);

fn timestamp(now: i64, tz_offset_secs: i64) -> String {
    // "Aug 22 20:05:00 2026" — basic_mud_log's asctime-derived stamp
    // (log format: skip the leading weekday of ctime).
    let full = mud_game::act::wizard::ctime_like(now, tz_offset_secs);
    full.get(4..).unwrap_or(&full).to_string()
}

struct Logger {
    file: Option<std::fs::File>,
    tz: i64,
    /// The data directory; the topic files live beside it, in ../log/.
    lib: std::path::PathBuf,
}

/// The per-topic files under log/ that the file command reads. tbaMUD's
/// autorun script filled them by grepping the syslog when the game exited,
/// so between restarts they were stale, and a copyover never exits. The
/// server appends each matching line itself, as it is logged. The patterns
/// and files are autorun's own table, matched as fgrep matched them:
/// anywhere in the line, case-sensitively.
const TOPIC_LOGS: [(&str, &str); 15] = [
    ("self-delete", "delete"),
    ("PCLEAN", "delete"),
    ("death trap", "dts"),
    ("killed", "rip"),
    ("Running", "restarts"),
    ("advanced", "levels"),
    ("equipment lost", "rentgone"),
    ("usage", "usage"),
    ("new player", "newplayers"),
    ("SYSERR", "errors"),
    ("(GC)", "godcmds"),
    ("Bad PW", "badpws"),
    ("OLC", "olc"),
    ("get help on", "help"),
    ("trigger", "trigger"),
];

impl Logger {
    fn log(&mut self, now: i64, msg: &str) {
        let line = format!("{} :: {}\n", timestamp(now, self.tz), msg);
        match &mut self.file {
            Some(f) => {
                let _ = f.write_all(line.as_bytes());
            }
            None => {
                let _ = std::io::stderr().write_all(line.as_bytes());
            }
        }
        // A topic file that cannot be opened (no log/ directory, say) is
        // simply not kept; nothing here logs, which would recurse.
        for (pattern, name) in TOPIC_LOGS {
            if !msg.contains(pattern) {
                continue;
            }
            let path = self.lib.join("..").join("log").join(name);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }
}

fn drain_logs(g: &mut Game, logger: &mut Logger) {
    let lines = std::mem::take(&mut g.log_lines);
    for l in lines {
        logger.log(g.now, &l);
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// How long a new connection waits for its reverse lookup before it
/// is greeted under its numeric address. A blocking resolver can take up to
/// 10s on stock glibc settings; here only this one connection waits, and
/// only until this deadline.
const RESOLVE_DEADLINE: Duration = Duration::from_secs(5);

/// A socket that has been accepted but not yet greeted, because its
/// hostname is still being looked up. Nothing about it is in the game yet:
/// no descriptor exists until the ban check runs on the resolved name.
struct PendingConn {
    stream: mio::net::TcpStream,
    ip: IpAddr,
    deadline: Instant,
    /// `None` when nameserver_is_slow is set, i.e. no lookup was asked for.
    rx: Option<mpsc::Receiver<Option<String>>>,
}

/// The resolver thread. It touches no game state — it takes an address and
/// returns a name, which is the whole reason the wait can be moved off the
/// loop without any locking.
fn spawn_resolver() -> mpsc::Sender<(IpAddr, mpsc::Sender<Option<String>>)> {
    let (tx, rx) = mpsc::channel::<(IpAddr, mpsc::Sender<Option<String>>)>();
    std::thread::Builder::new()
        .name("resolver".into())
        .spawn(move || {
            // MUD_RESOLVE_AS is a test hook, in the same family as MUD_SEED:
            // the lookup still runs, with its real timing and its real
            // failure path, and only the OS's ANSWER is substituted. Linux
            // reads 127.0.0.1 as "localhost" out of /etc/hosts while Windows
            // answers the same getnameinfo with the machine name, so without
            // this a host column's width is platform-dependent -- which is
            // what hid a justification bug in `last`.
            let force = std::env::var("MUD_RESOLVE_AS").ok();
            while let Ok((ip, reply)) = rx.recv() {
                let got = mud_sys::resolve::reverse_lookup(ip);
                let _ = reply.send(match (&force, got) {
                    (Some(name), Some(_)) => Some(name.clone()),
                    (_, got) => got,
                });
            }
        })
        .expect("resolver thread");
    tx
}

/// `--rebuild-index`: reconstruct `lib/plrfiles/index` from the `.plr`
/// files. Prints what it found and what it could not recover, because a
/// rebuild silently losing the PINDEX_* bits is worse than one that says so.
fn run_rebuild_index(lib: &std::path::Path) -> i32 {
    println!("Rebuilding {}...", lib.join("plrfiles").join("index").display());
    let report = match mud_world::rebuild::rebuild_player_index(lib) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("SYSERR: {e}");
            return 1;
        }
    };

    println!("{} player(s) indexed.", report.entries.len());

    if !report.no_id.is_empty() {
        println!("\n{} file(s) had no `Id  :` line and were SKIPPED:", report.no_id.len());
        for n in &report.no_id {
            println!("  {n}");
        }
        println!("  A player file with no id cannot be indexed; the rest of the");
        println!("  game keys on it. These characters are not in the new index.");
    }

    if !report.unreadable.is_empty() {
        println!("\n{} problem(s) while reading:", report.unreadable.len());
        for (what, why) in &report.unreadable {
            println!("  {what}: {why}");
        }
    }

    println!("\nNOT recovered, because no player file records them:");
    println!("  NODELETE   - protected characters are deletable again.");
    println!("  DELETED    - a character flagged for deletion but not yet");
    println!("               swept by remove_player() is back as an ordinary");
    println!("               player.");
    println!("  SELFDELETE - as above.");
    println!("  NOWIZLIST  - an immortal kept off the wizlist reappears on it");
    println!("               at the next autowiz run.");
    println!("Re-apply those by editing the index by hand if any were set.");
    0
}

/// `--import-binary-pfiles`: convert a pre-3.x CircleMUD binary player
/// file into ASCII pfiles. The layout it assumes is stated up front,
/// because a wrong one decodes silently into somebody's level and gold.
fn run_import_pfiles(
    lib: &std::path::Path,
    src: &std::path::Path,
    endian: mud_world::import_binary::Endian,
    dry_run: bool,
) -> i32 {
    use mud_world::import_binary::{import_binary_pfiles, Endian, RECORD_SIZE};
    println!("Reading {} ...", src.display());
    println!(
        "Assuming stock CircleMUD 3.0 on a 32-bit host: {RECORD_SIZE}-byte records, \
{} endian.",
        if endian == Endian::Big { "big" } else { "little" }
    );
    if dry_run {
        println!("Dry run: nothing will be written.");
    }

    let report = match import_binary_pfiles(lib, src, endian, dry_run) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nSYSERR: {e}");
            eprintln!("\nIf the file came from a different machine, try --endian big.");
            eprintln!("A file from a 64-bit host has a different layout entirely and");
            eprintln!("cannot be read by this command.");
            return 1;
        }
    };

    println!("\n{} character(s) converted.", report.imported.len());
    for n in &report.imported {
        println!("  {}", String::from_utf8_lossy(n));
    }
    if !report.existing.is_empty() {
        println!("\n{} already had a .plr file and were LEFT ALONE:", report.existing.len());
        for n in &report.existing {
            println!("  {}", String::from_utf8_lossy(n));
        }
    }
    if !report.skipped.is_empty() {
        println!("\n{} record(s) skipped:", report.skipped.len());
        for (i, why) in &report.skipped {
            println!("  record {i}: {why}");
        }
    }
    if !report.imported.is_empty() && !dry_run {
        println!("\nRun --rebuild-index next: the imported characters are not in");
        println!("lib/plrfiles/index until it is rebuilt, and cannot log in.");
    }
    0
}

/// Open the mother connection: the listening socket every player arrives on.
///
/// Built by hand rather than with `TcpListener::bind` because `SO_REUSEADDR`
/// has to be set on the socket *before* it is bound, and there is no way to
/// apply it to a listener `bind` has already returned. Without it a restart
/// inside the kernel's teardown window fails outright: on Unix the listening
/// socket lingers for up to 2*MSL after any shutdown that had live
/// connections, which is exactly when a supervising script restarts the game.
/// Failing to set it is therefore fatal -- the alternative is a server that
/// cannot be restarted.
///
/// On Windows the option is stronger: it permits binding over a socket that is
/// still live, not only over one in teardown.
///
/// The bind address comes from `DFLT_IP` in `lib/etc/config`, which is why the
/// config is read here, before the socket exists. An unparseable address is
/// logged and falls back to every interface rather than refusing to boot. The
/// log lines that read produces are discarded: `boot_game` loads the config
/// again and keeps them, and emitting them twice would double every line in
/// the syslog.
///
/// `SO_SNDBUF` and `SO_LINGER` are set as well; neither is worth aborting
/// over.
fn open_mother_connection(
    port: u16,
    lib_dir: &str,
    logger: &mut Logger,
    boot_now: i64,
) -> std::io::Result<std::net::TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    const SEND_BUFFER: usize = 24 * 1024;

    let dflt_ip = {
        let mut cfg = mud_game::config::Config::default();
        let _ = mud_game::config_file::load_config(std::path::Path::new(lib_dir), &mut cfg);
        cfg.dflt_ip.clone()
    };

    // An unparseable address is not fatal: log it and fall back to every
    // interface rather than refusing to start.
    let bind_ip: IpAddr = match &dflt_ip {
        None => IpAddr::from([0, 0, 0, 0]),
        Some(raw) => {
            let text = String::from_utf8_lossy(raw).into_owned();
            match text.parse::<std::net::Ipv4Addr>() {
                Ok(v4) => IpAddr::V4(v4),
                Err(_) => {
                    logger.log(
                        boot_now,
                        &format!(
                            "SYSERR: DFLT_IP of {} appears to be an invalid IP address",
                            text
                        ),
                    );
                    IpAddr::from([0, 0, 0, 0])
                }
            }
        }
    };

    if bind_ip.is_unspecified() {
        logger.log(boot_now, "Binding to all IP interfaces on this host.");
    } else {
        logger.log(boot_now, &format!("Binding only to IP address {}", bind_ip));
    }

    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    // Fatal: without it the next restart is the one that fails.
    sock.set_reuse_address(true)?;
    // Neither is worth aborting over.
    let _ = sock.set_send_buffer_size(SEND_BUFFER);
    let _ = sock.set_linger(None);

    sock.bind(&std::net::SocketAddr::new(bind_ip, port).into())?;
    // The C listened with a backlog of 5, and a reconnect flood after a
    // crash showed what that costs: the queue fills, the kernel drops the
    // SYNs behind it, and every client past the fifth sits in TCP's
    // retransmit back-off, so 250 players took 42 seconds to be greeted.
    // 128 is what the kernel caps at by default; it holds a whole reconnect
    // burst while the loop drains it.
    sock.listen(128)?;
    Ok(sock.into())
}

fn usage(argv0: &str) {
    eprintln!("Usage: {argv0} [options] [port]");
    eprintln!("  -d <dir>   data directory (default: lib)");
    eprintln!("  -o <file>  log to <file>");
    eprintln!("  -m         mini-mud, no rent check");
    eprintln!("  -c         syntax check only");
    eprintln!("  -q         quick boot, no rent check");
    eprintln!("  --rebuild-index");
    eprintln!("             rebuild lib/plrfiles/index from the .plr files and");
    eprintln!("             exit. For when the index is lost or corrupt: every");
    eprintln!("             player file survives that, but nobody can log in.");
    eprintln!("  --import-binary-pfiles <file>");
    eprintln!("             convert a pre-3.x CircleMUD binary player file to");
    eprintln!("             ASCII pfiles. Assumes 32-bit CircleMUD 3.0 layout;");
    eprintln!("             refuses rather than guessing if it does not fit.");
    eprintln!("  --endian little|big");
    eprintln!("             byte order of the machine that wrote it (default little)");
    eprintln!("  --dry-run  with --import-binary-pfiles, report without writing");
    eprintln!("  --help     this text");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut dir = "lib".to_string();
    let mut port: u16 = 4000;
    let mut flags = BootFlags::default();
    let mut logname: Option<String> = None;
    let mut scheck = false;
    let mut copyover_fd: Option<i64> = None;
    let mut rebuild_index = false;
    let mut import_pfiles: Option<String> = None;
    let mut import_endian = mud_world::import_binary::Endian::Little;
    let mut import_dry_run = false;
    let mut pos_args: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        // Long options name the maintenance modes that ship as separate
        // programs. Checked before
        // the single-character path below, which would otherwise read
        // "--rebuild-index" as the option '-' with an inline value.
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "rebuild-index" => rebuild_index = true,
                "import-binary-pfiles" => {
                    if i + 1 >= args.len() {
                        eprintln!("SYSERR: --import-binary-pfiles needs a file.");
                        std::process::exit(1);
                    }
                    i += 1;
                    import_pfiles = Some(args[i].clone());
                }
                "endian" => {
                    if i + 1 >= args.len() {
                        eprintln!("SYSERR: --endian needs little or big.");
                        std::process::exit(1);
                    }
                    i += 1;
                    import_endian = match args[i].as_str() {
                        "little" => mud_world::import_binary::Endian::Little,
                        "big" => mud_world::import_binary::Endian::Big,
                        other => {
                            eprintln!("SYSERR: --endian must be little or big, not {other}.");
                            std::process::exit(1);
                        }
                    };
                }
                "dry-run" => import_dry_run = true,
                "help" => {
                    usage(&args[0]);
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("SYSERR: Unknown option --{long}.");
                    usage(&args[0]);
                    std::process::exit(1);
                }
            }
            i += 1;
            continue;
        }
        if let Some(rest) = arg.strip_prefix('-') {
            let (opt, inline) = rest.split_at(1.min(rest.len()));
            match opt {
                "d" => {
                    if !inline.is_empty() {
                        dir = inline.to_string();
                    } else if i + 1 < args.len() {
                        i += 1;
                        dir = args[i].clone();
                    } else {
                        eprintln!("SYSERR: Directory arg expected after option -d.");
                        std::process::exit(1);
                    }
                }
                "o" => {
                    if !inline.is_empty() {
                        logname = Some(inline.to_string());
                    } else if i + 1 < args.len() {
                        i += 1;
                        logname = Some(args[i].clone());
                    }
                }
                "m" => {
                    flags.mini_mud = true;
                    flags.no_rent_check = true;
                    println!("Running in minimized mode & with no rent check.");
                }
                "c" => {
                    scheck = true;
                    println!("Syntax check mode enabled.");
                }
                "q" => {
                    flags.no_rent_check = true;
                    println!("Quick boot mode -- rent check supressed.");
                }
                "r" => {
                    flags.restrict = 1;
                    println!("Restricting game -- no new players allowed.");
                }
                "s" => {
                    flags.no_specials = true;
                    println!("Suppressing assignment of special routines.");
                }
                "C" => {
                    // C<mother socket> — copyover recovery.
                    copyover_fd = Some(if inline.is_empty() {
                        -1
                    } else {
                        inline.parse::<i64>().unwrap_or(-1)
                    });
                }
                "h" => {
                    println!(
                        "Usage: {} [-c] [-m] [-q] [-r] [-s] [-d pathname] [port #]",
                        args[0]
                    );
                    std::process::exit(0);
                }
                _ => {
                    eprintln!("SYSERR: Unknown option -{} in argument string.", opt);
                }
            }
        } else {
            pos_args.push(arg.clone());
        }
        i += 1;
    }
    if let Some(p) = pos_args.first() {
        match p.parse::<u16>() {
            Ok(n) if n > 1024 => port = n,
            _ => {
                eprintln!("SYSERR: Illegal port number {}.", p);
                std::process::exit(1);
            }
        }
    }

    let mut logger = Logger {
        // Appended, not truncated: a copyover reopens this same file in the
        // successor, and a log that erased itself on every reboot would keep
        // exactly the history that is least useful.
        file: logname
            .as_ref()
            .and_then(|n| std::fs::OpenOptions::new().create(true).append(true).open(n).ok()),
        tz: mud_game::run::local_tz_offset_secs(0),
        lib: std::path::PathBuf::from(&dir),
    };
    let boot_now = now_unix();
    logger.log(boot_now, "Loading configuration.");
    logger.log(boot_now, mud_data::tables::TBAMUD_VERSION);
    logger.log(boot_now, &format!("Using {} as data directory.", dir));

    let lib_dir = std::path::PathBuf::from(&dir);

    if rebuild_index {
        std::process::exit(run_rebuild_index(&lib_dir));
    }

    if let Some(src) = import_pfiles {
        std::process::exit(run_import_pfiles(
            &lib_dir,
            std::path::Path::new(&src),
            import_endian,
            import_dry_run,
        ));
    }

    if scheck {
        match boot_game(lib_dir, flags, boot_now, boot_now) {
            Ok(mut g) => {
                drain_logs(&mut g, &mut logger);
                logger.log(boot_now, "Done.");
                std::process::exit(0);
            }
            Err(e) => {
                logger.log(boot_now, &format!("SYSERR: {}", e));
                std::process::exit(1);
            }
        }
    }

    logger.log(boot_now, &format!("Running game on port {}.", port));

    // Seeded from the wall clock; MUD_SEED overrides it.
    let seed = std::env::var("MUD_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(boot_now);

    // Copyover recovery reads the handoff file before anything else so the
    // inherited listener can be adopted in place of a fresh bind.
    let copyover_file = copyover_fd.and_then(|_| {
        let path = std::path::PathBuf::from(&dir).join("..").join("copyover.dat");
        let f = await_handoff_file(&path);
        if f.is_none() {
            logger.log(boot_now, "copyover_recover:fopen");
            logger.log(boot_now, "Copyover file not found. Exiting.");
            std::process::exit(1);
        }
        let _ = std::fs::remove_file(&path);
        f
    });

    let mut listener = match &copyover_file {
        Some(cf) => {
            let raw = mud_sys::adopt_listener(copyover_fd.unwrap_or(-1), &cf.listener_blob);
            match raw {
                Ok(l) => {
                    l.set_nonblocking(true).ok();
                    TcpListener::from_std(l)
                }
                Err(e) => {
                    logger.log(boot_now, &format!("SYSERR: copyover listener: {}", e));
                    std::process::exit(1);
                }
            }
        }
        None => {
            logger.log(boot_now, "Opening mother connection.");
            match open_mother_connection(port, &dir, &mut logger, boot_now) {
                Ok(l) => {
                    l.set_nonblocking(true).ok();
                    TcpListener::from_std(l)
                }
                Err(e) => {
                    logger.log(boot_now, &format!("SYSERR: Error creating socket: {}", e));
                    std::process::exit(1);
                }
            }
        }
    };

    let mut g = match boot_game(lib_dir, flags, seed, boot_now) {
        Ok(g) => g,
        Err(e) => {
            logger.log(boot_now, &format!("SYSERR: {}", e));
            std::process::exit(1);
        }
    };
    drain_logs(&mut g, &mut logger);

    let mut poll = Poll::new().expect("poll");
    let mut events = Events::with_capacity(256);
    poll.registry().register(&mut listener, LISTENER, Interest::READABLE).expect("register");

    let mut next_token: usize = 0;
    let mut token_map: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

    // Reverse DNS happens here, not on the accept path.
    let resolve_tx = spawn_resolver();
    let mut pending: Vec<PendingConn> = Vec::new();

    if let Some(cf) = copyover_file {
        logger.log(g.now, "Copyover recovery initiated");
        copyover_recover(&mut g, cf, &mut poll, &mut next_token, &mut token_map);
        drain_logs(&mut g, &mut logger);
    }

    logger.log(g.now, "Entering game loop.");

    // The pulse engine: 100ms pulses with catch-up.
    const OPT_USEC: u64 = 100_000;
    let opt = Duration::from_micros(OPT_USEC);
    let mut last_time = Instant::now();

    while !g.circle_shutdown {
        // Empty-MUD sleep: block on the listener alone.
        // A connection waiting on its reverse lookup has no descriptor
        // yet, so it would never be promoted if we slept through its
        // deadline — stay awake while any are pending.
        if g.descriptors.is_empty() && pending.is_empty() {
            logger.log(g.now, "No connections.  Going to sleep.");
            loop {
                match poll.poll(&mut events, None) {
                    Ok(()) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        logger.log(g.now, &format!("SYSERR: Select coma: {}", e));
                        std::process::exit(1);
                    }
                }
            }
            logger.log(g.now, "New connection.  Waking up.");
            last_time = Instant::now();
        }

        // Sleep to the next pulse boundary, tracking missed pulses.
        let now_i = Instant::now();
        let process_time = now_i.duration_since(last_time);
        let mut missed_pulses: u64 = 0;
        if process_time >= opt {
            missed_pulses = process_time.as_micros() as u64 / OPT_USEC;
            let remainder = Duration::from_micros(process_time.as_micros() as u64 % OPT_USEC);
            last_time = now_i - remainder;
        }
        last_time += opt;
        let now_i = Instant::now();
        if last_time > now_i {
            std::thread::sleep(last_time - now_i);
        }

        // Poll with zero timeout purely to reap readiness notifications; the
        // accept and read paths below run unconditionally on nonblocking
        // sockets, so event contents don't matter (select-equivalent).
        let _ = poll.poll(&mut events, Some(Duration::ZERO));

        loop {
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    if g.descriptors.len() as i32 >= g.config.max_playing {
                        let _ = std::io::Write::write_all(
                            &mut stream,
                            b"Sorry, the game is full right now... please try again later!\r\n",
                        );
                        continue;
                    }
                    // Resolving inline here would block the whole game
                    // loop, because the ban check needs the hostname. Park
                    // the socket instead and let the worker answer; the ban
                    // check still runs on the resolved name.
                    let ip = peer.ip();
                    let rx = if g.config.nameserver_is_slow {
                        None
                    } else {
                        let (tx, rx) = mpsc::channel();
                        resolve_tx.send((ip, tx)).ok().map(|()| rx)
                    };
                    pending.push(PendingConn {
                        stream,
                        ip,
                        deadline: Instant::now() + RESOLVE_DEADLINE,
                        rx,
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    logger.log(g.now, &format!("SYSERR: accept: {}", e));
                    break;
                }
            }
        }

        // Promote the connections whose lookup has landed, or whose
        // deadline has passed — those fall back to the numeric form.
        let mut resolved: Vec<(mio::net::TcpStream, String)> = Vec::new();
        let mut still_waiting: Vec<PendingConn> = Vec::new();
        for mut p in pending.drain(..) {
            let answer = match p.rx.as_ref() {
                None => Some(p.ip.to_string()),
                Some(rx) => match rx.try_recv() {
                    Ok(Some(name)) => Some(name),
                    Ok(None) | Err(mpsc::TryRecvError::Disconnected) => Some(p.ip.to_string()),
                    Err(mpsc::TryRecvError::Empty) => {
                        if Instant::now() >= p.deadline {
                            Some(p.ip.to_string())
                        } else {
                            None
                        }
                    }
                },
            };
            match answer {
                Some(host) => resolved.push((p.stream, host)),
                None => {
                    p.rx = p.rx.take();
                    still_waiting.push(p);
                }
            }
        }
        pending = still_waiting;

        for (stream, host) in resolved {
            // Site ban — before the descriptor exists.
            if mud_game::ban::isbanned(&g, host.as_bytes()) == mud_game::ban::BAN_ALL {
                drop(stream);
                g.mudlog(
                    mud_game::game::MudlogKind::Cmp,
                    mud_data::types::LVL_GOD,
                    true,
                    &format!("Connection attempt denied from [{}]", host),
                );
                continue;
            }
            let di = new_connection(&mut g, Some(stream), host.as_bytes());
            let tok = next_token;
            next_token += 1;
            token_map.insert(tok, di);
            if let Some(d) = g.descriptors.get_mut(di) {
                if let Some(s) = d.stream.as_mut() {
                    let _ = poll.registry().register(s, Token(tok), Interest::READABLE);
                }
            }
        }

        // Refresh wall clock once per iteration.
        g.now = now_unix();

        // Input phase: drain every descriptor (nonblocking reads; EAGAIN is
        // free) — readiness events only serve as wakeups.
        let mut dead: Vec<usize> = Vec::new();
        for di in g.descriptors.indices() {
            if g.descriptors.get(di).is_none_or(|d| d.stream.is_none()) {
                continue;
            }
            match g.descriptors.process_input(di) {
                Ok(bugs) => {
                    for b in bugs {
                        g.log(b);
                    }
                }
                Err(bugs) => {
                    for b in bugs {
                        g.log(b);
                    }
                    dead.push(di);
                }
            }
        }
        for di in dead {
            mud_game::run::close_socket(&mut g, di);
        }

        // Pulses: run the current one plus any missed (cap 300).
        let mut to_run = missed_pulses + 1;
        if to_run > 30 * PASSES_PER_SEC {
            logger.log(
                g.now,
                &format!("SYSERR: Missed {} seconds worth of pulses.", to_run / PASSES_PER_SEC),
            );
            to_run = 30 * PASSES_PER_SEC;
        }
        for _ in 0..to_run {
            game_pulse(&mut g);
        }
        drain_logs(&mut g, &mut logger);

        if g.copyover.is_some() {
            perform_copyover(
                &mut g,
                &mut listener,
                &args,
                port,
                &dir,
                logname.as_deref(),
                &mut logger,
            );
            // Only reached when the handoff failed.
            drain_logs(&mut g, &mut logger);
        }
    }

    // Shutdown sequence. `save_all` — the OLC pending list —
    // is skipped for `shutdown reboot`/`now`, which set circle_reboot to 2
    // precisely to avoid it. B23: houses are saved here too — otherwise a
    // clean shutdown loses everything dropped in a house since the last
    // autosave.
    mud_game::objsave::crash_save_all(&mut g);
    mud_game::house::house_save_all(&mut g);
    logger.log(g.now, "Closing all sockets.");
    for di in g.descriptors.indices() {
        mud_game::run::close_socket(&mut g, di);
    }
    if g.circle_reboot != 2 {
        mud_game::db::save_all(&mut g);
    }
    logger.log(g.now, "Saving current MUD time.");
    save_mud_time(&mut g);
    drain_logs(&mut g, &mut logger);
    if g.circle_reboot != 0 {
        logger.log(g.now, "Rebooting.");
        relaunch(&args, g.now, &mut logger);
    }
    logger.log(g.now, "Normal termination of game.");
}

/// `shutdown reboot` / `shutdown now`: start this binary again.
///
/// The C leaves this to its `autorun` script -- it exits 52 and trusts a
/// loop outside the process to run it again. Nothing of the kind ships
/// here, so a reboot has to bring the game back by itself, and it does so
/// the way copyover does: on Unix the process image is replaced with a
/// fresh run of the binary on disk (so a reboot after a rebuild loads the
/// new build, as it does under autorun), and on Windows a successor is
/// spawned and this process exits.
///
/// The arguments are the ones this process was started with, less any
/// copyover handoff (`-C`): the successor binds its own listener. The
/// working directory is inherited, so a relative `-d` still resolves.
///
/// Exit codes keep their documented meaning for anyone who does run a
/// supervising loop: on Unix an exec never returns, so the loop sees
/// nothing; on Windows a successful spawn exits 0, because the game is
/// already back and a loop that restarted on 52 would start a second copy.
/// 52 is reached only when the relaunch itself failed, which is the one
/// case where outside help is still wanted.
fn relaunch(args: &[String], now: i64, logger: &mut Logger) {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from(&args[0]));
    let mut cmd = std::process::Command::new(&exe);
    let mut skip = false;
    for a in &args[1..] {
        if skip {
            skip = false;
            continue;
        }
        if let Some(rest) = a.strip_prefix("-C") {
            skip = rest.is_empty();
            continue;
        }
        cmd.arg(a);
    }

    if mud_sys::EXEC_IN_PLACE {
        let err = mud_sys::exec(&mut cmd);
        logger.log(now, &format!("SYSERR: reboot: exec: {}", err));
        std::process::exit(52);
    }
    match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            logger.log(now, &format!("SYSERR: reboot: spawn: {}", e));
            std::process::exit(52);
        }
    }
}

/// Pass the log file on to the successor.
///
/// A reboot is not supposed to be visible in the log beyond the boot lines
/// themselves. `-o` opens the file inside the process, so unlike a shell
/// redirection it does not carry across on its own and has to be handed to
/// the successor explicitly, or the log stops at the copyover.
fn with_log(cmd: &mut std::process::Command, logname: Option<&str>) {
    if let Some(l) = logname {
        cmd.arg("-o").arg(l);
    }
}

/// How long a successor waits for the handoff file to appear.
///
/// Where the process image is replaced in place, copyover.dat is complete
/// before the successor exists at all, so a miss there is final. Where the
/// successor has to be spawned instead, it starts running *before* the file
/// is written — the predecessor needs its pid to duplicate the sockets into
/// it — and an early miss only means the handoff is still on its way.
const HANDOFF_WAIT: Duration = Duration::from_secs(10);

fn await_handoff_file(path: &std::path::Path) -> Option<mud_game::copyover::CopyoverFile> {
    let deadline = Instant::now() + HANDOFF_WAIT;
    loop {
        if let Some(f) = mud_game::copyover::read_copyover_file(path) {
            return Some(f);
        }
        if mud_sys::EXEC_IN_PLACE || Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The tail of do_copyover: hand every socket to the
/// successor and replace this process.
fn perform_copyover(
    g: &mut Game,
    listener: &mut TcpListener,
    args: &[String],
    port: u16,
    dir: &str,
    logname: Option<&str>,
    logger: &mut Logger,
) {
    let Some(plan) = g.copyover.take() else { return };
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from(&args[0]));

    // Every process-image replacement leaves the working directory alone; the
    // A chdir("..") would be needed when running from lib/ and exec'ing
    // ../bin/circle. Here
    // pass -d explicitly instead, which is equivalent and platform-neutral.
    let mut plan = plan;
    let mut cmd = std::process::Command::new(&exe);

    if mud_sys::EXEC_IN_PLACE {
        // Unix: clear CLOEXEC everywhere, write the file, exec.
        let fd = match mud_sys::keep_open(listener) {
            Ok(fd) => fd,
            Err(e) => {
                logger.log(g.now, &format!("SYSERR: copyover keep_open(mother): {}", e));
                return;
            }
        };
        for (i, di) in plan.descs.clone().into_iter().enumerate() {
            if let Some(d) = g.descriptors.get(di) {
                if let Some(st) = d.stream.as_ref() {
                    match mud_sys::keep_open(st) {
                        Ok(sfd) => plan.entries[i].fd = sfd,
                        Err(e) => logger.log(g.now, &format!("SYSERR: copyover keep_open: {}", e)),
                    }
                }
            }
        }
        if let Err(e) = mud_game::copyover::write_copyover_file(g, &plan, &[]) {
            logger.log(g.now, &format!("SYSERR: copyover file: {}", e));
            return;
        }
        cmd.arg(format!("-C{}", fd)).arg("-d").arg(dir);
        with_log(&mut cmd, logname);
        cmd.arg(port.to_string());
        let err = mud_sys::exec(&mut cmd);
        logger.log(g.now, &format!("do_copyover: exec: {}", err));
        std::process::exit(1);
    }

    // Windows: spawn first (we need the pid), then duplicate into it.
    cmd.arg("-C0").arg("-d").arg(dir);
    with_log(&mut cmd, logname);
    cmd.arg(port.to_string());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            logger.log(g.now, &format!("do_copyover: spawn: {}", e));
            return;
        }
    };
    let pid = child.id();

    // From here the successor is already running and waiting on the handoff
    // file. Anything that stops us writing it leaves this process holding the
    // game, so the successor has to be stopped rather than left to sit out
    // its wait on a file that is never coming.
    let listener_blob = match mud_sys::dup_for_child(listener, pid) {
        Ok(b) => b,
        Err(e) => {
            logger.log(g.now, &format!("SYSERR: copyover dup(mother): {}", e));
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
    };
    for (i, di) in plan.descs.clone().into_iter().enumerate() {
        if let Some(d) = g.descriptors.get(di) {
            if let Some(st) = d.stream.as_ref() {
                match mud_sys::dup_for_child(st, pid) {
                    Ok(b) => plan.entries[i].blob = b,
                    Err(e) => logger.log(g.now, &format!("SYSERR: copyover dup: {}", e)),
                }
            }
        }
    }
    if let Err(e) = mud_game::copyover::write_copyover_file(g, &plan, &listener_blob) {
        logger.log(g.now, &format!("SYSERR: copyover file: {}", e));
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    logger.log(g.now, "Copyover handoff written; successor is taking over.");
    std::process::exit(0);
}

fn copyover_recover(
    g: &mut Game,
    cf: mud_game::copyover::CopyoverFile,
    poll: &mut Poll,
    next_token: &mut usize,
    token_map: &mut std::collections::HashMap<usize, usize>,
) {
    g.boot_time = cf.boot_time;
    for e in cf.entries {
        let stream = match mud_sys::adopt_stream(e.fd, &e.blob) {
            Ok(s) => s,
            Err(err) => {
                g.log(format!("SYSERR: copyover adopt: {}", err));
                continue;
            }
        };
        if stream.set_nonblocking(true).is_err() {
            continue;
        }
        let mut stream = mio::net::TcpStream::from_std(stream);
        {
            use std::io::Write;
            if stream.write_all(b"\r\nRestoring from copyover...\r\n").is_err() {
                continue;
            }
        }
        let tok = *next_token;
        *next_token += 1;
        if poll.registry().register(&mut stream, Token(tok), Interest::READABLE).is_err() {
            continue;
        }
        let di = mud_game::run::copyover_attach(g, stream, &e.host, &e.guiopt, &e.name, e.pref);
        match di {
            Some(di) => {
                token_map.insert(tok, di);
            }
            None => {
                token_map.remove(&tok);
            }
        }
    }
}
