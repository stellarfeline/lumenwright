//! The library entry point: one frame from an explicit [`Request`].
//!
//! [`render()`] takes every input from the request. It resolves no client jar,
//! performs no network access, writes no file and prints nothing (it emits
//! `log` records, which reach no output unless the caller installs a logger).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use glam::{DVec3, Vec3};
use lumenwright_kernel::Params;
use lumenwright_kernel::scene::{
    EMITTER_SAMPLING_ALL, EMITTER_SAMPLING_NONE, EMITTER_SAMPLING_ONE, EMITTER_SAMPLING_ONE_BLOCK,
};

use crate::assets;
use crate::render::post::{self, Tonemap};
use crate::render::sky::{self, HqSky, QuickSky};
use crate::render::{self, BackendKind, CameraRays, HqFrame, Mode, QuickFrame};
use crate::scene::block_entities::{self, BlockEntityStats};
use crate::scene::camera::Camera;
use crate::scene::entities::EntityStats;
use crate::scene::mesh::{self, Lighting, MeshParams};
use crate::scene::{BuildOptions, Scene, voxel};
use crate::world::{Dimension, Entity, World};

/// Where the blocks come from.
#[derive(Clone, Copy)]
pub enum WorldSource<'a> {
    /// A save directory (the folder holding `level.dat`) and the dimension to read.
    Dir {
        /// The save directory.
        path: &'a Path,
        /// The dimension whose regions are read.
        dimension: Dimension,
    },
    /// A world the caller has already opened.
    Opened(&'a World),
}

/// An inclusive rectangle of chunk coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRect {
    /// The lowest chunk `[x, z]`, included.
    pub min: [i32; 2],
    /// The highest chunk `[x, z]`, included.
    pub max: [i32; 2],
}

impl ChunkRect {
    /// The square of chunks of `radius` (at least 1) around the chunk holding `pos`.
    pub fn around(pos: DVec3, radius: i32) -> ChunkRect {
        let c = [(pos.x.floor() as i32) >> 4, (pos.z.floor() as i32) >> 4];
        let r = radius.max(1);
        ChunkRect {
            min: [c[0] - r, c[1] - r],
            max: [c[0] + r, c[1] + r],
        }
    }
}

/// How entities without a model are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownEntities {
    /// A box the size of the entity's hitbox.
    Box,
    /// Nothing; the kind is counted in [`EntityStats::skipped`].
    Skip,
}

/// Which bodies are drawn.
#[derive(Clone, Copy, Debug)]
pub struct EntityPolicy {
    /// Draw the entities stored in the loaded chunks.
    pub entities: bool,
    /// How an entity with no model is drawn.
    pub unknown: UnknownEntities,
    /// Draw block entities whose look is Java code (signs, chests, banners...).
    pub block_entities: bool,
    /// Also draw the players of `level.dat` and `playerdata` (needs `entities`).
    pub players: bool,
}

impl Default for EntityPolicy {
    fn default() -> Self {
        EntityPolicy {
            entities: true,
            unknown: UnknownEntities::Box,
            block_entities: true,
            players: false,
        }
    }
}

/// Quick-mode options.
#[derive(Clone, Copy, Debug)]
pub struct QuickOptions {
    /// Supersampling factor, 1 to 4: renders at N× and box-filters down.
    pub ssaa: u32,
    /// Blocks drawn around the camera; the fog closes at this distance.
    pub draw_distance: f32,
    /// Vanilla brightness option: 0 = moody, 0.5 = default, 1 = bright.
    pub brightness: f32,
    /// Smooth lighting and ambient occlusion.
    pub smooth: bool,
    /// Mip levels for block textures, 0 (off) to 4: the client's *Mipmap Levels*
    /// option, default 4.
    pub mipmap_levels: u32,
}

impl Default for QuickOptions {
    fn default() -> Self {
        QuickOptions {
            ssaa: 1,
            draw_distance: 128.0,
            brightness: 0.5,
            smooth: true,
            mipmap_levels: crate::assets::atlas::MIPMAP_LEVELS,
        }
    }
}

/// Chunky's default emitter intensity (`Scene.DEFAULT_EMITTER_INTENSITY`,
/// chunky-dev/chunky 156e2bb): the emitter intensity of Chunky's look.
pub const DEFAULT_EMITTER_INTENSITY: f32 = 13.0;

/// The emitter intensity of the game's look: **authored by Lumenwright**, neither a
/// vanilla nor a Chunky value. Fitted so that, at the Treehouse Camp's dusk (tick 12800,
/// the default look), the mean display luma of the lantern-lit pixels equals that of
/// Delvewright's engine's Chunky render of the same frame (`docs/research/game-sky.md`,
/// *Lanterns*). Chunky's emitters light little beyond their block where the game's block
/// light reaches 14; the scale stands in for that, as the engine's does.
pub const GAME_EMITTER_INTENSITY: f32 = 13.0;

/// Chunky's emitter sampling strategies (`EmitterSamplingStrategy`): which emitters a
/// diffuse surface samples, among those its emitter-grid cell lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmitterSampling {
    /// None: emitters light the scene only where a path meets them.
    None,
    /// One face of one emitter.
    One,
    /// Every face of one emitter.
    OneBlock,
    /// Every face of every emitter.
    All,
}

/// HQ-mode options: the sampling budget and the post-processing.
#[derive(Clone, Copy, Debug)]
pub struct HqOptions {
    /// Samples per pixel.
    pub spp: u32,
    /// Diffuse bounces per path after the first hit.
    pub max_bounces: u32,
    /// Random seed; the same seed gives the same image.
    pub seed: u32,
    /// Chunky's emitter intensity: an emitting surface emits its texel colour squared
    /// times its block's emittance times this. 0 turns emitters off. `None` takes each
    /// look's own, mixed by the day weight: Chunky's [`DEFAULT_EMITTER_INTENSITY`] and,
    /// under the game's sky, [`GAME_EMITTER_INTENSITY`].
    pub emitter_intensity: Option<f32>,
    /// How emitters are sampled at a diffuse surface (Chunky's emitter sampling
    /// strategy).
    pub emitter_sampling: EmitterSampling,
    /// While emitters are sampled, a surface met after the first bounce does not add its
    /// emitted light (Chunky's "prevent normal emitter with sampling").
    pub prevent_normal_emitter: bool,
    /// Linear multiplier applied to the radiance before tonemapping.
    pub exposure: f32,
    /// The tonemapping operator from linear radiance to display values. `None` takes each
    /// look's own, mixed in display values by the day weight: Chunky's [`Tonemap::Gamma`]
    /// and, under the game's sky, [`Tonemap::FilmicMidGrey`].
    pub tonemap: Option<Tonemap>,
}

impl Default for HqOptions {
    fn default() -> Self {
        HqOptions {
            spp: 64,
            max_bounces: 4,
            seed: 0,
            emitter_intensity: None,
            emitter_sampling: EmitterSampling::One,
            prevent_normal_emitter: false,
            exposure: 1.0,
            tonemap: None,
        }
    }
}

/// The sky and the daylight a frame is drawn under.
#[derive(Clone, Copy, Debug)]
pub enum Sky {
    /// Lumenwright derives the vanilla sky from a time of day.
    Vanilla {
        /// Ticks (0 = sunrise, 6000 = noon); `None` is the world's `DayTime`.
        time: Option<i64>,
        /// Unit vector towards the sun; `None` follows `time`. Under
        /// [`HqSkyModel::Game`] it moves only the sun drawn in the sky.
        sun_dir: Option<Vec3>,
        /// What HQ mode lights the frame with and draws in its sky.
        hq: HqSkyModel,
    },
    /// The caller's sky, used as given; nothing is derived from the world's time.
    Stated(StatedSky),
}

impl Default for Sky {
    fn default() -> Self {
        Sky::Vanilla {
            time: None,
            sun_dir: None,
            hq: HqSkyModel::default(),
        }
    }
}

/// The client's default render distance, 12 chunks (`options.renderDistance`), in blocks.
pub const DEFAULT_RENDER_DISTANCE: f32 = 192.0;

/// Which look HQ mode draws under [`Sky::Vanilla`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyLook {
    /// By the time of day: Chunky's look on the client's day plateau, the game's at night,
    /// and between them a mix weighted by where the client's `visual/sky_light_factor`
    /// stands between its night and day values ([`sky::day_weight`]).
    ByTime,
    /// The game's own sky at the tick, lit as the 1.21.11 client lights it
    /// ([`sky::game_sky_maps`]): the lens sees the client's sky (its disc, fog, sunrise
    /// glow, sun, moon and stars); the sun casts no light; the sky casts the lightmap's
    /// colour for a cell open to it. Needs a client jar with environment attributes
    /// (1.21.11 or later).
    Game,
    /// Chunky's simulated sky: the Preetham sky and a sun at the time's angle that lights
    /// and shadows, as Chunky lights a scene ([`sky::hq_sky`]).
    Chunky,
}

/// The sky HQ mode traces under [`Sky::Vanilla`]: the look and each look's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HqSkyModel {
    /// Which look, or a mix of the two by the time of day.
    pub look: SkyLook,
    /// The client's render distance in blocks, which the game's sky fog and its
    /// horizon colour depend on.
    pub render_distance: f32,
    /// Chunky's sun intensity: its irradiance is `intensity^2.2`.
    pub sun_intensity: f32,
    /// Chunky's Preetham sky radiance scale.
    pub sky_intensity: f32,
}

impl Default for HqSkyModel {
    fn default() -> Self {
        HqSkyModel {
            look: SkyLook::ByTime,
            render_distance: DEFAULT_RENDER_DISTANCE,
            sun_intensity: sky::DEFAULT_SUN_INTENSITY,
            sky_intensity: 1.0,
        }
    }
}

impl HqSkyModel {
    /// Chunky's look with Chunky's defaults: a sun of intensity 1.25 and the sky at its own
    /// scale.
    pub const CHUNKY: HqSkyModel = HqSkyModel {
        look: SkyLook::Chunky,
        render_distance: DEFAULT_RENDER_DISTANCE,
        sun_intensity: sky::DEFAULT_SUN_INTENSITY,
        sky_intensity: 1.0,
    };

    /// The game's look at the client's default render distance.
    pub const GAME: HqSkyModel = HqSkyModel {
        look: SkyLook::Game,
        render_distance: DEFAULT_RENDER_DISTANCE,
        sun_intensity: sky::DEFAULT_SUN_INTENSITY,
        sky_intensity: 1.0,
    };
}

/// A caller-stated sky for both modes.
#[derive(Clone, Copy, Debug)]
pub struct StatedSky {
    /// The sky quick mode draws.
    pub quick: QuickSkyColours,
    /// The sun and sky HQ mode traces.
    pub hq: HqSky,
}

/// Quick mode's sky: the colours [`QuickSky`] draws and the lightmap's daylight.
#[derive(Clone, Copy, Debug)]
pub struct QuickSkyColours {
    /// Zenith colour, sRGB 0..1.
    pub sky: Vec3,
    /// Fog and horizon colour, sRGB 0..1.
    pub fog: Vec3,
    /// Unit vector towards the sun (places the sun sprite).
    pub sun_dir: Vec3,
    /// [`Lighting::sky_factor`]: 1 at noon, about 0.2 at midnight.
    pub sky_factor: f32,
}

/// One frame to draw. Every input is explicit.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// The world to draw.
    pub world: WorldSource<'a>,
    /// The chunks loaded; nothing outside them is drawn.
    pub chunks: ChunkRect,
    /// The Minecraft client jar the textures and models are read from.
    pub jar: &'a Path,
    /// The directory holding the Unifont the client reads from the launcher's asset
    /// index (`minecraft/font/include/unifont.json` and the hex zips it names; see
    /// [`assets::unifont`]). Without it, a character no bitmap font covers (CJK among
    /// them) draws nothing. Layered over the jar and under the resource packs.
    pub unifont: Option<&'a Path>,
    /// Resource packs (zip or directory) layered over the jar, later ones winning.
    pub resource_packs: &'a [PathBuf],
    /// The eye position, orientation and vertical field of view.
    pub camera: Camera,
    /// Quick (rasterized, vanilla look) or HQ (path traced).
    pub mode: Mode,
    /// The backend to draw with; [`BackendKind::Auto`] takes a GPU if one is available, else the CPU.
    pub backend: BackendKind,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Which entities and block entities are drawn.
    pub entities: EntityPolicy,
    /// Options used in quick mode; ignored in HQ mode.
    pub quick: QuickOptions,
    /// Options used in HQ mode; ignored in quick mode.
    pub hq: HqOptions,
    /// The sky and daylight.
    pub sky: Sky,
}

/// A drawn frame.
pub struct Rendered {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// RGBA8, sRGB, row major, top row first: `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// HQ mode: the linear radiance per pixel, in the same order. `None` in quick mode.
    pub hdr: Option<Vec<[f32; 3]>>,
    /// The backend that drew the frame, e.g. `cpu (10 threads)`.
    pub backend: String,
    /// Set when [`BackendKind::Auto`] took the CPU: why no GPU was used.
    pub backend_fallback: Option<String>,
    /// Seconds per stage, in order.
    pub timings: Vec<(&'static str, f64)>,
    /// Chunks of the rectangle that exist in the save and were loaded.
    pub chunks_loaded: usize,
    /// Entities handed to the scene (stored ones plus players).
    pub entities_loaded: usize,
    /// Block entities found in the loaded chunks.
    pub block_entities_loaded: usize,
    /// Entity counts per kind: drawn with a model, drawn as a box, or skipped.
    pub entity_stats: EntityStats,
    /// What happened to each block-entity kind.
    pub block_entity_stats: BlockEntityStats,
}

/// A request [`render()`] refuses before drawing. Returned inside the
/// `anyhow::Error`; `err.downcast_ref::<RequestError>()` names it.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestError {
    /// Width or height is zero.
    EmptyImage {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },
    /// The client jar path is not a file.
    JarNotFound(PathBuf),
    /// A resource pack path does not exist.
    ResourcePackNotFound(PathBuf),
    /// The unifont directory lacks a file, or a file is not what the client reads.
    Unifont(assets::unifont::UnifontError),
    /// `chunks.min` exceeds `chunks.max` on an axis.
    InvertedChunkRect(ChunkRect),
    /// None of the requested chunks exists in the save.
    NoChunks {
        /// The requested rectangle.
        chunks: ChunkRect,
        /// The region directory that was searched.
        region_dir: PathBuf,
    },
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::EmptyImage { width, height } => {
                write!(f, "image size must be positive, got {width}x{height}")
            }
            RequestError::JarNotFound(p) => {
                write!(f, "client jar {} does not exist", p.display())
            }
            RequestError::ResourcePackNotFound(p) => {
                write!(f, "resource pack {} does not exist", p.display())
            }
            RequestError::Unifont(e) => e.fmt(f),
            RequestError::InvertedChunkRect(r) => {
                write!(f, "chunk rectangle min {:?} exceeds max {:?}", r.min, r.max)
            }
            RequestError::NoChunks { chunks, region_dir } => write!(
                f,
                "no chunks found in chunks x {}..={}, z {}..={} of {}",
                chunks.min[0],
                chunks.max[0],
                chunks.min[1],
                chunks.max[1],
                region_dir.display()
            ),
        }
    }
}

impl std::error::Error for RequestError {}

fn validate(req: &Request) -> std::result::Result<(), RequestError> {
    if req.width == 0 || req.height == 0 {
        return Err(RequestError::EmptyImage {
            width: req.width,
            height: req.height,
        });
    }
    if !req.jar.is_file() {
        return Err(RequestError::JarNotFound(req.jar.to_path_buf()));
    }
    if let Some(p) = req.resource_packs.iter().find(|p| !p.exists()) {
        return Err(RequestError::ResourcePackNotFound(p.clone()));
    }
    if let Some(dir) = req.unifont {
        assets::unifont::read_dir(dir).map_err(RequestError::Unifont)?;
    }
    let c = req.chunks;
    if c.min[0] > c.max[0] || c.min[1] > c.max[1] {
        return Err(RequestError::InvertedChunkRect(c));
    }
    Ok(())
}

/// Draws one frame. See the [module documentation](self) for what it never does.
pub fn render(req: &Request) -> Result<Rendered> {
    validate(req)?;
    let mut timings = Vec::new();
    let mut t = Instant::now();
    let mut lap = |name: &'static str, t: &mut Instant| {
        timings.push((name, t.elapsed().as_secs_f64()));
        *t = Instant::now();
    };
    let opened;
    let world = match req.world {
        WorldSource::Opened(w) => w,
        WorldSource::Dir { path, dimension } => {
            opened = World::open(path, dimension)?;
            &opened
        }
    };
    let cam = req.camera;

    // 1. World.
    let policy = req.entities;
    let mut area = world.load_area(req.chunks.min, req.chunks.max, policy.entities)?;
    if area.chunks_loaded == 0 {
        return Err(RequestError::NoChunks {
            chunks: req.chunks,
            region_dir: world.region_dir(),
        }
        .into());
    }
    let mut entities: Vec<Entity> = std::mem::take(&mut area.entities);
    if policy.players && policy.entities {
        let mut players: Vec<_> = world.player().cloned().into_iter().collect();
        players.extend(world.other_players());
        for p in players {
            let mut nbt = crate::world::nbt::Compound::new();
            nbt.insert(
                "id".into(),
                fastnbt::Value::String("minecraft:player".into()),
            );
            entities.push(Entity {
                id: "minecraft:player".into(),
                pos: p.pos,
                yaw: p.yaw,
                pitch: p.pitch,
                nbt,
            });
        }
    }
    log::info!(
        "loaded {} chunks, {} entities",
        area.chunks_loaded,
        entities.len()
    );
    lap("load world", &mut t);

    // 2. Assets.
    let store = assets::load_resources(req.jar, req.unifont, req.resource_packs)?;
    lap("load assets", &mut t);

    // 3. Scene.
    let opts = BuildOptions {
        entities: policy.entities,
        unknown_entities_as_boxes: policy.unknown == UnknownEntities::Box,
        block_entities: policy.block_entities,
        eye: Some([cam.pos.x, cam.pos.y, cam.pos.z]),
        view: Some([cam.yaw, cam.pitch]),
        // The path tracer reads level 0 only: it gets the unpadded atlas.
        mipmap_levels: match req.mode {
            Mode::Quick => req.quick.mipmap_levels,
            Mode::Hq => 0,
        },
    };
    let block_entities = block_entities::parse_all(&area.block_entities);
    let scene = Scene::build(
        &store,
        area.registry,
        area.volume,
        &entities,
        &block_entities,
        &opts,
    );
    lap("build scene", &mut t);

    let render::OpenedBackend {
        backend: mut be,
        fallback,
    } = render::open_backend(req.backend)?;
    log::info!("backend: {}", be.name());
    let eye = Vec3::new(
        (cam.pos.x - scene.origin[0] as f64) as f32,
        (cam.pos.y - scene.origin[1] as f64) as f32,
        (cam.pos.z - scene.origin[2] as f64) as f32,
    );
    let (f, r, u) = cam.basis();
    let aspect = req.width as f32 / req.height as f32;
    let rays = CameraRays {
        forward: f,
        right: r,
        up: u,
        tan_half_fov: (cam.fov_y.to_radians() * 0.5).tan(),
        aspect,
    };
    // The environment attributes at the camera: the sky's colours and the sun's,
    // moon's and stars' angles for the day time.
    let biome = scene.registry.biomes[scene.volume.biome(
        cam.pos.x.floor() as i32,
        cam.pos.y.floor() as i32,
        cam.pos.z.floor() as i32,
    ) as usize]
        .clone();
    let dimension = match world.dimension {
        Dimension::Overworld => "minecraft:overworld",
        Dimension::Nether => "minecraft:the_nether",
        Dimension::End => "minecraft:the_end",
    };
    let day_time = match req.sky {
        Sky::Vanilla { time, .. } => time.unwrap_or(world.level.day_time),
        Sky::Stated(_) => world.level.day_time,
    };
    let env = assets::environment::sample(&store, dimension, &biome, day_time);
    // The night and day values of `visual/sky_light_factor` over the day timeline.
    let day_range = assets::environment::sky_light_factor_range(&store, dimension, &biome);
    if env.is_none() {
        log::warn!(
            "the client jar carries no environment attributes (before 1.21.11): \
             drawing the sky from the biome's temperature and the old day curve"
        );
    }
    // The vanilla sun when the caller states none: `visual/sun_angle`, or the
    // pre-1.21.11 day curve for a jar without environment attributes.
    let vanilla_sun = |time: Option<i64>, sun: Option<Vec3>| {
        sun.unwrap_or_else(|| match &env {
            Some(e) => sky::celestial_direction(e.sun_angle),
            None => sky::sun_direction_from_angle(sky::celestial_angle(
                time.unwrap_or(world.level.day_time),
            )),
        })
    };

    let (pixels, hdr) = match req.mode {
        Mode::Quick => {
            let q = req.quick;
            let k = q.ssaa.clamp(1, 4);
            let (w, h) = (req.width * k, req.height * k);
            let max_dist = q.draw_distance;
            let view_proj =
                cam.projection(aspect, 0.05, max_dist * 2.0 + 512.0) * cam.view(Vec3::ZERO);
            let rect = |loc: &str| {
                scene.atlas.id(loc).map(|id| {
                    let s = scene.atlas.sprites[id as usize];
                    [s.x as f32, s.y as f32, s.w as f32, s.h as f32]
                })
            };
            let sun_rect = crate::scene::SUN_TEXTURES.iter().find_map(|t| rect(t));
            let render_fog = (max_dist - (max_dist / 10.0).clamp(4.0, 64.0), max_dist);
            let (colours, desc) = quick_sky(
                &req.sky,
                env.as_ref(),
                &biome,
                world.level.day_time,
                f,
                max_dist,
                render_fog,
                sun_rect,
                &rect,
            );
            let lighting = Lighting {
                sky_factor: colours.sky_factor,
                gamma: q.brightness.clamp(0.0, 1.0),
                smooth: q.smooth,
            };
            let mesh = mesh::build_mesh(
                &scene,
                &MeshParams {
                    eye,
                    view_proj,
                    max_distance: max_dist,
                    lighting,
                },
            );
            lap("build mesh", &mut t);
            let qs = QuickSky::new(&rays, w, h, &desc);
            let frame = QuickFrame {
                mesh: &mesh,
                atlas_pixels: &scene.atlas.pixels,
                atlas_size: [scene.atlas.width, scene.atlas.height],
                atlas_mips: &scene.atlas.mips,
                mip_levels: scene.atlas.mip_levels,
                mip_rows: scene.atlas.mip_rows,
                stars: sky::star_table(),
                width: w,
                height: h,
                view_proj,
                sky: qs,
            };
            let px = be.render_quick(&frame)?;
            lap("render", &mut t);
            (post::downsample(&px, w, h, k), None)
        }
        Mode::Hq => {
            let hq = voxel::build_hq(&scene)?;
            lap("build voxel scene", &mut t);
            // How much of the frame is Chunky's look: 1 for `Chunky` and a stated sky, 0 for
            // `Game`, and by the time of day for `ByTime` (`sky::day_weight`).
            let (model, weight) = match req.sky {
                Sky::Stated(_) => (None, 1.0),
                Sky::Vanilla { hq: m, .. } => {
                    let w = match m.look {
                        SkyLook::Chunky => 1.0,
                        SkyLook::Game => 0.0,
                        SkyLook::ByTime => match (&env, day_range) {
                            (Some(e), Some((night, day))) => {
                                sky::day_weight(e.sky_light_factor, night, day)
                            }
                            _ => {
                                log::warn!(
                                    "no day timeline for `visual/sky_light_factor` in this jar: \
                                     drawing Chunky's sky"
                                );
                                1.0
                            }
                        },
                    };
                    (Some(m), w)
                }
            };
            if weight < 1.0 && env.is_none() {
                anyhow::bail!(
                    "the game's sky needs the environment attributes of a 1.21.11 or later client \
                     jar, and this jar has none; draw Chunky's sky instead (SkyLook::Chunky, \
                     `--sky chunky`)"
                );
            }
            let emitter_intensity = req.hq.emitter_intensity.unwrap_or(
                weight * DEFAULT_EMITTER_INTENSITY + (1.0 - weight) * GAME_EMITTER_INTENSITY,
            );
            // Chunky's sun and sky, for the Chunky part of the frame.
            let chunky_sky = match (req.sky, model) {
                (Sky::Stated(s), _) => s.hq,
                (Sky::Vanilla { time, sun_dir, .. }, Some(m)) => {
                    sky::hq_sky(vanilla_sun(time, sun_dir), m.sun_intensity, m.sky_intensity)
                }
                (Sky::Vanilla { .. }, None) => unreachable!("a vanilla sky has a model"),
            };
            let mut params = hq_params(
                &req.hq,
                emitter_intensity,
                req.width,
                req.height,
                &hq,
                eye,
                &rays,
                &chunky_sky,
            );
            let sky_buffer = if weight >= 1.0 {
                let d = chunky_sky.sun_dir;
                log::info!(
                    "hq sky: Chunky's (day weight 1); sun direction {:.6} {:.6} {:.6}, altitude \
                     {:.6} rad, azimuth from +x towards +z {:.6} rad (Chunky's sun angles); \
                     emitter intensity {emitter_intensity}",
                    d.x,
                    d.y,
                    d.z,
                    d.y.clamp(-1.0, 1.0).asin(),
                    d.z.atan2(d.x)
                );
                sky::sky_cache(&params)
            } else {
                let e = env.as_ref().expect("checked above");
                let render_distance = model.map_or(DEFAULT_RENDER_DISTANCE, |m| m.render_distance);
                let rect = |loc: &str| {
                    scene.atlas.id(loc).map(|id| {
                        let s = scene.atlas.sprites[id as usize];
                        [s.x as f32, s.y as f32, s.w as f32, s.h as f32]
                    })
                };
                let sun_rect = crate::scene::SUN_TEXTURES.iter().find_map(|t| rect(t));
                let render_fog = (
                    render_distance - (render_distance / 10.0).clamp(4.0, 64.0),
                    render_distance,
                );
                let (_, desc) = quick_sky(
                    &req.sky,
                    Some(e),
                    &biome,
                    world.level.day_time,
                    f,
                    render_distance,
                    render_fog,
                    sun_rect,
                    &rect,
                );
                // What is drawn in the sky casts no light in the game.
                let bare = sky::SkyDescription {
                    sun: None,
                    moon: None,
                    star_brightness: 0.0,
                    ..desc
                };
                let mut maps = sky::game_sky_maps(
                    &QuickSky::new(&rays, req.width, req.height, &desc),
                    &QuickSky::new(&rays, req.width, req.height, &bare),
                    e.sky_light_factor,
                    rgb(e.sky_light_color),
                    &scene.atlas.pixels,
                    scene.atlas.width,
                    sky::star_table(),
                );
                log::info!(
                    "hq sky: day weight {weight:.6} (sky light factor {:.6} between the \
                     timeline's night {:.6} and day {:.6}); the game's at day time {day_time}: \
                     sky light colour #{:06x}, light {:.6} {:.6} {:.6} (the lightmap's, linear) \
                     = {:.6} x (the sky's mean {:.6} {:.6} {:.6} + veil {:.6} {:.6} {:.6}); \
                     sun {}; emitter intensity {emitter_intensity}",
                    e.sky_light_factor,
                    day_range.map_or(f32::NAN, |r| r.0),
                    day_range.map_or(f32::NAN, |r| r.1),
                    e.sky_light_color & 0xFF_FFFF,
                    maps.light.x,
                    maps.light.y,
                    maps.light.z,
                    maps.scale,
                    maps.mean.x,
                    maps.mean.y,
                    maps.mean.z,
                    maps.veil.x,
                    maps.veil.y,
                    maps.veil.z,
                    match desc.sun {
                        Some((d, _)) =>
                            format!("drawn at altitude {:.6} rad", d.y.clamp(-1.0, 1.0).asin()),
                        None => "not drawn".to_string(),
                    }
                );
                if weight > 0.0 {
                    // Chunky's sky, as the kernel reads it through its cache, weighted into
                    // both maps; Chunky's sun lights and is drawn at the same weight.
                    let cache = sky::sky_cache(&params);
                    let view = lumenwright_kernel::SceneView {
                        params: &params,
                        grid: &[],
                        models: &[],
                        quads: &[],
                        atlas: &[],
                        emitters: &[],
                        emitter_grid: &[],
                        sky: &cache,
                    };
                    sky::blend_chunky_sky(&mut maps, weight, |d| {
                        let c = lumenwright_kernel::integrator::sky_radiance(
                            &view,
                            lumenwright_kernel::math::v3(d.x, d.y, d.z),
                        );
                        Vec3::new(c.x, c.y, c.z)
                    });
                    for c in &mut params.sun_color[..3] {
                        *c *= weight;
                    }
                    params.sun_su[3] *= weight;
                } else {
                    params.sun_color = [0.0, 0.0, 0.0, params.sun_color[3]];
                    // The lens's map draws the client's sun; Chunky's is not drawn.
                    params.sun_sprite = [0.0; 4];
                }
                params.sky_map = maps.sizes;
                params.misc_f[2] = 0.0;
                maps.buffer
            };
            let frame = HqFrame {
                scene: &hq,
                params,
                spp: req.hq.spp,
                sky: sky_buffer,
            };
            lap("sky", &mut t);
            let started = Instant::now();
            let progress = |done: u32, total: u32| {
                log::debug!("{done}/{total} ({:.1?})", started.elapsed());
            };
            let hdr = be.render_hq(&frame, &progress)?;
            lap("render", &mut t);
            let px = match req.hq.tonemap {
                Some(op) => post::tonemap(&hdr, req.hq.exposure, op),
                // Each look's own curve, mixed in display values by the day weight.
                None => post::tonemap_mix(
                    &hdr,
                    req.hq.exposure,
                    Tonemap::Gamma,
                    Tonemap::FilmicMidGrey,
                    weight,
                ),
            };
            (px, Some(hdr))
        }
    };
    Ok(Rendered {
        width: req.width,
        height: req.height,
        rgba: pixels.iter().flat_map(|c| c.to_le_bytes()).collect(),
        hdr,
        backend: be.name(),
        backend_fallback: fallback,
        timings,
        chunks_loaded: area.chunks_loaded,
        entities_loaded: entities.len(),
        block_entities_loaded: block_entities.len(),
        entity_stats: scene.entity_stats.clone(),
        block_entity_stats: scene.block_entity_stats.clone(),
    })
}

/// `ARGB` channel `i` (0 = red) of `0xAARRGGBB` as 0..1.
fn channel(c: u32, shift: u32) -> f32 {
    ((c >> shift) & 0xFF) as f32 / 255.0
}

fn rgb(c: u32) -> Vec3 {
    Vec3::new(channel(c, 16), channel(c, 8), channel(c, 0))
}

/// The quick sky for a frame: the lightmap's colours and what the sky passes draw.
///
/// For [`Sky::Vanilla`] with the jar's environment attributes, the fog colour is
/// `AtmosphericFogEnvironment.getBaseColor`: the fog attribute, blended towards
/// the sunrise colour by how directly the camera faces the sun's side of the sky
/// (render distance 4 chunks or more), then towards the sky colour by
/// `1 - lerp(min(sky fog end / 16, render distance) / 32, 0.25, 1)^0.25`.
#[allow(clippy::too_many_arguments)]
fn quick_sky(
    req_sky: &Sky,
    env: Option<&assets::environment::EnvironmentState>,
    biome: &str,
    world_day_time: i64,
    forward: Vec3,
    draw_distance: f32,
    render_fog: (f32, f32),
    sun_rect: Option<[f32; 4]>,
    rect: &dyn Fn(&str) -> Option<[f32; 4]>,
) -> (QuickSkyColours, sky::SkyDescription) {
    use assets::environment::{Skybox, srgb_lerp};
    let env_fog = env.map_or((0.0, 1024.0), |e| (e.fog_start, e.fog_end));
    let sky_fog_end = draw_distance.min(env.map_or(512.0, |e| e.sky_fog_end));
    let mut desc = sky::SkyDescription {
        sky: Vec3::ZERO,
        fog: Vec3::ZERO,
        sky_fog_end,
        render_fog,
        env_fog,
        sky_disc: env.is_none_or(|e| e.skybox == Skybox::Overworld),
        sunrise: [0.0; 4],
        sun_angle: 0.0,
        sun: None,
        moon: None,
        star_angle: 0.0,
        star_brightness: 0.0,
    };
    match *req_sky {
        Sky::Stated(s) => {
            desc.sky = s.quick.sky;
            desc.fog = s.quick.fog;
            desc.sun = sun_rect.map(|r| (s.quick.sun_dir, r));
            (s.quick, desc)
        }
        Sky::Vanilla { time, sun_dir, .. } => {
            let ticks = time.unwrap_or(world_day_time);
            // The lightmap keeps its daylight factor on the pre-1.21.11 day curve.
            let angle = sky::celestial_angle(ticks);
            let day = sky::daylight(angle);
            let Some(e) = env else {
                let temp = assets::biome::biome_info(biome).temperature;
                let sun = sun_dir.unwrap_or_else(|| sky::sun_direction_from_angle(angle));
                let colours = QuickSkyColours {
                    sky: Vec3::from(assets::biome::sky_color(temp)) * day,
                    fog: Vec3::new(0.753, 0.847, 1.0) * (day * 0.94 + 0.06),
                    sun_dir: sun,
                    sky_factor: day,
                };
                desc.sky = colours.sky;
                desc.fog = colours.fog;
                desc.sun = sun_rect.map(|r| (sun, r));
                return (colours, desc);
            };
            let render_chunks = (draw_distance / 16.0).floor();
            let mut fog = e.fog_color;
            if render_chunks >= 4.0 {
                let side = if e.sun_angle.to_radians().sin() > 0.0 {
                    -1.0
                } else {
                    1.0
                };
                let facing = forward.x * side;
                let glow = e.sunrise_sunset_color >> 24;
                if facing > 0.0 && glow > 0 {
                    let a = glow as f32 / 255.0;
                    fog = srgb_lerp(facing * a, fog, e.sunrise_sunset_color | 0xFF00_0000);
                }
            }
            let reach = (e.sky_fog_end / 16.0).min(render_chunks);
            let lerp = (reach / 32.0).clamp(0.0, 1.0) * 0.75 + 0.25;
            fog = srgb_lerp(1.0 - lerp.powf(0.25), fog, e.sky_color);
            let sun = sun_dir.unwrap_or_else(|| sky::celestial_direction(e.sun_angle));
            let moon = if sun_dir.is_some() {
                -sun
            } else {
                sky::celestial_direction(e.moon_angle)
            };
            let sr = e.sunrise_sunset_color;
            desc.sky = rgb(e.sky_color);
            desc.fog = rgb(fog);
            desc.sunrise = [
                channel(sr, 16),
                channel(sr, 8),
                channel(sr, 0),
                channel(sr, 24),
            ];
            desc.sun_angle = e.sun_angle;
            desc.sun = sun_rect.map(|r| (sun, r));
            desc.moon = rect(&crate::scene::moon_texture(&e.moon_phase)).map(|r| (moon, r));
            desc.star_angle = e.star_angle;
            desc.star_brightness = e.star_brightness;
            let colours = QuickSkyColours {
                sky: desc.sky,
                fog: desc.fog,
                sun_dir: sun,
                sky_factor: day,
            };
            (colours, desc)
        }
    }
}

/// The path tracer's parameters for a camera and a sky, with the emitter intensity the
/// frame is drawn at.
#[allow(clippy::too_many_arguments)]
pub fn hq_params(
    o: &HqOptions,
    emitter_intensity: f32,
    width: u32,
    height: u32,
    hq: &voxel::HqScene,
    eye: Vec3,
    rays: &CameraRays,
    s: &HqSky,
) -> Params {
    let sun_radius = sky::SUN_RADIUS;
    let f4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
    let sky = sky::Preetham::new(s.sun_dir);
    let (su, sv) = sky::sun_texture_axes(s.sun_dir);
    Params {
        cam_pos: f4(eye, rays.tan_half_fov),
        cam_fwd: f4(rays.forward, rays.aspect),
        cam_right: f4(rays.right, 0.0),
        cam_up: f4(rays.up, 0.0),
        sun_dir: f4(s.sun_dir, sun_radius.cos()),
        sun_color: f4(s.sun_color, sun_radius),
        sky_sun: f4(sky.sun, s.sky_light),
        sky_zenith: f4(Vec3::from(sky.zenith), 0.0),
        perez_a: f4(Vec3::from(sky.perez[0]), 0.0),
        perez_b: f4(Vec3::from(sky.perez[1]), 0.0),
        perez_c: f4(Vec3::from(sky.perez[2]), 0.0),
        perez_d: f4(Vec3::from(sky.perez[3]), 0.0),
        perez_e: f4(Vec3::from(sky.perez[4]), 0.0),
        water_color: f4(Vec3::new(0.42, 0.16, 0.10), 1.333),
        misc_f: [
            emitter_intensity,
            1e-3,
            sky::SKY_CACHE_RESOLUTION as f32,
            0.0,
        ],
        grid_dims: [
            hq.dims[0],
            hq.dims[1],
            hq.dims[2],
            hq.dims[0] * hq.dims[1] * hq.dims[2],
        ],
        off: [hq.offsets[0], hq.offsets[1], hq.offsets[2], hq.offsets[3]],
        off2: [
            hq.offsets[4],
            hq.atlas_size[0],
            hq.atlas_size[1],
            hq.quads.len() as u32,
        ],
        frame: [width, height, 0, o.max_bounces],
        misc_u: [o.seed, 1, hq.models.len() as u32, hq.emitter_count],
        see_through: [hq.see_through[0], hq.see_through[1], 0, 0],
        emitter: [
            if emitter_intensity > 0.0 {
                match o.emitter_sampling {
                    EmitterSampling::None => EMITTER_SAMPLING_NONE,
                    EmitterSampling::One => EMITTER_SAMPLING_ONE,
                    EmitterSampling::OneBlock => EMITTER_SAMPLING_ONE_BLOCK,
                    EmitterSampling::All => EMITTER_SAMPLING_ALL,
                }
            } else {
                EMITTER_SAMPLING_NONE
            },
            o.prevent_normal_emitter as u32,
            0,
            0,
        ],
        sun_su: f4(su, sky::SUN_TEXTURE_BRIGHTNESS),
        sun_sv: f4(sv, 4.0 * sun_radius),
        sun_sprite: hq.sun_sprite.map_or([0.0; 4], |r| r.map(|v| v as f32)),
        sky_map: [0.0; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AtmosphericFogEnvironment.getBaseColor` in the 1.21.11 client lerps the clear colour
    /// towards the sky colour by `1 − Mth.clampedLerp(min(sky fog end / 16, render
    /// distance) / 32, 0.25, 1)^0.25`, and `Mth.clampedLerp(delta, start, end)` is
    /// `start` below 0, `end` above 1 and `lerp(delta, start, end)` between (both read off
    /// the jar's bytecode): at the client's default 12 chunks the weight is
    /// `1 − 0.53125^0.25` = 0.146, not `1 − 0.375^0.25` = 0.218.
    #[test]
    fn the_clear_colour_leans_to_the_sky_as_the_client_does() {
        let env = assets::environment::EnvironmentState {
            sky_color: 0xFF78_A7FF,
            fog_color: 0xFFC0_D8FF,
            sunrise_sunset_color: 0,
            sun_angle: 0.0,
            moon_angle: 180.0,
            star_angle: 0.0,
            star_brightness: 0.0,
            moon_phase: "full_moon".into(),
            fog_start: 0.0,
            fog_end: 1024.0,
            sky_fog_end: 512.0,
            sky_light_factor: 1.0,
            sky_light_color: 0xFFFF_FFFF,
            skybox: assets::environment::Skybox::Overworld,
        };
        let sky = Sky::Vanilla {
            time: Some(6000),
            sun_dir: None,
            hq: HqSkyModel::default(),
        };
        let none = |_: &str| None;
        for (chunks, w) in [
            (12.0f32, 1.0 - 0.53125f32.powf(0.25)),
            (2.0, 1.0 - (0.25f32 + 0.75 * 2.0 / 32.0).powf(0.25)),
            (40.0, 0.0),
        ] {
            let (_, desc) = quick_sky(
                &sky,
                Some(&env),
                "minecraft:plains",
                0,
                Vec3::Z,
                chunks * 16.0,
                (0.0, 1.0),
                None,
                &none,
            );
            let lerp = |a: u32, b: u32| (a as f32 + (w * (b as f32 - a as f32)).floor()) / 255.0;
            let want = Vec3::new(lerp(0xC0, 0x78), lerp(0xD8, 0xA7), lerp(0xFF, 0xFF));
            assert_eq!(desc.fog, want, "{chunks} chunks");
        }
    }
}
