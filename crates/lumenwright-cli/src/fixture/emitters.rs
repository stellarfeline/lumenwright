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

/// Emitters beside sunlit blocks in the open: on a smooth stone yard 21 × 21 blocks, a lit
/// furnace at (−3, 65, 0) and an unlit one at (3, 65, 0), whose tops are the same texture;
/// a lantern on a cobblestone wall block at (0, 66, −3), glowstone beside sandstone at
/// (−3, 65, −4) and (−2, 65, −4), and a torch at (3, 65, −3). Noon unless rendered at a
/// stated time.
pub fn daylight_emitters() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    for x in -10..=10 {
        for z in -10..=10 {
            w.set(x, FLOOR_Y, z, bs("smooth_stone"));
        }
    }
    let furnace = |lit: &str| props("furnace", &[("facing", "south"), ("lit", lit)]);
    w.set(-3, FLOOR_Y + 1, 0, furnace("true"));
    w.set(3, FLOOR_Y + 1, 0, furnace("false"));
    w.set(
        0,
        FLOOR_Y + 1,
        -3,
        props(
            "cobblestone_wall",
            &[
                ("up", "true"),
                ("north", "none"),
                ("south", "none"),
                ("east", "none"),
                ("west", "none"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.set(
        0,
        FLOOR_Y + 2,
        -3,
        props("lantern", &[("hanging", "false"), ("waterlogged", "false")]),
    );
    w.set(-3, FLOOR_Y + 1, -4, bs("glowstone"));
    w.set(-2, FLOOR_Y + 1, -4, bs("sandstone"));
    w.set(3, FLOOR_Y + 1, -3, bs("torch"));
    w.player = Some(([0.5, 72.0, 7.5], 180.0, 40.0));
    w.day_time = 6000;
    w
}

/// Where the daylight emitters are seen from: south of the yard and above it, looking down
/// on both furnaces' tops.
pub const DAYLIGHT_EMITTERS_VIEW: [&str; 6] = [
    "--camera",
    "0.5,72,7.5",
    "--look-at",
    "0.5,65,-0.5",
    "--fov",
    "55",
];

/// A closed 9 × 9 room of stone bricks, 5 high, with no opening to the sky, at noon, lit
/// only by lanterns: two hanging from the ceiling at (−2, 68, −2) and (2, 68, 2) and one
/// standing on the floor at (2, 65, −3).
pub fn lantern_room() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let (lo, hi, top) = (-4, 4, FLOOR_Y + 5);
    for x in lo - 1..=hi + 1 {
        for z in lo - 1..=hi + 1 {
            for y in FLOOR_Y..=top {
                let shell = y == FLOOR_Y || y == top || x < lo || x > hi || z < lo || z > hi;
                if shell {
                    w.set(x, y, z, bs("stone_bricks"));
                }
            }
        }
    }
    let lantern =
        |hanging: &str| props("lantern", &[("hanging", hanging), ("waterlogged", "false")]);
    w.set(-2, top - 1, -2, lantern("true"));
    w.set(2, top - 1, 2, lantern("true"));
    w.set(2, FLOOR_Y + 1, -3, lantern("false"));
    w.player = Some(([0.5, 67.0, 3.5], 180.0, 15.0));
    w.day_time = 6000;
    w
}

/// Where the lantern room is seen from: inside its south side, looking across the room.
pub const LANTERN_ROOM_VIEW: [&str; 6] = [
    "--camera",
    "0.5,67,4.2",
    "--look-at",
    "0.5,65.5,-3",
    "--fov",
    "75",
];
