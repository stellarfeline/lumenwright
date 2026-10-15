//! Sun position, sky colours and fog shared by both modes.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use super::CameraRays;

/// Vanilla celestial angle (0 = noon) for a time of day in ticks.
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

/// Background, fog and sun for quick mode. Mirrored in `quick.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct QuickSky {
    pub sky_color: [f32; 4],
    /// w = fog start
    pub fog_color: [f32; 4],
    /// xyz = sun direction, w = fog end
    pub sun_dir: [f32; 4],
    /// Sun sprite rect in the atlas (x, y, w, h) in pixels; w = 0 disables it.
    pub sun_rect: [f32; 4],
    pub cam_forward: [f32; 4],
    pub cam_right: [f32; 4],
    pub cam_up: [f32; 4],
    /// x = tan(fov/2), y = aspect, z = width, w = height
    pub cam_misc: [f32; 4],
}

impl QuickSky {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        cam: &CameraRays,
        width: u32,
        height: u32,
        sky: Vec3,
        fog: Vec3,
        sun: Vec3,
        fog_range: (f32, f32),
        sun_rect: [f32; 4],
    ) -> Self {
        QuickSky {
            sky_color: sky.extend(1.0).to_array(),
            fog_color: [fog.x, fog.y, fog.z, fog_range.0],
            sun_dir: [sun.x, sun.y, sun.z, fog_range.1],
            sun_rect,
            cam_forward: cam.forward.extend(0.0).to_array(),
            cam_right: cam.right.extend(0.0).to_array(),
            cam_up: cam.up.extend(0.0).to_array(),
            cam_misc: [cam.tan_half_fov, cam.aspect, width as f32, height as f32],
        }
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

    /// Background colour (sRGB 0..1) for a view direction.
    pub fn background(&self, d: Vec3, atlas: &[u32], atlas_w: u32) -> Vec3 {
        let sky = Vec3::from_slice(&self.sky_color[..3]);
        let fog = Vec3::from_slice(&self.fog_color[..3]);
        let t = (d.y * 3.0).clamp(0.0, 1.0);
        let mut c = fog.lerp(sky, t);
        if self.sun_rect[2] > 0.0 {
            let s = Vec3::from_slice(&self.sun_dir[..3]);
            let cs = d.dot(s);
            if cs > 0.0 {
                // Sun quad in the plane perpendicular to the sun direction (vanilla size 30 at 100).
                let t1 = if s.y.abs() < 0.99 {
                    s.cross(Vec3::Y).normalize()
                } else {
                    Vec3::X
                };
                let t2 = t1.cross(s);
                let px = d.dot(t1) / cs / 0.3;
                let py = d.dot(t2) / cs / 0.3;
                if px.abs() < 1.0 && py.abs() < 1.0 {
                    let u = self.sun_rect[0] + (px * 0.5 + 0.5) * self.sun_rect[2];
                    let v = self.sun_rect[1] + (py * 0.5 + 0.5) * self.sun_rect[3];
                    let texel = atlas[(v.floor() as u32 * atlas_w + u.floor() as u32) as usize];
                    let a = (texel >> 24) as f32 / 255.0;
                    let rgb = Vec3::new(
                        (texel & 0xFF) as f32,
                        ((texel >> 8) & 0xFF) as f32,
                        ((texel >> 16) & 0xFF) as f32,
                    ) / 255.0;
                    c += rgb * a;
                }
            }
        }
        c.min(Vec3::ONE)
    }

    /// Vanilla environmental fog (cylindrical distance).
    pub fn fog_factor(&self, rel: Vec3) -> f32 {
        let d = (rel.x * rel.x + rel.z * rel.z).sqrt().max(rel.y.abs());
        let (start, end) = (self.fog_color[3], self.sun_dir[3]);
        ((d - start) / (end - start).max(1e-3)).clamp(0.0, 1.0)
    }
}

/// Physically-motivated sky and sun parameters for HQ mode (linear RGB).
pub struct HqSky {
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub zenith: Vec3,
    pub horizon: Vec3,
}

pub fn hq_sky(sun_dir: Vec3, sun_intensity: f32, sky_intensity: f32) -> HqSky {
    let alt = sun_dir.y;
    let day = (alt * 4.0 + 0.4).clamp(0.0, 1.0);
    // Warmer and dimmer sunlight near the horizon.
    let warm = (alt / 0.35).clamp(0.0, 1.0);
    let sun_tint = Vec3::new(1.0, 0.55, 0.3).lerp(Vec3::new(1.0, 0.96, 0.9), warm);
    let sun_color = sun_tint * sun_intensity * (alt * 6.0).clamp(0.0, 1.0);
    let zenith =
        Vec3::new(0.16, 0.32, 0.75).lerp(Vec3::new(0.01, 0.015, 0.04), 1.0 - day) * sky_intensity;
    let horizon = Vec3::new(0.42, 0.56, 0.8).lerp(Vec3::new(0.9, 0.55, 0.35), (1.0 - warm) * day)
        * day.max(0.03)
        * sky_intensity;
    HqSky {
        sun_dir,
        sun_color,
        zenith,
        horizon,
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

    #[test]
    fn altitude_azimuth() {
        assert!((sun_direction(90.0, 0.0) - Vec3::Y).length() < 1e-5);
        assert!((sun_direction(0.0, 90.0) - Vec3::X).length() < 1e-5);
        assert!((sun_direction(0.0, 0.0) + Vec3::Z).length() < 1e-5);
    }
}
