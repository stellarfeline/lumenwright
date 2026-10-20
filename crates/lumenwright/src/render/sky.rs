//! Sun position, sky colours and fog shared by both modes, and the quick-mode sky.
//!
//! The quick-mode sky is the 1.21.11 client's (`net.minecraft.client.renderer.SkyRenderer`,
//! drawn by `LevelRenderer.addSkyPass`), evaluated per pixel as a function of the view
//! direction instead of drawn as geometry, so the CPU rasterizer and `quick.wgsl` agree:
//!
//! 1. the frame is cleared to the fog colour;
//! 2. the sky disc (`buildSkyDisc`: an eight-sided fan at height 16, radius 512) in the sky
//!    colour, fogged towards the fog colour by its interpolated vertex distances with the
//!    `sky` shader's fog (`apply_fog` with start 0 and end `FogSkyEnd`);
//! 3. the sunrise/sunset glow (`buildSunriseFan`, `renderSunriseAndSunset`), alpha blended;
//! 4. the sun (size 30) and the moon (size 20, this night's phase) at distance 100, added
//!    with `BlendFunction.OVERLAY`;
//! 5. the 1500 stars of `buildStars` (seed 10842), rotated by the star angle, each adding
//!    the square of the star brightness (`OVERLAY` with colour and alpha both the brightness).
//!
//! The rotation of every celestial body is `Axis.YP(-90°)` then `Axis.XP(angle)`.

use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Vec3};

use super::CameraRays;

/// Vanilla celestial angle (0 = noon) for a time of day in ticks: the pre-1.21.11
/// `DimensionType.timeOfDay` curve, which the quick lightmap's daylight factor uses.
pub fn celestial_angle(ticks: i64) -> f32 {
    let f = ((ticks.rem_euclid(24000)) as f64 / 24000.0 - 0.25).rem_euclid(1.0);
    let g = 0.5 - (f * std::f64::consts::PI).cos() / 2.0;
    ((f * 2.0 + g) / 3.0) as f32
}

/// Unit vector towards the sun for a celestial angle (sun rises in the east).
pub fn sun_direction_from_angle(angle: f32) -> Vec3 {
    let t = angle * std::f32::consts::TAU;
    Vec3::new(-t.sin(), t.cos(), 0.0).normalize()
}

/// Sun direction from altitude/azimuth in degrees. Azimuth 0 = north, 90 = east.
pub fn sun_direction(altitude_deg: f32, azimuth_deg: f32) -> Vec3 {
    let (alt, az) = (altitude_deg.to_radians(), azimuth_deg.to_radians());
    Vec3::new(az.sin() * alt.cos(), alt.sin(), -az.cos() * alt.cos()).normalize()
}

/// Vanilla `skyDarken`-style daylight factor in [0.2, 1].
pub fn daylight(angle: f32) -> f32 {
    let f = 1.0 - ((angle * std::f32::consts::TAU).cos() * 2.0 + 0.2);
    let f = 1.0 - f.clamp(0.0, 1.0);
    f * 0.8 + 0.2
}

/// `Axis.YP.rotationDegrees(-90)` then `Axis.XP.rotationDegrees(angle)`: the
/// rotation the client applies to the sun, the moon and the stars.
pub fn celestial_rotation(angle_deg: f32) -> Mat3 {
    Mat3::from_rotation_y(-std::f32::consts::FRAC_PI_2)
        * Mat3::from_rotation_x(angle_deg.to_radians())
}

/// Unit vector towards a body at a celestial angle in degrees (the sun at
/// `visual/sun_angle`, the moon at `visual/moon_angle`).
pub fn celestial_direction(angle_deg: f32) -> Vec3 {
    celestial_rotation(angle_deg) * Vec3::Y
}

/// Star bins: azimuth × elevation cells over the star sphere.
const STAR_BINS_AZ: u32 = 128;
const STAR_BINS_EL: u32 = 64;
/// Words per star entry: centre, then the two half-axes divided by their squared length.
const STAR_WORDS: usize = 9;

/// Java's `LegacyRandomSource` (the 48-bit LCG of `java.util.Random`).
struct LegacyRandom(u64);

impl LegacyRandom {
    const MASK: u64 = (1 << 48) - 1;
    fn new(seed: i64) -> LegacyRandom {
        LegacyRandom((seed as u64 ^ 0x5_DEEC_E66D) & Self::MASK)
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5_DEEC_E66D).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i64 as i32
    }
    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5e-8
    }
    /// `BitRandomSource.nextDouble`: the 53-bit integer times a float constant,
    /// so computed in float.
    fn next_double(&mut self) -> f64 {
        let hi = self.next(26) as i64;
        let lo = self.next(27) as i64;
        (((hi << 27) + lo) as f32 * 1.110_223e-16_f32) as f64
    }
}

/// One star of `SkyRenderer.buildStars`: its centre at distance 100 and the two
/// half-axes of its square.
#[derive(Clone, Copy, Debug)]
pub struct Star {
    /// Centre, in star space.
    pub center: Vec3,
    /// Half-axis along the square's first side.
    pub a1: Vec3,
    /// Half-axis along the square's second side.
    pub a2: Vec3,
}

/// The client's 1500 star draws (those inside the unit ball's shell), in star space.
pub fn stars() -> &'static [Star] {
    static S: OnceLock<Vec<Star>> = OnceLock::new();
    S.get_or_init(|| {
        let mut r = LegacyRandom::new(10842);
        let mut out = Vec::new();
        for _ in 0..1500 {
            let x = r.next_float() * 2.0 - 1.0;
            let y = r.next_float() * 2.0 - 1.0;
            let z = r.next_float() * 2.0 - 1.0;
            let size = 0.15 + r.next_float() * 0.1;
            let len2 = x * x + y * y + z * z;
            if len2 <= 0.010_000_001 || len2 >= 1.0 {
                continue;
            }
            let v = Vec3::new(x, y, z);
            let center = v * (100.0 / v.length());
            let theta = (r.next_double() * std::f32::consts::PI as f64 * 2.0) as f32;
            // Matrix3f().rotateTowards(-center, +Y).rotateZ(-theta).
            let ndir = (-center).normalize();
            let left = Vec3::Y.cross(ndir).normalize();
            let upn = ndir.cross(left);
            let (s, c) = (-theta).sin_cos();
            let a1 = (left * c + upn * s) * size;
            let a2 = (left * -s + upn * c) * size;
            out.push(Star { center, a1, a2 });
        }
        out
    })
}

fn star_bin(q: Vec3) -> (u32, u32) {
    let az = q.z.atan2(q.x);
    let el = q.y.clamp(-1.0, 1.0).asin();
    let ia = (((az + std::f32::consts::PI) / std::f32::consts::TAU) * STAR_BINS_AZ as f32).floor()
        as i32;
    let ie = (((el + std::f32::consts::FRAC_PI_2) / std::f32::consts::PI) * STAR_BINS_EL as f32)
        .floor() as i32;
    (
        ia.clamp(0, STAR_BINS_AZ as i32 - 1) as u32,
        ie.clamp(0, STAR_BINS_EL as i32 - 1) as u32,
    )
}

/// The stars binned by direction for per-pixel lookup, as `u32` words: one start
/// index per bin plus an end (`STAR_BINS_AZ * STAR_BINS_EL + 1` words), then
/// nine float words per entry (centre, `a1 / |a1|²`, `a2 / |a2|²`).
/// A star is entered in every bin its angular extent, with a margin, touches.
pub fn star_table() -> &'static [u32] {
    static T: OnceLock<Vec<u32>> = OnceLock::new();
    T.get_or_init(|| {
        let nbins = (STAR_BINS_AZ * STAR_BINS_EL) as usize;
        let mut bins: Vec<Vec<usize>> = vec![Vec::new(); nbins];
        let (da, de) = (
            std::f32::consts::TAU / STAR_BINS_AZ as f32,
            std::f32::consts::PI / STAR_BINS_EL as f32,
        );
        for (i, s) in stars().iter().enumerate() {
            let dir = s.center / 100.0;
            let radius = 2.0 * (s.a1.length() + s.a2.length()) / 100.0;
            let el = dir.y.asin();
            let az = dir.z.atan2(dir.x);
            let (el0, el1) = (el - radius, el + radius);
            let ie0 = (((el0 + std::f32::consts::FRAC_PI_2) / de).floor() as i32).max(0);
            let ie1 = (((el1 + std::f32::consts::FRAC_PI_2) / de).floor() as i32)
                .min(STAR_BINS_EL as i32 - 1);
            let cos_min = el0
                .abs()
                .max(el1.abs())
                .min(std::f32::consts::FRAC_PI_2)
                .cos();
            let half = if cos_min < 0.05 {
                std::f32::consts::PI
            } else {
                radius / cos_min
            };
            let ia0 = ((az - half + std::f32::consts::PI) / da).floor() as i32;
            let ia1 = ((az + half + std::f32::consts::PI) / da).floor() as i32;
            for ie in ie0..=ie1 {
                if half >= std::f32::consts::PI {
                    for ia in 0..STAR_BINS_AZ as i32 {
                        bins[(ie as u32 * STAR_BINS_AZ + ia as u32) as usize].push(i);
                    }
                    continue;
                }
                for ia in ia0..=ia1 {
                    let ia = ia.rem_euclid(STAR_BINS_AZ as i32) as u32;
                    let b = (ie as u32 * STAR_BINS_AZ + ia) as usize;
                    if !bins[b].contains(&i) {
                        bins[b].push(i);
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(nbins + 1);
        let mut n = 0u32;
        for b in &bins {
            out.push(n);
            n += b.len() as u32;
        }
        out.push(n);
        for b in &bins {
            for &i in b {
                let s = stars()[i];
                let a1 = s.a1 / s.a1.length_squared();
                let a2 = s.a2 / s.a2.length_squared();
                for v in [s.center, a1, a2] {
                    out.extend(v.to_array().map(f32::to_bits));
                }
            }
        }
        out
    })
}

/// Everything the quick-mode sky is drawn from, already evaluated for one frame.
#[derive(Clone, Copy, Debug)]
pub struct SkyDescription {
    /// `visual/sky_color`, sRGB 0..1: the sky disc.
    pub sky: Vec3,
    /// The fog colour (`AtmosphericFogEnvironment.getBaseColor`), sRGB 0..1: the clear
    /// colour, the sky disc's fog and the terrain's fog.
    pub fog: Vec3,
    /// `FogSkyEnd`: `min(render distance, visual/sky_fog_end_distance)`, blocks.
    pub sky_fog_end: f32,
    /// Render-distance fog (cylindrical distance) start and end, blocks.
    pub render_fog: (f32, f32),
    /// Environmental fog (spherical distance) start and end, blocks.
    pub env_fog: (f32, f32),
    /// Whether the sky disc is drawn (`Skybox.OVERWORLD`).
    pub sky_disc: bool,
    /// `visual/sunrise_sunset_color` as sRGB 0..1 with its alpha; alpha ≤ 0.001 draws no glow.
    pub sunrise: [f32; 4],
    /// `visual/sun_angle`, degrees: which side the glow is drawn on.
    pub sun_angle: f32,
    /// The sun: direction and its sprite's atlas rectangle (x, y, w, h) in texels.
    pub sun: Option<(Vec3, [f32; 4])>,
    /// The moon: direction and this phase's sprite rectangle.
    pub moon: Option<(Vec3, [f32; 4])>,
    /// `visual/star_angle`, degrees.
    pub star_angle: f32,
    /// `visual/star_brightness`, 0..1; 0 draws no stars.
    pub star_brightness: f32,
}

/// The quick-mode sky and fog as the shaders read them. Mirrored in `quick.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct QuickSky {
    /// xyz = sky disc colour (sRGB 0..1), w = `FogSkyEnd`.
    pub sky_color: [f32; 4],
    /// xyz = fog colour, w = render-distance fog start.
    pub fog_color: [f32; 4],
    /// x = render-distance fog end, y/z = environmental fog start/end, w = what one star adds.
    pub fog_dist: [f32; 4],
    /// rgb = sunrise glow colour, a = its alpha at the fan's centre.
    pub sunrise: [f32; 4],
    /// x = 1 to draw the sky disc, y = 1 to draw the glow, z = 1 to draw stars.
    pub flags: [f32; 4],
    /// The sun quad: centre (w = 1 when drawn), u axis, v axis, atlas rect.
    pub sun: [[f32; 4]; 4],
    /// The moon quad, laid out as `sun`.
    pub moon: [[f32; 4]; 4],
    /// Rows of the world-to-star-space rotation.
    pub star_rot: [[f32; 4]; 3],
    /// The glow fan: centre then 17 rim points (camera-relative), w = alpha.
    pub fan: [[f32; 4]; 18],
    /// Camera forward vector in xyz.
    pub cam_forward: [f32; 4],
    /// Camera right vector in xyz.
    pub cam_right: [f32; 4],
    /// Camera up vector in xyz.
    pub cam_up: [f32; 4],
    /// x = tan(fov/2), y = aspect, z = width, w = height
    pub cam_misc: [f32; 4],
}

/// `linear_fog_value` from the client's `fog.glsl`.
#[inline]
pub fn linear_fog(v: f32, start: f32, end: f32) -> f32 {
    if v <= start {
        0.0
    } else if v >= end {
        1.0
    } else {
        (v - start) / (end - start)
    }
}

const RIM_SPHERICAL: f32 = 512.249_94;

/// Lays a sprite quad of half-size `size` at distance 100 towards `dir`, with the
/// client's axes: local x along the sky's +Z, local z = local x × dir. `flip`
/// mirrors both texture axes (the moon's UVs run the other way).
fn body_quad(dir: Vec3, size: f32, rect: [f32; 4], flip: bool) -> [[f32; 4]; 4] {
    let c = dir * 100.0;
    let mut ex = Vec3::Z - dir * dir.z;
    if ex.length_squared() < 1e-8 {
        ex = Vec3::X;
    }
    let ex = ex.normalize();
    let ez = ex.cross(dir);
    let s = if flip { -1.0 / size } else { 1.0 / size };
    [
        [c.x, c.y, c.z, 1.0],
        (ex * s).extend(0.0).to_array(),
        (ez * s).extend(0.0).to_array(),
        rect,
    ]
}

impl QuickSky {
    /// Packs the quick-mode sky for a camera and an output size.
    pub fn new(cam: &CameraRays, width: u32, height: u32, d: &SkyDescription) -> Self {
        let mut q = QuickSky {
            sky_color: d.sky.extend(d.sky_fog_end).to_array(),
            fog_color: d.fog.extend(d.render_fog.0).to_array(),
            fog_dist: [
                d.render_fog.1,
                d.env_fog.0,
                d.env_fog.1,
                d.star_brightness * d.star_brightness,
            ],
            sunrise: d.sunrise,
            flags: [
                d.sky_disc as u32 as f32,
                (d.sky_disc && d.sunrise[3] > 0.001) as u32 as f32,
                (d.sky_disc && d.star_brightness > 0.0) as u32 as f32,
                0.0,
            ],
            cam_forward: cam.forward.extend(0.0).to_array(),
            cam_right: cam.right.extend(0.0).to_array(),
            cam_up: cam.up.extend(0.0).to_array(),
            cam_misc: [cam.tan_half_fov, cam.aspect, width as f32, height as f32],
            ..Default::default()
        };
        if d.sky_disc {
            if let Some((dir, rect)) = d.sun {
                q.sun = body_quad(dir, 30.0, rect, false);
            }
            if let Some((dir, rect)) = d.moon {
                q.moon = body_quad(dir, 20.0, rect, true);
            }
        }
        let inv = celestial_rotation(d.star_angle).transpose();
        for (i, r) in q.star_rot.iter_mut().enumerate() {
            *r = inv.row(i).extend(0.0).to_array();
        }
        // renderSunriseAndSunset: XP(90°), ZP(flip + 90°), then the fan scaled by
        // the glow's alpha along z.
        let flip = if d.sun_angle.to_radians().sin() < 0.0 {
            180.0f32
        } else {
            0.0
        };
        let m = Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * Mat3::from_rotation_z((flip + 90.0).to_radians())
            * Mat3::from_diagonal(Vec3::new(1.0, 1.0, d.sunrise[3]));
        q.fan[0] = (m * Vec3::new(0.0, 100.0, 0.0))
            .extend(d.sunrise[3])
            .to_array();
        for k in 0..=16 {
            let t = k as f32 * std::f32::consts::TAU / 16.0;
            let (s, c) = t.sin_cos();
            q.fan[k + 1] = (m * Vec3::new(s * 120.0, c * 120.0, -c * 40.0))
                .extend(0.0)
                .to_array();
        }
        q
    }

    /// View direction through pixel centre (x, y).
    pub fn pixel_dir(&self, x: f32, y: f32) -> Vec3 {
        let m = self.cam_misc;
        let sx = (2.0 * (x + 0.5) / m[2] - 1.0) * m[1] * m[0];
        let sy = (1.0 - 2.0 * (y + 0.5) / m[3]) * m[0];
        (Vec3::from_slice(&self.cam_forward[..3])
            + Vec3::from_slice(&self.cam_right[..3]) * sx
            + Vec3::from_slice(&self.cam_up[..3]) * sy)
            .normalize()
    }

    /// The sky disc's colour along `d`, or `None` past its rim (or below the horizon).
    fn disc(&self, d: Vec3) -> Option<Vec3> {
        if d.y <= 0.0 {
            return None;
        }
        let p = d * (16.0 / d.y);
        let step = std::f32::consts::FRAC_PI_4;
        let phi = p.z.atan2(p.x);
        let k = ((phi + std::f32::consts::PI) / step)
            .floor()
            .clamp(0.0, 7.0);
        let th0 = -std::f32::consts::PI + k * step;
        let (s0, c0) = th0.sin_cos();
        let (s1, c1) = (th0 + step).sin_cos();
        let (r0x, r0z, r1x, r1z) = (512.0 * c0, 512.0 * s0, 512.0 * c1, 512.0 * s1);
        let det = r0x * r1z - r1x * r0z;
        let b1 = (p.x * r1z - r1x * p.z) / det;
        let b2 = (r0x * p.z - p.x * r0z) / det;
        let s = b1 + b2;
        if s > 1.0 {
            return None;
        }
        let end = self.sky_color[3];
        let sph = 16.0 * (1.0 - s) + RIM_SPHERICAL * s;
        let cyl = 16.0 * (1.0 - s) + 512.0 * s;
        let f = linear_fog(sph, 0.0, end).max(linear_fog(cyl, end, end));
        let sky = Vec3::from_slice(&self.sky_color[..3]);
        Some(sky.lerp(Vec3::from_slice(&self.fog_color[..3]), f))
    }

    /// The glow's alpha along `d` (nearest fan triangle hit).
    fn fan_alpha(&self, d: Vec3) -> f32 {
        let c = Vec3::from_slice(&self.fan[0][..3]);
        let mut best_t = f32::INFINITY;
        let mut alpha = 0.0;
        for k in 0..16 {
            let v1 = Vec3::from_slice(&self.fan[k + 1][..3]);
            let v2 = Vec3::from_slice(&self.fan[k + 2][..3]);
            let (e1, e2) = (v1 - c, v2 - c);
            let pv = d.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-6 {
                continue;
            }
            let inv = 1.0 / det;
            let tv = -c;
            let u = tv.dot(pv) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = tv.cross(e1);
            let v = d.dot(qv) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(qv) * inv;
            if t > 0.0 && t < best_t {
                best_t = t;
                alpha = (1.0 - u - v) * self.fan[0][3];
            }
        }
        alpha
    }

    /// A sun or moon quad's texel along `d` (straight alpha), or zero.
    fn body(b: &[[f32; 4]; 4], d: Vec3, atlas: &[u32], atlas_w: u32) -> [f32; 4] {
        if b[0][3] == 0.0 || b[3][2] <= 0.0 {
            return [0.0; 4];
        }
        let c = Vec3::from_slice(&b[0][..3]);
        let dn = d.dot(c);
        if dn <= 0.0 {
            return [0.0; 4];
        }
        let q = d * (c.dot(c) / dn) - c;
        let lx = q.dot(Vec3::from_slice(&b[1][..3]));
        let lz = q.dot(Vec3::from_slice(&b[2][..3]));
        if lx.abs() >= 1.0 || lz.abs() >= 1.0 {
            return [0.0; 4];
        }
        let r = b[3];
        let u = r[0] + (lx * 0.5 + 0.5) * r[2];
        let v = r[1] + (lz * 0.5 + 0.5) * r[3];
        let t = atlas[(v.floor() as u32 * atlas_w + u.floor() as u32) as usize];
        [
            (t & 0xFF) as f32 / 255.0,
            ((t >> 8) & 0xFF) as f32 / 255.0,
            ((t >> 16) & 0xFF) as f32 / 255.0,
            (t >> 24) as f32 / 255.0,
        ]
    }

    /// How many stars lie along `d`.
    fn star_hits(&self, d: Vec3, table: &[u32]) -> u32 {
        let row = |i: usize| Vec3::from_slice(&self.star_rot[i][..3]);
        let q = Vec3::new(row(0).dot(d), row(1).dot(d), row(2).dot(d));
        let (ia, ie) = star_bin(q);
        let b = (ie * STAR_BINS_AZ + ia) as usize;
        let base = (STAR_BINS_AZ * STAR_BINS_EL + 1) as usize;
        let f = |w: usize| f32::from_bits(table[w]);
        let mut n = 0;
        for e in table[b]..table[b + 1] {
            let o = base + e as usize * STAR_WORDS;
            let v = |k: usize| Vec3::new(f(o + 3 * k), f(o + 3 * k + 1), f(o + 3 * k + 2));
            let c = v(0);
            let dn = q.dot(c);
            if dn <= 0.0 {
                continue;
            }
            let p = q * (c.dot(c) / dn) - c;
            if p.dot(v(1)).abs() < 1.0 && p.dot(v(2)).abs() < 1.0 {
                n += 1;
            }
        }
        n
    }

    /// Background colour (sRGB 0..1) for a view direction: the sky passes in order,
    /// each clamped to 0..1 as the client's 8-bit target clamps them.
    pub fn background(&self, d: Vec3, atlas: &[u32], atlas_w: u32, stars: &[u32]) -> Vec3 {
        let mut c = Vec3::from_slice(&self.fog_color[..3]);
        if self.flags[0] > 0.5
            && let Some(disc) = self.disc(d)
        {
            c = disc;
        }
        if self.flags[1] > 0.5 {
            let a = self.fan_alpha(d);
            c = c.lerp(Vec3::from_slice(&self.sunrise[..3]), a);
        }
        for b in [&self.sun, &self.moon] {
            let t = Self::body(b, d, atlas, atlas_w);
            c = (c + Vec3::new(t[0], t[1], t[2]) * t[3]).min(Vec3::ONE);
        }
        if self.flags[2] > 0.5 {
            let n = self.star_hits(d, stars);
            for _ in 0..n {
                c = (c + Vec3::splat(self.fog_dist[3])).min(Vec3::ONE);
            }
        }
        c.clamp(Vec3::ZERO, Vec3::ONE)
    }

    /// The client's world fog (`fog.glsl` `total_fog_value`): the larger of the
    /// environmental fog over spherical distance and the render-distance fog over
    /// cylindrical distance.
    pub fn fog_factor(&self, rel: Vec3) -> f32 {
        let sph = rel.length();
        let cyl = (rel.x * rel.x + rel.z * rel.z).sqrt().max(rel.y.abs());
        linear_fog(sph, self.fog_dist[1], self.fog_dist[2]).max(linear_fog(
            cyl,
            self.fog_color[3],
            self.fog_dist[0],
        ))
    }
}

/// The sun and sky HQ mode traces (linear RGB).
#[derive(Clone, Copy, Debug)]
pub struct HqSky {
    /// Unit vector towards the sun.
    pub sun_dir: Vec3,
    /// Sun irradiance, linear RGB.
    pub sun_color: Vec3,
    /// Scale of the Preetham sky radiance: Chunky's sky light (1 by default).
    pub sky_light: f32,
}

/// Chunky's default sun intensity (`Sun.DEFAULT_INTENSITY`, chunky-dev/chunky 156e2bb).
pub const DEFAULT_SUN_INTENSITY: f32 = 1.25;

/// The sun's angular radius in radians (Chunky's `Sun.DEFAULT_RADIUS`, the `radius` of
/// the benchmark scene, chunky-dev/chunky 156e2bb).
pub const SUN_RADIUS: f32 = 0.03;

/// The brightness of the sun texture drawn in the sky: Chunky's `apparentTextureBrightness`
/// times 10 (`Sun.intersect`), where `apparentTextureBrightness` is the apparent brightness
/// to the power 2.2 and the apparent brightness defaults to `Sun.DEFAULT_INTENSITY`
/// (chunky-dev/chunky 156e2bb). Chunky sets it apart from the sun's intensity.
pub const SUN_TEXTURE_BRIGHTNESS: f32 = 16.338_118;

/// The axes along which the sun texture lies in the sky for a unit vector `sw` towards
/// the sun: Chunky's `Sun.initSun` (chunky-dev/chunky 156e2bb). `su` starts as up, or as
/// +x when `sw` is within 0.1 of the yz plane; `sv = normalize(sw × su)`; `su = sv × sw`.
pub fn sun_texture_axes(sw: Vec3) -> (Vec3, Vec3) {
    let sw = sw.as_dvec3();
    let su = if sw.x.abs() > 0.1 {
        glam::DVec3::Y
    } else {
        glam::DVec3::X
    };
    let sv = sw.cross(su).normalize();
    let su = sv.cross(sw);
    (su.as_vec3(), sv.as_vec3())
}

/// The sun's altitude at which civil twilight ends: its centre 6 degrees below the
/// horizon (the definition the US Naval Observatory uses).
pub const CIVIL_TWILIGHT_END: f32 = -6.0 * std::f32::consts::PI / 180.0;

/// The sky light of a night, as a fraction of the sky's: Lumenwright's.
pub const NIGHT_SKY_LIGHT: f32 = 0.03;

/// The fraction of the Preetham sky's light a sun at `altitude` (radians) leaves: 1
/// while any part of the sun's disc is above the horizon (Chunky's sky, which has no
/// other behaviour), falling linearly to [`NIGHT_SKY_LIGHT`] at [`CIVIL_TWILIGHT_END`]
/// and holding it below. The fall is Lumenwright's own: Chunky has no time of day, and
/// its Preetham sky keeps the sunset sky at any altitude below the horizon.
pub fn twilight(altitude: f32) -> f32 {
    ((altitude - CIVIL_TWILIGHT_END) / (-SUN_RADIUS - CIVIL_TWILIGHT_END))
        .clamp(0.0, 1.0)
        .max(NIGHT_SKY_LIGHT)
}

/// Derives the HQ sun and sky for a sun direction. The sun is Chunky's (GPL-3.0,
/// `Sun.initSun` at chunky-dev/chunky 156e2bb): white, with irradiance
/// `sun_intensity^2.2` (`Scene.DEFAULT_GAMMA`) at every altitude; the kernel hides the
/// part of its disc below the horizon. The sky is Chunky's default, the Preetham sky
/// ([`Preetham`]), scaled by `sky_intensity` (Chunky's sky light) and by [`twilight`].
pub fn hq_sky(sun_dir: Vec3, sun_intensity: f32, sky_intensity: f32) -> HqSky {
    let altitude = sun_dir.y.clamp(-1.0, 1.0).asin();
    HqSky {
        sun_dir,
        sun_color: Vec3::ONE * sun_intensity.powf(lumenwright_kernel::math::GAMMA),
        sky_light: sky_intensity * twilight(altitude),
    }
}

/// The Preetham sky's parameters for one sun position, as the kernel evaluates them
/// (`Params::sky_sun`, `sky_zenith`, `perez_a` to `perez_e`).
///
/// Ported from Chunky (GPL-3.0, `renderer/scene/sky/PreethamSky.java` at
/// chunky-dev/chunky 156e2bb), which implements A. J. Preetham, P. Shirley and B. Smits,
/// "A Practical Analytic Model for Daylight", SIGGRAPH 1999: the Perez distribution
/// coefficients of the chromaticities x, y and the luminance Y for turbidity 2.5, the
/// zenith values, and their normalisation by the distribution at the zenith (`f0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preetham {
    /// The sun direction the sky is lit by: the sun's altitude limited to `[0, π/2]`
    /// (mirrored beyond the zenith), as `PreethamSky.updateSun` does.
    pub sun: Vec3,
    /// The zenith value times its normalisation, for x, y and Y.
    pub zenith: [f32; 3],
    /// The Perez coefficients A to E, each for x, y and Y.
    pub perez: [[f32; 3]; 5],
}

impl Preetham {
    /// The sky for a unit vector towards the sun.
    pub fn new(sun_dir: Vec3) -> Preetham {
        const TURB: f64 = 2.5;
        const X_ZENITH: [[f64; 4]; 3] = [
            [0.00166, -0.00375, 0.00209, 0.0],
            [-0.02903, 0.06377, -0.03203, 0.00394],
            [0.11693, -0.21196, 0.06052, 0.25886],
        ];
        const Y_ZENITH: [[f64; 4]; 3] = [
            [0.00275, -0.00610, 0.00317, 0.0],
            [-0.04214, 0.08970, -0.04153, 0.00516],
            [0.15346, -0.26756, 0.06670, 0.26688],
        ];
        const MDX: [[f64; 2]; 5] = [
            [-0.0193, -0.2592],
            [-0.0665, 0.0008],
            [-0.0004, 0.2125],
            [-0.0641, -0.8989],
            [-0.0033, 0.0452],
        ];
        const MDY: [[f64; 2]; 5] = [
            [-0.0167, -0.2608],
            [-0.0950, 0.0092],
            [-0.0079, 0.2102],
            [-0.0441, -1.6537],
            [-0.0109, 0.0529],
        ];
        const MD_LUM: [[f64; 2]; 5] = [
            [0.1787, -1.4630],
            [-0.3554, 0.4275],
            [-0.0227, 5.3251],
            [0.1206, -2.5771],
            [-0.0670, 0.3703],
        ];
        let d = sun_dir.as_dvec3().normalize();
        let azimuth = d.z.atan2(d.x);
        // `updateSun`: the altitude clamped to [0, π], then mirrored beyond π/2.
        let mut alt = d.y.clamp(-1.0, 1.0).asin().clamp(0.0, std::f64::consts::PI);
        if alt > std::f64::consts::FRAC_PI_2 {
            alt = std::f64::consts::PI - alt;
        }
        let r = alt.cos().abs();
        let sun = glam::DVec3::new(azimuth.cos() * r, alt.sin(), azimuth.sin() * r);

        let coeff =
            |m: &[[f64; 2]; 5]| -> [f64; 5] { std::array::from_fn(|i| m[i][0] * TURB + m[i][1]) };
        let (cx, cy, cl) = (coeff(&MDX), coeff(&MDY), coeff(&MD_LUM));
        let perez_f = |cos_theta: f64, gamma: f64, cos2_gamma: f64, c: &[f64; 5]| -> f64 {
            (1.0 + c[0] * (c[1] / cos_theta).exp())
                * (1.0 + c[2] * (c[3] * gamma).exp() + c[4] * cos2_gamma)
        };
        let chroma = |sun_theta: f64, m: &[[f64; 4]; 3]| -> f64 {
            let (t1, t2, t3) = (
                sun_theta,
                sun_theta * sun_theta,
                sun_theta * sun_theta * sun_theta,
            );
            let row = |r: &[f64; 4]| r[0] * t3 + r[1] * t2 + r[2] * t1 + r[3];
            TURB * TURB * row(&m[0]) + TURB * row(&m[1]) + row(&m[2])
        };
        let sun_theta = std::f64::consts::FRAC_PI_2 - alt;
        let cos2_theta = sun_theta.cos().powi(2);
        let chi = (4.0 / 9.0 - TURB / 120.0) * (std::f64::consts::PI - 2.0 * sun_theta);
        let zenith_lum = ((4.0453 * TURB - 4.9710) * chi.tan() - 0.2155 * TURB + 2.4192).abs();
        let zenith_x = chroma(sun_theta, &X_ZENITH);
        let zenith_y = chroma(sun_theta, &Y_ZENITH);
        let f0 = |c: &[f64; 5]| 1.0 / perez_f(1.0, sun_theta, cos2_theta, c);
        Preetham {
            sun: sun.as_vec3(),
            zenith: [
                (zenith_x * f0(&cx)) as f32,
                (zenith_y * f0(&cy)) as f32,
                (zenith_lum * f0(&cl)) as f32,
            ],
            perez: std::array::from_fn(|i| [cx[i] as f32, cy[i] as f32, cl[i] as f32]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_follows_vanilla_day_cycle() {
        assert!(celestial_angle(6000).abs() < 1e-4);
        let noon = sun_direction_from_angle(celestial_angle(6000));
        assert!(noon.y > 0.999);
        let morning = sun_direction_from_angle(celestial_angle(0));
        // Vanilla's eased curve puts the tick-0 sun ~12 degrees above the eastern horizon.
        assert!(
            morning.x > 0.95 && morning.y > 0.1 && morning.y < 0.3,
            "sun rises in the east: {morning}"
        );
        let evening = sun_direction_from_angle(celestial_angle(12000));
        assert!(
            evening.x < -0.95 && evening.y > 0.0,
            "sets in the west: {evening}"
        );
        assert!(daylight(celestial_angle(6000)) > 0.99);
        assert!(daylight(celestial_angle(18000)) < 0.25);
    }

    /// Chunky's `PreethamSky.calcIncidentLight`, transcribed in `f64` from the Java with
    /// its own `updateSkylightValues`, against the kernel's `sky_radiance` fed from
    /// [`Preetham::new`].
    #[test]
    fn the_kernel_sky_is_chunkys_preetham_sky() {
        fn chunky(sun_alt: f64, sun_az: f64, d: [f64; 3]) -> [f64; 3] {
            let turb = 2.5f64;
            let mdx = [
                [-0.0193, -0.2592],
                [-0.0665, 0.0008],
                [-0.0004, 0.2125],
                [-0.0641, -0.8989],
                [-0.0033, 0.0452],
            ];
            let mdy = [
                [-0.0167, -0.2608],
                [-0.0950, 0.0092],
                [-0.0079, 0.2102],
                [-0.0441, -1.6537],
                [-0.0109, 0.0529],
            ];
            let mdl = [
                [0.1787, -1.4630],
                [-0.3554, 0.4275],
                [-0.0227, 5.3251],
                [0.1206, -2.5771],
                [-0.0670, 0.3703],
            ];
            let xz = [
                [0.00166, -0.00375, 0.00209, 0.0],
                [-0.02903, 0.06377, -0.03203, 0.00394],
                [0.11693, -0.21196, 0.06052, 0.25886],
            ];
            let yz = [
                [0.00275, -0.00610, 0.00317, 0.0],
                [-0.04214, 0.08970, -0.04153, 0.00516],
                [0.15346, -0.26756, 0.06670, 0.26688],
            ];
            let c = |m: [[f64; 2]; 5]| m.map(|r| r[0] * turb + r[1]);
            let (a, b, cl) = (c(mdx), c(mdy), c(mdl));
            let perez = |ct: f64, g: f64, c2g: f64, k: [f64; 5]| {
                (1.0 + k[0] * (k[1] / ct).exp()) * (1.0 + k[2] * (k[3] * g).exp() + k[4] * c2g)
            };
            let chroma = |st: f64, m: [[f64; 4]; 3]| {
                let r = |r: [f64; 4]| r[0] * st * st * st + r[1] * st * st + r[2] * st + r[3];
                turb * turb * r(m[0]) + turb * r(m[1]) + r(m[2])
            };
            let st = std::f64::consts::FRAC_PI_2 - sun_alt;
            let c2t = st.cos() * st.cos();
            let chi = (4.0 / 9.0 - turb / 120.0) * (std::f64::consts::PI - 2.0 * st);
            let zy = ((4.0453 * turb - 4.9710) * chi.tan() - 0.2155 * turb + 2.4192).abs();
            let (zx, zyc) = (chroma(st, xz), chroma(st, yz));
            let (f0x, f0y, f0l) = (
                1.0 / perez(1.0, st, c2t, a),
                1.0 / perez(1.0, st, c2t, b),
                1.0 / perez(1.0, st, c2t, cl),
            );
            let r = sun_alt.cos().abs();
            let sw = [sun_az.cos() * r, sun_alt.sin(), sun_az.sin() * r];
            let ct = d[1].max(0.0);
            let cg = d[0] * sw[0] + d[1] * sw[1] + d[2] * sw[2];
            let g = cg.acos();
            let x = zx * perez(ct, g, cg * cg, a) * f0x;
            let y = zyc * perez(ct, g, cg * cg, b) * f0y;
            let z = zy * perez(ct, g, cg * cg, cl) * f0l;
            let f = z / y;
            let (x2, y2, z2) = (x * f, z, (1.0 - x - y) * f);
            [
                (2.3706743 * x2 - 0.9000405 * y2 - 0.4706338 * z2) * 0.045,
                (-0.513885 * x2 + 1.4253036 * y2 + 0.0885814 * z2) * 0.045,
                (0.0052982 * x2 - 0.0146949 * y2 + 1.0093968 * z2) * 0.045,
            ]
        }
        let mut worst = 0.0f64;
        let mut n = 0;
        for (alt, az) in [(45f64, 45f64), (10.0, 200.0), (80.0, 300.0), (2.0, 90.0)] {
            let (alt, az) = (alt.to_radians(), az.to_radians());
            let sun = Vec3::new(
                (az.cos() * alt.cos()) as f32,
                alt.sin() as f32,
                (az.sin() * alt.cos()) as f32,
            );
            let sky = Preetham::new(sun);
            let f4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
            let params = lumenwright_kernel::Params {
                sky_sun: f4(sky.sun.into(), 1.0),
                sky_zenith: f4(sky.zenith, 0.0),
                perez_a: f4(sky.perez[0], 0.0),
                perez_b: f4(sky.perez[1], 0.0),
                perez_c: f4(sky.perez[2], 0.0),
                perez_d: f4(sky.perez[3], 0.0),
                perez_e: f4(sky.perez[4], 0.0),
                ..Default::default()
            };
            let view = lumenwright_kernel::SceneView {
                params: &params,
                grid: &[],
                models: &[],
                quads: &[],
                atlas: &[],
                emitters: &[],
                emitter_grid: &[],
            };
            for i in 0..64 {
                for j in 0..=16 {
                    let phi = i as f64 / 64.0 * std::f64::consts::TAU;
                    let h = j as f64 / 16.0 * 1.2 - 0.2; // a little below the horizon too
                    let r = (1.0 - h * h).sqrt();
                    let d = [phi.cos() * r, h, phi.sin() * r];
                    let want = chunky(alt, az, d);
                    let got = lumenwright_kernel::integrator::sky_radiance(
                        &view,
                        lumenwright_kernel::math::v3(d[0] as f32, d[1] as f32, d[2] as f32),
                    );
                    for (g, w) in [got.x, got.y, got.z].into_iter().zip(want) {
                        worst = worst.max((g as f64 - w).abs() / w.abs().max(0.01));
                    }
                    n += 1;
                }
            }
        }
        eprintln!("Preetham sky: worst relative difference {worst:.2e} over {n} directions");
        assert_eq!(n, 4 * 64 * 17);
        assert!(worst < 1e-4, "{worst:.2e}");
    }

    /// The sky keeps Chunky's full Preetham light while the sun's disc touches the
    /// horizon and falls to the night's at the end of civil twilight; the sun is never
    /// dimmed on the host.
    #[test]
    fn twilight_holds_chunkys_sky_until_the_disc_sets() {
        assert_eq!(twilight(0.5), 1.0);
        assert_eq!(twilight(0.0), 1.0);
        assert_eq!(twilight(-SUN_RADIUS), 1.0);
        assert!(twilight(-SUN_RADIUS - 1e-3) < 1.0);
        assert_eq!(twilight(CIVIL_TWILIGHT_END), NIGHT_SKY_LIGHT);
        assert_eq!(twilight(-std::f32::consts::FRAC_PI_2), NIGHT_SKY_LIGHT);
        let mut last = 1.0;
        for i in 0..=100 {
            let t = twilight(-SUN_RADIUS + (CIVIL_TWILIGHT_END + SUN_RADIUS) * i as f32 / 100.0);
            assert!(t <= last, "monotone");
            last = t;
        }
        // The Treehouse Camp's --time 12800 sun: altitude -0.005091 rad.
        let dusk = hq_sky(sun_direction(-0.005091f32.to_degrees(), 270.0), 1.25, 1.0);
        assert_eq!(dusk.sky_light, 1.0);
        assert_eq!(dusk.sun_color, Vec3::splat(1.25f32.powf(2.2)));
        let night = hq_sky(-Vec3::Y, 1.25, 1.0);
        assert_eq!(night.sky_light, NIGHT_SKY_LIGHT);
    }

    #[test]
    fn the_sun_texture_is_as_bright_as_chunkys() {
        assert!(
            (SUN_TEXTURE_BRIGHTNESS as f64 - 10.0 * 1.25f64.powf(2.2)).abs() < 1e-4,
            "{}",
            10.0 * 1.25f64.powf(2.2)
        );
        let (su, sv) = sun_texture_axes(Vec3::new(-0.999987, -0.005091, 0.0));
        assert!(
            (su - Vec3::new(-0.005091, 0.999987, 0.0)).length() < 1e-5,
            "{su}"
        );
        assert!((sv - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5, "{sv}");
    }

    /// Chunky's `Sun.intersect` and `Texture.getColor`, transcribed in `f64` from the
    /// Java, against the kernel's `sun_texture` on a 32 × 32 sprite whose every texel
    /// differs, placed at (3, 5) in a larger atlas.
    #[test]
    fn the_kernel_draws_chunkys_sun_texture() {
        let (aw, ah) = (40u32, 40u32);
        let texel = |x: u32, y: u32| -> u32 {
            0xFF00_0000 | (x * 7 + 3) | ((y * 7 + 3) << 8) | (((x + y) * 3) << 16)
        };
        let mut atlas = vec![0u32; (aw * ah) as usize];
        for y in 0..32 {
            for x in 0..32 {
                atlas[((y + 5) * aw + x + 3) as usize] = texel(x, y);
            }
        }
        let lin = |c: u32, s: u32| ((c >> s) & 0xFF) as f64 / 255.0;
        let chunky = |sw: glam::DVec3, d: glam::DVec3| -> Option<(u32, u32)> {
            let r = SUN_RADIUS as f64;
            if d.dot(sw) < 0.5 {
                return None;
            }
            let su0 = if sw.x.abs() > 0.1 {
                glam::DVec3::Y
            } else {
                glam::DVec3::X
            };
            let sv = sw.cross(su0).normalize();
            let su = sv.cross(sw);
            let (width, width2) = (r * 4.0, r * 8.0);
            let a = std::f64::consts::FRAC_PI_2 - d.dot(su).acos() + width;
            let b = std::f64::consts::FRAC_PI_2 - d.dot(sv).acos() + width;
            if !(a >= 0.0 && a < width2 && b >= 0.0 && b < width2) {
                return None;
            }
            let x = ((a / width2) * 32.0 - 5e-5) as i32;
            let y = ((1.0 - b / width2) * 32.0 - 5e-5) as i32;
            Some((x.clamp(0, 31) as u32, y.clamp(0, 31) as u32))
        };
        let (mut drawn, mut checked, mut edge) = (0, 0, 0);
        for sun in [
            Vec3::new(-0.999987, -0.005091, 0.0),
            sun_direction(8.0, 330.0),
            sun_direction(45.0, 135.0),
            sun_direction(80.0, 3.0),
        ] {
            let (su, sv) = sun_texture_axes(sun);
            let params = lumenwright_kernel::Params {
                sun_dir: [sun.x, sun.y, sun.z, SUN_RADIUS.cos()],
                sun_su: [su.x, su.y, su.z, SUN_TEXTURE_BRIGHTNESS],
                sun_sv: [sv.x, sv.y, sv.z, 4.0 * SUN_RADIUS],
                sun_sprite: [3.0, 5.0, 32.0, 32.0],
                off2: [0, aw, ah, 0],
                ..Default::default()
            };
            let view = lumenwright_kernel::SceneView {
                params: &params,
                grid: &[],
                models: &[],
                quads: &[],
                atlas: &atlas,
                emitters: &[],
                emitter_grid: &[],
            };
            let sw = sun.as_dvec3();
            for i in -60..=60 {
                for j in -60..=60 {
                    // Directions over the square the texture covers and a little beyond.
                    let (da, db) = (i as f64 * 0.0025, j as f64 * 0.0025);
                    let d = (sw + su.as_dvec3() * da + sv.as_dvec3() * db).normalize();
                    let want = chunky(sw, d);
                    let got = lumenwright_kernel::integrator::sun_texture(
                        &view,
                        lumenwright_kernel::math::v3(d.x as f32, d.y as f32, d.z as f32),
                    );
                    checked += 1;
                    let Some((x, y)) = want else {
                        if got.x != 0.0 || got.y != 0.0 || got.z != 0.0 {
                            edge += 1;
                        }
                        continue;
                    };
                    let c = texel(x, y);
                    let w = [lin(c, 0), lin(c, 8), lin(c, 16)]
                        .map(|v| v.powf(2.2) * SUN_TEXTURE_BRIGHTNESS as f64);
                    let g = [got.x as f64, got.y as f64, got.z as f64];
                    if (0..3).all(|k| (g[k] - w[k]).abs() <= 1e-4 * w[k].max(1.0)) {
                        drawn += 1;
                    } else {
                        // An f32 direction on a texel border may fall in the next texel.
                        edge += 1;
                    }
                }
            }
        }
        eprintln!(
            "sun texture: {drawn} drawn texels agree, {edge} on texel or square borders differ, of {checked} directions"
        );
        assert_eq!(checked, 4 * 121 * 121);
        assert!(drawn > 4 * 90 * 90, "{drawn}");
        assert!(edge * 100 < checked, "{edge}");
    }

    #[test]
    fn altitude_azimuth() {
        assert!((sun_direction(90.0, 0.0) - Vec3::Y).length() < 1e-5);
        assert!((sun_direction(0.0, 90.0) - Vec3::X).length() < 1e-5);
        assert!((sun_direction(0.0, 0.0) + Vec3::Z).length() < 1e-5);
    }
}
