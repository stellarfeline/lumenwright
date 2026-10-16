//! The CLI's render job: flags -> [`Request`] -> [`crate::render`] -> image file.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use glam::DVec3;

use crate::assets::jar;
use crate::render::post::{self, Tonemap};
use crate::render::sky;
use crate::render::{BackendKind, Mode};
use crate::request::{ChunkRect, EntityPolicy, HqOptions, QuickOptions, Request, Sky, WorldSource};
use crate::scene::camera::Camera;
use crate::world::{Dimension, World};

#[derive(Clone, Debug, clap::Args)]
pub struct RenderArgs {
    /// Path to the world save directory (the folder containing level.dat).
    #[arg(long, short = 'w')]
    pub world: PathBuf,
    /// Output PNG path.
    #[arg(long, short = 'o', default_value = "render.png")]
    pub output: PathBuf,
    /// Also write the linear HDR image (HQ mode) as a .pfm file.
    #[arg(long)]
    pub hdr_output: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = Mode::Quick)]
    pub mode: Mode,
    #[arg(long, value_enum, default_value_t = BackendKind::Auto)]
    pub backend: BackendKind,
    #[arg(long, default_value = "overworld")]
    pub dimension: String,
    #[arg(long, default_value_t = 1280)]
    pub width: u32,
    #[arg(long, default_value_t = 720)]
    pub height: u32,
    /// Camera eye position "x,y,z". Defaults to the player's eye (level.dat), else spawn.
    #[arg(long, allow_hyphen_values = true)]
    pub camera: Option<String>,
    /// Yaw in degrees, Minecraft convention (0 = south, 90 = west, 180 = north, -90 = east).
    #[arg(long, allow_hyphen_values = true)]
    pub yaw: Option<f32>,
    /// Pitch in degrees, Minecraft convention (positive looks down).
    #[arg(long, allow_hyphen_values = true)]
    pub pitch: Option<f32>,
    /// Point "x,y,z" to look at (overrides yaw/pitch).
    #[arg(long, allow_hyphen_values = true)]
    pub look_at: Option<String>,
    /// Vertical field of view in degrees.
    #[arg(long, default_value_t = 70.0)]
    pub fov: f32,
    /// Chunks loaded around the camera (radius).
    #[arg(long, default_value_t = 8)]
    pub render_distance: i32,
    /// HQ: samples per pixel.
    #[arg(long, default_value_t = 64)]
    pub spp: u32,
    /// HQ: maximum diffuse bounces.
    #[arg(long, default_value_t = 4)]
    pub max_bounces: u32,
    #[arg(long, default_value_t = 0)]
    pub seed: u32,
    /// Quick: supersampling factor (renders at N× and downsamples).
    #[arg(long, default_value_t = 1)]
    pub ssaa: u32,
    /// Time of day in ticks (0 = sunrise, 6000 = noon). Defaults to the world's time.
    #[arg(long)]
    pub time: Option<i64>,
    /// Sun altitude in degrees (overrides --time for the sun position).
    #[arg(long, allow_hyphen_values = true)]
    pub sun_altitude: Option<f32>,
    /// Sun azimuth in degrees (0 = north, 90 = east).
    #[arg(long, default_value_t = 135.0, allow_hyphen_values = true)]
    pub sun_azimuth: f32,
    /// HQ: sun irradiance scale.
    #[arg(long, default_value_t = 1.5)]
    pub sun_intensity: f32,
    /// HQ: sky radiance scale.
    #[arg(long, default_value_t = 1.0)]
    pub sky_intensity: f32,
    /// HQ: emissive block strength.
    #[arg(long, default_value_t = 6.0)]
    pub emitter_intensity: f32,
    #[arg(long, default_value_t = 1.0)]
    pub exposure: f32,
    #[arg(long, value_enum, default_value_t = Tonemap::Aces)]
    pub tonemap: Tonemap,
    /// Quick: vanilla brightness option (0 = moody, 0.5 = default, 1 = bright).
    #[arg(long, default_value_t = 0.5)]
    pub brightness: f32,
    /// Quick: disable smooth lighting / ambient occlusion.
    #[arg(long)]
    pub flat_lighting: bool,
    /// Explicit Minecraft client jar (textures/models).
    #[arg(long)]
    pub minecraft_jar: Option<PathBuf>,
    /// Minecraft version whose jar to use (default: the world's version).
    #[arg(long)]
    pub mc_version: Option<String>,
    /// Never download the client jar.
    #[arg(long)]
    pub offline: bool,
    /// Resource packs (zip or directory), applied in order on top of vanilla.
    #[arg(long = "resource-pack")]
    pub resource_packs: Vec<PathBuf>,
    /// Do not render entities.
    #[arg(long)]
    pub no_entities: bool,
    /// Do not render block entities drawn by Java code (signs, chests, banners...).
    #[arg(long)]
    pub no_block_entities: bool,
    /// How to draw entities without a model.
    #[arg(long, value_enum, default_value_t = UnknownEntities::Box)]
    pub unknown_entities: UnknownEntities,
    /// Render players from level.dat / playerdata (default skin).
    #[arg(long)]
    pub players: bool,
}

pub use crate::request::UnknownEntities;

pub fn parse_vec3(s: &str) -> Result<DVec3> {
    let v: Vec<f64> = s
        .split(',')
        .map(|p| p.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("expected \"x,y,z\", got \"{s}\""))?;
    if v.len() != 3 {
        bail!("expected \"x,y,z\", got \"{s}\"");
    }
    Ok(DVec3::new(v[0], v[1], v[2]))
}

pub struct RenderReport {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub timings: Vec<(&'static str, f64)>,
    pub entity_stats: crate::scene::entities::EntityStats,
    pub block_entity_stats: crate::scene::block_entities::BlockEntityStats,
    pub camera: Camera,
}

fn camera_from_args(a: &RenderArgs, world: &World) -> Result<Camera> {
    let player = world.player();
    let pos = match &a.camera {
        Some(s) => parse_vec3(s)?,
        None => match player {
            Some(p) => DVec3::new(p.pos[0], p.pos[1] + 1.62, p.pos[2]),
            None => {
                let s = world.level.spawn;
                DVec3::new(s[0] as f64 + 0.5, s[1] as f64 + 1.62, s[2] as f64 + 0.5)
            }
        },
    };
    let (mut yaw, mut pitch) = match (a.camera.is_none(), player) {
        (true, Some(p)) => (p.yaw, p.pitch),
        _ => (0.0, 0.0),
    };
    if let Some(y) = a.yaw {
        yaw = y;
    }
    if let Some(p) = a.pitch {
        pitch = p;
    }
    if let Some(t) = &a.look_at {
        (yaw, pitch) = Camera::look_at(pos, parse_vec3(t)?);
    }
    Ok(Camera {
        pos,
        yaw,
        pitch: pitch.clamp(-90.0, 90.0),
        fov_y: a.fov,
    })
}

/// The CLI's `render`: resolves the jar, derives the camera and the chunk
/// square from the flags, calls [`crate::render`], and writes the image.
pub fn run(a: &RenderArgs) -> Result<RenderReport> {
    let t = Instant::now();
    let dimension = Dimension::parse(&a.dimension)?;
    let world = World::open(&a.world, dimension)?;
    let cam = camera_from_args(a, &world)?;
    log::info!(
        "world '{}' (Minecraft {}), camera at ({:.1}, {:.1}, {:.1}) yaw {:.1} pitch {:.1}",
        world.level.name,
        world.level.version_name.as_deref().unwrap_or("?"),
        cam.pos.x,
        cam.pos.y,
        cam.pos.z,
        cam.yaw,
        cam.pitch
    );
    let version = a
        .mc_version
        .clone()
        .or_else(|| world.level.version_name.clone());
    let jar_path = jar::resolve_jar(&jar::JarRequest {
        explicit: a.minecraft_jar.as_deref(),
        version: version.as_deref(),
        allow_download: !a.offline,
    })?;
    let opening = t.elapsed().as_secs_f64();
    let r = a.render_distance.max(1);
    let req = Request {
        world: WorldSource::Opened(&world),
        chunks: ChunkRect::around(cam.pos, r),
        jar: &jar_path,
        resource_packs: &a.resource_packs,
        camera: cam,
        mode: a.mode,
        backend: a.backend,
        width: a.width,
        height: a.height,
        entities: EntityPolicy {
            entities: !a.no_entities,
            unknown: a.unknown_entities,
            block_entities: !a.no_block_entities,
            players: a.players,
        },
        quick: QuickOptions {
            ssaa: a.ssaa,
            draw_distance: (r * 16) as f32,
            brightness: a.brightness,
            smooth: !a.flat_lighting,
        },
        hq: HqOptions {
            spp: a.spp,
            max_bounces: a.max_bounces,
            seed: a.seed,
            emitter_intensity: a.emitter_intensity,
            exposure: a.exposure,
            tonemap: a.tonemap,
        },
        sky: Sky::Vanilla {
            time: a.time,
            sun_dir: a
                .sun_altitude
                .map(|alt| sky::sun_direction(alt, a.sun_azimuth)),
            sun_intensity: a.sun_intensity,
            sky_intensity: a.sky_intensity,
        },
    };
    let out = crate::render(&req)?;
    if let Some(why) = &out.backend_fallback {
        log::warn!("{why}; falling back to the CPU backend");
    }
    let mut timings = vec![("open world", opening)];
    timings.extend(out.timings);
    let t = Instant::now();
    if let (Some(p), Some(hdr)) = (&a.hdr_output, &out.hdr) {
        post::write_pfm(p, hdr, out.width, out.height)?;
    }
    post::write_png_rgba(&a.output, &out.rgba, out.width, out.height)?;
    timings.push(("write image", t.elapsed().as_secs_f64()));
    Ok(RenderReport {
        output: a.output.clone(),
        width: out.width,
        height: out.height,
        backend: out.backend,
        timings,
        entity_stats: out.entity_stats,
        block_entity_stats: out.block_entity_stats,
        camera: cam,
    })
}
