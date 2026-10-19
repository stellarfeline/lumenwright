//! Texture atlas: every sprite used by the scene packed into one RGBA8 image.
//!
//! Sprites the vanilla client stitches into its mipmapped block atlas (the
//! sources in `atlases/blocks.json`) are packed first, the way
//! `net.minecraft.client.renderer.texture.Stitcher` packs them: each sprite gets
//! its own mip chain ([`super::mipmap`]), a padding of `2^levels << 1` texels on
//! every side filled by clamping each level to its edge (the `animate_sprite`
//! blit with clamp-to-edge sampling), and a slot rounded up to a multiple of
//! `2^levels`, so a sprite's level-`l` texels sit at its level-0 rectangle
//! shifted right by `l` and no filter tap at any level reaches a neighbour.
//! Those sprites fill the rows above [`Atlas::mip_rows`]. Every other sprite
//! (entities, items, text, the sky) is sampled without mipmaps, as vanilla
//! samples its other atlases, and is packed below with a 1-texel border.

use std::collections::HashMap;

use super::mipmap::{self, MipImage, TextureMeta};
use super::source::{ResourceStore, split_loc};
use crate::assets::model::MISSING_TEXTURE;

/// Mip levels the client builds by default (`options.mipmapLevels`, 0..4, default 4).
pub const MIPMAP_LEVELS: u32 = 4;
/// `options.maxAnisotropyBit` default (2, i.e. 4×), which sets the stitcher's
/// padding: `1 << levels << clamp(bit - 1, 0, 4)`.
pub const MAX_ANISOTROPY_BIT: u32 = 2;
/// The quick shader's anisotropic tap limit: `1 << MAX_ANISOTROPY_BIT`.
pub const MAX_ANISOTROPY: u32 = 1 << MAX_ANISOTROPY_BIT;

/// A plain white sprite, tinted per quad (entity fallback boxes).
pub const WHITE_TEXTURE: &str = "lumenwright:white";

/// One texture's rectangle in the [`Atlas`], in atlas pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sprite {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
    /// Some texel has alpha < 255.
    pub transparent: bool,
    /// Some texel has 0 < alpha < 255 (needs blending, not just cutout).
    pub translucent: bool,
}

impl Sprite {
    /// Converts sprite-space UV (0..16 for block textures, or pixel UV for
    /// entity textures via `scale`) to atlas pixels.
    pub fn to_atlas(&self, u: f32, v: f32, uv_size: [f32; 2]) -> [f32; 2] {
        [
            self.x as f32 + u / uv_size[0] * self.w as f32,
            self.y as f32 + v / uv_size[1] * self.h as f32,
        ]
    }
}

struct Image {
    w: u32,
    h: u32,
    px: Vec<u32>,
    /// Rows of the whole image `px` holds; above `h` for an animation strip, whose
    /// frame 0 is the top `h` rows. The client builds mip levels over the strip.
    strip_h: u32,
    /// The `.mcmeta` `texture` section.
    meta: TextureMeta,
}

/// Collects textures by location; `build` packs them.
#[derive(Default)]
pub struct AtlasBuilder {
    names: Vec<String>,
    ids: HashMap<String, u32>,
    /// Images made in code (composited banners), by location.
    generated: HashMap<String, Image>,
}

/// Every sprite the scene uses, packed into one image.
pub struct Atlas {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// RGBA8, red in the low byte, straight (non-premultiplied) alpha, sRGB.
    pub pixels: Vec<u32>,
    /// Sprite rectangles, indexed by the id [`AtlasBuilder::add`] returned.
    pub sprites: Vec<Sprite>,
    /// Mip levels above level 0 (0 when no sprite is mipmapped).
    pub mip_levels: u32,
    /// Rows `0..mip_rows` hold the mipmapped sprites; a multiple of `2^mip_levels`.
    pub mip_rows: u32,
    /// Levels `1..=mip_levels`, each `(width >> l) × (height >> l)`, concatenated
    /// (see [`Atlas::level_offset`]). Only the rows above `mip_rows >> l` are filled.
    pub mips: Vec<u32>,
    ids: HashMap<String, u32>,
}

impl Atlas {
    /// Offset of level `l` (1-based) in [`Atlas::mips`].
    pub fn level_offset(&self, l: u32) -> usize {
        level_offset(self.width, self.height, l)
    }

    /// Level `l` of the atlas, row major (`l = 0` is [`Atlas::pixels`]).
    pub fn level(&self, l: u32) -> &[u32] {
        if l == 0 {
            return &self.pixels;
        }
        let o = self.level_offset(l);
        &self.mips[o..o + ((self.width >> l) * (self.height >> l)) as usize]
    }

    /// The sprite for a texture location, or the missing texture's sprite when
    /// `loc` was never added.
    pub fn sprite(&self, loc: &str) -> Sprite {
        let id = self
            .ids
            .get(loc)
            .or_else(|| self.ids.get(MISSING_TEXTURE))
            .copied()
            .unwrap_or(0);
        self.sprites[id as usize]
    }

    /// The sprite id of a texture location, if it was added.
    pub fn id(&self, loc: &str) -> Option<u32> {
        self.ids.get(loc).copied()
    }

    /// Average sRGB colour of a sprite (opaque texels), handy for previews and fallbacks.
    pub fn average(&self, loc: &str) -> [u8; 4] {
        let s = self.sprite(loc);
        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
        for y in s.y..s.y + s.h {
            for x in s.x..s.x + s.w {
                let c = self.pixels[(y * self.width + x) as usize];
                if c >> 24 > 0 {
                    r += (c & 0xFF) as u64;
                    g += ((c >> 8) & 0xFF) as u64;
                    b += ((c >> 16) & 0xFF) as u64;
                    n += 1;
                }
            }
        }
        let n = n.max(1);
        [(r / n) as u8, (g / n) as u8, (b / n) as u8, 255]
    }
}

/// Offset of level `l` (1-based) in the concatenated levels of a `w × h` atlas.
pub fn level_offset(w: u32, h: u32, l: u32) -> usize {
    (1..l).map(|k| ((w >> k) * (h >> k)) as usize).sum()
}

fn plain(w: u32, h: u32, px: Vec<u32>) -> Image {
    Image {
        w,
        h,
        px,
        strip_h: h,
        meta: TextureMeta::default(),
    }
}

fn missing_image() -> Image {
    let mut px = vec![0u32; 256];
    for y in 0..16 {
        for x in 0..16 {
            let magenta = ((x / 8) + (y / 8)) % 2 == 0;
            px[y * 16 + x] = if magenta { 0xFFF8_00F8 } else { 0xFF00_0000 };
        }
    }
    plain(16, 16, px)
}

/// The sprites the client's block atlas holds: `atlases/blocks.json`'s
/// `directory` and `single` sources (the only kinds it uses).
struct MippedSources {
    directories: Vec<String>,
    singles: Vec<String>,
}

impl MippedSources {
    fn load(store: &ResourceStore) -> MippedSources {
        let mut m = MippedSources {
            directories: Vec::new(),
            singles: Vec::new(),
        };
        let Some(bytes) = store.get("assets/minecraft/atlases/blocks.json") else {
            log::warn!("no atlases/blocks.json in the client jar: no texture is mipmapped");
            return m;
        };
        let v: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
        for src in v["sources"].as_array().into_iter().flatten() {
            let kind = src["type"].as_str().unwrap_or("");
            match kind.strip_prefix("minecraft:").unwrap_or(kind) {
                "directory" => {
                    if let Some(d) = src["source"].as_str() {
                        m.directories.push(format!("{d}/"));
                    }
                }
                "single" => {
                    if let Some(r) = src["resource"].as_str() {
                        m.singles.push(super::source::norm_loc(r));
                    }
                }
                other => log::warn!("atlases/blocks.json: source type {other:?} is not read"),
            }
        }
        m
    }

    fn contains(&self, loc: &str) -> bool {
        if loc == MISSING_TEXTURE {
            return true;
        }
        let (_, path) = split_loc(loc);
        self.directories
            .iter()
            .any(|d| path.starts_with(d.as_str()))
            || self.singles.iter().any(|s| s == loc)
    }
}

/// Decodes a texture as RGBA8 texels (red in the low byte), full height.
pub fn load_texture(store: &ResourceStore, loc: &str) -> Option<(u32, u32, Vec<u32>)> {
    let (ns, path) = split_loc(loc);
    let bytes = store.get(&format!("assets/{ns}/textures/{path}.png"))?;
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|e| log::warn!("texture {loc}: {e}"))
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let px = img.pixels().map(|p| u32::from_le_bytes(p.0)).collect();
    Some((w, h, px))
}

fn load_png(store: &ResourceStore, loc: &str) -> Option<Image> {
    let (ns, path) = split_loc(loc);
    let bytes = store.get(&format!("assets/{ns}/textures/{path}.png"))?;
    let img = match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            log::warn!("texture {loc}: {e}");
            return None;
        }
    };
    let (w, strip_h) = img.dimensions();
    // Animated block textures are vertical strips of square frames: draw frame 0.
    let mcmeta = store.get(&format!("assets/{ns}/textures/{path}.png.mcmeta"));
    let mut h = strip_h;
    if (mcmeta.is_some() || path.starts_with("block/")) && h > w && h % w == 0 {
        h = w;
    }
    let px = img.pixels().map(|p| u32::from_le_bytes(p.0)).collect();
    Some(Image {
        w,
        h,
        px,
        strip_h,
        meta: mcmeta.map(TextureMeta::parse).unwrap_or_default(),
    })
}

impl AtlasBuilder {
    /// An empty builder holding only the magenta and black missing texture.
    pub fn new() -> AtlasBuilder {
        let mut b = AtlasBuilder::default();
        b.add(MISSING_TEXTURE);
        b
    }

    /// Registers an image made in code under `loc` (RGBA8, red in the low byte).
    pub fn add_generated(&mut self, loc: &str, w: u32, h: u32, px: Vec<u32>) -> u32 {
        assert_eq!(px.len(), (w * h) as usize, "generated texture {loc}");
        self.generated.insert(loc.to_string(), plain(w, h, px));
        self.add(loc)
    }

    /// Registers a texture location (`namespace:path`, read from
    /// `assets/<ns>/textures/<path>.png`) and returns its sprite id. Adding a
    /// location twice returns the same id.
    pub fn add(&mut self, loc: &str) -> u32 {
        if let Some(&id) = self.ids.get(loc) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(loc.to_string());
        self.ids.insert(loc.to_string(), id);
        id
    }

    /// Loads every registered texture from `store` and packs them. A texture that
    /// cannot be read is drawn as the missing texture.
    pub fn build(mut self, store: &ResourceStore) -> Atlas {
        // Images made in code are never in the client's block atlas.
        let generated: Vec<bool> = self
            .names
            .iter()
            .map(|n| self.generated.contains_key(n))
            .collect();
        let images: Vec<Image> = self
            .names
            .iter()
            .map(|n| {
                if n == MISSING_TEXTURE {
                    return missing_image();
                }
                if let Some(img) = self.generated.remove(n) {
                    return img;
                }
                if n == WHITE_TEXTURE {
                    return plain(16, 16, vec![0xFFFF_FFFF; 256]);
                }
                load_png(store, n).unwrap_or_else(|| {
                    log::debug!("missing texture {n}");
                    missing_image()
                })
            })
            .collect();
        let sources = MippedSources::load(store);
        let mipped: Vec<bool> = self
            .names
            .iter()
            .zip(&generated)
            .map(|(n, &g)| !g && sources.contains(n))
            .collect();
        // SpriteLoader: the smallest power of two dividing every mipmapped sprite's
        // sides caps the level count.
        let levels = images
            .iter()
            .zip(&mipped)
            .filter(|(_, m)| **m)
            .map(|(i, _)| (i.w | i.h).trailing_zeros())
            .min()
            .map_or(0, |l| l.min(MIPMAP_LEVELS));
        let align = 1u32 << levels;
        let pad = if levels > 0 {
            align << MAX_ANISOTROPY_BIT.saturating_sub(1).min(4)
        } else {
            1
        };
        let round = |v: u32| v.div_ceil(align) * align;
        // Slot sizes: mipmapped sprites padded and rounded as the stitcher does,
        // the rest with a 1px extruded border.
        let slot = |i: usize| {
            let img = &images[i];
            if mipped[i] && levels > 0 {
                (round(img.w + 2 * pad), round(img.h + 2 * pad))
            } else {
                (img.w + 2, img.h + 2)
            }
        };
        let total: u64 = (0..images.len())
            .map(|i| {
                let (w, h) = slot(i);
                w as u64 * h as u64
            })
            .sum();
        let widest = (0..images.len()).map(|i| slot(i).0).max().unwrap_or(0);
        let mut width = 256u32;
        while ((width as u64 * width as u64) < total * 2 || width < widest) && width < 16384 {
            width *= 2;
        }
        let mut order: Vec<usize> = (0..images.len()).collect();
        // Mipmapped first; name as the final key keeps the layout independent of
        // registration order.
        order.sort_by(|&a, &b| {
            (mipped[b] && levels > 0, images[b].h, images[b].w)
                .cmp(&(mipped[a] && levels > 0, images[a].h, images[a].w))
                .then_with(|| self.names[a].cmp(&self.names[b]))
        });
        // Slot origins.
        let mut slots = vec![(0u32, 0u32); images.len()];
        let (mut x, mut y, mut shelf_h) = (0u32, 0u32, 0u32);
        let mut mip_rows = 0;
        let mut in_mip_group = true;
        for &i in &order {
            let is_mip = mipped[i] && levels > 0;
            if in_mip_group && !is_mip {
                in_mip_group = false;
                y += shelf_h;
                mip_rows = round(y);
                y = mip_rows;
                (x, shelf_h) = (0, 0);
            }
            let (w, h) = slot(i);
            if x + w > width {
                x = 0;
                y += shelf_h;
                shelf_h = 0;
            }
            slots[i] = (x, y);
            x += w;
            shelf_h = shelf_h.max(h);
        }
        if in_mip_group {
            mip_rows = round(y + shelf_h);
        }
        if levels == 0 {
            mip_rows = 0;
        }
        let height = (y + shelf_h)
            .max(mip_rows)
            .next_power_of_two()
            .max(16)
            .max(align);
        let mut pixels = vec![0u32; (width * height) as usize];
        let mut mips = vec![0u32; level_offset(width, height, levels + 1)];
        let mut sprites = Vec::with_capacity(images.len());
        for (i, img) in images.into_iter().enumerate() {
            let (sx, sy) = slots[i];
            let (mut transparent, mut translucent) = (false, false);
            for y in 0..img.h {
                for x in 0..img.w {
                    let a = img.px[(y * img.w + x) as usize] >> 24;
                    transparent |= a < 255;
                    translucent |= a > 0 && a < 255;
                }
            }
            let is_mip = mipped[i] && levels > 0;
            let (p, (w, h)) = (if is_mip { pad } else { 1 }, (img.w, img.h));
            sprites.push(Sprite {
                x: sx + p,
                y: sy + p,
                w,
                h,
                transparent,
                translucent,
            });
            let chain = if is_mip {
                let (_, path) = split_loc(&self.names[i]);
                let strip = MipImage {
                    w: img.w,
                    h: img.strip_h,
                    px: img.px,
                };
                mipmap::generate(strip, levels, img.meta, path.starts_with("item/"))
            } else {
                vec![MipImage {
                    w: img.w,
                    h: img.strip_h,
                    px: img.px,
                }]
            };
            for (l, level) in chain.iter().enumerate() {
                let l = l as u32;
                let (lw, lh, lp) = (w >> l, h >> l, p >> l);
                let aw = width >> l;
                let dst: &mut [u32] = if l == 0 {
                    &mut pixels
                } else {
                    let o = level_offset(width, height, l);
                    &mut mips[o..o + (aw * (height >> l)) as usize]
                };
                let (ox, oy) = (sx >> l, sy >> l);
                for yy in 0..lh + 2 * lp {
                    for xx in 0..lw + 2 * lp {
                        let srcx = (xx as i32 - lp as i32).clamp(0, lw as i32 - 1) as u32;
                        let srcy = (yy as i32 - lp as i32).clamp(0, lh as i32 - 1) as u32;
                        dst[((oy + yy) * aw + ox + xx) as usize] =
                            level.px[(srcy * level.w + srcx) as usize];
                    }
                }
            }
        }
        log::info!(
            "atlas: {} sprites ({} mipmapped, {levels} levels), {width}x{height}",
            sprites.len(),
            mipped.iter().filter(|m| **m).count()
        );
        Atlas {
            width,
            height,
            pixels,
            sprites,
            mip_levels: levels,
            mip_rows,
            mips,
            ids: self.ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_missing_and_unknown() {
        let store = ResourceStore::new();
        let mut b = AtlasBuilder::new();
        b.add("minecraft:block/does_not_exist");
        let a = b.build(&store);
        assert_eq!(a.sprites.len(), 2);
        let s = a.sprite("minecraft:block/does_not_exist");
        assert_eq!((s.w, s.h), (16, 16));
        assert!(!s.transparent);
        // Sprites don't overlap.
        let s0 = a.sprites[0];
        assert!(s0.x + s0.w < s.x || s.x + s.w < s0.x || s0.y + s0.h < s.y || s.y + s.h < s0.y);
    }
}
