//! Writes small, deterministic Minecraft worlds (1.21.x format). Used by the
//! integration tests and by `lumenwright demo-world` to produce a showcase scene
//! without needing a real save.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use fastnbt::{ByteArray, IntArray, LongArray, Value};
use serde::Serialize;

use crate::world::BlockState;
use crate::world::chunk::block_bits;
use crate::world::region::write_region;

pub const DATA_VERSION: i32 = 4671;
pub const VERSION_NAME: &str = "1.21.11";

/// Region coordinates -> chunks (local x, local z, NBT bytes).
type RegionChunks = BTreeMap<(i32, i32), Vec<(usize, usize, Vec<u8>)>>;

#[derive(Clone, Debug)]
pub struct FixtureEntity {
    pub id: String,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub extra: Vec<(String, Value)>,
}

/// A block entity's id and extra fields.
pub type FixtureBlockEntity = (String, Vec<(String, Value)>);

/// An in-memory world: sparse blocks (default air) plus entities and block entities.
pub struct FixtureWorld {
    blocks: HashMap<[i32; 3], BlockState>,
    pub entities: Vec<FixtureEntity>,
    /// Block entity compounds by position (`id`, `x`, `y`, `z` are added on write).
    pub block_entities: BTreeMap<[i32; 3], FixtureBlockEntity>,
    pub biome: String,
    pub spawn: [i32; 3],
    pub player: Option<([f64; 3], f32, f32)>,
    pub day_time: i64,
}

impl Default for FixtureWorld {
    fn default() -> Self {
        Self::new()
    }
}

fn bs(name: &str) -> BlockState {
    BlockState::new(&format!("minecraft:{name}"))
}

impl FixtureWorld {
    pub fn new() -> FixtureWorld {
        FixtureWorld {
            blocks: HashMap::new(),
            entities: Vec::new(),
            block_entities: BTreeMap::new(),
            biome: "minecraft:plains".into(),
            spawn: [0, 64, 0],
            player: None,
            day_time: 6000,
        }
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, s: BlockState) {
        if s.is_air() {
            self.blocks.remove(&[x, y, z]);
        } else {
            self.blocks.insert([x, y, z], s);
        }
    }

    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<&BlockState> {
        self.blocks.get(&[x, y, z])
    }

    pub fn entity(&mut self, id: &str, pos: [f64; 3], yaw: f32, extra: Vec<(&str, Value)>) {
        self.entities.push(FixtureEntity {
            id: format!("minecraft:{id}"),
            pos,
            yaw,
            extra: extra.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        });
    }

    /// Adds a block entity (`id` without namespace) at a block position.
    pub fn block_entity(&mut self, pos: [i32; 3], id: &str, extra: Vec<(&str, Value)>) {
        self.block_entities.insert(
            pos,
            (
                format!("minecraft:{id}"),
                extra.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            ),
        );
    }

    fn is_opaque(&self, p: [i32; 3]) -> bool {
        match self.blocks.get(&p) {
            None => false,
            Some(s) => {
                let n = s.short_name();
                !(n.contains("glass")
                    || n.contains("leaves")
                    || n.contains("torch")
                    || n.contains("lantern")
                    || n.contains("stairs")
                    || n.contains("slab")
                    || n.contains("fence")
                    || n.contains("door")
                    || n.contains("sign")
                    || n.contains("banner")
                    || n.contains("chest")
                    || n.contains("skull")
                    || n.ends_with("_head")
                    || matches!(
                        n,
                        "bell" | "lectern" | "enchanting_table" | "decorated_pot" | "barrel"
                    )
                    || matches!(
                        n,
                        "water"
                            | "short_grass"
                            | "poppy"
                            | "dandelion"
                            | "air"
                            | "lava"
                            | "oxeye_daisy"
                            | "cornflower"
                    )
                    || n.contains("carpet"))
            }
        }
    }

    /// Simple flood-fill light engine: sky light falls straight down unattenuated
    /// and spreads with -1 per block; block light spreads from emitters.
    fn compute_light(
        &self,
        lo: [i32; 3],
        hi: [i32; 3],
    ) -> (HashMap<[i32; 3], u8>, HashMap<[i32; 3], u8>) {
        let mut sky: HashMap<[i32; 3], u8> = HashMap::new();
        let mut queue = VecDeque::new();
        for x in lo[0]..hi[0] {
            for z in lo[2]..hi[2] {
                let mut y = hi[1] - 1;
                while y >= lo[1] && !self.is_opaque([x, y, z]) {
                    sky.insert([x, y, z], 15);
                    queue.push_back([x, y, z]);
                    y -= 1;
                }
            }
        }
        let spread = |map: &mut HashMap<[i32; 3], u8>, queue: &mut VecDeque<[i32; 3]>| {
            while let Some(p) = queue.pop_front() {
                let l = map[&p];
                if l <= 1 {
                    continue;
                }
                for d in [
                    [1, 0, 0],
                    [-1, 0, 0],
                    [0, 1, 0],
                    [0, -1, 0],
                    [0, 0, 1],
                    [0, 0, -1],
                ] {
                    let q = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
                    if (0..3).any(|i| q[i] < lo[i] || q[i] >= hi[i]) || self.is_opaque(q) {
                        continue;
                    }
                    if map.get(&q).copied().unwrap_or(0) < l - 1 {
                        map.insert(q, l - 1);
                        queue.push_back(q);
                    }
                }
            }
        };
        spread(&mut sky, &mut queue);
        let mut block: HashMap<[i32; 3], u8> = HashMap::new();
        for (p, s) in &self.blocks {
            let e = crate::assets::blockinfo::light_emission(s);
            if e > 0 {
                block.insert(*p, e);
                queue.push_back(*p);
            }
        }
        spread(&mut block, &mut queue);
        (sky, block)
    }

    /// Writes `level.dat`, `region/` and `entities/` into `dir`.
    pub fn write(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir.join("region"))?;
        std::fs::create_dir_all(dir.join("entities"))?;
        let keys: Vec<[i32; 3]> = self.blocks.keys().copied().collect();
        let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
        for k in &keys {
            for i in 0..3 {
                lo[i] = lo[i].min(k[i]);
                hi[i] = hi[i].max(k[i]);
            }
        }
        if keys.is_empty() {
            lo = [0, 0, 0];
            hi = [0, 0, 0];
        }
        // Chunk-aligned bounds, with a section of air headroom for sky light.
        let cmin = [lo[0] >> 4, lo[2] >> 4];
        let cmax = [hi[0] >> 4, hi[2] >> 4];
        let smin = lo[1] >> 4;
        let smax = (hi[1] >> 4) + 1;
        let light_lo = [cmin[0] * 16, smin * 16, cmin[1] * 16];
        let light_hi = [(cmax[0] + 1) * 16, (smax + 1) * 16, (cmax[1] + 1) * 16];
        let (sky, block) = self.compute_light(light_lo, light_hi);

        let mut regions: RegionChunks = BTreeMap::new();
        for cz in cmin[1]..=cmax[1] {
            for cx in cmin[0]..=cmax[0] {
                let mut sections = Vec::new();
                for sy in smin..=smax {
                    sections.push(self.section_nbt(cx, sy, cz, &sky, &block));
                }
                let chunk = ChunkOut {
                    data_version: DATA_VERSION,
                    x_pos: cx,
                    z_pos: cz,
                    y_pos: smin,
                    status: "minecraft:full".into(),
                    is_light_on: 1,
                    sections,
                    block_entities: self
                        .block_entities
                        .iter()
                        .filter(|(p, _)| p[0] >> 4 == cx && p[2] >> 4 == cz)
                        .map(|(p, (id, extra))| {
                            let mut c: HashMap<String, Value> = HashMap::new();
                            c.insert("id".into(), Value::String(id.clone()));
                            c.insert("x".into(), Value::Int(p[0]));
                            c.insert("y".into(), Value::Int(p[1]));
                            c.insert("z".into(), Value::Int(p[2]));
                            for (k, v) in extra {
                                c.insert(k.clone(), v.clone());
                            }
                            Value::Compound(c)
                        })
                        .collect(),
                };
                let bytes = fastnbt::to_bytes(&chunk)?;
                regions.entry((cx >> 5, cz >> 5)).or_default().push((
                    (cx & 31) as usize,
                    (cz & 31) as usize,
                    bytes,
                ));
            }
        }
        for ((rx, rz), chunks) in &regions {
            std::fs::write(
                dir.join("region").join(format!("r.{rx}.{rz}.mca")),
                write_region(chunks),
            )?;
        }

        // Entities, grouped by chunk.
        let mut by_chunk: BTreeMap<(i32, i32), Vec<Value>> = BTreeMap::new();
        for e in &self.entities {
            let mut c: HashMap<String, Value> = HashMap::new();
            c.insert("id".into(), Value::String(e.id.clone()));
            c.insert(
                "Pos".into(),
                Value::List(e.pos.iter().map(|v| Value::Double(*v)).collect()),
            );
            c.insert(
                "Rotation".into(),
                Value::List(vec![Value::Float(e.yaw), Value::Float(0.0)]),
            );
            for (k, v) in &e.extra {
                c.insert(k.clone(), v.clone());
            }
            let ch = (
                (e.pos[0].floor() as i32) >> 4,
                (e.pos[2].floor() as i32) >> 4,
            );
            by_chunk.entry(ch).or_default().push(Value::Compound(c));
        }
        let mut eregions: RegionChunks = BTreeMap::new();
        for ((cx, cz), list) in by_chunk {
            let mut root: HashMap<String, Value> = HashMap::new();
            root.insert("DataVersion".into(), Value::Int(DATA_VERSION));
            root.insert(
                "Position".into(),
                Value::IntArray(IntArray::new(vec![cx, cz])),
            );
            root.insert("Entities".into(), Value::List(list));
            let bytes = fastnbt::to_bytes(&Value::Compound(root))?;
            eregions.entry((cx >> 5, cz >> 5)).or_default().push((
                (cx & 31) as usize,
                (cz & 31) as usize,
                bytes,
            ));
        }
        for ((rx, rz), chunks) in &eregions {
            std::fs::write(
                dir.join("entities").join(format!("r.{rx}.{rz}.mca")),
                write_region(chunks),
            )?;
        }

        // level.dat
        let mut data: HashMap<String, Value> = HashMap::new();
        data.insert(
            "LevelName".into(),
            Value::String("Lumenwright showcase".into()),
        );
        data.insert("DataVersion".into(), Value::Int(DATA_VERSION));
        let mut version: HashMap<String, Value> = HashMap::new();
        version.insert("Name".into(), Value::String(VERSION_NAME.into()));
        version.insert("Id".into(), Value::Int(DATA_VERSION));
        data.insert("Version".into(), Value::Compound(version));
        data.insert("SpawnX".into(), Value::Int(self.spawn[0]));
        data.insert("SpawnY".into(), Value::Int(self.spawn[1]));
        data.insert("SpawnZ".into(), Value::Int(self.spawn[2]));
        data.insert("DayTime".into(), Value::Long(self.day_time));
        if let Some((pos, yaw, pitch)) = self.player {
            let mut p: HashMap<String, Value> = HashMap::new();
            p.insert(
                "Pos".into(),
                Value::List(pos.iter().map(|v| Value::Double(*v)).collect()),
            );
            p.insert(
                "Rotation".into(),
                Value::List(vec![Value::Float(yaw), Value::Float(pitch)]),
            );
            p.insert(
                "Dimension".into(),
                Value::String("minecraft:overworld".into()),
            );
            data.insert("Player".into(), Value::Compound(p));
        }
        let mut root: HashMap<String, Value> = HashMap::new();
        root.insert("Data".into(), Value::Compound(data));
        let nbt = fastnbt::to_bytes(&Value::Compound(root))?;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&nbt)?;
        std::fs::write(dir.join("level.dat"), gz.finish()?)?;
        Ok(())
    }

    fn section_nbt(
        &self,
        cx: i32,
        sy: i32,
        cz: i32,
        sky: &HashMap<[i32; 3], u8>,
        block: &HashMap<[i32; 3], u8>,
    ) -> SectionOut {
        let mut palette: Vec<BlockState> = vec![bs("air")];
        let mut index: HashMap<BlockState, u16> = HashMap::new();
        index.insert(bs("air"), 0);
        let mut ids = vec![0u16; 4096];
        let mut sky_arr = vec![0u8; 2048];
        let mut block_arr = vec![0u8; 2048];
        for ly in 0..16 {
            for lz in 0..16 {
                for lx in 0..16 {
                    let p = [cx * 16 + lx, sy * 16 + ly, cz * 16 + lz];
                    let i = ((ly * 16 + lz) * 16 + lx) as usize;
                    if let Some(s) = self.blocks.get(&p) {
                        let n = palette.len() as u16;
                        let id = *index.entry(s.clone()).or_insert_with(|| {
                            palette.push(s.clone());
                            n
                        });
                        ids[i] = id;
                    }
                    let s = sky.get(&p).copied().unwrap_or(0);
                    let b = block.get(&p).copied().unwrap_or(0);
                    sky_arr[i >> 1] |= s << ((i & 1) * 4);
                    block_arr[i >> 1] |= b << ((i & 1) * 4);
                }
            }
        }
        let data = if palette.len() > 1 {
            let bits = block_bits(palette.len());
            let per = (64 / bits) as usize;
            let mut longs = vec![0i64; 4096usize.div_ceil(per)];
            for (i, id) in ids.iter().enumerate() {
                longs[i / per] |= (*id as i64) << ((i % per) as u32 * bits);
            }
            Some(LongArray::new(longs))
        } else {
            None
        };
        SectionOut {
            y: sy as i8,
            block_states: PalettedOut {
                palette: palette
                    .into_iter()
                    .map(|s| BlockStateOut {
                        name: s.name,
                        properties: s.properties,
                    })
                    .collect(),
                data,
            },
            biomes: BiomesOut {
                palette: vec![self.biome.clone()],
            },
            sky_light: ByteArray::new(sky_arr.into_iter().map(|b| b as i8).collect()),
            block_light: ByteArray::new(block_arr.into_iter().map(|b| b as i8).collect()),
        }
    }
}

#[derive(Serialize)]
struct BlockStateOut {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Properties", skip_serializing_if = "BTreeMap::is_empty")]
    properties: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PalettedOut {
    palette: Vec<BlockStateOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<LongArray>,
}

#[derive(Serialize)]
struct BiomesOut {
    palette: Vec<String>,
}

#[derive(Serialize)]
struct SectionOut {
    #[serde(rename = "Y")]
    y: i8,
    block_states: PalettedOut,
    biomes: BiomesOut,
    #[serde(rename = "SkyLight")]
    sky_light: ByteArray,
    #[serde(rename = "BlockLight")]
    block_light: ByteArray,
}

#[derive(Serialize)]
struct ChunkOut {
    #[serde(rename = "DataVersion")]
    data_version: i32,
    #[serde(rename = "xPos")]
    x_pos: i32,
    #[serde(rename = "zPos")]
    z_pos: i32,
    #[serde(rename = "yPos")]
    y_pos: i32,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "isLightOn")]
    is_light_on: i8,
    sections: Vec<SectionOut>,
    block_entities: Vec<Value>,
}

fn props(name: &str, kv: &[(&str, &str)]) -> BlockState {
    let mut s = bs(name);
    for (k, v) in kv {
        s = s.with(k, v);
    }
    s
}

/// Rolling hills, a lake, trees, flowers, a lit cottage and a cast of mobs.
pub fn showcase() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let r = 40;
    let water_level = 62;
    let height = |x: i32, z: i32| -> i32 {
        let (fx, fz) = (x as f32, z as f32);
        let h =
            64.0 + 3.0 * (fx / 9.0).sin() + 2.5 * (fz / 7.0).cos() + 1.5 * ((fx + fz) / 5.0).sin();
        // A lake basin to the south-east.
        let d = ((fx - 14.0).powi(2) + (fz - 12.0).powi(2)).sqrt();
        let lake = if d < 11.0 { (11.0 - d) * 0.7 } else { 0.0 };
        // Flatten the building plot.
        if (-12..=0).contains(&x) && (-12..=-2).contains(&z) {
            return 65;
        }
        (h - lake).round() as i32
    };
    for x in -r..r {
        for z in -r..r {
            let h = height(x, z);
            for y in 52..=h {
                let s = if y == h {
                    if h <= water_level {
                        "sand"
                    } else {
                        "grass_block"
                    }
                } else if y >= h - 3 {
                    if h <= water_level + 1 { "sand" } else { "dirt" }
                } else {
                    "stone"
                };
                let mut st = bs(s);
                if s == "grass_block" {
                    st = st.with("snowy", "false");
                }
                w.set(x, y, z, st);
            }
            for y in h + 1..=water_level {
                w.set(x, y, z, props("water", &[("level", "0")]));
            }
            // Vegetation.
            let hash = crate::assets::blockstate::pos_hash(x, 0, z, 99) % 100;
            if h > water_level
                && w.get(x, h, z)
                    .is_some_and(|s| s.short_name() == "grass_block")
            {
                let plant = match hash {
                    0..=17 => Some("short_grass"),
                    18..=20 => Some("poppy"),
                    21..=23 => Some("dandelion"),
                    24 => Some("oxeye_daisy"),
                    25 => Some("cornflower"),
                    _ => None,
                };
                if let Some(p) = plant
                    && !((-13..=1).contains(&x) && (-13..=-1).contains(&z))
                {
                    w.set(x, h + 1, z, bs(p));
                }
            }
        }
    }

    // Trees.
    let tree = |w: &mut FixtureWorld, x: i32, z: i32, log: &str, leaves: &str, tall: i32| {
        let base = height(x, z) + 1;
        for y in base..base + tall {
            w.set(x, y, z, props(log, &[("axis", "y")]));
        }
        let top = base + tall;
        for dy in -2..=1 {
            let rad: i32 = if dy >= 0 { 1 } else { 2 };
            for dx in -rad..=rad {
                for dz in -rad..=rad {
                    if dx == 0 && dz == 0 && dy < 0 {
                        continue;
                    }
                    if dx.abs() == rad
                        && dz.abs() == rad
                        && (dy == 1
                            || crate::assets::blockstate::pos_hash(dx, dy, dz, 5).is_multiple_of(2))
                    {
                        continue;
                    }
                    let p = (x + dx, top + dy, z + dz);
                    if w.get(p.0, p.1, p.2).is_none() {
                        w.set(
                            p.0,
                            p.1,
                            p.2,
                            props(
                                leaves,
                                &[
                                    ("distance", "1"),
                                    ("persistent", "true"),
                                    ("waterlogged", "false"),
                                ],
                            ),
                        );
                    }
                }
            }
        }
    };
    tree(&mut w, -20, 8, "oak_log", "oak_leaves", 5);
    tree(&mut w, -26, -14, "oak_log", "oak_leaves", 4);
    tree(&mut w, 8, -20, "birch_log", "birch_leaves", 6);
    tree(&mut w, 22, -6, "spruce_log", "spruce_leaves", 6);
    tree(&mut w, -6, 18, "oak_log", "oak_leaves", 5);

    // Cottage: 11x9 footprint at x -11..=-1, z -11..=-3, floor y=65.
    let (x0, x1, z0, z1, fy) = (-11, -1, -11, -3, 65);
    for x in x0..=x1 {
        for z in z0..=z1 {
            w.set(x, fy, z, bs("oak_planks"));
            let wall = x == x0 || x == x1 || z == z0 || z == z1;
            for y in fy + 1..=fy + 4 {
                if wall {
                    let corner = (x == x0 || x == x1) && (z == z0 || z == z1);
                    let s = if corner {
                        props("stripped_oak_log", &[("axis", "y")])
                    } else if y == fy + 2 && (x - x0) % 3 == 2 && !corner {
                        bs("glass")
                    } else if y == fy + 2 && (z - z0) % 3 == 2 && (x == x0 || x == x1) {
                        bs("glass_pane")
                    } else {
                        bs("cobblestone")
                    };
                    w.set(x, y, z, s);
                } else {
                    w.set(x, y, z, bs("air"));
                }
            }
        }
    }
    // Door opening on the south wall, with an oak door.
    w.set(
        -6,
        fy + 1,
        z1,
        props(
            "oak_door",
            &[
                ("facing", "south"),
                ("half", "lower"),
                ("hinge", "left"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
    );
    w.set(
        -6,
        fy + 2,
        z1,
        props(
            "oak_door",
            &[
                ("facing", "south"),
                ("half", "upper"),
                ("hinge", "left"),
                ("open", "false"),
                ("powered", "false"),
            ],
        ),
    );
    // Gable roof of stairs and slabs.
    for i in 0..=5 {
        let y = fy + 5 + i;
        for x in x0 - 1..=x1 + 1 {
            let zs = z0 - 1 + i;
            let zn = z1 + 1 - i;
            if zs > zn {
                break;
            }
            if zs == zn {
                w.set(
                    x,
                    y,
                    zs,
                    props("oak_slab", &[("type", "bottom"), ("waterlogged", "false")]),
                );
                continue;
            }
            let stair = |facing: &str| {
                props(
                    "oak_stairs",
                    &[
                        ("facing", facing),
                        ("half", "bottom"),
                        ("shape", "straight"),
                        ("waterlogged", "false"),
                    ],
                )
            };
            w.set(x, y, zs, stair("south"));
            w.set(x, y, zn, stair("north"));
            if i > 0 && (x == x0 || x == x1) {
                for z in zs + 1..zn {
                    w.set(x, y, z, bs("oak_planks"));
                }
            }
        }
    }
    // Interior lights and furniture.
    w.set(
        -9,
        fy + 3,
        z0 + 1,
        props("wall_torch", &[("facing", "south")]),
    );
    w.set(
        -3,
        fy + 3,
        z0 + 1,
        props("wall_torch", &[("facing", "south")]),
    );
    w.set(-6, fy + 1, z0 + 1, props("crafting_table", &[]));
    w.set(
        -5,
        fy + 1,
        z0 + 1,
        props("furnace", &[("facing", "south"), ("lit", "true")]),
    );
    w.set(
        -8,
        fy + 1,
        z0 + 1,
        props("barrel", &[("facing", "up"), ("open", "false")]),
    );
    w.set(
        -2,
        fy + 1,
        -5,
        props("lantern", &[("hanging", "false"), ("waterlogged", "false")]),
    );
    // Path and fence.
    for z in z1 + 1..=z1 + 6 {
        let h = height(-6, z);
        w.set(-6, h, z, bs("dirt_path"));
        w.set(-6, h + 1, z, bs("air"));
    }
    for x in -16..=-13 {
        let h = height(x, 0);
        let e = if x == -16 { "false" } else { "true" };
        let wv = if x == -13 { "false" } else { "true" };
        w.set(
            x,
            h + 1,
            0,
            props(
                "oak_fence",
                &[
                    ("east", wv),
                    ("west", e),
                    ("north", "false"),
                    ("south", "false"),
                    ("waterlogged", "false"),
                ],
            ),
        );
    }
    // Lamp post with glowstone and a jack o'lantern by the lake.
    let lh = height(4, 2);
    for y in lh + 1..=lh + 3 {
        w.set(
            4,
            y,
            2,
            props(
                "oak_fence",
                &[
                    ("east", "false"),
                    ("west", "false"),
                    ("north", "false"),
                    ("south", "false"),
                    ("waterlogged", "false"),
                ],
            ),
        );
    }
    w.set(4, lh + 4, 2, bs("glowstone"));
    let jh = height(2, 6);
    w.set(
        2,
        jh + 1,
        6,
        props("jack_o_lantern", &[("facing", "south")]),
    );
    // Stained glass gazebo by the lake.
    let gh = height(-2, 14);
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        w.set(-2 + dx, gh + 3, 14 + dz, bs("light_blue_stained_glass"));
    }
    for (dx, dz) in [(-1, -1), (2, -1), (-1, 2), (2, 2)] {
        for y in gh + 1..=gh + 2 {
            w.set(-2 + dx, y, 14 + dz, bs("quartz_pillar").with("axis", "y"));
        }
    }

    // Mobs (y = ground + 1).
    let g = |x: f64, z: f64| height(x.floor() as i32, z.floor() as i32) as f64 + 1.0;
    w.entity(
        "pig",
        [6.5, g(6.5, -4.5), -4.5],
        40.0,
        vec![("variant", Value::String("minecraft:temperate".into()))],
    );
    w.entity("cow", [10.5, g(10.5, -10.5), -10.5], -120.0, vec![]);
    w.entity(
        "sheep",
        [3.5, g(3.5, -9.5), -9.5],
        200.0,
        vec![("Color", Value::Byte(0))],
    );
    w.entity("chicken", [-14.5, g(-14.5, 3.5), 3.5], 15.0, vec![]);
    w.entity("villager", [-6.5, g(-6.5, 1.5), 1.5], 0.0, vec![]);
    w.entity("zombie", [-18.5, g(-18.5, -4.5), -4.5], 60.0, vec![]);
    w.entity("skeleton", [16.5, g(16.5, -16.5), -16.5], 150.0, vec![]);
    w.entity("creeper", [-2.5, g(-2.5, 8.5), 8.5], 190.0, vec![]);
    w.entity("armor_stand", [-9.5, (fy + 1) as f64, -6.5], 30.0, vec![]);
    w.entity("wolf", [0.5, g(0.5, -0.5), -0.5], 110.0, vec![]);
    w.entity(
        "cat",
        [-4.5, (fy + 1) as f64, -9.5],
        200.0,
        vec![("variant", Value::String("minecraft:calico".into()))],
    );
    w.entity("iron_golem", [12.5, g(12.5, 2.5), 2.5], -60.0, vec![]);
    w.entity("boat", [14.5, water_level as f64 + 0.4, 12.5], 30.0, vec![]);
    // Item frames on the cottage's south wall, either side of the door.
    for x in [-8.5, -3.5] {
        w.entity(
            "item_frame",
            [x, (fy + 2) as f64 + 0.5, z1 as f64 + 1.03125],
            0.0,
            vec![("Facing", Value::Byte(3))],
        );
    }
    // A painting on the cottage's east outer wall, facing east.
    w.entity(
        "painting",
        [x1 as f64 + 1.03125, (fy + 3) as f64, -7.0],
        270.0,
        vec![
            ("variant", Value::String("minecraft:kebab".into())),
            ("facing", Value::Byte(5)),
        ],
    );

    w.spawn = [0, 66, 0];
    w.player = Some(([18.5, 72.0, 22.5], 150.0, 18.0));
    w.day_time = 3000;
    w
}

/// A flat platform with the given entities in rows, all facing south (+Z),
/// for checking entity models. Rows are 4 blocks apart, columns 3.
pub fn lineup(ids: &[String], per_row: usize) -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let rows = ids.len().div_ceil(per_row) as i32;
    let (wx, wz) = (per_row as i32 * 3 + 6, rows * 5 + 10);
    for x in -4..wx {
        for z in -6..wz {
            w.set(x, 63, z, bs("stone"));
            let checker = ((x.div_euclid(3) + z.div_euclid(5)) & 1) == 0;
            w.set(
                x,
                64,
                z,
                bs(if checker {
                    "smooth_stone"
                } else {
                    "polished_andesite"
                }),
            );
        }
    }
    for (i, id) in ids.iter().enumerate() {
        let (col, row) = ((i % per_row) as f64, (i / per_row) as f64);
        let id = id.trim_start_matches("minecraft:");
        let extra = if id == "painting" {
            vec![
                ("variant", Value::String("minecraft:kebab".into())),
                ("facing", Value::Byte(3)),
            ]
        } else {
            vec![]
        };
        w.entity(id, [1.5 + col * 3.0, 65.0, 1.5 + row * 5.0], 0.0, extra);
    }
    let cx = per_row as f64 * 1.5;
    w.player = Some(([cx, 72.0, rows as f64 * 5.0 + 8.0], 180.0, 30.0));
    w
}

fn text_side(lines: [Value; 4], color: &str, glowing: bool) -> Value {
    // NBT lists hold one tag type; vanilla wraps the elements of a mixed list as
    // `{"": element}`.
    let mixed = lines.iter().any(|l| matches!(l, Value::Compound(_)));
    let lines: Vec<Value> = lines
        .into_iter()
        .map(|l| match l {
            Value::String(_) if mixed => compound(vec![("", l)]),
            l => l,
        })
        .collect();
    let mut c: HashMap<String, Value> = HashMap::new();
    c.insert("messages".into(), Value::List(lines));
    c.insert("color".into(), Value::String(color.into()));
    c.insert("has_glowing_text".into(), Value::Byte(glowing as i8));
    Value::Compound(c)
}

fn s(t: &str) -> Value {
    Value::String(t.into())
}

fn styled(t: &str, color: &str) -> Value {
    let mut c: HashMap<String, Value> = HashMap::new();
    c.insert("text".into(), s(t));
    c.insert("color".into(), s(color));
    Value::Compound(c)
}

fn compound(kv: Vec<(&str, Value)>) -> Value {
    Value::Compound(kv.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// One of every block entity Lumenwright draws, on a platform in three rows facing
/// south (+Z), with the walls and ceilings they hang from.
pub fn block_entity_gallery() -> FixtureWorld {
    let mut w = FixtureWorld::new();
    for x in -2..30 {
        for z in -12..6 {
            w.set(x, 63, z, bs("stone"));
            w.set(x, 64, z, bs("smooth_stone"));
        }
    }
    let wall = |w: &mut FixtureWorld, x: i32, y: i32, z: i32| w.set(x, y, z, bs("stone_bricks"));
    let y = 65;

    // Row 1 (z = 2): signs.
    let z = 2;
    w.set(
        0,
        y,
        z,
        props("oak_sign", &[("rotation", "0"), ("waterlogged", "false")]),
    );
    w.block_entity(
        [0, y, z],
        "sign",
        vec![
            (
                "front_text",
                text_side(
                    [
                        s("Lumenwright"),
                        styled("red span", "red"),
                        s(""),
                        s("oak, front"),
                    ],
                    "black",
                    false,
                ),
            ),
            (
                "back_text",
                text_side([s(""), s(""), s(""), s("")], "black", false),
            ),
            ("is_waxed", Value::Byte(0)),
        ],
    );
    // Facing north: the camera in the south reads its back text.
    w.set(
        3,
        y,
        z,
        props(
            "spruce_sign",
            &[("rotation", "8"), ("waterlogged", "false")],
        ),
    );
    w.block_entity(
        [3, y, z],
        "sign",
        vec![
            (
                "front_text",
                text_side([s("front"), s(""), s(""), s("")], "black", false),
            ),
            (
                "back_text",
                text_side(
                    [s(""), s("the back"), s("of a sign"), s("")],
                    "white",
                    false,
                ),
            ),
        ],
    );
    wall(&mut w, 6, y, z - 1);
    w.set(
        6,
        y,
        z,
        props(
            "dark_oak_wall_sign",
            &[("facing", "south"), ("waterlogged", "false")],
        ),
    );
    w.block_entity(
        [6, y, z],
        "sign",
        vec![(
            "front_text",
            text_side([s("glowing"), s("cyan text"), s(""), s("")], "cyan", true),
        )],
    );
    wall(&mut w, 9, y, z - 1);
    w.set(
        9,
        y,
        z,
        props(
            "birch_wall_sign",
            &[("facing", "south"), ("waterlogged", "false")],
        ),
    );
    w.block_entity(
        [9, y, z],
        "sign",
        vec![(
            "front_text",
            text_side([s(""), s("glowing black"), s(""), s("")], "black", true),
        )],
    );
    for dx in -1..=1 {
        wall(&mut w, 12 + dx, y + 2, z);
    }
    w.set(
        12,
        y + 1,
        z,
        props(
            "spruce_hanging_sign",
            &[
                ("attached", "false"),
                ("rotation", "0"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity(
        [12, y + 1, z],
        "hanging_sign",
        vec![(
            "front_text",
            text_side([s("hanging"), s("ceiling"), s(""), s("")], "black", false),
        )],
    );
    w.set(
        15,
        y + 1,
        z,
        props(
            "cherry_hanging_sign",
            &[
                ("attached", "true"),
                ("rotation", "0"),
                ("waterlogged", "false"),
            ],
        ),
    );
    wall(&mut w, 15, y + 2, z);
    w.block_entity(
        [15, y + 1, z],
        "hanging_sign",
        vec![(
            "front_text",
            text_side([s("attached"), s(""), s(""), s("")], "black", false),
        )],
    );
    wall(&mut w, 17, y + 1, z);
    wall(&mut w, 19, y + 1, z);
    w.set(
        18,
        y + 1,
        z,
        props(
            "oak_wall_hanging_sign",
            &[("facing", "south"), ("waterlogged", "false")],
        ),
    );
    w.block_entity(
        [18, y + 1, z],
        "hanging_sign",
        vec![(
            "front_text",
            text_side([s("wall"), s("hanging"), s(""), s("")], "black", false),
        )],
    );

    // Row 2 (z = -3): bells, books, chests, barrel.
    let z = -3;
    w.set(
        0,
        y,
        z,
        props(
            "bell",
            &[
                ("attachment", "floor"),
                ("facing", "south"),
                ("powered", "false"),
            ],
        ),
    );
    w.block_entity([0, y, z], "bell", vec![]);
    wall(&mut w, 2, y + 1, z);
    w.set(
        2,
        y,
        z,
        props(
            "bell",
            &[
                ("attachment", "ceiling"),
                ("facing", "south"),
                ("powered", "false"),
            ],
        ),
    );
    w.block_entity([2, y, z], "bell", vec![]);
    wall(&mut w, 4, y, z - 1);
    w.set(
        4,
        y,
        z,
        props(
            "bell",
            &[
                ("attachment", "single_wall"),
                ("facing", "north"),
                ("powered", "false"),
            ],
        ),
    );
    w.block_entity([4, y, z], "bell", vec![]);
    wall(&mut w, 6, y, z);
    wall(&mut w, 8, y, z);
    w.set(
        7,
        y,
        z,
        props(
            "bell",
            &[
                ("attachment", "double_wall"),
                ("facing", "east"),
                ("powered", "false"),
            ],
        ),
    );
    w.block_entity([7, y, z], "bell", vec![]);
    w.set(
        10,
        y,
        z,
        props(
            "lectern",
            &[
                ("facing", "south"),
                ("has_book", "true"),
                ("powered", "false"),
            ],
        ),
    );
    w.block_entity([10, y, z], "lectern", vec![]);
    w.set(12, y, z, bs("enchanting_table"));
    w.block_entity([12, y, z], "enchanting_table", vec![]);
    w.set(
        14,
        y,
        z,
        props(
            "chest",
            &[
                ("facing", "south"),
                ("type", "single"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity([14, y, z], "chest", vec![]);
    // A double chest facing south: the right half is west of the left half.
    w.set(
        16,
        y,
        z,
        props(
            "chest",
            &[
                ("facing", "south"),
                ("type", "right"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.set(
        17,
        y,
        z,
        props(
            "chest",
            &[
                ("facing", "south"),
                ("type", "left"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity([16, y, z], "chest", vec![]);
    w.block_entity([17, y, z], "chest", vec![]);
    w.set(
        19,
        y,
        z,
        props(
            "trapped_chest",
            &[
                ("facing", "south"),
                ("type", "single"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity([19, y, z], "trapped_chest", vec![]);
    w.set(
        21,
        y,
        z,
        props(
            "ender_chest",
            &[("facing", "south"), ("waterlogged", "false")],
        ),
    );
    w.block_entity([21, y, z], "ender_chest", vec![]);
    w.set(
        23,
        y,
        z,
        props("barrel", &[("facing", "up"), ("open", "false")]),
    );
    w.block_entity([23, y, z], "barrel", vec![]);
    w.set(
        25,
        y,
        z,
        props(
            "copper_chest",
            &[
                ("facing", "south"),
                ("type", "single"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity([25, y, z], "copper_chest", vec![]);

    // Row 3 (z = -8): banners, pot, skulls and heads.
    let z = -8;
    let layer = |p: &str, c: &str| compound(vec![("pattern", s(p)), ("color", s(c))]);
    w.set(0, y, z, props("white_banner", &[("rotation", "0")]));
    w.block_entity(
        [0, y, z],
        "banner",
        vec![(
            "patterns",
            Value::List(vec![
                layer("minecraft:stripe_bottom", "red"),
                layer("minecraft:cross", "blue"),
                layer("minecraft:border", "black"),
            ]),
        )],
    );
    for dy in 0..3 {
        wall(&mut w, 3, y + dy, z - 1);
    }
    w.set(
        3,
        y + 1,
        z,
        props("purple_wall_banner", &[("facing", "south")]),
    );
    w.block_entity(
        [3, y + 1, z],
        "banner",
        vec![(
            "patterns",
            Value::List(vec![
                layer("minecraft:curly_border", "yellow"),
                layer("minecraft:skull", "white"),
            ]),
        )],
    );
    // A pot's front faces away from its `facing` (the placer looks along it).
    w.set(
        6,
        y,
        z,
        props(
            "decorated_pot",
            &[
                ("cracked", "false"),
                ("facing", "north"),
                ("waterlogged", "false"),
            ],
        ),
    );
    w.block_entity(
        [6, y, z],
        "decorated_pot",
        vec![(
            "sherds",
            Value::List(vec![
                s("minecraft:brick"),
                s("minecraft:heart_pottery_sherd"),
                s("minecraft:brick"),
                s("minecraft:skull_pottery_sherd"),
            ]),
        )],
    );
    let heads = [
        "skeleton_skull",
        "wither_skeleton_skull",
        "zombie_head",
        "creeper_head",
        "player_head",
        "dragon_head",
        "piglin_head",
    ];
    for (i, h) in heads.iter().enumerate() {
        let x = 8 + i as i32 * 2;
        // Skull rotation 8 faces south (a skull's rotation 0 looks north).
        w.set(
            x,
            y,
            z,
            props(h, &[("powered", "false"), ("rotation", "8")]),
        );
        let extra = if *h == "player_head" {
            vec![(
                "profile",
                compound(vec![
                    ("name", s("lineup")),
                    ("id", Value::IntArray(IntArray::new(vec![1, 2, 3, 4]))),
                ]),
            )]
        } else {
            vec![]
        };
        w.block_entity([x, y, z], "skull", extra);
    }
    wall(&mut w, 23, y + 1, z - 1);
    w.set(
        23,
        y + 1,
        z,
        props(
            "zombie_wall_head",
            &[("facing", "south"), ("powered", "false")],
        ),
    );
    w.block_entity([23, y + 1, z], "skull", vec![]);

    w.spawn = [12, 66, 8];
    w.player = Some(([13.5, 72.0, 12.0], 180.0, 25.0));
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Dimension, World};

    #[test]
    fn roundtrip_through_reader() {
        let mut fw = FixtureWorld::new();
        for x in 0..32 {
            for z in 0..32 {
                fw.set(x, 60, z, bs("stone"));
            }
        }
        fw.set(
            5,
            61,
            5,
            props(
                "oak_stairs",
                &[
                    ("facing", "east"),
                    ("half", "bottom"),
                    ("shape", "straight"),
                    ("waterlogged", "false"),
                ],
            ),
        );
        fw.set(6, 61, 5, bs("glowstone"));
        fw.entity("pig", [3.5, 61.0, 3.5], 45.0, vec![]);
        let dir = std::env::temp_dir().join(format!("lumenwright-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        fw.write(&dir).unwrap();

        let w = World::open(&dir, Dimension::Overworld).unwrap();
        assert_eq!(w.level.version_name.as_deref(), Some(VERSION_NAME));
        let a = w.load_area([0, 0], [1, 1], true).unwrap();
        let s = a.registry.state(a.volume.block(5, 61, 5));
        assert_eq!(s.name, "minecraft:oak_stairs");
        assert_eq!(s.prop("facing"), Some("east"));
        assert_eq!(
            a.registry.state(a.volume.block(19, 60, 19)).name,
            "minecraft:stone"
        );
        assert!(a.registry.state(a.volume.block(19, 61, 19)).is_air());
        // Sky light above the floor, darkness below it, block light near glowstone.
        assert_eq!(a.volume.light(10, 61, 10).0, 15);
        assert_eq!(a.volume.light(10, 59, 10).0, 0);
        assert_eq!(a.volume.light(7, 61, 5).1, 14);
        assert_eq!(a.entities.len(), 1);
        assert_eq!(a.entities[0].id, "minecraft:pig");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
