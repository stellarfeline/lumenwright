//! The library entry point: one frame from an explicit [`Request`].
//!
//! [`render()`] takes every input from the request. It resolves no client jar,
//! performs no network access, writes no file and prints nothing (it emits
//! `log` records, which reach no output unless the caller installs a logger).
//! The CLI's `render` subcommand ([`crate::job::run`]) is a caller of it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use glam::{DVec3, Vec3};
use lumenwright_kernel::Params;

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
        path: &'a Path,
        dimension: Dimension,
    },
    /// A world the caller has already opened.
    Opened(&'a World),
}

/// An inclusive rectangle of chunk coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRect {
    pub min: [i32; 2],
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
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
}

impl Default for QuickOptions {
    fn default() -> Self {
        QuickOptions {
            ssaa: 1,
            draw_distance: 128.0,
            brightness: 0.5,
            smooth: true,
        }
    }
}

/// HQ-mode options: the sampling budget and the post-processing.
#[derive(Clone, Copy, Debug)]
pub struct HqOptions {
    /// Samples per pixel.
    pub spp: u32,
    pub max_bounces: u32,
    pub seed: u32,
    /// Emissive block strength.
    pub emitter_intensity: f32,
    pub exposure: f32,
    pub tonemap: Tonemap,
}

impl Default for HqOptions {
    fn default() -> Self {
        HqOptions {
            spp: 64,
            max_bounces: 4,
            seed: 0,
            emitter_intensity: 6.0,
            exposure: 1.0,
            tonemap: Tonemap::Aces,
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
        /// Unit vector towards the sun; `None` follows `time`.
        sun_dir: Option<Vec3>,
        /// HQ sun irradiance scale.
        sun_intensity: f32,
        /// HQ sky radiance scale.
        sky_intensity: f32,
    },
    /// The caller's sky, used as given; nothing is derived from the world's time.
    Stated(StatedSky),
}

impl Default for Sky {
    fn default() -> Self {
        Sky::Vanilla {
            time: None,
            sun_dir: None,
            sun_intensity: 1.5,
            sky_intensity: 1.0,
        }
    }
}

/// A caller-stated sky for both modes.
#[derive(Clone, Copy, Debug)]
pub struct StatedSky {
    pub quick: QuickSkyColours,
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
    pub world: WorldSource<'a>,
    /// The chunks loaded; nothing outside them is drawn.
    pub chunks: ChunkRect,
    /// The Minecraft client jar the textures and models are read from.
    pub jar: &'a Path,
    /// Resource packs (zip or directory) layered over the jar, later ones winning.
    pub resource_packs: &'a [PathBuf],
    pub camera: Camera,
    pub mode: Mode,
    pub backend: BackendKind,
    pub width: u32,
    pub height: u32,
    pub entities: EntityPolicy,
    pub quick: QuickOptions,
    pub hq: HqOptions,
    pub sky: Sky,
}

/// A drawn frame.
pub struct Rendered {
    pub width: u32,
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
    pub chunks_loaded: usize,
    /// Entities handed to the scene (stored ones plus players).
    pub entities_loaded: usize,
    pub block_entities_loaded: usize,
    pub entity_stats: EntityStats,
    pub block_entity_stats: BlockEntityStats,
}

/// A request [`render()`] refuses before drawing. Returned inside the
/// `anyhow::Error`; `err.downcast_ref::<RequestError>()` names it.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestError {
    /// Width or height is zero.
    EmptyImage { width: u32, height: u32 },
    /// The client jar path is not a file.
    JarNotFound(PathBuf),
    /// A resource pack path does not exist.
    ResourcePackNotFound(PathBuf),
    /// `chunks.min` exceeds `chunks.max` on an axis.
    InvertedChunkRect(ChunkRect),
    /// None of the requested chunks exists in the save.
    NoChunks {
        chunks: ChunkRect,
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
    let store = assets::load_resources(req.jar, req.resource_packs)?;
    lap("load assets", &mut t);

    // 3. Scene.
    let opts = BuildOptions {
        entities: policy.entities,
        unknown_entities_as_boxes: policy.unknown == UnknownEntities::Box,
        block_entities: policy.block_entities,
        eye: Some([cam.pos.x, cam.pos.y, cam.pos.z]),
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
    // The vanilla derivation, when the caller states no sky.
    let vanilla = |time: Option<i64>, sun: Option<Vec3>| {
        let angle = sky::celestial_angle(time.unwrap_or(world.level.day_time));
        let sun_dir = sun.unwrap_or_else(|| sky::sun_direction_from_angle(angle));
        (angle, sun_dir)
    };

    let (pixels, hdr) = match req.mode {
        Mode::Quick => {
            let q = req.quick;
            let k = q.ssaa.clamp(1, 4);
            let (w, h) = (req.width * k, req.height * k);
            let max_dist = q.draw_distance;
            let view_proj =
                cam.projection(aspect, 0.05, max_dist * 2.0 + 512.0) * cam.view(Vec3::ZERO);
            let colours = match req.sky {
                Sky::Stated(s) => s.quick,
                Sky::Vanilla { time, sun_dir, .. } => {
                    let (angle, sun_dir) = vanilla(time, sun_dir);
                    let day = sky::daylight(angle);
                    let biome = scene.registry.biomes[scene.volume.biome(
                        cam.pos.x.floor() as i32,
                        cam.pos.y.floor() as i32,
                        cam.pos.z.floor() as i32,
                    ) as usize]
                        .clone();
                    let temp = assets::biome::biome_info(&biome).temperature;
                    QuickSkyColours {
                        sky: Vec3::from(assets::biome::sky_color(temp)) * day,
                        fog: Vec3::new(0.753, 0.847, 1.0) * (day * 0.94 + 0.06),
                        sun_dir,
                        sky_factor: day,
                    }
                }
            };
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
            let fog_range = (max_dist - (max_dist / 10.0).clamp(4.0, 64.0), max_dist);
            let sun_rect = crate::scene::SUN_TEXTURES
                .iter()
                .find_map(|t| scene.atlas.id(t))
                .map(|id| {
                    let s = scene.atlas.sprites[id as usize];
                    [s.x as f32, s.y as f32, s.w as f32, s.h as f32]
                })
                .filter(|_| colours.sun_dir.y > -0.2)
                .unwrap_or([0.0; 4]);
            let qs = QuickSky::new(
                &rays,
                w,
                h,
                colours.sky,
                colours.fog,
                colours.sun_dir,
                fog_range,
                sun_rect,
            );
            let frame = QuickFrame {
                mesh: &mesh,
                atlas_pixels: &scene.atlas.pixels,
                atlas_size: [scene.atlas.width, scene.atlas.height],
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
            let hq_sky = match req.sky {
                Sky::Stated(s) => s.hq,
                Sky::Vanilla {
                    time,
                    sun_dir,
                    sun_intensity,
                    sky_intensity,
                } => {
                    let (_, sun_dir) = vanilla(time, sun_dir);
                    sky::hq_sky(sun_dir, sun_intensity, sky_intensity)
                }
            };
            let params = hq_params(&req.hq, req.width, req.height, &hq, eye, &rays, &hq_sky);
            let frame = HqFrame {
                scene: &hq,
                params,
                spp: req.hq.spp,
            };
            let started = Instant::now();
            let progress = |done: u32, total: u32| {
                log::debug!("{done}/{total} ({:.1?})", started.elapsed());
            };
            let hdr = be.render_hq(&frame, &progress)?;
            lap("render", &mut t);
            let px = post::tonemap(&hdr, req.hq.exposure, req.hq.tonemap);
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

/// The path tracer's parameters for a camera and a sky.
pub fn hq_params(
    o: &HqOptions,
    width: u32,
    height: u32,
    hq: &voxel::HqScene,
    eye: Vec3,
    rays: &CameraRays,
    s: &HqSky,
) -> Params {
    let sun_radius: f32 = 0.03;
    let solid_angle = std::f32::consts::TAU * (1.0 - sun_radius.cos());
    let f4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
    Params {
        cam_pos: f4(eye, rays.tan_half_fov),
        cam_fwd: f4(rays.forward, rays.aspect),
        cam_right: f4(rays.right, 0.0),
        cam_up: f4(rays.up, 0.0),
        sun_dir: f4(s.sun_dir, sun_radius.cos()),
        sun_color: f4(s.sun_color, sun_radius),
        sky_zenith: f4(s.zenith, 1.0),
        sky_horizon: f4(s.horizon, 0.25),
        water_color: f4(Vec3::new(0.42, 0.16, 0.10), 1.333),
        misc_f: [o.emitter_intensity, 1e-3, 1.0 / solid_angle, 0.0],
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
        misc_u: [o.seed, 1, hq.models.len() as u32, 0],
        _reserved: [0; 4],
    }
}
