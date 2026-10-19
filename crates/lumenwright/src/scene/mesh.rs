//! Quick mode: vanilla-style triangle mesh with face culling, directional
//! shading, smooth lighting/ambient occlusion and a vanilla-like lightmap.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use rayon::prelude::*;

use super::{Alpha, Quad, Scene};
use crate::assets::model::Dir;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
/// One quick-mode vertex, laid out for the GPU vertex buffer.
pub struct Vertex {
    /// Camera-relative position.
    pub pos: [f32; 3],
    /// Normalized atlas UV.
    pub uv: [f32; 2],
    /// Vertex colour (tint × shade × AO × light), sRGB, rgba8.
    pub color: [u8; 4],
}

#[derive(Default)]
/// The quick-mode triangle mesh: a shared vertex list and two index lists,
/// three indices per triangle.
pub struct Mesh {
    /// Every vertex the index lists refer to.
    pub vertices: Vec<Vertex>,
    /// Opaque + cutout triangles (alpha tested).
    pub opaque: Vec<u32>,
    /// Translucent triangles, sorted back to front.
    pub translucent: Vec<u32>,
}

impl Mesh {
    /// Number of triangles in both index lists.
    pub fn triangle_count(&self) -> usize {
        (self.opaque.len() + self.translucent.len()) / 3
    }
}

#[derive(Clone, Copy, Debug)]
/// Lighting inputs to the quick-mode vertex colours.
pub struct Lighting {
    /// Daylight factor (vanilla `skyDarken`): 1 at noon, ~0.2 at midnight.
    pub sky_factor: f32,
    /// Vanilla brightness option: 0 = moody, 0.5 = default, 1 = bright.
    pub gamma: f32,
    /// Smooth lighting and ambient occlusion; `false` is vanilla's flat lighting.
    pub smooth: bool,
}

/// Vanilla lightmap for (sky, block) light levels (fractional, 0..15). sRGB-ish 0..1.
pub fn lightmap(sky: f32, block: f32, l: &Lighting) -> Vec3 {
    let curve = |lv: f32| {
        let f = (lv / 15.0).clamp(0.0, 1.0);
        f / (4.0 - 3.0 * f)
    };
    let sky_col = Vec3::new(l.sky_factor, l.sky_factor, 1.0).lerp(Vec3::ONE, 0.35);
    let s = curve(sky) * l.sky_factor;
    let b = curve(block) * 1.5;
    let bg = b * ((b * 0.6 + 0.4) * 0.6 + 0.4);
    let bb = b * (b * b * 0.6 + 0.4);
    let mut c = Vec3::new(b, bg, bb) + sky_col * s;
    c = c.lerp(Vec3::splat(0.75), 0.04).clamp(Vec3::ZERO, Vec3::ONE);
    let not_gamma = Vec3::ONE - (Vec3::ONE - c).powf(4.0);
    c = c.lerp(not_gamma, l.gamma);
    c.lerp(Vec3::splat(0.75), 0.04).clamp(Vec3::ZERO, Vec3::ONE)
}

/// Vanilla block face shading by normal (up 1.0, down 0.5, N/S 0.8, E/W 0.6).
pub fn face_shade(n: Vec3) -> f32 {
    let up = if n.y > 0.0 { 1.0 } else { 0.5 };
    (n.x * n.x * 0.6 + n.z * n.z * 0.8 + n.y * n.y * up).min(1.0)
}

/// Vanilla entity lighting: two fixed directional lights plus ambient.
pub fn entity_shade(n: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    (0.4 + 0.6 * (n.dot(l0).max(0.0) + n.dot(l1).max(0.0))).min(1.0)
}

#[derive(Clone, Copy)]
struct Sample {
    occ: bool,
    sky: u8,
    block: u8,
}

/// Tangent axes (A, B) used to parameterise a face for AO interpolation.
fn face_axes(d: Dir) -> (usize, usize) {
    match d {
        Dir::Up | Dir::Down => (0, 2),
        Dir::North | Dir::South => (0, 1),
        Dir::East | Dir::West => (2, 1),
    }
}

fn on_boundary(q: &Quad, d: Dir) -> bool {
    let p = q.p0;
    let near = |v: f32, t: f32| (v - t).abs() < 1e-4;
    match d {
        Dir::Up => near(p.y, 1.0),
        Dir::Down => near(p.y, 0.0),
        Dir::North => near(p.z, 0.0),
        Dir::South => near(p.z, 1.0),
        Dir::West => near(p.x, 0.0),
        Dir::East => near(p.x, 1.0),
    }
}

struct Ctx<'a> {
    scene: &'a Scene,
    lighting: Lighting,
}

impl Ctx<'_> {
    fn sample(&self, p: [i32; 3]) -> Sample {
        let occ = self.scene.occludes_at(p[0], p[1], p[2]);
        let (sky, block) = self.scene.volume.light(p[0], p[1], p[2]);
        Sample { occ, sky, block }
    }

    /// Per-corner (AO, sky, block) for a face, corners indexed [sa][sb].
    fn corners(&self, base: [i32; 3], d: Dir) -> [[(f32, f32, f32); 2]; 2] {
        let (ai, bi) = face_axes(d);
        let center = self.sample(base);
        let mut out = [[(1.0, 15.0, 0.0); 2]; 2];
        let off = |v: [i32; 3], axis: usize, s: i32| {
            let mut v = v;
            v[axis] += s;
            v
        };
        let shade = |s: &Sample| if s.occ { 0.2 } else { 1.0 };
        for (ia, sa) in [-1, 1].into_iter().enumerate() {
            for (ib, sb) in [-1, 1].into_iter().enumerate() {
                let a = self.sample(off(base, ai, sa));
                let b = self.sample(off(base, bi, sb));
                let c = if a.occ && b.occ {
                    a
                } else {
                    self.sample(off(off(base, ai, sa), bi, sb))
                };
                let ao = (shade(&a) + shade(&b) + shade(&c) + shade(&center)) * 0.25;
                // Vanilla blend: zero (fully dark, typically solid) samples take the centre value.
                let pick = |s: &Sample| {
                    if s.sky == 0 && s.block == 0 {
                        center
                    } else {
                        *s
                    }
                };
                let (a, b, c) = (pick(&a), pick(&b), pick(&c));
                let sky = (a.sky as f32 + b.sky as f32 + c.sky as f32 + center.sky as f32) * 0.25;
                let block =
                    (a.block as f32 + b.block as f32 + c.block as f32 + center.block as f32) * 0.25;
                out[ia][ib] = (ao, sky, block);
            }
        }
        out
    }

    /// Computes the 4 vertex colours of a block quad at world position `pos`.
    fn block_colors(&self, q: &Quad, pos: [i32; 3]) -> [[u8; 4]; 4] {
        let n = q.normal();
        let dir = Dir::from_normal(n);
        let shade = if q.shade { face_shade(n) } else { 1.0 };
        let tint = Vec3::new(q.tint[0] as f32, q.tint[1] as f32, q.tint[2] as f32) / 255.0;
        let emissive = q.emission > 0.0;
        let corners = q.corners();
        let mut out = [[0u8; 4]; 4];
        let finish = |ao: f32, sky: f32, block: f32| -> [u8; 4] {
            let block = if emissive {
                block.max(q.emission * 15.0)
            } else {
                block
            };
            let c = tint * shade * ao * lightmap(sky, block, &self.lighting);
            [
                (c.x * 255.0).round() as u8,
                (c.y * 255.0).round() as u8,
                (c.z * 255.0).round() as u8,
                255,
            ]
        };
        match dir {
            Some(d) if self.lighting.smooth && q.ao && !emissive => {
                let base = if on_boundary(q, d) {
                    let o = d.offset();
                    [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]]
                } else {
                    pos
                };
                let cv = self.corners(base, d);
                let (ai, bi) = face_axes(d);
                for (i, c) in corners.iter().enumerate() {
                    let s = c[ai].clamp(0.0, 1.0);
                    let t = c[bi].clamp(0.0, 1.0);
                    let lerp3 = |a: (f32, f32, f32), b: (f32, f32, f32), w: f32| {
                        (
                            a.0 + (b.0 - a.0) * w,
                            a.1 + (b.1 - a.1) * w,
                            a.2 + (b.2 - a.2) * w,
                        )
                    };
                    let v0 = lerp3(cv[0][0], cv[1][0], s);
                    let v1 = lerp3(cv[0][1], cv[1][1], s);
                    let (ao, sky, block) = lerp3(v0, v1, t);
                    out[i] = finish(ao, sky, block);
                }
            }
            _ => {
                let p = match dir {
                    Some(d) if on_boundary(q, d) => {
                        let o = d.offset();
                        [pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]]
                    }
                    _ => pos,
                };
                let (sky, block) = self.scene.volume.light(p[0], p[1], p[2]);
                let c = finish(1.0, sky as f32, block as f32);
                out = [c; 4];
            }
        }
        out
    }
}

#[derive(Default)]
struct Chunk {
    vertices: Vec<Vertex>,
    opaque: Vec<u32>,
    translucent: Vec<(f32, [u32; 6])>,
}

impl Chunk {
    fn push_quad(
        &mut self,
        corners: [Vec3; 4],
        uvs: [[f32; 2]; 4],
        colors: [[u8; 4]; 4],
        alpha: Alpha,
        inv_atlas: [f32; 2],
        eye: Vec3,
    ) {
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(Vertex {
                pos: (corners[i] - eye).to_array(),
                uv: [uvs[i][0] * inv_atlas[0], uvs[i][1] * inv_atlas[1]],
                color: colors[i],
            });
        }
        // TL, BL, BR, TR is counter-clockwise around the outward normal.
        let idx = [base, base + 1, base + 2, base, base + 2, base + 3];
        if alpha == Alpha::Translucent {
            let c = (corners[0] + corners[2]) * 0.5 - eye;
            self.translucent.push((c.length_squared(), idx));
        } else {
            self.opaque.extend_from_slice(&idx);
        }
    }
}

/// Six frustum planes (ax + by + cz + d >= 0 inside) from a view-projection matrix.
fn frustum_planes(m: &Mat4) -> [Vec4; 6] {
    let r = [m.row(0), m.row(1), m.row(2), m.row(3)];
    [
        r[3] + r[0],
        r[3] - r[0],
        r[3] + r[1],
        r[3] - r[1],
        r[2],
        r[3] - r[2],
    ]
}

fn aabb_visible(planes: &[Vec4; 6], lo: Vec3, hi: Vec3) -> bool {
    planes.iter().all(|p| {
        let v = Vec3::new(
            if p.x >= 0.0 { hi.x } else { lo.x },
            if p.y >= 0.0 { hi.y } else { lo.y },
            if p.z >= 0.0 { hi.z } else { lo.z },
        );
        p.x * v.x + p.y * v.y + p.z * v.z + p.w >= 0.0
    })
}

/// What [`build_mesh`] needs to know about the view.
pub struct MeshParams {
    /// Eye position in scene coordinates.
    pub eye: Vec3,
    /// Camera-relative view-projection, used for section frustum culling.
    pub view_proj: Mat4,
    /// Horizontal render distance in blocks.
    pub max_distance: f32,
    /// Daylight, brightness and smooth-lighting settings.
    pub lighting: Lighting,
}

/// Builds the quick-mode mesh for `scene`, blocks and entity quads together:
/// sections outside the frustum or beyond `max_distance` are skipped, hidden
/// faces are culled, vertex colours carry tint, directional shade, ambient
/// occlusion and the lightmap, and translucent triangles are sorted back to
/// front from `eye`.
pub fn build_mesh(scene: &Scene, p: &MeshParams) -> Mesh {
    let t = std::time::Instant::now();
    let v = &scene.volume;
    let ctx = Ctx {
        scene,
        lighting: p.lighting,
    };
    let planes = frustum_planes(&p.view_proj);
    let inv_atlas = [
        1.0 / scene.atlas.width as f32,
        1.0 / scene.atlas.height as f32,
    ];
    let origin = Vec3::new(
        scene.origin[0] as f32,
        scene.origin[1] as f32,
        scene.origin[2] as f32,
    );
    let eye_world = p.eye + origin;

    let section_ids: Vec<usize> = (0..v.sections.len())
        .filter(|&i| v.sections[i].as_ref().is_some_and(|s| !s.is_empty()))
        .collect();
    let chunks: Vec<Chunk> = section_ids
        .par_iter()
        .filter_map(|&i| {
            let i = i as i32;
            let sx = v.min_chunk[0] + i % v.chunks[0];
            let sz = v.min_chunk[1] + (i / v.chunks[0]) % v.chunks[1];
            let sy = v.min_section + i / (v.chunks[0] * v.chunks[1]);
            let lo = Vec3::new((sx * 16) as f32, (sy * 16) as f32, (sz * 16) as f32);
            let hi = lo + Vec3::splat(16.0);
            // Cylindrical distance check, then frustum check (camera-relative).
            let cx = (eye_world.x).clamp(lo.x, hi.x) - eye_world.x;
            let cz = (eye_world.z).clamp(lo.z, hi.z) - eye_world.z;
            if (cx * cx + cz * cz).sqrt() > p.max_distance {
                return None;
            }
            if !aabb_visible(&planes, lo - eye_world, hi - eye_world) {
                return None;
            }
            let sec = v.section(sx, sy, sz)?;
            let mut chunk = Chunk::default();
            let mut quads = Vec::new();
            let mut scratch = Vec::new();
            for ly in 0..16 {
                for lz in 0..16 {
                    for lx in 0..16 {
                        let id = sec.block(lx, ly, lz);
                        if id == 0 {
                            continue;
                        }
                        let pos = [
                            sx * 16 + lx as i32,
                            sy * 16 + ly as i32,
                            sz * 16 + lz as i32,
                        ];
                        quads.clear();
                        scene.block_quads(
                            pos,
                            id,
                            &|d| scene.face_culled(id, pos, d),
                            &mut scratch,
                            &mut quads,
                        );
                        let local = Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) - origin;
                        for q in &quads {
                            let colors = ctx.block_colors(q, pos);
                            let corners = q.corners().map(|c| c + local);
                            chunk.push_quad(
                                corners,
                                q.corner_uvs(),
                                colors,
                                q.alpha,
                                inv_atlas,
                                p.eye,
                            );
                        }
                    }
                }
            }
            Some(chunk)
        })
        .collect();

    // Entities.
    let mut ent = Chunk::default();
    for (q, el) in scene.entity_quads.iter().zip(&scene.entity_quad_light) {
        let lp = el.pos;
        let w = [
            lp[0] + scene.origin[0],
            lp[1] + scene.origin[1],
            lp[2] + scene.origin[2],
        ];
        let (sky, block) = if el.full_bright {
            (15, 15)
        } else {
            scene.volume.light(w[0], w[1], w[2])
        };
        let shade = if el.shade {
            entity_shade(q.normal())
        } else {
            1.0
        };
        let tint = Vec3::new(q.tint[0] as f32, q.tint[1] as f32, q.tint[2] as f32) / 255.0;
        let c = tint * shade * lightmap(sky as f32, block as f32, &p.lighting);
        let col = [
            (c.x * 255.0) as u8,
            (c.y * 255.0) as u8,
            (c.z * 255.0) as u8,
            255,
        ];
        ent.push_quad(
            q.corners(),
            q.corner_uvs(),
            [col; 4],
            q.alpha,
            inv_atlas,
            p.eye,
        );
    }

    let mut mesh = Mesh::default();
    let mut translucent: Vec<(f32, [u32; 6])> = Vec::new();
    for c in chunks.into_iter().chain(std::iter::once(ent)) {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend_from_slice(&c.vertices);
        mesh.opaque.extend(c.opaque.iter().map(|i| i + base));
        translucent.extend(
            c.translucent
                .into_iter()
                .map(|(d, idx)| (d, idx.map(|i| i + base))),
        );
    }
    translucent.par_sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    mesh.translucent = translucent.into_iter().flat_map(|(_, idx)| idx).collect();
    log::info!(
        "mesh: {} vertices, {} triangles ({} translucent) in {:.2?}",
        mesh.vertices.len(),
        mesh.triangle_count(),
        mesh.translucent.len() / 3,
        t.elapsed()
    );
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightmap_extremes() {
        let l = Lighting {
            sky_factor: 1.0,
            gamma: 0.5,
            smooth: true,
        };
        let full = lightmap(15.0, 0.0, &l);
        assert!(full.min_element() > 0.95, "{full}");
        let dark = lightmap(0.0, 0.0, &l);
        assert!(
            dark.max_element() < 0.12,
            "vanilla keeps a ~0.1 floor: {dark}"
        );
        let torch = lightmap(0.0, 8.0, &l);
        assert!(torch.x > torch.z, "block light is warm: {torch}");
    }

    #[test]
    fn vanilla_face_shades() {
        assert_eq!(face_shade(Vec3::Y), 1.0);
        assert_eq!(face_shade(-Vec3::Y), 0.5);
        assert!((face_shade(Vec3::Z) - 0.8).abs() < 1e-6);
        assert!((face_shade(Vec3::X) - 0.6).abs() < 1e-6);
    }
}
