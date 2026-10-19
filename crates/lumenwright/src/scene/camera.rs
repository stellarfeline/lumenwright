//! Pinhole camera using Minecraft's yaw/pitch convention.

use glam::{DVec3, Mat4, Vec3, Vec4};

#[derive(Clone, Copy, Debug)]
/// A pinhole camera: an eye position, Minecraft yaw and pitch, and a vertical field of view.
pub struct Camera {
    /// Absolute world position (eye).
    pub pos: DVec3,
    /// Degrees. 0 = facing south (+Z), 90 = west (-X), 180 = north, 270 = east.
    pub yaw: f32,
    /// Degrees. Positive looks down.
    pub pitch: f32,
    /// Vertical field of view in degrees (vanilla default 70).
    pub fov_y: f32,
}

impl Camera {
    /// Unit view direction for the yaw and pitch, in world axes (+Y up, +Z south).
    pub fn forward(&self) -> Vec3 {
        let (y, p) = (self.yaw.to_radians(), self.pitch.to_radians());
        Vec3::new(-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()).normalize()
    }

    /// Orthonormal (forward, right, up) basis.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let f = self.forward();
        let mut r = f.cross(Vec3::Y);
        if r.length_squared() < 1e-8 {
            // Looking straight up/down: derive "right" from yaw alone.
            let y = self.yaw.to_radians();
            r = Vec3::new(-y.cos(), 0.0, -y.sin());
        }
        let r = r.normalize();
        let u = r.cross(f).normalize();
        (f, r, u)
    }

    /// Yaw/pitch that look from `from` towards `to`.
    pub fn look_at(from: DVec3, to: DVec3) -> (f32, f32) {
        let d = (to - from).normalize();
        let pitch = (-d.y).asin().to_degrees() as f32;
        let yaw = (-d.x).atan2(d.z).to_degrees() as f32;
        (yaw, pitch)
    }

    /// View matrix for a camera at `eye` (scene coordinates).
    pub fn view(&self, eye: Vec3) -> Mat4 {
        let (f, r, u) = self.basis();
        // Right-handed view: camera looks down -Z.
        Mat4::from_cols(
            Vec4::new(r.x, u.x, -f.x, 0.0),
            Vec4::new(r.y, u.y, -f.y, 0.0),
            Vec4::new(r.z, u.z, -f.z, 0.0),
            Vec4::new(-r.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
        )
    }

    /// Right-handed perspective projection with a [0, 1] depth range (wgpu convention).
    pub fn projection(&self, aspect: f32, near: f32, far: f32) -> Mat4 {
        let h = 1.0 / (self.fov_y.to_radians() * 0.5).tan();
        let w = h / aspect;
        let r = far / (near - far);
        Mat4::from_cols(
            Vec4::new(w, 0.0, 0.0, 0.0),
            Vec4::new(0.0, h, 0.0, 0.0),
            Vec4::new(0.0, 0.0, r, -1.0),
            Vec4::new(0.0, 0.0, r * near, 0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(yaw: f32, pitch: f32) -> Camera {
        Camera {
            pos: DVec3::ZERO,
            yaw,
            pitch,
            fov_y: 70.0,
        }
    }

    #[test]
    fn minecraft_yaw_convention() {
        assert!((cam(0.0, 0.0).forward() - Vec3::Z).length() < 1e-5);
        assert!((cam(90.0, 0.0).forward() + Vec3::X).length() < 1e-5);
        assert!((cam(180.0, 0.0).forward() + Vec3::Z).length() < 1e-5);
        assert!((cam(0.0, 90.0).forward() + Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn basis_is_right_handed() {
        let (f, r, u) = cam(30.0, 20.0).basis();
        assert!(u.y > 0.0);
        assert!(
            (r.cross(u) - (-f)).length() < 1e-5,
            "camera looks down -Z of its own frame"
        );
        // Facing south, right is west (-X).
        let (_, r, _) = cam(0.0, 0.0).basis();
        assert!((r + Vec3::X).length() < 1e-5);
    }

    #[test]
    fn projection_maps_forward_point_to_center() {
        let c = cam(37.0, 12.0);
        let eye = Vec3::new(3.0, 4.0, 5.0);
        let vp = c.projection(1.5, 0.1, 100.0) * c.view(eye);
        let p = vp * (eye + c.forward() * 10.0).extend(1.0);
        let ndc = p.truncate() / p.w;
        assert!(
            ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4 && ndc.z > 0.0 && ndc.z < 1.0,
            "{ndc}"
        );
        let (_, r, u) = c.basis();
        let p = vp * (eye + c.forward() * 10.0 + r + u).extend(1.0);
        assert!(p.x > 0.0 && p.y > 0.0, "right/up map to +x/+y");
        let near = vp * (eye + c.forward() * 0.1).extend(1.0);
        assert!((near.z / near.w).abs() < 1e-4);
    }

    #[test]
    fn look_at_roundtrip() {
        let (yaw, pitch) = Camera::look_at(DVec3::ZERO, DVec3::new(-5.0, -5.0, 5.0));
        let c = cam(yaw, pitch);
        let d = Vec3::new(-1.0, -1.0, 1.0).normalize();
        assert!((c.forward() - d).length() < 1e-5);
    }
}
