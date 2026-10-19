//! Quick-mode texture sampling, mirrored in `quick.wgsl`.
//!
//! Block textures (the atlas rows above [`AtlasView::mip_rows`]) are sampled the
//! way the 1.21.11 client's `terrain.fsh` samples them: `sampleNearest` moves
//! the coordinate to the nearest texel centre except within half a screen pixel
//! of a texel edge, then `textureGrad` reads the mip chain with the sampler the
//! client's `LevelRenderer` builds for terrain (linear min/mag filters, so
//! trilinear between levels, clamp to edge) and, with the client's
//! *Anisotropic* texture filtering at its default 4×, anisotropic filtering.
//! The anisotropic filter here is the reference footprint approximation of the
//! `EXT_texture_filter_anisotropic` specification: up to [`MAX_ANISOTROPY`]
//! trilinear taps spread along the footprint's major axis at the level of the
//! minor axis. Everything else (entities, items, text) is read nearest at level
//! 0, as the client reads its other atlases.

use glam::{Vec2, Vec4};

pub use crate::assets::atlas::MAX_ANISOTROPY;

/// The atlas levels a quick frame samples.
#[derive(Clone, Copy)]
pub struct AtlasView<'a> {
    /// Level 0, RGBA8 with red in the low byte.
    pub level0: &'a [u32],
    /// Levels `1..=levels`, concatenated.
    pub mips: &'a [u32],
    /// Level-0 width and height.
    pub size: [u32; 2],
    /// Mip levels above 0.
    pub levels: u32,
    /// Rows `0..mip_rows` hold mipmapped textures.
    pub mip_rows: u32,
}

#[inline]
fn unpack(c: u32) -> Vec4 {
    Vec4::new(
        (c & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        ((c >> 16) & 0xFF) as f32,
        (c >> 24) as f32,
    ) / 255.0
}

impl AtlasView<'_> {
    /// The nearest level-0 texel (the sampling of everything that is not mipmapped).
    pub fn nearest(&self, uv: Vec2) -> Vec4 {
        let [w, h] = self.size;
        let x = ((uv.x * w as f32).floor() as i32).clamp(0, w as i32 - 1) as u32;
        let y = ((uv.y * h as f32).floor() as i32).clamp(0, h as i32 - 1) as u32;
        unpack(self.level0[(y * w + x) as usize])
    }

    /// Whether `uv` lies on a mipmapped texture.
    pub fn is_mipmapped(&self, uv: Vec2) -> bool {
        self.levels > 0 && uv.y * (self.size[1] as f32) < self.mip_rows as f32
    }

    fn texel(&self, l: u32, x: i32, y: i32) -> Vec4 {
        let (w, h) = ((self.size[0] >> l) as i32, (self.size[1] >> l) as i32);
        let i = (y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize;
        if l == 0 {
            unpack(self.level0[i])
        } else {
            let mut o = 0usize;
            for k in 1..l {
                o += ((self.size[0] >> k) * (self.size[1] >> k)) as usize;
            }
            unpack(self.mips[o + i])
        }
    }

    fn bilinear(&self, uv: Vec2, l: u32) -> Vec4 {
        let size = Vec2::new((self.size[0] >> l) as f32, (self.size[1] >> l) as f32);
        let t = uv * size - 0.5;
        let f = t.floor();
        let fr = t - f;
        let (x, y) = (f.x as i32, f.y as i32);
        let top = self.texel(l, x, y).lerp(self.texel(l, x + 1, y), fr.x);
        let bottom = self
            .texel(l, x, y + 1)
            .lerp(self.texel(l, x + 1, y + 1), fr.x);
        top.lerp(bottom, fr.y)
    }

    fn trilinear(&self, uv: Vec2, lod: f32) -> Vec4 {
        let lo = lod.floor();
        let fr = lod - lo;
        let l = lo as u32;
        let a = self.bilinear(uv, l);
        if fr > 0.0 && l < self.levels {
            a.lerp(self.bilinear(uv, l + 1), fr)
        } else {
            a
        }
    }

    /// `textureGrad` with the terrain sampler: anisotropic trilinear filtering
    /// of the footprint `du`, `dv` (the screen-space derivatives of `uv`).
    pub fn sample_grad(&self, uv: Vec2, du: Vec2, dv: Vec2) -> Vec4 {
        let size = Vec2::new(self.size[0] as f32, self.size[1] as f32);
        let (px, py) = ((du * size).length(), (dv * size).length());
        let (pmax, pmin) = (px.max(py), px.min(py));
        let n = (pmax / pmin.max(1e-8))
            .ceil()
            .clamp(1.0, MAX_ANISOTROPY as f32);
        let lod = (pmax / n).max(1e-8).log2().clamp(0.0, self.levels as f32);
        let axis = if px >= py { du } else { dv };
        let mut acc = Vec4::ZERO;
        let taps = n as u32;
        for i in 0..taps {
            let off = (i as f32 + 0.5) / n - 0.5;
            acc += self.trilinear(uv + axis * off, lod);
        }
        acc / n
    }

    /// The client's `sampleNearest` (`terrain.fsh`): snap to the texel centre
    /// unless within half a screen pixel of a texel edge, then `textureGrad`.
    pub fn sample_block(&self, uv: Vec2, du: Vec2, dv: Vec2) -> Vec4 {
        let size = Vec2::new(self.size[0] as f32, self.size[1] as f32);
        let pixel = Vec2::ONE / size;
        let t = uv * size;
        let center = Vec2::new(t.x.round_ties_even(), t.y.round_ties_even()) - 0.5;
        let off = t - center;
        let screen = (du * du + dv * dv).max(Vec2::splat(1e-20));
        let screen = Vec2::new(screen.x.sqrt(), screen.y.sqrt());
        let off = ((off - 0.5) * pixel / screen + 0.5).clamp(Vec2::ZERO, Vec2::ONE);
        self.sample_grad((center + off) * pixel, du, dv)
    }

    /// Samples whatever lies at `uv`: a block texture through its mip chain, or
    /// anything else nearest.
    pub fn sample(&self, uv: Vec2, du: Vec2, dv: Vec2) -> Vec4 {
        if self.is_mipmapped(uv) {
            self.sample_block(uv, du, dv)
        } else {
            self.nearest(uv)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnified_texels_stay_sharp() {
        // A 2-level 4×4 atlas with a checker; a large on-screen texel reads exactly.
        let level0: Vec<u32> = (0..16)
            .map(|i| {
                if (i % 4 + i / 4) % 2 == 0 {
                    0xFFFF_FFFF
                } else {
                    0xFF00_0000
                }
            })
            .collect();
        let mips = vec![0xFF80_8080; 4 + 1];
        let v = AtlasView {
            level0: &level0,
            mips: &mips,
            size: [4, 4],
            levels: 2,
            mip_rows: 4,
        };
        // 0.01 texels per pixel: 100 pixels per texel.
        let d = Vec2::new(0.01 / 4.0, 0.0);
        let e = Vec2::new(0.0, 0.01 / 4.0);
        let c = v.sample_block(Vec2::new(0.3 / 4.0, 0.3 / 4.0), d, e);
        assert!((c.x - 1.0).abs() < 1e-5, "{c}");
        let c = v.sample_block(Vec2::new(1.3 / 4.0, 0.3 / 4.0), d, e);
        assert!(c.x.abs() < 1e-5, "{c}");
        // 4 texels per pixel: level 2, the mip colour.
        let c = v.sample_block(Vec2::new(0.5, 0.5), d * 400.0, e * 400.0);
        assert!((c.x - 128.0 / 255.0).abs() < 1e-5, "{c}");
    }
}
