//! HQ mode scene: a two-level brick grid of model instances plus entity overlay
//! cells, flattened into the buffers described in `lumenwright_kernel::scene`.

use std::collections::HashMap;

use glam::Vec3;
use lumenwright_kernel::scene::{
    BRICK_WORDS, EMITTER_GRID_HEADER, EMPTY, FLAG_FRONT_ONLY, GpuEmitter, GpuModel, GpuQuad,
    OV_CELL_WORDS, UNIFORM,
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
    /// The emitters: one entry per block whose material emits. Never empty (one unused
    /// entry when there are none); `emitter_count` says how many count.
    pub emitters: Vec<GpuEmitter>,
    /// Number of emitters in `emitters`.
    pub emitter_count: u32,
    /// The emitter grid (layout in `lumenwright_kernel::scene::SceneView`).
    pub emitter_grid: Vec<u32>,
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
    /// Emitting blocks in the emitter list.
    pub emitters: usize,
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
            + self.emitters.len() * 32
            + self.emitter_grid.len() * 4
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
    // Per model id: whether its block's material emits.
    let mut model_emits: Vec<bool> = vec![false];
    // (voxel corner in grid space, model id), in brick then voxel order.
    let mut emitter_sites: Vec<([i32; 3], u32)> = Vec::new();
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
                        &mut model_emits,
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
        // Every emitting block is an emitter, buried or not, as Chunky adds every
        // block whose material emits to its grid.
        for (local, &id) in ids.iter().enumerate() {
            if model_emits[id as usize] {
                let (lx, ly, lz) = (
                    local as i32 & 15,
                    local as i32 >> 8,
                    (local as i32 >> 4) & 15,
                );
                emitter_sites.push(([bx * 16 + lx, by * 16 + ly, bz * 16 + lz], id as u32));
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

    let (emitters, emitter_grid) = emitter_list(&models, &emitter_sites);
    stats.emitters = emitter_sites.len();
    if log::log_enabled!(log::Level::Info) && !emitter_sites.is_empty() {
        let mut by_name: std::collections::BTreeMap<&str, usize> = Default::default();
        for &([x, y, z], _) in &emitter_sites {
            let w = [
                x + v.min_chunk[0] * 16,
                y + v.min_section * 16,
                z + v.min_chunk[1] * 16,
            ];
            let state = scene.registry.state(scene.volume.block(w[0], w[1], w[2]));
            *by_name.entry(state.short_name()).or_default() += 1;
        }
        log::info!("emitters: {by_name:?}");
    }

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
        emitter_count: emitter_sites.len() as u32,
        emitters,
        emitter_grid,
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
    model_emits: &mut Vec<bool>,
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
    model_emits.push(!buf.is_empty() && scene.blocks[id as usize].emittance > 0.0);
    models.push(GpuModel {
        quad_offset: offset,
        quad_count: buf.len() as u32,
        flags: 0,
        _pad: 0,
    });
    (models.len() - 1) as u32
}

/// The edge of an emitter-grid cell in blocks: Chunky's default (`PersistentSettings.
/// getGridSizeDefault`, chunky-dev/chunky 156e2bb).
pub const EMITTER_CELL: i32 = 10;

/// The emitter list and the emitter grid (layouts in `lumenwright_kernel::scene`) from
/// (voxel corner in grid space, model id) sites: Chunky's `Grid.addEmitter` and
/// `Grid.prepare` (GPL-3.0, chunky-dev/chunky 156e2bb). Cells of [`EMITTER_CELL`]
/// blocks span the emitters' cells and one more on each side; each emitter is listed
/// in its own cell and in its 26 neighbours. An emitter's faces are its block model's
/// quads. Chunky's cells start at its octree origin, the loaded chunks' smallest
/// corner with the height rounded down to a multiple of 16; these start at the grid
/// origin, the same point when the scene's lowest height is the world's.
fn emitter_list(models: &[GpuModel], sites: &[([i32; 3], u32)]) -> (Vec<GpuEmitter>, Vec<u32>) {
    if sites.is_empty() {
        let mut grid = vec![0u32; EMITTER_GRID_HEADER as usize];
        grid[0] = EMITTER_CELL as u32;
        return (vec![GpuEmitter::default()], grid);
    }
    let emitters: Vec<GpuEmitter> = sites
        .iter()
        .map(|&(c, m)| {
            let model = models[m as usize];
            GpuEmitter {
                pos: [
                    c[0] as f32,
                    c[1] as f32,
                    c[2] as f32,
                    f32::from_bits(model.quad_offset),
                ],
                faces: [model.quad_count, 0, 0, 0],
            }
        })
        .collect();
    let cell_of = |c: [i32; 3]| c.map(|v| v.div_euclid(EMITTER_CELL));
    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    for &(c, _) in sites {
        let g = cell_of(c);
        for a in 0..3 {
            lo[a] = lo[a].min(g[a]);
            hi[a] = hi[a].max(g[a]);
        }
    }
    let offset = lo.map(|v| v - 1);
    let size: [i32; 3] = std::array::from_fn(|a| hi[a] - lo[a] + 3);
    let index = |g: [i32; 3]| -> usize {
        let l = [g[0] - offset[0], g[1] - offset[1], g[2] - offset[2]];
        ((l[1] * size[2] + l[2]) * size[0] + l[0]) as usize
    };
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); (size[0] * size[1] * size[2]) as usize];
    for (i, &(c, _)) in sites.iter().enumerate() {
        let g = cell_of(c);
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    cells[index([g[0] + dx, g[1] + dy, g[2] + dz])].push(i as u32);
                }
            }
        }
    }
    let mut grid = vec![
        EMITTER_CELL as u32,
        size[0] as u32,
        size[1] as u32,
        size[2] as u32,
        offset[0] as u32,
        offset[1] as u32,
        offset[2] as u32,
        0,
    ];
    let mut list = Vec::new();
    for c in &cells {
        grid.push(list.len() as u32);
        grid.push(c.len() as u32);
        list.extend_from_slice(c);
    }
    grid.extend(list);
    (emitters, grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chunky's grid guarantee (`Grid`): from any point, every emitter less than a cell
    /// away along each axis is a candidate, and none is two cells or more away; read
    /// back through the kernel's own cell lookup.
    #[test]
    fn the_emitter_grid_lists_every_emitter_within_a_cell() {
        let models = vec![
            GpuModel::default(),
            GpuModel {
                quad_offset: 3,
                quad_count: 2,
                flags: 0,
                _pad: 0,
            },
        ];
        let sites: Vec<([i32; 3], u32)> = vec![([25, 3, 0], 1), ([4, 40, 17], 1), ([26, 3, 1], 1)];
        let (emitters, grid) = emitter_list(&models, &sites);
        assert_eq!(emitters.len(), 3);
        assert_eq!(emitters[1].pos, [4.0, 40.0, 17.0, f32::from_bits(3)]);
        assert_eq!(emitters[1].face_count(), 2);
        let params = lumenwright_kernel::Params::default();
        let view = lumenwright_kernel::SceneView {
            params: &params,
            grid: &[],
            models: &[],
            quads: &[],
            atlas: &[],
            emitters: &emitters,
            emitter_grid: &grid,
        };
        let cells = (grid[1] * grid[2] * grid[3]) as usize;
        let list = &grid[EMITTER_GRID_HEADER as usize + cells * 2..];
        let mut checked = 0;
        for x in -5..50 {
            for y in -5..60 {
                for z in -5..40 {
                    let p = lumenwright_kernel::math::v3(
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        z as f32 + 0.5,
                    );
                    let (start, count) = lumenwright_kernel::integrator::emitter_cell(&view, p);
                    let found = &list[start as usize..(start + count) as usize];
                    for (i, &(c, _)) in sites.iter().enumerate() {
                        let d = [x - c[0], y - c[1], z - c[2]].map(i32::abs);
                        let listed = found.contains(&(i as u32));
                        if d.iter().all(|&a| a < EMITTER_CELL) {
                            assert!(listed, "emitter {i} from {x},{y},{z}");
                        }
                        if d.iter().any(|&a| a >= 2 * EMITTER_CELL) {
                            assert!(!listed, "emitter {i} listed from {x},{y},{z}");
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 55 * 65 * 45 * 3);
    }
}
