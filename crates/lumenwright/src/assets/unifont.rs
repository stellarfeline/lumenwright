//! The Unifont the client draws every character with that no bitmap font covers
//! (CJK, Hangul, and the rest), read the way the 1.21.11 client reads it.
//!
//! The client jar's `font/include/unifont.json` lists no providers. The real one, with
//! the hex files it names, is in the launcher's asset index, which the client lays over
//! the jar (`DefaultClientPackResources` asks the index first). Lumenwright takes those
//! files as a directory named like the index's own keys:
//!
//! ```text
//! minecraft/font/include/unifont.json
//! minecraft/font/unifont.zip
//! ```
//!
//! The client's `UnihexProvider` (obfuscated `gon`, with `gon$b` as its definition) is
//! what this module follows:
//!
//! * `gon$b.a(InputStream)` reads every zip entry ending in `.hex` as `<code point>:<rows>`
//!   lines (`gon.a(InputStream, gon$h)`: 4 to 6 hex digits, a colon, then 32, 64, 96 or
//!   128 hex digits, which are sixteen rows of 8, 16, 24 or 32 pixels, the leftmost pixel
//!   in the top bit). A later entry replaces an earlier one.
//! * Each `size_overrides` entry (`from`, `to`, `left`, `right`) takes the glyphs in its
//!   inclusive range out of the pool and gives them those columns.
//! * Every glyph left in the pool is cut to the columns it lights: `gon$f.d()`, left =
//!   leading zeros of the OR of its rows, right = 31 minus its trailing zeros; a blank
//!   glyph is `left` 0 and `right` its row width.
//! * A glyph is `right - left + 1` pixels wide and 16 tall, drawn at an oversample of 2
//!   (`gon$d$2.d()`), so half a font pixel per texel, with its top at the line's top
//!   (`SheetGlyphInfo.getUp` is `7 - 7`). Its advance is `width / 2 + 1` with Java's
//!   integer division (`gon$d$1.getAdvance`); its bold and shadow offsets are 0.5.
//! * A provider whose `filter` names an option takes part only when that option is off
//!   (`jp` and `uniform` both default to off).

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use super::font::Glyph;
use super::source::split_loc;

/// Where, inside a unifont directory, the provider list lives.
pub const INCLUDE: &str = "minecraft/font/include/unifont.json";

/// Texture locations of generated unihex glyphs start with this.
pub const TEXTURE_PREFIX: &str = "lumenwright:font/unihex/";

/// `GlyphInfo.getBoldOffset` and `getShadowOffset` of a unihex glyph.
pub const OFFSET: f32 = 0.5;

/// Rows per glyph.
const ROWS: usize = 16;

/// Whether a provider takes part under the client's default options: every `filter`
/// entry must name an option that is off with the value `false`.
pub fn provider_enabled(provider: &serde_json::Value) -> bool {
    provider
        .get("filter")
        .and_then(|f| f.as_object())
        .is_none_or(|f| f.values().all(|v| v.as_bool() == Some(false)))
}

/// One hex-font glyph, cut to its columns.
#[derive(Clone, Debug, PartialEq)]
pub struct HexGlyph {
    /// Sixteen rows, the leftmost pixel in bit 31.
    rows: [u32; ROWS],
    /// Pixels per row in the hex file (8, 16, 24 or 32).
    bits: u32,
    left: i32,
    right: i32,
}

impl HexGlyph {
    fn auto(rows: [u32; ROWS], bits: u32) -> HexGlyph {
        let lit = rows.iter().fold(0, |a, r| a | r);
        let (left, right) = if lit == 0 {
            (0, bits as i32)
        } else {
            (
                lit.leading_zeros() as i32,
                32 - lit.trailing_zeros() as i32 - 1,
            )
        };
        HexGlyph {
            rows,
            bits,
            left,
            right,
        }
    }

    /// Pixels wide.
    fn width(&self) -> i32 {
        (self.right - self.left + 1).max(1)
    }

    /// `UnihexProvider.getAdvance`: half the width, rounded down, plus one.
    pub fn advance(&self) -> f32 {
        (self.width() / 2 + 1) as f32
    }

    /// The sprite of the glyph for `c`: a texture made in code, 16 texels tall, drawn at
    /// half a font pixel per texel.
    pub fn glyph(&self, c: char) -> Glyph {
        let w = self.width() as f32;
        Glyph {
            texture: texture_loc(c),
            texture_size: [w, ROWS as f32],
            cell: [0.0, 0.0, w, ROWS as f32],
            scale: 0.5,
            ascent: 7.0,
            advance: self.advance(),
            offset: OFFSET,
        }
    }

    /// The glyph's texels: white where a pixel is lit, clear elsewhere.
    pub fn image(&self) -> (u32, u32, Vec<u32>) {
        let w = self.width();
        let mut px = Vec::with_capacity(w as usize * ROWS);
        for row in &self.rows {
            for x in 0..w {
                let bit = 31 - self.left - x;
                let lit = (0..32).contains(&bit) && (row >> bit) & 1 == 1;
                px.push(if lit { 0xFFFF_FFFF } else { 0 });
            }
        }
        (w as u32, ROWS as u32, px)
    }
}

/// The texture location a unihex glyph's sprite is registered under.
pub fn texture_loc(c: char) -> String {
    format!("{TEXTURE_PREFIX}{:x}", c as u32)
}

/// The character a [`texture_loc`] names.
pub fn texture_char(loc: &str) -> Option<char> {
    char::from_u32(u32::from_str_radix(loc.strip_prefix(TEXTURE_PREFIX)?, 16).ok()?)
}

/// One `<code point>:<rows>` line.
fn parse_line(line: &str) -> Result<(u32, [u32; ROWS], u32), String> {
    let (cp, hex) = line
        .split_once(':')
        .ok_or_else(|| format!("no colon in {line:?}"))?;
    if !(4..=6).contains(&cp.len()) {
        return Err(format!("code point {cp:?} is not 4 to 6 digits"));
    }
    let cp = u32::from_str_radix(cp, 16)
        .ok()
        .filter(|_| cp.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'F')))
        .ok_or_else(|| format!("code point {cp:?} is not upper-case hex"))?;
    let bits = match hex.len() {
        32 => 8,
        64 => 16,
        96 => 24,
        128 => 32,
        n => {
            return Err(format!(
                "{n} hex digits for U+{cp:04X}, expected 32, 64, 96 or 128"
            ));
        }
    };
    if !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'F')) {
        return Err(format!("bitmap of U+{cp:04X} is not upper-case hex"));
    }
    let digits = (bits / 4) as usize;
    let mut rows = [0; ROWS];
    for (i, row) in rows.iter_mut().enumerate() {
        let v = u32::from_str_radix(&hex[i * digits..(i + 1) * digits], 16).unwrap_or(0);
        *row = v << (32 - bits);
    }
    Ok((cp, rows, bits))
}

/// Reads the glyphs of one hex zip as the provider `provider` (a `unihex` provider
/// object) cuts them. The result is ordered by code point.
pub fn read_hex_zip(
    zip_bytes: &[u8],
    provider: &serde_json::Value,
) -> Result<Vec<(u32, HexGlyph)>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| format!("not a zip: {e}"))?;
    let mut pool: BTreeMap<u32, ([u32; ROWS], u32)> = BTreeMap::new();
    let mut found = false;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.to_string();
        if !name.ends_with(".hex") {
            continue;
        }
        found = true;
        for (n, line) in std::io::BufReader::new(entry).lines().enumerate() {
            let line = line.map_err(|e| format!("{name}: {e}"))?;
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let (cp, rows, bits) =
                parse_line(line).map_err(|e| format!("{name} line {}: {e}", n + 1))?;
            pool.insert(cp, (rows, bits));
        }
    }
    if !found {
        return Err("holds no .hex file".to_string());
    }
    let mut out: BTreeMap<u32, HexGlyph> = BTreeMap::new();
    let overrides = provider
        .get("size_overrides")
        .and_then(|o| o.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    for o in overrides {
        let cp = |k: &str| {
            o.get(k)
                .and_then(|v| v.as_str())
                .and_then(|s| s.chars().next())
                .map(|c| c as u32)
        };
        let num = |k: &str| o.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
        let (Some(from), Some(to), Some(left), Some(right)) =
            (cp("from"), cp("to"), num("left"), num("right"))
        else {
            return Err(format!("size override {o} needs from, to, left and right"));
        };
        for c in from..=to {
            if let Some((rows, bits)) = pool.remove(&c) {
                out.insert(
                    c,
                    HexGlyph {
                        rows,
                        bits,
                        left,
                        right,
                    },
                );
            }
        }
    }
    for (c, (rows, bits)) in pool {
        out.insert(c, HexGlyph::auto(rows, bits));
    }
    Ok(out.into_iter().collect())
}

/// The `hex_file` locations (`minecraft:font/unifont.zip`) of the providers in a
/// `unifont.json` that take part under the client's default options.
pub fn hex_files(include_json: &[u8]) -> Result<Vec<String>, String> {
    let v: serde_json::Value = serde_json::from_slice(include_json).map_err(|e| e.to_string())?;
    let providers = v
        .get("providers")
        .and_then(|p| p.as_array())
        .ok_or("no providers list")?;
    Ok(providers
        .iter()
        .filter(|p| provider_enabled(p) && p.get("type").and_then(|t| t.as_str()) == Some("unihex"))
        .filter_map(|p| p.get("hex_file").and_then(|f| f.as_str()))
        .map(str::to_string)
        .collect())
}

/// The path of a hex file inside a unifont directory.
pub fn hex_path(dir: &Path, loc: &str) -> PathBuf {
    let (ns, path) = split_loc(loc);
    dir.join(ns).join(path)
}

/// Why a unifont directory is refused.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifontError {
    /// A file the directory must hold is not there.
    Missing(PathBuf),
    /// A file is there but is not what the client reads.
    Invalid {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        why: String,
    },
}

impl fmt::Display for UnifontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnifontError::Missing(p) => write!(f, "unifont file {} does not exist", p.display()),
            UnifontError::Invalid { path, why } => {
                write!(f, "unifont file {} is not usable: {why}", path.display())
            }
        }
    }
}

impl std::error::Error for UnifontError {}

fn read_file(path: &Path) -> Result<Vec<u8>, UnifontError> {
    std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            UnifontError::Missing(path.to_path_buf())
        } else {
            UnifontError::Invalid {
                path: path.to_path_buf(),
                why: e.to_string(),
            }
        }
    })
}

/// A hex zip holds a `.hex` entry whose first line is a glyph.
fn check_first_line(path: &Path, bytes: &[u8]) -> Result<(), UnifontError> {
    let invalid = |why: String| UnifontError::Invalid {
        path: path.to_path_buf(),
        why,
    };
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| invalid(format!("not a zip: {e}")))?;
    let name = (0..zip.len())
        .filter_map(|i| Some(zip.name_for_index(i)?.ok()?.to_string()))
        .find(|n| n.ends_with(".hex"))
        .ok_or_else(|| invalid("holds no .hex file".to_string()))?;
    let mut first = String::new();
    let entry = zip.by_name(&name).map_err(|e| invalid(e.to_string()))?;
    std::io::BufReader::new(entry.take(4096))
        .read_line(&mut first)
        .map_err(|e| invalid(format!("{name}: {e}")))?;
    parse_line(first.trim_end()).map_err(|e| invalid(format!("{name} line 1: {e}")))?;
    Ok(())
}

/// The files a unifont directory supplies, as `(path in the pack, bytes)`: the
/// provider list and every hex file it names. A file that is missing or is not what
/// the client reads is refused by name; each hex zip's first line is parsed.
pub fn read_dir(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, UnifontError> {
    let include = dir.join(INCLUDE);
    let json = read_file(&include)?;
    let invalid = |path: &Path, why: String| UnifontError::Invalid {
        path: path.to_path_buf(),
        why,
    };
    let files = hex_files(&json).map_err(|e| invalid(&include, e))?;
    if files.is_empty() {
        return Err(invalid(&include, "names no unihex hex file".to_string()));
    }
    let mut out = vec![(
        "assets/minecraft/font/include/unifont.json".to_string(),
        json,
    )];
    for loc in files {
        let path = hex_path(dir, &loc);
        let bytes = read_file(&path)?;
        check_first_line(&path, &bytes)?;
        let (ns, rest) = split_loc(&loc);
        out.push((format!("assets/{ns}/{rest}"), bytes));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(name: &str, body: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
            z.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        out
    }

    /// 16 rows of 16 pixels with column `col` lit on every row.
    fn column16(cp: u32, col: u32) -> String {
        format!(
            "{cp:04X}:{}",
            format!("{:04X}", 0x8000u32 >> col).repeat(16)
        )
    }

    #[test]
    fn a_glyph_is_cut_to_the_columns_it_lights() {
        let body = format!("{}\n{}\n", column16(0x4E00, 3), column16(0x4E01, 9));
        let g = read_hex_zip(&zip_of("a.hex", &body), &serde_json::json!({})).unwrap();
        let (cp, a) = &g[0];
        assert_eq!(*cp, 0x4E00);
        assert_eq!((a.left, a.right, a.width()), (3, 3, 1));
        // One pixel wide: advance 1 / 2 + 1 = 1.
        assert_eq!(a.advance(), 1.0);
        let (w, h, px) = a.image();
        assert_eq!((w, h, px.len()), (1, 16, 16));
        assert!(px.iter().all(|p| *p == 0xFFFF_FFFF));
        assert_eq!(g[1].1.advance(), 1.0);
    }

    #[test]
    fn a_size_override_replaces_the_cut() {
        let body = format!("{}\n", column16(0x4E00, 3));
        let p = serde_json::json!({"size_overrides": [
            {"from": "\u{4E00}", "to": "\u{9FFF}", "left": 0, "right": 15}]});
        let g = read_hex_zip(&zip_of("a.hex", &body), &p).unwrap();
        let a = &g[0].1;
        assert_eq!((a.left, a.right, a.width()), (0, 15, 16));
        assert_eq!(a.advance(), 9.0);
        let (w, _, px) = a.image();
        assert_eq!(w, 16);
        let lit: Vec<usize> = (0..16).filter(|x| px[*x] != 0).collect();
        assert_eq!(lit, [3]);
    }

    #[test]
    fn rows_of_every_width_are_left_aligned() {
        let line = format!("00E9:{}", "80".repeat(16));
        let (cp, rows, bits) = parse_line(&line).unwrap();
        assert_eq!((cp, bits, rows[0]), (0xE9, 8, 0x8000_0000));
        let line = format!("1F600:{}", "000001".repeat(16));
        let (_, rows, bits) = parse_line(&line).unwrap();
        assert_eq!((bits, rows[0]), (24, 0x0000_0100));
        let line = format!("1F600:{}", "00000001".repeat(16));
        assert_eq!(parse_line(&line).unwrap().1[0], 1);
    }

    #[test]
    fn a_blank_glyph_is_as_wide_as_its_row_plus_one() {
        let g = read_hex_zip(
            &zip_of("a.hex", &format!("0020:{}\n", "00".repeat(16))),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(g[0].1.width(), 9);
        assert_eq!(g[0].1.advance(), 5.0);
    }

    #[test]
    fn malformed_lines_are_refused() {
        for bad in [
            "4E00:ABC",
            "4e00:".to_string().as_str(),
            "XYZW:00",
            "no colon",
        ] {
            assert!(parse_line(bad).is_err(), "{bad}");
        }
        let e = read_hex_zip(&zip_of("a.hex", "4E00:12\n"), &serde_json::json!({})).unwrap_err();
        assert!(e.contains("a.hex line 1"), "{e}");
        let e = read_hex_zip(&zip_of("a.txt", "x"), &serde_json::json!({})).unwrap_err();
        assert!(e.contains("no .hex"), "{e}");
    }

    #[test]
    fn filters_follow_the_default_options() {
        let on = |s: &str| provider_enabled(&serde_json::from_str(s).unwrap());
        assert!(on(r#"{"type":"bitmap"}"#));
        assert!(on(r#"{"filter":{"uniform":false}}"#));
        assert!(!on(r#"{"filter":{"uniform":true}}"#));
        assert!(!on(r#"{"filter":{"jp":true}}"#));
    }

    #[test]
    fn texture_locations_round_trip() {
        assert_eq!(texture_char(&texture_loc('收')), Some('收'));
        assert_eq!(texture_char("minecraft:font/ascii"), None);
    }
}
