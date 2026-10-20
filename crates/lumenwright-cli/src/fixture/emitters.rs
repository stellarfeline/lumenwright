//! Lamp-lit rooms for HQ emitter lighting: a single torch on a long floor (the
//! calibration against the game's block light) and a candle-lit room.

use super::{FixtureWorld, bs, props};

/// Floor height of both rooms: the floor blocks are at `FLOOR_Y`, the air above at `FLOOR_Y + 1`.
pub const FLOOR_Y: i32 = 64;

/// A closed room of white concrete, 41 × 21 blocks inside and 8 high, dark but for
/// one torch standing on the floor at (0, 65, 0). The floor at x = d (z = 0) is
/// lit to block light 14 − d in the game.
pub fn torch_room() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let (x0, x1, z0, z1, top) = (-20, 20, -10, 10, FLOOR_Y + 8);
    for x in x0 - 1..=x1 + 1 {
        for z in z0 - 1..=z1 + 1 {
            for y in FLOOR_Y..=top {
                let shell = y == FLOOR_Y || y == top || x < x0 || x > x1 || z < z0 || z > z1;
                if shell {
                    w.set(x, y, z, bs("white_concrete"));
                }
            }
        }
    }
    w.set(0, FLOOR_Y + 1, 0, bs("torch"));
    w.player = Some(([-4.0, 70.5, 0.5], -90.0, 20.0));
    w.day_time = 18000;
    w
}

/// Where the torch room is seen from: down the +x line of floor the torch lights.
pub const TORCH_ROOM_VIEW: [&str; 6] = [
    "--camera",
    "-4.5,70.5,0.5",
    "--look-at",
    "9,65,0.5",
    "--fov",
    "70",
];

/// A closed 9 × 9 room of oak planks, 5 high, at midnight, lit only by lit candles:
/// a cluster of four on a table in the middle and single candles in two corners.
pub fn candle_room() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let (lo, hi, top) = (-4, 4, FLOOR_Y + 5);
    for x in lo - 1..=hi + 1 {
        for z in lo - 1..=hi + 1 {
            for y in FLOOR_Y..=top {
                let shell = y == FLOOR_Y || y == top || x < lo || x > hi || z < lo || z > hi;
                if shell {
                    let block = if y == FLOOR_Y {
                        "spruce_planks"
                    } else {
                        "oak_planks"
                    };
                    w.set(x, y, z, bs(block));
                }
            }
        }
    }
    let candle = |n: &str, colour: &str| {
        props(
            &format!("{colour}candle"),
            &[("candles", n), ("lit", "true"), ("waterlogged", "false")],
        )
    };
    w.set(0, FLOOR_Y + 1, 0, bs("oak_planks"));
    w.set(0, FLOOR_Y + 2, 0, candle("4", ""));
    w.set(-3, FLOOR_Y + 1, -3, candle("1", "white_"));
    w.set(3, FLOOR_Y + 1, -3, candle("2", "red_"));
    w.set(3, FLOOR_Y + 1, 2, props("bookshelf", &[]));
    w.set(
        -3,
        FLOOR_Y + 1,
        2,
        props("barrel", &[("facing", "up"), ("open", "false")]),
    );
    w.player = Some(([0.5, 68.0, 3.5], 180.0, 25.0));
    w.day_time = 18000;
    w
}

/// Where the candle room is seen from: its south side, looking across the table.
pub const CANDLE_ROOM_VIEW: [&str; 6] = [
    "--camera",
    "0.5,68.2,4.2",
    "--look-at",
    "0.5,65.5,-2",
    "--fov",
    "75",
];
