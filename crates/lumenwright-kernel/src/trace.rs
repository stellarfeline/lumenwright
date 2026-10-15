//! Two-level DDA through the brick grid plus parallelogram intersection.

use crate::math::{V3, abs, floor, max, min, v3};
use crate::scene::{
    ALPHA_CUTOUT, ALPHA_TRANSLUCENT, BRICK_WORDS, EMPTY, FLAG_ALPHA_MASK, GpuQuad, OV_CELL_WORDS,
    SceneView, UNIFORM,
};

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub quad: u32,
    /// Texel at the hit (rgba8, sRGB).
    pub texel: u32,
}

const NO_HIT: Hit = Hit {
    t: f32::INFINITY,
    quad: u32::MAX,
    texel: 0,
};
const MAX_STEPS: u32 = 65536;

/// Intersects a ray with a (two-sided) quad. Returns `(t, s, tt)`.
#[inline(always)]
pub fn intersect_quad(q: &GpuQuad, o: V3, d: V3, tmin: f32, tmax: f32) -> Option<(f32, f32, f32)> {
    let eu = q.edge_u();
    let ev = q.edge_v();
    let n = eu.cross(ev);
    let denom = n.dot(d);
    if abs(denom) < 1e-12 {
        return None;
    }
    let p0 = q.origin();
    let t = n.dot(p0 - o) / denom;
    if !(t > tmin && t < tmax) {
        return None;
    }
    let p = o + d * t - p0;
    let nn = n.dot(n);
    let s = p.cross(ev).dot(n) / nn;
    let tt = eu.cross(p).dot(n) / nn;
    if s < 0.0 || s > 1.0 || tt < 0.0 || tt > 1.0 {
        return None;
    }
    Some((t, s, tt))
}

/// Texel lookup for a quad at parametric coordinates (s, t).
#[inline(always)]
pub fn quad_texel(scene: &SceneView, q: &GpuQuad, s: f32, t: f32) -> u32 {
    // Stay strictly inside the sprite so nearest sampling never bleeds.
    let s = min(max(s, 0.0005), 0.9995);
    let t = min(max(t, 0.0005), 0.9995);
    let u = q.uv[0] + s * q.uv[2] + t * q.uvt[0];
    let v = q.uv[1] + s * q.uv[3] + t * q.uvt[1];
    scene.texel(u, v)
}

/// Tests a quad, applying alpha testing. Updates `best` when closer.
#[inline(always)]
fn test_quad(scene: &SceneView, qi: u32, o: V3, d: V3, tmin: f32, best: &mut Hit) {
    let q = &scene.quads[qi as usize];
    if let Some((t, s, tt)) = intersect_quad(q, o, d, tmin, best.t) {
        let texel = quad_texel(scene, q, s, tt);
        let alpha = texel >> 24;
        let mode = q.flags() & FLAG_ALPHA_MASK;
        if (mode == ALPHA_CUTOUT && alpha < 128) || (mode == ALPHA_TRANSLUCENT && alpha == 0) {
            return;
        }
        *best = Hit { t, quad: qi, texel };
    }
}

/// Finds the closest surface along the ray `o + t*d`, `t ∈ (tmin, tmax)`.
/// `o` is in grid-relative coordinates.
pub fn trace(scene: &SceneView, o: V3, d: V3, tmin: f32, tmax: f32) -> Option<Hit> {
    let p = scene.params;
    let dims = p.grid_dims;
    let size = scene.grid_size();

    // Slab test against the grid bounds.
    let inv = v3(safe_inv(d.x), safe_inv(d.y), safe_inv(d.z));
    let ta = (V3::ZERO - o).mul(inv);
    let tb = (size - o).mul(inv);
    let t_enter = max(max(min(ta.x, tb.x), min(ta.y, tb.y)), min(ta.z, tb.z));
    let t_exit = min(min(max(ta.x, tb.x), max(ta.y, tb.y)), max(ta.z, tb.z));
    let mut t = max(t_enter, 0.0);
    let t_end = min(t_exit, tmax);
    if t > t_end {
        return None;
    }

    let start = o + d * (t + 1e-4);
    let gs = [
        dims[0] as i32 * 16,
        dims[1] as i32 * 16,
        dims[2] as i32 * 16,
    ];
    let mut cell = [
        (floor(start.x) as i32).clamp(0, gs[0] - 1),
        (floor(start.y) as i32).clamp(0, gs[1] - 1),
        (floor(start.z) as i32).clamp(0, gs[2] - 1),
    ];
    let step = [sign(d.x), sign(d.y), sign(d.z)];
    let delta = [abs(inv.x), abs(inv.y), abs(inv.z)];
    let mut next = [0.0f32; 3];
    for a in 0..3 {
        next[a] = next_boundary(o.get(a), inv.get(a), cell[a], step[a]);
    }

    let top_off = p.off[0];
    let ov_top_off = p.off[2];
    let best_t = t_end + 1e-3;

    // Bounded like the WGSL kernel so a malformed scene can never hang a backend.
    for _ in 0..MAX_STEPS {
        let bx = (cell[0] >> 4) as u32;
        let by = (cell[1] >> 4) as u32;
        let bz = (cell[2] >> 4) as u32;
        let b = (by * dims[2] + bz) * dims[0] + bx;
        let entry = scene.grid[(top_off + b) as usize];
        let ov = scene.grid[(ov_top_off + b) as usize];

        if entry == EMPTY && ov == EMPTY {
            // Skip the whole brick: jump to its exit face.
            let bc = [bx as i32, by as i32, bz as i32];
            let mut t_out = f32::INFINITY;
            let mut axis = 0;
            for a in 0..3 {
                if step[a] != 0 {
                    let bound = if step[a] > 0 {
                        (bc[a] + 1) * 16
                    } else {
                        bc[a] * 16
                    };
                    let tt = (bound as f32 - o.get(a)) * inv.get(a);
                    if tt < t_out {
                        t_out = tt;
                        axis = a;
                    }
                }
            }
            if t_out > t_end {
                break;
            }
            let pos = o + d * t_out;
            for a in 0..3 {
                if a == axis {
                    cell[a] = if step[a] > 0 {
                        (bc[a] + 1) * 16
                    } else {
                        bc[a] * 16 - 1
                    };
                } else {
                    cell[a] = (floor(pos.get(a)) as i32).clamp(bc[a] * 16, bc[a] * 16 + 15);
                }
            }
            if cell[axis] < 0 || cell[axis] >= gs[axis] {
                break;
            }
            for a in 0..3 {
                next[a] = next_boundary(o.get(a), inv.get(a), cell[a], step[a]);
            }
            t = t_out;
            continue;
        }

        let t_cell_exit = min(next[0], min(next[1], next[2]));
        let t_lo = max(t - 1e-4, tmin);
        let t_hi = t_cell_exit + 1e-4;

        // Block model in this voxel (block-local coordinates).
        let model = if entry == EMPTY {
            0
        } else if entry & UNIFORM != 0 {
            entry & !UNIFORM
        } else {
            let local = (((cell[1] & 15) * 16 + (cell[2] & 15)) * 16 + (cell[0] & 15)) as u32;
            let w = scene.grid[(p.off[1] + entry * BRICK_WORDS + (local >> 1)) as usize];
            (w >> ((local & 1) * 16)) & 0xFFFF
        };
        let mut cand = NO_HIT;
        cand.t = min(t_hi, best_t);
        if model != 0 {
            let m = scene.models[model as usize];
            let lo = o - v3(cell[0] as f32, cell[1] as f32, cell[2] as f32);
            for i in 0..m.quad_count {
                test_quad(scene, m.quad_offset + i, lo, d, t_lo, &mut cand);
            }
        }
        // Entity overlay quads (grid-relative coordinates).
        if ov != EMPTY {
            let local = (((cell[1] & 15) * 16 + (cell[2] & 15)) * 16 + (cell[0] & 15)) as u32;
            let base = (p.off[3] + ov * OV_CELL_WORDS + local * 2) as usize;
            let off = scene.grid[base];
            let count = scene.grid[base + 1];
            for i in 0..count {
                let qi = scene.grid[(p.off2[0] + off + i) as usize];
                test_quad(scene, qi, o, d, t_lo, &mut cand);
            }
        }
        if cand.quad != u32::MAX {
            return Some(cand);
        }

        // Advance to the next voxel.
        let axis = if next[0] < next[1] {
            if next[0] < next[2] { 0 } else { 2 }
        } else if next[1] < next[2] {
            1
        } else {
            2
        };
        t = next[axis];
        if t > t_end {
            break;
        }
        cell[axis] += step[axis];
        if cell[axis] < 0 || cell[axis] >= gs[axis] {
            break;
        }
        next[axis] += delta[axis];
    }
    None
}

#[inline(always)]
fn safe_inv(x: f32) -> f32 {
    if abs(x) < 1e-12 {
        if x < 0.0 { -1e12 } else { 1e12 }
    } else {
        1.0 / x
    }
}

#[inline(always)]
fn sign(x: f32) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

#[inline(always)]
fn next_boundary(o: f32, inv: f32, cell: i32, step: i32) -> f32 {
    if step == 0 {
        return f32::INFINITY;
    }
    let bound = if step > 0 { cell + 1 } else { cell } as f32;
    (bound - o) * inv
}

/// Re-exported for the integrator.
pub fn hit_point(o: V3, d: V3, h: &Hit) -> V3 {
    o + d * h.t
}

#[allow(dead_code)]
fn _assert_layout() {
    // Compile-time size checks used by the GPU mirrors.
    const _: () = assert!(core::mem::size_of::<GpuQuad>() == 80);
    const _: () = assert!(core::mem::size_of::<crate::scene::Params>() == 256);
    const _: () = assert!(core::mem::size_of::<crate::scene::GpuModel>() == 16);
}
