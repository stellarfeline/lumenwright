//! HDR -> display post-processing shared by every backend.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The tone-mapping operator that maps linear HDR radiance to display values. Each is
/// Chunky's post-processing filter of the same name (GPL-3.0, `renderer/postprocessing`
/// at chunky-dev/chunky 156e2bb), ending, as Chunky's do, in the display gamma
/// `v^(1/2.2)` ([`lumenwright_kernel::math::GAMMA`]).
pub enum Tonemap {
    /// Clamp and the display gamma: Chunky's default, `GammaCorrectionFilter`.
    Gamma,
    /// ACES filmic fit (Narkowicz), Chunky's `ACESFilmicFilter`.
    Aces,
    /// Hable / Uncharted 2 filmic with Chunky's default curve, `HableToneMappingFilter`
    /// at its `FILMIC_WORLDS` preset ([`HABLE_FILMIC_WORLDS`]).
    Filmic,
    /// The same Hable curve read brighter by the gain that shows linear 0.18 as the
    /// game's transfer shows a decoded colour, `0.18^(1/2.2)` ([`game_tone_curve`]):
    /// below mid-grey it sits within a few hundredths of the game's transfer, above it it
    /// rolls off along Hable's shoulder where the game would clip.
    FilmicMidGrey,
}

fn aces(x: f32) -> f32 {
    let (a, b, c, d, e) = (2.51, 0.03, 2.43, 0.59, 0.14);
    ((x * (a * x + b)) / (x * (c * x + d) + e)).clamp(0.0, 1.0)
}

/// A Hable curve's seven parameters, as Chunky's `HableToneMappingFilter` reads them
/// (`loadConfiguration`). Per channel Chunky computes `y = 2 · exposure · x`, `h(y) =
/// (y(Ay + CB) + DE) / (y(Ay + B) + DF) − E/F` with each product of two parameters in
/// float and the rest in double, the white scale `1 / h(W)` in float
/// (`recalculateWhiteScale`), and shows `h(y) · whiteScale` through the display gamma
/// (`processPixel`). Ported, with [`HableCurve::brightened`] and
/// [`HableCurve::mid_grey_gain`], from Delvewright's engine (GPL-3.0, `HableCurve` in
/// `crates/delvec/src/compiler/view/scene.rs` at stellarfeline/delvewright 0fd300da7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HableCurve {
    /// `A`, the shoulder strength.
    pub a: f32,
    /// `B`, the linear strength.
    pub b: f32,
    /// `C`, the linear angle.
    pub c: f32,
    /// `D`, the toe strength.
    pub d: f32,
    /// `E`, the toe numerator.
    pub e: f32,
    /// `F`, the toe denominator.
    pub f: f32,
    /// `W`, the linear value the curve shows as white.
    pub w: f32,
}

/// Chunky's default Hable preset, `FILMIC_WORLDS` (`HableToneMappingFilter.applyPreset`,
/// chunky-dev/chunky 156e2bb).
pub const HABLE_FILMIC_WORLDS: HableCurve = HableCurve {
    a: 0.15,
    b: 0.5,
    c: 0.1,
    d: 0.2,
    e: 0.02,
    f: 0.3,
    w: 11.2,
};

/// The mid-tone the game's tone curve is anchored at: linear 0.18, the reflectance of the
/// photographic grey card exposure is metered against (Kodak Gray Card, R-27).
pub const MID_GREY: f64 = 0.18;

impl HableCurve {
    /// `1 / h(W)`, in float arithmetic as Chunky computes it.
    pub fn white_scale(&self) -> f32 {
        let HableCurve {
            a,
            b,
            c,
            d,
            e,
            f,
            w,
        } = *self;
        let num = w * (a * w + c * b) + d * e;
        let den = w * (a * w + b) + d * f;
        1.0 / (num / den - e / f)
    }

    /// `h(y)`, the parameter products in float and the rest in double.
    pub fn h(&self, y: f64) -> f64 {
        let (a, b) = (f64::from(self.a), f64::from(self.b));
        let (cb, de, df, ef) = (
            f64::from(self.c * self.b),
            f64::from(self.d * self.e),
            f64::from(self.d * self.f),
            f64::from(self.e / self.f),
        );
        (y * (a * y + cb) + de) / (y * (a * y + b) + df) - ef
    }

    /// The value the curve holds for linear `x` at exposure 1, before the display gamma.
    pub fn before_gamma(&self, x: f64) -> f64 {
        self.h(2.0 * x) * f64::from(self.white_scale())
    }

    /// The same curve read `k` times brighter, its white held: `h_k(y) = h(k y)` is the
    /// curve with `A k²`, `B k` and `C`, `D`, `E`, `F` (multiply out the numerator and the
    /// denominator), and `W / k` keeps `h_k(W / k) = h(W)`, so the white scale is the
    /// preset's.
    pub fn brightened(&self, k: f64) -> HableCurve {
        HableCurve {
            a: (f64::from(self.a) * k * k) as f32,
            b: (f64::from(self.b) * k) as f32,
            w: (f64::from(self.w) / k) as f32,
            ..*self
        }
    }

    /// The gain `k` at which [`HableCurve::brightened`] holds linear `m` at `m` before the
    /// display gamma, so `m` shows as the game's transfer shows it. With `y = 2km` and
    /// `q = m / whiteScale + E/F`, `h(y) = m / whiteScale` is the quadratic
    /// `A(1 − q)y² + (CB − qB)y + (DE − qDF) = 0`, whose positive root is the one on the
    /// curve's rising branch.
    pub fn mid_grey_gain(&self, m: f64) -> f64 {
        let (a, b) = (f64::from(self.a), f64::from(self.b));
        let (cb, de, df, ef) = (
            f64::from(self.c * self.b),
            f64::from(self.d * self.e),
            f64::from(self.d * self.f),
            f64::from(self.e / self.f),
        );
        let q = m / f64::from(self.white_scale()) + ef;
        let (qa, qb, qc) = (a * (1.0 - q), cb - q * b, de - q * df);
        let y = (-qb + (qb * qb - 4.0 * qa * qc).sqrt()) / (2.0 * qa);
        y / (2.0 * m)
    }
}

/// **The game's tone curve** ([`Tonemap::FilmicMidGrey`]): Chunky's Hable preset
/// brightened by the gain that shows [`MID_GREY`] as the game shows it (1.463761), the
/// curve Delvewright's engine writes for every Chunky scene (`scene::tone_curve`).
pub fn game_tone_curve() -> HableCurve {
    HABLE_FILMIC_WORLDS.brightened(HABLE_FILMIC_WORLDS.mid_grey_gain(MID_GREY))
}

/// Encodes one linear channel value, clamped to `[0, 1]`, for display: `v^(1/GAMMA)`.
pub fn encode(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
        .powf(1.0 / lumenwright_kernel::math::GAMMA)
}

/// Converts linear radiance to packed display RGBA8, rounding as Chunky's
/// `ColorUtil.getRGB` does.
pub fn tonemap(hdr: &[[f32; 3]], exposure: f32, op: Tonemap) -> Vec<u32> {
    let curve = match op {
        Tonemap::FilmicMidGrey => game_tone_curve(),
        _ => HABLE_FILMIC_WORLDS,
    };
    let white = f64::from(curve.white_scale());
    hdr.iter()
        .map(|p| {
            let m = |v: f32| {
                let v = (v * exposure).max(0.0);
                let v = match op {
                    Tonemap::Gamma => v,
                    Tonemap::Aces => aces(v),
                    Tonemap::Filmic | Tonemap::FilmicMidGrey => {
                        (curve.h(2.0 * f64::from(v)) * white) as f32
                    }
                };
                (encode(v) * 255.0 + 0.5) as u32
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
        // Chunky's gamma filter: 0.18 linear shows as 0.18^(1/2.2) = 0.4586 -> 117.
        assert_eq!(tonemap(&[[0.18; 3]], 1.0, Tonemap::Gamma)[0] & 0xFF, 117);
    }

    /// The game's curve holds mid-grey at mid-grey before the display gamma, so 0.18 shows
    /// at `0.18^(1/2.2)` = 0.458656 as under the gamma filter; it is the preset read
    /// 1.463761 times brighter (the engine's `k` at the pin), with the preset's white.
    #[test]
    fn the_game_tone_curve_shows_mid_grey_as_the_game() {
        let k = HABLE_FILMIC_WORLDS.mid_grey_gain(MID_GREY);
        assert!((k - 1.463761).abs() < 1e-6, "{k}");
        let g = game_tone_curve();
        assert!((g.before_gamma(MID_GREY) - MID_GREY).abs() < 1e-6);
        assert_eq!(
            [g.a, g.b, g.c, g.d, g.e, g.f, g.w],
            [0.32138956, 0.7318806, 0.1, 0.2, 0.02, 0.3, 7.651521]
        );
        // The preset at k times the light, over 1e-4..7e2.
        let mut x = 1e-4;
        while x < 7e2 {
            let (a, b) = (
                g.before_gamma(x),
                HABLE_FILMIC_WORLDS.h(2.0 * k * x) * f64::from(HABLE_FILMIC_WORLDS.white_scale()),
            );
            assert!((a - b).abs() <= 1e-5 * b.abs().max(1e-6), "{x}: {a} {b}");
            x *= 1.5;
        }
        let px = tonemap(&[[0.18; 3]], 1.0, Tonemap::FilmicMidGrey)[0] & 0xFF;
        assert_eq!(px, tonemap(&[[0.18; 3]], 1.0, Tonemap::Gamma)[0] & 0xFF);
        // At the preset 0.18 shows darker, at 0.393 (Chunky's own default curve).
        let preset = tonemap(&[[0.18; 3]], 1.0, Tonemap::Filmic)[0] & 0xFF;
        assert_eq!(preset, (0.393285f32 * 255.0 + 0.5) as u32);
        // White is white under both.
        let w = g.w;
        assert_eq!(
            tonemap(&[[w; 3]], 1.0, Tonemap::FilmicMidGrey)[0],
            0xFFFF_FFFF
        );
    }

    #[test]
    fn downsample_averages() {
        let px = vec![0xFF00_0000, 0xFFFF_FFFF, 0xFF00_0000, 0xFFFF_FFFF];
        let d = downsample(&px, 2, 2, 2);
        assert_eq!(d[0] & 0xFF, 128);
    }
}
