//! The library entry point, `lumenwright::render`: it draws what the CLI draws,
//! refuses a missing jar by name before reading anything, and gives the same
//! CPU pixels for any thread count. Tests that draw need the vanilla client jar
//! (found like the CLI does, or via LUMENWRIGHT_MC_JAR) and skip without it.

use std::path::{Path, PathBuf};
use std::process::Command;

use lumenwright::glam::{DVec3, Vec3};
use lumenwright::render::sky;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::scene::camera::Camera;
use lumenwright::world::Dimension;
use lumenwright::{
    ChunkRect, EntityPolicy, HqOptions, HqSkyModel, QuickOptions, QuickSkyColours, Rendered,
    Request, RequestError, Sky, SkyLook, StatedSky, StatedTick, WorldSource,
};
use lumenwright_cli::fixture;
use lumenwright_cli::jar;
use lumenwright_cli::unifont;

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

/// The Unifont directory (LUMENWRIGHT_UNIFONT, or the cache's), if there is one.
fn client_unifont() -> Option<&'static Path> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        if let Ok(p) = std::env::var("LUMENWRIGHT_UNIFONT") {
            return Some(PathBuf::from(p));
        }
        unifont::resolve_unifont(&unifont::UnifontRequest {
            explicit: None,
            version: Some(fixture::VERSION_NAME),
            allow_download: false,
        })
        .ok()
    })
    .as_deref()
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
        unifont: client_unifont(),
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
            hq: HqSkyModel {
                render_distance: (RADIUS * 16) as f32,
                ..HqSkyModel::default()
            },
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
        .arg("--unifont")
        .arg(client_unifont().expect("the unifont: set LUMENWRIGHT_UNIFONT or run `lumenwright assets fetch-unifont 1.21.11`"))
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
    req.sky = Sky::Stated(lumenwright::StatedSky::new(
        lumenwright::QuickSkyColours {
            sky: Vec3::new(1.0, 0.0, 0.0),
            fog: Vec3::new(1.0, 0.0, 0.0),
            sun_dir: Vec3::new(0.0, -1.0, 0.0),
            sky_factor: 1.0,
        },
        sky::hq_sky(Vec3::Y, 1.5, 1.0),
    ));
    let out = lumenwright::render(&req).unwrap();
    let red = out
        .rgba
        .chunks(4)
        .filter(|p| p[0] == 255 && p[1] == 0 && p[2] == 0)
        .count();
    assert_eq!(red, 192 * 108, "the stated sky colour fills the frame");
}

/// A stated sky standing at a tick is drawn as `Sky::Vanilla` at that `time` is, when the
/// caller states the colours and suns Lumenwright derives there
/// (`QuickSkyColours::vanilla`, `sky::hq_sky`): byte for byte on the CPU, in quick mode
/// (the sunrise glow, the moon and the stars of the tick) and in HQ mode under the game's
/// look and under the look by time of day (its weight, the sky light and the lens's sky).
/// The tick moves the frame: without it the same stated sky draws something else.
#[test]
fn a_stated_sky_at_a_tick_is_the_vanilla_sky_at_it() {
    let jar = require_jar!();
    let dir = tmp("stated-tick");
    let world = dir.join("world");
    fixture::block_entity_gallery().write(&world).unwrap();
    let store = lumenwright::assets::load_resources(&jar, None, &[]).unwrap();
    let distance = (RADIUS * 16) as f32;
    let forward = camera().basis().0;
    // 12800: the sun just set, the glow up; 18000: midnight, the moon and the stars out.
    for (time, mode, look) in [
        (12800, Mode::Quick, SkyLook::ByTime),
        (18000, Mode::Quick, SkyLook::ByTime),
        (12800, Mode::Hq, SkyLook::Game),
        (12800, Mode::Hq, SkyLook::ByTime),
        (18000, Mode::Hq, SkyLook::Game),
    ] {
        let env = lumenwright::assets::environment::sample(
            &store,
            "minecraft:overworld",
            "minecraft:plains",
            time,
        )
        .expect("a 1.21.11 jar carries environment attributes");
        let sun = sky::celestial_direction(env.sun_angle);
        let mut vanilla = request(&world, &jar, mode);
        vanilla.sky = Sky::Vanilla {
            time: Some(time),
            sun_dir: Some(sun),
            hq: HqSkyModel {
                look,
                render_distance: distance,
                ..HqSkyModel::default()
            },
        };
        let colours = QuickSkyColours::vanilla(
            Some(&env),
            "minecraft:plains",
            time,
            Some(sun),
            forward,
            distance,
        );
        let untimed = StatedSky::new(colours, sky::hq_sky(sun, sky::DEFAULT_SUN_INTENSITY, 1.0));
        let mut tick = StatedTick::new(time, look);
        tick.render_distance = distance;
        let mut stated = vanilla;
        stated.sky = Sky::Stated(untimed.at(tick));
        let mut bare = vanilla;
        bare.sky = Sky::Stated(untimed);
        let name = format!("{mode:?} {look:?} at {time}");
        let want = lumenwright::render(&vanilla).unwrap().rgba;
        let got = lumenwright::render(&stated).unwrap().rgba;
        let differing = want
            .chunks(4)
            .zip(got.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(differing, 0, "{name}: pixels differ from Sky::Vanilla's");
        assert_ne!(
            lumenwright::render(&bare).unwrap().rgba,
            want,
            "{name}: the tick moved nothing"
        );
    }
}

/// A stated sky at no stated tick (`StatedSky::new`) draws what 0.2.0, which had no tick to
/// state, draws for the same request: byte for byte on the CPU in both modes, under a world
/// whose `DayTime` is midnight, where a tick's moon, stars or game look would show.
#[test]
fn a_stated_sky_at_no_tick_draws_as_0_2_0_did() {
    let jar = require_jar!();
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
        let mut req = request(&world, &jar, mode);
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

/// A JSON file from the client jar.
fn jar_json(jar: &Path, name: &str) -> serde_json::Value {
    let f = std::fs::File::open(jar).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut e = z.by_name(name).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut e, &mut s).unwrap();
    serde_json::from_str(&s).unwrap()
}

fn hex_rgb(v: &serde_json::Value) -> [i32; 3] {
    let n = i32::from_str_radix(v.as_str().unwrap().trim_start_matches('#'), 16).unwrap();
    [(n >> 16) & 0xFF, (n >> 8) & 0xFF, n & 0xFF]
}

/// A linear colour track of `day.json` at `ticks`, read straight from its
/// keyframes: the two around `ticks` (wrapping over the 24000-tick period),
/// each channel `a + floor(f * (b - a))` as the client's `ARGB.srgbLerp`.
fn day_track(day: &serde_json::Value, track: &str, ticks: i64) -> [i32; 3] {
    let keys: Vec<(i64, [i32; 3])> = day["tracks"][track]["keyframes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["ticks"].as_i64().unwrap(), hex_rgb(&k["value"])))
        .collect();
    let t = ticks.rem_euclid(24000);
    let i = keys.iter().rposition(|k| k.0 <= t);
    let (a, b) = match i {
        Some(i) if i + 1 < keys.len() => (keys[i], keys[i + 1]),
        Some(i) => (keys[i], (keys[0].0 + 24000, keys[0].1)),
        None => (
            (keys[keys.len() - 1].0 - 24000, keys[keys.len() - 1].1),
            keys[0],
        ),
    };
    let f = (t - a.0) as f32 / (b.0 - a.0) as f32;
    [0, 1, 2].map(|c| a.1[c] + (f * (b.1[c] - a.1[c]) as f32).floor() as i32)
}

#[test]
fn quick_sky_follows_the_vanilla_day() {
    let jar = require_jar!();
    let dir = tmp("vanilla-sky");
    let world = dir.join("world");
    fixture::block_entity_gallery().write(&world).unwrap();
    let day = jar_json(&jar, "data/minecraft/timeline/day.json");
    let plains = jar_json(&jar, "data/minecraft/worldgen/biome/plains.json");
    let overworld = jar_json(&jar, "data/minecraft/dimension_type/overworld.json");
    let base_sky = hex_rgb(&plains["attributes"]["minecraft:visual/sky_color"]);
    let base_fog = hex_rgb(&overworld["attributes"]["minecraft:visual/fog_color"]);
    let mul = |a: [i32; 3], b: [i32; 3]| [0, 1, 2].map(|c| a[c] * b[c] / 255);
    // Looking north, 60° up: no sunrise tint (the camera faces neither the east
    // nor the west), the sun and moon 30° off the view at noon and midnight.
    // Along that ray the sky disc (height 16, rim 512) is hit at radius
    // 16 / tan 60°, so its fog is the vertex distances 16 and 512.25 mixed by
    // radius / 512, over FogSkyEnd = 128 (8 chunks).
    let s = 16.0 / 60f32.to_radians().tan() / 512.0;
    let disc_fog = (16.0 * (1.0 - s) + 512.249_94 * s) / 128.0;
    // The fog colour leans to the sky by 1 - lerp(min(512 / 16, 8) / 32, 0.25, 1)^0.25.
    let lean = 1.0 - (0.25f32 + 0.75 * 0.25).powf(0.25);
    for (name, ticks) in [("noon", 6000), ("dusk", 12500), ("midnight", 18000)] {
        let sky = mul(
            base_sky,
            day_track(&day, "minecraft:visual/sky_color", ticks),
        );
        let fog0 = mul(
            base_fog,
            day_track(&day, "minecraft:visual/fog_color", ticks),
        );
        let fog = [0, 1, 2].map(|c| fog0[c] + (lean * (sky[c] - fog0[c]) as f32).floor() as i32);
        let want = [0, 1, 2].map(|c| sky[c] as f32 + (fog[c] - sky[c]) as f32 * disc_fog);
        let mut req = request(&world, &jar, Mode::Quick);
        req.camera.yaw = 180.0;
        req.camera.pitch = -60.0;
        req.sky = Sky::Vanilla {
            time: Some(ticks),
            sun_dir: None,
            hq: HqSkyModel {
                sun_intensity: 1.5,
                ..HqSkyModel::CHUNKY
            },
        };
        let out = lumenwright::render(&req).unwrap();
        // The median of the centre 5×5 pixels (a star may sit on one).
        let (w, h) = (out.width as usize, out.height as usize);
        let got = [0, 1, 2].map(|c| {
            let mut v: Vec<u8> = (0..25)
                .map(|i| out.rgba[((h / 2 - 2 + i / 5) * w + w / 2 - 2 + i % 5) * 4 + c])
                .collect();
            v.sort();
            v[12]
        });
        eprintln!(
            "{name} ({ticks}): sky #{:02x}{:02x}{:02x}, fog #{:02x}{:02x}{:02x}; zenith-side pixel {got:?}, vanilla {want:.1?}",
            sky[0], sky[1], sky[2], fog[0], fog[1], fog[2]
        );
        for c in 0..3 {
            assert!(
                (got[c] as f32 - want[c]).abs() <= 2.0,
                "{name}: channel {c} is {} where the vanilla sky gives {:.1}",
                got[c],
                want[c]
            );
        }
    }
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
    let items = dir.join("items");
    fixture::item_gallery(true).write(&items).unwrap();
    // The item gallery's view (fixture::ITEM_GALLERY_VIEW).
    let item_eye = DVec3::new(7.5, 67.6, 9.5);
    let (yaw, pitch) = Camera::look_at(item_eye, DVec3::new(7.5, 66.0, 2.5));
    let views = [
        (
            &world,
            Camera {
                // The showcase's own player view (fixture::showcase).
                pos: DVec3::new(18.5, 72.0 + 1.62, 22.5),
                yaw: 150.0,
                pitch: 18.0,
                fov_y: 70.0,
            },
        ),
        (
            &items,
            Camera {
                pos: item_eye,
                yaw,
                pitch,
                fov_y: 60.0,
            },
        ),
    ];
    for ((world, camera), mode) in views.iter().flat_map(|v| [(v, Mode::Quick), (v, Mode::Hq)]) {
        let mut req = request(world, &jar, mode);
        req.camera = *camera;
        req.chunks = ChunkRect::around(req.camera.pos, RADIUS);
        let one = on_threads(1, &req);
        if world == &&items {
            assert!(
                one.entity_stats.items_rendered.values().sum::<usize>() > 0,
                "no item drawn"
            );
        }
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
