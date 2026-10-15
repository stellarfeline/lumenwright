//! Vanilla block model JSON: parent resolution and baking into quads.

use std::collections::HashMap;

use glam::{Mat3, Vec3};
use serde::Deserialize;

use super::source::{ResourceStore, norm_loc};

/// The six face directions, in vanilla order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const ALL: [Dir; 6] = [
        Dir::Down,
        Dir::Up,
        Dir::North,
        Dir::South,
        Dir::West,
        Dir::East,
    ];

    pub fn parse(s: &str) -> Option<Dir> {
        Some(match s {
            "down" | "bottom" => Dir::Down,
            "up" | "top" => Dir::Up,
            "north" => Dir::North,
            "south" => Dir::South,
            "west" => Dir::West,
            "east" => Dir::East,
            _ => return None,
        })
    }

    pub fn offset(self) -> [i32; 3] {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub fn normal(self) -> Vec3 {
        let o = self.offset();
        Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    /// Nearest axis direction for a (unit) vector, if it is axis aligned.
    pub fn from_normal(n: Vec3) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.normal().dot(n) > 0.999)
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct ModelJson {
    pub parent: Option<String>,
    #[serde(default)]
    pub textures: HashMap<String, String>,
    pub elements: Option<Vec<ElementJson>>,
    pub ambientocclusion: Option<bool>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ElementJson {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub rotation: Option<ElementRotation>,
    #[serde(default = "yes")]
    pub shade: bool,
    #[serde(default)]
    pub light_emission: u8,
    #[serde(default)]
    pub faces: HashMap<String, FaceJson>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone, Debug)]
pub struct ElementRotation {
    #[serde(default = "center")]
    pub origin: [f32; 3],
    pub axis: Option<String>,
    #[serde(default)]
    pub angle: f32,
    // Newer formats may specify per-axis angles.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub z: Option<f32>,
    #[serde(default)]
    pub rescale: bool,
}

fn center() -> [f32; 3] {
    [8.0, 8.0, 8.0]
}

#[derive(Deserialize, Clone, Debug)]
pub struct FaceJson {
    pub uv: Option<[f32; 4]>,
    pub texture: String,
    pub cullface: Option<String>,
    #[serde(default)]
    pub rotation: i32,
    #[serde(default = "no_tint")]
    pub tintindex: i32,
}

fn no_tint() -> i32 {
    -1
}

/// A fully resolved model: elements plus a flat texture variable map.
#[derive(Clone, Debug)]
pub struct ResolvedModel {
    pub elements: Vec<ElementJson>,
    pub textures: HashMap<String, String>,
    pub ambient_occlusion: bool,
}

pub const MISSING_TEXTURE: &str = "lumenwright:missing";

impl ResolvedModel {
    /// Resolves `#var` chains to a texture location.
    pub fn texture<'s>(&'s self, mut name: &'s str) -> String {
        for _ in 0..16 {
            match name.strip_prefix('#') {
                Some(var) => match self.textures.get(var) {
                    Some(next) => name = next,
                    None => return MISSING_TEXTURE.to_string(),
                },
                None => return norm_loc(name),
            }
        }
        MISSING_TEXTURE.to_string()
    }
}

/// Loads and resolves a model with its parent chain.
pub fn resolve_model(store: &ResourceStore, loc: &str) -> Option<ResolvedModel> {
    let mut chain: Vec<ModelJson> = Vec::new();
    let mut cur = Some(loc.to_string());
    while let Some(name) = cur.take() {
        if chain.len() > 32 {
            break;
        }
        // "builtin/..." parents have no JSON.
        if name.contains("builtin/") {
            break;
        }
        let bytes = store.resolve("models", &name, "json")?;
        let m: ModelJson = match serde_json::from_slice(bytes) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("model {name}: {e}");
                return None;
            }
        };
        cur = m.parent.clone();
        chain.push(m);
    }
    let mut textures = HashMap::new();
    let mut elements = None;
    let mut ao = None;
    // Root first so children override.
    for m in chain.iter().rev() {
        for (k, v) in &m.textures {
            textures.insert(k.clone(), v.clone());
        }
    }
    for m in &chain {
        if elements.is_none() {
            elements = m.elements.clone();
        }
        if ao.is_none() {
            ao = m.ambientocclusion;
        }
    }
    Some(ResolvedModel {
        elements: elements.unwrap_or_default(),
        textures,
        ambient_occlusion: ao.unwrap_or(true),
    })
}

/// A quad in block-local coordinates (0..1), ready for either renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedQuad {
    pub p0: Vec3,
    pub eu: Vec3,
    pub ev: Vec3,
    /// Texture location (`ns:path`).
    pub texture: String,
    /// UV (0..16 sprite space) at p0, and its derivatives along eu / ev.
    pub uv0: [f32; 2],
    pub du: [f32; 2],
    pub dv: [f32; 2],
    pub cullface: Option<Dir>,
    pub tintindex: i32,
    pub shade: bool,
    pub light_emission: u8,
}

impl BakedQuad {
    /// Outward unit normal.
    pub fn normal(&self) -> Vec3 {
        self.ev.cross(self.eu).normalize_or_zero()
    }

    /// Axis-aligned face direction, if any.
    pub fn dir(&self) -> Option<Dir> {
        Dir::from_normal(self.normal())
    }

    pub fn corners(&self) -> [Vec3; 4] {
        // Vanilla order: TL, BL, BR, TR.
        [
            self.p0,
            self.p0 + self.ev,
            self.p0 + self.eu + self.ev,
            self.p0 + self.eu,
        ]
    }

    /// UV (0..16) at parametric (s, t).
    pub fn uv_at(&self, s: f32, t: f32) -> [f32; 2] {
        [
            self.uv0[0] + s * self.du[0] + t * self.dv[0],
            self.uv0[1] + s * self.du[1] + t * self.dv[1],
        ]
    }
}

#[derive(Clone, Debug, Default)]
pub struct BakedModel {
    pub quads: Vec<BakedQuad>,
    pub ambient_occlusion: bool,
    /// The model has a single element spanning the whole block with 6 faces.
    pub full_cube: bool,
}

/// Face parallelogram for an axis-aligned box face (positions in 0..16 units),
/// with s along the texture's u axis and t along v (vanilla orientation).
fn face_geometry(dir: Dir, from: Vec3, to: Vec3) -> (Vec3, Vec3, Vec3) {
    let d = to - from;
    match dir {
        Dir::North => (
            Vec3::new(to.x, to.y, from.z),
            Vec3::new(-d.x, 0.0, 0.0),
            Vec3::new(0.0, -d.y, 0.0),
        ),
        Dir::South => (
            Vec3::new(from.x, to.y, to.z),
            Vec3::new(d.x, 0.0, 0.0),
            Vec3::new(0.0, -d.y, 0.0),
        ),
        Dir::East => (
            Vec3::new(to.x, to.y, to.z),
            Vec3::new(0.0, 0.0, -d.z),
            Vec3::new(0.0, -d.y, 0.0),
        ),
        Dir::West => (
            Vec3::new(from.x, to.y, from.z),
            Vec3::new(0.0, 0.0, d.z),
            Vec3::new(0.0, -d.y, 0.0),
        ),
        Dir::Up => (
            Vec3::new(from.x, to.y, from.z),
            Vec3::new(d.x, 0.0, 0.0),
            Vec3::new(0.0, 0.0, d.z),
        ),
        Dir::Down => (
            Vec3::new(from.x, from.y, to.z),
            Vec3::new(d.x, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -d.z),
        ),
    }
}

/// Vanilla's default UVs for a face of a box (0..16 units).
fn default_uv(dir: Dir, from: Vec3, to: Vec3) -> [f32; 4] {
    match dir {
        Dir::Down => [from.x, 16.0 - to.z, to.x, 16.0 - from.z],
        Dir::Up => [from.x, from.z, to.x, to.z],
        Dir::North => [16.0 - to.x, 16.0 - to.y, 16.0 - from.x, 16.0 - from.y],
        Dir::South => [from.x, 16.0 - to.y, to.x, 16.0 - from.y],
        Dir::West => [from.z, 16.0 - to.y, to.z, 16.0 - from.y],
        Dir::East => [16.0 - to.z, 16.0 - to.y, 16.0 - from.z, 16.0 - from.y],
    }
}

/// UV corners for a face rectangle `[u0, v0, u1, v1]` with a 0/90/180/270
/// rotation, following vanilla's `BlockFaceUV` index shifting. Returns UVs for
/// the corners TL, BL, BR, TR.
fn rotated_uv_corners(uv: [f32; 4], rotation: i32) -> [[f32; 2]; 4] {
    let shift = (rotation.rem_euclid(360) / 90) as usize;
    let at = |i: usize| -> [f32; 2] {
        let i = (i + shift) % 4;
        let u = if i == 0 || i == 1 { uv[0] } else { uv[2] };
        let v = if i == 0 || i == 3 { uv[1] } else { uv[3] };
        [u, v]
    };
    [at(0), at(1), at(2), at(3)]
}

fn axis_rotation(axis: &str, deg: f32) -> Mat3 {
    let r = deg.to_radians();
    match axis {
        "x" => Mat3::from_rotation_x(r),
        "y" => Mat3::from_rotation_y(r),
        _ => Mat3::from_rotation_z(r),
    }
}

/// Blockstate-level rotation (x then y, in 90° steps) as a matrix acting on
/// block-local positions centred at the origin.
pub fn variant_rotation(x: i32, y: i32) -> Mat3 {
    Mat3::from_rotation_y((-y as f32).to_radians())
        * Mat3::from_rotation_x((-x as f32).to_radians())
}

fn snap(v: Vec3) -> Vec3 {
    let s = |f: f32| {
        let r = (f * 16.0).round() / 16.0;
        if (f - r).abs() < 1e-4 { r } else { f }
    };
    Vec3::new(s(v.x), s(v.y), s(v.z))
}

/// Bakes a resolved model with a blockstate rotation.
pub fn bake(model: &ResolvedModel, x: i32, y: i32, uvlock: bool) -> BakedModel {
    let rot = variant_rotation(x, y);
    let rotated = x.rem_euclid(360) != 0 || y.rem_euclid(360) != 0;
    let half = Vec3::splat(0.5);
    let mut quads = Vec::new();
    let mut full_cube = model.elements.len() == 1;
    for el in &model.elements {
        let from = Vec3::from(el.from);
        let to = Vec3::from(el.to);
        if from != Vec3::ZERO
            || to != Vec3::splat(16.0)
            || el.faces.len() != 6
            || el.rotation.is_some()
        {
            full_cube = false;
        }
        // Element rotation (about `origin`, with optional rescale).
        let el_rot: Option<(Mat3, Vec3, Vec3)> = el.rotation.as_ref().and_then(|r| {
            let (m, axis_scale) = if let Some(axis) = r.axis.as_deref() {
                if r.angle == 0.0 {
                    return None;
                }
                let m = axis_rotation(axis, r.angle);
                let mut scale = match axis {
                    "x" => Vec3::new(0.0, 1.0, 1.0),
                    "y" => Vec3::new(1.0, 0.0, 1.0),
                    _ => Vec3::new(1.0, 1.0, 0.0),
                };
                if r.rescale {
                    scale *= 1.0 / r.angle.to_radians().cos() - 1.0;
                    scale += Vec3::ONE;
                } else {
                    scale = Vec3::ONE;
                }
                (m, scale)
            } else {
                let m = Mat3::from_rotation_x(r.x.unwrap_or(0.0).to_radians())
                    * Mat3::from_rotation_y(r.y.unwrap_or(0.0).to_radians())
                    * Mat3::from_rotation_z(r.z.unwrap_or(0.0).to_radians());
                (m, Vec3::ONE)
            };
            Some((m, Vec3::from(r.origin), axis_scale))
        });
        // Fixed face order (not HashMap order) so baking is reproducible.
        let mut faces: Vec<(Dir, &FaceJson)> = el
            .faces
            .iter()
            .filter_map(|(n, f)| Dir::parse(n).map(|d| (d, f)))
            .collect();
        faces.sort_by_key(|(d, _)| d.index());
        for (dir, face) in faces {
            let (p0, eu, ev) = face_geometry(dir, from, to);
            let uv = face.uv.unwrap_or_else(|| default_uv(dir, from, to));
            let c = rotated_uv_corners(uv, face.rotation);
            // Corners TL=p0, BL=p0+ev, BR, TR=p0+eu.
            let (mut uv0, mut du, mut dv) = (
                c[0],
                [c[3][0] - c[0][0], c[3][1] - c[0][1]],
                [c[1][0] - c[0][0], c[1][1] - c[0][1]],
            );

            // Positions in 0..16 -> element rotation -> 0..1 -> variant rotation.
            let tf = |p: Vec3| -> Vec3 {
                let p = match &el_rot {
                    Some((m, o, s)) => (*m * (p - *o)) * *s + *o,
                    None => p,
                };
                rot * (p / 16.0 - half) + half
            };
            let a = tf(p0);
            let b = tf(p0 + eu);
            let cc = tf(p0 + ev);
            let (q0, qu, qv) = (snap(a), snap(b) - snap(a), snap(cc) - snap(a));
            let cullface = face.cullface.as_deref().and_then(Dir::parse).map(|d| {
                if rotated {
                    Dir::from_normal(rot * d.normal()).unwrap_or(d)
                } else {
                    d
                }
            });

            let mut quad = BakedQuad {
                p0: q0,
                eu: qu,
                ev: qv,
                texture: model.texture(&face.texture),
                uv0,
                du,
                dv,
                cullface,
                tintindex: face.tintindex,
                shade: el.shade,
                light_emission: el.light_emission,
            };
            if uvlock && rotated {
                // Approximation of vanilla uvlock: re-derive the UVs from the
                // rotated face position, as if the face had never been rotated.
                if let Some(nd) = quad.dir() {
                    let corners = quad.corners();
                    let lo = corners.iter().fold(Vec3::splat(f32::MAX), |m, c| m.min(*c)) * 16.0;
                    let hi = corners.iter().fold(Vec3::splat(f32::MIN), |m, c| m.max(*c)) * 16.0;
                    let (g0, gu, gv) = face_geometry(nd, lo, hi);
                    let duv = default_uv(nd, lo, hi);
                    let cr = rotated_uv_corners(duv, 0);
                    // Re-express quad on the canonical parallelogram for its direction.
                    quad.p0 = g0 / 16.0;
                    quad.eu = gu / 16.0;
                    quad.ev = gv / 16.0;
                    uv0 = cr[0];
                    du = [cr[3][0] - cr[0][0], cr[3][1] - cr[0][1]];
                    dv = [cr[1][0] - cr[0][0], cr[1][1] - cr[0][1]];
                    quad.uv0 = uv0;
                    quad.du = du;
                    quad.dv = dv;
                }
            }
            quads.push(quad);
        }
    }
    BakedModel {
        quads,
        ambient_occlusion: model.ambient_occlusion,
        full_cube,
    }
}

/// A unit cube using one texture on all faces (used for missing models).
pub fn cube_all(texture: &str) -> BakedModel {
    let mut textures = HashMap::new();
    textures.insert("all".to_string(), texture.to_string());
    let faces: HashMap<String, FaceJson> = ["down", "up", "north", "south", "west", "east"]
        .iter()
        .map(|d| {
            (
                d.to_string(),
                FaceJson {
                    uv: None,
                    texture: "#all".into(),
                    cullface: Some(d.to_string()),
                    rotation: 0,
                    tintindex: -1,
                },
            )
        })
        .collect();
    let el = ElementJson {
        from: [0.0; 3],
        to: [16.0; 3],
        rotation: None,
        shade: true,
        light_emission: 0,
        faces,
    };
    bake(
        &ResolvedModel {
            elements: vec![el],
            textures,
            ambient_occlusion: true,
        },
        0,
        0,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> BakedModel {
        cube_all("minecraft:block/stone")
    }

    #[test]
    fn cube_faces_point_outward() {
        let m = cube();
        assert!(m.full_cube);
        assert_eq!(m.quads.len(), 6);
        for q in &m.quads {
            let d = q.cullface.unwrap();
            assert_eq!(q.dir(), Some(d), "face {:?} normal {:?}", d, q.normal());
            // Face centre lies on the boundary in the face direction.
            let c = q.p0 + (q.eu + q.ev) * 0.5;
            let expect = Vec3::splat(0.5) + d.normal() * 0.5;
            assert!((c - expect).length() < 1e-5, "{d:?}: {c:?}");
        }
    }

    #[test]
    fn default_uvs_full_face() {
        for q in cube().quads {
            assert_eq!(q.uv0, [0.0, 0.0]);
            assert_eq!(q.du, [16.0, 0.0]);
            assert_eq!(q.dv, [0.0, 16.0]);
        }
    }

    #[test]
    fn y_rotation_moves_north_to_east() {
        let mut textures = HashMap::new();
        textures.insert(
            "front".to_string(),
            "minecraft:block/furnace_front".to_string(),
        );
        let mut faces = HashMap::new();
        faces.insert(
            "north".to_string(),
            FaceJson {
                uv: None,
                texture: "#front".into(),
                cullface: Some("north".into()),
                rotation: 0,
                tintindex: -1,
            },
        );
        let el = ElementJson {
            from: [0.0; 3],
            to: [16.0; 3],
            rotation: None,
            shade: true,
            light_emission: 0,
            faces,
        };
        let m = ResolvedModel {
            elements: vec![el],
            textures,
            ambient_occlusion: true,
        };
        let b = bake(&m, 0, 90, false);
        assert_eq!(b.quads[0].dir(), Some(Dir::East));
        assert_eq!(b.quads[0].cullface, Some(Dir::East));
        let b = bake(&m, 0, 180, false);
        assert_eq!(b.quads[0].dir(), Some(Dir::South));
        // x=90 rotates the top towards the north (vanilla log/pillar convention).
        let mut up = m.clone();
        let f = up.elements[0].faces.remove("north").unwrap();
        up.elements[0].faces.insert(
            "up".into(),
            FaceJson {
                cullface: Some("up".into()),
                ..f
            },
        );
        let b = bake(&up, 90, 0, false);
        assert_eq!(b.quads[0].dir(), Some(Dir::North));
    }

    #[test]
    fn uv_rotation_matches_vanilla_shift() {
        // rotation 90: TL shows the texture's bottom-left.
        let c = rotated_uv_corners([0.0, 0.0, 16.0, 16.0], 90);
        assert_eq!(c[0], [0.0, 16.0]);
        assert_eq!(c[3], [0.0, 0.0]);
    }
}
