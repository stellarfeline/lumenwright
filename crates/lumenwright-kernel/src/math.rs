//! Minimal vector math that works in `no_std` (nvptx) and std (host) builds.

use core::ops::{Add, Div, Mul, Neg, Sub};

/// Square root: std's on the host, `libm`'s without `std`.
#[inline(always)]
pub fn sqrt(x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        x.sqrt()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::sqrtf(x)
    }
}

/// Largest integer not greater than `x`: std's on the host, `libm`'s without `std`.
#[inline(always)]
pub fn floor(x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        x.floor()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::floorf(x)
    }
}

/// `(sin 2πu, cos 2πu)`: the sine and cosine of `u` turns, in `f32` arithmetic alone and
/// without a branch.
///
/// The CPU and CUDA backends share this one implementation, so a sampled direction is the
/// same on both. Its reduction is exact for `|u| < 2^20`: a quarter turn is `u * 4`, a power
/// of two; adding and subtracting `1.5 * 2^23` rounds it to the nearest integer `q`; the
/// remainder `t - q` in `[-1/2, 1/2]` is exact. It evaluates the Taylor series of `sin` and
/// `cos` on `[-π/4, π/4]` to the terms in `x^9` and `x^10`, whose truncation error (below
/// `2e-9` and `1.2e-10`) is far under one `f32` ulp; the error left is rounding, held under
/// [`MAX_TURN_ERROR`] by a test against `f64`. The quadrant is applied by selects: a
/// `match` on it compiled to an indirect jump that diverges across a CUDA warp.
/// `libm::sinf`/`cosf` reduce in `f64`, which put every `f64` instruction in the CUDA
/// kernel's PTX.
#[inline(always)]
pub fn sin_cos_turns(u: f32) -> (f32, f32) {
    const ROUND: f32 = 12_582_912.0; // 1.5 * 2^23: t + ROUND has no fraction bits left.
    let t = u * 4.0;
    let q = (t + ROUND) - ROUND;
    let x = (t - q) * core::f32::consts::FRAC_PI_2;
    let x2 = x * x;
    let s = x
        * (1.0
            + x2 * (-1.0 / 6.0
                + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0)))));
    let c = 1.0
        + x2 * (-0.5
            + x2 * (1.0 / 24.0
                + x2 * (-1.0 / 720.0 + x2 * (1.0 / 40_320.0 + x2 * (-1.0 / 3_628_800.0)))));
    // sin and cos of q quarter turns plus x; `& 3` is q mod 4 for negative q too.
    let k = (q as i32) & 3;
    let odd = k & 1 != 0;
    let (s, c) = (if odd { c } else { s }, if odd { -s } else { c });
    if k & 2 != 0 { (-s, -c) } else { (s, c) }
}

/// The bound `sin_cos_turns` is tested to, in absolute error against `f64`.
pub const MAX_TURN_ERROR: f32 = 1.5e-7;

/// `e^x`: std's on the host, `libm`'s without `std`.
#[inline(always)]
pub fn exp(x: f32) -> f32 {
    #[cfg(feature = "std")]
    {
        x.exp()
    }
    #[cfg(not(feature = "std"))]
    {
        libm::expf(x)
    }
}

/// `acos x` for `x` in `[-1, 1]` in `f32` arithmetic alone (no `f64`, no call), so the
/// CPU and CUDA backends compute it alike: Abramowitz and Stegun 4.4.46,
/// `acos x = √(1−x) · (a₀ + a₁x + … + a₇x⁷)` on `[0, 1]` (approximation error at most
/// `2e-8`), and `π − acos(−x)` below 0. A test holds it to [`MAX_ACOS_ERROR`] against `f64`.
#[inline(always)]
pub fn acos(x: f32) -> f32 {
    let a = clamp(abs(x), 0.0, 1.0);
    let p = 1.570_796_3
        + a * (-0.214_598_8
            + a * (0.088_978_99
                + a * (-0.050_174_3
                    + a * (0.030_891_88
                        + a * (-0.017_088_126 + a * (0.006_670_09 + a * -0.001_262_491_1))))));
    let r = sqrt(1.0 - a) * p;
    if x < 0.0 {
        core::f32::consts::PI - r
    } else {
        r
    }
}

/// The bound [`acos`] is tested to, in absolute error against `f64` on `[-1, 1]`.
pub const MAX_ACOS_ERROR: f32 = 1e-6;

/// Absolute value.
#[inline(always)]
pub fn abs(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}

/// The smaller of `a` and `b`; `b` when either is NaN.
#[inline(always)]
pub fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// The larger of `a` and `b`; `b` when either is NaN.
#[inline(always)]
pub fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `x` limited to `[lo, hi]`.
#[inline(always)]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    min(max(x, lo), hi)
}

/// The display gamma: a texel value `c` stands for linear `c^GAMMA`, and a frame's
/// linear value `v` is shown as `v^(1/GAMMA)`. Ported from Chunky (GPL-3.0),
/// `Scene.DEFAULT_GAMMA` = 2.2, used by `ColorUtil.toLinearLut` to decode textures and
/// by `GammaCorrectionFilter` to encode frames (chunky-dev/chunky at 156e2bb).
pub const GAMMA: f32 = 2.2;

/// The natural logarithm of `x > 0` in `f32` arithmetic alone (no `f64`, no call), so
/// the CPU and CUDA backends compute it alike. `x = m·2^e` with `m` in `[√½, √2]`;
/// `ln m = 2 atanh(s)`, `s = (m−1)/(m+1)`, `|s| ≤ 0.1716`, by its series to `s⁹`
/// (truncation below `4e-9`). A test holds it to [`MAX_LN_ERROR`] against `f64`.
/// Returns `-∞` for 0 and NaN for negative input; subnormals are not handled.
#[inline(always)]
pub fn ln(x: f32) -> f32 {
    if x <= 0.0 {
        return if x == 0.0 {
            f32::NEG_INFINITY
        } else {
            f32::NAN
        };
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xFF) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    if m > core::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let series =
        2.0 * s * (1.0 + s2 * (1.0 / 3.0 + s2 * (1.0 / 5.0 + s2 * (1.0 / 7.0 + s2 * (1.0 / 9.0)))));
    e as f32 * core::f32::consts::LN_2 + series
}

/// The bound [`ln`] is tested to, in absolute error against `f64` on `[1/255, 1]`.
pub const MAX_LN_ERROR: f32 = 5e-7;

/// The bound [`gamma_to_linear`] is tested to, in relative error against `f64`, at
/// every byte value of a texel channel.
pub const MAX_GAMMA_ERROR: f32 = 1e-6;

/// A display-encoded channel value in `[0, 1]` as linear light: `c^`[`GAMMA`].
#[inline(always)]
pub fn gamma_to_linear(c: f32) -> f32 {
    if c <= 0.0 { 0.0 } else { exp(GAMMA * ln(c)) }
}

/// A three-component `f32` vector with C layout, usable in `no_std` builds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct V3 {
    /// First component.
    pub x: f32,
    /// Second component.
    pub y: f32,
    /// Third component.
    pub z: f32,
}

/// Builds a [`V3`] from its components.
#[inline(always)]
pub const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

// `mul` (component-wise) mirrors WGSL's `vec3 * vec3`; `Mul<f32>` is the scalar form.
#[allow(clippy::should_implement_trait)]
impl V3 {
    /// All components 0.
    pub const ZERO: V3 = v3(0.0, 0.0, 0.0);
    /// All components 1.
    pub const ONE: V3 = v3(1.0, 1.0, 1.0);

    /// The xyz of a vec4 buffer slot, dropping w.
    #[inline(always)]
    pub fn from4(a: [f32; 4]) -> V3 {
        v3(a[0], a[1], a[2])
    }
    /// All three components set to `v`.
    #[inline(always)]
    pub fn splat(v: f32) -> V3 {
        v3(v, v, v)
    }
    /// Dot product.
    #[inline(always)]
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    /// Cross product `self × o` (right-handed).
    #[inline(always)]
    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    /// Euclidean length.
    #[inline(always)]
    pub fn length(self) -> f32 {
        sqrt(self.dot(self))
    }
    /// Unit vector in the same direction; a zero vector is returned unchanged.
    #[inline(always)]
    pub fn normalize(self) -> V3 {
        let l = self.length();
        if l > 0.0 { self * (1.0 / l) } else { self }
    }
    /// Component-wise product, like WGSL's `vec3 * vec3`.
    #[inline(always)]
    pub fn mul(self, o: V3) -> V3 {
        v3(self.x * o.x, self.y * o.y, self.z * o.z)
    }
    /// The largest component.
    #[inline(always)]
    pub fn max_elem(self) -> f32 {
        max(self.x, max(self.y, self.z))
    }
    /// Component `i`: 0 is x, 1 is y, anything else is z.
    #[inline(always)]
    pub fn get(self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
    /// Component-wise `exp(-self * t)` (Beer-Lambert transmittance).
    #[inline(always)]
    pub fn transmittance(self, t: f32) -> V3 {
        v3(exp(-self.x * t), exp(-self.y * t), exp(-self.z * t))
    }
    /// Linear interpolation: `self` at `t = 0`, `o` at `t = 1`.
    #[inline(always)]
    pub fn lerp(self, o: V3, t: f32) -> V3 {
        self + (o - self) * t
    }
}

impl Add for V3 {
    type Output = V3;
    #[inline(always)]
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    #[inline(always)]
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    #[inline(always)]
    fn mul(self, s: f32) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f32> for V3 {
    type Output = V3;
    #[inline(always)]
    fn div(self, s: f32) -> V3 {
        self * (1.0 / s)
    }
}
impl Neg for V3 {
    type Output = V3;
    #[inline(always)]
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

/// Builds an orthonormal basis around `n` (Duff et al. 2017).
#[inline(always)]
pub fn onb(n: V3) -> (V3, V3) {
    let sign = if n.z >= 0.0 { 1.0 } else { -1.0 };
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    (
        v3(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
        v3(b, sign + n.y * n.y * a, -n.y),
    )
}
