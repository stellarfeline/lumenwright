//! A stated sky at no stated tick against 0.2.0 as published. Needs the vanilla client jar
//! (found as the command finds it, or via LUMENWRIGHT_MC_JAR) and skips without it.

use std::path::PathBuf;

use lumenwright::glam::{DVec3, Vec3};
use lumenwright::render::sky;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::scene::camera::Camera;
use lumenwright::world::Dimension;
use lumenwright::{
    ChunkRect, EntityPolicy, HqOptions, QuickOptions, Request, Sky, StatedSky, WorldSource,
};
use lumenwright_cli::{fixture, jar, unifont};

fn client_jar() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LUMENWRIGHT_MC_JAR") {
        return Some(PathBuf::from(p));
    }
    jar::resolve_jar(&jar::JarRequest {
        explicit: None,
        version: Some(fixture::VERSION_NAME),
        allow_download: false,
    })
    .ok()
}

fn client_unifont() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LUMENWRIGHT_UNIFONT") {
        return Some(PathBuf::from(p));
    }
    unifont::resolve_unifont(&unifont::UnifontRequest {
        explicit: None,
        version: Some(fixture::VERSION_NAME),
        allow_download: false,
    })
    .ok()
}

macro_rules! require_jar {
    () => {
        match client_jar() {
            Some(j) => j,
            None => {
                eprintln!(
                    "skipping: no Minecraft {} client jar available",
                    fixture::VERSION_NAME
                );
                return;
            }
        }
    };
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lumenwright-compat-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The block-entity gallery from the north side, as the library tests look at it.
const EYE: [f64; 3] = [9.5, 70.5, 11.5];
const TARGET: [f64; 3] = [9.5, 65.4, -2.0];
const RADIUS: i32 = 8;

fn request<'a>(
    world: &'a std::path::Path,
    jar: &'a std::path::Path,
    unifont: Option<&'a std::path::Path>,
    mode: Mode,
) -> Request<'a> {
    let pos = DVec3::from_array(EYE);
    let (yaw, pitch) = Camera::look_at(pos, DVec3::from_array(TARGET));
    let camera = Camera {
        pos,
        yaw,
        pitch,
        fov_y: 60.0,
    };
    Request {
        world: WorldSource::Dir {
            path: world,
            dimension: Dimension::Overworld,
        },
        chunks: ChunkRect::around(pos, RADIUS),
        jar,
        unifont,
        resource_packs: &[],
        camera,
        mode,
        backend: BackendKind::Cpu,
        width: 192,
        height: 108,
        entities: EntityPolicy::default(),
        quick: QuickOptions {
            ssaa: 2,
            draw_distance: (RADIUS * 16) as f32,
            ..QuickOptions::default()
        },
        hq: HqOptions {
            spp: 8,
            ..HqOptions::default()
        },
        sky: Sky::default(),
    }
}

/// A stated sky at no stated tick (`StatedSky::new`) draws what 0.2.0, which had no tick to
/// state, draws for the same request: byte for byte on the CPU in both modes, under a world
/// whose `DayTime` is midnight, where a tick's moon, stars or game look would show.
#[test]
fn a_stated_sky_at_no_tick_draws_as_0_2_0_did() {
    let jar = require_jar!();
    let unifont = client_unifont();
    let dir = tmp("stated-0-2-0");
    let world = dir.join("world");
    let mut fx = fixture::block_entity_gallery();
    fx.day_time = 18000;
    fx.write(&world).unwrap();
    let quick = lumenwright::QuickSkyColours {
        sky: Vec3::new(0.2, 0.3, 0.6),
        fog: Vec3::new(0.7, 0.5, 0.4),
        sun_dir: sky::sun_direction(10.0, 120.0),
        sky_factor: 0.6,
    };
    let hq = sky::hq_sky(quick.sun_dir, 1.4, 0.9);
    let old_quick = lumenwright_020::QuickSkyColours {
        sky: quick.sky,
        fog: quick.fog,
        sun_dir: quick.sun_dir,
        sky_factor: quick.sky_factor,
    };
    let old_hq = lumenwright_020::render::sky::HqSky {
        sun_dir: hq.sun_dir,
        sun_color: hq.sun_color,
        sky_light: hq.sky_light,
    };
    for mode in [Mode::Quick, Mode::Hq] {
        let mut req = request(&world, &jar, unifont.as_deref(), mode);
        req.sky = Sky::Stated(StatedSky::new(quick, hq));
        let cam = req.camera;
        let old = lumenwright_020::Request {
            world: lumenwright_020::WorldSource::Dir {
                path: &world,
                dimension: lumenwright_020::world::Dimension::Overworld,
            },
            chunks: lumenwright_020::ChunkRect {
                min: req.chunks.min,
                max: req.chunks.max,
            },
            jar: &jar,
            unifont: req.unifont,
            resource_packs: &[],
            camera: lumenwright_020::scene::camera::Camera {
                pos: cam.pos,
                yaw: cam.yaw,
                pitch: cam.pitch,
                fov_y: cam.fov_y,
            },
            mode: match mode {
                Mode::Quick => lumenwright_020::render::Mode::Quick,
                Mode::Hq => lumenwright_020::render::Mode::Hq,
            },
            backend: lumenwright_020::render::BackendKind::Cpu,
            width: req.width,
            height: req.height,
            entities: lumenwright_020::EntityPolicy::default(),
            quick: lumenwright_020::QuickOptions {
                ssaa: req.quick.ssaa,
                draw_distance: req.quick.draw_distance,
                ..lumenwright_020::QuickOptions::default()
            },
            hq: lumenwright_020::HqOptions {
                spp: req.hq.spp,
                ..lumenwright_020::HqOptions::default()
            },
            sky: lumenwright_020::Sky::Stated(lumenwright_020::StatedSky {
                quick: old_quick,
                hq: old_hq,
            }),
        };
        let want = lumenwright_020::render(&old).unwrap().rgba;
        let got = lumenwright::render(&req).unwrap().rgba;
        let differing = want
            .chunks(4)
            .zip(got.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differing, 0, "{mode:?}: pixels differ from 0.2.0's");
    }
}
