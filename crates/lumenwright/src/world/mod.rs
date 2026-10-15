//! Read-only access to a Minecraft Java world save.

pub mod chunk;
pub mod entity;
pub mod level;
pub mod nbt;
pub mod region;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;

pub use chunk::BlockState;
pub use entity::Entity;
pub use level::LevelInfo;

/// Interned block state id. `0` is always air.
pub type StateId = u32;

/// Interns block states and biome names seen in the loaded area.
#[derive(Default)]
pub struct Registry {
    pub states: Vec<BlockState>,
    state_ids: HashMap<BlockState, StateId>,
    pub biomes: Vec<String>,
    biome_ids: HashMap<String, u16>,
}

impl Registry {
    pub fn new() -> Registry {
        let mut r = Registry::default();
        r.intern(&BlockState::new("minecraft:air"));
        r.intern_biome("minecraft:plains");
        r
    }

    pub fn intern(&mut self, s: &BlockState) -> StateId {
        if s.is_air()
            && let Some(&id) = self.state_ids.get(&BlockState::new("minecraft:air"))
        {
            return id;
        }
        if let Some(&id) = self.state_ids.get(s) {
            return id;
        }
        let id = self.states.len() as StateId;
        self.states.push(s.clone());
        self.state_ids.insert(s.clone(), id);
        id
    }

    pub fn intern_biome(&mut self, b: &str) -> u16 {
        if let Some(&id) = self.biome_ids.get(b) {
            return id;
        }
        let id = self.biomes.len() as u16;
        self.biomes.push(b.to_string());
        self.biome_ids.insert(b.to_string(), id);
        id
    }

    pub fn state(&self, id: StateId) -> &BlockState {
        &self.states[id as usize]
    }
}

pub enum SectionBlocks {
    Uniform(StateId),
    Paletted {
        palette: Vec<StateId>,
        indices: Box<[u16]>,
    },
}

pub struct Section {
    pub blocks: SectionBlocks,
    pub biomes: [u16; 64],
    pub sky_light: Option<Box<[u8]>>,
    pub block_light: Option<Box<[u8]>>,
}

impl Section {
    #[inline]
    pub fn block(&self, lx: usize, ly: usize, lz: usize) -> StateId {
        match &self.blocks {
            SectionBlocks::Uniform(s) => *s,
            SectionBlocks::Paletted { palette, indices } => {
                palette[indices[(ly * 16 + lz) * 16 + lx] as usize]
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.blocks, SectionBlocks::Uniform(0))
    }
}

#[inline]
fn nibble(arr: &[u8], lx: usize, ly: usize, lz: usize) -> u8 {
    let i = (ly * 16 + lz) * 16 + lx;
    (arr[i >> 1] >> ((i & 1) * 4)) & 15
}

/// A rectangular, chunk-aligned region of loaded blocks.
pub struct BlockVolume {
    pub min_chunk: [i32; 2],
    pub chunks: [i32; 2],
    pub min_section: i32,
    pub sections_y: i32,
    pub sections: Vec<Option<Section>>,
}

impl BlockVolume {
    pub fn min_block(&self) -> [i32; 3] {
        [
            self.min_chunk[0] * 16,
            self.min_section * 16,
            self.min_chunk[1] * 16,
        ]
    }

    pub fn size_blocks(&self) -> [i32; 3] {
        [
            self.chunks[0] * 16,
            self.sections_y * 16,
            self.chunks[1] * 16,
        ]
    }

    #[inline]
    pub fn section_index(&self, sx: i32, sy: i32, sz: i32) -> Option<usize> {
        let x = sx - self.min_chunk[0];
        let y = sy - self.min_section;
        let z = sz - self.min_chunk[1];
        if x < 0
            || y < 0
            || z < 0
            || x >= self.chunks[0]
            || y >= self.sections_y
            || z >= self.chunks[1]
        {
            return None;
        }
        Some(((y * self.chunks[1] + z) * self.chunks[0] + x) as usize)
    }

    #[inline]
    pub fn section(&self, sx: i32, sy: i32, sz: i32) -> Option<&Section> {
        self.section_index(sx, sy, sz)
            .and_then(|i| self.sections[i].as_ref())
    }

    /// Block state at world coordinates; air outside the loaded area.
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> StateId {
        match self.section(x >> 4, y >> 4, z >> 4) {
            Some(s) => s.block((x & 15) as usize, (y & 15) as usize, (z & 15) as usize),
            None => 0,
        }
    }

    /// Biome id at world coordinates (4×4×4 resolution).
    pub fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        match self.section(x >> 4, y >> 4, z >> 4) {
            Some(s) => {
                s.biomes[((((y & 15) >> 2) * 4 + ((z & 15) >> 2)) * 4 + ((x & 15) >> 2)) as usize]
            }
            None => 0,
        }
    }

    /// (sky, block) light at world coordinates, following vanilla's rule that a
    /// section without a sky-light layer inherits the bottom of the next stored
    /// layer above it (or full light above the world).
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let (lx, lz) = ((x & 15) as usize, (z & 15) as usize);
        let sx = x >> 4;
        let sz = z >> 4;
        let mut sy = y >> 4;
        let block = self
            .section(sx, sy, sz)
            .and_then(|s| s.block_light.as_deref())
            .map(|b| nibble(b, lx, (y & 15) as usize, lz))
            .unwrap_or(0);
        let mut ly = (y & 15) as usize;
        let top = self.min_section + self.sections_y;
        let sky = loop {
            if sy >= top {
                break 15;
            }
            if let Some(sl) = self
                .section(sx, sy, sz)
                .and_then(|s| s.sky_light.as_deref())
            {
                break nibble(sl, lx, ly, lz);
            }
            if self.section_index(sx, sy, sz).is_none() {
                break 15;
            }
            sy += 1;
            ly = 0;
        };
        (sky, block)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}

impl Dimension {
    pub fn parse(s: &str) -> Result<Dimension> {
        Ok(match s.trim_start_matches("minecraft:") {
            "overworld" => Dimension::Overworld,
            "nether" | "the_nether" => Dimension::Nether,
            "end" | "the_end" => Dimension::End,
            other => bail!("unknown dimension '{other}' (expected overworld, nether, end)"),
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Dimension::Overworld => "minecraft:overworld",
            Dimension::Nether => "minecraft:the_nether",
            Dimension::End => "minecraft:the_end",
        }
    }
}

pub struct World {
    pub root: PathBuf,
    pub level: LevelInfo,
    pub dimension: Dimension,
    dim_dir: PathBuf,
}

pub struct LoadedArea {
    pub registry: Registry,
    pub volume: BlockVolume,
    pub entities: Vec<Entity>,
    pub block_entities: Vec<fastnbt::Value>,
    pub chunks_loaded: usize,
}

impl World {
    pub fn open(root: &Path, dimension: Dimension) -> Result<World> {
        let level_path = root.join("level.dat");
        if !level_path.exists() {
            bail!("{} is not a Minecraft world (no level.dat)", root.display());
        }
        let level = level::read_level_dat(&level_path)?;
        let legacy = match dimension {
            Dimension::Overworld => root.to_path_buf(),
            Dimension::Nether => root.join("DIM-1"),
            Dimension::End => root.join("DIM1"),
        };
        let name = dimension.id().trim_start_matches("minecraft:");
        let modern = root.join("dimensions").join("minecraft").join(name);
        let dim_dir = if modern.join("region").is_dir() {
            modern
        } else {
            legacy
        };
        Ok(World {
            root: root.to_path_buf(),
            level,
            dimension,
            dim_dir,
        })
    }

    pub fn region_dir(&self) -> PathBuf {
        self.dim_dir.join("region")
    }

    pub fn entities_dir(&self) -> PathBuf {
        self.dim_dir.join("entities")
    }

    /// Lists region coordinates present in the dimension.
    pub fn regions(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.region_dir()) {
            for e in rd.flatten() {
                if e.metadata().map(|m| m.len() >= 8192).unwrap_or(false)
                    && let Some(rc) = region::parse_region_name(&e.path())
                {
                    out.push(rc);
                }
            }
        }
        out.sort();
        out
    }

    /// Loads all chunks in the inclusive chunk rectangle `[min, max]`.
    pub fn load_area(
        &self,
        min_chunk: [i32; 2],
        max_chunk: [i32; 2],
        with_entities: bool,
    ) -> Result<LoadedArea> {
        let chunks = [
            max_chunk[0] - min_chunk[0] + 1,
            max_chunk[1] - min_chunk[1] + 1,
        ];
        let mut by_region: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
        for cz in min_chunk[1]..=max_chunk[1] {
            for cx in min_chunk[0]..=max_chunk[0] {
                by_region
                    .entry((cx >> 5, cz >> 5))
                    .or_default()
                    .push((cx, cz));
            }
        }
        // Sorted so state interning (and everything downstream) is reproducible.
        let mut jobs: Vec<_> = by_region.into_iter().collect();
        jobs.sort();

        // Decode chunks in parallel, intern afterwards.
        let parsed: Vec<chunk::ParsedChunk> = jobs
            .par_iter()
            .map(|((rx, rz), list)| -> Result<Vec<chunk::ParsedChunk>> {
                let path = self.region_dir().join(format!("r.{rx}.{rz}.mca"));
                let Some(region) = region::Region::open(&path)? else {
                    return Ok(Vec::new());
                };
                let out: Vec<chunk::ParsedChunk> = list
                    .par_iter()
                    .filter_map(|&(cx, cz)| {
                        match region.chunk_nbt((cx & 31) as usize, (cz & 31) as usize) {
                            Ok(Some(bytes)) => match chunk::parse_chunk(&bytes) {
                                Ok(c) => Some(c),
                                Err(e) => {
                                    log::warn!("chunk ({cx},{cz}): {e:#}");
                                    None
                                }
                            },
                            Ok(None) => None,
                            Err(e) => {
                                log::warn!("chunk ({cx},{cz}): {e:#}");
                                None
                            }
                        }
                    })
                    .collect();
                Ok(out)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        let (min_section, max_section) = parsed
            .iter()
            .flat_map(|c| c.sections.iter().map(|s| s.y))
            .fold((i32::MAX, i32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        let (min_section, sections_y) = if min_section > max_section {
            (0, 1)
        } else {
            (min_section, max_section - min_section + 1)
        };

        let mut registry = Registry::new();
        let mut volume = BlockVolume {
            min_chunk,
            chunks,
            min_section,
            sections_y,
            sections: (0..(chunks[0] * chunks[1] * sections_y))
                .map(|_| None)
                .collect(),
        };
        let mut entities = Vec::new();
        let mut block_entities = Vec::new();
        let chunks_loaded = parsed.len();
        for c in parsed {
            for e in &c.legacy_entities {
                entity::collect_entity(e, &mut entities);
            }
            block_entities.extend(c.block_entities);
            for s in c.sections {
                let Some(idx) = volume.section_index(c.x, s.y, c.z) else {
                    continue;
                };
                let palette: Vec<StateId> = s.palette.iter().map(|b| registry.intern(b)).collect();
                if palette.is_empty() && s.sky_light.is_none() && s.block_light.is_none() {
                    continue;
                }
                let blocks = match (palette.len(), s.indices) {
                    (0, _) => SectionBlocks::Uniform(0),
                    (1, _) | (_, None) => SectionBlocks::Uniform(palette[0]),
                    (_, Some(idx)) => {
                        // Guard against corrupt indices.
                        let max = palette.len() as u16 - 1;
                        let idx: Vec<u16> = idx.into_iter().map(|i| i.min(max)).collect();
                        if palette.iter().all(|&p| p == palette[0]) {
                            SectionBlocks::Uniform(palette[0])
                        } else {
                            SectionBlocks::Paletted {
                                palette,
                                indices: idx.into_boxed_slice(),
                            }
                        }
                    }
                };
                let bp: Vec<u16> = s
                    .biome_palette
                    .iter()
                    .map(|b| registry.intern_biome(b))
                    .collect();
                let mut biomes = [0u16; 64];
                if !bp.is_empty() {
                    for (i, b) in biomes.iter_mut().enumerate() {
                        let pi = s.biome_indices.as_ref().map(|v| v[i] as usize).unwrap_or(0);
                        *b = bp[pi.min(bp.len() - 1)];
                    }
                }
                volume.sections[idx] = Some(Section {
                    blocks,
                    biomes,
                    sky_light: s.sky_light.map(Vec::into_boxed_slice),
                    block_light: s.block_light.map(Vec::into_boxed_slice),
                });
            }
        }

        if with_entities {
            entities.extend(self.load_entities(min_chunk, max_chunk)?);
        }
        Ok(LoadedArea {
            registry,
            volume,
            entities,
            block_entities,
            chunks_loaded,
        })
    }

    /// Reads entity-region files covering the chunk rectangle.
    pub fn load_entities(&self, min_chunk: [i32; 2], max_chunk: [i32; 2]) -> Result<Vec<Entity>> {
        let dir = self.entities_dir();
        let mut out = Vec::new();
        for rz in (min_chunk[1] >> 5)..=(max_chunk[1] >> 5) {
            for rx in (min_chunk[0] >> 5)..=(max_chunk[0] >> 5) {
                let path = dir.join(format!("r.{rx}.{rz}.mca"));
                let Some(region) = region::Region::open(&path)? else {
                    continue;
                };
                for lz in 0..32 {
                    for lx in 0..32 {
                        let (cx, cz) = (rx * 32 + lx, rz * 32 + lz);
                        if cx < min_chunk[0]
                            || cx > max_chunk[0]
                            || cz < min_chunk[1]
                            || cz > max_chunk[1]
                        {
                            continue;
                        }
                        match region.chunk_nbt(lx as usize, lz as usize) {
                            Ok(Some(bytes)) => match entity::parse_entity_chunk(&bytes) {
                                Ok(list) => out.extend(list),
                                Err(e) => log::warn!("entity chunk ({cx},{cz}): {e:#}"),
                            },
                            Ok(None) => {}
                            Err(e) => log::warn!("entity chunk ({cx},{cz}): {e:#}"),
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Single-player position from level.dat, if it is in this dimension.
    pub fn player(&self) -> Option<&level::PlayerInfo> {
        self.level
            .player
            .as_ref()
            .filter(|p| Dimension::parse(&p.dimension).ok() == Some(self.dimension))
    }

    /// Player records from `playerdata/*.dat` (multiplayer saves).
    pub fn other_players(&self) -> Vec<level::PlayerInfo> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(self.root.join("playerdata")) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("dat") {
                continue;
            }
            let parsed = (|| -> Result<Option<level::PlayerInfo>> {
                use std::io::Read;
                let raw = std::fs::read(&p)?;
                let mut bytes = Vec::new();
                flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut bytes)?;
                let v: fastnbt::Value = fastnbt::from_bytes(&bytes).context("player NBT")?;
                Ok(nbt::compound(&v).and_then(level::parse_player))
            })();
            if let Ok(Some(pi)) = parsed
                && Dimension::parse(&pi.dimension).ok() == Some(self.dimension)
            {
                out.push(pi);
            }
        }
        out
    }
}
