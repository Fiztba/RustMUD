//! The `map` command and the side-by-side automap.
//!
//! The canvas is a fixed 51×51 int grid walked recursively from the player's
//! room. Odd/even column parity carries the door cells in normal mode, which
//! is why the blank pass writes DOOR_NONE into even columns only.
//!
//! `ns_size`/`ew_size` are initialised to 0 and never assigned, so every
//! position argument threaded through MapArea is 0 in practice and the
//! "virtual exit" branch is unreachable. Both are kept as written.

use mud_data::flags;
use mud_data::ids::CharId;
use mud_data::types::*;

use crate::comm::send_to_char;
use crate::game::Game;
use crate::handler::{is_abbrev, rev_dir, room_flagged};
use crate::interpreter::two_arguments;

const CANVAS_HEIGHT: i32 = 19;
const CANVAS_WIDTH: i32 = 51;
const LEGEND_WIDTH: i32 = 15;

const MAX_MAP_SIZE: i32 = (CANVAS_WIDTH - 1) / 4;
const MAX_MAP: i32 = CANVAS_WIDTH;
const MAX_MAP_DIR: usize = 10;
const MAX_MAP_FOLLOW: usize = 10;

const SECT_EMPTY: i32 = 30;
const SECT_STRANGE: i32 = SECT_EMPTY + 1;
const SECT_HERE: i32 = SECT_STRANGE + 1;

const DOOR_NS: i32 = -1;
const DOOR_EW: i32 = -2;
const DOOR_UP: i32 = -3;
const DOOR_DOWN: i32 = -4;
const DOOR_DIAGNE: i32 = -5;
const DOOR_DIAGNW: i32 = -6;
const VDOOR_NS: i32 = -7;
const VDOOR_EW: i32 = -8;
const DOOR_UP_AND_NE: i32 = -11;
const DOOR_DOWN_AND_SE: i32 = -12;
const DOOR_NONE: i32 = -13;
const NUM_DOOR_TYPES: i32 = 13;

const MAP_CIRCLE: i32 = 0;
const MAP_RECTANGLE: i32 = 1;
const MAP_NORMAL: i32 = 0;
const MAP_COMPACT: i32 = 1;

// cedit's map_option.
pub const MAP_OFF: i32 = 0;
pub const MAP_ON: i32 = 1;
pub const MAP_IMM_ONLY: i32 = 2;

/// door_info[], indexed by `NUM_DOOR_TYPES + mark`.
const DOOR_INFO: [&[u8]; 13] = [
    b"   ",           // DOOR_NONE   (-13)
    b"\tr-\tn\\ ",    // DOOR_DOWN_AND_SE (-12)
    b"\tr+\tn/ ",     // DOOR_UP_AND_NE   (-11)
    b" \tm+\tn ",     // VDOOR_DIAGNW (-10)
    b" \tm+\tn ",     // VDOOR_DIAGNE (-9)
    b" \tm+\tn ",     // VDOOR_EW     (-8)
    b" \tm+\tn ",     // VDOOR_NS     (-7)
    b" \\ ",          // DOOR_DIAGNW  (-6)
    b" / ",           // DOOR_DIAGNE  (-5)
    b"\tr-\tn  ",     // DOOR_DOWN    (-4)
    b"\tr+\tn  ",     // DOOR_UP      (-3)
    b" - ",           // DOOR_EW      (-2)
    b" | ",           // DOOR_NS      (-1)
];

/// compact_door_info[]. Note VDOOR_EW/NS and DOOR_NS
/// keep their three-character forms here — a table asymmetry, preserved.
const COMPACT_DOOR_INFO: [&[u8]; 13] = [
    b" ",
    b"\tR\\\tn",
    b"\tR/\tn",
    b"\tm+\tn",
    b"\tm+\tn",
    b" \tm+\tn ",
    b" \tm+\tn ",
    b"\\",
    b"/",
    b"\tr-\tn",
    b"\tr+\tn",
    b"-",
    b" | ",
];

fn map_disp(sect: i32) -> &'static [u8] {
    match sect {
        flags::SECT_INSIDE => b"\tc[\tn.\tc]\tn",
        flags::SECT_CITY => b"\tc[\twC\tc]\tn",
        flags::SECT_FIELD => b"\tc[\tg,\tc]\tn",
        flags::SECT_FOREST => b"\tc[\tgY\tc]\tn",
        flags::SECT_HILLS => b"\tc[\tMm\tc]\tn",
        flags::SECT_MOUNTAIN => b"\tc[\trM\tc]\tn",
        flags::SECT_WATER_SWIM => b"\tc[\tc~\tc]\tn",
        flags::SECT_WATER_NOSWIM => b"\tc[\tb=\tc]\tn",
        flags::SECT_FLYING => b"\tc[\tC^\tc]\tn",
        flags::SECT_UNDERWATER => b"\tc[\tbU\tc]\tn",
        SECT_EMPTY => b"   ",
        SECT_STRANGE => b"\tc[\tR?\tc]\tn",
        SECT_HERE => b"\tc[\tB!\tc]\tn",
        _ => b"",
    }
}

fn world_map_disp(sect: i32) -> &'static [u8] {
    match sect {
        flags::SECT_INSIDE => b"\tn.",
        flags::SECT_CITY => b"\twC",
        flags::SECT_FIELD => b"\tg,",
        flags::SECT_FOREST => b"\tgY",
        flags::SECT_HILLS => b"\tMm",
        flags::SECT_MOUNTAIN => b"\trM",
        flags::SECT_WATER_SWIM => b"\tc~",
        flags::SECT_WATER_NOSWIM => b"\tb=",
        flags::SECT_FLYING => b"\tC^",
        flags::SECT_UNDERWATER => b"\tbU",
        SECT_EMPTY => b" ",
        SECT_STRANGE => b"\tR?",
        SECT_HERE => b"\tB!",
        _ => b"",
    }
}

const OFFSETS: [[i32; 2]; 10] =
    [[-2, 0], [0, 2], [2, 0], [0, -2], [0, 0], [0, 0], [-2, -2], [-2, 2], [2, 2], [2, -2]];
const OFFSETS_WORLDMAP: [[i32; 2]; 10] =
    [[-1, 0], [0, 1], [1, 0], [0, -1], [0, 0], [0, 0], [-1, -1], [-1, 1], [1, 1], [1, -1]];
const DOOR_OFFSETS: [[i32; 2]; 10] =
    [[-1, 0], [0, 1], [1, 0], [0, -1], [-1, 1], [1, 1], [-1, -1], [-1, 1], [1, 1], [1, -1]];
const DOOR_MARKS: [i32; 10] = [
    DOOR_NS,
    DOOR_EW,
    DOOR_NS,
    DOOR_EW,
    DOOR_UP,
    DOOR_DOWN,
    DOOR_DIAGNW,
    DOOR_DIAGNE,
    DOOR_DIAGNW,
    DOOR_DIAGNE,
];
const VDOOR_MARKS: [i32; 4] = [VDOOR_NS, VDOOR_EW, VDOOR_NS, VDOOR_EW];

type Canvas = Vec<Vec<i32>>;

fn blank_canvas(worldmap: bool) -> Canvas {
    (0..MAX_MAP as usize)
        .map(|_| {
            (0..MAX_MAP as usize)
                .map(|y| if y % 2 == 0 && !worldmap { DOOR_NONE } else { SECT_EMPTY })
                .collect()
        })
        .collect()
}

fn at(map: &Canvas, x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= MAX_MAP || y >= MAX_MAP {
        SECT_EMPTY
    } else {
        map[x as usize][y as usize]
    }
}

fn put(map: &mut Canvas, x: i32, y: i32, v: i32) {
    if x >= 0 && y >= 0 && x < MAX_MAP && y < MAX_MAP {
        map[x as usize][y as usize] = v;
    }
}

pub fn can_see_map(g: &Game, chid: CharId) -> bool {
    match g.config.map_option {
        MAP_OFF => false,
        MAP_IMM_ONLY => g.ch(chid).level >= LVL_IMMORT,
        _ => true,
    }
}

pub fn show_worldmap(g: &Game, chid: CharId) -> bool {
    let rm = g.ch(chid).in_room;
    if rm == NOWHERE {
        return false;
    }
    if room_flagged(g, rm, flags::ROOM_WORLDMAP) {
        return true;
    }
    let zn = g.world.rooms[rm as usize].zone as usize;
    g.world.zones[zn].zone_flags[flags::ZONE_WORLDMAP / 32] & (1 << (flags::ZONE_WORLDMAP % 32))
        != 0
}

/// MapArea.
#[allow(clippy::too_many_arguments)]
fn map_area(
    g: &Game,
    map: &mut Canvas,
    room: RoomRnum,
    chid: CharId,
    x: i32,
    y: i32,
    min: i32,
    max: i32,
    xpos: i32,
    ypos: i32,
    worldmap: bool,
) {
    if at(map, x, y) < 0 {
        return; // this is a door
    }
    if room == g.ch(chid).in_room {
        put(map, x, y, SECT_HERE);
    } else {
        put(map, x, y, g.world.rooms[room as usize].sector_type);
    }
    if x < min || y < min || x > max || y > max {
        return;
    }

    // ns_size / ew_size / x_exit_pos / y_exit_pos are all 0 — see the
    // module docs.
    let (ns_size, ew_size, x_exit_pos, y_exit_pos) = (0i32, 0i32, 0i32, 0i32);
    let holylight = g.ch(chid).prf(flags::PRF_HOLYLIGHT);

    for door in 0..MAX_MAP_DIR {
        let (dx, dy) = (DOOR_OFFSETS[door][0], DOOR_OFFSETS[door][1]);
        if door < MAX_MAP_FOLLOW
            && xpos + dx >= 0
            && xpos + dx <= ns_size
            && ypos + dy >= 0
            && ypos + dy <= ew_size
        {
            // Virtual exit (unreachable while the sizes are zero).
            put(map, x + dx, y + dy, VDOOR_MARKS[door.min(3)]);
            if at(map, x + OFFSETS[door][0], y + OFFSETS[door][1]) == SECT_EMPTY {
                map_area(
                    g,
                    map,
                    room,
                    chid,
                    x + OFFSETS[door][0],
                    y + OFFSETS[door][1],
                    min,
                    max,
                    xpos + dx,
                    ypos + dy,
                    worldmap,
                );
            }
            continue;
        }

        let Some(pexit) = g.world.rooms[room as usize].dir_option[door].as_ref() else { continue };
        if pexit.to_room == 0 || pexit.to_room == NOWHERE {
            continue;
        }
        if pexit.exit_info & flags::EX_CLOSED != 0 {
            continue;
        }
        if pexit.exit_info & flags::EX_HIDDEN != 0 && !holylight {
            continue;
        }

        // "But is the door here...".
        let skip = match door {
            0 => xpos > 0 || ypos != y_exit_pos,                      // NORTH
            2 => xpos < ns_size || ypos != y_exit_pos,                // SOUTH
            1 => ypos < ew_size || xpos != x_exit_pos,                // EAST
            3 => ypos > 0 || xpos != x_exit_pos,                      // WEST
            6 => xpos > 0 || ypos != y_exit_pos || ypos > 0 || xpos != x_exit_pos, // NW
            7 => {
                xpos > 0 || ypos != y_exit_pos || ypos < ew_size || xpos != x_exit_pos
            } // NE
            8 => {
                xpos < ns_size || ypos != y_exit_pos || ypos < ew_size || xpos != x_exit_pos
            } // SE
            9 => xpos < ns_size || ypos != y_exit_pos || ypos > 0 || xpos != x_exit_pos, // SW
            _ => false,
        };
        if skip {
            continue;
        }

        let prospect_room = pexit.to_room;
        let back = &g.world.rooms[prospect_room as usize].dir_option[rev_dir(door)];
        // One way into area OR maze.
        if let Some(b) = back {
            if b.to_room != room {
                put(map, x, y, SECT_STRANGE);
                return;
            }
        }

        if !worldmap {
            let cur = at(map, x + dx, y + dy);
            if cur == DOOR_NONE || cur == SECT_EMPTY {
                put(map, x + dx, y + dy, DOOR_MARKS[door]);
            } else if (door == 7 && cur == DOOR_UP) || (door == 4 && cur == DOOR_DIAGNE) {
                put(map, x + dx, y + dy, DOOR_UP_AND_NE);
            } else if (door == 8 && cur == DOOR_DOWN) || (door == 5 && cur == DOOR_DIAGNW) {
                put(map, x + dx, y + dy, DOOR_DOWN_AND_SE);
            }
        }

        // prospect_xpos/ypos, with the deliberate switch fallthrough.
        let (mut px, mut py) = (0i32, 0i32);
        let back_exists = back.is_some();
        match door {
            0 => {
                px = ns_size;
                py = if back_exists { y_exit_pos } else { ew_size / 2 };
            }
            2 => py = if back_exists { y_exit_pos } else { ew_size / 2 },
            3 => {
                py = ew_size;
                px = if back_exists { x_exit_pos } else { ns_size / 2 };
            }
            1 => px = if back_exists { x_exit_pos } else { ns_size / 2 },
            6..=9 => {
                px = if back_exists { x_exit_pos } else { ns_size / 2 };
                py = if back_exists { y_exit_pos } else { ew_size / 2 };
            }
            _ => {}
        }

        let off = if worldmap { OFFSETS_WORLDMAP } else { OFFSETS };
        if door < MAX_MAP_FOLLOW
            && at(map, x + off[door][0], y + off[door][1]) == SECT_EMPTY
        {
            map_area(
                g,
                map,
                prospect_room,
                chid,
                x + off[door][0],
                y + off[door][1],
                min,
                max,
                px,
                py,
                worldmap,
            );
        }
    }
}

fn door_disp(mark: i32, compact: bool) -> &'static [u8] {
    let idx = (NUM_DOOR_TYPES + mark).clamp(0, 12) as usize;
    if compact {
        COMPACT_DOOR_INFO[idx]
    } else {
        DOOR_INFO[idx]
    }
}

fn string_map(map: &Canvas, centre: i32, size: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for x in (centre - CANVAS_HEIGHT / 2)..=(centre + CANVAS_HEIGHT / 2) {
        for y in (centre - CANVAS_WIDTH / 6)..=(centre + CANVAS_WIDTH / 6) {
            let cell = at(map, x, y);
            let tmp: &[u8] = if (centre - x).abs() <= size && (centre - y).abs() <= size {
                if cell < 0 {
                    door_disp(cell, false)
                } else {
                    map_disp(cell)
                }
            } else {
                map_disp(SECT_EMPTY)
            };
            out.extend_from_slice(tmp);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn world_map(map: &Canvas, centre: i32, size: i32, mapshape: i32, maptype: i32) -> Vec<u8> {
    let (xmin, xmax, ymin, ymax) = if maptype == MAP_COMPACT {
        (centre - size, centre + size, centre - 2 * size, centre + 2 * size)
    } else {
        (
            centre - CANVAS_HEIGHT / 2,
            centre + CANVAS_HEIGHT / 2,
            centre - CANVAS_WIDTH / 2,
            centre + CANVAS_WIDTH / 2,
        )
    };
    let mut out = Vec::new();
    for x in xmin..=xmax {
        for y in ymin..=ymax {
            let inside = (mapshape == MAP_RECTANGLE
                && (centre - y).abs() <= size * 2
                && (centre - x).abs() <= size)
                || (mapshape == MAP_CIRCLE
                    && (centre - x) * (centre - x) + (centre - y) * (centre - y) / 4
                        <= size * size + 1);
            if inside {
                out.extend_from_slice(world_map_disp(at(map, x, y)));
            } else {
                out.push(b' ');
            }
        }
        out.extend_from_slice(b"\tn\r\n");
    }
    out
}

fn compact_string_map(map: &Canvas, centre: i32, size: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for x in (centre - size)..=(centre + size) {
        for y in (centre - size)..=(centre + size) {
            let cell = at(map, x, y);
            out.extend_from_slice(if cell < 0 { door_disp(cell, true) } else { map_disp(cell) });
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn perform_map(g: &mut Game, chid: CharId, argument: &[u8], mut worldmap: bool) {
    let mut size = g.config.default_map_size;
    let mut mapshape = MAP_CIRCLE;
    let (arg1, arg2, _) = two_arguments(argument);
    if !arg1.is_empty() {
        size = crate::handler::atoi(&arg1);
    }
    if !arg2.is_empty() {
        if is_abbrev(&arg2, b"normal") {
            worldmap = false;
        } else if is_abbrev(&arg2, b"world") {
            worldmap = true;
        } else {
            // No CRLF here.
            send_to_char(g, chid, b"Usage: \tymap <distance> [ normal | world ]\tn");
            return;
        }
    }
    if size < 0 {
        size = -size;
        mapshape = MAP_RECTANGLE;
    }
    size = size.clamp(1, MAX_MAP_SIZE);

    let centre = MAX_MAP / 2;
    let (min, max) = if worldmap {
        (centre - 2 * size, centre + 2 * size)
    } else {
        (centre - size, centre + size)
    };

    let mut map = blank_canvas(worldmap);
    let room = g.ch(chid).in_room;
    map_area(g, &mut map, room, chid, centre, centre, min, max, 0, 0, worldmap);
    put(&mut map, centre, centre, SECT_HERE);

    send_to_char(
        g,
        chid,
        b" \tY-\tytbaMUD Map System\tY-\tn\r\n\tD  .-.__--.,--.__.-.\tn\r\n",
    );

    let mut legend = Vec::new();
    let entry = |legend: &mut Vec<u8>, disp: &[u8], label: &[u8], triple: bool| {
        legend.extend_from_slice(if triple { &b"\tn\tn\tn"[..] } else { &b"\tn"[..] });
        legend.extend_from_slice(disp);
        legend.push(b' ');
        legend.extend_from_slice(label);
        legend.extend_from_slice(b"\\\\");
    };
    entry(&mut legend, door_disp(DOOR_UP, false), b"Up", true);
    entry(&mut legend, door_disp(DOOR_DOWN, false), b"Down", true);
    entry(&mut legend, map_disp(SECT_HERE), b"You", false);
    entry(&mut legend, map_disp(flags::SECT_INSIDE), b"Inside", false);
    entry(&mut legend, map_disp(flags::SECT_CITY), b"City", false);
    entry(&mut legend, map_disp(flags::SECT_FIELD), b"Field", false);
    entry(&mut legend, map_disp(flags::SECT_FOREST), b"Forest", false);
    entry(&mut legend, map_disp(flags::SECT_HILLS), b"Hills", false);
    entry(&mut legend, map_disp(flags::SECT_MOUNTAIN), b"Mountain", false);
    entry(&mut legend, map_disp(flags::SECT_WATER_SWIM), b"Swim", false);
    entry(&mut legend, map_disp(flags::SECT_WATER_NOSWIM), b"Boat", false);
    entry(&mut legend, map_disp(flags::SECT_FLYING), b"Flying", false);
    entry(&mut legend, map_disp(flags::SECT_UNDERWATER), b"Underwater", false);

    let legend = crate::text::strfrmt(&legend, LEGEND_WIDTH, CANVAS_HEIGHT + 2, false, true, true);
    let blank_col = crate::text::strfrmt(b"", 0, CANVAS_HEIGHT + 2, false, false, true);
    let mut buf2 = crate::text::strpaste(&blank_col, &legend, b"\tD | \tn");

    // The map column: a blank first row, the map, a blank last row.
    let mut canvas = vec![b' '; CANVAS_WIDTH as usize];
    canvas.extend_from_slice(b"\r\n");
    canvas.extend_from_slice(&if worldmap {
        world_map(&map, centre, size, mapshape, MAP_NORMAL)
    } else {
        string_map(&map, centre, size)
    });
    canvas.extend(std::iter::repeat(b' ').take(CANVAS_WIDTH as usize));
    canvas.extend_from_slice(b"\r\n");

    buf2 = crate::text::strpaste(&buf2, &canvas, b"\tD | \tn");
    buf2 = crate::text::strpaste(&buf2, &blank_col, b"  ");
    send_to_char(g, chid, &buf2);
    send_to_char(g, chid, b"\tD `.-.__--.,-.__.-.-'\tn\r\n");
}

/// str_and_map: the room description with the minimap
/// pasted beside it.
pub fn str_and_map(g: &mut Game, chid: CharId, str_: &[u8], target_room: RoomRnum) {
    let width = g.ch(chid).ps().screen_width;
    if !can_see_map(g, chid) {
        let s = crate::text::strfrmt(str_, width, 1, false, false, false);
        send_to_char(g, chid, &s);
        return;
    }
    let worldmap = show_worldmap(g, chid);
    if !g.ch(chid).prf(flags::PRF_AUTOMAP) {
        let s = crate::text::strfrmt(str_, width, 1, false, false, false);
        send_to_char(g, chid, &s);
        return;
    }

    let size = g.config.default_minimap_size;
    let centre = MAX_MAP / 2;
    let (min, max) = (centre - 2 * size, centre + 2 * size);
    let mut map = blank_canvas(worldmap);
    map_area(g, &mut map, target_room, chid, centre, centre, min, max, 0, 0, worldmap);
    put(&mut map, centre, centre, SECT_HERE);

    // char_size = rooms + doors + padding
    let char_size = if worldmap { size * 4 + 5 } else { 3 * (size + 1) + size + 4 };
    let left = crate::text::strfrmt(str_, width - char_size, size * 2 + 1, false, true, true);
    let right = if worldmap {
        world_map(&map, centre, size, MAP_CIRCLE, MAP_COMPACT)
    } else {
        compact_string_map(&map, centre, size)
    };
    let out = crate::text::strpaste(&left, &right, b" \tn");
    send_to_char(g, chid, &out);
}

pub fn do_map(g: &mut Game, chid: CharId, argument: &[u8], _cmd: usize, _subcmd: i32) {
    if !can_see_map(g, chid) {
        send_to_char(g, chid, b"Sorry, the map is disabled!\r\n");
        return;
    }
    let room = g.ch(chid).in_room;
    if crate::handler::room_is_dark(g, room) && !crate::handler::can_see_in_dark(g, chid) {
        send_to_char(g, chid, b"It is too dark to see the map.\r\n");
        return;
    }
    if g.ch(chid).aff(flags::AFF_BLIND) && g.ch(chid).level < LVL_IMMORT {
        send_to_char(g, chid, b"You can't see the map while blind!\r\n");
        return;
    }
    let worldmap = show_worldmap(g, chid);
    perform_map(g, chid, argument, worldmap);
}
