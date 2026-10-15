//! HQ mode scene: a two-level brick grid of model instances plus entity overlay
//! cells, flattened into the buffers described in `lumenwright_kernel::scene`.

use std::collections::HashMap;

use glam::Vec3;
use lumenwright_kernel::scene::{BRICK_WORDS, EMPTY, GpuModel, GpuQuad, OV_CELL_WORDS, UNIFORM};
use rayon::prelude::*;

use super::{Fluid, Scene};
use crate::assets::model::Dir;
use crate::world::StateId;

pub struct HqScene {
    pub grid: Vec<u32>,
    pub models: Vec<GpuModel>,
    pub quads: Vec<GpuQuad>,
    pub atlas: Vec<u32>,
    pub atlas_size: [u32; 2],
    /// Grid size in bricks.
    pub dims: [u32; 3],
    /// Word offsets: top, bricks, ov_top, ov_cells, ov_quads.
    pub offsets: [u32; 5],
    pub stats: HqStats,
}

#[derive(Debug, Default, Clone)]
pub struct HqStats {
    pub bricks: usize,
    pub uniform_bricks: usize,
    pub buried_bricks: usize,
    pub instances: usize,
    pub overlay_bricks: usize,
    pub entity_quads: usize,
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
    pub fn memory_bytes(&self) -> usize {
        self.grid.len() * 4 + self.models.len() * 16 + self.quads.len() * 80 + self.atlas.len() * 4
    }
}

fn dir_bit(d: Dir) -> u8 {
    1 << d.index()
}

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
    for (i, q) in scene.entity_quads.iter().enumerate() {
        quads.push(q.to_gpu());
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
    stats.entity_quads = scene.entity_quads.len();

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
    models.push(GpuModel {
        quad_offset: offset,
        quad_count: buf.len() as u32,
        flags: 0,
        _pad: 0,
    });
    (models.len() - 1) as u32
}
