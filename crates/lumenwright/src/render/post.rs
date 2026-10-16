//! HDR -> display post-processing shared by every backend.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tonemap {
    /// Clamp + sRGB (Chunky's default "gamma correction").
    Gamma,
    /// ACES filmic fit (Narkowicz).
    Aces,
    /// Hable / Uncharted 2 filmic.
    Filmic,
}

fn aces(x: f32) -> f32 {
    let (a, b, c, d, e) = (2.51, 0.03, 2.43, 0.59, 0.14);
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

fn hable(x: f32) -> f32 {
    let (a, b, c, d, e, f) = (0.15, 0.50, 0.10, 0.20, 0.02, 0.30);
    ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Converts linear radiance to packed sRGB RGBA8.
pub fn tonemap(hdr: &[[f32; 3]], exposure: f32, op: Tonemap) -> Vec<u32> {
    let white = 1.0 / hable(11.2);
    hdr.iter()
        .map(|p| {
            let m = |v: f32| {
                let v = (v * exposure).max(0.0);
                let v = match op {
                    Tonemap::Gamma => v,
                    Tonemap::Aces => aces(v),
                    Tonemap::Filmic => hable(v * 2.0) * white,
                };
                (linear_to_srgb(v) * 255.0 + 0.5) as u32
            };
            m(p[0]) | (m(p[1]) << 8) | (m(p[2]) << 16) | 0xFF00_0000
        })
        .collect()
}

/// Box-filters an RGBA8 image down by an integer factor.
pub fn downsample(px: &[u32], w: u32, h: u32, k: u32) -> Vec<u32> {
    if k <= 1 {
        return px.to_vec();
    }
    let (ow, oh) = (w / k, h / k);
    let mut out = Vec::with_capacity((ow * oh) as usize);
    for y in 0..oh {
        for x in 0..ow {
            let mut acc = [0u32; 4];
            for dy in 0..k {
                for dx in 0..k {
                    let c = px[((y * k + dy) * w + x * k + dx) as usize];
                    for (i, a) in acc.iter_mut().enumerate() {
                        *a += (c >> (8 * i)) & 0xFF;
                    }
                }
            }
            let n = k * k;
            out.push(
                acc.iter()
                    .enumerate()
                    .map(|(i, a)| ((a + n / 2) / n) << (8 * i))
                    .sum(),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tonemap_endpoints() {
        let px = tonemap(&[[0.0; 3], [1.0; 3], [100.0; 3]], 1.0, Tonemap::Gamma);
        assert_eq!(px[0], 0xFF00_0000);
        assert_eq!(px[1], 0xFFFF_FFFF);
        assert_eq!(px[2], 0xFFFF_FFFF);
        let a = tonemap(&[[0.18; 3]], 1.0, Tonemap::Aces)[0] & 0xFF;
        assert!(a > 100 && a < 200, "{a}");
    }

    #[test]
    fn downsample_averages() {
        let px = vec![0xFF00_0000, 0xFFFF_FFFF, 0xFF00_0000, 0xFFFF_FFFF];
        let d = downsample(&px, 2, 2, 2);
        assert_eq!(d[0] & 0xFF, 128);
    }
}
