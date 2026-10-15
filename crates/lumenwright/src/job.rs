//! End-to-end render job: world -> scene -> backend -> image.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use glam::{DVec3, Vec3};
use lumenwright_kernel::Params;

use crate::assets::{self, jar};
use crate::render::post::{self, Tonemap};
use crate::render::sky::{self, QuickSky};
use crate::render::{self, BackendKind, CameraRays, HqFrame, Mode, QuickFrame};
use crate::scene::camera::Camera;
use crate::scene::mesh::{self, Lighting, MeshParams};
use crate::scene::{BuildOptions, Scene, voxel};
use crate::world::{Dimension, Entity, World};

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
    /// How to draw entities without a model.
    #[arg(long, value_enum, default_value_t = UnknownEntities::Box)]
    pub unknown_entities: UnknownEntities,
    /// Render players from level.dat / playerdata (default skin).
    #[arg(long)]
    pub players: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum UnknownEntities {
    Box,
    Skip,
}

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

pub fn run(a: &RenderArgs) -> Result<RenderReport> {
    let mut timings = Vec::new();
    let mut t = Instant::now();
    let mut lap = |name: &'static str, t: &mut Instant| {
        timings.push((name, t.elapsed().as_secs_f64()));
        *t = Instant::now();
    };
    if a.width == 0 || a.height == 0 {
        bail!("image size must be positive");
    }
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

    // 1. World.
    let cc = [
        (cam.pos.x.floor() as i32) >> 4,
        (cam.pos.z.floor() as i32) >> 4,
    ];
    let r = a.render_distance.max(1);
    let mut area = world.load_area(
        [cc[0] - r, cc[1] - r],
        [cc[0] + r, cc[1] + r],
        !a.no_entities,
    )?;
    if area.chunks_loaded == 0 {
        bail!(
            "no chunks found around ({:.0}, {:.0}) in {}",
            cam.pos.x,
            cam.pos.z,
            world.region_dir().display()
        );
    }
    let mut entities: Vec<Entity> = std::mem::take(&mut area.entities);
    if a.players && !a.no_entities {
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

    // 2. Assets (Chunky-style: the official client jar).
    let version = a
        .mc_version
        .clone()
        .or_else(|| world.level.version_name.clone());
    let jar_path = jar::resolve_jar(&jar::JarRequest {
        explicit: a.minecraft_jar.as_deref(),
        version: version.as_deref(),
        allow_download: !a.offline,
    })?;
    let store = assets::load_resources(&jar_path, &a.resource_packs)?;
    lap("load assets", &mut t);

    // 3. Scene.
    let opts = BuildOptions {
        entities: !a.no_entities,
        unknown_entities_as_boxes: a.unknown_entities == UnknownEntities::Box,
    };
    let scene = Scene::build(&store, area.registry, area.volume, &entities, &opts);
    lap("build scene", &mut t);

    let mut backend = render::create_backend(a.backend)?;
    log::info!("backend: {}", backend.name());
    let eye = Vec3::new(
        (cam.pos.x - scene.origin[0] as f64) as f32,
        (cam.pos.y - scene.origin[1] as f64) as f32,
        (cam.pos.z - scene.origin[2] as f64) as f32,
    );
    let ticks = a.time.unwrap_or(world.level.day_time);
    let angle = sky::celestial_angle(ticks);
    let sun_dir = match a.sun_altitude {
        Some(alt) => sky::sun_direction(alt, a.sun_azimuth),
        None => sky::sun_direction_from_angle(angle),
    };
    let (f, r_, u) = cam.basis();
    let aspect = a.width as f32 / a.height as f32;
    let rays = CameraRays {
        forward: f,
        right: r_,
        up: u,
        tan_half_fov: (cam.fov_y.to_radians() * 0.5).tan(),
        aspect,
    };

    let (pixels, w, h) = match a.mode {
        Mode::Quick => {
            let k = a.ssaa.clamp(1, 4);
            let (w, h) = (a.width * k, a.height * k);
            let max_dist = (r * 16) as f32;
            let view_proj =
                cam.projection(aspect, 0.05, max_dist * 2.0 + 512.0) * cam.view(Vec3::ZERO);
            let day = sky::daylight(angle);
            let lighting = Lighting {
                sky_factor: day,
                gamma: a.brightness.clamp(0.0, 1.0),
                smooth: !a.flat_lighting,
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
            let biome = scene.registry.biomes[scene.volume.biome(
                cam.pos.x.floor() as i32,
                cam.pos.y.floor() as i32,
                cam.pos.z.floor() as i32,
            ) as usize]
                .clone();
            let temp = crate::assets::biome::biome_info(&biome).temperature;
            let sky_c = Vec3::from(crate::assets::biome::sky_color(temp)) * day;
            let fog_c = Vec3::new(0.753, 0.847, 1.0) * (day * 0.94 + 0.06);
            let fog_range = (max_dist - (max_dist / 10.0).clamp(4.0, 64.0), max_dist);
            let sun_rect = crate::scene::SUN_TEXTURES
                .iter()
                .find_map(|t| scene.atlas.id(t))
                .map(|id| {
                    let s = scene.atlas.sprites[id as usize];
                    [s.x as f32, s.y as f32, s.w as f32, s.h as f32]
                })
                .filter(|_| sun_dir.y > -0.2)
                .unwrap_or([0.0; 4]);
            let qs = QuickSky::new(&rays, w, h, sky_c, fog_c, sun_dir, fog_range, sun_rect);
            let frame = QuickFrame {
                mesh: &mesh,
                atlas_pixels: &scene.atlas.pixels,
                atlas_size: [scene.atlas.width, scene.atlas.height],
                width: w,
                height: h,
                view_proj,
                sky: qs,
            };
            let px = backend.render_quick(&frame)?;
            lap("render", &mut t);
            (post::downsample(&px, w, h, k), a.width, a.height)
        }
        Mode::Hq => {
            let hq = voxel::build_hq(&scene)?;
            lap("build voxel scene", &mut t);
            let params = hq_params(a, &hq, eye, &rays, sun_dir);
            let frame = HqFrame {
                scene: &hq,
                params,
                spp: a.spp,
            };
            let started = Instant::now();
            let progress = |done: u32, total: u32| {
                log::debug!("{done}/{total} ({:.1?})", started.elapsed());
            };
            let hdr = backend.render_hq(&frame, &progress)?;
            lap("render", &mut t);
            if let Some(p) = &a.hdr_output {
                post::write_pfm(p, &hdr, a.width, a.height)?;
            }
            (
                post::tonemap(&hdr, a.exposure, a.tonemap),
                a.width,
                a.height,
            )
        }
    };
    post::write_png(&a.output, &pixels, w, h)?;
    lap("write image", &mut t);
    Ok(RenderReport {
        output: a.output.clone(),
        width: w,
        height: h,
        backend: backend.name(),
        timings,
        entity_stats: scene.entity_stats.clone(),
        camera: cam,
    })
}

pub fn hq_params(
    a: &RenderArgs,
    hq: &voxel::HqScene,
    eye: Vec3,
    rays: &CameraRays,
    sun_dir: Vec3,
) -> Params {
    let s = sky::hq_sky(sun_dir, a.sun_intensity, a.sky_intensity);
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
        misc_f: [a.emitter_intensity, 1e-3, 1.0 / solid_angle, 0.0],
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
        frame: [a.width, a.height, 0, a.max_bounces],
        misc_u: [a.seed, 1, hq.models.len() as u32, 0],
        _reserved: [0; 4],
    }
}
