//! The vanilla font (`assets/<ns>/font/*.json`): glyph sprites and advances. Bitmap
//! providers are measured the way the client's `BitmapProvider` measures them; unihex
//! providers (the Unifont that draws CJK and every other character no bitmap covers)
//! the way its `UnihexProvider` does (see [`super::unifont`]).

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::atlas::load_texture;
use super::source::{ResourceStore, norm_loc, split_loc};
use super::unifont::{self, HexGlyph};

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
    /// How far bold draws the glyph a second time to the right, and how far its shadow
    /// is moved right and down, in font pixels (`GlyphInfo.getBoldOffset` and
    /// `getShadowOffset`: 1 for a bitmap glyph, 0.5 for a unihex one).
    pub offset: f32,
}

/// Decoded font textures by location (`None` when missing).
type TextureCache = BTreeMap<String, Option<(u32, u32, Vec<u32>)>>;

/// A bitmap font from the jar and resource packs (`assets/<ns>/font/*.json`),
/// as used for sign text.
#[derive(Clone, Debug, Default)]
pub struct Font {
    glyphs: HashMap<char, Glyph>,
    /// Advance-only glyphs (`space` providers).
    spaces: HashMap<char, f32>,
    /// Glyphs of the unihex providers, drawn from generated textures.
    hex: HashMap<char, HexGlyph>,
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

    /// Loads the font with id `id` (such as `minecraft:default`), following
    /// `reference` providers.
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
            if !unifont::provider_enabled(p) {
                continue;
            }
            match p.get("type").and_then(|t| t.as_str()) {
                Some("reference") => {
                    if let Some(rid) = p.get("id").and_then(|i| i.as_str()) {
                        self.add_file(store, &norm_loc(rid), textures, depth + 1);
                    }
                }
                Some("unihex") => self.add_unihex(store, p),
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
                if c == '\0' || c == ' ' || self.has(c) {
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
                        offset: 1.0,
                    },
                );
            }
        }
    }

    /// A `unihex` provider: the glyphs of its hex files, each cut to its size
    /// override or to the pixels it lights. An earlier provider keeps a character.
    fn add_unihex(&mut self, store: &ResourceStore, p: &serde_json::Value) {
        let Some(file) = p.get("hex_file").and_then(|f| f.as_str()) else {
            return;
        };
        let (ns, path) = split_loc(file);
        let Some(bytes) = store.get(&format!("assets/{ns}/{path}")) else {
            log::warn!("unihex font file {file} not found");
            return;
        };
        let glyphs = match unifont::read_hex_zip(bytes, p) {
            Ok(g) => g,
            Err(e) => {
                log::warn!("unihex font file {file}: {e}");
                return;
            }
        };
        for (cp, g) in glyphs {
            if let Some(c) = char::from_u32(cp)
                && !self.has(c)
            {
                self.hex.insert(c, g);
            }
        }
    }

    /// Some provider already gave `c` a glyph or an advance.
    fn has(&self, c: char) -> bool {
        self.glyphs.contains_key(&c) || self.spaces.contains_key(&c) || self.hex.contains_key(&c)
    }

    /// The glyph for `c`, if a bitmap or unihex provider has one.
    pub fn glyph(&self, c: char) -> Option<Glyph> {
        if let Some(g) = self.glyphs.get(&c) {
            return Some(g.clone());
        }
        self.hex.get(&c).map(|h| h.glyph(c))
    }

    /// The image of a unihex glyph's texture (`lumenwright:font/unihex/...`, as
    /// [`Glyph::texture`] names it): width, height and RGBA8 texels with red in the
    /// low byte.
    pub fn generated_texture(&self, loc: &str) -> Option<(u32, u32, Vec<u32>)> {
        let c = unifont::texture_char(loc)?;
        Some(self.hex.get(&c)?.image())
    }

    /// Advance of `c` in font pixels; unknown characters advance like vanilla's
    /// missing glyph (5 + 1).
    pub fn advance(&self, c: char, bold: bool) -> f32 {
        if let Some(a) = self.spaces.get(&c) {
            return *a;
        }
        let (base, offset) = match (self.glyphs.get(&c), self.hex.get(&c)) {
            (Some(g), _) => (g.advance, g.offset),
            (None, Some(h)) => (h.advance(), unifont::OFFSET),
            (None, None) => (6.0, 1.0),
        };
        if bold { base + offset } else { base }
    }

    /// The font has no glyphs (its files were not found).
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty() && self.hex.is_empty()
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
        assert_eq!(a.offset, 1.0);
        assert_eq!(a.advance, 6.0);
        assert_eq!(a.cell, [0.0, 0.0, 8.0, 8.0]);
        assert_eq!(f.glyph('B').unwrap().advance, 1.0);
        assert_eq!(f.advance(' ', false), 4.0);
        assert_eq!(f.advance('A', true), 7.0);
        assert_eq!(f.advance('?', false), 6.0);
    }

    #[test]
    fn unihex_glyphs_follow_the_client_and_bitmaps_come_first() {
        use std::io::Write;
        let mut out = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
            z.start_file("a.hex", zip::write::SimpleFileOptions::default())
                .unwrap();
            // U+4E00 lights column 0 (cut to one pixel unless overridden), U+0041
            // ('A') is also in the bitmap font and must not be taken from here.
            writeln!(z, "4E00:{}", "8000".repeat(16)).unwrap();
            writeln!(z, "0041:{}", "FFFF".repeat(16)).unwrap();
            z.finish().unwrap();
        }
        let mut store = ResourceStore::new();
        store.insert("assets/minecraft/font/unifont.zip", out);
        store.insert(
            "assets/minecraft/textures/font/test.png",
            png(8, 8, &[(4, 2)]),
        );
        store.insert(
            "assets/minecraft/font/default.json",
            br#"{"providers":[
                {"type":"bitmap","file":"minecraft:font/test.png","ascent":7,"chars":["A"]},
                {"type":"reference","id":"minecraft:include/unifont"}]}"#
                .to_vec(),
        );
        store.insert(
            "assets/minecraft/font/include/unifont.json",
            br#"{"providers":[
                {"type":"unihex","hex_file":"minecraft:font/unifont.zip","filter":{"jp":true}},
                {"type":"unihex","hex_file":"minecraft:font/unifont.zip",
                 "size_overrides":[{"from":"\u4E00","to":"\u4E00","left":0,"right":15}]}]}"#
                .to_vec(),
        );
        let f = Font::load(&store);
        // The bitmap provider is first: 'A' keeps its 5 + 1 advance.
        assert_eq!(f.glyph('A').unwrap().texture, "minecraft:font/test");
        assert_eq!(f.advance('A', false), 6.0);
        // The hex glyph is 16 wide through its override: advance 16 / 2 + 1, half a
        // font pixel per texel, top of the line, bold and shadow offsets of 0.5.
        let g = f.glyph('\u{4E00}').unwrap();
        assert_eq!(g.cell, [0.0, 0.0, 16.0, 16.0]);
        assert_eq!(
            (g.scale, g.ascent, g.advance, g.offset),
            (0.5, 7.0, 9.0, 0.5)
        );
        assert_eq!(f.advance('\u{4E00}', true), 9.5);
        let (w, h, px) = f.generated_texture(&g.texture).unwrap();
        assert_eq!((w, h), (16, 16));
        assert_eq!(px.iter().filter(|p| **p != 0).count(), 16);
        assert!(f.glyph('\u{4E01}').is_none());
    }
}
