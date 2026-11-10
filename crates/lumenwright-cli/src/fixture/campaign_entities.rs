//! The entity gallery: one of every entity body, layer and variant the models drawn
//! from 1.21.11's client classes add, for the tests that check each is drawn.

use fastnbt::Value;

use super::{FixtureWorld, bs, compound, s};

/// One body of the gallery.
pub struct EntityCase {
    /// What the case shows, as the tests name it.
    pub name: &'static str,
    /// Entity id without the namespace.
    pub id: &'static str,
    /// Feet position.
    pub pos: [f64; 3],
    /// Yaw in degrees.
    pub yaw: f32,
    /// A point on the body, for checking that something is drawn there.
    pub spot: [f64; 3],
    /// The entity's extra NBT.
    pub extra: fn() -> Vec<(&'static str, Value)>,
}

fn none() -> Vec<(&'static str, Value)> {
    Vec::new()
}

fn item(id: &str) -> Value {
    compound(vec![
        ("id", s(&format!("minecraft:{id}"))),
        ("count", Value::Int(1)),
    ])
}

/// The bodies of [`entity_gallery`].
pub const ENTITY_CASES: &[EntityCase] = &[
    EntityCase {
        name: "happy_ghast",
        id: "happy_ghast",
        pos: [3.0, 65.0, 2.0],
        yaw: 0.0,
        spot: [3.0, 67.0, 4.0],
        extra: none,
    },
    EntityCase {
        name: "happy_ghast (harness)",
        id: "happy_ghast",
        pos: [9.0, 65.0, 2.0],
        yaw: 0.0,
        spot: [9.0, 67.0, 4.0],
        extra: || vec![("equipment", compound(vec![("body", item("red_harness"))]))],
    },
    EntityCase {
        name: "happy_ghast (baby)",
        id: "happy_ghast",
        pos: [6.0, 65.0, 5.5],
        yaw: 0.0,
        spot: [6.0, 65.5, 6.0],
        extra: || vec![("Age", Value::Int(-24000))],
    },
    EntityCase {
        name: "wither",
        id: "wither",
        pos: [18.5, 65.0, 2.5],
        yaw: 0.0,
        spot: [18.5, 68.5, 2.5],
        extra: none,
    },
    EntityCase {
        name: "ravager",
        id: "ravager",
        pos: [22.5, 65.0, 2.5],
        yaw: 0.0,
        spot: [22.5, 66.5, 2.5],
        extra: none,
    },
    EntityCase {
        name: "camel (saddle)",
        id: "camel",
        pos: [27.5, 65.0, 2.5],
        yaw: 0.0,
        spot: [27.5, 66.8, 2.5],
        extra: || vec![("equipment", compound(vec![("saddle", item("saddle"))]))],
    },
    EntityCase {
        name: "sheep (white)",
        id: "sheep",
        pos: [1.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [1.5, 65.9, 9.5],
        extra: none,
    },
    EntityCase {
        name: "sheep (red)",
        id: "sheep",
        pos: [3.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [3.5, 65.9, 9.5],
        extra: || vec![("Color", Value::Byte(14))],
    },
    EntityCase {
        name: "sheep (red, sheared)",
        id: "sheep",
        pos: [5.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [5.5, 65.8, 9.5],
        extra: || vec![("Color", Value::Byte(14)), ("Sheared", Value::Byte(1))],
    },
    EntityCase {
        name: "sheep (blue, baby)",
        id: "sheep",
        pos: [7.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [7.5, 65.4, 9.5],
        extra: || vec![("Color", Value::Byte(11)), ("Age", Value::Int(-24000))],
    },
    EntityCase {
        name: "spider (eyes)",
        id: "spider",
        pos: [10.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [10.5, 65.5, 9.0],
        extra: none,
    },
    EntityCase {
        name: "enderman (eyes)",
        id: "enderman",
        pos: [13.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [13.5, 67.5, 9.5],
        extra: none,
    },
    EntityCase {
        name: "warden",
        id: "warden",
        pos: [15.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [15.5, 67.0, 9.5],
        extra: none,
    },
    EntityCase {
        name: "firework_rocket",
        id: "firework_rocket",
        pos: [18.5, 66.5, 9.5],
        yaw: 0.0,
        spot: [18.5, 66.5, 9.5],
        extra: none,
    },
    EntityCase {
        name: "allay",
        id: "allay",
        pos: [20.5, 65.5, 9.5],
        yaw: 0.0,
        spot: [20.5, 65.8, 9.5],
        extra: none,
    },
    EntityCase {
        name: "parrot (green)",
        id: "parrot",
        pos: [22.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [22.5, 65.4, 9.5],
        extra: || vec![("Variant", Value::Int(2))],
    },
    EntityCase {
        name: "axolotl (gold)",
        id: "axolotl",
        pos: [24.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [24.5, 65.2, 9.5],
        extra: || vec![("Variant", Value::Int(2))],
    },
    EntityCase {
        name: "turtle",
        id: "turtle",
        pos: [27.5, 65.0, 9.5],
        yaw: 0.0,
        spot: [27.5, 65.25, 9.5],
        extra: none,
    },
    EntityCase {
        name: "oak_boat",
        id: "oak_boat",
        pos: [1.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [1.5, 65.3, 14.5],
        extra: none,
    },
    EntityCase {
        name: "cherry_chest_boat",
        id: "cherry_chest_boat",
        pos: [4.0, 65.0, 14.5],
        yaw: 90.0,
        spot: [4.0, 65.6, 14.5],
        extra: none,
    },
    EntityCase {
        name: "bamboo_raft",
        id: "bamboo_raft",
        pos: [6.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [6.5, 65.2, 14.5],
        extra: none,
    },
    EntityCase {
        name: "minecart",
        id: "minecart",
        pos: [9.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [9.5, 65.3, 14.5],
        extra: none,
    },
    EntityCase {
        name: "chest_minecart",
        id: "chest_minecart",
        pos: [11.5, 65.0, 14.5],
        yaw: 90.0,
        spot: [11.5, 65.9, 14.5],
        extra: none,
    },
    EntityCase {
        name: "furnace_minecart",
        id: "furnace_minecart",
        pos: [13.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [13.5, 65.8, 14.5],
        extra: none,
    },
    EntityCase {
        name: "hopper_minecart",
        id: "hopper_minecart",
        pos: [15.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [15.5, 65.6, 14.5],
        extra: none,
    },
    EntityCase {
        name: "squid",
        id: "squid",
        pos: [18.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [18.5, 65.6, 14.5],
        extra: none,
    },
    EntityCase {
        name: "glow_squid",
        id: "glow_squid",
        pos: [21.0, 65.0, 14.5],
        yaw: 0.0,
        spot: [21.0, 65.6, 14.5],
        extra: none,
    },
    EntityCase {
        name: "dolphin",
        id: "dolphin",
        pos: [24.5, 65.0, 14.5],
        yaw: 0.0,
        spot: [24.5, 65.3, 14.5],
        extra: none,
    },
];

/// A south-facing view over [`entity_gallery`] that has every spot in frame.
pub const ENTITY_GALLERY_VIEW: [&str; 6] = [
    "--camera",
    "14.5,71.0,25.0",
    "--look-at",
    "14.5,65.5,8.0",
    "--fov",
    "70",
];

/// One of every body in [`ENTITY_CASES`] on a stone platform, all facing south
/// (+Z) unless a case turns them. With `with` false the platform stands empty,
/// which is the same frame without its bodies.
pub fn entity_gallery(with: bool) -> FixtureWorld {
    let mut w = FixtureWorld::new();
    for x in -4..34 {
        for z in -6..28 {
            w.set(x, 63, z, bs("stone"));
            w.set(x, 64, z, bs("smooth_stone"));
        }
    }
    if with {
        for c in ENTITY_CASES {
            w.entity(c.id, c.pos, c.yaw, (c.extra)());
        }
    }
    w.player = Some(([14.5, 71.0, 25.0], 180.0, 18.0));
    w.day_time = 6000;
    w
}

/// Where [`ocean`] is looked at from: on the island's tower, level, out to sea.
pub const OCEAN_VIEW: [&str; 8] = [
    "--camera",
    "0.5,70,0.5",
    "--yaw",
    "0",
    "--pitch",
    "0",
    "--fov",
    "50",
];

/// An ocean filling the 17 × 17 chunks around chunk (0, 0), the area a render distance
/// of 8 loads there: water from y 50 to the sea surface in the block at y 62 (sea level
/// 63), over sand, with a stone island and tower at the centre to stand on.
pub fn ocean() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    w.biome = "minecraft:ocean".into();
    for x in -128..144 {
        for z in -128..144 {
            w.set(x, 49, z, bs("sand"));
            let island = x * x + z * z <= 25;
            for y in 50..=62 {
                w.set(x, y, z, bs(if island { "stone" } else { "water" }));
            }
        }
    }
    for y in 63..69 {
        w.set(0, y, 0, bs("stone_bricks"));
    }
    w.player = Some(([0.5, 70.0, 0.5], 0.0, 0.0));
    w.day_time = 6000;
    w
}
