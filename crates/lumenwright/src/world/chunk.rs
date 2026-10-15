//! Chunk NBT decoding for the 1.18+ layout (`sections[].block_states`) and the
//! 1.16/1.17 layout (`Level.Sections[].Palette/BlockStates`).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use fastnbt::{ByteArray, LongArray, Value};
use serde::Deserialize;

/// A block state as written in the save: namespaced name plus sorted properties.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub struct BlockState {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Properties", default)]
    pub properties: BTreeMap<String, String>,
}

impl BlockState {
    pub fn new(name: &str) -> BlockState {
        BlockState {
            name: name.to_string(),
            properties: BTreeMap::new(),
        }
    }

    pub fn with(mut self, k: &str, v: &str) -> BlockState {
        self.properties.insert(k.to_string(), v.to_string());
        self
    }

    pub fn prop(&self, k: &str) -> Option<&str> {
        self.properties.get(k).map(|s| s.as_str())
    }

    /// Name without the `minecraft:` namespace.
    pub fn short_name(&self) -> &str {
        self.name.strip_prefix("minecraft:").unwrap_or(&self.name)
    }

    pub fn is_air(&self) -> bool {
        matches!(self.short_name(), "air" | "cave_air" | "void_air")
    }
}

impl std::fmt::Display for BlockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        if !self.properties.is_empty() {
            let props: Vec<String> = self
                .properties
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            write!(f, "[{}]", props.join(","))?;
        }
        Ok(())
    }
}

/// One decoded 16³ section with chunk-local palettes.
pub struct ParsedSection {
    pub y: i32,
    pub palette: Vec<BlockState>,
    /// 4096 palette indices (YZX order), or `None` when the palette has one entry.
    pub indices: Option<Vec<u16>>,
    pub biome_palette: Vec<String>,
    /// 64 biome palette indices (4×4×4, YZX order), or `None` for a single biome.
    pub biome_indices: Option<Vec<u8>>,
    pub sky_light: Option<Vec<u8>>,
    pub block_light: Option<Vec<u8>>,
}

pub struct ParsedChunk {
    pub x: i32,
    pub z: i32,
    pub data_version: i32,
    pub sections: Vec<ParsedSection>,
    /// Pre-1.17 worlds keep entities inside the chunk.
    pub legacy_entities: Vec<Value>,
    pub block_entities: Vec<Value>,
}

#[derive(Deserialize)]
struct Paletted<T> {
    palette: Vec<T>,
    data: Option<LongArray>,
}

#[derive(Deserialize)]
struct SectionNbt {
    #[serde(rename = "Y")]
    y: i8,
    block_states: Option<Paletted<BlockState>>,
    biomes: Option<Paletted<String>>,
    #[serde(rename = "SkyLight")]
    sky_light: Option<ByteArray>,
    #[serde(rename = "BlockLight")]
    block_light: Option<ByteArray>,
    // 1.16 / 1.17
    #[serde(rename = "Palette")]
    legacy_palette: Option<Vec<BlockState>>,
    #[serde(rename = "BlockStates")]
    legacy_states: Option<LongArray>,
}

#[derive(Deserialize)]
struct LegacyLevel {
    #[serde(rename = "xPos")]
    x_pos: i32,
    #[serde(rename = "zPos")]
    z_pos: i32,
    #[serde(rename = "Sections", default)]
    sections: Vec<SectionNbt>,
    #[serde(rename = "Entities", default)]
    entities: Vec<Value>,
    #[serde(rename = "TileEntities", default)]
    tile_entities: Vec<Value>,
}

#[derive(Deserialize)]
struct ChunkNbt {
    #[serde(rename = "DataVersion", default)]
    data_version: i32,
    #[serde(rename = "xPos")]
    x_pos: Option<i32>,
    #[serde(rename = "zPos")]
    z_pos: Option<i32>,
    sections: Option<Vec<SectionNbt>>,
    #[serde(rename = "Level")]
    level: Option<LegacyLevel>,
    #[serde(default)]
    block_entities: Vec<Value>,
}

/// Unpacks a 1.16+ (non-spanning) packed long array.
pub fn unpack_indices(data: &[i64], bits: u32, count: usize) -> Vec<u16> {
    let per_long = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let word = data.get(i / per_long).copied().unwrap_or(0) as u64;
        let shift = (i % per_long) as u32 * bits;
        out.push(((word >> shift) & mask) as u16);
    }
    out
}

/// Bits per entry for block palettes (minimum 4).
pub fn block_bits(palette_len: usize) -> u32 {
    bits_for(palette_len).max(4)
}

fn bits_for(len: usize) -> u32 {
    if len <= 1 {
        0
    } else {
        usize::BITS - (len - 1).leading_zeros()
    }
}

fn decode_section(s: SectionNbt) -> Option<ParsedSection> {
    let (palette, data) = if let Some(bs) = s.block_states {
        (bs.palette, bs.data)
    } else if let Some(p) = s.legacy_palette {
        (p, s.legacy_states)
    } else {
        (Vec::new(), None)
    };
    let indices = match (&data, palette.len()) {
        (Some(d), n) if n > 1 => Some(unpack_indices(d, block_bits(n), 4096)),
        _ => None,
    };
    let (biome_palette, biome_indices) = match s.biomes {
        Some(b) => {
            let idx = match (&b.data, b.palette.len()) {
                (Some(d), n) if n > 1 => Some(
                    unpack_indices(d, bits_for(n), 64)
                        .into_iter()
                        .map(|v| v as u8)
                        .collect(),
                ),
                _ => None,
            };
            (b.palette, idx)
        }
        None => (Vec::new(), None),
    };
    let sky_light = s.sky_light.map(|a| {
        a.into_inner()
            .into_iter()
            .map(|b| b as u8)
            .collect::<Vec<u8>>()
    });
    let block_light = s.block_light.map(|a| {
        a.into_inner()
            .into_iter()
            .map(|b| b as u8)
            .collect::<Vec<u8>>()
    });
    Some(ParsedSection {
        y: s.y as i32,
        palette,
        indices,
        biome_palette,
        biome_indices,
        sky_light: sky_light.filter(|v| v.len() == 2048),
        block_light: block_light.filter(|v| v.len() == 2048),
    })
}

pub fn parse_chunk(nbt: &[u8]) -> Result<ParsedChunk> {
    let c: ChunkNbt = fastnbt::from_bytes(nbt).context("decoding chunk NBT")?;
    if let Some(level) = c.level {
        return Ok(ParsedChunk {
            x: level.x_pos,
            z: level.z_pos,
            data_version: c.data_version,
            sections: level
                .sections
                .into_iter()
                .filter_map(decode_section)
                .collect(),
            legacy_entities: level.entities,
            block_entities: level.tile_entities,
        });
    }
    Ok(ParsedChunk {
        x: c.x_pos.context("chunk without xPos")?,
        z: c.z_pos.context("chunk without zPos")?,
        data_version: c.data_version,
        sections: c
            .sections
            .unwrap_or_default()
            .into_iter()
            .filter_map(decode_section)
            .collect(),
        legacy_entities: Vec::new(),
        block_entities: c.block_entities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits() {
        assert_eq!(block_bits(1), 4);
        assert_eq!(block_bits(16), 4);
        assert_eq!(block_bits(17), 5);
        assert_eq!(bits_for(2), 1);
        assert_eq!(bits_for(3), 2);
        assert_eq!(bits_for(64), 6);
    }

    #[test]
    fn unpack_non_spanning() {
        // 5 bits per entry -> 12 entries per long, top 4 bits unused.
        let mut words = vec![0i64; 2];
        let vals: Vec<u16> = (0..13).map(|i| (i * 2) as u16).collect();
        for (i, v) in vals.iter().enumerate() {
            let w = i / 12;
            words[w] |= (*v as i64) << ((i % 12) * 5);
        }
        assert_eq!(unpack_indices(&words, 5, 13), vals);
    }
}
