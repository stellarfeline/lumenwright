//! Chinese text on a sign and on a text display, drawn from the Unifont the client
//! reads out of the launcher's asset index: against the same frame with no Unifont
//! (what the jar alone draws: nothing), against the bitmaps in the hex file, and with a
//! Unifont a caller supplies itself. A missing or mismatched Unifont is refused by name,
//! in the library and in the command, with no network. Tests that draw need the vanilla
//! client jar (LUMENWRIGHT_MC_JAR or the cache) and the Unifont (LUMENWRIGHT_UNIFONT or
//! the cache; `lumenwright assets fetch-unifont 1.21.11` puts it there).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use lumenwright::glam::DVec3;
use lumenwright::render::{BackendKind, Mode};
use lumenwright::scene::camera::Camera;
use lumenwright::world::Dimension;
use lumenwright::{
    ChunkRect, EntityPolicy, HqOptions, QuickOptions, Rendered, Request, RequestError, Sky,
    WorldSource,
};
use lumenwright_cli::fixture::{self, CHINESE_DISPLAY, CHINESE_SIGN, CHINESE_TEXT};
use lumenwright_cli::{jar, unifont};

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

/// The real Unifont directory. A test that has the jar and not this fails, naming the
/// remedy: a skipped test would pass without drawing a character.
fn real_unifont() -> PathBuf {
    if let Ok(p) = std::env::var("LUMENWRIGHT_UNIFONT") {
        return PathBuf::from(p);
    }
    unifont::resolve_unifont(&unifont::UnifontRequest {
        explicit: None,
        version: Some(fixture::VERSION_NAME),
        allow_download: false,
    })
    .expect(
        "the unifont: set LUMENWRIGHT_UNIFONT or run `lumenwright assets fetch-unifont 1.21.11`",
    )
}

/// A fresh directory for one call: the tests run in parallel in one process, and several
/// of them write the same fixture worlds, so each call gets its own number.
fn tmp(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "lumenwright-unifont-{}-{n}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const W: u32 = 480;
const H: u32 = 270;
const FOV: f32 = 50.0;
/// Blocks from the camera to the text.
const DISTANCE: f64 = 1.6;

/// Looking at the display's text from the south.
fn display_camera() -> Camera {
    let [x, y, z] = CHINESE_DISPLAY;
    let eye = DVec3::new(x, y + 0.12, z + DISTANCE);
    let (yaw, pitch) = Camera::look_at(eye, DVec3::new(x, y + 0.12, z));
    Camera {
        pos: eye,
        yaw,
        pitch,
        fov_y: FOV,
    }
}

/// Looking at the sign's board from the south.
fn sign_camera() -> Camera {
    let [x, y, z] = CHINESE_SIGN;
    let target = DVec3::new(x as f64 + 0.5, y as f64 + 0.66, z as f64 + 0.5);
    let eye = target + DVec3::new(0.0, 0.0, 1.4);
    let (yaw, pitch) = Camera::look_at(eye, target);
    Camera {
        pos: eye,
        yaw,
        pitch,
        fov_y: FOV,
    }
}

fn request<'a>(
    world: &'a Path,
    jar: &'a Path,
    unifont: Option<&'a Path>,
    cam: Camera,
) -> Request<'a> {
    Request {
        world: WorldSource::Dir {
            path: world,
            dimension: Dimension::Overworld,
        },
        chunks: ChunkRect::around(cam.pos, 2),
        jar,
        unifont,
        resource_packs: &[],
        camera: cam,
        mode: Mode::Quick,
        backend: BackendKind::Cpu,
        width: W,
        height: H,
        entities: EntityPolicy::default(),
        quick: QuickOptions {
            draw_distance: 32.0,
            ..QuickOptions::default()
        },
        hq: HqOptions::default(),
        sky: Sky::default(),
    }
}

fn worlds() -> (PathBuf, PathBuf) {
    let (a, b) = (tmp("with-text"), tmp("without-text"));
    fixture::chinese_text_gallery(true).write(&a).unwrap();
    fixture::chinese_text_gallery(false).write(&b).unwrap();
    (a, b)
}

fn luma(p: &[u8]) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

/// Pixels where `a` is lighter than `b` by more than 50 (white ink on a dark backdrop).
fn lighter(a: &Rendered, b: &Rendered) -> usize {
    a.rgba
        .chunks(4)
        .zip(b.rgba.chunks(4))
        .filter(|(p, q)| luma(p) > luma(q) + 50.0)
        .count()
}

/// Pixels where `a` is darker than `b` by more than 50 (black ink on a light board).
fn darker(a: &Rendered, b: &Rendered) -> usize {
    lighter(b, a)
}

fn save(r: &Rendered, name: &str) {
    if let Ok(dir) = std::env::var("LUMENWRIGHT_UNIFONT_FRAMES") {
        let path = Path::new(&dir).join(format!("{name}.png"));
        image::RgbaImage::from_raw(r.width, r.height, r.rgba.clone())
            .unwrap()
            .save(path)
            .unwrap();
    }
}

#[test]
fn chinese_text_is_drawn_on_a_display_and_on_a_sign() {
    let jar = require_jar!();
    let unifont = real_unifont();
    let (with, without) = worlds();
    // The display: white text on a dark backdrop. "Before" is a render with no
    // Unifont, which is what the jar alone gives.
    let cam = display_camera();
    let after = lumenwright::render(&request(&with, &jar, Some(&unifont), cam)).unwrap();
    let before = lumenwright::render(&request(&with, &jar, None, cam)).unwrap();
    let bare = lumenwright::render(&request(&without, &jar, Some(&unifont), cam)).unwrap();
    save(&before, "display-before");
    save(&after, "display-after");
    let (ink_before, ink_after) = (lighter(&before, &bare), lighter(&after, &bare));
    eprintln!("display glyph pixels: before {ink_before}, after {ink_after}");
    assert_eq!(ink_before, 0, "no Unifont draws no Chinese");
    assert!(ink_after > 1000, "{ink_after} pixels of Chinese text");
    // The sign: black text on a white-concrete backdrop and a pale board.
    let cam = sign_camera();
    let after = lumenwright::render(&request(&with, &jar, Some(&unifont), cam)).unwrap();
    let before = lumenwright::render(&request(&with, &jar, None, cam)).unwrap();
    let bare = lumenwright::render(&request(&without, &jar, Some(&unifont), cam)).unwrap();
    save(&before, "sign-before");
    save(&after, "sign-after");
    let (ink_before, ink_after) = (darker(&before, &bare), darker(&after, &bare));
    eprintln!("sign glyph pixels: before {ink_before}, after {ink_after}");
    assert_eq!(ink_before, 0, "no Unifont draws no Chinese");
    assert!(ink_after > 200, "{ink_after} pixels of Chinese text");
    // One quad per character on the sign line.
    assert_eq!(before.block_entity_stats.glyphs, 0);
    assert_eq!(
        after.block_entity_stats.glyphs,
        CHINESE_TEXT.chars().count(),
        "{:?}",
        after.block_entity_stats
    );
}

/// The lit texels of each character in the hex file, counted from its line of hex
/// digits with no help from the renderer's own decoder.
fn lit_texels(unifont: &Path, chars: &str) -> Vec<usize> {
    let f = std::fs::File::open(unifont.join("minecraft/font/unifont.zip")).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut text = String::new();
    let name = (0..zip.len())
        .filter_map(|i| Some(zip.name_for_index(i)?.ok()?.to_string()))
        .find(|n| n.ends_with(".hex"))
        .unwrap();
    std::io::Read::read_to_string(&mut zip.by_name(&name).unwrap(), &mut text).unwrap();
    chars
        .chars()
        .map(|c| {
            let key = format!("{:04X}:", c as u32);
            let line = text.lines().find(|l| l.starts_with(&key)).unwrap();
            line[key.len()..]
                .chars()
                .map(|d| d.to_digit(16).unwrap().count_ones() as usize)
                .sum()
        })
        .collect()
}

#[test]
fn the_ink_follows_the_bitmaps_of_the_hex_file() {
    let jar = require_jar!();
    let unifont = real_unifont();
    let (with, without) = worlds();
    let cam = display_camera();
    let after = lumenwright::render(&request(&with, &jar, Some(&unifont), cam)).unwrap();
    let bare = lumenwright::render(&request(&without, &jar, Some(&unifont), cam)).unwrap();
    let ink = lighter(&after, &bare) as f64;
    // A texel is half a font pixel, a font pixel is 0.025 blocks, and the camera sees
    // `H / (2 d tan(fov / 2))` pixels per block at distance d.
    let per_block = H as f64 / (2.0 * DISTANCE * (FOV as f64 / 2.0).to_radians().tan());
    let texel = 0.0125 * per_block;
    let lit: usize = lit_texels(&unifont, CHINESE_TEXT).iter().sum();
    let expected = lit as f64 * texel * texel;
    eprintln!("ink {ink}, expected {expected:.0} from {lit} lit texels of {texel:.2} pixels");
    assert!(
        (0.8..1.2).contains(&(ink / expected)),
        "ink {ink} against {expected:.0} expected from {lit} lit texels"
    );
}

/// A Unifont directory a caller makes itself: solid 16 by 16 blocks for `chars`.
fn solid_unifont(dir: &Path, chars: &str) {
    let font = dir.join("minecraft/font");
    std::fs::create_dir_all(font.join("include")).unwrap();
    std::fs::write(
        font.join("include/unifont.json"),
        r#"{"providers":[{"type":"unihex","hex_file":"minecraft:font/unifont.zip"}]}"#,
    )
    .unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(font.join("unifont.zip")).unwrap());
    zip.start_file("solid.hex", zip::write::SimpleFileOptions::default())
        .unwrap();
    for c in chars.chars() {
        writeln!(zip, "{:04X}:{}", c as u32, "F".repeat(64)).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn a_caller_supplies_the_unifont_itself_with_no_network() {
    let jar = require_jar!();
    let dir = tmp("solid");
    solid_unifont(&dir, CHINESE_TEXT);
    let (with, without) = worlds();
    let cam = display_camera();
    let after = lumenwright::render(&request(&with, &jar, Some(&dir), cam)).unwrap();
    let bare = lumenwright::render(&request(&without, &jar, Some(&dir), cam)).unwrap();
    // Four solid blocks of 8 font pixels, one font pixel apart: along the middle row of
    // the ink, four runs of 8 units separated by gaps of 1.
    let mut rows = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (i, (p, q)) in after.rgba.chunks(4).zip(bare.rgba.chunks(4)).enumerate() {
        if luma(p) > luma(q) + 50.0 {
            rows.entry(i / W as usize).or_default().push(i % W as usize);
        }
    }
    assert!(!rows.is_empty(), "no ink");
    let mid = *rows.keys().nth(rows.len() / 2).unwrap();
    let cols = &rows[&mid];
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &x in cols {
        match runs.last_mut() {
            Some(r) if x == r.1 + 1 => r.1 = x,
            _ => runs.push((x, x)),
        }
    }
    assert_eq!(runs.len(), 4, "runs {runs:?}");
    let unit = (runs[3].1 + 1 - runs[0].0) as f64 / 35.0;
    for (i, r) in runs.iter().enumerate() {
        let w = (r.1 + 1 - r.0) as f64 / unit;
        assert!(
            (7.0..9.0).contains(&w),
            "block {i} is {w:.1} font pixels wide"
        );
        if i > 0 {
            let gap = (r.0 - runs[i - 1].1 - 1) as f64 / unit;
            assert!((0.4..1.8).contains(&gap), "gap {i} is {gap:.1} font pixels");
        }
    }
}

#[test]
fn a_missing_or_mismatched_unifont_is_refused_by_name() {
    // The "jar" only has to be a file: the request is refused before it is read, and
    // neither the world nor any network is touched.
    let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let world = Path::new("/nonexistent/lumenwright/world");
    let refused = |dir: &Path| -> RequestError {
        let err = match lumenwright::render(&request(world, &jar, Some(dir), display_camera())) {
            Ok(_) => panic!("rendered with the unifont {}", dir.display()),
            Err(e) => e,
        };
        err.downcast_ref::<RequestError>()
            .unwrap_or_else(|| panic!("{err:#}"))
            .clone()
    };
    use lumenwright::assets::unifont::UnifontError::{Invalid, Missing};
    let include = "minecraft/font/include/unifont.json";

    // No directory at all.
    let dir = tmp("refused").join("nothing");
    assert_eq!(
        refused(&dir),
        RequestError::Unifont(Missing(dir.join(include)))
    );

    // The provider list is there but the hex zip it names is not.
    solid_unifont(&dir, "收");
    std::fs::remove_file(dir.join("minecraft/font/unifont.zip")).unwrap();
    let zip = dir.join("minecraft/font/unifont.zip");
    assert_eq!(refused(&dir), RequestError::Unifont(Missing(zip.clone())));

    // The file is there and is not a zip.
    std::fs::write(&zip, b"not a zip").unwrap();
    match refused(&dir) {
        RequestError::Unifont(Invalid { path, why }) => {
            assert_eq!(path, zip);
            assert!(why.contains("not a zip"), "{why}");
        }
        e => panic!("{e}"),
    }

    // A zip that holds something else.
    let mut z = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    z.start_file("readme.txt", zip::write::SimpleFileOptions::default())
        .unwrap();
    z.finish().unwrap();
    match refused(&dir) {
        RequestError::Unifont(Invalid { path, why }) => {
            assert_eq!(path, zip);
            assert!(why.contains("no .hex"), "{why}");
        }
        e => panic!("{e}"),
    }

    // A hex file whose first line is not a glyph.
    let mut z = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    z.start_file("a.hex", zip::write::SimpleFileOptions::default())
        .unwrap();
    z.write_all(b"4E00:ABC\n").unwrap();
    z.finish().unwrap();
    match refused(&dir) {
        RequestError::Unifont(Invalid { path, why }) => {
            assert_eq!(path, zip);
            assert!(why.contains("a.hex line 1"), "{why}");
        }
        e => panic!("{e}"),
    }
    // The message names the file.
    assert!(refused(&dir).to_string().contains("unifont.zip"));
}

/// The command, offline, with a cache that holds nothing and no `--unifont`.
fn render_offline(world: &Path, cache: &Path, unifont: Option<&Path>) -> std::process::Output {
    let jar = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lumenwright"));
    cmd.env("LUMENWRIGHT_CACHE", cache)
        .args(["--log", "warn", "render", "--offline", "--backend", "cpu"])
        .arg("--world")
        .arg(world)
        .arg("--minecraft-jar")
        .arg(&jar)
        .arg("--output")
        .arg(cache.join("never.png"));
    if let Some(u) = unifont {
        cmd.arg("--unifont").arg(u);
    }
    cmd.output().unwrap()
}

#[test]
fn the_command_refuses_offline_without_a_unifont_and_names_it() {
    let dir = tmp("cli-offline");
    let world = dir.join("world");
    fixture::chinese_text_gallery(true).write(&world).unwrap();
    let cache = dir.join("cache");

    // Nothing cached, nothing given, no network allowed.
    let out = render_offline(&world, &cache, None);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("no usable unifont"), "{err}");
    assert!(
        err.contains("--offline") && err.contains("--unifont"),
        "{err}"
    );
    assert!(!cache.join("never.png").exists());

    // A directory given with a corrupt hex zip is refused by the file's name.
    let bad = dir.join("bad");
    solid_unifont(&bad, "收");
    std::fs::write(bad.join("minecraft/font/unifont.zip"), b"corrupt").unwrap();
    let out = render_offline(&world, &cache, Some(&bad));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("--unifont"), "{err}");
    assert!(
        err.contains(&bad.join("minecraft/font/unifont.zip").display().to_string()),
        "{err}"
    );
}
