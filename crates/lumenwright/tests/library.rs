//! The library entry point, `lumenwright::render`: it draws what the CLI draws,
//! refuses a missing jar by name before reading anything, and gives the same
//! CPU pixels for any thread count. Tests that draw need the vanilla client jar
//! (found like the CLI does, or via LUMENWRIGHT_MC_JAR) and skip without it.

use std::path::{Path, PathBuf};
use std::process::Command;

use lumenwright::assets::jar;
use lumenwright::fixture;
use lumenwright::glam::{DVec3, Vec3};
use lumenwright::render::sky;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::scene::camera::Camera;
use lumenwright::world::Dimension;
use lumenwright::{
    ChunkRect, EntityPolicy, HqOptions, QuickOptions, Rendered, Request, RequestError, Sky,
    WorldSource,
};

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
    let d = std::env::temp_dir().join(format!("lumenwright-lib-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The block-entity gallery, looked at from the north side as the CLI tests do.
const EYE: [f64; 3] = [9.5, 70.5, 11.5];
const TARGET: [f64; 3] = [9.5, 65.4, -2.0];
const RADIUS: i32 = 8;

fn camera() -> Camera {
    let pos = DVec3::from_array(EYE);
    let (yaw, pitch) = Camera::look_at(pos, DVec3::from_array(TARGET));
    Camera {
        pos,
        yaw,
        pitch,
        fov_y: 60.0,
    }
}

fn request<'a>(world: &'a Path, jar: &'a Path, mode: Mode) -> Request<'a> {
    let cam = camera();
    Request {
        world: WorldSource::Dir {
            path: world,
            dimension: Dimension::Overworld,
        },
        chunks: ChunkRect::around(cam.pos, RADIUS),
        jar,
        resource_packs: &[],
        camera: cam,
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
        sky: Sky::Vanilla {
            time: None,
            sun_dir: Some(sky::sun_direction(40.0, 135.0)),
            sun_intensity: 1.5,
            sky_intensity: 1.0,
        },
    }
}

fn cli_png(world: &Path, jar: &Path, mode: &str, out: &Path) -> image::RgbaImage {
    let status = Command::new(env!("CARGO_BIN_EXE_lumenwright"))
        .args(["--log", "warn", "render", "--offline", "--backend", "cpu"])
        .arg("--world")
        .arg(world)
        .arg("--minecraft-jar")
        .arg(jar)
        .arg("--output")
        .arg(out)
        .args(["--mode", mode, "--width", "192", "--height", "108"])
        .args(["--camera", &format!("{},{},{}", EYE[0], EYE[1], EYE[2])])
        .args([
            "--look-at",
            &format!("{},{},{}", TARGET[0], TARGET[1], TARGET[2]),
        ])
        .args(["--fov", "60", "--render-distance", &RADIUS.to_string()])
        .args(["--ssaa", "2", "--spp", "8", "--sun-altitude", "40"])
        .status()
        .unwrap();
    assert!(status.success());
    image::open(out).unwrap().to_rgba8()
}

#[test]
fn cli_png_equals_library_pixels() {
    let jar = require_jar!();
    let dir = tmp("cli-vs-lib");
    let world = dir.join("world");
    fixture::block_entity_gallery().write(&world).unwrap();
    for (name, mode) in [("quick", Mode::Quick), ("hq", Mode::Hq)] {
        let png = cli_png(&world, &jar, name, &dir.join(format!("{name}.png")));
        let lib = lumenwright::render(&request(&world, &jar, mode)).unwrap();
        assert_eq!((png.width(), png.height()), (lib.width, lib.height));
        assert_eq!(lib.rgba.len(), 192 * 108 * 4);
        assert!(
            png.as_raw() == &lib.rgba,
            "{name}: the CLI's PNG and the library's pixels differ"
        );
        assert!(lib.backend.starts_with("cpu"), "{}", lib.backend);
        assert_eq!(lib.backend_fallback, None);
        assert_eq!(lib.hdr.is_some(), mode == Mode::Hq);
        assert!(lib.chunks_loaded > 0);
        assert!(
            lib.block_entity_stats.rendered.values().sum::<usize>() > 0,
            "{name}: no block entity drawn"
        );
    }
}

#[test]
fn a_missing_jar_is_refused_by_name_before_anything_is_read() {
    // Neither the world nor the jar exists: the jar is named, so nothing was opened first.
    let world = Path::new("/nonexistent/lumenwright/world");
    let jar = Path::new("/nonexistent/lumenwright/client.jar");
    let err = match lumenwright::render(&request(world, jar, Mode::Quick)) {
        Ok(_) => panic!("rendered without a jar"),
        Err(e) => e,
    };
    assert_eq!(
        err.downcast_ref::<RequestError>(),
        Some(&RequestError::JarNotFound(jar.to_path_buf())),
        "{err:#}"
    );
}

#[test]
fn a_stated_sky_is_drawn_as_given() {
    let jar = require_jar!();
    let dir = tmp("stated-sky");
    let world = dir.join("world");
    fixture::block_entity_gallery().write(&world).unwrap();
    let mut req = request(&world, &jar, Mode::Quick);
    // Look straight up: the frame is all sky.
    req.camera.pitch = -90.0;
    req.sky = Sky::Stated(lumenwright::StatedSky {
        quick: lumenwright::QuickSkyColours {
            sky: Vec3::new(1.0, 0.0, 0.0),
            fog: Vec3::new(1.0, 0.0, 0.0),
            sun_dir: Vec3::new(0.0, -1.0, 0.0),
            sky_factor: 1.0,
        },
        hq: sky::hq_sky(Vec3::Y, 1.5, 1.0),
    });
    let out = lumenwright::render(&req).unwrap();
    let red = out
        .rgba
        .chunks(4)
        .filter(|p| p[0] == 255 && p[1] == 0 && p[2] == 0)
        .count();
    assert_eq!(red, 192 * 108, "the stated sky colour fills the frame");
}

fn on_threads(n: usize, req: &Request) -> Rendered {
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .unwrap()
        .install(|| lumenwright::render(req).unwrap())
}

#[test]
fn cpu_pixels_are_identical_across_thread_counts() {
    let jar = require_jar!();
    let dir = tmp("threads");
    let world = dir.join("world");
    fixture::showcase().write(&world).unwrap();
    for mode in [Mode::Quick, Mode::Hq] {
        let mut req = request(&world, &jar, mode);
        req.camera = Camera {
            // The showcase's own player view (fixture::showcase).
            pos: DVec3::new(18.5, 72.0 + 1.62, 22.5),
            yaw: 150.0,
            pitch: 18.0,
            fov_y: 70.0,
        };
        req.chunks = ChunkRect::around(req.camera.pos, RADIUS);
        let one = on_threads(1, &req);
        assert!(one.backend.contains("1 thread"), "{}", one.backend);
        for n in [3, 8] {
            let many = on_threads(n, &req);
            assert!(
                many.backend.contains(&format!("{n} threads")),
                "{}",
                many.backend
            );
            assert!(
                one.rgba == many.rgba,
                "{mode:?}: pixels differ between 1 and {n} threads"
            );
            assert_eq!(
                one.hdr.as_ref().map(|h| h
                    .iter()
                    .flatten()
                    .map(|f| f.to_bits())
                    .collect::<Vec<_>>()),
                many.hdr.as_ref().map(|h| h
                    .iter()
                    .flatten()
                    .map(|f| f.to_bits())
                    .collect::<Vec<_>>()),
                "{mode:?}: radiance differs between 1 and {n} threads"
            );
        }
    }
}
