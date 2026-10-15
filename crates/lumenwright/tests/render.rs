//! End-to-end tests on generated worlds. They need the vanilla client jar
//! (found like the CLI does, or via LUMENWRIGHT_MC_JAR) and skip without it;
//! GPU parity tests also skip when no GPU adapter is available.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use clap::Parser;
use lumenwright::assets::jar;
use lumenwright::fixture;
use lumenwright::job::{self, RenderArgs};

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

#[test]
fn gpu_matches_cpu_in_quick_mode() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-quick");
    let args = |b: &'static str| vec!["--backend", b, "--width", "320", "--height", "180"];
    render(showcase(), &jar, &dir.join("cpu.png"), &args("cpu")).unwrap();
    render(showcase(), &jar, &dir.join("gpu.png"), &args("gpu")).unwrap();
    let (a, b) = (load(&dir.join("cpu.png")), load(&dir.join("gpu.png")));
    let off = a
        .pixels()
        .zip(b.pixels())
        .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 8))
        .count() as f32
        / (a.width() * a.height()) as f32;
    assert!(
        off < 0.01,
        "{:.3}% of pixels differ between CPU and GPU",
        off * 100.0
    );
}

#[test]
fn gpu_matches_cpu_in_hq_mode() {
    let jar = require_jar!();
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = tmp("parity-hq");
    let args = |b: &'static str| {
        vec![
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
            "--sun-altitude",
            "40",
        ]
    };
    render(showcase(), &jar, &dir.join("cpu.png"), &args("cpu")).unwrap();
    render(showcase(), &jar, &dir.join("gpu.png"), &args("gpu")).unwrap();
    let (a, b) = (load(&dir.join("cpu.png")), load(&dir.join("gpu.png")));
    let lum = |img: &image::RgbaImage| {
        img.pixels()
            .map(|p| p.0[0] as f64 * 0.2126 + p.0[1] as f64 * 0.7152 + p.0[2] as f64 * 0.0722)
            .sum::<f64>()
    };
    let (la, lb) = (lum(&a), lum(&b));
    let rel = (la - lb).abs() / la.max(1.0);
    assert!(rel < 0.03, "mean luminance differs by {:.2}%", rel * 100.0);
    // Same RNG streams on both backends: almost every pixel should agree.
    let off = a
        .pixels()
        .zip(b.pixels())
        .filter(|(p, q)| (0..3).any(|i| (p.0[i] as i32 - q.0[i] as i32).abs() > 16))
        .count() as f32
        / (a.width() * a.height()) as f32;
    assert!(off < 0.02, "{:.2}% of pixels differ", off * 100.0);
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
