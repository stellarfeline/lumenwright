//! End-to-end tests on generated worlds. They need the vanilla client jar
//! (found like the CLI does, or via LUMENWRIGHT_MC_JAR) and skip without it;
//! GPU parity tests also skip when no GPU adapter is available.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use clap::Parser;
use lumenwright_cli::fixture;
use lumenwright_cli::jar;
use lumenwright_cli::job::{self, RenderArgs};
use lumenwright_cli::unifont;

#[derive(Parser)]
struct Wrap {
    #[command(flatten)]
    args: RenderArgs,
}

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
    let d = std::env::temp_dir().join(format!("lumenwright-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn showcase() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = tmp("showcase");
        fixture::showcase().write(&d).unwrap();
        d
    })
}

fn lineup() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = tmp("lineup");
        let ids: Vec<String> = ["pig", "zombie", "creeper", "armor_stand", "painting"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        fixture::lineup(&ids, 5).write(&d).unwrap();
        d
    })
}

fn gallery() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = tmp("block-entities");
        fixture::block_entity_gallery().write(&d).unwrap();
        d
    })
}

fn items_world(holding: bool) -> &'static Path {
    static WITH: OnceLock<PathBuf> = OnceLock::new();
    static WITHOUT: OnceLock<PathBuf> = OnceLock::new();
    let (cell, name) = if holding {
        (&WITH, "items")
    } else {
        (&WITHOUT, "items-empty")
    };
    cell.get_or_init(|| {
        let d = tmp(name);
        fixture::item_gallery(holding).write(&d).unwrap();
        d
    })
}

/// Projects a world point into a frame drawn from `cam` (pixel x, y).
fn project(
    cam: &lumenwright::scene::camera::Camera,
    p: [f64; 3],
    w: u32,
    h: u32,
) -> Option<(i32, i32)> {
    let (f, r, u) = cam.basis();
    let d = (lumenwright::glam::DVec3::from_array(p) - cam.pos).as_vec3();
    let z = d.dot(f);
    if z <= 0.0 {
        return None;
    }
    let t = (cam.fov_y.to_radians() * 0.5).tan();
    let x = d.dot(r) / (z * t * w as f32 / h as f32);
    let y = d.dot(u) / (z * t);
    Some((
        ((x + 1.0) * 0.5 * w as f32) as i32,
        ((1.0 - y) * 0.5 * h as f32) as i32,
    ))
}

/// The most any channel moves between two frames within `radius` of a pixel.
fn change_near(a: &image::RgbaImage, b: &image::RgbaImage, at: (i32, i32), radius: i32) -> i32 {
    let mut most = 0;
    for y in at.1 - radius..=at.1 + radius {
        for x in at.0 - radius..=at.0 + radius {
            if x < 0 || y < 0 || x >= a.width() as i32 || y >= a.height() as i32 {
                continue;
            }
            let (p, q) = (
                a.get_pixel(x as u32, y as u32),
                b.get_pixel(x as u32, y as u32),
            );
            for i in 0..3 {
                most = most.max((p.0[i] as i32 - q.0[i] as i32).abs());
            }
        }
    }
    most
}

/// Looks north over the three rows of the block-entity gallery.
const GALLERY_VIEW: [&str; 6] = [
    "--camera",
    "9.5,70.5,11.5",
    "--look-at",
    "9.5,65.4,-2",
    "--fov",
    "60",
];

const UNIFONT_HINT: &str =
    "the unifont: set LUMENWRIGHT_UNIFONT or run `lumenwright assets fetch-unifont 1.21.11`";

fn render(
    world: &Path,
    jar: &Path,
    out: &Path,
    extra: &[&str],
) -> anyhow::Result<job::RenderReport> {
    let mut argv: Vec<String> = vec![
        "t".into(),
        "--world".into(),
        world.display().to_string(),
        "--minecraft-jar".into(),
        jar.display().to_string(),
        "--unifont".into(),
        client_unifont().expect(UNIFONT_HINT).display().to_string(),
        "--offline".into(),
        "--output".into(),
        out.display().to_string(),
    ];
    argv.extend(extra.iter().map(|s| s.to_string()));
    job::run(&Wrap::parse_from(argv).args)
}

fn load(p: &Path) -> image::RgbaImage {
    image::open(p).unwrap().to_rgba8()
}

/// Fraction of pixels where `pred(r, g, b)` holds.
fn fraction(img: &image::RgbaImage, pred: impl Fn(f32, f32, f32) -> bool) -> f32 {
    let n = img
        .pixels()
        .filter(|p| pred(p.0[0] as f32, p.0[1] as f32, p.0[2] as f32))
        .count();
    n as f32 / (img.width() * img.height()) as f32
}

fn gpu_available() -> bool {
    lumenwright::render::create_backend(lumenwright::render::BackendKind::Gpu).is_ok()
}

#[test]
fn quick_top_down_colours_match_terrain() {
    let jar = require_jar!();
    let dir = tmp("colours");
    // Straight down onto open grassland: mostly green.
    let grass = dir.join("grass.png");
    render(
        showcase(),
        &jar,
        &grass,
        &[
            "--backend",
            "cpu",
            "--camera",
            "-28,90,-28",
            "--pitch",
            "90",
            "--fov",
            "20",
            "--width",
            "160",
            "--height",
            "90",
        ],
    )
    .unwrap();
    let img = load(&grass);
    let green = fraction(&img, |r, g, b| g > r && g > b);
    assert!(green > 0.6, "grassland should be green, got {green:.2}");
    // Straight down onto the lake: mostly blue.
    let lake = dir.join("lake.png");
    render(
        showcase(),
        &jar,
        &lake,
        &[
            "--backend",
            "cpu",
            "--camera",
            "14,90,12",
            "--pitch",
            "90",
            "--fov",
            "10",
            "--width",
            "160",
            "--height",
            "90",
        ],
    )
    .unwrap();
    let img = load(&lake);
    let blue = fraction(&img, |r, g, b| b > r && b > g);
    assert!(blue > 0.6, "lake should be blue, got {blue:.2}");
}

#[test]
fn quick_mode_draws_entities() {
    let jar = require_jar!();
    let dir = tmp("entities");
    let common = [
        "--backend",
        "cpu",
        "--camera",
        "7.5,67,7",
        "--look-at",
        "7.5,65.6,1.5",
        "--width",
        "320",
        "--height",
        "180",
    ];
    let with = dir.join("with.png");
    let rep = render(lineup(), &jar, &with, &common).unwrap();
    for id in ["pig", "zombie", "creeper", "armor_stand", "painting"] {
        assert_eq!(
            rep.entity_stats.rendered.get(id),
            Some(&1),
            "{id} not rendered: {:?}",
            rep.entity_stats
        );
    }
    let without = dir.join("without.png");
    let mut args = common.to_vec();
    args.push("--no-entities");
    render(lineup(), &jar, &without, &args).unwrap();
    let (a, b) = (load(&with), load(&without));
    let changed = a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count() as f32
        / (a.width() * a.height()) as f32;
    assert!(
        changed > 0.03,
        "entities should cover part of the frame, changed {changed:.3}"
    );
}

#[test]
fn hq_cpu_produces_a_lit_image() {
    let jar = require_jar!();
    let out = tmp("hq").join("hq.png");
    render(
        showcase(),
        &jar,
        &out,
        &[
            "--mode",
            "hq",
            "--backend",
            "cpu",
            "--spp",
            "4",
            "--width",
            "96",
            "--height",
            "54",
            "--sun-altitude",
            "40",
        ],
    )
    .unwrap();
    let img = load(&out);
    let mean = img
        .pixels()
        .map(|p| (p.0[0] as f32 + p.0[1] as f32 + p.0[2] as f32) / 3.0)
        .sum::<f32>()
        / (96.0 * 54.0);
    assert!(mean > 40.0 && mean < 240.0, "mean brightness {mean}");
    // The top rows look at the sky: blue dominates.
    let top_blue = (0..96).filter(|&x| {
        let p = img.get_pixel(x, 0).0;
        p[2] > p[0]
    });
    assert!(top_blue.count() > 80);
}

/// A 192×192 plane of gravel, the texture with the most fine detail, under
/// an empty sky.
fn gravel_plane() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = tmp("gravel-plane");
        let mut w = fixture::FixtureWorld::new();
        for x in -96..96 {
            for z in -96..96 {
                w.set(
                    x,
                    64,
                    z,
                    lumenwright::world::BlockState::new("minecraft:gravel"),
                );
            }
        }
        w.write(&d).unwrap();
        d
    })
}

/// Luma (0..255) of every pixel in rows `rows`.
fn luma_rows(img: &image::RgbaImage, rows: std::ops::Range<u32>) -> Vec<Vec<f32>> {
    rows.map(|y| {
        (0..img.width())
            .map(|x| {
                let p = img.get_pixel(x, y).0;
                0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
            })
            .collect()
    })
    .collect()
}

/// Mean squared 4-neighbour Laplacian: the image's high-frequency energy.
fn laplacian_energy(l: &[Vec<f32>]) -> f32 {
    let (mut sum, mut n) = (0f64, 0u64);
    for y in 1..l.len() - 1 {
        for x in 1..l[y].len() - 1 {
            let v = 4.0 * l[y][x] - l[y - 1][x] - l[y + 1][x] - l[y][x - 1] - l[y][x + 1];
            sum += (v * v) as f64;
            n += 1;
        }
    }
    (sum / n as f64) as f32
}

/// Mean absolute luma difference between two renders.
fn mean_abs_diff(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    let (mut sum, mut n) = (0f64, 0u64);
    for (ra, rb) in a.iter().zip(b) {
        for (p, q) in ra.iter().zip(rb) {
            sum += (p - q).abs() as f64;
            n += 1;
        }
    }
    (sum / n as f64) as f32
}

#[test]
fn mipmaps_calm_distant_textures() {
    let jar = require_jar!();
    let dir = tmp("mipmaps");
    // Three blocks over the plane, looking north 5.7° down: the horizon sits
    // at row ~77 and rows 79..100 see the plane from about 140 down to 17
    // blocks away, where one pixel spans 2 to 17 gravel texels across and
    // several times that along the view.
    let view = |x: f64, levels: u32, name: &str| {
        let out = dir.join(format!("{name}.png"));
        let cam = format!("{x},68.0,70.0");
        let at = format!("{x},62.0,10.0");
        let lv = levels.to_string();
        render(
            gravel_plane(),
            &jar,
            &out,
            &[
                "--backend",
                "cpu",
                "--width",
                "320",
                "--height",
                "180",
                "--camera",
                &cam,
                "--look-at",
                &at,
                "--render-distance",
                "10",
                "--time",
                "6000",
                "--mipmap-levels",
                &lv,
            ],
        )
        .unwrap();
        load(&out)
    };
    let band = 79..100;
    let mut measured = Vec::new();
    for levels in [0u32, 4] {
        let a = luma_rows(&view(0.5, levels, &format!("l{levels}")), band.clone());
        // A sideways step of 0.05 blocks: under a pixel everywhere in the band.
        let shifted = luma_rows(
            &view(0.55, levels, &format!("l{levels}-shift")),
            band.clone(),
        );
        let energy = laplacian_energy(&a);
        let shimmer = mean_abs_diff(&a, &shifted);
        eprintln!(
            "mipmap levels {levels}: high-frequency energy {energy:.1}, mean |luma change| under a 0.05-block shift {shimmer:.2}/255 (rows {band:?}, 320 px wide)"
        );
        measured.push((energy, shimmer));
    }
    let (off, on) = (measured[0], measured[1]);
    assert!(
        on.0 < off.0 * 0.5,
        "mipmaps should at least halve the far plane's high-frequency energy: {:.1} vs {:.1}",
        on.0,
        off.0
    );
    assert!(
        on.1 < off.1 * 0.5,
        "mipmaps should at least halve the shimmer under a sub-pixel move: {:.2} vs {:.2}",
        on.1,
        off.1
    );
}

#[test]
fn gpu_matches_cpu_in_quick_mode() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-quick");
    for (name, world, view) in [
        ("showcase", showcase(), &[][..]),
        ("block-entities", gallery(), &GALLERY_VIEW[..]),
        ("items", items_world(true), &fixture::ITEM_GALLERY_VIEW[..]),
    ] {
        let args = |b: &'static str| {
            let mut v = vec!["--backend", b, "--width", "320", "--height", "180"];
            v.extend_from_slice(view);
            v
        };
        let (cpu, gpu) = (
            dir.join(format!("{name}-cpu.png")),
            dir.join(format!("{name}-gpu.png")),
        );
        render(world, &jar, &cpu, &args("cpu")).unwrap();
        render(world, &jar, &gpu, &args("gpu")).unwrap();
        let (a, b) = (load(&cpu), load(&gpu));
        let off = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 8))
            .count() as f32
            / (a.width() * a.height()) as f32;
        eprintln!(
            "quick {name}: {:.3}% of pixels differ by more than 8/255 between CPU and GPU",
            off * 100.0
        );
        assert!(
            off < 0.01,
            "{name}: {:.3}% of pixels differ between CPU and GPU",
            off * 100.0
        );
    }
}

/// The parity frames' suns: a day sun; a dusk sun half a degree below the horizon, part
/// of its disc above it (the sky at full light and the sun's horizon test); and a low sun
/// at the centre of a narrow view, exposed so the sun texture drawn in the sky does not
/// clip.
/// Chunky's sky with a sun at a stated altitude.
const DAY_SUN: &[&str] = &["--sky", "chunky", "--sun-altitude", "40"];
const DUSK_SUN: &[&str] = &["--sky", "chunky", "--sun-altitude", "-0.5"];
const SUN_IN_VIEW: &[&str] = &[
    "--sky",
    "chunky",
    "--sun-altitude",
    "8",
    "--sun-azimuth",
    "330",
];
/// The game's sky in the morning and at the setting sun.
const GAME_DAY: &[&str] = &["--sky", "game", "--time", "4000"];
const GAME_DUSK: &[&str] = &["--sky", "game", "--time", "12800"];
/// A camera on the showcase looking at the [`SUN_IN_VIEW`] sun with a 12 degree view.
const SUN_VIEW: &[&str] = &[
    "--camera",
    "18.5,76,22.5",
    "--look-at",
    "13.55,77.39,13.92",
    "--fov",
    "12",
    "--exposure",
    "0.1",
];

#[test]
fn gpu_matches_cpu_in_hq_mode() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-hq");
    for (name, world, view, sun) in [
        ("showcase", showcase(), &[][..], DAY_SUN),
        ("showcase-dusk", showcase(), &[][..], DUSK_SUN),
        ("showcase-sun", showcase(), SUN_VIEW, SUN_IN_VIEW),
        ("showcase-game-day", showcase(), &[][..], GAME_DAY),
        ("showcase-game-dusk", showcase(), &[][..], GAME_DUSK),
        ("block-entities", gallery(), &GALLERY_VIEW[..], DAY_SUN),
        (
            "items",
            items_world(true),
            &fixture::ITEM_GALLERY_VIEW[..],
            DAY_SUN,
        ),
    ] {
        let args = |b: &'static str| {
            let mut v = vec![
                "--mode",
                "hq",
                "--backend",
                b,
                "--spp",
                "16",
                "--width",
                "160",
                "--height",
                "90",
            ];
            v.extend_from_slice(sun);
            v.extend_from_slice(view);
            v
        };
        let (cpu, gpu) = (
            dir.join(format!("{name}-cpu.png")),
            dir.join(format!("{name}-gpu.png")),
        );
        render(world, &jar, &cpu, &args("cpu")).unwrap();
        render(world, &jar, &gpu, &args("gpu")).unwrap();
        let (a, b) = (load(&cpu), load(&gpu));
        let lum = |img: &image::RgbaImage| {
            img.pixels()
                .map(|p| p.0[0] as f64 * 0.2126 + p.0[1] as f64 * 0.7152 + p.0[2] as f64 * 0.0722)
                .sum::<f64>()
        };
        let (la, lb) = (lum(&a), lum(&b));
        let rel = (la - lb).abs() / la.max(1.0);
        eprintln!(
            "hq {name}: mean luminance differs by {:.3}% between CPU and GPU",
            rel * 100.0
        );
        assert!(
            rel < 0.03,
            "{name}: mean luminance differs by {:.2}%",
            rel * 100.0
        );
        // Same RNG streams on both backends: almost every pixel should agree.
        let off = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 16))
            .count() as f32
            / (a.width() * a.height()) as f32;
        eprintln!(
            "hq {name}: {:.3}% of pixels differ by more than 16/255 between CPU and GPU",
            off * 100.0
        );
        assert!(off < 0.02, "{name}: {:.2}% of pixels differ", off * 100.0);
    }
}

/// A frame of nothing but sky is the same function of the pixel on every backend, so the
/// GPU's sky cache lookup must give the CPU's sky to within 2/255 everywhere, a bound the
/// path-traced frames above are too noisy to hold.
#[test]
fn gpu_draws_the_sky_as_the_cpu_does() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-sky");
    let args = |b: &'static str| {
        vec![
            "--mode",
            "hq",
            "--backend",
            b,
            "--spp",
            "4",
            "--width",
            "160",
            "--height",
            "90",
            "--camera",
            "18.5,120,22.5",
            "--look-at",
            "18.5,200,40",
            "--fov",
            "150",
            "--sky",
            "chunky",
            "--sun-altitude",
            "5",
            "--sun-azimuth",
            "0",
        ]
    };
    let (cpu, gpu) = (dir.join("sky-cpu.png"), dir.join("sky-gpu.png"));
    render(showcase(), &jar, &cpu, &args("cpu")).unwrap();
    render(showcase(), &jar, &gpu, &args("gpu")).unwrap();
    let (a, b) = (load(&cpu), load(&gpu));
    let worst = a
        .pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| (0..3).map(move |i| (p.0[i] as i32 - q.0[i] as i32).abs()))
        .max()
        .unwrap();
    let mean = a.pixels().map(|p| p.0[2] as f64).sum::<f64>() / (a.width() * a.height()) as f64;
    eprintln!(
        "hq sky: largest channel difference {worst}/255 between CPU and GPU (mean blue {mean:.1})"
    );
    assert!(mean > 60.0, "the frame shows sky: mean blue {mean:.1}");
    assert!(worst <= 2, "{worst}/255");
}

/// The game's sky is two maps the host draws and every backend reads the same way: a
/// frame that looks at the setting sun, its glow and the dusk sky (the lens's map) agrees
/// between the CPU and the GPU within 2/255 everywhere.
#[test]
fn gpu_draws_the_game_sky_as_the_cpu_does() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-game-sky");
    let args = |b: &'static str| {
        vec![
            "--mode",
            "hq",
            "--backend",
            b,
            "--spp",
            "4",
            "--width",
            "160",
            "--height",
            "90",
            "--camera",
            "18.5,120,22.5",
            "--look-at",
            "-80,125,22.5",
            "--fov",
            "90",
            "--sky",
            "game",
            "--time",
            "12800",
        ]
    };
    let (cpu, gpu) = (dir.join("sky-cpu.png"), dir.join("sky-gpu.png"));
    render(showcase(), &jar, &cpu, &args("cpu")).unwrap();
    render(showcase(), &jar, &gpu, &args("gpu")).unwrap();
    let (a, b) = (load(&cpu), load(&gpu));
    let worst = a
        .pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| (0..3).map(move |i| (p.0[i] as i32 - q.0[i] as i32).abs()))
        .max()
        .unwrap();
    let mean_red = a.pixels().map(|p| p.0[0] as f64).sum::<f64>() / (a.width() * a.height()) as f64;
    eprintln!(
        "hq game sky: largest channel difference {worst}/255 between CPU and GPU (mean red {mean_red:.1})"
    );
    assert!(
        mean_red > 60.0,
        "the frame shows the dusk sky: mean red {mean_red:.1}"
    );
    assert!(worst <= 2, "{worst}/255");
}

/// Chunky's sun and sky scales mean nothing under the game's sky (the default): either
/// flag is refused by name before anything is drawn, and is taken under `--sky chunky`.
#[test]
fn chunky_scales_are_refused_under_the_game_sky() {
    let jar = require_jar!();
    let dir = tmp("game-sky-flags");
    for flag in ["--sun-intensity", "--sky-intensity"] {
        let small = [
            "--mode",
            "hq",
            "--backend",
            "cpu",
            "--spp",
            "1",
            "--width",
            "16",
            "--height",
            "9",
        ];
        let mut args = small.to_vec();
        args.extend([flag, "2"]);
        let err = render(showcase(), &jar, &dir.join("refused.png"), &args)
            .err()
            .unwrap_or_else(|| panic!("{flag} was taken under the game's sky"));
        let msg = format!("{err:#}");
        assert!(msg.contains(flag) && msg.contains("--sky chunky"), "{msg}");
        args.extend(["--sky", "chunky"]);
        render(showcase(), &jar, &dir.join("taken.png"), &args).unwrap();
    }
}

/// The CUDA kernel is the CPU kernel compiled to PTX: an HQ frame agrees with
/// the CPU's within the GPU bounds above. Quick mode on CUDA is the CPU rasterizer.
#[cfg(feature = "cuda")]
#[test]
fn cuda_matches_cpu_in_hq_mode() {
    let jar = require_jar!();
    if let Err(e) = lumenwright::render::create_backend(lumenwright::render::BackendKind::Cuda) {
        eprintln!("skipping: no CUDA device ({e:#})");
        return;
    }
    let dir = tmp("parity-cuda");
    for (name, world, view, sun) in [
        ("showcase", showcase(), &[][..], DAY_SUN),
        ("showcase-dusk", showcase(), &[][..], DUSK_SUN),
        ("showcase-sun", showcase(), SUN_VIEW, SUN_IN_VIEW),
        ("showcase-game-day", showcase(), &[][..], GAME_DAY),
        ("showcase-game-dusk", showcase(), &[][..], GAME_DUSK),
        ("block-entities", gallery(), &GALLERY_VIEW[..], DAY_SUN),
        (
            "items",
            items_world(true),
            &fixture::ITEM_GALLERY_VIEW[..],
            DAY_SUN,
        ),
    ] {
        let args = |b: &'static str| {
            let mut v = vec![
                "--mode",
                "hq",
                "--backend",
                b,
                "--spp",
                "16",
                "--width",
                "160",
                "--height",
                "90",
            ];
            v.extend_from_slice(sun);
            v.extend_from_slice(view);
            v
        };
        let (cpu, gpu) = (
            dir.join(format!("{name}-cpu.png")),
            dir.join(format!("{name}-cuda.png")),
        );
        render(world, &jar, &cpu, &args("cpu")).unwrap();
        let rep = render(world, &jar, &gpu, &args("cuda")).unwrap();
        assert!(rep.backend.contains("cuda"), "{}", rep.backend);
        let (a, b) = (load(&cpu), load(&gpu));
        let lum = |img: &image::RgbaImage| {
            img.pixels()
                .map(|p| p.0[0] as f64 * 0.2126 + p.0[1] as f64 * 0.7152 + p.0[2] as f64 * 0.0722)
                .sum::<f64>()
        };
        let (la, lb) = (lum(&a), lum(&b));
        let rel = (la - lb).abs() / la.max(1.0);
        let off = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 16))
            .count() as f32
            / (a.width() * a.height()) as f32;
        eprintln!(
            "hq {name}: CUDA mean luminance differs by {:.3}%, {:.3}% of pixels by more than 16/255",
            rel * 100.0,
            off * 100.0
        );
        assert!(
            rel < 0.03,
            "{name}: mean luminance differs by {:.2}%",
            rel * 100.0
        );
        assert!(off < 0.02, "{name}: {:.2}% of pixels differ", off * 100.0);
    }
}

#[test]
fn cli_info_and_render() {
    let jar = require_jar!();
    let exe = env!("CARGO_BIN_EXE_lumenwright");
    let out = std::process::Command::new(exe)
        .args(["info"])
        .arg(showcase())
        .arg("--entities")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(fixture::VERSION_NAME), "{text}");
    assert!(text.contains("pig"), "{text}");

    let png = tmp("cli").join("cli.png");
    let st = std::process::Command::new(exe)
        .args([
            "--log",
            "warn",
            "render",
            "--backend",
            "cpu",
            "--width",
            "64",
            "--height",
            "36",
            "--offline",
        ])
        .arg("--world")
        .arg(showcase())
        .arg("--minecraft-jar")
        .arg(&jar)
        .arg("--unifont")
        .arg(client_unifont().expect(UNIFONT_HINT))
        .arg("--output")
        .arg(&png)
        .status()
        .unwrap();
    assert!(st.success());
    assert_eq!(load(&png).dimensions(), (64, 36));

    // Errors are reported, not panics.
    let bad = std::process::Command::new(exe)
        .args(["render", "--world", "/nonexistent/world"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a Minecraft world"));
}

#[test]
fn renders_are_deterministic() {
    let jar = require_jar!();
    let dir = tmp("determinism");
    for mode in ["quick", "hq"] {
        let args = [
            "--mode",
            mode,
            "--backend",
            "cpu",
            "--spp",
            "4",
            "--width",
            "120",
            "--height",
            "68",
            "--sun-altitude",
            "35",
        ];
        render(showcase(), &jar, &dir.join(format!("{mode}-a.png")), &args).unwrap();
        render(showcase(), &jar, &dir.join(format!("{mode}-b.png")), &args).unwrap();
        assert_eq!(
            load(&dir.join(format!("{mode}-a.png"))).into_raw(),
            load(&dir.join(format!("{mode}-b.png"))).into_raw(),
            "{mode} render changed between identical runs"
        );
    }
}

#[test]
fn block_entities_render_in_both_modes() {
    let jar = require_jar!();
    let dir = tmp("block-entity-modes");
    let expected = [
        ("sign", 4),
        ("hanging_sign", 3),
        ("bell", 4),
        ("lectern_book", 1),
        ("enchanting_table_book", 1),
        ("chest", 5),
        ("ender_chest", 1),
        ("banner", 2),
        ("decorated_pot", 1),
        ("skull", 7),
        ("player_head", 1),
    ];
    for mode in ["quick", "hq"] {
        let mut args = vec![
            "--mode",
            mode,
            "--backend",
            "cpu",
            "--spp",
            "4",
            "--width",
            "240",
            "--height",
            "135",
            "--sun-altitude",
            "45",
        ];
        args.extend_from_slice(&GALLERY_VIEW);
        let with = dir.join(format!("{mode}-with.png"));
        let rep = render(gallery(), &jar, &with, &args).unwrap();
        let st = &rep.block_entity_stats;
        for (kind, n) in expected {
            assert_eq!(st.rendered.get(kind), Some(&n), "{mode} {kind}: {st:?}");
        }
        // A barrel is a JSON model: nothing for the block-entity pass to draw.
        assert_eq!(st.skipped.get("barrel"), Some(&1), "{st:?}");
        assert_eq!(st.text_lines, 15, "{st:?}");
        assert!(st.glyphs > 100, "{st:?}");
        args.push("--no-block-entities");
        let without = dir.join(format!("{mode}-without.png"));
        let rep = render(gallery(), &jar, &without, &args).unwrap();
        assert!(rep.block_entity_stats.rendered.is_empty());
        let (a, b) = (load(&with), load(&without));
        let changed = a
            .pixels()
            .zip(b.pixels())
            .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 24))
            .count() as f32
            / (a.width() * a.height()) as f32;
        assert!(
            changed > 0.02,
            "{mode}: block entities should cover part of the frame, changed {changed:.3}"
        );
    }
}

#[test]
fn sign_text_reads_in_both_modes() {
    let jar = require_jar!();
    let dir = tmp("sign-text");
    // Close up on the oak sign's front: black text on light oak.
    for mode in ["quick", "hq"] {
        let args = [
            "--mode",
            mode,
            "--backend",
            "cpu",
            "--spp",
            "8",
            "--width",
            "200",
            "--height",
            "120",
            "--sun-altitude",
            "45",
            "--camera",
            "1.5,66.1,-1.4",
            "--look-at",
            "1.5,65.95,-2.5",
            "--fov",
            "50",
        ];
        let dark = |r: f32, g: f32, b: f32| r < 70.0 && g < 70.0 && b < 70.0;
        let red = |r: f32, g: f32, b: f32| r > g + 70.0 && r > b + 70.0;
        let out = dir.join(format!("{mode}.png"));
        render(gallery(), &jar, &out, &args).unwrap();
        let img = load(&out);
        // The board fills the middle of the frame; text strokes are its darkest pixels.
        let (d, r) = (fraction(&img, dark), fraction(&img, red));
        // The same frame without block entities.
        let mut bare = args.to_vec();
        bare.push("--no-block-entities");
        let out = dir.join(format!("{mode}-bare.png"));
        render(gallery(), &jar, &out, &bare).unwrap();
        let img = load(&out);
        let (d0, r0) = (fraction(&img, dark), fraction(&img, red));
        assert!(
            d > d0 + 0.006,
            "{mode}: text pixels {d:.4}, {d0:.4} without"
        );
        assert!(r > r0 + 0.002, "{mode}: red pixels {r:.4}, {r0:.4} without");
    }
}

#[test]
fn items_draw_in_both_modes() {
    let jar = require_jar!();
    let dir = tmp("item-modes");
    let kinds = [
        ("item", 2),
        ("item_display", 2),
        ("item_frame", 1),
        ("glow_item_frame", 1),
        ("armor_stand", 1),
        ("mannequin", 1),
        ("zombie", 1),
    ];
    let items = [
        "diamond",
        "bricks",
        "grass_block",
        "map",
        "lantern",
        "end_rod",
        "iron_helmet",
        "diamond_chestplate",
        "iron_sword",
        "shield",
        "golden_helmet",
        "leather_chestplate",
        "chainmail_leggings",
        "iron_boots",
        "diamond_axe",
        "carved_pumpkin",
        "stone_sword",
    ];
    for mode in ["quick", "hq"] {
        let mut args = vec![
            "--mode",
            mode,
            "--backend",
            "cpu",
            "--spp",
            "8",
            "--width",
            "640",
            "--height",
            "360",
            "--sun-altitude",
            "45",
        ];
        args.extend_from_slice(&fixture::ITEM_GALLERY_VIEW);
        let with = dir.join(format!("{mode}-with.png"));
        let rep = render(items_world(true), &jar, &with, &args).unwrap();
        let st = &rep.entity_stats;
        for (kind, n) in kinds {
            assert_eq!(st.rendered.get(kind), Some(&n), "{mode} {kind}: {st:?}");
        }
        assert!(
            st.skipped.is_empty() && st.fallback_boxes.is_empty(),
            "{st:?}"
        );
        for id in items {
            assert_eq!(st.items_rendered.get(id), Some(&1), "{mode} {id}: {st:?}");
        }
        assert_eq!(
            st.items_rendered.values().sum::<usize>(),
            items.len(),
            "{st:?}"
        );
        assert!(st.items_skipped.is_empty(), "{st:?}");
        // The same frame with the holders empty: every item's spot changes.
        let without = dir.join(format!("{mode}-without.png"));
        let bare = render(items_world(false), &jar, &without, &args).unwrap();
        assert!(bare.entity_stats.items_rendered.is_empty());
        let (a, b) = (load(&with), load(&without));
        let mut blank = Vec::new();
        for (spot, p) in fixture::ITEM_GALLERY_SPOTS {
            let at = project(&rep.camera, *p, a.width(), a.height())
                .unwrap_or_else(|| panic!("{spot} is behind the camera"));
            let moved = change_near(&a, &b, at, 3);
            if moved <= 24 {
                blank.push(format!("{spot} (pixel {at:?}, largest change {moved})"));
            }
        }
        assert!(blank.is_empty(), "{mode}: nothing drawn at {blank:?}");
    }
}

#[test]
fn every_item_definition_in_the_jar_resolves() {
    use lumenwright::assets::items::DisplayContext;
    use lumenwright::assets::items::ItemStack;
    use lumenwright::scene::items::ItemBakery;
    let jar = require_jar!();
    let store = lumenwright::assets::load_resources(&jar, client_unifont(), &[]).unwrap();
    let defs = store.paths_ending_with(".json");
    let defs: Vec<&String> = defs
        .iter()
        .filter(|p| p.starts_with("assets/minecraft/items/"))
        .collect();
    assert!(defs.len() > 1000, "{} item definitions", defs.len());
    let mut bakery = ItemBakery::new(&store);
    let (mut drawn, mut special) = (0, std::collections::BTreeMap::<String, usize>::new());
    for path in &defs {
        let name = path
            .trim_start_matches("assets/minecraft/items/")
            .trim_end_matches(".json");
        let stack = ItemStack {
            id: format!("minecraft:{name}"),
            count: 1,
            components: Default::default(),
        };
        for ctx in [
            DisplayContext::Ground,
            DisplayContext::Fixed,
            DisplayContext::ThirdPersonRightHand,
        ] {
            let p = bakery.prepare(&stack, ctx);
            for why in &p.undrawn {
                // Only the Java-drawn special models other than the shield go undrawn.
                let kind = why
                    .strip_prefix("special model ")
                    .unwrap_or_else(|| panic!("{name} ({}): {why}", ctx.name()));
                if ctx == DisplayContext::Fixed {
                    *special.entry(kind.to_string()).or_default() += 1;
                }
            }
            if ctx == DisplayContext::Fixed && p.drawn() {
                drawn += 1;
            }
        }
    }
    let undrawn: usize = special.values().sum();
    eprintln!(
        "{} item definitions: {drawn} drawn, {undrawn} special (not drawn): {special:?}",
        defs.len()
    );
    assert_eq!(drawn + undrawn, defs.len(), "{special:?}");
}
