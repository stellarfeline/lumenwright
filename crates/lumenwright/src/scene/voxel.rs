//! HQ mode scene: a two-level brick grid of model instances plus entity overlay
//! cells, flattened into the buffers described in `lumenwright_kernel::scene`.

use std::collections::HashMap;

use glam::Vec3;
use lumenwright_kernel::scene::{
    ALPHA_CUTOUT, ALPHA_TRANSLUCENT, BRICK_WORDS, EMPTY, FLAG_ALPHA_MASK, FLAG_FRONT_ONLY,
    GpuLight, GpuModel, GpuQuad, OV_CELL_WORDS, UNIFORM, unpack_rgba,
};
use rayon::prelude::*;

use super::{Fluid, Scene};
use crate::assets::model::Dir;
use crate::world::StateId;

/// The HQ-mode scene, flattened into the buffers every path-tracing backend reads
/// (layout in `lumenwright_kernel::scene`).
pub struct HqScene {
    /// The grid buffer: top-level brick entries, brick voxels and the entity overlay,
    /// at the word offsets in `offsets`.
    pub grid: Vec<u32>,
    /// Block model instances; voxels hold indices into this list.
    pub models: Vec<GpuModel>,
    /// Every quad: block-model quads in block-local coordinates, then entity quads
    /// in grid-relative world coordinates.
    pub quads: Vec<GpuQuad>,
    /// The texture atlas, one RGBA8 texel per word, row by row.
    pub atlas: Vec<u32>,
    /// Atlas width and height in texels.
    pub atlas_size: [u32; 2],
    /// Grid size in bricks.
    pub dims: [u32; 3],
    /// Word offsets: top, bricks, ov_top, ov_cells, ov_quads.
    pub offsets: [u32; 5],
    /// First index and count of the see-through quads in `quads`.
    pub see_through: [u32; 2],
    /// The light list: one entry per instance of a light-casting quad. Never empty
    /// (one unused entry when there are no lights); `light_count` says how many count.
    pub lights: Vec<GpuLight>,
    /// Number of lights in `lights`.
    pub light_count: u32,
    /// What the grid holds, for logging.
    pub stats: HqStats,
}

#[derive(Debug, Default, Clone)]
/// Counts of what [`build_hq`] put in the grid.
pub struct HqStats {
    /// Bricks stored voxel by voxel.
    pub bricks: usize,
    /// Bricks whose voxels are all one instance, stored as a single entry.
    pub uniform_bricks: usize,
    /// Bricks of occluding blocks whose six neighbour bricks also occlude, collapsed
    /// to one entry.
    pub buried_bricks: usize,
    /// Distinct block model instances.
    pub instances: usize,
    /// Bricks that hold entity geometry.
    pub overlay_bricks: usize,
    /// Entity and block-entity quads.
    pub entity_quads: usize,
    /// Instances of light-casting quads in the light list.
    pub lights: usize,
}

/// Identifies one block model instance: everything that changes its quads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct InstanceKey {
    state: StateId,
    /// Chosen baked models (multipart parts beyond 8 are folded into a hash).
    models: [u32; 8],
    tint_pos: u32,
    cull: u8,
    fluid_mask: u8,
}

/// A section's voxels as indices into a section-local list of instance keys.
struct SectionKeys {
    /// (key, a position where it occurs) per local id; local id 0 is air.
    keys: Vec<(InstanceKey, [i32; 3])>,
    ids: Vec<u16>,
}

impl HqScene {
    /// Approximate GPU memory the buffers take, in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.grid.len() * 4
            + self.models.len() * 16
            + self.quads.len() * 80
            + self.atlas.len() * 4
            + self.lights.len() * 32
    }
}

fn dir_bit(d: Dir) -> u8 {
    1 << d.index()
}

/// Builds the HQ-mode buffers from `scene`.
///
/// # Errors
///
/// Fails when the view holds more than 65535 distinct block model instances,
/// the most a voxel's 16-bit id can name.
pub fn build_hq(scene: &Scene) -> anyhow::Result<HqScene> {
    let t = std::time::Instant::now();
    let v = &scene.volume;
    let dims = [v.chunks[0] as u32, v.sections_y as u32, v.chunks[1] as u32];
    let nb = (dims[0] * dims[1] * dims[2]) as usize;
    let mut stats = HqStats::default();

    // Section index in the volume == brick index (same YZX layout).
    let all_occluding: Vec<bool> = v
        .sections
        .par_iter()
        .map(|s| match s {
            None => false,
            Some(s) => match &s.blocks {
                crate::world::SectionBlocks::Uniform(id) => scene.blocks[*id as usize].occludes,
                crate::world::SectionBlocks::Paletted { palette, .. } => {
                    palette.iter().all(|p| scene.blocks[*p as usize].occludes)
                }
            },
        })
        .collect();
    let brick_xyz = |i: usize| -> [i32; 3] {
        let i = i as i32;
        [
            i % v.chunks[0],
            i / (v.chunks[0] * v.chunks[1]),
            (i / v.chunks[0]) % v.chunks[1],
        ]
    };
    let brick_index = |b: [i32; 3]| -> Option<usize> {
        if b.iter().any(|&c| c < 0)
            || b[0] >= v.chunks[0]
            || b[1] >= v.sections_y
            || b[2] >= v.chunks[1]
        {
            None
        } else {
            Some(((b[1] * v.chunks[1] + b[2]) * v.chunks[0] + b[0]) as usize)
        }
    };

    // Per-state fast path: instances that do not depend on position.
    let simple: Vec<bool> = scene
        .blocks
        .iter()
        .map(|b| {
            b.fluid.is_none()
                && !b.waterlogged
                && !b.self_cull
                && b.models.is_deterministic()
                && !b.models.parts.iter().flatten().any(|(m, _)| {
                    scene.baked[*m as usize]
                        .quads
                        .iter()
                        .any(|q| q.tintindex >= 0)
                })
        })
        .collect();

    // Phase 1 (parallel): per-section instance keys with a local palette.
    let keyed: Vec<Option<SectionKeys>> = v
        .sections
        .par_iter()
        .enumerate()
        .map(|(bi, sec)| {
            let sec = sec.as_ref().filter(|s| !s.is_empty())?;
            let [bx, by, bz] = brick_xyz(bi);
            let base = [
                (v.min_chunk[0] + bx) * 16,
                (v.min_section + by) * 16,
                (v.min_chunk[1] + bz) * 16,
            ];
            let mut local: HashMap<InstanceKey, u16> = HashMap::new();
            let mut keys = Vec::new();
            let mut ids = vec![0u16; 4096];
            let mut scratch = Vec::new();
            for ly in 0..16 {
                for lz in 0..16 {
                    for lx in 0..16 {
                        let id = sec.block(lx, ly, lz);
                        let info = &scene.blocks[id as usize];
                        if id == 0 || info.invisible {
                            continue;
                        }
                        let pos = [
                            base[0] + lx as i32,
                            base[1] + ly as i32,
                            base[2] + lz as i32,
                        ];
                        let key = if simple[id as usize] {
                            InstanceKey {
                                state: id,
                                models: [u32::MAX; 8],
                                tint_pos: 0,
                                cull: 0,
                                fluid_mask: 0,
                            }
                        } else {
                            info.models.choose(pos[0], pos[1], pos[2], &mut scratch);
                            let mut models = [u32::MAX; 8];
                            for (i, m) in scratch.iter().enumerate() {
                                if i < 7 {
                                    models[i] = *m;
                                } else {
                                    models[7] =
                                        models[7].rotate_left(5) ^ m.wrapping_mul(0x9E37_79B9);
                                }
                            }
                            let mut cull = 0u8;
                            if info.self_cull && info.fluid.is_none() {
                                for d in Dir::ALL {
                                    if scene.face_culled(id, pos, d) {
                                        cull |= dir_bit(d);
                                    }
                                }
                            }
                            let fluid_mask = if info.fluid.is_some() || info.waterlogged {
                                scene.fluid_face_mask(
                                    pos,
                                    !matches!(info.fluid, Some(Fluid::Lava(_))),
                                )
                            } else {
                                0
                            };
                            let tint_pos = if info.fluid.is_some()
                                || info.tint != Some(crate::assets::blockinfo::TintKind::None)
                            {
                                scene.volume.biome(pos[0], pos[1], pos[2]) as u32
                            } else {
                                0
                            };
                            InstanceKey {
                                state: id,
                                models,
                                tint_pos,
                                cull,
                                fluid_mask,
                            }
                        };
                        let lid = *local.entry(key).or_insert_with(|| {
                            keys.push((key, pos));
                            keys.len() as u16
                        });
                        ids[(ly * 16 + lz) * 16 + lx] = lid;
                    }
                }
            }
            Some(SectionKeys { keys, ids })
        })
        .collect();

    // Phase 2 (serial): intern instances globally and lay out bricks.
    let mut models: Vec<GpuModel> = vec![GpuModel::default()];
    let mut quads: Vec<GpuQuad> = Vec::new();
    let mut instance_ids: HashMap<InstanceKey, u32> = HashMap::new();
    let mut top = vec![EMPTY; nb];
    let mut bricks: Vec<u32> = Vec::new();
    let mut scratch = Vec::new();
    let mut quad_buf = Vec::new();
    // Per model id: its light-casting quads (index, cullface).
    let mut model_lights: Vec<Vec<(u32, Option<Dir>)>> = vec![Vec::new()];
    // (voxel corner in grid space, quad index), in brick then voxel order.
    let mut light_sites: Vec<([f32; 3], u32)> = Vec::new();
    for (bi, sk) in keyed.into_iter().enumerate() {
        let Some(sk) = sk else { continue };
        if sk.keys.is_empty() {
            continue;
        }
        let mut map = vec![0u32; sk.keys.len() + 1];
        for (i, (key, pos)) in sk.keys.iter().enumerate() {
            map[i + 1] = match instance_ids.get(key) {
                Some(&iid) => iid,
                None => {
                    let iid = add_instance(
                        scene,
                        &mut models,
                        &mut quads,
                        &mut model_lights,
                        *pos,
                        key.state,
                        key.cull,
                        &mut scratch,
                        &mut quad_buf,
                    );
                    instance_ids.insert(*key, iid);
                    iid
                }
            };
        }
        if models.len() > 0x10000 {
            anyhow::bail!(
                "more than 65535 distinct block model instances in view; reduce --render-distance"
            );
        }
        let ids: Vec<u16> = sk.ids.iter().map(|&l| map[l as usize] as u16).collect();
        let [bx, by, bz] = brick_xyz(bi);
        // Fully buried: nothing inside can ever be seen by a ray from outside.
        let buried = all_occluding[bi]
            && Dir::ALL.iter().all(|d| {
                let o = d.offset();
                brick_index([bx + o[0], by + o[1], bz + o[2]]).is_some_and(|n| all_occluding[n])
            });
        if !buried {
            let base = [
                (v.min_chunk[0] + bx) * 16,
                (v.min_section + by) * 16,
                (v.min_chunk[1] + bz) * 16,
            ];
            for (local, &id) in ids.iter().enumerate() {
                let ml = &model_lights[id as usize];
                if ml.is_empty() {
                    continue;
                }
                let (lx, ly, lz) = (
                    local as i32 & 15,
                    local as i32 >> 8,
                    (local as i32 >> 4) & 15,
                );
                let world = [base[0] + lx, base[1] + ly, base[2] + lz];
                let state = scene.volume.block(world[0], world[1], world[2]);
                let corner = [
                    (bx * 16 + lx) as f32,
                    (by * 16 + ly) as f32,
                    (bz * 16 + lz) as f32,
                ];
                for &(qi, cullface) in ml {
                    // A face against an occluding neighbour casts no light anyone sees.
                    if cullface.is_some_and(|d| scene.face_culled(state, world, d)) {
                        continue;
                    }
                    light_sites.push((corner, qi));
                }
            }
        }
        if buried {
            stats.buried_bricks += 1;
            top[bi] = UNIFORM | ids.iter().copied().find(|&i| i != 0).unwrap_or(1) as u32;
            continue;
        }
        if ids.iter().all(|&i| i == ids[0]) {
            stats.uniform_bricks += 1;
            top[bi] = UNIFORM | ids[0] as u32;
            continue;
        }
        top[bi] = (bricks.len() / BRICK_WORDS as usize) as u32;
        bricks.extend(
            ids.as_chunks::<2>()
                .0
                .iter()
                .map(|p| p[0] as u32 | ((p[1] as u32) << 16)),
        );
        stats.bricks += 1;
    }
    stats.instances = models.len() - 1;

    // Entity overlay: index world-space quads by every voxel their bounds touch.
    let entity_base = quads.len() as u32;
    let size = [
        dims[0] as i32 * 16,
        dims[1] as i32 * 16,
        dims[2] as i32 * 16,
    ];
    let mut cells: HashMap<usize, HashMap<u32, Vec<u32>>> = HashMap::new();
    // Entity quads, then text-display quads (seen from the front only), the
    // see-through ones last so they are one range.
    let front_only = |q: &super::Quad| {
        let mut g = q.to_gpu();
        g.p0[3] = f32::from_bits(g.p0[3].to_bits() | FLAG_FRONT_ONLY);
        g
    };
    let overlay: Vec<(GpuQuad, &super::Quad)> = scene
        .entity_quads
        .iter()
        .map(|q| (q.to_gpu(), q))
        .chain(
            scene
                .text_quads
                .iter()
                .filter(|t| !t.see_through)
                .map(|t| (front_only(&t.quad), &t.quad)),
        )
        .chain(
            scene
                .text_quads
                .iter()
                .filter(|t| t.see_through)
                .map(|t| (front_only(&t.quad), &t.quad)),
        )
        .collect();
    let see_through_count = scene.text_quads.iter().filter(|t| t.see_through).count() as u32;
    let see_through = [
        entity_base + overlay.len() as u32 - see_through_count,
        see_through_count,
    ];
    for (i, (g, q)) in overlay.iter().enumerate() {
        quads.push(*g);
        let c = q.corners();
        let lo = c.iter().fold(Vec3::splat(f32::MAX), |m, p| m.min(*p)) - Vec3::splat(1e-3);
        let hi = c.iter().fold(Vec3::splat(f32::MIN), |m, p| m.max(*p)) + Vec3::splat(1e-3);
        let (x0, y0, z0) = (
            (lo.x.floor() as i32).max(0),
            (lo.y.floor() as i32).max(0),
            (lo.z.floor() as i32).max(0),
        );
        let (x1, y1, z1) = (
            (hi.x.floor() as i32).min(size[0] - 1),
            (hi.y.floor() as i32).min(size[1] - 1),
            (hi.z.floor() as i32).min(size[2] - 1),
        );
        for y in y0..=y1 {
            for z in z0..=z1 {
                for x in x0..=x1 {
                    let b = brick_index([x >> 4, y >> 4, z >> 4]).unwrap();
                    let local = (((y & 15) * 16 + (z & 15)) * 16 + (x & 15)) as u32;
                    cells
                        .entry(b)
                        .or_default()
                        .entry(local)
                        .or_default()
                        .push(entity_base + i as u32);
                }
            }
        }
    }
    let mut ov_top = vec![EMPTY; nb];
    let mut ov_cells: Vec<u32> = Vec::new();
    let mut ov_quads: Vec<u32> = Vec::new();
    let mut overlay_bricks: Vec<_> = cells.into_iter().collect();
    overlay_bricks.sort_by_key(|(b, _)| *b);
    for (b, cell_map) in overlay_bricks {
        ov_top[b] = (ov_cells.len() / OV_CELL_WORDS as usize) as u32;
        let start = ov_cells.len();
        ov_cells.resize(start + OV_CELL_WORDS as usize, 0);
        for (local, list) in cell_map {
            ov_cells[start + local as usize * 2] = ov_quads.len() as u32;
            ov_cells[start + local as usize * 2 + 1] = list.len() as u32;
            ov_quads.extend(list);
        }
    }
    stats.overlay_bricks = ov_cells.len() / OV_CELL_WORDS as usize;
    stats.entity_quads = overlay.len();

    let (lights, light_count) = light_list(
        &mut quads,
        &scene.atlas.pixels,
        scene.atlas.width,
        &light_sites,
    );
    stats.lights = light_count as usize;

    let mut grid =
        Vec::with_capacity(top.len() * 2 + bricks.len() + ov_cells.len() + ov_quads.len() + 1);
    let off_top = 0u32;
    grid.extend_from_slice(&top);
    let off_bricks = grid.len() as u32;
    grid.extend_from_slice(&bricks);
    let off_ov_top = grid.len() as u32;
    grid.extend_from_slice(&ov_top);
    let off_ov_cells = grid.len() as u32;
    grid.extend_from_slice(&ov_cells);
    let off_ov_quads = grid.len() as u32;
    grid.extend_from_slice(&ov_quads);
    grid.push(0);
    if quads.is_empty() {
        quads.push(GpuQuad::default());
    }

    let hq = HqScene {
        grid,
        models,
        quads,
        atlas: scene.atlas.pixels.clone(),
        atlas_size: [scene.atlas.width, scene.atlas.height],
        dims,
        offsets: [off_top, off_bricks, off_ov_top, off_ov_cells, off_ov_quads],
        see_through,
        lights,
        light_count,
        stats,
    };
    log::info!(
        "hq scene: {:?}, {:.1} MiB, built in {:.2?}",
        hq.stats,
        hq.memory_bytes() as f64 / (1 << 20) as f64,
        t.elapsed()
    );
    Ok(hq)
}

#[allow(clippy::too_many_arguments)]
fn add_instance(
    scene: &Scene,
    models: &mut Vec<GpuModel>,
    quads: &mut Vec<GpuQuad>,
    model_lights: &mut Vec<Vec<(u32, Option<Dir>)>>,
    pos: [i32; 3],
    id: StateId,
    cull: u8,
    scratch: &mut Vec<u32>,
    buf: &mut Vec<super::Quad>,
) -> u32 {
    buf.clear();
    scene.block_quads(pos, id, &|d| cull & dir_bit(d) != 0, scratch, buf);
    let offset = quads.len() as u32;
    quads.extend(buf.iter().map(|q| q.to_gpu()));
    // The block's light level decides what its glowing quads cast.
    let level = scene.blocks[id as usize].emission;
    let mut lit = Vec::new();
    if level > 0 {
        let glowing: Vec<usize> = (0..buf.len()).filter(|&i| buf[i].emission > 0.0).collect();
        let power: Vec<f32> = glowing
            .iter()
            .map(|&i| {
                unit_power(
                    &quads[offset as usize + i],
                    &scene.atlas.pixels,
                    scene.atlas.width,
                )
            })
            .collect();
        let scale = light_scale(level, power.iter().sum());
        for (&i, &p) in glowing.iter().zip(&power) {
            if p > 0.0 && scale > 0.0 {
                quads[offset as usize + i].uvt[3] = scale;
                lit.push((offset + i as u32, buf[i].cullface));
            }
        }
    }
    model_lights.push(lit);
    models.push(GpuModel {
        quad_offset: offset,
        quad_count: buf.len() as u32,
        flags: 0,
        _pad: 0,
    });
    (models.len() - 1) as u32
}

/// Samples per quad axis when averaging a quad's texels.
const POWER_SAMPLES: u32 = 16;

/// The radiant power a quad emits for a unit radiance scale: π × area × the mean
/// luminance of its squared linear texel colours (the kernel's emitter mapping),
/// times their coverage, sampled on a regular 16×16 grid of (s, t) the way the
/// kernel reads texels.
fn unit_power(q: &GpuQuad, atlas: &[u32], atlas_w: u32) -> f32 {
    let area = q.normal().length();
    let h = (atlas.len() as u32 / atlas_w.max(1)) as i32;
    let tint = linear(q.tint());
    let mode = q.flags() & FLAG_ALPHA_MASK;
    let mut sum = 0.0f64;
    for j in 0..POWER_SAMPLES {
        for i in 0..POWER_SAMPLES {
            let s = ((i as f32 + 0.5) / POWER_SAMPLES as f32).clamp(0.0005, 0.9995);
            let t = ((j as f32 + 0.5) / POWER_SAMPLES as f32).clamp(0.0005, 0.9995);
            let u = q.uv[0] + s * q.uv[2] + t * q.uvt[0];
            let v = q.uv[1] + s * q.uv[3] + t * q.uvt[1];
            let x = (u.floor() as i32).clamp(0, atlas_w as i32 - 1);
            let y = (v.floor() as i32).clamp(0, h - 1);
            let texel = atlas[(y * atlas_w as i32 + x) as usize];
            let a = (texel >> 24) as f32 / 255.0;
            let cover = match mode {
                ALPHA_CUTOUT if texel >> 24 < 128 => 0.0,
                ALPHA_CUTOUT => 1.0,
                ALPHA_TRANSLUCENT => a,
                _ => 1.0,
            };
            let c = linear(texel);
            let e = [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]];
            let lum = 0.2126 * e[0] * e[0] + 0.7152 * e[1] * e[1] + 0.0722 * e[2] * e[2];
            sum += (lum * cover) as f64;
        }
    }
    let mean = (sum / (POWER_SAMPLES * POWER_SAMPLES) as f64) as f32;
    std::f32::consts::PI * area * mean
}

/// The kernel's sRGB -> linear polynomial, per channel.
fn linear(c: u32) -> [f32; 3] {
    let c = unpack_rgba(c);
    let f = lumenwright_kernel::math::srgb_to_linear;
    [f(c[0]), f(c[1]), f(c[2])]
}

/// Builds the light list from (voxel corner, quad) sites: each light is picked with
/// probability proportional to its power, and each light quad records that
/// probability ([`GpuQuad::light_pick`]) for the kernel's MIS weights.
fn light_list(
    quads: &mut [GpuQuad],
    atlas: &[u32],
    atlas_w: u32,
    sites: &[([f32; 3], u32)],
) -> (Vec<GpuLight>, u32) {
    let mut power_of: std::collections::BTreeMap<u32, f64> = std::collections::BTreeMap::new();
    let mut total = 0.0f64;
    for &(_, qi) in sites {
        let q = &quads[qi as usize];
        let p = *power_of
            .entry(qi)
            .or_insert_with(|| (unit_power(q, atlas, atlas_w) * q.light_scale()) as f64);
        total += p;
    }
    if sites.is_empty() || total <= 0.0 {
        return (vec![GpuLight::default()], 0);
    }
    for (&qi, &p) in &power_of {
        quads[qi as usize].uvt[2] = (p / total) as f32;
    }
    let mut acc = 0.0f64;
    let mut lights: Vec<GpuLight> = sites
        .iter()
        .map(|&(c, qi)| {
            acc += power_of[&qi];
            GpuLight {
                pos: [c[0], c[1], c[2], f32::from_bits(qi)],
                cdf: [(acc / total) as f32, 0.0, 0.0, 0.0],
            }
        })
        .collect();
    if let Some(last) = lights.last_mut() {
        last.cdf[0] = 1.0;
    }
    let n = lights.len() as u32;
    (lights, n)
}

/// Radiant power of a light-level-15 block, in the kernel's units (the sun's
/// irradiance is `sun_color`). Calibrated by `emitter_calibration` in
/// `lumenwright-cli/tests/render.rs`; see docs/research/emitters.md.
pub const LEVEL_15_POWER: f32 = 1.0;

/// The radiant power of a block of light level `level`: `LEVEL_15_POWER × (level/15)²`.
/// Under inverse-square falloff the distance at which a light's irradiance falls to
/// a given value grows with the square root of its power, so this makes a block's
/// reach proportional to its level, as the game's light does (one level per block).
pub fn block_light_power(level: u8) -> f32 {
    let f = level as f32 / 15.0;
    LEVEL_15_POWER * f * f
}

/// The emitted radiance scale of a block's glowing quads: the scale at which their
/// total power is [`block_light_power`] of the block's level, whatever its shape.
fn light_scale(level: u8, unit_power: f32) -> f32 {
    if unit_power <= 0.0 {
        return 0.0;
    }
    block_light_power(level) / unit_power
}
