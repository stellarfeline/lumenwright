//! Text displays, drawn through the library on the text-display gallery
//! (`fixture::text_display_gallery`): every property in both modes, each display's
//! frame against the same frame with no displays; CPU against the GPU and CUDA
//! backends; and CPU pixels across thread counts. Tests that draw need the vanilla
//! client jar (found like the CLI does, or via LUMENWRIGHT_MC_JAR) and skip without
//! it; GPU and CUDA tests also skip without the device.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use lumenwright::glam::DVec3;
use lumenwright::render::sky;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::scene::camera::Camera;
use lumenwright::world::Dimension;
use lumenwright::{
    ChunkRect, EntityPolicy, HqOptions, QuickOptions, Rendered, Request, Sky, WorldSource,
};
use lumenwright_cli::fixture::{self, TEXT_DISPLAYS, TextDisplayCase};
use lumenwright_cli::jar;

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
    let d = std::env::temp_dir().join(format!("lumenwright-text-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The gallery with its displays, and the same blocks without them.
fn worlds() -> &'static (PathBuf, PathBuf) {
    static DIRS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    DIRS.get_or_init(|| {
        let (a, b) = (tmp("with"), tmp("without"));
        fixture::text_display_gallery(true).write(&a).unwrap();
        fixture::text_display_gallery(false).write(&b).unwrap();
        (a, b)
    })
}

fn case(name: &str) -> &'static TextDisplayCase {
    TEXT_DISPLAYS.iter().find(|c| c.name == name).unwrap()
}

/// Where a camera looks at a display from.
#[derive(Clone, Copy)]
enum View {
    /// Three blocks south, level with the middle of a two-line text.
    Front,
    /// South and above, looking down at about 50 degrees.
    HighSouth,
    /// East and above.
    HighEast,
}

fn camera(c: &TextDisplayCase, view: View) -> Camera {
    let [x, y, z] = c.pos;
    let target = DVec3::new(x, y + 0.25, z);
    let eye = match view {
        View::Front => DVec3::new(x, y + 0.25, z + 3.0),
        View::HighSouth => DVec3::new(x, y + 3.0, z + 2.2),
        View::HighEast => DVec3::new(x + 2.2, y + 3.0, z),
    };
    let (yaw, pitch) = Camera::look_at(eye, target);
    Camera {
        pos: eye,
        yaw,
        pitch,
        fov_y: 50.0,
    }
}

const W: u32 = 192;
const H: u32 = 120;

fn request<'a>(world: &'a Path, jar: &'a Path, mode: Mode, cam: Camera) -> Request<'a> {
    Request {
        world: WorldSource::Dir {
            path: world,
            dimension: Dimension::Overworld,
        },
        chunks: ChunkRect::around(cam.pos, 3),
        jar,
        resource_packs: &[],
        camera: cam,
        mode,
        backend: BackendKind::Cpu,
        width: W,
        height: H,
        entities: EntityPolicy::default(),
        quick: QuickOptions {
            draw_distance: 48.0,
            ..QuickOptions::default()
        },
        hq: HqOptions {
            spp: 8,
            ..HqOptions::default()
        },
        sky: Sky::Vanilla {
            time: None,
            sun_dir: Some(sky::sun_direction(45.0, 160.0)),
            sun_intensity: 1.5,
            sky_intensity: 1.0,
        },
    }
}

/// RGB per pixel.
type Img = Vec<[f32; 3]>;

fn pixels(r: &Rendered) -> Img {
    r.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect()
}

/// A display's frame with and without the displays, and the stats of the first.
fn pair(jar: &Path, mode: Mode, name: &str, view: View) -> (Img, Img, Rendered) {
    let (with, without) = worlds();
    let cam = camera(case(name), view);
    let a = lumenwright::render(&request(with, jar, mode, cam)).unwrap();
    let b = lumenwright::render(&request(without, jar, mode, cam)).unwrap();
    // LUMENWRIGHT_TEXT_FRAMES=<dir> keeps the frames for a look.
    if let Ok(dir) = std::env::var("LUMENWRIGHT_TEXT_FRAMES") {
        for (r, tag) in [(&a, "with"), (&b, "without")] {
            let path = Path::new(&dir).join(format!("{name}-{mode:?}-{tag}.png"));
            image::RgbaImage::from_raw(W, H, r.rgba.clone())
                .unwrap()
                .save(path)
                .unwrap();
        }
    }
    assert_eq!(
        b.entity_stats.rendered.get("text_display"),
        None,
        "{:?}",
        b.entity_stats
    );
    (pixels(&a), pixels(&b), a)
}

fn luma(p: &[f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Pixel coordinates where `pred(with, without)` holds.
fn mask(a: &Img, b: &Img, pred: impl Fn(&[f32; 3], &[f32; 3]) -> bool) -> Vec<(u32, u32)> {
    a.iter()
        .zip(b)
        .enumerate()
        .filter(|(_, (p, q))| pred(p, q))
        .map(|(i, _)| (i as u32 % W, i as u32 / W))
        .collect()
}

fn fraction(m: &[(u32, u32)]) -> f32 {
    m.len() as f32 / (W * H) as f32
}

/// Pixels that changed by more than 24 in some channel.
fn changed(a: &Img, b: &Img) -> Vec<(u32, u32)> {
    mask(a, b, |p, q| (0..3).any(|i| (p[i] - q[i]).abs() > 24.0))
}

/// Text ink: pixels the display made much lighter (white text).
fn ink(a: &Img, b: &Img) -> Vec<(u32, u32)> {
    mask(a, b, |p, q| luma(p) > luma(q) + 50.0)
}

/// The bands of consecutive rows holding ink, with each band's x extent.
fn bands(m: &[(u32, u32)]) -> Vec<(u32, u32, u32, u32)> {
    let mut rows: Vec<Option<(u32, u32)>> = vec![None; H as usize];
    for &(x, y) in m {
        let r = &mut rows[y as usize];
        *r = Some(match *r {
            Some((lo, hi)) => (lo.min(x), hi.max(x)),
            None => (x, x),
        });
    }
    // (first row, last row, min x, max x)
    let mut out: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (y, r) in rows.iter().enumerate() {
        let y = y as u32;
        match (r, out.last_mut()) {
            (Some((lo, hi)), Some(b)) if b.1 + 1 == y => {
                b.1 = y;
                b.2 = b.2.min(*lo);
                b.3 = b.3.max(*hi);
            }
            (Some((lo, hi)), _) => out.push((y, y, *lo, *hi)),
            (None, _) => {}
        }
    }
    out
}

fn rows_spanned(m: &[(u32, u32)]) -> u32 {
    let lo = m.iter().map(|p| p.1).min().unwrap_or(0);
    let hi = m.iter().map(|p| p.1).max().unwrap_or(0);
    if m.is_empty() { 0 } else { hi - lo + 1 }
}

#[test]
fn every_text_display_is_drawn_in_both_modes() {
    let jar = require_jar!();
    for mode in [Mode::Quick, Mode::Hq] {
        for c in TEXT_DISPLAYS {
            let (a, b, r) = pair(&jar, mode, c.name, View::Front);
            let st = &r.entity_stats;
            // Every display in the loaded chunks is drawn.
            let rect = ChunkRect::around(camera(c, View::Front).pos, 3);
            let loaded = TEXT_DISPLAYS
                .iter()
                .filter(|d| {
                    let ch = [
                        (d.pos[0].floor() as i32) >> 4,
                        (d.pos[2].floor() as i32) >> 4,
                    ];
                    (0..2).all(|i| rect.min[i] <= ch[i] && ch[i] <= rect.max[i])
                })
                .count();
            assert!(loaded >= 1);
            assert_eq!(
                st.rendered.get("text_display"),
                Some(&loaded),
                "{mode:?} {}: {st:?}",
                c.name
            );
            assert!(
                !st.fallback_boxes.contains_key("text_display")
                    && !st.skipped.contains_key("text_display"),
                "{st:?}"
            );
            let f = fraction(&changed(&a, &b));
            // Behind the wall, or turned edge-on to this camera: nothing to see.
            let unseen = matches!(
                c.name,
                "hidden" | "billboard_fixed" | "billboard_horizontal"
            );
            if unseen {
                if mode == Mode::Quick {
                    assert!(f < 0.0005, "{mode:?} {}: {:.3}% changed", c.name, f * 100.0);
                }
            } else {
                assert!(f > 0.003, "{mode:?} {}: {:.3}% changed", c.name, f * 100.0);
            }
        }
    }
}

#[test]
fn lines_align_and_wrap() {
    let jar = require_jar!();
    let lines = |name: &str| {
        let (a, b, _) = pair(&jar, Mode::Quick, name, View::Front);
        bands(&ink(&a, &b))
    };
    for name in ["align_left", "align_center", "align_right"] {
        let l = lines(name);
        assert_eq!(l.len(), 2, "{name}: {l:?}");
        let (long, short) = (l[0], l[1]);
        assert!(short.3 - short.2 + 10 < long.3 - long.2, "{name}: {l:?}");
        let (dl, dr) = (
            short.2 as i32 - long.2 as i32,
            long.3 as i32 - short.3 as i32,
        );
        match name {
            "align_left" => assert!(dl.abs() <= 2 && dr > 10, "{name}: {l:?}"),
            "align_right" => assert!(dr.abs() <= 2 && dl > 10, "{name}: {l:?}"),
            _ => assert!((dl - dr).abs() <= 3 && dl > 5, "{name}: {l:?}"),
        }
    }
    // Forty pixels hold about six characters: the sentence takes five lines or more.
    let l = lines("wrapped");
    assert!(l.len() >= 5, "wrapped: {l:?}");
}

#[test]
fn backgrounds_shadows_and_see_through() {
    let jar = require_jar!();
    for mode in [Mode::Quick, Mode::Hq] {
        let (a, b, _) = pair(&jar, mode, "coloured_background", View::Front);
        let blue = |p: &[f32; 3]| p[2] > p[0] + 60.0 && p[2] > p[1] + 30.0;
        let with = mask(&a, &b, |p, _| blue(p));
        let without = mask(&a, &b, |_, q| blue(q));
        assert!(
            fraction(&with) > 0.01 && without.is_empty(),
            "{mode:?}: blue {:.3}% with, {} pixels without",
            fraction(&with) * 100.0,
            without.len()
        );
        // The default background (alpha 63) darkens the backdrop behind the text.
        let (a, b, _) = pair(&jar, mode, "default_background", View::Front);
        let darker = fraction(&mask(&a, &b, |p, q| luma(p) + 8.0 < luma(q)));
        assert!(darker > 0.01, "{mode:?}: default background {darker:.4}");
        // A shadow adds dark ink under the text; neither has a background.
        let dark = |name| {
            let (a, b, _) = pair(&jar, mode, name, View::Front);
            fraction(&mask(&a, &b, |p, q| luma(p) + 25.0 < luma(q)))
        };
        let (s, n) = (dark("shadow"), dark("no_shadow"));
        assert!(s > n + 0.002, "{mode:?}: shadow {s:.4}, none {n:.4}");
        // Behind the same wall, only the see-through display shows. (In HQ it is
        // lit where it stands, in the slot behind the wall, so it reads dark.)
        let (a, b, _) = pair(&jar, mode, "see_through", View::Front);
        let seen = fraction(&changed(&a, &b));
        let (a, b, _) = pair(&jar, mode, "hidden", View::Front);
        let hidden = fraction(&changed(&a, &b));
        eprintln!("{mode:?}: see-through {seen:.5}, hidden {hidden:.5}");
        assert!(seen > 0.005, "{mode:?}: see-through {seen:.4}");
        assert!(hidden < 0.0005, "{mode:?}: hidden {hidden:.4}");
    }
}

#[test]
fn billboards_turn_to_the_camera() {
    let jar = require_jar!();
    let ink_of = |name: &str, view: View| {
        let (a, b, _) = pair(&jar, Mode::Quick, name, view);
        ink(&a, &b)
    };
    // All four face east; the camera is in the south. Center and vertical turn
    // round; fixed and horizontal keep the yaw and are seen edge-on.
    for (name, seen) in [
        ("billboard_fixed", false),
        ("billboard_center", true),
        ("billboard_vertical", true),
        ("billboard_horizontal", false),
    ] {
        let f = fraction(&ink_of(name, View::Front));
        assert_eq!(f > 0.005, seen, "{name}: ink {f:.4}");
    }
    // From above, center also tilts to face the camera; vertical stays upright and
    // is foreshortened.
    let (c, v) = (
        rows_spanned(&ink_of("billboard_center", View::HighSouth)),
        rows_spanned(&ink_of("billboard_vertical", View::HighSouth)),
    );
    assert!(c * 10 > v * 13, "center {c} rows, vertical {v}");
    // From the east and above, horizontal tilts up to the camera; fixed does not.
    let (h, f) = (
        rows_spanned(&ink_of("billboard_horizontal", View::HighEast)),
        rows_spanned(&ink_of("billboard_fixed", View::HighEast)),
    );
    assert!(h * 10 > f * 13, "horizontal {h} rows, fixed {f}");
}

#[test]
fn transformation_scales_and_turns_the_text() {
    let jar = require_jar!();
    let ink_of = |name: &str| {
        let (a, b, _) = pair(&jar, Mode::Quick, name, View::Front);
        ink(&a, &b)
    };
    let (t, u) = (ink_of("transformed"), ink_of("untransformed"));
    assert!(
        t.len() * 10 > u.len() * 25,
        "ink: {} transformed, {} plain",
        t.len(),
        u.len()
    );
    // The principal axis of the ink is the turned baseline (image y points down).
    let n = t.len() as f32;
    let mx = t.iter().map(|p| p.0 as f32).sum::<f32>() / n;
    let my = t.iter().map(|p| p.1 as f32).sum::<f32>() / n;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for &(x, y) in &t {
        let (dx, dy) = (x as f32 - mx, my - y as f32);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let angle = (0.5 * (2.0 * sxy).atan2(sxx - syy)).to_degrees();
    assert!(
        (15.0..45.0).contains(&angle),
        "baseline at {angle:.1} degrees"
    );
}

#[test]
fn a_text_component_keeps_its_colours_and_bold() {
    let jar = require_jar!();
    for mode in [Mode::Quick, Mode::Hq] {
        let count = |name: &str| {
            let (a, b, _) = pair(&jar, mode, name, View::Front);
            let gold = mask(&a, &b, |p, q| {
                p[0] > 150.0 && p[1] > 90.0 && p[2] < 80.0 && p[0] > p[1] && luma(p) > luma(q)
            });
            let red = mask(&a, &b, |p, _| {
                p[0] > 120.0 && p[0] > p[1] + 50.0 && p[0] > p[2] + 50.0
            });
            (gold.len(), red.len())
        };
        let (gb, rb) = count("component");
        let (gp, rp) = count("component_plain");
        assert!(
            gb > 40 && rb > 20 && rp > 20,
            "{mode:?}: gold {gb}, red {rb}, {rp}"
        );
        // Bold draws each glyph twice, a pixel apart.
        assert!(gb * 10 > gp * 11, "{mode:?}: bold gold {gb}, plain {gp}");
    }
}

/// A wide view over four bays (default background, see-through, hidden, shadow).
fn overview() -> Camera {
    let eye = DVec3::new(33.0, 67.5, 9.0);
    let (yaw, pitch) = Camera::look_at(eye, DVec3::new(33.0, 66.2, 0.5));
    Camera {
        pos: eye,
        yaw,
        pitch,
        fov_y: 60.0,
    }
}

/// The frames the backend comparisons draw: the overview, and close-ups of the
/// see-through display behind its wall and of the coloured text component.
fn parity_frames() -> [(&'static str, Camera); 3] {
    [
        ("overview", overview()),
        ("see_through", camera(case("see_through"), View::Front)),
        ("component", camera(case("component"), View::Front)),
    ]
}

/// One frame on the CPU with and without the displays, and on `other`.
fn backend_frames(
    jar: &Path,
    mode: Mode,
    cam: Camera,
    other: BackendKind,
    spp: u32,
) -> (Img, Img, Img) {
    let (with, without) = worlds();
    let draw = |world: &Path, backend: BackendKind| {
        let mut req = request(world, jar, mode, cam);
        req.chunks = ChunkRect::around(cam.pos, 4);
        (req.width, req.height) = (480, 270);
        req.hq.spp = spp;
        req.backend = backend;
        lumenwright::render(&req).unwrap()
    };
    let (cpu, bare, dev) = (
        draw(with, BackendKind::Cpu),
        draw(without, BackendKind::Cpu),
        draw(with, other),
    );
    assert_eq!(
        cpu.entity_stats.rendered.get("text_display"),
        dev.entity_stats.rendered.get("text_display")
    );
    (pixels(&cpu), pixels(&bare), pixels(&dev))
}

fn differing(a: &Img, b: &Img, thr: f32) -> f32 {
    a.iter()
        .zip(b)
        .filter(|(p, q)| (0..3).any(|i| (p[i] - q[i]).abs() > thr))
        .count() as f32
        / a.len() as f32
}

fn mean_luma(a: &Img) -> f64 {
    a.iter().map(|p| luma(p) as f64).sum::<f64>() / a.len() as f64
}

/// Compares `other` with the CPU in `mode` on every parity frame, within the
/// bounds of the render tests (quick: under 1% of pixels off by more than 8; HQ:
/// mean luminance within 3% and under 2% of pixels off by more than 16), and on
/// the pixels the displays change on the CPU (the text), where under 5% may be off.
fn assert_parity(other: BackendKind, mode: Mode, spp: u32) {
    let thr = if mode == Mode::Quick { 8.0 } else { 16.0 };
    for (name, cam) in parity_frames() {
        let (cpu, bare, dev) = backend_frames(&client_jar().unwrap(), mode, cam, other, spp);
        let off = differing(&cpu, &dev, thr);
        let (la, lb) = (mean_luma(&cpu), mean_luma(&dev));
        let rel = (la - lb).abs() / la.max(1.0);
        let text: Vec<usize> = cpu
            .iter()
            .zip(&bare)
            .enumerate()
            .filter(|(_, (p, q))| (0..3).any(|i| (p[i] - q[i]).abs() > 24.0))
            .map(|(i, _)| i)
            .collect();
        let text_off = text
            .iter()
            .filter(|&&i| (0..3).any(|c| (cpu[i][c] - dev[i][c]).abs() > thr))
            .count();
        eprintln!(
            "{other:?} {mode:?} {name} at {spp} spp: {:.4}% of pixels off by more than {thr}, \
             mean luminance {la:.3} vs {lb:.3} ({:.4}%), text pixels off {text_off} of {}",
            off * 100.0,
            rel * 100.0,
            text.len()
        );
        assert!(!text.is_empty(), "{name}: the displays change no pixel");
        if mode == Mode::Quick {
            assert!(off < 0.01, "{name}: {:.3}%", off * 100.0);
        } else {
            assert!(rel < 0.03, "{name}: luminance {:.2}%", rel * 100.0);
            assert!(off < 0.02, "{name}: {:.2}% of pixels", off * 100.0);
        }
        assert!(
            text_off * 20 < text.len(),
            "{name}: {text_off} of {} text pixels off",
            text.len()
        );
    }
}

#[test]
fn gpu_matches_cpu_on_text_displays() {
    let _jar = require_jar!();
    if lumenwright::render::create_backend(BackendKind::Gpu).is_err() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    assert_parity(BackendKind::Gpu, Mode::Quick, 1);
    assert_parity(BackendKind::Gpu, Mode::Hq, 16);
}

#[cfg(feature = "cuda")]
#[test]
fn cuda_matches_cpu_on_text_displays() {
    let _jar = require_jar!();
    if lumenwright::render::create_backend(BackendKind::Cuda).is_err() {
        eprintln!("skipping: no CUDA device");
        return;
    }
    // HQ runs the shared kernel as PTX (CUDA quick mode is the CPU rasterizer).
    // LUMENWRIGHT_CUDA_SPP sets the samples per pixel.
    let spp = std::env::var("LUMENWRIGHT_CUDA_SPP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    assert_parity(BackendKind::Cuda, Mode::Hq, spp);
}

#[test]
fn cpu_text_displays_are_identical_across_thread_counts() {
    let jar = require_jar!();
    let (with, _) = worlds();
    for mode in [Mode::Quick, Mode::Hq] {
        let mut req = request(with, &jar, mode, overview());
        req.chunks = ChunkRect::around(req.camera.pos, 4);
        let on = |n: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
                .install(|| lumenwright::render(&req).unwrap())
        };
        let one = on(1);
        assert!(
            one.entity_stats.rendered.get("text_display").copied() > Some(5),
            "{:?}",
            one.entity_stats
        );
        for n in [3, 8] {
            let many = on(n);
            assert!(
                one.rgba == many.rgba,
                "{mode:?}: pixels differ on {n} threads"
            );
            let bits = |r: &Rendered| {
                r.hdr
                    .as_ref()
                    .map(|h| h.iter().flatten().map(|f| f.to_bits()).collect::<Vec<_>>())
            };
            assert_eq!(bits(&one), bits(&many), "{mode:?}: radiance on {n} threads");
        }
    }
}
