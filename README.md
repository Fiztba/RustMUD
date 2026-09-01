# RustMUD

A complete rewrite of [tbaMUD](https://github.com/tbamud/tbamud) in Rust. What a
player sees is the same game: every command, every admin command, all of OLC, DG
Scripts, and the on-disk file formats, down to the byte where a byte is
observable. Genuine bugs in the original are fixed here and sent upstream rather
than reproduced.

If you have run tbaMUD, you already know how to run this.

---

## Building

You need a Rust toolchain, 1.85 or newer (the tree is edition 2024). Get one
from [rustup.rs](https://rustup.rs) on either platform.

### Linux

```
git clone https://github.com/Fiztba/RustMUD
cd RustMUD
cargo build --release
```

The server lands at `target/release/circle`.

### Windows

The same, in PowerShell or a terminal of your choice:

```
git clone https://github.com/Fiztba/RustMUD
cd RustMUD
cargo build --release
```

The server lands at `target\release\circle.exe`. Nothing else is needed — no
Visual Studio workload beyond the C++ build tools rustup itself asks for, no
Cygwin, no WSL. Windows is a first-class target here rather than an
afterthought; `copyover` works on it, which it does not in the C.

## Starting it

### Linux

```
./target/release/circle -q -d lib 4000
```

### Windows

```
.\target\release\circle.exe -q -d lib 4000
```

Then point a MUD client, or plain `telnet`, at `localhost 4000`.

Port defaults to 4000 and the data directory to `lib`, so a bare `circle` does
the same thing. The server prints its boot to standard error and finishes with
`Entering game loop.` — that line, not a successful connection, is the honest
readiness signal, because a connection made before it is a descriptor the game
counts.

| flag | effect |
|---|---|
| `-d <dir>` | data directory (default `lib`) |
| `-o <file>` | write the log to a file instead of stderr |
| `-q` | quick boot: skip the rent check |
| `-m` | minimized: read `text/help/index.mini`, stay quiet about missing castle mobs, no rent check |
| `-c` | syntax-check the world files and exit |
| `-r` | restrict the game: no new players |
| `-s` | suppress special procedures |
| `--help` | the full list |

`-c` is worth knowing: it boots far enough to parse every world file and then
exits, so it is the cheap way to find out whether a builder's export or a
hand-edited zone will come up before you take the live game down for it.

`-m` does not load a smaller world; it loads a smaller *help* file and stops
complaining about a `lib/` that has no King's Castle. It is for running against
a cut-down data directory, not for making the stock one lighter.

## The first character is the implementor

On a data directory with no players, **whoever creates the first character
becomes the implementor** — level 34, and every skill and command with it.
There is no console, no separate admin tool, and no way to grant that level
from outside the game. So the order on a fresh install is: start the server,
connect yourself, make your character, and only then tell anyone else the
address.

If someone else gets there first, the fix is to stop the server, delete their
`.plr` file and the matching `lib/plrfiles/index` entry, and start again.

## What a running game consists of

Two things: the binary, and the data directory. `lib/` **is** the game — the
world, the text files, the configuration and every player who has ever logged
in:

| `lib/` | holds |
|---|---|
| `world/` | the zones: `.wld` `.mob` `.obj` `.zon` `.shp` `.trg` `.qst` |
| `text/` | greeting, motd, imotd, help, news, credits, policies, handbook |
| `etc/` | `config` (see below), `time`, the ban and invalid-name lists |
| `plrfiles/` | one `.plr` per player, plus `index` |
| `plrobjs/` | rent files — what each player was carrying |
| `plrvars/` | per-player DG script variables |
| `house/` | player house contents |
| `misc/` | boards, mail, quests, ideas/bugs/typos |

The binary is disposable; `lib/` is not.

## Stopping and restarting

All five are in-game commands, and none of them is `kill`. Every one of them
runs the same orderly sequence — rent files written, houses written, sockets
closed, mud time saved — so none of them loses a player:

| command | file touched | exit | also |
|---|---|---|---|
| `shutdown` | — | 0 | |
| `shutdown die` | `.killscript` | 0 | |
| `shutdown pause` | `pause` | 0 | |
| `shutdown reboot` | `.fastboot` | 52 | skips the OLC pending-save flush |
| `shutdown now` | — | 52 | skips the OLC pending-save flush |

The one thing worth knowing before you type it: **`reboot` and `now` skip
`save_all`**, the queue of world changes builders have made and not yet written
out. Rent, houses and player files are saved either way — it is zone, mob,
object and shop edits sitting in the pending list that go. If builders are
working, give them a minute to `save` first, or use plain `shutdown`.

`now` is not more abrupt than `reboot`; the two differ only in that `reboot`
leaves `.fastboot` behind.

The files are signals for a supervising script and land **beside** the data
directory — `.fastboot` next to `lib/`, not inside it. Exit code **52** is the
same signal in the other channel: it means "start me again", and 0 means "stay
down".

**No autorun script ships with this.** If you want the game to come back by
itself, a loop that restarts the binary while it exits 52, stops when it sees
`.killscript`, and waits while `pause` exists is the whole of it. Delete
`.fastboot` after acting on it, or the next restart reads it again.

## Copyover

`copyover` restarts the binary underneath the players without dropping them —
what it is for is picking up a new build mid-session. Sockets are handed to the
new process through `copyover.dat`, which sits beside the data directory rather
than inside it, and everyone sees a brief pause rather than a disconnect. It
works on Windows as well as Unix.

It is the one restart that keeps players connected, and also the one that will
strand them if the new binary does not start. Have the build in hand and
`-c`-checked before running it.

## Configuration

Two layers, and the second wins:

1. `lib/etc/config` — read at boot. Lines are `TAG = value`.
2. `cedit` in-game, which edits the same settings and saves them back to that
   file, so a change made there survives a reboot.

Anything not in the file takes a built-in default, and a missing file is not an
error — the server says so and carries on. Stock ships without one.

`DFLT_IP` is worth calling out: set it and the game binds only that address, so
`DFLT_IP = 127.0.0.1` keeps it on the loopback while you set things up. Leave it
unset and it listens on every interface.

## Logs

By default the log goes to standard error; `-o <file>` sends it to a file
instead. It is the only place a good deal of trouble is reported: script errors,
world-file complaints at boot, and every `SYSERR`. If you run the game
unattended, capture it.

`SYSERR` is worth grepping for on its own. It means the server found something
it was not willing to guess about, and most of them name the zone, room or
trigger at fault.

## What to back up

`lib/` — all of it, and while the server is down or at least quiet. The parts
that cannot be reconstructed are `plrfiles/`, `plrobjs/`, `plrvars/`, `house/`
and `misc/`: those are the players. `world/` and `text/` are recoverable from
your builders' sources if you keep them elsewhere, and `etc/config` is small but
fiddly to redo by hand.

Backing up a running game is safe to *attempt* — nothing is corrupted by being
read — but a player who saves during the copy can leave you a file from one
moment and an index from another, so a quiet moment is worth waiting for.

## Maintenance modes

Two things ship in the binary rather than as separate programs:

```
circle --rebuild-index
```

rebuilds `lib/plrfiles/index` from the `.plr` files and exits. The index is a
derived file: losing it does not lose a single player, but nobody can log in
until it is rebuilt, which makes this the answer to a login failure that looks
catastrophic and is not.

Three things in the index are *not* derivable from the player files, and the
command says so as it runs: a player marked `DELETED`, one marked `SELFDELETE`,
and an immortal marked `NOWIZLIST`. A rebuild brings all three back as ordinary
entries. If you had any set, re-apply them by editing the index by hand
afterwards.

```
circle --import-binary-pfiles <file> [--endian little|big] [--dry-run]
```

converts a pre-3.x CircleMUD binary player file to the ASCII pfiles this server
uses. It assumes a 32-bit CircleMUD 3.0 layout and refuses rather than guessing
if the file does not fit that shape. Run it with `--dry-run` first; it reports
what it would write and touches nothing.

## Ports

The game listens on one TCP port and speaks telnet. Nothing else is exposed —
there is no HTTP side, no admin socket, no second listener. If you are
forwarding a port from a home router, that one port is all that needs to go
through, and the server does not need to know its public address.

## Licence

See [LICENSE](LICENSE). RustMUD carries **tbaMUD's** licence, which is the 1995
CircleMUD licence, and follows tbaMUD if and when that changes.

Worth knowing where those terms come from, because the text is stricter than it
looks and stricter than its own authors now intend. DikuMUD relicensed to LGPL
in 2020 and CircleMUD followed; the restrictions in the file below — no
charging, mandatory credit — are no longer in force *upstream of tbaMUD*.
tbaMUD has not made that change yet, and RustMUD deliberately does not get ahead
of it. So the terms here are tbaMUD's, by choice rather than necessity.
