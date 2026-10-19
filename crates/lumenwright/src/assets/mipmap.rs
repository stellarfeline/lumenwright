//! Mip chains for block textures, as the 1.21.11 client builds them.
//!
//! A port of `net.minecraft.client.renderer.texture.MipmapGenerator` (with the
//! `ARGB.meanLinear` box filter and `TextureUtil.solidify` /
//! `fillEmptyAreasWithDarkColor`): each level halves the previous one with a 2×2
//! mean taken in linear light through the client's own 10-bit lookup tables, and
//! the alpha of cutout textures is rescaled per level so the share of texels that
//! pass the 0.5 alpha test (0.3 for `strict_cutout`) stays what it is at level 0.
//! The strategy and the alpha bias come from the texture's `.mcmeta`
//! (`texture.mipmap_strategy`, `texture.alpha_cutoff_bias`), as in
//! `SpriteContents`.
//!
//! Pixels are RGBA8 with red in the low byte; every operation here treats the
//! three colour channels alike, so the client's ARGB order does not matter.

use std::sync::OnceLock;

/// How a texture's mip levels are made (`MipmapStrategy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipmapStrategy {
    /// `cutout` when some texel is fully transparent, else `mean`.
    Auto,
    /// Plain linear-light mean.
    Mean,
    /// Mean, with transparent texels recoloured from their nearest opaque
    /// neighbour and the alpha-test coverage kept at the 0.5 cutoff.
    Cutout,
    /// As `Cutout`, with the coverage kept at a 0.3 cutoff.
    StrictCutout,
    /// Transparent texels take a darkened copy of the darkest opaque colour, and
    /// levels average only the non-transparent texels' contributions.
    DarkCutout,
}

impl MipmapStrategy {
    /// The strategy named in a `.mcmeta` `texture.mipmap_strategy` field.
    pub fn from_name(s: &str) -> Option<MipmapStrategy> {
        Some(match s {
            "auto" => MipmapStrategy::Auto,
            "mean" => MipmapStrategy::Mean,
            "cutout" => MipmapStrategy::Cutout,
            "strict_cutout" => MipmapStrategy::StrictCutout,
            "dark_cutout" => MipmapStrategy::DarkCutout,
            _ => return None,
        })
    }
}

/// The `texture` section of a texture's `.mcmeta`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureMeta {
    /// `mipmap_strategy`, default `auto`.
    pub strategy: MipmapStrategy,
    /// `alpha_cutoff_bias`, default 0.
    pub alpha_cutoff_bias: f32,
}

impl Default for TextureMeta {
    fn default() -> Self {
        TextureMeta {
            strategy: MipmapStrategy::Auto,
            alpha_cutoff_bias: 0.0,
        }
    }
}

impl TextureMeta {
    /// Reads the `texture` section of a `.mcmeta` file; anything missing or
    /// unreadable keeps its default.
    pub fn parse(mcmeta: &[u8]) -> TextureMeta {
        let mut m = TextureMeta::default();
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(mcmeta) else {
            return m;
        };
        let t = &v["texture"];
        if let Some(s) = t["mipmap_strategy"].as_str() {
            match MipmapStrategy::from_name(s) {
                Some(st) => m.strategy = st,
                None => log::warn!("unknown mipmap_strategy {s:?}"),
            }
        }
        if let Some(b) = t["alpha_cutoff_bias"].as_f64() {
            m.alpha_cutoff_bias = b as f32;
        }
        m
    }
}

/// One image of a mip chain: RGBA8, red in the low byte, row major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipImage {
    /// Width in texels.
    pub w: u32,
    /// Height in texels.
    pub h: u32,
    /// The texels.
    pub px: Vec<u32>,
}

impl MipImage {
    fn get(&self, x: u32, y: u32) -> u32 {
        self.px[(y * self.w + x) as usize]
    }
    fn set(&mut self, x: u32, y: u32, c: u32) {
        self.px[(y * self.w + x) as usize] = c;
    }
}

#[inline]
fn alpha(c: u32) -> u32 {
    c >> 24
}
#[inline]
fn ch(c: u32, i: u32) -> u32 {
    (c >> (8 * i)) & 0xFF
}
#[inline]
fn with_alpha(a: u32, c: u32) -> u32 {
    (a << 24) | (c & 0x00FF_FFFF)
}
#[inline]
fn rgba(c: [u32; 3], a: u32) -> u32 {
    c[0] | (c[1] << 8) | (c[2] << 16) | (a << 24)
}
/// `ARGB.as8BitChannel`.
#[inline]
fn as_8bit(f: f32) -> u32 {
    ((f * 255.0).floor() as i32).clamp(0, 255) as u32
}
/// `ARGB.alphaFloat`.
#[inline]
fn alpha_f(c: u32) -> f32 {
    alpha(c) as f32 / 255.0
}

struct Tables {
    srgb_to_linear: [u16; 256],
    linear_to_srgb: [u8; 1024],
}

/// `ARGB.SRGB_TO_LINEAR` and `ARGB.LINEAR_TO_SRGB`, built as the client builds them.
fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        // Math.round(float) is floor(x + 0.5) in float arithmetic.
        let round = |x: f32| (x + 0.5).floor();
        let mut s2l = [0u16; 256];
        for (i, v) in s2l.iter_mut().enumerate() {
            let f = i as f32 / 255.0;
            let lin = if f >= 0.04045 {
                ((f as f64 + 0.055) / 1.055).powf(2.4) as f32
            } else {
                f / 12.92
            };
            *v = round(lin * 1023.0) as u16;
        }
        let mut l2s = [0u8; 1024];
        for (i, v) in l2s.iter_mut().enumerate() {
            let f = i as f32 / 1023.0;
            let s = if f >= 0.003_130_8 {
                (1.055 * (f as f64).powf(0.416_666_666_666_666_7) - 0.055) as f32
            } else {
                12.92 * f
            };
            *v = round(s * 255.0) as u8;
        }
        Tables {
            srgb_to_linear: s2l,
            linear_to_srgb: l2s,
        }
    })
}

/// `ARGB.meanLinear`: alpha averaged as stored, colour averaged in linear light.
pub fn mean_linear(c: [u32; 4]) -> u32 {
    let t = tables();
    let a = (alpha(c[0]) + alpha(c[1]) + alpha(c[2]) + alpha(c[3])) / 4;
    let mut out = [0u32; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let sum: u32 = c
            .iter()
            .map(|&p| t.srgb_to_linear[ch(p, i as u32) as usize] as u32)
            .sum();
        *o = t.linear_to_srgb[(sum / 4) as usize] as u32;
    }
    rgba(out, a)
}

/// `MipmapGenerator.darkenedAlphaBlend`: the mean of the non-transparent texels'
/// linear values (alpha included), always divided by four.
fn darkened_alpha_blend(c: [u32; 4]) -> u32 {
    let t = tables();
    let lin = |v: u32| t.srgb_to_linear[v as usize] as f32 / 1023.0;
    let to_srgb = |f: f32| t.linear_to_srgb[((f * 1023.0).floor() as usize).min(1023)] as u32;
    let mut acc = [0f32; 4];
    for &p in &c {
        if alpha(p) != 0 {
            acc[3] += lin(alpha(p));
            for (i, a) in acc.iter_mut().take(3).enumerate() {
                *a += lin(ch(p, i as u32));
            }
        }
    }
    let acc = acc.map(|v| v / 4.0);
    rgba(
        [to_srgb(acc[0]), to_srgb(acc[1]), to_srgb(acc[2])],
        to_srgb(acc[3]),
    )
}

/// `TextureUtil.solidify`: every fully transparent texel takes the colour of the
/// nearest non-transparent texel (breadth first, in the client's visiting
/// order) and stays fully transparent.
fn solidify(img: &mut MipImage) {
    let (w, h) = (img.w as i32, img.h as i32);
    let n = (w * h) as usize;
    let mut colour = vec![0u32; n];
    let mut dist = vec![i32::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for x in 0..w {
        for y in 0..h {
            let c = img.get(x as u32, y as u32);
            if alpha(c) != 0 {
                let i = (x + y * w) as usize;
                dist[i] = 0;
                colour[i] = c;
                queue.push_back(i);
            }
        }
    }
    const DIRECTIONS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i as i32 % w, i as i32 / w);
        for (dx, dy) in DIRECTIONS {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && nx < w && ny < h {
                let j = (nx + ny * w) as usize;
                if dist[j] > dist[i] + 1 {
                    dist[j] = dist[i] + 1;
                    colour[j] = colour[i];
                    queue.push_back(j);
                }
            }
        }
    }
    for x in 0..w {
        for y in 0..h {
            let c = img.get(x as u32, y as u32);
            if alpha(c) == 0 {
                img.set(
                    x as u32,
                    y as u32,
                    with_alpha(0, colour[(x + y * w) as usize]),
                );
            }
        }
    }
}

/// `TextureUtil.fillEmptyAreasWithDarkColor`: transparent texels take three
/// quarters of the darkest opaque colour, with alpha 0.
fn fill_empty_with_dark(img: &mut MipImage) {
    let mut darkest = u32::MAX;
    let mut best = u32::MAX;
    for x in 0..img.w {
        for y in 0..img.h {
            let c = img.get(x, y);
            if alpha(c) != 0 {
                let sum = ch(c, 0) + ch(c, 1) + ch(c, 2);
                if sum < best {
                    best = sum;
                    darkest = c;
                }
            }
        }
    }
    let dark = rgba(
        [
            3 * ch(darkest, 0) / 4,
            3 * ch(darkest, 1) / 4,
            3 * ch(darkest, 2) / 4,
        ],
        0,
    );
    for x in 0..img.w {
        for y in 0..img.h {
            if alpha(img.get(x, y)) == 0 {
                img.set(x, y, dark);
            }
        }
    }
}

/// `MipmapGenerator.alphaTestCoverage`: the share of the image that passes an
/// alpha test at `cutoff` after scaling alpha by `scale`, measured on a 4×4
/// grid of bilinear samples between each 2×2 block of texels. NaN for an image
/// one texel wide or high, as in the client.
fn alpha_test_coverage(img: &MipImage, cutoff: f32, scale: f32) -> f32 {
    let (w, h) = (img.w, img.h);
    let mut total = 0f32;
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            let a = |xx, yy| (alpha_f(img.get(xx, yy)) * scale).clamp(0.0, 1.0);
            let (a00, a10, a01, a11) = (a(x, y), a(x + 1, y), a(x, y + 1), a(x + 1, y + 1));
            let mut passed = 0f32;
            for j in 0..4 {
                let fy = (j as f32 + 0.5) / 4.0;
                for i in 0..4 {
                    let fx = (i as f32 + 0.5) / 4.0;
                    let v = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if v > cutoff {
                        passed += 1.0;
                    }
                }
            }
            total += passed / 16.0;
        }
    }
    total / ((w as f32 - 1.0) * (h as f32 - 1.0))
}

/// `MipmapGenerator.scaleAlphaToCoverage`: a five-step bisection for the alpha
/// scale that best restores `target` coverage, then alpha × scale + bias + 0.025.
fn scale_alpha_to_coverage(img: &mut MipImage, target: f32, cutoff: f32, bias: f32) {
    let (mut lo, mut hi) = (0f32, 4f32);
    let mut scale = 1f32;
    let mut best_scale = 1f32;
    let mut best = f32::MAX;
    for _ in 0..5 {
        let c = alpha_test_coverage(img, cutoff, scale);
        let d = (c - target).abs();
        if d < best {
            best = d;
            best_scale = scale;
        }
        if c < target {
            lo = scale;
        } else if c > target {
            hi = scale;
        } else {
            break;
        }
        scale = (lo + hi) * 0.5;
    }
    for p in img.px.iter_mut() {
        let a = (alpha_f(*p) * best_scale + bias + 0.025).clamp(0.0, 1.0);
        *p = with_alpha(as_8bit(a), *p);
    }
}

/// `MipmapGenerator.generateMipLevels` for a sprite with only its base image:
/// returns levels 0..=`levels`. Level 0 is `base` after the strategy's
/// transparent-texel fill; each later level halves the one before. `base`'s
/// sides must be divisible by `2^levels`. `item_sprite` is the client's
/// `item/` path exemption from the fill.
pub fn generate(
    mut base: MipImage,
    levels: u32,
    meta: TextureMeta,
    item_sprite: bool,
) -> Vec<MipImage> {
    let mut strategy = meta.strategy;
    if strategy == MipmapStrategy::Auto {
        strategy = if base.px.iter().any(|&c| alpha(c) == 0) {
            MipmapStrategy::Cutout
        } else {
            MipmapStrategy::Mean
        };
    }
    if !item_sprite {
        match strategy {
            MipmapStrategy::Cutout | MipmapStrategy::StrictCutout => solidify(&mut base),
            MipmapStrategy::DarkCutout => fill_empty_with_dark(&mut base),
            _ => {}
        }
    }
    let cutout = matches!(
        strategy,
        MipmapStrategy::Cutout | MipmapStrategy::StrictCutout | MipmapStrategy::DarkCutout
    );
    let cutoff = if strategy == MipmapStrategy::StrictCutout {
        0.3
    } else {
        0.5
    };
    let coverage = if cutout {
        alpha_test_coverage(&base, cutoff, 1.0)
    } else {
        0.0
    };
    let mut out = vec![base];
    for _ in 1..=levels {
        let prev = out.last().unwrap();
        let (w, h) = (prev.w >> 1, prev.h >> 1);
        let mut next = MipImage {
            w,
            h,
            px: vec![0; (w * h) as usize],
        };
        for x in 0..w {
            for y in 0..h {
                let q = [
                    prev.get(2 * x, 2 * y),
                    prev.get(2 * x + 1, 2 * y),
                    prev.get(2 * x, 2 * y + 1),
                    prev.get(2 * x + 1, 2 * y + 1),
                ];
                let c = if strategy == MipmapStrategy::DarkCutout {
                    darkened_alpha_blend(q)
                } else {
                    mean_linear(q)
                };
                next.set(x, y, c);
            }
        }
        if cutout {
            scale_alpha_to_coverage(&mut next, coverage, cutoff, meta.alpha_cutoff_bias);
        }
        out.push(next);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> u32) -> MipImage {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.push(f(x, y));
            }
        }
        MipImage { w, h, px }
    }

    #[test]
    fn tables_match_the_client_endpoints() {
        let t = tables();
        assert_eq!(t.srgb_to_linear[0], 0);
        assert_eq!(t.srgb_to_linear[255], 1023);
        assert_eq!(t.linear_to_srgb[0], 0);
        assert_eq!(t.linear_to_srgb[1023], 255);
        // sRGB 128 is 21.6% linear: 0.2158605 * 1023 = 220.8 -> 221.
        assert_eq!(t.srgb_to_linear[128], 221);
    }

    #[test]
    fn mean_is_taken_in_linear_light() {
        // Black and white average to sRGB ~188, not 128.
        let c = mean_linear([0xFF00_0000, 0xFFFF_FFFF, 0xFF00_0000, 0xFFFF_FFFF]);
        assert_eq!(c >> 24, 255);
        let r = c & 0xFF;
        assert!((186..=190).contains(&r), "{r}");
    }

    #[test]
    fn opaque_chain_halves_to_one_texel() {
        let base = img(16, 16, |x, y| {
            if (x + y) % 2 == 0 {
                0xFF20_4060
            } else {
                0xFF60_4020
            }
        });
        let chain = generate(base, 4, TextureMeta::default(), false);
        assert_eq!(chain.len(), 5);
        assert_eq!((chain[4].w, chain[4].h), (1, 1));
        assert!(chain[4].px[0] >> 24 == 255);
    }

    #[test]
    fn cutout_keeps_alpha_test_coverage() {
        // A thin cross: a quarter of the texels opaque. A plain mean would fade it
        // below the 0.5 alpha test by level 2; coverage scaling keeps it.
        let base = img(16, 16, |x, y| {
            if x % 4 == 0 || y % 4 == 0 {
                0xFF30_A030
            } else {
                0
            }
        });
        let chain = generate(base.clone(), 4, TextureMeta::default(), false);
        let passing = |m: &MipImage| {
            m.px.iter().filter(|&&c| c >> 24 >= 128).count() as f32 / m.px.len() as f32
        };
        let l0 = passing(&chain[0]);
        for (i, m) in chain.iter().enumerate().skip(1).take(2) {
            let p = passing(m);
            assert!(
                p > 0.25,
                "level {i}: {p:.2} of texels pass (level 0: {l0:.2})"
            );
        }
        // Solidify recoloured the transparent texels from their neighbours.
        assert_eq!(chain[0].px[(16 + 1) as usize] & 0x00FF_FFFF, 0x30_A030);
        assert_eq!(chain[0].px[(16 + 1) as usize] >> 24, 0);
    }

    #[test]
    fn mcmeta_texture_section() {
        let m = TextureMeta::parse(
            br#"{"animation":{"frametime":2},"texture":{"alpha_cutoff_bias":0.1}}"#,
        );
        assert_eq!(m.strategy, MipmapStrategy::Auto);
        assert!((m.alpha_cutoff_bias - 0.1).abs() < 1e-6);
        let m = TextureMeta::parse(br#"{"texture":{"mipmap_strategy":"dark_cutout"}}"#);
        assert_eq!(m.strategy, MipmapStrategy::DarkCutout);
    }
}
