//! The image files the CLI writes.

use std::path::Path;

use anyhow::{Context, Result};

/// Writes RGBA8 bytes, row major and top row first, as a PNG, creating the
/// parent directory.
pub fn write_png_rgba(path: &Path, rgba: &[u8], w: u32, h: u32) -> Result<()> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).context("image size mismatch")?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    img.save(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Writes linear HDR as a Portable Float Map (bottom-to-top rows).
pub fn write_pfm(path: &Path, hdr: &[[f32; 3]], w: u32, h: u32) -> Result<()> {
    let mut out = format!("PF\n{w} {h}\n-1.0\n").into_bytes();
    for y in (0..h).rev() {
        for x in 0..w {
            for c in hdr[(y * w + x) as usize] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
}
