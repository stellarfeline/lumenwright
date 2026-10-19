//! Scene assembly shared by both render modes: per-state block info, tinted and
//! material-resolved quads, fluids and entities.

pub mod block_entities;
pub mod camera;
pub mod entities;
pub mod items;
pub mod mesh;
pub mod text;
pub mod voxel;

use glam::Vec3;
use lumenwright_kernel::scene::{
    ALPHA_CUTOUT, ALPHA_OPAQUE, ALPHA_TRANSLUCENT, FLAG_SHADE, FLAG_WATER, GpuQuad,
};

use crate::assets::ResourceStore;
use crate::assets::atlas::{Atlas, AtlasBuilder};
use crate::assets::biome::BiomeColors;
use crate::assets::blockinfo::{self, TintKind};
use crate::assets::blockstate::{BlockModelSet, ModelBakery};
use crate::assets::model::{BakedModel, BakedQuad, Dir};
use crate::world::{BlockState, BlockVolume, Entity, Registry, StateId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// How a surface's texels combine with what lies behind it.
pub enum Alpha {
    /// Every texel is drawn; the surface hides what is behind it.
    Opaque,
    /// Texels are either drawn or discarded by their alpha (leaves, glass panes).
    Cutout,
    /// Texels are blended over what is behind them (stained glass, ice, water).
    Translucent,
}

/// A material-resolved quad with atlas-pixel UVs.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    /// First corner (top-left of the texture), in block-local or scene coordinates.
    pub p0: Vec3,
    /// Edge from `p0` along the texture's horizontal axis, towards the top-right corner.
    pub eu: Vec3,
    /// Edge from `p0` along the texture's vertical axis, towards the bottom-left corner.
    pub ev: Vec3,
    /// Atlas pixel coordinate at `p0`.
    pub uv0: [f32; 2],
    /// Change of the atlas pixel coordinate along `eu`.
    pub uvs: [f32; 2],
    /// Change of the atlas pixel coordinate along `ev`.
    pub uvt: [f32; 2],
    /// sRGB tint (rgb), a = 255.
    pub tint: [u8; 4],
    /// How the quad's texels combine with what is behind them.
    pub alpha: Alpha,
    /// The quad is a water surface (Fresnel and refraction in HQ mode).
    pub water: bool,
    /// The quad takes vanilla's per-face directional shading.
    pub shade: bool,
    /// Block light emission / 15.
    pub emission: f32,
    /// The neighbour direction whose occluding block hides this face, if any.
    pub cullface: Option<Dir>,
    /// The model allows smooth lighting / AO.
    pub ao: bool,
}

impl Quad {
    /// Unit normal of the front face (`ev × eu`), or zero for a degenerate quad.
    pub fn normal(&self) -> Vec3 {
        self.ev.cross(self.eu).normalize_or_zero()
    }

    /// The four corners in the order top-left, bottom-left, bottom-right, top-right.
    pub fn corners(&self) -> [Vec3; 4] {
        [
            self.p0,
            self.p0 + self.ev,
            self.p0 + self.eu + self.ev,
            self.p0 + self.eu,
        ]
    }

    /// Atlas UV at the corners TL, BL, BR, TR.
    pub fn corner_uvs(&self) -> [[f32; 2]; 4] {
        let at = |s: f32, t: f32| {
            [
                self.uv0[0] + s * self.uvs[0] + t * self.uvt[0],
                self.uv0[1] + s * self.uvs[1] + t * self.uvt[1],
            ]
        };
        [at(0.0, 0.0), at(0.0, 1.0), at(1.0, 1.0), at(1.0, 0.0)]
    }

    /// The same quad moved by `by`.
    pub fn translated(mut self, by: Vec3) -> Quad {
        self.p0 += by;
        self
    }

    /// Packs the quad into the kernel's `#[repr(C)]` layout, with the alpha mode,
    /// water and shade flags, tint and emission in the spare lanes.
    pub fn to_gpu(&self) -> GpuQuad {
        let mut flags = match self.alpha {
            Alpha::Opaque => ALPHA_OPAQUE,
            Alpha::Cutout => ALPHA_CUTOUT,
            Alpha::Translucent => ALPHA_TRANSLUCENT,
        };
        if self.water {
            flags |= FLAG_WATER;
        }
        if self.shade {
            flags |= FLAG_SHADE;
        }
        GpuQuad {
            p0: [self.p0.x, self.p0.y, self.p0.z, f32::from_bits(flags)],
            eu: [
                self.eu.x,
                self.eu.y,
                self.eu.z,
                f32::from_bits(u32::from_le_bytes(self.tint)),
            ],
            ev: [self.ev.x, self.ev.y, self.ev.z, self.emission],
            uv: [self.uv0[0], self.uv0[1], self.uvs[0], self.uvs[1]],
            uvt: [self.uvt[0], self.uvt[1], 0.0, 0.0],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A fluid block and its vanilla `level` block-state property
/// (0 is a source, 1 to 7 flow away from it, 8 and above fall).
pub enum Fluid {
    /// Water at the given level.
    Water(u32),
    /// Lava at the given level.
    Lava(u32),
}

/// Everything the renderers need to know about one block state.
#[derive(Clone, Debug, Default)]
pub struct BlockInfo {
    /// The models chosen for the state (variants or multipart parts).
    pub models: BlockModelSet,
    /// Full, opaque cube: hides neighbour faces and casts AO.
    pub occludes: bool,
    /// A face is hidden by a neighbour of the same state (glass, leaves, fluids).
    pub self_cull: bool,
    /// Block light the state emits, 0 to 15.
    pub emission: u8,
    /// Which biome or fixed colour tints the state's tinted faces.
    pub tint: Option<TintKind>,
    /// The fluid the block is, if it is one.
    pub fluid: Option<Fluid>,
    /// The block also holds water (`waterlogged=true`), drawn as a water source.
    pub waterlogged: bool,
    /// The state's geometry is in vanilla's translucent render layer.
    pub translucent: bool,
    /// Nothing is drawn for the state (air, light, barrier, structure void).
    pub invisible: bool,
}

/// Names whose geometry is in the vanilla translucent render layer.
fn translucent_layer(s: &BlockState) -> bool {
    let n = s.short_name();
    matches!(
        n,
        "ice" | "frosted_ice" | "slime_block" | "honey_block" | "nether_portal" | "tinted_glass"
    ) || n.ends_with("_stained_glass")
        || n.ends_with("_stained_glass_pane")
}

/// Quick-mode lighting of one entity or block-entity quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityLight {
    /// Block light / sky light sample position (scene coords).
    pub pos: [i32; 3],
    /// Vanilla entity diffuse shading (two directional lights); text has none.
    pub shade: bool,
    /// Lit at full block and sky light whatever the cell holds (glowing sign text).
    pub full_bright: bool,
}

impl EntityLight {
    /// Lighting sampled at `pos` with entity shading and no full-bright override.
    pub fn shaded(pos: [i32; 3]) -> EntityLight {
        EntityLight {
            pos,
            shade: true,
            full_bright: false,
        }
    }
}

/// What [`Scene::build`] adds beyond the blocks.
pub struct BuildOptions {
    /// Bake entities.
    pub entities: bool,
    /// Draw an entity without a model as a box the size of its hitbox; otherwise skip it.
    pub unknown_entities_as_boxes: bool,
    /// Draw block entities whose look is Java code (signs, chests, banners...).
    pub block_entities: bool,
    /// Camera position in world coordinates, when known.
    pub eye: Option<[f64; 3]>,
    /// Camera yaw and pitch in degrees (Minecraft's convention), when known:
    /// billboarded display entities turn to it.
    pub view: Option<[f32; 2]>,
}

/// The world baked against a resource set.
pub struct Scene {
    /// Integer world origin; all scene coordinates are relative to it.
    pub origin: [i32; 3],
    /// Every block state and biome name in the loaded volume, by id.
    pub registry: Registry,
    /// The loaded blocks, light and biomes.
    pub volume: BlockVolume,
    /// Per-state render info, indexed by [`StateId`].
    pub blocks: Vec<BlockInfo>,
    /// Baked block models, indexed by model id.
    pub baked: Vec<BakedModel>,
    /// Texture atlas holding every sprite the scene samples.
    pub atlas: Atlas,
    /// Biome grass, foliage and water colours.
    pub biomes: BiomeColors,
    /// Entity geometry in scene coordinates.
    pub entity_quads: Vec<Quad>,
    /// How each entity quad is lit in quick mode.
    pub entity_quad_light: Vec<EntityLight>,
    /// Per-kind counts of the entities drawn, drawn as boxes, or skipped.
    pub entity_stats: entities::EntityStats,
    /// Per-kind counts of the block entities drawn or skipped.
    pub block_entity_stats: block_entities::BlockEntityStats,
}

/// Textures used by generated geometry and the sky, beyond those of block models.
pub const SUN_TEXTURES: &[&str] = &[
    "minecraft:environment/celestial/sun",
    "minecraft:environment/sun",
];
const EXTRA_TEXTURES: &[&str] = &[
    "minecraft:block/water_still",
    "minecraft:block/water_flow",
    "minecraft:block/lava_still",
    "minecraft:block/lava_flow",
];

impl Scene {
    /// Bakes the loaded volume, entities and block entities against `store`:
    /// resolves every block state's models, packs the atlas, decides which states
    /// occlude, and builds entity and block-entity quads as `opts` asks.
    pub fn build(
        store: &ResourceStore,
        mut registry: Registry,
        volume: BlockVolume,
        entity_list: &[Entity],
        block_entity_list: &[block_entities::BlockEntity],
        opts: &BuildOptions,
    ) -> Scene {
        let t = std::time::Instant::now();
        // Block displays render block models, so their states must be baked too.
        for s in entities::display_states(entity_list) {
            registry.intern(&s);
        }
        let mut bakery = ModelBakery::new(store);
        let mut infos: Vec<BlockInfo> = Vec::with_capacity(registry.states.len());
        for s in &registry.states {
            let invisible =
                s.is_air() || matches!(s.short_name(), "light" | "barrier" | "structure_void");
            let fluid = if blockinfo::is_water(s) {
                Some(Fluid::Water(
                    s.prop("level").and_then(|l| l.parse().ok()).unwrap_or(0),
                ))
            } else if blockinfo::is_lava(s) {
                Some(Fluid::Lava(
                    s.prop("level").and_then(|l| l.parse().ok()).unwrap_or(0),
                ))
            } else {
                None
            };
            let models = if invisible || fluid.is_some() {
                BlockModelSet::default()
            } else {
                let m = bakery.models_for_state(s);
                if m.missing {
                    log::debug!("no model for {s}");
                }
                m
            };
            infos.push(BlockInfo {
                models,
                occludes: false,
                self_cull: blockinfo::culls_same_block(s) || fluid.is_some(),
                emission: blockinfo::light_emission(s),
                tint: Some(blockinfo::tint_kind(s)),
                fluid,
                waterlogged: blockinfo::is_waterlogged(s),
                translucent: translucent_layer(s),
                invisible,
            });
        }

        let mut ab = AtlasBuilder::new();
        for t in EXTRA_TEXTURES {
            ab.add(t);
        }
        for t in SUN_TEXTURES {
            if store.resolve("textures", t, "png").is_some() {
                ab.add(t);
            }
        }
        for m in &bakery.baked {
            for q in &m.quads {
                ab.add(&q.texture);
            }
        }
        let models_json = entities::EntityModels::load_builtin();
        entities::register_textures(&models_json, entity_list, &mut ab);
        let mut item_bakery = items::ItemBakery::new(store);
        if opts.entities {
            entities::register_item_textures(&mut item_bakery, entity_list, &mut ab);
        }
        let origin = volume.min_block();
        let mut be_plan = block_entities::plan(
            &block_entities::PlanContext {
                store,
                registry: &registry,
                volume: &volume,
                origin,
                eye: opts.eye,
            },
            if opts.block_entities {
                block_entity_list
            } else {
                &[]
            },
        );
        be_plan.register_textures(&mut ab);
        let atlas = ab.build(store);

        // Occlusion needs texture transparency, so it is computed after packing.
        for info in infos.iter_mut() {
            if info.models.parts.len() == 1 && !info.models.parts[0].is_empty() && !info.translucent
            {
                info.occludes = info.models.parts[0].iter().all(|(id, _)| {
                    let m = &bakery.baked[*id as usize];
                    m.full_cube
                        && *id != ModelBakery::MISSING
                        && m.quads
                            .iter()
                            .all(|q| !atlas.sprite(&q.texture).transparent)
                });
            }
        }

        let biomes = BiomeColors::new(store);
        let mut scene = Scene {
            origin,
            registry,
            volume,
            blocks: infos,
            baked: std::mem::take(&mut bakery.baked),
            atlas,
            biomes,
            entity_quads: Vec::new(),
            entity_quad_light: Vec::new(),
            entity_stats: Default::default(),
            block_entity_stats: be_plan.stats.clone(),
        };
        if opts.entities {
            entities::build_entity_quads(
                &mut scene,
                &models_json,
                &mut item_bakery,
                entity_list,
                store,
                opts,
            );
        }
        if !be_plan.is_empty() {
            let models = block_entities::BlockEntityModels::load_builtin();
            block_entities::build_quads(&mut scene, &models, &be_plan);
        }
        log::info!(
            "scene: {} block states, {} baked models, {} entity quads ({:.2?})",
            scene.blocks.len(),
            scene.baked.len(),
            scene.entity_quads.len(),
            t.elapsed()
        );
        scene
    }

    #[inline]
    /// Render info of the block at world block coordinates `(x, y, z)`.
    pub fn info_at(&self, x: i32, y: i32, z: i32) -> &BlockInfo {
        &self.blocks[self.volume.block(x, y, z) as usize]
    }

    #[inline]
    /// Whether the block at world block coordinates `(x, y, z)` is a full opaque cube.
    pub fn occludes_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.info_at(x, y, z).occludes
    }

    fn is_waterish(&self, x: i32, y: i32, z: i32) -> bool {
        let i = self.info_at(x, y, z);
        matches!(i.fluid, Some(Fluid::Water(_))) || i.waterlogged
    }

    /// Whether the face of block `id` at `pos` towards `dir` is hidden by its neighbour.
    pub fn face_culled(&self, id: StateId, pos: [i32; 3], dir: Dir) -> bool {
        let o = dir.offset();
        let n = self
            .volume
            .block(pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]);
        let ni = &self.blocks[n as usize];
        if ni.occludes {
            return true;
        }
        let me = &self.blocks[id as usize];
        me.self_cull && n == id
    }

    fn tint_color(&self, kind: TintKind, x: i32, y: i32, z: i32) -> u32 {
        let biome = || self.registry.biomes[self.volume.biome(x, y, z) as usize].as_str();
        match kind {
            TintKind::None => 0xFFFFFF,
            TintKind::Grass => self.biomes.grass(biome()),
            TintKind::Foliage => self.biomes.foliage(biome()),
            TintKind::DryFoliage => self.biomes.dry_foliage(biome()),
            TintKind::Water => self.biomes.water(biome()),
            TintKind::Fixed(c) => c,
        }
    }

    /// Resolves a baked model quad to atlas UVs, applying `tint` when the quad is
    /// tinted, the alpha mode from its sprite (or translucent when `translucent`),
    /// and the larger of `emission` and the quad's own light emission, scaled to 0..1.
    pub fn resolve_quad(
        &self,
        q: &BakedQuad,
        tint: u32,
        ao: bool,
        translucent: bool,
        emission: u8,
    ) -> Quad {
        let sprite = self.atlas.sprite(&q.texture);
        let to = |uv: [f32; 2]| sprite.to_atlas(uv[0], uv[1], [16.0, 16.0]);
        let a = to(q.uv0);
        let b = to([q.uv0[0] + q.du[0], q.uv0[1] + q.du[1]]);
        let c = to([q.uv0[0] + q.dv[0], q.uv0[1] + q.dv[1]]);
        let tint = if q.tintindex >= 0 { tint } else { 0xFFFFFF };
        let alpha = if translucent {
            Alpha::Translucent
        } else if sprite.transparent {
            Alpha::Cutout
        } else {
            Alpha::Opaque
        };
        Quad {
            p0: q.p0,
            eu: q.eu,
            ev: q.ev,
            uv0: a,
            uvs: [b[0] - a[0], b[1] - a[1]],
            uvt: [c[0] - a[0], c[1] - a[1]],
            tint: [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8, 255],
            alpha,
            water: false,
            shade: q.shade,
            emission: emission.max(q.light_emission) as f32 / 15.0,
            cullface: q.cullface,
            ao,
        }
    }

    /// Emits the block-local quads of the block at `pos` (world coordinates).
    /// `cull(dir)` decides whether faces with that cullface are dropped.
    pub fn block_quads(
        &self,
        pos: [i32; 3],
        id: StateId,
        cull: &dyn Fn(Dir) -> bool,
        scratch: &mut Vec<u32>,
        out: &mut Vec<Quad>,
    ) {
        let info = &self.blocks[id as usize];
        if info.invisible {
            return;
        }
        let [x, y, z] = pos;
        match info.fluid {
            Some(Fluid::Water(level)) => self.fluid_quads(pos, level, true, out),
            Some(Fluid::Lava(level)) => self.fluid_quads(pos, level, false, out),
            None => {
                let tint = self.tint_color(info.tint.unwrap_or(TintKind::None), x, y, z);
                info.models.choose(x, y, z, scratch);
                let state = self.registry.state(id);
                for &mid in scratch.iter() {
                    let m = &self.baked[mid as usize];
                    for q in &m.quads {
                        if q.cullface.is_some_and(cull) {
                            continue;
                        }
                        let emission = if info.emission > 0
                            && blockinfo::emissive_texture(state, &q.texture)
                        {
                            info.emission
                        } else {
                            0
                        };
                        out.push(self.resolve_quad(
                            q,
                            tint,
                            m.ambient_occlusion,
                            info.translucent,
                            emission,
                        ));
                    }
                }
                if info.waterlogged {
                    self.fluid_quads(pos, 0, true, out);
                }
            }
        }
    }

    /// Bitmask of hidden fluid faces (bit = `Dir::index`), plus bit 7 when the
    /// block above holds the same fluid (full-height surface, no top face).
    pub fn fluid_face_mask(&self, pos: [i32; 3], water: bool) -> u8 {
        let [x, y, z] = pos;
        let same = |o: [i32; 3]| -> bool {
            if water {
                self.is_waterish(x + o[0], y + o[1], z + o[2])
            } else {
                matches!(
                    self.info_at(x + o[0], y + o[1], z + o[2]).fluid,
                    Some(Fluid::Lava(_))
                )
            }
        };
        let mut mask = 0u8;
        if same([0, 1, 0]) {
            mask |= 0x80 | (1 << Dir::Up.index());
        }
        for d in [Dir::Down, Dir::North, Dir::South, Dir::West, Dir::East] {
            let o = d.offset();
            if same(o) || self.occludes_at(x + o[0], y + o[1], z + o[2]) {
                mask |= 1 << d.index();
            }
        }
        mask
    }

    /// Generates fluid geometry (vanilla LiquidBlockRenderer, simplified to a
    /// flat surface per block).
    pub fn fluid_quads(&self, pos: [i32; 3], level: u32, water: bool, out: &mut Vec<Quad>) {
        let [x, y, z] = pos;
        let mask = self.fluid_face_mask(pos, water);
        let covered = mask & 0x80 != 0;
        let h = blockinfo::fluid_height(level, covered);
        let (still, flow) = if water {
            ("minecraft:block/water_still", "minecraft:block/water_flow")
        } else {
            ("minecraft:block/lava_still", "minecraft:block/lava_flow")
        };
        let tint = if water {
            self.tint_color(TintKind::Water, x, y, z)
        } else {
            0xFFFFFF
        };
        let emission = if water { 0 } else { 15 };
        let mk = |p0: Vec3, eu: Vec3, ev: Vec3, tex: &str, uv: [f32; 4], dir: Dir| -> Quad {
            let bq = BakedQuad {
                p0,
                eu,
                ev,
                texture: tex.to_string(),
                uv0: [uv[0], uv[1]],
                du: [uv[2] - uv[0], 0.0],
                dv: [0.0, uv[3] - uv[1]],
                cullface: Some(dir),
                tintindex: 0,
                shade: true,
                light_emission: 0,
            };
            let mut q = self.resolve_quad(&bq, tint, true, water, emission);
            q.water = water;
            q.alpha = if water {
                Alpha::Translucent
            } else {
                Alpha::Opaque
            };
            q
        };
        let shown = |d: Dir| mask & (1 << d.index()) == 0;
        if shown(Dir::Up) {
            out.push(mk(
                Vec3::new(0.0, h, 0.0),
                Vec3::X,
                Vec3::Z,
                still,
                [0.0, 0.0, 16.0, 16.0],
                Dir::Up,
            ));
        }
        if shown(Dir::Down) {
            out.push(mk(
                Vec3::new(0.0, 0.0, 1.0),
                Vec3::X,
                -Vec3::Z,
                still,
                [0.0, 0.0, 16.0, 16.0],
                Dir::Down,
            ));
        }
        let side_uv = [0.0, (1.0 - h) * 8.0, 8.0, 8.0];
        let down = Vec3::new(0.0, -h, 0.0);
        if shown(Dir::North) {
            out.push(mk(
                Vec3::new(1.0, h, 0.0),
                -Vec3::X,
                down,
                flow,
                side_uv,
                Dir::North,
            ));
        }
        if shown(Dir::South) {
            out.push(mk(
                Vec3::new(0.0, h, 1.0),
                Vec3::X,
                down,
                flow,
                side_uv,
                Dir::South,
            ));
        }
        if shown(Dir::East) {
            out.push(mk(
                Vec3::new(1.0, h, 1.0),
                -Vec3::Z,
                down,
                flow,
                side_uv,
                Dir::East,
            ));
        }
        if shown(Dir::West) {
            out.push(mk(
                Vec3::new(0.0, h, 0.0),
                Vec3::Z,
                down,
                flow,
                side_uv,
                Dir::West,
            ));
        }
    }
}
