//! Texture atlas: every sprite used by the scene packed into one RGBA8 image.

use std::collections::HashMap;

use super::source::{ResourceStore, split_loc};
use crate::assets::model::MISSING_TEXTURE;

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
    ids: HashMap<String, u32>,
}

impl Atlas {
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

fn missing_image() -> Image {
    let mut px = vec![0u32; 256];
    for y in 0..16 {
        for x in 0..16 {
            let magenta = ((x / 8) + (y / 8)) % 2 == 0;
            px[y * 16 + x] = if magenta { 0xFFF8_00F8 } else { 0xFF00_0000 };
        }
    }
    Image { w: 16, h: 16, px }
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
    let (w, mut h) = img.dimensions();
    // Animated block textures are vertical strips of square frames: keep frame 0.
    let animated = store
        .get(&format!("assets/{ns}/textures/{path}.png.mcmeta"))
        .is_some();
    if (animated || path.starts_with("block/")) && h > w && h % w == 0 {
        h = w;
    }
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            px.push(u32::from_le_bytes(p));
        }
    }
    Some(Image { w, h, px })
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
        self.generated.insert(loc.to_string(), Image { w, h, px });
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
        let mut images: Vec<Image> = self
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
                    return Image {
                        w: 16,
                        h: 16,
                        px: vec![0xFFFF_FFFF; 256],
                    };
                }
                load_png(store, n).unwrap_or_else(|| {
                    log::debug!("missing texture {n}");
                    missing_image()
                })
            })
            .collect();
        // Shelf packing with a 1px extruded border around every sprite.
        let total: u64 = images.iter().map(|i| ((i.w + 2) * (i.h + 2)) as u64).sum();
        let mut width = 256u32;
        while (width as u64 * width as u64) < total * 2 && width < 16384 {
            width *= 2;
        }
        let mut order: Vec<usize> = (0..images.len()).collect();
        // Name as the final key keeps the layout independent of registration order.
        order.sort_by(|&a, &b| {
            (images[b].h, images[b].w)
                .cmp(&(images[a].h, images[a].w))
                .then_with(|| self.names[a].cmp(&self.names[b]))
        });
        let mut rects = vec![(0u32, 0u32); images.len()];
        let (mut x, mut y, mut shelf_h) = (0u32, 0u32, 0u32);
        for &i in &order {
            let (w, h) = (images[i].w + 2, images[i].h + 2);
            if x + w > width {
                x = 0;
                y += shelf_h;
                shelf_h = 0;
            }
            rects[i] = (x + 1, y + 1);
            x += w;
            shelf_h = shelf_h.max(h);
        }
        let height = (y + shelf_h).next_power_of_two().max(16);
        let mut pixels = vec![0u32; (width * height) as usize];
        let mut sprites = Vec::with_capacity(images.len());
        for (i, img) in images.iter_mut().enumerate() {
            let (ox, oy) = rects[i];
            let mut transparent = false;
            let mut translucent = false;
            for yy in -1..=img.h as i32 {
                for xx in -1..=img.w as i32 {
                    let sx = xx.clamp(0, img.w as i32 - 1) as u32;
                    let sy = yy.clamp(0, img.h as i32 - 1) as u32;
                    let c = img.px[(sy * img.w + sx) as usize];
                    let px = (ox as i32 + xx) as u32;
                    let py = (oy as i32 + yy) as u32;
                    pixels[(py * width + px) as usize] = c;
                    if xx >= 0 && yy >= 0 && xx < img.w as i32 && yy < img.h as i32 {
                        let a = c >> 24;
                        transparent |= a < 255;
                        translucent |= a > 0 && a < 255;
                    }
                }
            }
            sprites.push(Sprite {
                x: ox,
                y: oy,
                w: img.w,
                h: img.h,
                transparent,
                translucent,
            });
        }
        log::info!("atlas: {} sprites, {width}x{height}", sprites.len());
        Atlas {
            width,
            height,
            pixels,
            sprites,
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
