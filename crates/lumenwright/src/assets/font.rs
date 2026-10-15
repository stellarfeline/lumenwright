//! The vanilla bitmap font (`assets/<ns>/font/*.json`): glyph sprites and advances,
//! measured the way the client's `BitmapProvider` measures them.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::atlas::load_texture;
use super::source::{ResourceStore, norm_loc, split_loc};

/// One glyph: a cell of a bitmap font texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    /// Texture location (`ns:font/ascii`).
    pub texture: String,
    /// Texture size in pixels.
    pub texture_size: [f32; 2],
    /// The cell's top-left corner and size in texture pixels.
    pub cell: [f32; 4],
    /// Cell pixels to font pixels (`height / cell height`).
    pub scale: f32,
    /// Font pixels from the top of the line to the baseline.
    pub ascent: f32,
    /// Horizontal advance in font pixels (glyph width + 1).
    pub advance: f32,
}

/// Decoded font textures by location (`None` when missing).
type TextureCache = BTreeMap<String, Option<(u32, u32, Vec<u32>)>>;

#[derive(Clone, Debug, Default)]
pub struct Font {
    glyphs: HashMap<char, Glyph>,
    /// Advance-only glyphs (`space` providers).
    spaces: HashMap<char, f32>,
}

#[derive(Deserialize)]
struct FontFile {
    #[serde(default)]
    providers: Vec<serde_json::Value>,
}

impl Font {
    /// Loads `minecraft:default`, following `reference` providers. Earlier providers
    /// win for a character, as in vanilla's `FontSet`.
    pub fn load(store: &ResourceStore) -> Font {
        Font::load_id(store, "minecraft:default")
    }

    pub fn load_id(store: &ResourceStore, id: &str) -> Font {
        let mut font = Font::default();
        let mut textures = TextureCache::new();
        font.add_file(store, &norm_loc(id), &mut textures, 0);
        font
    }

    fn add_file(
        &mut self,
        store: &ResourceStore,
        id: &str,
        textures: &mut TextureCache,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        let (ns, path) = split_loc(id);
        let Some(bytes) = store.get(&format!("assets/{ns}/font/{path}.json")) else {
            log::debug!("font {id} not found");
            return;
        };
        let file: FontFile = match serde_json::from_slice(bytes) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("font {id}: {e}");
                return;
            }
        };
        for p in &file.providers {
            match p.get("type").and_then(|t| t.as_str()) {
                Some("reference") => {
                    // `filter: {uniform: false}` keeps the bitmap glyphs (the default
                    // look); a filter that asks for the uniform font drops the provider.
                    let uniform_only = p
                        .get("filter")
                        .and_then(|f| f.get("uniform"))
                        .and_then(|u| u.as_bool())
                        == Some(true);
                    if let Some(rid) = p.get("id").and_then(|i| i.as_str())
                        && !uniform_only
                    {
                        self.add_file(store, &norm_loc(rid), textures, depth + 1);
                    }
                }
                Some("space") => {
                    if let Some(adv) = p.get("advances").and_then(|a| a.as_object()) {
                        for (k, v) in adv {
                            if let (Some(c), Some(a)) = (k.chars().next(), v.as_f64()) {
                                self.spaces.entry(c).or_insert(a as f32);
                            }
                        }
                    }
                }
                Some("bitmap") => self.add_bitmap(store, p, textures),
                _ => {}
            }
        }
    }

    fn add_bitmap(
        &mut self,
        store: &ResourceStore,
        p: &serde_json::Value,
        textures: &mut TextureCache,
    ) {
        let Some(file) = p.get("file").and_then(|f| f.as_str()) else {
            return;
        };
        // `file` names a texture with its extension, relative to `textures/`.
        let loc = norm_loc(file.strip_suffix(".png").unwrap_or(file));
        let height = p.get("height").and_then(|h| h.as_f64()).unwrap_or(8.0) as f32;
        let ascent = p.get("ascent").and_then(|a| a.as_f64()).unwrap_or(7.0) as f32;
        let rows: Vec<Vec<char>> = p
            .get("chars")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str())
                    .map(|r| r.chars().collect())
                    .collect()
            })
            .unwrap_or_default();
        let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if rows.is_empty() || cols == 0 {
            return;
        }
        let tex = textures
            .entry(loc.clone())
            .or_insert_with(|| load_texture(store, &loc));
        let Some((tw, th, px)) = tex.as_ref() else {
            log::debug!("font texture {loc} not found");
            return;
        };
        let (cw, ch) = (*tw as usize / cols, *th as usize / rows.len());
        if cw == 0 || ch == 0 {
            return;
        }
        let scale = height / ch as f32;
        for (ry, row) in rows.iter().enumerate() {
            for (rx, &c) in row.iter().enumerate() {
                if c == '\0' || c == ' ' || self.glyphs.contains_key(&c) {
                    continue;
                }
                // BitmapProvider.getActualGlyphWidth: the rightmost column with any
                // non-transparent texel, plus one.
                let (x0, y0) = (rx * cw, ry * ch);
                let mut width = 0;
                for x in (0..cw).rev() {
                    let lit = (0..ch).any(|y| px[(y0 + y) * *tw as usize + x0 + x] >> 24 != 0);
                    if lit {
                        width = x + 1;
                        break;
                    }
                }
                let advance = (0.5 + width as f32 * scale).floor() + 1.0;
                self.glyphs.insert(
                    c,
                    Glyph {
                        texture: loc.clone(),
                        texture_size: [*tw as f32, *th as f32],
                        cell: [x0 as f32, y0 as f32, cw as f32, ch as f32],
                        scale,
                        ascent,
                        advance,
                    },
                );
            }
        }
    }

    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs.get(&c)
    }

    /// Advance of `c` in font pixels; unknown characters advance like vanilla's
    /// missing glyph (5 + 1).
    pub fn advance(&self, c: char, bold: bool) -> f32 {
        if let Some(a) = self.spaces.get(&c) {
            return *a;
        }
        let base = self.glyphs.get(&c).map(|g| g.advance).unwrap_or(6.0);
        if bold { base + 1.0 } else { base }
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, lit: &[(u32, u32)]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for &(x, y) in lit {
            img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn measures_glyphs_like_bitmap_provider() {
        let mut store = ResourceStore::new();
        // A 2x1 grid of 8x8 cells: 'A' lights column 4, 'B' lights nothing.
        store.insert(
            "assets/minecraft/textures/font/test.png",
            png(16, 8, &[(4, 2)]),
        );
        store.insert(
            "assets/minecraft/font/default.json",
            br#"{"providers":[{"type":"reference","id":"minecraft:include/space"},
                {"type":"bitmap","file":"minecraft:font/test.png","ascent":7,"chars":["AB"]}]}"#
                .to_vec(),
        );
        store.insert(
            "assets/minecraft/font/include/space.json",
            br#"{"providers":[{"type":"space","advances":{" ":4}}]}"#.to_vec(),
        );
        let f = Font::load(&store);
        let a = f.glyph('A').unwrap();
        assert_eq!(a.advance, 6.0);
        assert_eq!(a.cell, [0.0, 0.0, 8.0, 8.0]);
        assert_eq!(f.glyph('B').unwrap().advance, 1.0);
        assert_eq!(f.advance(' ', false), 4.0);
        assert_eq!(f.advance('A', true), 7.0);
        assert_eq!(f.advance('?', false), 6.0);
    }
}
