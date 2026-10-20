//! `level.dat` metadata.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use fastnbt::Value;

use super::nbt::{self, Compound};

#[derive(Clone, Debug, Default)]
/// The single-player position stored in `level.dat`.
pub struct PlayerInfo {
    /// Feet position in world coordinates (blocks).
    pub pos: [f64; 3],
    /// Yaw in degrees, Minecraft convention (0 = south, 90 = west).
    pub yaw: f32,
    /// Pitch in degrees; positive looks down.
    pub pitch: f32,
    /// Namespaced dimension id, e.g. `minecraft:overworld`.
    pub dimension: String,
}

#[derive(Clone, Debug, Default)]
/// What `level.dat` says about the world.
pub struct LevelInfo {
    /// The world's name (`LevelName`).
    pub name: String,
    /// The game version that last saved the world, e.g. `1.21.11`.
    pub version_name: Option<String>,
    /// The save's `DataVersion`.
    pub data_version: Option<i32>,
    /// World spawn in block coordinates.
    pub spawn: [i32; 3],
    /// Time of day in ticks (`DayTime`); 6000 when absent.
    pub day_time: i64,
    /// The single-player player, when the save has one.
    pub player: Option<PlayerInfo>,
}

/// Reads `level.dat` (gzip-compressed or raw NBT).
pub fn read_level_dat(path: &Path) -> Result<LevelInfo> {
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&raw[..])
        .read_to_end(&mut bytes)
        .or_else(|_| {
            bytes = raw.clone();
            Ok::<usize, std::io::Error>(raw.len())
        })?;
    let root: Value = fastnbt::from_bytes(&bytes).context("decoding level.dat")?;
    let data = nbt::compound(&root)
        .and_then(|r| r.get("Data"))
        .and_then(nbt::compound)
        .context("level.dat has no Data compound")?;
    Ok(parse_level(data))
}

fn parse_level(d: &Compound) -> LevelInfo {
    let spawn = if let Some(pos) = nbt::get(d, "spawn.pos").and_then(nbt::as_f64_list) {
        // 1.21.9+: spawn = {dimension, pos: [I; x, y, z], ...}
        [pos[0] as i32, pos[1] as i32, pos[2] as i32]
    } else {
        [
            nbt::get_f64(d, "SpawnX").unwrap_or(0.0) as i32,
            nbt::get_f64(d, "SpawnY").unwrap_or(64.0) as i32,
            nbt::get_f64(d, "SpawnZ").unwrap_or(0.0) as i32,
        ]
    };
    let player = nbt::get(d, "Player")
        .and_then(nbt::compound)
        .and_then(parse_player);
    LevelInfo {
        name: nbt::get_str(d, "LevelName").unwrap_or("").to_string(),
        version_name: nbt::get_str(d, "Version.Name").map(str::to_string),
        data_version: nbt::get_f64(d, "DataVersion").map(|v| v as i32),
        spawn,
        day_time: nbt::get_f64(d, "DayTime").unwrap_or(6000.0) as i64,
        player,
    }
}

/// Reads a player compound (`Pos`, `Rotation`, `Dimension`). Returns `None`
/// without a position.
pub fn parse_player(p: &Compound) -> Option<PlayerInfo> {
    let pos = nbt::get(p, "Pos").and_then(nbt::as_f64_list)?;
    let rot = nbt::get(p, "Rotation")
        .and_then(nbt::as_f64_list)
        .unwrap_or(vec![0.0, 0.0]);
    let dimension = match nbt::get(p, "Dimension") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => match nbt::as_i64(v) {
            Some(-1) => "minecraft:the_nether".into(),
            Some(1) => "minecraft:the_end".into(),
            _ => "minecraft:overworld".into(),
        },
        None => "minecraft:overworld".into(),
    };
    Some(PlayerInfo {
        pos: [pos[0], pos[1], pos[2]],
        yaw: rot[0] as f32,
        pitch: rot.get(1).copied().unwrap_or(0.0) as f32,
        dimension,
    })
}
